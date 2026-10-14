//! UI-independent, owned HTTP transfer workers with bounded, coalescing state.

use std::path::PathBuf;

use tokio::{
    runtime::Handle,
    sync::{oneshot, watch},
};

use crate::transfer::{
    self, ResumeCapability, TransferControl, TransferOptions, TransferTelemetry,
};
use crate::{DownloadError, DownloadProgress, DownloadReceipt, DownloadRequest, Downloader};

/// The latest transfer state. Intermediate progress updates may be coalesced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferState {
    Preparing,
    Downloading(DownloadProgress),
    /// Session work remains owned by this task while its range streams pause.
    Paused(DownloadProgress),
    Completed(DownloadReceipt),
    Failed(String),
    /// The download future was dropped before reporting completion.
    /// Publication already underway can still finish; this is not a guarantee
    /// that no completed file exists. Temporary-file cleanup is best effort.
    Cancelled,
}

struct TerminalGuard(watch::Sender<TransferState>);

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let active = self.0.borrow().is_active();
        if active {
            self.0.send_replace(TransferState::Failed(
                "Transfer worker stopped unexpectedly".into(),
            ));
        }
    }
}

impl TransferState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed(_) | Self::Failed(_) | Self::Cancelled)
    }

    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }
}

/// Owns the cancellation signal for one HTTP transfer.
///
/// Closing this handle cancels without blocking or joining the worker. Keep the
/// supplied runtime alive until terminal state is observed. This handle does
/// not retain or expose the source URL through a `Debug` implementation.
pub struct HttpDownloadTask {
    state: watch::Receiver<TransferState>,
    cancellation: Option<oneshot::Sender<()>>,
    control: Option<TransferControl>,
    telemetry: Option<watch::Receiver<TransferTelemetry>>,
}

impl HttpDownloadTask {
    /// Starts preparation and transfer on the supplied runtime, not the caller.
    /// URL parsing, path checks, and client construction happen in the worker.
    pub fn start(runtime: &Handle, url: String, destination: PathBuf) -> Self {
        let (state_tx, state) = watch::channel(TransferState::Preparing);
        let (cancellation, cancelled) = oneshot::channel();
        let guard = TerminalGuard(state_tx.clone());
        runtime.spawn(async move {
            let _guard = guard;
            let terminal = match DownloadRequest::new(&url, destination) {
                Ok(request) => match Downloader::new() {
                    Ok(downloader) => {
                        // Scope the future so cancellation drops it (and its owned
                        // staging-file guards) before publishing terminal state.
                        let result = {
                            let transfer = downloader.download(&request, |progress| {
                                state_tx.send_replace(TransferState::Downloading(progress));
                            });
                            tokio::pin!(transfer);
                            tokio::select! {
                                biased;
                                result = &mut transfer => Some(result),
                                _ = cancelled => None,
                            }
                        };
                        match result {
                            Some(Ok(receipt)) => TransferState::Completed(receipt),
                            Some(Err(DownloadError::Cancelled)) | None => TransferState::Cancelled,
                            Some(Err(error)) => TransferState::Failed(error.to_string()),
                        }
                    }
                    Err(error) => TransferState::Failed(error.to_string()),
                },
                Err(error) => TransferState::Failed(error.to_string()),
            };
            state_tx.send_replace(terminal);
        });
        Self {
            state,
            cancellation: Some(cancellation),
            control: None,
            telemetry: None,
        }
    }

