//! A worktree's live branch, read without starting git.
//!
//! `bp list` runs constantly and must not spawn a git per member (ADR-0003), so
//! the branch comes from the one place that answers with a single small read: a
//! linked worktree's `.git` **file** points at
//! `<source-repo>/worktrees/<name>/`, and the `HEAD` in there is one line.
//!
//! This is a read of git's own data in git's own format, not a reimplementation
//! of git: the two shapes `HEAD` can hold are a symbolic ref and a raw object
//! id, and both have been stable for the whole life of the format.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How much of an object id a detached `HEAD` is reported by.
const ABBREVIATED_COMMIT: usize = 7;

/// What a worktree's `HEAD` names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "head", rename_all = "snake_case")]
pub enum Head {
    /// The ordinary case: a branch.
    Branch { branch: String },
    /// A detached `HEAD`, which is **ordinary use** and never drift: bitplane
    /// owns a worktree's existence and location, and its `HEAD` is the user's.
    Detached { commit: String },
}

impl Head {
    /// Reads the `HEAD` of the worktree at `path`.
    ///
    /// `None` where there is nothing to read — the directory is gone, or it is
    /// not a worktree — which the health scan reports as a finding in its own
    /// words rather than being guessed at here.
    pub fn read(worktree: &Path) -> Option<Head> {
        let git = worktree.join(".git");

        // A linked worktree's `.git` is a file; a plain checkout's is the
        // directory itself. Both are read the same way from here.
        let directory = if git.is_dir() {
            git
        } else {
            gitdir_named_in(&git)?
        };

        Head::parse(&fs::read_to_string(directory.join("HEAD")).ok()?)
    }

    /// Reads the contents of a `HEAD` file.
    pub fn parse(text: &str) -> Option<Head> {
        let text = text.trim();

        if let Some(reference) = text.strip_prefix("ref:") {
            // A symbolic ref outside `refs/heads/` — a tag, or something a tool
            // wrote — is not a branch, and guessing one from it would be worse
            // than saying nothing.
            return reference
                .trim()
                .strip_prefix("refs/heads/")
                .filter(|branch| !branch.is_empty())
                .map(|branch| Head::Branch {
                    branch: branch.to_owned(),
                });
        }

        (!text.is_empty()).then(|| Head::Detached {
            commit: text.to_owned(),
        })
    }

    /// The branch, where this `HEAD` is on one.
    pub fn branch(&self) -> Option<&str> {
        match self {
            Head::Branch { branch } => Some(branch),
            Head::Detached { .. } => None,
        }
    }
}

/// Rendered identically wherever a branch column is printed, so `bp list`,
/// `bp show` and `bp status` never disagree about what a worktree is on.
impl fmt::Display for Head {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Head::Branch { branch } => f.write_str(branch),
            Head::Detached { commit } => {
                let abbreviated: String = commit.chars().take(ABBREVIATED_COMMIT).collect();
                write!(f, "(detached at {abbreviated})")
            }
        }
    }
}

/// The directory a linked worktree's `.git` file points at.
///
/// git writes the path relative to the worktree when it can, so it is resolved
/// against the worktree rather than the process's own directory.
fn gitdir_named_in(pointer: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(pointer).ok()?;
    let named = Path::new(text.trim().strip_prefix("gitdir:")?.trim());

    Some(if named.is_absolute() {
        named.to_path_buf()
    } else {
        pointer.parent()?.join(named)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_dir;

    #[test]
    fn a_symbolic_head_is_the_branch_it_names() {
        assert_eq!(
            Head::parse("ref: refs/heads/feat-login\n"),
            Some(Head::Branch {
                branch: "feat-login".to_owned()
            })
        );
    }

    #[test]
    fn a_branch_name_containing_slashes_survives() {
        assert_eq!(
            Head::parse("ref: refs/heads/alfonz/feat/login\n")
                .unwrap()
                .to_string(),
            "alfonz/feat/login"
        );
    }

    #[test]
    fn a_raw_object_id_is_a_detached_head_reported_abbreviated() {
        let head = Head::parse("9f2c1ab0d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9\n").unwrap();

        assert_eq!(
            head,
            Head::Detached {
                commit: "9f2c1ab0d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9".to_owned()
            }
        );
        assert_eq!(head.to_string(), "(detached at 9f2c1ab)");
        assert_eq!(head.branch(), None);
    }

    #[test]
    fn nothing_readable_is_no_head_rather_than_a_guess() {
        for text in ["", "   \n", "ref: refs/tags/v1\n", "ref: \n"] {
            assert_eq!(Head::parse(text), None, "for {text:?}");
        }
    }

    #[test]
    fn a_linked_worktree_is_read_through_the_gitdir_its_dot_git_file_names() {
        let dir = scratch_dir("head-linked");
        let administrative = dir.join("repo").join(".git").join("worktrees").join("api");
        fs::create_dir_all(&administrative).unwrap();
        fs::write(administrative.join("HEAD"), "ref: refs/heads/feat-login\n").unwrap();

        let worktree = dir.join("plane").join("acme").join("api");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", administrative.display()),
        )
        .unwrap();

        assert_eq!(
            Head::read(&worktree).unwrap().branch(),
            Some("feat-login"),
            "no git process is started to answer this"
        );
    }

    #[test]
    fn a_gitdir_written_relative_resolves_against_the_worktree() {
        let dir = scratch_dir("head-relative");
        let administrative = dir.join("administrative");
        fs::create_dir_all(&administrative).unwrap();
        fs::write(administrative.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let worktree = dir.join("worktree");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(worktree.join(".git"), "gitdir: ../administrative\n").unwrap();

        assert_eq!(Head::read(&worktree).unwrap().branch(), Some("main"));
    }

    #[test]
    fn a_worktree_that_is_not_there_has_no_head() {
        let dir = scratch_dir("head-missing");

        assert_eq!(Head::read(&dir.join("gone")), None);
    }
}
