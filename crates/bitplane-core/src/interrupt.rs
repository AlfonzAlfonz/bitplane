//! Ctrl-C, as something the engine can read.
//!
//! bitplane cannot protect its children: SIGINT reaches the whole process
//! group, so an in-flight `git worktree add` gets it too. All bitplane controls
//! is what it records and prints (ADR-0004) — so the first interrupt stops the
//! fan-out *scheduling* new members, lets the children already running land,
//! unwinds, and prints the partial vector. A second exits immediately.
//!
//! An [`Interrupt`] is a handle onto a counter rather than a flag, because
//! "again" is the whole of the second-interrupt rule. It borrows a `'static`
//! counter rather than owning an `Arc` so that a signal handler — which can
//! capture nothing — and the engine can share one, and so that a test can hold
//! a counter of its own instead of racing every other test in the binary.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The counter the process's own SIGINT handler raises.
static PROCESS: AtomicUsize = AtomicUsize::new(0);

/// How many times the run has been interrupted.
#[derive(Debug, Clone, Copy)]
pub struct Interrupt {
    raised: &'static AtomicUsize,
}

impl Interrupt {
    /// The process-wide interrupt, raised by [`raise_process`].
    pub fn process() -> Interrupt {
        Interrupt { raised: &PROCESS }
    }

    /// An interrupt on a counter of the caller's own.
    pub const fn on(raised: &'static AtomicUsize) -> Interrupt {
        Interrupt { raised }
    }

    /// One that is never raised, for a call that has no interrupt to honour.
    pub fn never() -> Interrupt {
        static NEVER: AtomicUsize = AtomicUsize::new(0);
        Interrupt { raised: &NEVER }
    }

    /// Raises it, as the signal handler does.
    pub fn raise(self) {
        self.raised.fetch_add(1, Ordering::SeqCst);
    }

    /// Whether the run has been asked to stop.
    pub fn is_raised(self) -> bool {
        self.count() > 0
    }

    /// How many times. Two or more is "exit now, leave what is in flight".
    pub fn count(self) -> usize {
        self.raised.load(Ordering::SeqCst)
    }
}

/// Raises the process-wide interrupt, and reports how many had come before.
///
/// Async-signal-safe — an atomic read-modify-write and nothing else — so it is
/// callable from a real SIGINT handler, which is the only caller that matters.
pub fn raise_process() -> usize {
    PROCESS.fetch_add(1, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interrupt_starts_unraised_and_counts_up() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let interrupt = Interrupt::on(&COUNTER);

        assert!(!interrupt.is_raised());

        interrupt.raise();
        assert!(interrupt.is_raised());
        assert_eq!(interrupt.count(), 1);

        interrupt.raise();
        assert_eq!(interrupt.count(), 2, "a second Ctrl-C is a different thing");
    }

    #[test]
    fn never_is_never_raised() {
        assert!(!Interrupt::never().is_raised());
    }
}
