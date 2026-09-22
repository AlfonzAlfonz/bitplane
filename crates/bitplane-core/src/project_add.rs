//! `project_add`: the registry goes live, one owned project at a time.
//!
//! ```text
//! 1. resolve the name           ← refused with a suggestion if taken
//! 2. mkdir <project-dir>        ← reused where a previous attempt left one
//! 3. lock the sentinel
//! 4. git init --bare repo.git   ┐
//!    git remote add origin      │ the source repo, built rather than cloned
//!    git config …fetch = …      │ (ADR-0005)
//!    git fetch origin           │
//!    git remote set-head -a     │
//!    git config push.default    ┘
//! 5. write project.toml         ← the registration, and the last step
//! ```
//!
//! **The source repo is built by hand rather than cloned, and the refspec is
//! the load-bearing line.** `git clone --mirror` sets
//! `remote.origin.mirror = true`, which a worktree inherits, turning an
//! ordinary `git push` into a force-push-everything-and-delete-the-rest against
//! the forge; `git clone --bare` copies the forge's branches into
//! `refs/heads/*` before the refspec can be set, and a ref missed there is
//! indistinguishable from a plane branch forever after. With
//! `+refs/heads/*:refs/remotes/origin/*`, `refs/heads/*` holds **exactly** the
//! branches plane members were created on, `fetch` always succeeds because it
//! writes only where nothing is ever checked out, and `@{u}` means the forge's
//! view rather than a local copy of it.
//!
//! **A failure keeps the object store and unwinds only the registration.** The
//! cold fetch *is* the expensive step, and `init --bare` + `fetch` is resumable
//! in a way `clone` is not — a partial object store is reused by the next
//! attempt. Because `project.toml` is written last and never written on
//! failure, the leftover directory simply is not a project: nothing lists it,
//! and `doctor` is what reports it.

use std::path::Path;

use crate::directories::Directories;
use crate::error::{EngineError, Problem};
use crate::member::ProjectName;
use crate::project_dir::ProjectDirectory;
use crate::project_file::{ProjectFile, ProjectSource};
use crate::repo::Git;
use crate::wire::{ProjectAddRequest, ProjectAdded};

/// The refspec a bitplane-owned source repo is given. **Not a default** — it is
/// what makes the plane-branch invariant, a fetch that always succeeds, and an
/// honest `@{u}` true at the same time (ADR-0005).
pub const CLONE_SHAPED_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";

/// Everything `project_add` needs that is not in the request.
pub struct ProjectContext<'a> {
    pub directories: &'a Directories,
    pub git: &'a Git,
    /// Called once if the project's lock is contended, so a caller can say
    /// `waiting for …` without the common case printing anything.
    pub on_lock_wait: &'a (dyn Fn(&Path) + Sync),
}

/// Registers a project from a URL, building the source repo bitplane will own.
pub fn project_add(
    request: &ProjectAddRequest,
    context: &ProjectContext<'_>,
) -> Result<ProjectAdded, EngineError> {
    let source = ProjectSource::Owned {
        url: request.url.trim().to_owned(),
    };
    if request.url.trim().is_empty() {
        return Err(EngineError::InvalidRequest {
            message: "a project needs a url to fetch from".to_owned(),
        });
    }

    // Before anything is created: a repo whose worktrees could only land in
    // bitplane's own reserved directory is not one a plane can ever hold.
    source.worktree_path()?;

    let name = resolve_name(request, &source, context.directories)?;
    let project = ProjectDirectory::of(context.directories, name);
    project.make(context.directories)?;

    let _lock = project.lock(context.on_lock_wait)?;

    // Re-checked under the lock: the name was free when it was resolved, and
    // two `bp project add`s racing on one name is exactly what the lock is for.
    // Refused the same way as the first check, so the loser of the race is told
    // the same thing it would have been told a moment earlier.
    if project.is_registered() {
        return Err(refuse_taken(
            request,
            project.name(),
            &source,
            context.directories,
        ));
    }

    // Everything that can still fail happens before the registration is
    // written, the default branch included: `project.toml` is what makes the
    // directory a project, so a failure after it would report a completed add
    // as an error and send the user into `project_name_taken` on the retry.
    let built = build_source_repo(&project, &source, context)
        .and_then(|()| {
            context
                .git
                .default_branch(&project.source_repo())
                .map_err(|error| Problem::about("origin", format!("could not be read: {error}")))
        })
        .map_err(|problem| aborted(&project, problem))?;

    project.write_project_file(&ProjectFile::new(project.name().clone(), source.clone()))?;

    Ok(ProjectAdded {
        name: project.name().clone(),
        source,
        directory: project.path().to_path_buf(),
        default_branch: built,
    })
}

