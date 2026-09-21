//! The engine that answers for this machine.

use std::path::PathBuf;

use crate::engine::{Engine, Reader};
use crate::error::EngineError;
use crate::git::{GitPrerequisite, GitVersion, SystemGit};

/// bitplane on the local host.
///
/// Holds the git version check, which is why it is constructed once per process
/// and shared: the check runs on first use and never again (ADR-0001).
#[derive(Debug, Default)]
pub struct LocalEngine {
    git: GitPrerequisite<SystemGit>,
}

impl LocalEngine {
    /// Uses the git on the process's own `PATH`.
    pub fn new() -> LocalEngine {
        LocalEngine {
            git: GitPrerequisite::new(SystemGit::new()),
        }
    }

    /// Uses the git on `search_path` instead of the process's own.
    pub fn with_git_search_path(search_path: impl Into<PathBuf>) -> LocalEngine {
        LocalEngine {
            git: GitPrerequisite::new(SystemGit::on_path(search_path)),
        }
    }

    /// Fails unless there is a git new enough to run bitplane. Every command
    /// goes through here before it touches anything.
    pub fn ensure_git_supported(&self) -> Result<GitVersion, EngineError> {
        self.git.ensure_supported()
    }
}

impl Reader for LocalEngine {}

impl Engine for LocalEngine {}
