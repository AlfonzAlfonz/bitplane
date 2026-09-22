//! Ctrl-C, installed.
//!
//! **The first interrupt** raises the process-wide flag the engine reads, and
//! returns — so the fan-out stops scheduling, the children already running are
//! left to land, the abort path runs, the rows go to stdout and `bp` exits
//! `130`. Those rows are not a courtesy; they are the repair instruction.
//!
//! **The second exits immediately**, leaving whatever was in flight where it
//! fell. It cannot be a graceful unwind: the user has already asked twice.
//!
//! The handler does two things and both are async-signal-safe — an atomic
//! read-modify-write, and `_exit(2)`. Nothing is allocated, locked or printed
//! from inside it.

use bitplane_core::interrupt;

/// The shell's convention: `128 + SIGINT`.
const INTERRUPTED: libc::c_int = 130;

/// Installs the SIGINT handler for this process.
pub fn listen_for_ctrl_c() {
    // SAFETY: `on_interrupt` is async-signal-safe, and `signal(2)` with a
    // function pointer is the whole of the interaction.
    unsafe {
        libc::signal(
            libc::SIGINT,
            on_interrupt as *const () as libc::sighandler_t,
        );
    }
}

extern "C" fn on_interrupt(_signal: libc::c_int) {
    if interrupt::raise_process() > 0 {
        // SAFETY: `_exit(2)` is async-signal-safe by definition; unlike
        // `exit(3)` it runs no atexit handlers and flushes no buffers, which is
        // exactly what "leave whatever was in flight where it fell" means.
        unsafe { libc::_exit(INTERRUPTED) }
    }
}
