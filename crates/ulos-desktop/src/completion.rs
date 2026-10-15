//! Explicitly opted-in completion shutdown and a per-download, single-use gate.
//!
//! The parent must obtain confirmation before arming, call `on_success` only
//! after publishing the successful completion receipt, and dispatch `shutdown`
//! only after `take_due` returns true AND the queue has no unfinished work.
//! The parent owns the countdown UI and must cancel on failure, cancellation,
//! or closure of all application windows. A fresh gate is needed for each item;
//! a finished item cannot be armed retroactively. No OS command is run by a gate.

use std::time::{Duration, Instant};

/// Completion behavior; shutdown is never selected implicitly.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CompletionAction {
    #[default]
    None,
    Shutdown,
}

pub const SHUTDOWN_COUNTDOWN: Duration = Duration::from_secs(30);

/// A fixed command plan. Inspecting it does not execute or schedule shutdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShutdownCommand {
    pub executable: &'static str,
    pub arguments: &'static [&'static str],
}

/// Whether this build has an OS shutdown provider (not a permission guarantee).
pub fn shutdown_supported() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    ))
}

pub fn shutdown_command() -> Option<ShutdownCommand> {
    #[cfg(target_os = "macos")]
    {
        Some(ShutdownCommand {
            executable: "/usr/bin/osascript",
            arguments: &["-e", "tell application \"System Events\" to shut down"],
        })
    }
    #[cfg(target_os = "windows")]
    {
        Some(ShutdownCommand {
            executable: "shutdown.exe",
            arguments: &["/s", "/t", "0"],
        })
    }
    #[cfg(target_os = "linux")]
    {
        // Production calls this planner inside the blocking worker. Prefer the
        // distro's absolute path; otherwise trust the host's executable PATH.
        let executable = if std::path::Path::new("/usr/bin/systemctl").is_file() {
            "/usr/bin/systemctl"
        } else if std::path::Path::new("/bin/systemctl").is_file() {
            "/bin/systemctl"
        } else {
            "systemctl"
        };
        Some(ShutdownCommand {
            executable,
            arguments: &["poweroff"],
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

/// Ask the actual OS provider to shut down, without elevation or forced closure.
///
/// `Ok` means the provider exited successfully, not that poweroff was observed:
/// the OS may still block shutdown on open applications. Automation permission
/// on macOS and platform shutdown permissions are left entirely to the OS.
/// Permission prompts may keep this request pending. Once dispatched, cancelling
/// the awaiting future cannot revoke the OS request; cancel the gate beforehand.
pub async fn shutdown() -> Result<(), String> {
    tokio::task::spawn_blocking(|| {
        let plan = shutdown_command()
            .ok_or_else(|| "Shutdown is not supported on this platform.".to_owned())?;
        let output = std::process::Command::new(plan.executable)
            .args(plan.arguments)
            .output()
            .map_err(|_| "Could not launch the operating system shutdown provider.".to_owned())?;
        if output.status.success() {
            Ok(())
        } else {
            // Do not expose raw OS diagnostics, paths, control characters, or
            // potentially sensitive stderr. The exit status is safe to display.
            Err(format!(
                "The operating system shutdown provider rejected or failed the request ({}). Check system permissions and open applications.",
                output.status
            ))
        }
    })
    .await
    .map_err(|_| "The operating system shutdown worker failed.".to_owned())?
}

/// A single item's opt-in and countdown state; it never executes OS commands.
#[derive(Debug, Default)]
pub struct CompletionGate {
    armed: bool,
    completed: bool,
    deadline: Option<Instant>,
    consumed: bool,
}

impl CompletionGate {
    /// Arm before successful completion, after the parent's explicit confirmation.
    /// Returns whether shutdown is armed. Selecting `None` always cancels.
    pub fn arm(&mut self, action: CompletionAction) -> bool {
        if action == CompletionAction::None {
            self.cancel();
            return false;
        }
        if self.completed || self.consumed {
            return false;
        }
        self.armed = true;
        true
    }

    /// Record success exactly once and start the default countdown if armed.
    /// Unarmed success is still remembered, preventing retroactive shutdown.
    /// Returns true only when a new countdown was started.
    pub fn on_success(&mut self, now: Instant) -> bool {
        if self.completed {
            return false;
        }
        self.completed = true;
        if !self.armed || self.consumed {
            return false;
        }
        // An unrepresentable deadline must fail closed, never become immediate.
        self.deadline = now.checked_add(SHUTDOWN_COUNTDOWN);
        if self.deadline.is_none() {
            self.cancel();
            return false;
        }
        true
    }

    pub fn on_failure_or_cancel(&mut self) {
        self.cancel();
    }

    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        if !self.armed || self.consumed {
            return None;
        }
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    /// Consume the due request once. The parent must check queue idleness before
    /// calling this method; it can defer dispatch without resetting the deadline.
    pub fn take_due(&mut self, now: Instant) -> bool {
        if !self.armed || !self.completed || self.consumed {
            return false;
        }
        if !self.deadline.is_some_and(|deadline| now >= deadline) {
            return false;
        }
        self.consumed = true;
        self.cancel();
        true
    }

    /// Withdraw an undispatched request; does not erase the completion receipt.
    pub fn cancel(&mut self) {
        self.armed = false;
        self.deadline = None;
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests inspect plans and manipulate synthetic instants only. Never invoke
    // `shutdown`, spawn a provider process, sleep, or perform OS power actions.
    #[test]
    fn command_plan_matches_platform() {
        assert_eq!(CompletionAction::default(), CompletionAction::None);
        assert_eq!(shutdown_supported(), shutdown_command().is_some());
        #[cfg(target_os = "macos")]
        assert_eq!(
            shutdown_command(),
            Some(ShutdownCommand {
                executable: "/usr/bin/osascript",
                arguments: &["-e", "tell application \"System Events\" to shut down"],
            })
        );
        #[cfg(target_os = "windows")]
        assert_eq!(
            shutdown_command(),
            Some(ShutdownCommand {
                executable: "shutdown.exe",
                arguments: &["/s", "/t", "0"],
            })
        );
        #[cfg(target_os = "linux")]
        {
            let plan = shutdown_command().expect("Linux provider");
            assert!(matches!(
                plan.executable,
                "/usr/bin/systemctl" | "/bin/systemctl" | "systemctl"
            ));
            assert_eq!(plan.arguments, &["poweroff"]);
        }
    }

    #[test]
    fn only_armed_success_starts_countdown() {
        let now = Instant::now();
        let mut gate = CompletionGate::default();
        assert!(gate.arm(CompletionAction::Shutdown));
        assert!(gate.is_armed());
        assert_eq!(gate.remaining(now), None);
        assert!(!gate.take_due(now + SHUTDOWN_COUNTDOWN));
        assert!(gate.on_success(now));
        assert_eq!(gate.remaining(now), Some(SHUTDOWN_COUNTDOWN));
        assert!(!gate.take_due(now + Duration::from_secs(29)));
        assert!(gate.take_due(now + SHUTDOWN_COUNTDOWN));
    }

    #[test]
    fn unarmed_success_never_dispatches_or_retroactively_arms() {
        let now = Instant::now();
        let mut gate = CompletionGate::default();
        assert!(!gate.on_success(now));
        assert!(!gate.arm(CompletionAction::Shutdown));
        assert_eq!(gate.remaining(now), None);
        assert!(!gate.take_due(now + SHUTDOWN_COUNTDOWN));
    }

    #[test]
    fn duplicate_success_does_not_extend_countdown() {
        let now = Instant::now();
        let mut gate = CompletionGate::default();
        assert!(gate.arm(CompletionAction::Shutdown));
        assert!(gate.on_success(now));
        assert!(!gate.on_success(now + Duration::from_secs(10)));
        assert_eq!(
            gate.remaining(now + Duration::from_secs(10)),
            Some(Duration::from_secs(20))
        );
        assert!(gate.take_due(now + SHUTDOWN_COUNTDOWN));
    }

    #[test]
    fn cancellation_or_none_prevents_dispatch() {
        let now = Instant::now();
        for select_none in [false, true] {
            let mut gate = CompletionGate::default();
            assert!(gate.arm(CompletionAction::Shutdown));
            assert!(gate.on_success(now));
            if select_none {
                assert!(!gate.arm(CompletionAction::None));
            } else {
                gate.cancel();
            }
            assert!(!gate.is_armed());
            assert_eq!(gate.remaining(now), None);
            assert!(!gate.take_due(now + SHUTDOWN_COUNTDOWN));
            assert!(!gate.arm(CompletionAction::Shutdown));
        }
    }

    #[test]
    fn failure_disarms_before_or_during_countdown() {
        let now = Instant::now();
        for already_succeeded in [false, true] {
            let mut gate = CompletionGate::default();
            assert!(gate.arm(CompletionAction::Shutdown));
            if already_succeeded {
                assert!(gate.on_success(now));
            }
            gate.on_failure_or_cancel();
            assert!(!gate.is_armed());
            assert_eq!(gate.remaining(now), None);
            assert!(!gate.on_success(now));
            assert!(!gate.take_due(now + SHUTDOWN_COUNTDOWN));
        }
    }

    #[test]
    fn due_request_is_consumed_once_and_finished_gate_cannot_rearm() {
        let now = Instant::now();
        let mut gate = CompletionGate::default();
        assert!(gate.arm(CompletionAction::Shutdown));
        assert!(gate.on_success(now));
        let due = now + SHUTDOWN_COUNTDOWN;
        assert_eq!(gate.remaining(due), Some(Duration::ZERO));
        assert!(gate.take_due(due));
        assert!(!gate.take_due(due));
        assert!(!gate.take_due(due + SHUTDOWN_COUNTDOWN));
        assert!(!gate.on_success(due));
        assert!(!gate.arm(CompletionAction::Shutdown));
        assert_eq!(gate.remaining(due), None);
        assert!(!gate.is_armed());
    }
}