/// The failure of an add that never got as far as registering.
///
/// `kept` names the object store **only where there is one**: a run that fell
/// over at `git init --bare` kept nothing, and a remedy pointing at a directory
/// that was never created is worse than no remedy at all.
fn aborted(project: &ProjectDirectory, problem: Problem) -> EngineError {
    let repo = project.source_repo();

    EngineError::ProjectAddAborted {
        name: project.name().to_string(),
        kept: repo.is_dir().then_some(repo),
        problem,
    }
}

/// The name the project will take: the one the user gave, or the last segment
/// of the source's path.
///
/// A collision is **refused with a suggestion, never silently disambiguated** —
/// a generated default that renames itself behind the user's back is worse than
/// an error naming the conflict. Re-running with `--name` is a second intent
/// rather than a retry, because the user supplied new information in between.
fn resolve_name(
    request: &ProjectAddRequest,
    source: &ProjectSource,
    directories: &Directories,
) -> Result<ProjectName, EngineError> {
    let name = match &request.name {
        Some(chosen) => ProjectName::parse(chosen)?,
        // A forge that names a repo `MyProject` has not told bitplane what to
        // call the project, so this is a question for the user rather than a
        // charset the URL got wrong.
        None => source.default_name().map_err(|_| {
            let folded = source.suggested_name();
            let stem = folded.as_ref().map(ProjectName::as_str).unwrap_or_default();

            EngineError::DerivedNameInvalid {
                derived: source.last_segment(),
                suggestion: first_free(alternatives(folded.clone(), stem), directories),
            }
        })?,
    };

    if ProjectDirectory::of(directories, name.clone()).is_registered() {
        return Err(refuse_taken(request, &name, source, directories));
    }

    Ok(name)
}

/// The refusal for a name that is already a project.
///
/// A **derived** name carries the first free alternative: `acme-codestyle` for
/// `acme/codestyle`, then `codestyle-2` and up where that is taken too. A name
/// the user chose carries none — the suggestion answers *"what should the
/// default have been"*, and they did not use the default.
fn refuse_taken(
    request: &ProjectAddRequest,
    name: &ProjectName,
    source: &ProjectSource,
    directories: &Directories,
) -> EngineError {
    if request.name.is_some() {
        return EngineError::ProjectNameTaken {
            name: name.to_string(),
            suggestion: None,
        };
    }

    EngineError::ProjectNameTaken {
        name: name.to_string(),
        suggestion: first_free(
            alternatives(source.qualified_name(), name.as_str()),
            directories,
        ),
    }
}

/// What to offer instead of a name bitplane will not take, best first: the
/// preferred alternative, then `<stem>-2` and up.
///
/// A `stem` that is not a project name contributes nothing, which is how an
/// unfoldable source ends up with no suggestion at all rather than with a
/// nonsense one.
fn alternatives(preferred: Option<ProjectName>, stem: &str) -> Vec<ProjectName> {
    preferred
        .into_iter()
        .chain((2..=9).filter_map(|ordinal| ProjectName::parse(&format!("{stem}-{ordinal}")).ok()))
        .collect()
}

/// The first candidate that is not already a project. `None` where every one is
/// taken, which is the point at which bitplane has nothing useful left to say
/// and the user picks a name themselves.
fn first_free(candidates: Vec<ProjectName>, directories: &Directories) -> Option<String> {
    candidates
        .into_iter()
        .find(|candidate| !ProjectDirectory::of(directories, candidate.clone()).is_registered())
        .map(|candidate| candidate.to_string())
}

/// The ADR-0005 sequence, in order, each step named so a failure says which one
/// gave up.
fn build_source_repo(
    project: &ProjectDirectory,
    source: &ProjectSource,
    context: &ProjectContext<'_>,
) -> Result<(), Problem> {
    let git = context.git;
    let repo = project.source_repo();
    let ProjectSource::Owned { url } = source else {
        unreachable!("project_add always builds an owned source");
    };

    step("repo.git", "could not be created", || {
        git.init_bare(project.path(), crate::project_dir::REPO_DIR_NAME)
    })?;
    step("origin", "could not be pointed at the url", || {
        git.set_origin(&repo, url)
    })?;
    step("origin", "could not be given bitplane's refspec", || {
        git.set_config(&repo, "remote.origin.fetch", CLONE_SHAPED_REFSPEC)
    })?;
    step("origin", "could not be fetched", || git.fetch(&repo, false))?;

    // Best effort, and deliberately so: a remote with no HEAD — an empty
    // repository — leaves the base unspecified, which is only an error at the
    // moment a new branch has to be cut from it (ADR-0005).
    step(
        "origin",
        "could not be asked for its default branch",
        || git.set_origin_head(&repo).map(drop),
    )
    .ok();

    // On bitplane-owned source repos only. A plane branch is cut from
    // `origin/main` and keeps it as its upstream, so git's default
    // `push.default = simple` would make a plain `git push` fail in a worktree
    // bitplane just handed the user. An adopted project's config is never
    // touched — bitplane does not own that repo.
    step("push.default", "could not be set to current", || {
        git.set_config(&repo, "push.default", "current")
    })
}

