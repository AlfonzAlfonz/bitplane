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
    /// Git's own lock, with the reason where one was given. Someone said "do
    /// not remove this", so removing it is a refusal bitplane reports in its
    /// own words and only a waiver gets past.
    pub locked: Option<WorktreeLock>,
}

/// `git worktree lock`, and what it was locked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeLock {
    /// The reason `git worktree lock --reason` was given, where it was given
    /// one. Empty is the honest answer: git stores no reason by default.
    pub reason: Option<String>,
}

/// What git's own status says about a worktree's contents.
///
/// Two commands rather than one parse: `--untracked-files=no` answers the first
/// and `ls-files --others` the second, so neither answer depends on splitting
/// porcelain records whose rename form carries a second path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkingTree {
    /// Tracked files differ from `HEAD` or from the index.
    pub uncommitted: bool,
    /// Files git does not track and no ignore rule covers.
    pub untracked: bool,
}

/// How hard `git worktree remove` is allowed to push.
///
/// Never a blanket flag. Each step up is unlocked by a waiver the user granted
/// for something this very member has, which is what keeps ADR-0004's
/// abort-path force from leaking onto `destroy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insistence {
    /// Git's own refusal stands.
    Polite,
    /// `--force`: work in the worktree was waived.
    Forced,
    /// `--force --force`: git's lock on the worktree was waived as well. Git
    /// wants the flag twice, and nothing less removes a locked worktree.
    ForcedThroughLock,
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

    /// The branch a new plane branch would be cut from, **named** rather than
    /// spelled as a ref — `main`, not `refs/remotes/origin/HEAD`.
    ///
    /// The same ladder as [`Git::base_for_a_new_branch`], and deliberately not
    /// a second source of truth: both read git, every time, because a forge
    /// that renames its default branch would make a stored copy silently wrong.
    pub fn default_branch(&self, repo: &Path) -> Result<Option<String>, EngineError> {
        if let Some(named) = self.symbolic_ref(repo, ORIGIN_HEAD)? {
            return Ok(Some(
                named.strip_prefix("origin/").unwrap_or(&named).to_owned(),
            ));
        }

        // Only once HEAD resolves to a commit. A bare repo's HEAD is a symref
        // to an unborn `refs/heads/main` from the moment it is created, and
        // reporting that as a default would be a guess dressed as an answer.
        if self.status(repo, &["rev-parse", "--verify", "--quiet", "HEAD"])? {
            return self.symbolic_ref(repo, "HEAD");
        }

        Ok(None)
    }

    /// `git init --bare <name>`, inside `parent`.
    ///
    /// Neither `clone --mirror` nor `clone --bare` (ADR-0005): `--mirror` turns
    /// an ordinary `git push` from a worktree into a force-push-everything
    /// against the forge, and `--bare` copies the forge's branches into
    /// `refs/heads/*` before the refspec can be set, polluting the namespace
    /// the refspec exists to keep clean. Re-running it on a repo a previous
    /// attempt left is a reinit, which is what makes a failed add resumable.
    pub fn init_bare(&self, parent: &Path, name: &str) -> Result<(), EngineError> {
        self.run(parent, &["init", "--bare", "--quiet", name])
            .map(drop)
    }

    /// Points `origin` at `url`, whether or not a previous attempt already
    /// added it.
    ///
    /// `remote add` on a second run fails with `remote origin already exists`,
    /// and a failed `project_add` is meant to be re-runnable — so the add is
    /// the fresh path and the set-url is the resumption.
    pub fn set_origin(&self, repo: &Path, url: &str) -> Result<(), EngineError> {
        if self.status(repo, &["remote", "get-url", "origin"])? {
            self.run(repo, &["remote", "set-url", "origin", url])
                .map(drop)
        } else {
            self.run(repo, &["remote", "add", "origin", url]).map(drop)
        }
    }

    /// `git config <key> <value>` in `repo`.
    ///
    /// Only ever called on a source repo bitplane built. An adopted project's
    /// config is the user's and is never touched.
    pub fn set_config(&self, repo: &Path, key: &str, value: &str) -> Result<(), EngineError> {
        self.run(repo, &["config", key, value]).map(drop)
    }

    /// `git fetch origin`, optionally pruning.
    ///
    /// `--prune` is safe **because of the refspec**: `refs/remotes/origin/*` is
    /// a pure cache of the forge and holds nothing a user owns, so pruning
    /// cannot lose work. Tags are left at git's default and never pruned —
    /// `refs/tags/*` is a namespace shared with every worktree bitplane does
    /// not own.
    pub fn fetch(&self, repo: &Path, prune: bool) -> Result<(), EngineError> {
        let mut args = vec!["fetch", "--quiet"];
        if prune {
            args.push("--prune");
        }
        args.push("origin");

        self.run(repo, &args).map(drop)
    }

    /// `git remote set-head origin -a`, which records the forge's default
    /// branch at `refs/remotes/origin/HEAD`.
    ///
    /// Answers whether the forge named one. A remote with no HEAD — an empty
    /// repository — is not a failure: the base is then simply unspecified, and
    /// that is only an error at the moment a new branch has to be cut from it
    /// (ADR-0005).
    pub fn set_origin_head(&self, repo: &Path) -> Result<bool, EngineError> {
        self.status(repo, &["remote", "set-head", "origin", "-a"])
    }

    /// Every branch under `refs/remotes/origin/*`, with the object each points
    /// at.
    ///
    /// The oracle for *"how much did that fetch change"*, which git's own
    /// output only reports as prose. Taken before and after, so a ref added,
    /// removed or moved all count alike.
    ///
    /// `origin/HEAD` is **excluded**: it is a symref recording which branch the
    /// forge defaults to, so it resolves to whatever that branch points at and
    /// would report every move of `main` twice.
    pub fn remote_refs(&self, repo: &Path) -> Result<Vec<(String, String)>, EngineError> {
        let listing = self.run(
            repo,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/remotes/origin",
            ],
        )?;

        Ok(listing
            .lines()
            .filter_map(|line| line.split_once(' '))
            .filter(|(reference, _)| *reference != ORIGIN_HEAD)
            .map(|(reference, object)| (reference.to_owned(), object.to_owned()))
            .collect())
    }

    /// Every `refs/heads/*` in `repo`.
    ///
    /// In a bitplane-owned source repo this is **exactly** the set of branches
    /// plane members were created on, past and present — which is the whole
    /// point of the clone-shaped refspec.
    pub fn local_branches(&self, repo: &Path) -> Result<Vec<String>, EngineError> {
        let listing = self.run(
            repo,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        )?;

        Ok(listing.lines().map(str::to_owned).collect())
    }

    /// What a symbolic ref points at, in short form, or `None` where it is not
    /// there or is not symbolic.
    fn symbolic_ref(&self, repo: &Path, reference: &str) -> Result<Option<String>, EngineError> {
        let args = ["symbolic-ref", "--quiet", "--short", reference];
        let output = self.spawn(repo, &args.map(OsStr::new))?;

        if !output.status.success() {
            return Ok(None);
        }

        Ok(String::from_utf8(output.stdout)
            .ok()
            .map(|named| named.trim().to_owned())
            .filter(|named| !named.is_empty()))
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

    /// The worktree of `repo` at `at`, where git records one.
    ///
    /// Matched on git's own recorded path: git canonicalises at `worktree add`
    /// time, so `/tmp/…` comes back as `/private/tmp/…` on macOS and the two
    /// never compare equal as written.
    pub fn worktree_at(
        &self,
        repo: &Path,
        at: &Path,
    ) -> Result<Option<WorktreeEntry>, EngineError> {
        let wanted = canonical_path(at);

        Ok(self
            .worktrees(repo)?
            .into_iter()
            .find(|entry| canonical_path(&entry.path) == wanted))
    }

    /// What git's own status says about the contents of the worktree at `at`.
    ///
    /// A pure read: it shells out, answers, and stores nothing (ADR-0006). The
    /// index refresh `status` performs is git's own business and is safe,
    /// because `index.lock` is per-worktree.
    pub fn working_tree(&self, at: &Path) -> Result<WorkingTree, EngineError> {
        let tracked = self.run(
            at,
            &["status", "--porcelain=v1", "-z", "--untracked-files=no"],
        )?;
        let others = self.run(at, &["ls-files", "--others", "--exclude-standard", "-z"])?;

        Ok(WorkingTree {
            uncommitted: !tracked.is_empty(),
            untracked: !others.is_empty(),
        })
    }

    /// What `HEAD` resolves to in the worktree at `at`.
    ///
    /// Used as the tip for the containment check, so a detached worktree is
    /// asked the same question as one on a branch.
    pub fn head_commit(&self, at: &Path) -> Result<Option<String>, EngineError> {
        let resolved = self.captured(at, &["rev-parse", "--verify", "--quiet", "HEAD"])?;

        Ok(resolved
            .map(|tip| tip.trim().to_owned())
            .filter(|tip| !tip.is_empty()))
    }

    /// Whether `tip` is contained in **any** `refs/remotes/origin/*`.
    ///
    /// This is the `unpushed` predicate, and it is deliberately **not**
    /// `git branch -d`'s check. A plane branch is cut from `origin/main` and
    /// keeps it as its upstream, so `-d` compares against the wrong ref and
    /// refuses a branch whose commits are already safe on the forge at
    /// `origin/feat` (ADR-0006). Containment answers the question that matters:
    /// *is this work recoverable?*
    ///
    /// No fetch is performed, here or anywhere on the `destroy` path. A
    /// remote-tracking ref left behind by a push from another machine can only
    /// make the answer more conservative — a false refusal the user waives —
    /// never more permissive.
    pub fn contained_in_origin(&self, repo: &Path, tip: &str) -> Result<bool, EngineError> {
        let found = self.captured(
            repo,
            &[
                "for-each-ref",
                "--contains",
                tip,
                "--count=1",
                "--format=%(refname)",
                ORIGIN_REFS,
            ],
        )?;

        // A tip git cannot resolve answers "not contained": the conservative
        // direction, and the only honest one when the question cannot be put.
        Ok(found.is_some_and(|refs| !refs.trim().is_empty()))
    }

    /// `git worktree remove`, pushing exactly as hard as [`Insistence`] says.
    ///
    /// The abort path passes [`Insistence::Forced`] because by construction
    /// nothing in the abort window is the user's yet. On `destroy` and `remove`
    /// the same flag is reached for **only** where the user waived the very
    /// thing git is refusing over, which is what stops ADR-0004's scoped force
    /// from being "consistency-fixed" into a hole in the uncommitted-work veto.
    pub fn remove_worktree(
        &self,
        repo: &Path,
        at: &Path,
        insistence: Insistence,
    ) -> Result<(), EngineError> {
        let mut args = vec![OsStr::new("worktree"), OsStr::new("remove")];
        match insistence {
            Insistence::Polite => {}
            Insistence::Forced => args.push(OsStr::new("--force")),
            // Twice, which is what git wants for a worktree it has locked.
            Insistence::ForcedThroughLock => {
                args.push(OsStr::new("--force"));
                args.push(OsStr::new("--force"));
            }
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

    /// What `git status` says about the worktree at `at`: how many tracked
    /// files have changes, and how many files git does not know about.
    ///
    /// This is the one read that makes git write — `git status` refreshes the
    /// per-worktree index, so it can touch `index.lock`. That lock is
    /// per-worktree and never shared, and bitplane stores nothing either way:
    /// "a read never writes" is about bitplane's own state (ADR-0003).
    pub fn working_tree_changes(&self, at: &Path) -> Result<(usize, usize), EngineError> {
        let listing = self.run(at, &["status", "--porcelain", "-z"])?;

        Ok(count_changes(&listing))
    }

    /// How many commits on the worktree's `HEAD` are contained in no
    /// `refs/remotes/origin/*`.
    ///
    /// Deliberately **not** `git branch -d`'s check, which compares against the
    /// branch's upstream — for a plane branch that is `origin/main`, so `-d`
    /// falsely reports a branch already safe on the forge as unmerged
    /// (ADR-0006). No fetch is performed: a stale tracking ref can only make
    /// this too large, never too small.
    pub fn commits_not_on_origin(&self, at: &Path) -> Result<usize, EngineError> {
        let counted = self.run(
            at,
            &["rev-list", "--count", "HEAD", "--not", "--remotes=origin"],
        )?;

        counted.trim().parse().map_err(|_| EngineError::GitFailed {
            message: format!(
                "git rev-list printed {:?}, which is not a count",
                counted.trim()
            ),
        })
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

    /// Runs git for its stdout, where a non-zero exit is the answer "no" rather
    /// than a failure — a probe that also has something to say when it works.
    fn captured(&self, repo: &Path, args: &[&str]) -> Result<Option<String>, EngineError> {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let output = self.spawn(repo, &args)?;

        if !output.status.success() {
            return Ok(None);
        }

        Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
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

/// The namespace the forge's branches live in, and the whole of what `unpushed`
/// asks about.
const ORIGIN_REFS: &str = "refs/remotes/origin/";

fn canonical(path: &str) -> PathBuf {
    canonical_path(Path::new(path))
}

/// A path as the filesystem sees it, resolving as much of it as exists.
///
/// Plain `canonicalize` fails outright once the leaf is gone, and that is
/// exactly the case comparisons here have to survive: git records a canonical
/// path at `worktree add` time, so a worktree whose directory has since been
/// deleted must still compare equal to git's record. On a host where `/tmp` or
/// the planes directory is a symlink — macOS out of the box — falling back to
/// the path as written makes the two differ, and a stale entry git is still
/// holding a branch on reads as *already gone*.
///
/// So the deepest existing ancestor is canonicalised and the rest appended.
pub fn canonical_path(path: &Path) -> PathBuf {
    if let Ok(resolved) = path.canonicalize() {
        return resolved;
    }

    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if parent != path => canonical_path(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// Reads `git worktree list --porcelain -z`: NUL-terminated attributes, with an
/// extra NUL between records.
fn parse_worktree_listing(listing: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut prunable = false;
    let mut locked: Option<WorktreeLock> = None;

    for attribute in listing.split('\0') {
        if attribute.is_empty() {
            if let Some(path) = path.take() {
                entries.push(WorktreeEntry {
                    path,
                    branch: branch.take(),
                    prunable,
                    locked: locked.take(),
                });
            }
            branch = None;
            prunable = false;
            locked = None;
            continue;
        }

        if let Some(value) = attribute.strip_prefix("worktree ") {
            path = Some(PathBuf::from(value));
        } else if let Some(value) = attribute.strip_prefix("branch refs/heads/") {
            branch = Some(value.to_owned());
        } else if attribute == "prunable" || attribute.starts_with("prunable ") {
            prunable = true;
        } else if let Some(reason) = flag_value(attribute, "locked") {
            locked = Some(WorktreeLock { reason });
        }
    }

    entries
}

/// Reads `git status --porcelain -z` as `(modified, untracked)`.
///
/// NUL-terminated for the same reason every listing here is: it is the only
/// form that does not mangle a path containing a newline. A record is
/// `XY <path>`, and a rename or a copy is followed by a **second** record
/// holding the path it came from — which has to be consumed rather than counted
/// as a change of its own.
fn count_changes(listing: &str) -> (usize, usize) {
    let mut modified = 0;
    let mut untracked = 0;
    let mut records = listing.split('\0');

    while let Some(record) = records.next() {
        let Some(code) = record.get(..2) else {
            continue;
        };

        match code {
            "??" => untracked += 1,
            // Ignored files are not reported without `--ignored`, and are not a
            // change if they ever are.
            "!!" => {}
            _ => modified += 1,
        }

        if matches!(code.as_bytes()[0], b'R' | b'C') {
            records.next();
        }
    }

    (modified, untracked)
}

/// A porcelain flag that may carry a reason: `locked` alone, or `locked <why>`.
fn flag_value(attribute: &str, flag: &str) -> Option<Option<String>> {
    if attribute == flag {
        return Some(None);
    }

    attribute
        .strip_prefix(flag)
        .and_then(|rest| rest.strip_prefix(' '))
        .map(|reason| Some(reason.trim().to_owned()).filter(|reason| !reason.is_empty()))
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
    use std::fs;

    use crate::testing::scratch_dir;

    #[test]
    fn a_path_whose_leaf_is_gone_still_resolves_through_a_symlinked_ancestor() {
        // The case that makes a stale worktree entry read as `already gone`:
        // git recorded the resolved path, the directory has since been deleted,
        // and `canonicalize` on the whole thing fails.
        let dir = scratch_dir("canonical-missing");
        let real = dir.join("real");
        let link = dir.join("link");
        fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert_eq!(
            canonical_path(&link.join("repos").join("alpha")),
            canonical_path(&real).join("repos").join("alpha"),
        );
    }

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
                    locked: None,
                },
                WorktreeEntry {
                    path: PathBuf::from("/planes/bp-a3f9c2e1/repos/api"),
                    branch: Some("feat-login".to_owned()),
                    prunable: false,
                    locked: None,
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
                locked: None,
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
    fn a_status_listing_counts_tracked_changes_apart_from_untracked_files() {
        let listing = " M src/main.rs\0M  Cargo.toml\0?? notes.md\0?? scratch/\0";

        assert_eq!(count_changes(listing), (2, 2));
    }

    #[test]
    fn a_rename_is_one_change_and_not_two() {
        // `R  <to>\0<from>\0` — the second field is where the file came from,
        // not another changed file.
        let listing = "R  src/new.rs\0src/old.rs\0 M src/main.rs\0";

        assert_eq!(count_changes(listing), (2, 0));
    }

    #[test]
    fn a_clean_worktree_counts_nothing() {
        assert_eq!(count_changes(""), (0, 0));
    }

    #[test]
    fn a_path_containing_a_newline_is_still_one_change() {
        let listing = " M src/od\nd.rs\0";

        assert_eq!(count_changes(listing), (1, 0));
    }

    #[test]
    fn a_locked_entry_is_read_with_the_reason_git_was_given() {
        let listing = concat!(
            "worktree /planes/x/api\0HEAD abc\0branch refs/heads/feat\0",
            "locked migration running\0\0",
            "worktree /planes/x/web\0HEAD def\0branch refs/heads/feat\0locked\0\0",
        );

        let entries = parse_worktree_listing(listing);

        assert_eq!(
            entries[0].locked,
            Some(WorktreeLock {
                reason: Some("migration running".to_owned()),
            })
        );
        assert_eq!(
            entries[1].locked,
            Some(WorktreeLock { reason: None }),
            "git stores no reason by default, and that is not `unlocked`"
        );
    }

    #[test]
    fn a_lock_never_leaks_into_the_next_record() {
        let listing = concat!(
            "worktree /planes/x/api\0HEAD abc\0locked migration\0\0",
            "worktree /planes/x/web\0HEAD def\0\0",
        );

        let entries = parse_worktree_listing(listing);

        assert!(entries[0].locked.is_some());
        assert_eq!(entries[1].locked, None);
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
