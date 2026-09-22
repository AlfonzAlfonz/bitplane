//! The git commands bitplane runs against a source repo.
//!
//! bitplane owns a worktree's **existence and location**; the user owns its
//! contents and its `HEAD` (ADR-0006). So this module adds worktrees, removes
//! them, and reads. It never commits, pushes, stashes, checks out or resets,
//! and the absence of those verbs here is the boundary rule made structural.
//!
//! Every listing uses `git worktree list --porcelain -z`, the NUL-terminated
//! form — the only one that handles paths and lock reasons containing unsafe
//! bytes correctly, and the reason ADR-0001 floors git at 2.36.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::error::EngineError;

/// Runs git.
///
/// Holds the `PATH` to find it on, so a test can drive the real code against a
/// git of its choosing rather than whichever one the developer happens to have.
#[derive(Debug, Clone, Default)]
pub struct Git {
    search_path: Option<PathBuf>,
}

/// One row of `git worktree list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// The branch it is on, absent when it is detached or bare.
    pub branch: Option<String>,
    /// Git's own word: the worktree's directory is gone. The admin entry
    /// survives, and until someone prunes it the branch still looks checked out
    /// — which would make every retry after a hard kill refuse.
    pub prunable: bool,
}

impl Git {
    /// Uses the git on the process's own `PATH`.
    pub fn new() -> Git {
        Git { search_path: None }
    }

    /// Uses the git on `search_path` instead of the process's own.
    pub fn on_path(search_path: impl Into<PathBuf>) -> Git {
        Git {
            search_path: Some(search_path.into()),
        }
    }

    /// Fails unless `path` is the **root of a working tree**, or a bare repo.
    ///
    /// The root, not merely somewhere inside one: `git rev-parse` answers for
    /// the containing repository from any subdirectory, so accepting that would
    /// write a source into `plane.toml` that is not where the repo is.
    ///
    /// Being a root is *not* enough to tell two members apart, though — a linked
    /// worktree is the root of its own working tree and shares its repository
    /// with the checkout it came from. That is what
    /// [`Git::repository_identity`] is for.
    pub fn ensure_repository(&self, path: &Path) -> Result<(), EngineError> {
        if !path.is_dir() {
            return Err(EngineError::MemberNotARepository {
                path: path.to_path_buf(),
                reason: if path.exists() {
                    "is not a directory".to_owned()
                } else {
                    "does not exist".to_owned()
                },
            });
        }

        // A bare repository has no working tree to be at the root of, so it can
        // only ever be named by its own path.
        if self
            .asked(path, &["rev-parse", "--is-bare-repository"])?
            .trim()
            == "true"
        {
            return Ok(());
        }

        let root = canonical(self.asked(path, &["rev-parse", "--show-toplevel"])?.trim());
        if root == path {
            return Ok(());
        }

        Err(EngineError::MemberNotARepository {
            path: path.to_path_buf(),
            reason: format!(
                "is inside the repository at {}, not its own repository",
                root.display()
            ),
        })
    }

    /// What repository `path` belongs to.
    ///
    /// The **common** git directory, which every linked worktree of one
    /// repository shares. `--show-toplevel` cannot answer this: in a linked
    /// worktree it returns that worktree's own root, so two worktrees of one
    /// repository would look like two repositories — and a plane holds at most
    /// one worktree per repository, which is what lets the fan-out run in
    /// parallel without the per-source-repo locks ever contending.
    pub fn repository_identity(&self, path: &Path) -> Result<PathBuf, EngineError> {
        let common = self.asked(path, &["rev-parse", "--git-common-dir"])?;
        let common = PathBuf::from(common.trim());

        // git answers relatively when it can, and the answer is relative to the
        // directory it was asked from.
        Ok(canonical_path(&if common.is_absolute() {
            common
        } else {
            path.join(common)
        }))
    }