    /// Starts the controlled engine. Legacy `start` retains its single GET path.
    /// Validation and all network/filesystem work run on the supplied runtime.
    pub fn start_with_options(
        runtime: &Handle,
        url: String,
        destination: PathBuf,
        options: TransferOptions,
    ) -> Self {
        let (state_tx, state) = watch::channel(TransferState::Preparing);
        let (telemetry_tx, telemetry) = watch::channel(TransferTelemetry {
            destination: destination.clone(),
            bytes_per_second: options.bytes_per_second,
            ..TransferTelemetry::default()
        });
        let control = TransferControl::new(options.bytes_per_second);
        let worker_control = control.clone();
        let (cancellation, cancelled) = oneshot::channel();
        let guard = TerminalGuard(state_tx.clone());
        runtime.spawn(async move {
            let _guard = guard;
            let updates = state_tx.clone();
            // Drop the engine, its range workers, and staging guards before
            // publishing cancellation. A simultaneously ready result wins.
            let result = {
                let transfer =
                    transfer::run(&url, destination, options, worker_control, move |value| {
                        // Aborted range workers must not overwrite a terminal
                        // result if their final callback races cancellation.
                        updates.send_if_modified(|state| {
                            if state.is_terminal() {
                                return false;
                            }
                            telemetry_tx.send_replace(value.clone());
                            *state = if value.paused {
                                TransferState::Paused(value.progress)
                            } else {
                                TransferState::Downloading(value.progress)
                            };
                            true
                        });
                    });
                tokio::pin!(transfer);
                tokio::select! {
                    biased;
                    result = &mut transfer => Some(result),
                    _ = cancelled => None,
                }
            };
            let terminal = match result {
                Some(Ok(receipt)) => TransferState::Completed(receipt),
                Some(Err(DownloadError::Cancelled)) | None => TransferState::Cancelled,
                Some(Err(error)) => TransferState::Failed(error.to_string()),
            };
            state_tx.send_replace(terminal);
        });
        Self {
            state,
            cancellation: Some(cancellation),
            control: Some(control),
            telemetry: Some(telemetry),
        }
    }

    /// Subscribes to bounded latest telemetry; unavailable for legacy tasks.
    pub fn telemetry(&self) -> Option<watch::Receiver<TransferTelemetry>> {
        self.telemetry.clone()
    }

    /// Requests a session pause only after range/validator support is observed.
    pub fn pause(&self) -> bool {
        let Some(control) = &self.control else {
            return false;
        };
        if !matches!(*self.state.borrow(), TransferState::Downloading(_))
            || self
                .telemetry
                .as_ref()
                .is_none_or(|value| value.borrow().resume != ResumeCapability::Supported)
            || control.is_paused()
        {
            return false;
        }
        control.pause();
        true
    }

    /// Resumes a previously requested pause without discarding session bytes.
    pub fn resume(&self) -> bool {
        let Some(control) = &self.control else {
            return false;
        };
        if self.state.borrow().is_terminal()
            || self
                .telemetry
                .as_ref()
                .is_none_or(|value| value.borrow().resume != ResumeCapability::Supported)
            || !control.is_paused()
        {
            return false;
        }
        control.resume();
        true
    }

    /// Changes the shared live budget. `None` disables limiting; zero is rejected.
    pub fn set_rate_limit(&self, bytes_per_second: Option<u64>) -> bool {
        let Some(control) = &self.control else {
            return false;
        };
        if self.state.borrow().is_terminal() || bytes_per_second == Some(0) {
            return false;
        }
        control.set_rate_limit(bytes_per_second);
        true
    }

    /// Subscribes to the latest state without allocating an unbounded queue.
    /// Use `borrow_and_update` before `changed` to include the current state.
    pub fn subscribe(&self) -> watch::Receiver<TransferState> {
        self.state.clone()
    }

    /// Requests cancellation once. Calling this again has no effect.
    /// A completed result wins if completion and cancellation are both ready.
    pub fn cancel(&mut self) {
        if let Some(sender) = self.cancellation.take() {
            let _ = sender.send(());
        }
    }
}

impl Drop for HttpDownloadTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn worker_unwind_publishes_a_generic_terminal_failure() {
        let (sender, receiver) = watch::channel(TransferState::Preparing);
        let worker = tokio::spawn(async move {
            let _guard = TerminalGuard(sender);
            panic!("fixture worker panic");
        });
        assert!(worker.await.unwrap_err().is_panic());
        assert_eq!(
            *receiver.borrow(),
            TransferState::Failed("Transfer worker stopped unexpectedly".into())
        );
    }
}
