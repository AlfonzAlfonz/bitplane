//! Scratch directories and git fixtures, shared by the unit tests in this
//! crate and the integration tests beside it.
//!
//! Compiled into the library rather than duplicated per test binary, because
//! the fixtures build real repositories and getting that subtly different in
//! two places is how a test suite starts lying. It is `#[doc(hidden)]`: it is
//! test support, not contract.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A directory this run owns outright, named after `label`.
///
/// Created rather than cleared, so an existing path — including one a symlink
/// points at — is an error rather than somewhere a test writes and then
/// executes.
pub fn scratch_dir(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let root = std::env::temp_dir().join("bitplane-tests");
    fs::create_dir_all(&root).expect("create the scratch root");

    let dir = root.join(format!(
        "{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir(&dir).expect("the scratch dir must not already exist");
    dir
}

/// A real git repository with one commit on `main`, at `path`.
///
/// Real rather than stubbed: every worktree operation bitplane performs is a
/// `git` subprocess, so a fake repository would only test bitplane's idea of
/// git.
pub fn repository_with_one_commit(path: &Path) -> PathBuf {
    fs::create_dir_all(path).expect("create the repository directory");

    git(path, &["init", "--initial-branch", "main", "--quiet"]);
    git(path, &["config", "user.name", "bitplane tests"]);
    git(path, &["config", "user.email", "tests@bitplane.invalid"]);
    git(path, &["config", "commit.gpgsign", "false"]);
    fs::write(path.join("README.md"), "fixture\n").expect("write the fixture file");
    git(path, &["add", "README.md"]);
    git(path, &["commit", "--quiet", "--message", "fixture"]);

    path.to_path_buf()
}

/// Runs a git command in `repository`, panicking with git's own words if it
/// fails — a broken fixture must not read as a bitplane bug.
pub fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("run git {args:?}: {err}"));

    assert!(
        output.status.success(),
        "git {args:?} in {} failed: {}",
        repository.display(),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("git printed utf-8")
}