    /// Runs a `rev-parse` whose failure means "not a repository" — and only
    /// that. A git that could not be spawned at all keeps its own typed
    /// failure, so a missing git is never reported as a bad member.
    fn asked(&self, path: &Path, args: &[&str]) -> Result<String, EngineError> {
        match self.run(path, args) {
            Ok(answer) => Ok(answer),
            Err(EngineError::GitFailed { .. }) => Err(EngineError::MemberNotARepository {
                path: path.to_path_buf(),
                reason: "is not a git repository".to_owned(),
            }),
            Err(other) => Err(other),
        }
    }

    /// Whether `refs/heads/<branch>` resolves in `repo`.
    pub fn branch_exists(&self, repo: &Path, branch: &str) -> Result<bool, EngineError> {
        let reference = format!("refs/heads/{branch}");
        let found = self.status(repo, &["show-ref", "--verify", "--quiet", &reference])?;

        Ok(found)
    }

    /// Every worktree of `repo`, its own included.
    pub fn worktrees(&self, repo: &Path) -> Result<Vec<WorktreeEntry>, EngineError> {
        let listing = self.run(repo, &["worktree", "list", "--porcelain", "-z"])?;

        Ok(parse_worktree_listing(&listing))
    }

    /// The worktree of `repo` holding `branch`, where one does.
    ///
    /// Git refuses `worktree add` on an occupied branch, so this is what lets
    /// bitplane report the refusal in its own words rather than passing git's
    /// message through. A bitplane-owned source repo is bare and occupies
    /// nothing; an adopted project's or an ad-hoc member's checkout occupies
    /// whatever the user is sitting on.
    ///
    /// The whole entry, because a **prunable** one occupies the branch just as
    /// firmly — measured: git refuses `worktree add` on a branch held by an
    /// entry whose directory is gone — but needs a different remedy, since
    /// "check out a different branch there" names a directory that is not
    /// there.
    pub fn worktree_holding(
        &self,
        repo: &Path,
        branch: &str,
    ) -> Result<Option<WorktreeEntry>, EngineError> {
        Ok(self
            .worktrees(repo)?
            .into_iter()
            .find(|entry| entry.branch.as_deref() == Some(branch)))
    }

    /// What a new branch in `repo` would be cut from.
    ///
    /// `refs/remotes/origin/HEAD` first, then the repo's own `HEAD`. Never
    /// stored: a forge that renames its default branch would make a stored copy
    /// silently wrong.
    pub fn base_for_a_new_branch(&self, repo: &Path) -> Result<Option<String>, EngineError> {
        if self.status(repo, &["show-ref", "--verify", "--quiet", ORIGIN_HEAD])? {
            return Ok(Some(ORIGIN_HEAD.to_owned()));
        }

        if self.status(repo, &["rev-parse", "--verify", "--quiet", "HEAD"])? {
            return Ok(Some("HEAD".to_owned()));
        }

        Ok(None)
    }

    /// `git worktree add`, checking out an existing branch.
    pub fn add_worktree(&self, repo: &Path, at: &Path, branch: &str) -> Result<(), EngineError> {
        self.run_os(
            repo,
            &[
                OsStr::new("worktree"),
                OsStr::new("add"),
                at.as_os_str(),
                OsStr::new(branch),
            ],
        )
        .map(drop)
    }

    /// `git worktree add -b`, creating the branch from `base`.
    pub fn add_worktree_on_a_new_branch(
        &self,
        repo: &Path,
        at: &Path,
        branch: &str,
        base: &str,
    ) -> Result<(), EngineError> {
        self.run_os(
            repo,
            &[
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("-b"),
                OsStr::new(branch),
                at.as_os_str(),
                OsStr::new(base),
            ],
        )
        .map(drop)
    }

