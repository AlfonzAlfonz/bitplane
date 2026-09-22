//! The sentinel a plane is locked on.
//!
//! **Lock a sentinel, never the data file.** Atomic rename *replaces the
//! inode*, so a process holding a lock on `plane.toml` holds it on an inode the
//! writer has already swapped out: a second writer wakes holding a lock on a
//! file that no longer exists and clobbers the first (ADR-0002).
//!
//! `flock(2)` rather than an `O_EXCL` pid file, because the kernel releases it
//! when the fd closes — **including on `SIGKILL`**. There is no stale lock to
//! probe for, no pid reuse to handle and no `--force-unlock`.
//!
//! ADR-0002 budgeted a crate for this (`fs4` or `fd-lock`). None is needed:
//! `File::try_lock` and `File::unlock` are `std` as of Rust 1.89, well under the
//! pinned toolchain, so the dependency is declined and the decision is otherwise
//! unchanged.
//!
//! Acquisition tries non-blocking first, so the common case is silent, then
//! blocks up to [`DEFAULT_TIMEOUT`] — a cold fetch of a large repo genuinely
//! takes that long — and then fails naming the **object**, because `flock` does
//! not identify its holder and a pid would be a guess.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::EngineError;

/// How long a contended lock is waited for before it is a [`EngineError::LockTimeout`].
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// The name of the sentinel inside a plane's `.bitplane/`.
pub const SENTINEL_NAME: &str = "lock";

/// How often the wait re-tries. `flock`'s blocking mode has no timeout, so the
/// wait is a poll; the interval only has to be short against human patience.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// An exclusive lock, held until it is dropped.
#[derive(Debug)]
pub struct Lock {
    object: PathBuf,
    handle: File,
}

impl Lock {
    /// Takes the lock on `sentinel`, creating the sentinel file if it is not
    /// there.
    ///
    /// `on_contended` runs once, only if the first non-blocking attempt fails,
    /// so a caller can say `waiting for …` without the common case printing
    /// anything.
    pub fn acquire(
        sentinel: &Path,
        timeout: Duration,
        on_contended: impl FnOnce(&Path),
    ) -> Result<Lock, EngineError> {
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(sentinel)
            .map_err(|err| EngineError::io(sentinel, err))?;

        if taken(&handle, sentinel)? {
            return Ok(Lock {
                object: sentinel.to_path_buf(),
                handle,
            });
        }

        on_contended(sentinel);

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            std::thread::sleep(POLL_INTERVAL);
            if taken(&handle, sentinel)? {
                return Ok(Lock {
                    object: sentinel.to_path_buf(),
                    handle,
                });
            }
        }

        Err(EngineError::LockTimeout {
            object: sentinel.to_path_buf(),
        })
    }

    /// What is locked. Named in the failure when someone else wants it.
    pub fn object(&self) -> &Path {
        &self.object
    }
}

/// Released by the kernel when the fd closes, so this is belt and braces —
/// but it releases the lock at the end of the block rather than at the end of
/// the process, which matters for a run that holds two in sequence.
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.handle.unlock();
    }
}

fn taken(handle: &File, sentinel: &Path) -> Result<bool, EngineError> {
    match handle.try_lock() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(err)) => Err(EngineError::io(sentinel, err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_dir;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn an_uncontended_lock_is_taken_without_announcing_a_wait() {
        let dir = scratch_dir("lock-uncontended");
        let sentinel = dir.join("lock");
        let announced = AtomicBool::new(false);

        let lock = Lock::acquire(&sentinel, DEFAULT_TIMEOUT, |_| {
            announced.store(true, Ordering::SeqCst)
        })
        .unwrap();

        assert_eq!(lock.object(), sentinel);
        assert!(
            !announced.load(Ordering::SeqCst),
            "the common case is silent"
        );
    }

    #[test]
    fn a_contended_lock_announces_the_wait_and_times_out_naming_the_object() {
        let dir = scratch_dir("lock-contended");
        let sentinel = dir.join("lock");
        let announced = AtomicBool::new(false);

        let _held = Lock::acquire(&sentinel, DEFAULT_TIMEOUT, |_| {}).unwrap();

        let error = Lock::acquire(&sentinel, Duration::from_millis(120), |_| {
            announced.store(true, Ordering::SeqCst)
        })
        .unwrap_err();

        assert!(announced.load(Ordering::SeqCst), "a wait must be announced");
        assert_eq!(
            error,
            EngineError::LockTimeout {
                object: sentinel.clone()
            }
        );
        assert_eq!(error.exit_code(), crate::ExitCode::Busy);
    }

    #[test]
    fn a_released_lock_can_be_taken_again() {
        let dir = scratch_dir("lock-released");
        let sentinel = dir.join("lock");

        drop(Lock::acquire(&sentinel, DEFAULT_TIMEOUT, |_| {}).unwrap());

        assert!(Lock::acquire(&sentinel, Duration::from_millis(120), |_| {}).is_ok());
    }
}
