//! bitplane's one runtime dependency.
//!
//! The "single self-contained file" story has one asterisk: bitplane requires
//! git on `PATH`, and not just any git. `git worktree list --porcelain -z` —
//! the only form that handles paths and lock reasons containing unsafe bytes
//! correctly — landed in 2.36, so that is the floor (ADR-0001).
//!
//! The version is checked **once**, on the first invocation, and every command
//! fails with a message naming the requirement below it. There is no runtime
//! capability detection: degrading around a missing orphan oracle means two
//! code paths for a situation nobody is in.

use std::fmt;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::EngineError;

/// The oldest git bitplane runs against (ADR-0001).
pub const MINIMUM_GIT_VERSION: GitVersion = GitVersion::new(2, 36, 0);

/// A git version, compared as `(major, minor, patch)` — which is the field
/// order, because that is what the derived ordering uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// Asks the git on `PATH` what version it is. A trait so the check can be
/// driven without a git installed.
pub trait GitProbe: Send + Sync {
    /// git's raw `git --version` output.
    fn report_version(&self) -> Result<String, ProbeFailure>;
}

/// Why git could not be asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeFailure {
    /// There is no git to ask.
    NotFound,
    /// There is a git, and it would not answer.
    Failed { message: String },
}

/// The real probe: the git on `PATH`.
#[derive(Debug, Clone, Default)]
pub struct SystemGit {
    /// The `PATH` to look for git on. `None` inherits the process's own.
    search_path: Option<PathBuf>,
}

/// The git version check, memoised: probed on first use and never again
/// (ADR-0001).
#[derive(Debug, Default)]
pub struct GitPrerequisite<P: GitProbe = SystemGit> {
    probe: P,
    checked: OnceLock<Result<GitVersion, EngineError>>,
}

impl GitVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> GitVersion {
        GitVersion {
            major,
            minor,
            patch,
        }
    }

    /// Reads a version out of a whole `git --version` line, which is
    /// `git version 2.50.1 (Apple Git-155)` on macOS and
    /// `git version 2.44.0.windows.1` under Git for Windows.
    pub fn from_version_line(line: &str) -> Option<GitVersion> {
        let version = line.trim().strip_prefix("git version ")?;
        let version = version.split_whitespace().next()?;
        version.parse().ok()
    }

    /// Whether this git can run bitplane.
    pub fn is_supported(self) -> bool {
        self >= MINIMUM_GIT_VERSION
    }
}

impl SystemGit {
    /// Looks for git on the process's own `PATH`.
    pub fn new() -> SystemGit {
        SystemGit { search_path: None }
    }

    /// Looks for git on `search_path` instead of the process's own — the seam
    /// tests use to drive the check without touching the ambient environment.
    pub fn on_path(search_path: impl Into<PathBuf>) -> SystemGit {
        SystemGit {
            search_path: Some(search_path.into()),
        }
    }
}

impl GitProbe for SystemGit {
    fn report_version(&self) -> Result<String, ProbeFailure> {
        let mut command = Command::new("git");
        command.arg("--version");
        if let Some(search_path) = &self.search_path {
            command.env("PATH", search_path);
        }

        let output = command.output().map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => ProbeFailure::NotFound,
            _ => ProbeFailure::Failed {
                message: err.to_string(),
            },
        })?;

        if !output.status.success() {
            return Err(ProbeFailure::Failed {
                message: describe_exit(&output),
            });
        }

        String::from_utf8(output.stdout).map_err(|_| ProbeFailure::Failed {
            message: "`git --version` printed something that is not UTF-8".to_owned(),
        })
    }
}

impl<P: GitProbe> GitPrerequisite<P> {
    pub fn new(probe: P) -> GitPrerequisite<P> {
        GitPrerequisite {
            probe,
            checked: OnceLock::new(),
        }
    }

    /// The version check. Runs the probe on the first call and returns the same
    /// answer to every call after it.
    pub fn ensure_supported(&self) -> Result<GitVersion, EngineError> {
        self.checked.get_or_init(|| self.check()).clone()
    }

    fn check(&self) -> Result<GitVersion, EngineError> {
        let reported = self
            .probe
            .report_version()
            .map_err(|failure| match failure {
                ProbeFailure::NotFound => EngineError::GitMissing,
                ProbeFailure::Failed { message } => EngineError::GitUnusable { message },
            })?;

        let found = GitVersion::from_version_line(&reported).ok_or_else(|| {
            EngineError::GitVersionUnreadable {
                reported: reported.trim().to_owned(),
            }
        })?;

        if found.is_supported() {
            Ok(found)
        } else {
            Err(EngineError::GitTooOld {
                found,
                required: MINIMUM_GIT_VERSION,
            })
        }
    }
}

impl GitVersion {
    /// How a message names this version.
    ///
    /// A zero patch is noise: git itself, ADR-0001 and every page of the
    /// reference call the floor **2.36**, so printing `2.36.0` at a user would
    /// be bitplane spelling the requirement differently from the document that
    /// states it. [`fmt::Display`] keeps all three components, because that is
    /// also the wire form.
    pub fn spoken(&self) -> String {
        match self.patch {
            0 => format!("{}.{}", self.major, self.minor),
            patch => format!("{}.{}.{patch}", self.major, self.minor),
        }
    }
}