    /// `git worktree remove`.
    ///
    /// `force` is passed by the **abort path only**, where it is safe because
    /// by construction nothing in the abort window is the user's yet. It must
    /// not be "consistency-fixed" onto `destroy`, where the same flag would
    /// step straight over the uncommitted-work veto (ADR-0004).
    pub fn remove_worktree(&self, repo: &Path, at: &Path, force: bool) -> Result<(), EngineError> {
        let mut args = vec![OsStr::new("worktree"), OsStr::new("remove")];
        if force {
            args.push(OsStr::new("--force"));
        }
        args.push(at.as_os_str());

        self.run_os(repo, &args).map(drop)
    }

    /// `git worktree prune`: forgets admin entries whose directory is gone.
    ///
    /// The fallback for an entry `worktree remove` will not take, which is one
    /// whose directory something already deleted. Repo-wide by nature — git has
    /// no per-entry prune — so it is reached for only when the precise removal
    /// has already failed.
    pub fn prune_worktrees(&self, repo: &Path) -> Result<(), EngineError> {
        self.run(repo, &["worktree", "prune"]).map(drop)
    }

    /// `git branch -D`.
    ///
    /// Only ever called on a branch **this run cut**, in the abort window,
    /// after its worktree has gone: it is seconds old and points at the base it
    /// was cut from, so there is nothing in it to lose.
    pub fn delete_branch(&self, repo: &Path, branch: &str) -> Result<(), EngineError> {
        self.run(repo, &["branch", "-D", branch]).map(drop)
    }

    /// Runs git and returns its stdout, failing with git's own first line.
    fn run(&self, repo: &Path, args: &[&str]) -> Result<String, EngineError> {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        self.run_os(repo, &args)
    }

    fn run_os(&self, repo: &Path, args: &[&OsStr]) -> Result<String, EngineError> {
        let output = self.spawn(repo, args)?;

        if !output.status.success() {
            return Err(EngineError::GitFailed {
                message: describe(args, &output),
            });
        }

        String::from_utf8(output.stdout).map_err(|_| EngineError::GitFailed {
            message: format!("git {} printed something that is not UTF-8", rendered(args)),
        })
    }

    /// Runs git for its exit status alone — the shape of every `--quiet`
    /// existence probe, where a non-zero exit is the answer rather than a
    /// failure.
    fn status(&self, repo: &Path, args: &[&str]) -> Result<bool, EngineError> {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let output = self.spawn(repo, &args)?;

        Ok(output.status.success())
    }

    fn spawn(&self, repo: &Path, args: &[&OsStr]) -> Result<Output, EngineError> {
        let mut command = Command::new("git");
        command.arg("-C").arg(repo).args(args);
        if let Some(search_path) = &self.search_path {
            command.env("PATH", search_path);
        }

        command.output().map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => EngineError::GitMissing,
            _ => EngineError::GitUnusable {
                message: err.to_string(),
            },
        })
    }
}

/// Where git records the forge's default branch.
const ORIGIN_HEAD: &str = "refs/remotes/origin/HEAD";

fn canonical(path: &str) -> PathBuf {
    canonical_path(Path::new(path))
}

/// A path as the filesystem sees it, or as given where it cannot be resolved.
fn canonical_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Reads `git worktree list --porcelain -z`: NUL-terminated attributes, with an
/// extra NUL between records.
fn parse_worktree_listing(listing: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut prunable = false;

    for attribute in listing.split('\0') {
        if attribute.is_empty() {
            if let Some(path) = path.take() {
                entries.push(WorktreeEntry {
                    path,
                    branch: branch.take(),
                    prunable,
                });
            }
            branch = None;
            prunable = false;
            continue;
        }

        if let Some(value) = attribute.strip_prefix("worktree ") {
            path = Some(PathBuf::from(value));
        } else if let Some(value) = attribute.strip_prefix("branch refs/heads/") {
            branch = Some(value.to_owned());
        } else if attribute == "prunable" || attribute.starts_with("prunable ") {
            prunable = true;
        }
    }

    entries
}

fn describe(args: &[&OsStr], output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let complaint = complaint(&stderr);

    match output.status.code() {
        Some(code) => format!("git {} exited {code}: {complaint}", rendered(args)),
        None => format!("git {} was killed by a signal: {complaint}", rendered(args)),
    }
}

