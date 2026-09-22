//! Reading the members a `create` or an `add` was asked for, before anything
//! is created.
//!
//! Shared by both verbs because they resolve a member identically: `@name` is a
//! registered project, anything else is a path to an **ad-hoc member**, and the
//! branch is everything after the last colon. Two implementations of that would
//! be two answers to *what did the user just name*, and the one place the
//! difference would surface is a plane file already written.
//!
//! The pass is split in two, and the split is load-bearing. [`resolve_all`]
//! answers *what are these members* using the filesystem and the registry;
//! [`preflight_all`] answers *can the branch be honoured*, which is a question
//! about refs and so must be asked **after** the fetch. Everything either half
//! refuses is collected before the claim, so the common mistake fails with
//! nothing created at all (ADR-0004).

use std::path::{Path, PathBuf};

use crate::directories::Directories;
use crate::error::{BranchWanted, EngineError};
use crate::member::{MemberRef, WorktreePath};
use crate::project_dir::{self, ProjectDirectory};
use crate::repo::Git;
use crate::wire::BranchIntent;

/// Everything member resolution needs that is not in the request.
pub struct PlanContext<'a> {
    pub directories: &'a Directories,
    pub git: &'a Git,
    /// What a member path's leading `~` means. `None` where the host has no
    /// home directory, which makes `~/…` a path called `~`.
    pub home: Option<&'a Path>,
}

/// What a branch this run has to create is cut from.
///
/// The two arms are the difference between **joining a colleague's work and
/// starting your own**, which is the one thing a `create` row marks — so it is
/// carried as the shape of the base rather than recomputed from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base {
    /// The forge has this branch and this repo does not yet, so the local
    /// branch is cut from the remote-tracking ref and tracks it. Joining.
    ///
    /// Reachable only because the fetch ran first, which is the whole of why
    /// `Resolve` insists on one (ADR-0007).
    Forge(String),
    /// Nothing anywhere has this branch, so it is cut from the **base branch**:
    /// `refs/remotes/origin/HEAD`, or the source repo's own `HEAD`. Starting.
    Fresh(String),
}

impl Base {
    /// The ref `git worktree add -b` is given.
    pub fn as_ref_name(&self) -> &str {
        match self {
            Base::Forge(reference) | Base::Fresh(reference) => reference,
        }
    }

    /// Whether cutting from here starts a branch of the user's own, rather than
    /// bringing one down that already exists on the forge.
    pub fn is_fresh(&self) -> bool {
        matches!(self, Base::Fresh(_))
    }
}

/// Where a member's worktree comes from, as far as the filesystem can say.
pub struct MemberSource {
    /// Where the repo is, or would be.
    pub at: PathBuf,
    /// Whether it is actually there. One `stat`, and the whole of the
    /// `source_repo_missing` check.
    pub present: bool,
    /// Whether bitplane built it, and so owns its refs.
    pub owned: bool,
}

impl MemberSource {
    /// The repo, where there is one to act on.
    pub fn found(&self) -> Option<&Path> {
        self.present.then_some(self.at.as_path())
    }
}

/// One member, resolved and checked, before anything is created.
pub struct PlannedMember {
    pub reference: MemberRef,
    /// The repo the worktree comes from: `repo.git` for an **owned** project,
    /// the user's own checkout for an **adopted** one or an ad-hoc member.
    pub source: PathBuf,
    pub branch: String,
    /// Where the worktree goes, relative to the plane directory.
    pub path: WorktreePath,
    /// What a branch that has to be cut is cut from. `None` where the branch is
    /// already in this repo and nothing has to be cut.
    pub base: Option<Base>,
    /// What repository the source belongs to. Two members sharing one are two
    /// worktrees of the same thing, however differently they were spelled.
    pub repository: PathBuf,
    /// Whether bitplane built this source repo and so owns its refs — which is
    /// what decides whether there is anything to fetch and, later, whether
    /// `destroy` may delete the member's branch (ADR-0007).
    pub owned: bool,
}

/// Reads every member as the user wrote it, and refuses a batch that names one
/// thing twice.
///
/// No ref is read here: the branch questions belong to [`preflight_all`],
/// which runs after the fetch, because resolving a branch against a source repo
/// nobody has updated is how `bp add @api:colleagues-branch` silently cuts a
/// new, unrelated branch.
pub fn resolve_all(
    specs: &[String],
    fallback_branch: Option<&str>,
    context: &PlanContext<'_>,
) -> Result<Vec<PlannedMember>, EngineError> {
    let mut planned: Vec<PlannedMember> = Vec::new();

    for spec in specs {
        let member = resolve(spec, fallback_branch, context)?;

        if let Some(earlier) = planned
            .iter()
            .find(|other| other.reference == member.reference)
        {
            return Err(EngineError::DuplicateMember {
                member: earlier.reference.to_string(),
                plane: None,
            });
        }
        if let Some(earlier) = planned
            .iter()
            .find(|other| other.repository == member.repository)
        {
            return Err(EngineError::SameRepository {
                first: earlier.reference.to_string(),
                second: member.reference.to_string(),
            });
        }
        if let Some(earlier) = planned.iter().find(|other| other.path == member.path) {
            return Err(EngineError::MemberPathCollision {
                first: earlier.reference.to_string(),
                second: member.reference.to_string(),
                path: member.path.to_string(),
            });
        }

        planned.push(member);
    }

    Ok(planned)
}

