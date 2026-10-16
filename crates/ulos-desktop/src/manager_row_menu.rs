//! Per-record commands deliberately independent of checkbox selection and row order.
use super::*;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowCommand {
    Details,
    Start,
    Pause,
    Resume,
    Stop,
    Retry,
    Reveal,
    Remove,
}

#[cfg(test)]
#[path = "manager_row_menu_tests.rs"]
mod tests;

impl DownloadManager {
    pub(crate) fn row_command_enabled(&self, id: DownloadId, command: RowCommand) -> bool {
        let Some(item) = self.queue.item(id) else {
            return false;
        };
        match command {
            RowCommand::Details => true,
            RowCommand::Start => {
                item.is_queued() && item.has_source() && self.active.is_none() && !self.removal_busy
            }
            RowCommand::Pause | RowCommand::Resume => {
                let paused = matches!(item.state, Some(TransferState::Paused(_)));
                self.active == Some(id)
                    && item.is_active()
                    && !item.cancelling
                    && paused == (command == RowCommand::Resume)
                    && if item.is_torrent {
                        item.torrent_telemetry.as_ref().is_some_and(|telemetry| {
                            matches!(
                                telemetry.phase,
                                ulos_core::torrent::TorrentPhase::Downloading
                                    | ulos_core::torrent::TorrentPhase::Paused
                            )
                        })
                    } else {
                        item.telemetry.resume == ulos_core::transfer::ResumeCapability::Supported
                    }
            }
            RowCommand::Stop => self.active == Some(id) && item.is_active() && !item.cancelling,
            RowCommand::Retry => item.can_retry() && !self.removal_busy,
            RowCommand::Reveal => item.is_finished(),
            RowCommand::Remove => {
                self.active != Some(id)
                    && !item.is_active()
                    && !self.removal_busy
                    && !self.removal_open
                    && !self.options_open
                    && !self.about_open
            }
        }
    }

    pub(crate) fn run_row_command(
        &mut self,
        id: DownloadId,
        command: RowCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.row_command_enabled(id, command) {
            return;
        }
        match command {
            RowCommand::Details => self.open_details(id, window, cx),
            RowCommand::Start => {
                self.cancel_all_shutdowns(cx);
                self.start_item(id, cx);
                self.sync_table(cx);
                cx.notify();
            }
            RowCommand::Pause | RowCommand::Resume => self.toggle_pause(id, cx),
            RowCommand::Stop => self.cancel_id(id, cx),
            // Context retry never reuses old overwrite consent.
            RowCommand::Retry => self.request_retry(id, CollisionPolicy::Reject, window, cx),
            RowCommand::Reveal => cx.reveal_path(&self.queue.item(id).unwrap().destination),
            RowCommand::Remove => self.confirm_record_removal(vec![id], window, cx),
        }
    }

    pub(crate) fn row_context_menu(
        &self,
        id: DownloadId,
        mut menu: PopupMenu,
        window: &Window,
        owner: WeakEntity<Self>,
    ) -> PopupMenu {
        let paused = self
            .queue
            .item(id)
            .is_some_and(|item| matches!(item.state, Some(TransferState::Paused(_))));
        let commands = [
            (RowCommand::Details, "Details"),
            (RowCommand::Start, "Start download"),
            (
                if paused {
                    RowCommand::Resume
                } else {
                    RowCommand::Pause
                },
                if paused { "Resume" } else { "Pause" },
            ),
            (RowCommand::Stop, "Stop download"),
            (RowCommand::Retry, "Retry (do not overwrite)"),
            (RowCommand::Reveal, "Show in folder"),
            (RowCommand::Remove, "Remove…"),
        ];
        let handle = window.window_handle();
        for (command, label) in commands {
            if matches!(command, RowCommand::Retry | RowCommand::Remove) {
                menu = menu.separator();
            }
            let owner = owner.clone();
            menu = menu.item(
                PopupMenuItem::new(label)
                    .disabled(!self.row_command_enabled(id, command))
                    .on_click(move |_, _, cx| {
                        let owner = owner.clone();
                        // The delegate may still own TableState while opening its popup.
                        cx.defer(move |cx| {
                            let _ = handle.update(cx, |_, window, cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.run_row_command(id, command, window, cx)
                                });
                            });
                        });
                    }),
            );
        }
        menu
    }
}