fn step(
    subject: &str,
    what: &str,
    run: impl FnOnce() -> Result<(), EngineError>,
) -> Result<(), Problem> {
    run().map_err(|error| Problem::about(subject, format!("{what}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_file::PROJECT_FILE_NAME;
    use crate::testing::scratch_dir;

    #[test]
    fn a_taken_default_name_is_refused_with_the_qualified_name_as_the_suggestion() {
        let directories = registry(&["codestyle"]);
        let source = ProjectSource::Owned {
            url: "git@gitlab.com:acme/codestyle.git".to_owned(),
        };

        let error = resolve_name(&request(None), &source, &directories).unwrap_err();

        assert_eq!(
            error,
            EngineError::ProjectNameTaken {
                name: "codestyle".to_owned(),
                suggestion: Some("acme-codestyle".to_owned()),
            }
        );
        assert_eq!(
            error.envelope().remedy.as_deref(),
            Some("acme-codestyle is free; re-run with --name acme-codestyle.")
        );
    }

    #[test]
    fn a_suggestion_that_is_taken_too_is_numbered_rather_than_offered() {
        let directories = registry(&["codestyle", "acme-codestyle"]);
        let source = ProjectSource::Owned {
            url: "git@gitlab.com:acme/codestyle.git".to_owned(),
        };

        let error = resolve_name(&request(None), &source, &directories).unwrap_err();

        assert_eq!(
            error,
            EngineError::ProjectNameTaken {
                name: "codestyle".to_owned(),
                suggestion: Some("codestyle-2".to_owned()),
            }
        );
    }

    #[test]
    fn a_taken_chosen_name_is_refused_with_nothing_to_suggest() {
        let directories = registry(&["codestyle"]);
        let source = ProjectSource::Owned {
            url: "git@gitlab.com:acme/other.git".to_owned(),
        };

        let error = resolve_name(&request(Some("codestyle")), &source, &directories).unwrap_err();

        assert_eq!(
            error,
            EngineError::ProjectNameTaken {
                name: "codestyle".to_owned(),
                suggestion: None,
            }
        );
        assert_eq!(error.exit_code(), crate::ExitCode::Usage);
    }

    #[test]
    fn a_name_the_url_derived_badly_names_the_flag_rather_than_the_charset() {
        let directories = registry(&[]);
        let source = ProjectSource::Owned {
            url: "git@gitlab.com:acme/MyProject.git".to_owned(),
        };

        let error = resolve_name(&request(None), &source, &directories).unwrap_err();

        assert_eq!(
            error,
            EngineError::DerivedNameInvalid {
                derived: "MyProject".to_owned(),
                suggestion: Some("myproject".to_owned()),
            }
        );
        assert_eq!(error.exit_code(), crate::ExitCode::Usage);
        assert_eq!(
            error.envelope().remedy.as_deref(),
            Some("Re-run with --name myproject."),
            "the user never typed this name; the URL did"
        );
    }

    #[test]
    fn a_free_name_resolves_to_itself() {
        let directories = registry(&[]);
        let source = ProjectSource::Owned {
            url: "git@gitlab.com:acme/codestyle.git".to_owned(),
        };

        assert_eq!(
            resolve_name(&request(None), &source, &directories)
                .unwrap()
                .as_str(),
            "codestyle"
        );
        assert_eq!(
            resolve_name(&request(Some("style")), &source, &directories)
                .unwrap()
                .as_str(),
            "style"
        );
    }

    fn request(name: Option<&str>) -> ProjectAddRequest {
        ProjectAddRequest {
            url: "git@gitlab.com:acme/codestyle.git".to_owned(),
            name: name.map(str::to_owned),
        }
    }

    fn registry(names: &[&str]) -> Directories {
        let dir = scratch_dir("project-add-names");
        let directories = Directories::new(dir.join("planes"), dir.join("projects"));

        for name in names {
            let project = ProjectDirectory::of(&directories, ProjectName::parse(name).unwrap());
            std::fs::create_dir_all(project.path()).unwrap();
            std::fs::write(project.path().join(PROJECT_FILE_NAME), "version = 1\n").unwrap();
        }

        directories
    }
}