/// What git was actually complaining about.
///
/// git narrates progress on stderr too — `Preparing worktree (new branch
/// 'feat-login')` precedes the `fatal:` that matters — so the first line is the
/// wrong one to quote. Prefer git's own diagnostic prefixes, and fall back to
/// the **last** thing it said rather than the first.
fn complaint(stderr: &str) -> &str {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    lines
        .iter()
        .find(|line| line.starts_with("fatal: ") || line.starts_with("error: "))
        .or_else(|| lines.last())
        .map(|line| {
            line.trim_start_matches("fatal: ")
                .trim_start_matches("error: ")
        })
        .unwrap_or("git said nothing")
}

/// The command, for a message. Only the verb and its subcommand: the paths are
/// already in the row's subject, and repeating them makes the sentence unreadable.
fn rendered(args: &[&OsStr]) -> String {
    args.iter()
        .take(2)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<String>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_porcelain_listing_reads_back_as_its_worktrees() {
        let listing = concat!(
            "worktree /repos/api\0HEAD abc\0branch refs/heads/main\0\0",
            "worktree /planes/bp-a3f9c2e1/repos/api\0HEAD def\0branch refs/heads/feat-login\0\0",
        );

        assert_eq!(
            parse_worktree_listing(listing),
            vec![
                WorktreeEntry {
                    path: PathBuf::from("/repos/api"),
                    branch: Some("main".to_owned()),
                    prunable: false,
                },
                WorktreeEntry {
                    path: PathBuf::from("/planes/bp-a3f9c2e1/repos/api"),
                    branch: Some("feat-login".to_owned()),
                    prunable: false,
                },
            ]
        );
    }

    #[test]
    fn a_detached_or_bare_worktree_reports_no_branch() {
        let listing = concat!(
            "worktree /repos/api.git\0bare\0\0",
            "worktree /planes/x/api\0HEAD abc\0detached\0\0",
        );

        let entries = parse_worktree_listing(listing);

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.branch.is_none()));
    }

    #[test]
    fn a_path_containing_a_newline_survives_the_nul_terminated_form() {
        // The reason ADR-0001 floors git at 2.36: `-z` is the only form that
        // does not mangle this.
        let listing = "worktree /repos/od\nd\0HEAD abc\0branch refs/heads/main\0\0";

        assert_eq!(
            parse_worktree_listing(listing),
            vec![WorktreeEntry {
                path: PathBuf::from("/repos/od\nd"),
                branch: Some("main".to_owned()),
                prunable: false,
            }]
        );
    }

    #[test]
    fn an_empty_listing_is_no_worktrees() {
        assert_eq!(parse_worktree_listing(""), Vec::new());
    }

    #[test]
    fn an_entry_whose_directory_is_gone_is_read_as_prunable() {
        let listing = concat!(
            "worktree /planes/x/api\0HEAD abc\0branch refs/heads/feat\0",
            "prunable gitdir file points to non-existent location\0\0",
        );

        let entries = parse_worktree_listing(listing);

        assert_eq!(entries.len(), 1);
        assert!(entries[0].prunable, "a stale entry occupies nothing");
        assert_eq!(entries[0].branch.as_deref(), Some("feat"));
    }

    #[test]
    fn a_failure_quotes_what_git_complained_about_not_what_it_narrated() {
        let stderr = concat!(
            "Preparing worktree (new branch 'feat-login')\n",
            "fatal: could not create leading directories of '/planes/x/api'\n",
        );

        assert_eq!(
            complaint(stderr),
            "could not create leading directories of '/planes/x/api'"
        );
    }

    #[test]
    fn with_no_diagnostic_the_last_thing_git_said_is_quoted() {
        assert_eq!(complaint("first\nlast\n"), "last");
        assert_eq!(complaint("   \n"), "git said nothing");
    }
}
