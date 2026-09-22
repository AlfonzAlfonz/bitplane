//! The git prerequisite, end to end against a real `git` on `PATH`.
//!
//! ADR-0001 sets the floor at 2.36 (`git worktree list --porcelain -z`) and says
//! the version is checked once on first invocation, with no runtime capability
//! detection. These tests drive the real `SystemGit` probe against stub
//! executables, so the parsing, the floor and the memoisation are all exercised
//! through the path the binary actually takes.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use bitplane_core::testing::scratch_dir;
use bitplane_core::{
    Directories, EngineError, ExitCode, GitVersion, LocalEngine, MINIMUM_GIT_VERSION,
};

#[test]
fn a_supported_git_passes_and_reports_its_version() {
    let dir = stub_git("2.36.0", "echo 'git version 2.36.0'");

    let engine = engine_with_git_at(&dir);

    assert_eq!(
        engine.ensure_git_supported().unwrap(),
        GitVersion::new(2, 36, 0)
    );
}

#[test]
fn a_git_below_the_floor_fails_with_the_prerequisite_exit_code() {
    let dir = stub_git("2.35.9", "echo 'git version 2.35.9'");

    let engine = engine_with_git_at(&dir);
    let error = engine.ensure_git_supported().unwrap_err();

    assert_eq!(
        error,
        EngineError::GitTooOld {
            found: GitVersion::new(2, 35, 9),
            required: MINIMUM_GIT_VERSION,
        }
    );

    let envelope = error.envelope();
    assert_eq!(envelope.code, ExitCode::PrerequisiteMissing);
    assert!(
        envelope.message.contains("2.36"),
        "the message must name the requirement, got: {}",
        envelope.message
    );
}

#[test]
fn a_missing_git_is_a_prerequisite_failure_not_a_crash() {
    let dir = empty_path_dir("no-git");

    let engine = engine_with_git_at(&dir);

    assert_eq!(
        engine.ensure_git_supported().unwrap_err(),
        EngineError::GitMissing
    );
}

#[test]
fn a_git_that_will_not_run_is_a_prerequisite_failure() {
    let dir = stub_git("broken", "echo 'boom' >&2; exit 1");

    let engine = engine_with_git_at(&dir);
    let error = engine.ensure_git_supported().unwrap_err();

    assert!(
        matches!(error, EngineError::GitUnusable { .. }),
        "expected GitUnusable, got {error:?}"
    );
    assert_eq!(error.envelope().code, ExitCode::PrerequisiteMissing);
}

#[test]
fn an_unintelligible_version_line_is_a_prerequisite_failure() {
    let dir = stub_git("gibberish", "echo 'not a version at all'");

    let engine = engine_with_git_at(&dir);
    let error = engine.ensure_git_supported().unwrap_err();

    assert_eq!(
        error,
        EngineError::GitVersionUnreadable {
            reported: "not a version at all".to_owned()
        }
    );
    assert_eq!(error.envelope().code, ExitCode::PrerequisiteMissing);
}

#[test]
fn the_check_runs_once_however_many_times_it_is_asked() {
    let dir = stub_git(
        "counting",
        // Append a line per invocation, so the file's length is the call count.
        "echo call >> \"${0%/*}/calls\"; echo 'git version 2.40.0'",
    );

    let engine = engine_with_git_at(&dir);
    for _ in 0..5 {
        engine.ensure_git_supported().expect("2.40.0 is supported");
    }

    let calls = fs::read_to_string(dir.join("calls")).unwrap_or_default();
    assert_eq!(calls.lines().count(), 1, "git was probed more than once");
}

/// Builds an engine whose `PATH` contains only `dir`, so the stub is the only
/// git it can find.
fn engine_with_git_at(dir: &Path) -> LocalEngine {
    let host = scratch_dir("git-preflight-host");

    LocalEngine::new(Directories::new(host.join("planes"), host.join("projects")))
        .with_git_search_path(dir)
}

/// Writes an executable `git` at a fresh directory, whose body is `body`.
fn stub_git(label: &str, body: &str) -> PathBuf {
    let dir = empty_path_dir(label);
    let git = dir.join("git");
    fs::write(&git, format!("#!/bin/sh\n{body}\n")).expect("write stub git");
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).expect("chmod stub git");
    dir
}

/// A directory this run owns outright. Created rather than cleared, so an
/// existing path — including one a symlink points at — is an error rather than
/// somewhere the stub `git` gets written and then executed.
fn empty_path_dir(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let root = std::env::temp_dir().join("bitplane-git-preflight");
    fs::create_dir_all(&root).expect("create the stub root");

    let dir = root.join(format!(
        "{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir(&dir).expect("the stub dir must not already exist");
    dir
}