/// Everything about each member's branch that can be known before the claim.
///
/// Fills in [`PlannedMember::base`] where a branch has to be cut, so the build
/// knows which `git worktree add` form to use without asking again.
pub fn preflight_all(
    planned: &mut [PlannedMember],
    intent: BranchIntent,
    git: &Git,
) -> Result<(), EngineError> {
    planned
        .iter_mut()
        .try_for_each(|member| preflight(member, intent, git))
}

/// The projects among `planned` there is something to fetch for, named the way
/// [`crate::wire::ProjectFetchRequest`] takes them.
///
/// Owned projects only: an adopted project's source repo is the user's own
/// checkout and an ad-hoc member's is a repo bitplane merely points at, so
/// neither has a forge of bitplane's to be brought up to date with.
pub fn fetchable(planned: &[PlannedMember]) -> Vec<String> {
    planned
        .iter()
        .filter(|member| member.owned)
        .filter_map(|member| match &member.reference {
            MemberRef::Project(name) => Some(name.to_string()),
            MemberRef::Repo(_) => None,
        })
        .collect()
}

/// The repo a member already in a plane is a worktree of.
///
/// The one place `plane.toml`'s `@name` is turned back into a directory, so
/// every verb that acts on an existing plane agrees about what a member's
/// source is and names the same thing when it is not there.
pub fn source_of(member: &MemberRef, directories: &Directories) -> MemberSource {
    match member {
        MemberRef::Repo(path) => MemberSource {
            present: path.is_dir(),
            at: path.clone(),
            owned: false,
        },
        MemberRef::Project(name) => {
            let project = ProjectDirectory::of(directories, name.clone());

            // A project that is not registered, or whose `project.toml` will
            // not parse, has no source repo bitplane can name — so the
            // **project directory** stands in, which is the thing the user has
            // to go and look at either way. Reading is all this does: a file a
            // read cannot parse is one it has no business rewriting.
            match project.read_project_file() {
                Ok(file) => {
                    let at = project_dir::source_repo_of(&project, &file);

                    MemberSource {
                        present: at.is_dir(),
                        at,
                        owned: file.source.is_owned(),
                    }
                }
                Err(_) => MemberSource {
                    at: project.path().to_path_buf(),
                    present: false,
                    owned: false,
                },
            }
        }
    }
}