impl fmt::Display for GitVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for GitVersion {
    type Err = UnreadableGitVersion;

    /// Parses `2.36`, `2.36.0`, `2.44.0.windows.1` and `2.45.GIT` alike.
    ///
    /// Major and minor must be numbers — they are what the floor is compared
    /// on. The patch need not be: a git built from source reports `2.45.GIT`,
    /// and refusing that would fail a git well above the floor. Components past
    /// the patch are the packager's and carry no ordering.
    fn from_str(version: &str) -> Result<GitVersion, UnreadableGitVersion> {
        let mut components = version.split('.');

        let major = number(components.next())?.ok_or(UnreadableGitVersion)?;
        let minor = number(components.next())?.unwrap_or(0);
        let patch = number(components.next()).unwrap_or(None).unwrap_or(0);

        Ok(GitVersion::new(major, minor, patch))
    }
}

/// A string that is not a git version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnreadableGitVersion;

impl fmt::Display for UnreadableGitVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a git version")
    }
}

impl std::error::Error for UnreadableGitVersion {}

impl From<GitVersion> for String {
    fn from(version: GitVersion) -> String {
        version.to_string()
    }
}

impl TryFrom<String> for GitVersion {
    type Error = UnreadableGitVersion;

    fn try_from(version: String) -> Result<GitVersion, UnreadableGitVersion> {
        version.parse()
    }
}

/// A missing component is `None`; a present one that is not a number is not a
/// version component at all.
fn number(component: Option<&str>) -> Result<Option<u32>, UnreadableGitVersion> {
    match component {
        None => Ok(None),
        Some(text) => text.parse().map(Some).map_err(|_| UnreadableGitVersion),
    }
}

fn describe_exit(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    match (output.status.code(), stderr.is_empty()) {
        (Some(code), true) => format!("`git --version` exited {code}"),
        (Some(code), false) => format!("`git --version` exited {code}: {stderr}"),
        (None, true) => "`git --version` was killed by a signal".to_owned(),
        (None, false) => format!("`git --version` was killed by a signal: {stderr}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_version_line_is_read_in_every_shape_git_prints_it() {
        let cases = [
            ("git version 2.36.0", GitVersion::new(2, 36, 0)),
            (
                "git version 2.50.1 (Apple Git-155)",
                GitVersion::new(2, 50, 1),
            ),
            ("git version 2.44.0.windows.1", GitVersion::new(2, 44, 0)),
            ("git version 2.39\n", GitVersion::new(2, 39, 0)),
            // A git built from source names its patch after the tag it came from.
            ("git version 2.45.GIT", GitVersion::new(2, 45, 0)),
            ("git version 2.51.0.rc1", GitVersion::new(2, 51, 0)),
            ("  git version 2.36.0  \n", GitVersion::new(2, 36, 0)),
        ];

        for (line, expected) in cases {
            assert_eq!(
                GitVersion::from_version_line(line),
                Some(expected),
                "for {line:?}"
            );
        }
    }

    #[test]
    fn a_line_that_is_not_a_version_is_refused() {
        let cases = [
            "",
            "not a version at all",
            "git version",
            "git version x.y.z",
            "git version 2.x.y",
            "git version GIT",
            "2.36.0",
        ];

        for line in cases {
            assert_eq!(GitVersion::from_version_line(line), None, "for {line:?}");
        }
    }

    #[test]
    fn the_floor_is_2_36_and_it_is_inclusive() {
        assert!(!GitVersion::new(2, 35, 9).is_supported());
        assert!(!GitVersion::new(1, 99, 0).is_supported());
        assert!(GitVersion::new(2, 36, 0).is_supported());
        assert!(GitVersion::new(2, 36, 1).is_supported());
        assert!(GitVersion::new(3, 0, 0).is_supported());
    }

    #[test]
    fn versions_order_by_major_then_minor_then_patch() {
        assert!(GitVersion::new(2, 9, 0) < GitVersion::new(2, 36, 0));
        assert!(GitVersion::new(2, 36, 0) < GitVersion::new(10, 0, 0));
    }

    #[test]
    fn the_probe_runs_once_however_many_times_the_check_is_asked() {
        let prerequisite = GitPrerequisite::new(CountingProbe::reporting("git version 2.40.0"));

        for _ in 0..5 {
            assert_eq!(
                prerequisite.ensure_supported().unwrap(),
                GitVersion::new(2, 40, 0)
            );
        }

        assert_eq!(prerequisite.probe.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_failing_check_is_remembered_too() {
        let prerequisite = GitPrerequisite::new(CountingProbe::reporting("git version 2.35.0"));

        for _ in 0..3 {
            assert_eq!(
                prerequisite.ensure_supported().unwrap_err(),
                EngineError::GitTooOld {
                    found: GitVersion::new(2, 35, 0),
                    required: MINIMUM_GIT_VERSION,
                }
            );
        }

        assert_eq!(prerequisite.probe.calls.load(Ordering::SeqCst), 1);
    }

    struct CountingProbe {
        reported: String,
        calls: AtomicUsize,
    }

    impl CountingProbe {
        fn reporting(reported: &str) -> CountingProbe {
            CountingProbe {
                reported: reported.to_owned(),
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl GitProbe for CountingProbe {
        fn report_version(&self) -> Result<String, ProbeFailure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.reported.clone())
        }
    }
}
