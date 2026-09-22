//! Writing a file so that a reboot cannot find half of it.
//!
//! Every file bitplane owns goes through [`write_atomically`]: a temp file **in
//! the target's own directory**, `fsync`'d, renamed, and then the parent
//! directory `fsync`'d (ADR-0002).
//!
//! Each step earns its place. The temp file is a sibling because `rename(2)`
//! across filesystems fails `EXDEV`, and `~/planes` on its own volume is
//! ordinary. The `fsync` is because `rename(2)` is atomic with respect to other
//! *processes* and says nothing about power loss — without it you can reboot
//! into a `plane.toml` that is present and zero bytes, which with no store means
//! a plane full of real worktrees that `bp` can no longer see. The parent
//! `fsync` is what makes the rename itself durable.
//!
//! `F_FULLFSYNC` is deliberately not used on macOS: it forces a full drive cache
//! flush at tens of milliseconds, and the residual power-loss window is one a
//! developer tool accepts.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::EngineError;

/// Writes `contents` to `target`, atomically and durably.
pub fn write_atomically(target: &Path, contents: &str) -> Result<(), EngineError> {
    let directory = target.parent().ok_or_else(|| EngineError::Io {
        path: target.to_path_buf(),
        message: "has no directory to write into".to_owned(),
    })?;
    let temporary = temporary_sibling(target)?;

    let written = write_and_sync(&temporary, contents)
        .and_then(|()| rename(&temporary, target))
        .and_then(|()| sync_directory(directory));

    if written.is_err() {
        // Best effort: the caller is already failing, and a leftover dotfile
        // beside the target is worse than nothing to report.
        let _ = fs::remove_file(&temporary);
    }

    written
}

/// `fsync`s a directory, so a rename or a `mkdir` into it survives a crash.
pub fn sync_directory(directory: &Path) -> Result<(), EngineError> {
    File::open(directory)
        .and_then(|handle| handle.sync_all())
        .map_err(|err| EngineError::io(directory, err))
}

fn write_and_sync(temporary: &Path, contents: &str) -> Result<(), EngineError> {
    let mut handle = File::create(temporary).map_err(|err| EngineError::io(temporary, err))?;

    handle
        .write_all(contents.as_bytes())
        .and_then(|()| handle.sync_all())
        .map_err(|err| EngineError::io(temporary, err))
}

fn rename(temporary: &Path, target: &Path) -> Result<(), EngineError> {
    fs::rename(temporary, target).map_err(|err| EngineError::io(target, err))
}

/// A name in the target's own directory that no concurrent writer will pick.
/// Hidden, so an interrupted write between `create` and `rename` leaves nothing
/// a `ls` of a worktree's parent will show.
fn temporary_sibling(target: &Path) -> Result<PathBuf, EngineError> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let directory = target.parent().ok_or_else(|| EngineError::Io {
        path: target.to_path_buf(),
        message: "has no directory to write into".to_owned(),
    })?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| EngineError::Io {
            path: target.to_path_buf(),
            message: "is not a file name bitplane can write".to_owned(),
        })?;

    Ok(directory.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_dir;

    #[test]
    fn the_contents_land_at_the_target() {
        let dir = scratch_dir("fsio-write");
        let target = dir.join("plane.toml");

        write_atomically(&target, "version = 1\n").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "version = 1\n");
    }

    #[test]
    fn a_rewrite_replaces_the_previous_contents_and_leaves_no_temp_file() {
        let dir = scratch_dir("fsio-rewrite");
        let target = dir.join("plane.toml");

        write_atomically(&target, "first").unwrap();
        write_atomically(&target, "second").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "second");
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "the temp file must not survive the write"
        );
    }

    #[test]
    fn a_target_whose_directory_does_not_exist_is_an_io_failure_naming_it() {
        let dir = scratch_dir("fsio-missing");
        let target = dir.join("nowhere").join("plane.toml");

        let error = write_atomically(&target, "version = 1\n").unwrap_err();

        assert!(matches!(error, EngineError::Io { .. }), "got {error:?}");
        assert_eq!(error.exit_code(), crate::ExitCode::Failure);
    }
}