/// A leading `~`, replaced by the home directory the engine was built with.
pub fn expand_home(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

/// The branch is everything after the **last** colon, where there is one.
///
/// Git forbids a colon in a ref name, so what *follows* one is unambiguously a
/// branch — but what precedes it is not always unambiguously a whole path.
pub fn split_branch_suffix(spec: &str) -> (&str, Option<&str>) {
    match spec.rsplit_once(':') {
        Some((subject, branch)) if !subject.is_empty() && !branch.is_empty() => {
            (subject, Some(branch))
        }
        _ => (spec, None),
    }
}

/// Reads one member as the user wrote it.
fn resolve(
    spec: &str,
    fallback_branch: Option<&str>,
    context: &PlanContext<'_>,
) -> Result<PlannedMember, EngineError> {
    let (subject, suffix) = split_branch_suffix(spec);

    let (reference, source, path, owned) = match MemberRef::parse(subject)? {
        MemberRef::Project(name) => {
            let project = project_dir::find(context.directories, &name)?;
            let file = project.read_project_file()?;

            // Derived from the **source**, not from the source repo: an owned
            // project's repo is at `<project-dir>/repo.git`, so deriving from
            // it would land every owned member at `<name>/repo.git`. A URL is
            // not a path either, and `ProjectSource` is what knows how to strip
            // a transport and a trailing `.git` (ADR-0005).
            let path = file.source.worktree_path()?;

            (
                MemberRef::Project(name),
                project_dir::source_repo_of(&project, &file),
                path,
                file.source.is_owned(),
            )
        }
        MemberRef::Repo(path) => {
            // A path whose last segment contains a colon cannot carry a branch
            // suffix: the split cannot tell the two apart, so bitplane refuses
            // rather than guessing.
            if suffix.is_some() && expand_home(Path::new(spec), context.home).is_dir() {
                return Err(EngineError::MemberPathAmbiguous {
                    spec: spec.to_owned(),
                });
            }
            // A path member may be relative, absolute, or start with `~`. The
            // canonical, symlink-followed result is what the plane records and
            // what identifies the member afterwards.
            let source = canonicalise(&path, context)?;
            let reference = MemberRef::Repo(source.clone());
            let path = reference.worktree_path(&source)?;

            (reference, source, path, false)
        }
    };

    let branch = suffix
        .or(fallback_branch)
        .ok_or_else(|| EngineError::BranchUnspecified {
            member: reference.to_string(),
        })?
        .to_owned();

    // Before anything else about the member is asked: everything below assumes
    // there is a repository here.
    context.git.ensure_repository(&source)?;

    Ok(PlannedMember {
        repository: context.git.repository_identity(&source)?,
        reference,
        source,
        branch,
        path,
        base: None,
        owned,
    })
}

/// Everything about one member's branch that can be known before the claim.
fn preflight(
    member: &mut PlannedMember,
    intent: BranchIntent,
    git: &Git,
) -> Result<(), EngineError> {
    let here = git.branch_exists(&member.source, &member.branch)?;
    // Asked only where the answer could change anything: a branch already in
    // this repo is the one that gets checked out whatever the forge has.
    let on_the_forge = match here {
        true => None,
        false => git.origin_branch(&member.source, &member.branch)?,
    };

    // **Exists means resolves**, from either namespace. For a bitplane-owned
    // source repo `refs/heads/*` holds only the branches plane members were
    // created on, so `--existing-branch @api:colleagues-branch` asking about
    // that namespace alone would refuse a branch plainly on the forge — and
    // `--new-branch` would cheerfully cut one that collides at push time.
    let exists = here || on_the_forge.is_some();

    match (intent, exists) {
        (BranchIntent::RequireNew, true) => {
            return Err(EngineError::BranchIntentUnmet {
                member: member.reference.to_string(),
                branch: member.branch.clone(),
                wanted: BranchWanted::New,
            });
        }
        (BranchIntent::RequireExisting, false) => {
            return Err(EngineError::BranchIntentUnmet {
                member: member.reference.to_string(),
                branch: member.branch.clone(),
                wanted: BranchWanted::Existing,
            });
        }
        _ => {}
    }

    if here {
        // Git refuses `worktree add` on a branch checked out in any worktree of
        // the same repo. bitplane says so in its own words rather than passing
        // git's message through. A bitplane-owned source repo is bare and
        // occupies nothing, so this can only fire for an adopted project, an
        // ad-hoc member, or a branch another plane already holds.
        if let Some(holder) = git.worktree_holding(&member.source, &member.branch)? {
            return Err(EngineError::BranchOccupied {
                branch: member.branch.clone(),
                repo: member.source.clone(),
                worktree: holder.path,
                stale: holder.prunable,
            });
        }
        return Ok(());
    }

    if let Some(reference) = on_the_forge {
        // Cut from the forge's own branch, which is what `git worktree add -b`
        // from a remote-tracking ref does — and it sets the upstream, so the
        // worktree bitplane hands over is on the colleague's branch rather than
        // on an unrelated branch that merely shares its name.
        member.base = Some(Base::Forge(reference));
        return Ok(());
    }

    member.base = Some(Base::Fresh(
        git.base_for_a_new_branch(&member.source)?.ok_or_else(|| {
            EngineError::BaseBranchUnresolved {
                member: member.reference.to_string(),
                branch: member.branch.clone(),
            }
        })?,
    ));

    Ok(())
}

/// Expands a leading `~`, then canonicalises — following symlinks, so a member
/// reached two ways is one member.
fn canonicalise(path: &Path, context: &PlanContext<'_>) -> Result<PathBuf, EngineError> {
    let expanded = expand_home(path, context.home);

    expanded
        .canonicalize()
        .map_err(|_| EngineError::MemberNotARepository {
            path: expanded.clone(),
            reason: if expanded.exists() {
                "could not be resolved".to_owned()
            } else {
                "does not exist".to_owned()
            },
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_suffix_is_everything_after_the_last_colon() {
        let cases = [
            ("@api", ("@api", None)),
            ("@api:feat-login", ("@api", Some("feat-login"))),
            (
                "~/projects/bitplane:feat-x",
                ("~/projects/bitplane", Some("feat-x")),
            ),
            (
                "~/projects/bitplane:feat/x",
                ("~/projects/bitplane", Some("feat/x")),
            ),
            ("/repos/api", ("/repos/api", None)),
            // Nothing on one side of the colon is not a suffix.
            ("/repos/api:", ("/repos/api:", None)),
        ];

        for (spec, expected) in cases {
            assert_eq!(split_branch_suffix(spec), expected, "for {spec}");
        }
    }

    #[test]
    fn a_leading_tilde_is_the_home_the_engine_was_built_with() {
        assert_eq!(
            expand_home(Path::new("~/projects/api"), Some(Path::new("/home/alfonz"))),
            PathBuf::from("/home/alfonz/projects/api")
        );
        assert_eq!(
            expand_home(Path::new("~/projects/api"), None),
            PathBuf::from("~/projects/api"),
            "a host with no home has a directory called ~"
        );
    }
}
