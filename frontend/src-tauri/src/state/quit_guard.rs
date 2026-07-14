//! Quit-during-write state machine, separated from GUI presentation for tests.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct QuitGuard {
    force_exit: AtomicBool,
    dialog_open: AtomicBool,
}

impl QuitGuard {
    /// Whether this exit request must be prevented and presented to the user.
    pub fn should_prompt(&self, write_active: bool) -> bool {
        write_active && !self.force_exit.load(Ordering::Acquire)
    }

    /// Claim presentation ownership. False coalesces repeated exit requests.
    pub fn begin_dialog(&self) -> bool {
        self.dialog_open
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Finish one prompt. Force exit bypasses the guard exactly once for the
    /// re-requested exit; cancel permits a future prompt.
    pub fn finish_dialog(&self, force_exit: bool) {
        if force_exit {
            self.force_exit.store(true, Ordering::Release);
        }
        self.dialog_open.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_only_for_active_writes_and_coalesces_dialogs() {
        let guard = QuitGuard::default();
        assert!(!guard.should_prompt(false));
        assert!(guard.should_prompt(true));
        assert!(guard.begin_dialog());
        assert!(!guard.begin_dialog());
        guard.finish_dialog(false);
        assert!(guard.begin_dialog());
    }

    #[test]
    fn force_exit_bypasses_the_re_requested_exit() {
        let guard = QuitGuard::default();
        assert!(guard.should_prompt(true));
        guard.finish_dialog(true);
        assert!(!guard.should_prompt(true));
    }
}
