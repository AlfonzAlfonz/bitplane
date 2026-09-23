//! `project_add`: the registry goes live, one owned project at a time.
//!
//! ```text
//! 1. resolve the name           ← refused if taken, nesting or underivable
//! 2. mkdir <project-dir>        ← reused where a previous attempt left one
//! 3. lock the sentinel
//! 4. git init --bare repo.git   ┐
//!    git remote add origin      │ the source repo, built rather than cloned
//!    git config …fetch = …      │ (ADR-0005)
//!    git fetch origin           │
//!    git remote set-head -a     │
//!    git config push.default    ┘
//! 5. re-check nesting           ← the one check the lock cannot cover
//! 6. write project.toml         ← the registration, and the last step
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
use crate::project_dir::{self, ProjectDirectory};
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

    // Nesting, re-checked here rather than under the lock above, because the
    // lock does not cover it: two adds whose names nest take **different**
    // sentinels, so the one thing that could have registered a blocker is
    // another process, during the fetch that just happened. Checked last, so
    // the answer is as fresh as it can be made. What is left is the instant
    // between this and the rename, which only a lock on the whole registry
    // would close — and there is no registry file to lock (ADR-0002).
    if let Some(blocker) = project_dir::nesting_blocker(context.directories, project.name())? {
        return Err(EngineError::ProjectNameNests {
            name: project.name().to_string(),
            blocker: blocker.to_string(),
        });
    }

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

/// The name the project will take: the one the user gave, or the one derived
/// from the source.
///
/// A collision is **refused with a suggestion, never silently disambiguated** —
/// a generated default that renames itself behind the user's back is worse than
/// an error naming the conflict. Re-running with `--name` is a second intent
/// rather than a retry, because the user supplied new information in between.
///
/// Nesting is checked after that, and separately, because it has no remedy of
/// the same shape: `@acme` and `@acme/codestyle` cannot coexist at all
/// (ADR-0009).
fn resolve_name(
    request: &ProjectAddRequest,
    source: &ProjectSource,
    directories: &Directories,
) -> Result<ProjectName, EngineError> {
    let name = match &request.name {
        // Never lowercased, never folded: this is the user's own word for the
        // project, and rewriting it is the silent disambiguation the design
        // refuses everywhere else.
        Some(chosen) => ProjectName::parse(chosen)?,
        None => source.default_name()?,
    };

    if ProjectDirectory::of(directories, name.clone()).is_registered() {
        return Err(refuse_taken(request, &name, source, directories));
    }

    if let Some(blocker) = project_dir::nesting_blocker(directories, &name)? {
        return Err(EngineError::ProjectNameNests {
            name: name.to_string(),
            blocker: blocker.to_string(),
        });
    }

    Ok(name)
}

/// The refusal for a name that is already a project.
///
/// Three different things wear one error code, because the name is taken in all
/// three and what to do about it is not:
///
/// - the registered project came from the **same url**, so the repo is already
///   registered and there is nothing to do;
/// - the name was **derived**, so the ladder offers the next free alternative;
/// - the user **chose** it, so there is nothing to suggest — the suggestion
///   answers *"what should the default have been"*, and they did not use the
///   default.
fn refuse_taken(
    request: &ProjectAddRequest,
    name: &ProjectName,
    source: &ProjectSource,
    directories: &Directories,
) -> EngineError {
    let taken = |same_source, suggestion| EngineError::ProjectNameTaken {
        name: name.to_string(),
        same_source,
        suggestion,
    };

    if registered_from(directories, name).is_some_and(|registered| registered.same_repo_as(source))
    {
        return taken(true, None);
    }

    match request.name.is_some() {
        true => taken(false, None),
        false => taken(false, first_free(name, directories)),
    }
}

/// The source of the project already holding `name`, where its file can be
/// read. An unreadable registration answers nothing, which sends the refusal
/// down the ordinary collision path rather than claiming a match it cannot see.
fn registered_from(directories: &Directories, name: &ProjectName) -> Option<ProjectSource> {
    ProjectDirectory::of(directories, name.clone())
        .read_project_file()
        .ok()
        .map(|file| file.source)
}

/// The first alternative that is free, `<name>-2` and up.
///
/// The ordinal goes on the **last segment**, so `acme/api` yields `acme/api-2`
/// rather than a nested `acme/api/2`. A candidate that is taken **or that would
/// nest** is skipped: the whole value of the remedy line is that the command it
/// prints runs, and one that cannot is worse than none.
///
/// `None` where every candidate is spoken for, which is the point at which
/// bitplane has nothing useful left to say and the user picks a name.
fn first_free(name: &ProjectName, directories: &Directories) -> Option<String> {
    (2..=9)
        .filter_map(|ordinal| name.with_last_segment(&format!("{}-{ordinal}", name.last_segment())))
        .find(|candidate| {
            !ProjectDirectory::of(directories, candidate.clone()).is_registered()
                && !matches!(
                    project_dir::nesting_blocker(directories, candidate),
                    Ok(Some(_)) | Err(_)
                )
        })
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
    use crate::testing::scratch_dir;

    #[test]
    fn an_owned_project_takes_the_whole_forge_path_after_the_host() {
        let directories = registry(&[]);

        assert_eq!(
            resolve_name(
                &request(None),
                &forge("acme/platform/tooling/codestyle"),
                &directories
            )
            .unwrap()
            .as_str(),
            "acme/platform/tooling/codestyle"
        );
    }

    #[test]
    fn a_name_the_user_chose_is_taken_as_typed_and_never_lowercased() {
        let directories = registry(&[]);

        assert_eq!(
            resolve_name(
                &request(Some("style")),
                &forge("acme/codestyle"),
                &directories
            )
            .unwrap()
            .as_str(),
            "style"
        );
        assert!(
            matches!(
                resolve_name(
                    &request(Some("Style")),
                    &forge("acme/codestyle"),
                    &directories
                ),
                Err(EngineError::InvalidProjectName { .. })
            ),
            "--name is the user's stated intent, so it is checked rather than fixed"
        );
    }

    #[test]
    fn a_derived_name_is_lowercased_silently() {
        let directories = registry(&[]);

        assert_eq!(
            resolve_name(&request(None), &forge("Acme/MyProject"), &directories)
                .unwrap()
                .as_str(),
            "acme/myproject"
        );
    }

    #[test]
    fn a_derived_name_still_invalid_after_lowercasing_names_the_segment_and_stops() {
        let directories = registry(&[]);

        let error =
            resolve_name(&request(None), &forge("acme/code+style"), &directories).unwrap_err();

        assert_eq!(
            error,
            EngineError::DerivedNameInvalid {
                derived: "code+style".to_owned(),
            }
        );
        assert_eq!(error.exit_code(), crate::ExitCode::Usage);
        assert_eq!(
            error.envelope().remedy.as_deref(),
            Some(
                "Re-run with --name <name>, using lowercase letters, digits and . _ - \
                 in each / -separated segment."
            ),
            "no suggestion: a URL that cannot produce a name is one worth looking at"
        );
    }

    #[test]
    fn a_taken_name_from_the_same_url_is_reported_as_already_registered() {
        let directories = registry(&[("acme/codestyle", "acme/codestyle")]);

        let error =
            resolve_name(&request(None), &forge("acme/codestyle"), &directories).unwrap_err();

        assert_eq!(
            error,
            EngineError::ProjectNameTaken {
                name: "acme/codestyle".to_owned(),
                same_source: true,
                suggestion: None,
            }
        );
        assert_eq!(
            error.to_string(),
            "acme/codestyle is already registered from that url"
        );
        assert_eq!(
            error.envelope().remedy,
            None,
            "there is nothing to fix, so there is nothing to say"
        );
    }

    #[test]
    fn a_taken_name_from_a_different_url_climbs_the_ladder_on_the_last_segment() {
        let directories = registry(&[("acme/codestyle", "other/codestyle")]);

        let error =
            resolve_name(&request(None), &forge("acme/codestyle"), &directories).unwrap_err();

        assert_eq!(
            error,
            EngineError::ProjectNameTaken {
                name: "acme/codestyle".to_owned(),
                same_source: false,
                suggestion: Some("acme/codestyle-2".to_owned()),
            }
        );
        assert_eq!(
            error.envelope().remedy.as_deref(),
            Some("acme/codestyle-2 is free; re-run with --name acme/codestyle-2.")
        );
    }

    #[test]
    fn a_taken_chosen_name_is_refused_with_nothing_to_suggest() {
        let directories = registry(&[("codestyle", "acme/codestyle")]);

        let error = resolve_name(
            &request(Some("codestyle")),
            &forge("acme/other"),
            &directories,
        )
        .unwrap_err();

        assert_eq!(
            error,
            EngineError::ProjectNameTaken {
                name: "codestyle".to_owned(),
                same_source: false,
                suggestion: None,
            }
        );
        assert_eq!(error.exit_code(), crate::ExitCode::Usage);
    }

    #[test]
    fn the_ladder_skips_a_candidate_that_would_nest_as_well_as_one_that_is_taken() {
        let directories = registry(&[
            ("acme/api", "one/api"),
            ("acme/api-2", "two/api"),
            ("acme/api-3/inner", "three/api"),
        ]);

        let error = resolve_name(&request(None), &forge("acme/api"), &directories).unwrap_err();

        assert_eq!(
            error,
            EngineError::ProjectNameTaken {
                name: "acme/api".to_owned(),
                same_source: false,
                suggestion: Some("acme/api-4".to_owned()),
            },
            "acme/api-3 would contain a project, so the remedy would not run"
        );
    }

    #[test]
    fn a_name_that_would_nest_is_refused_in_both_directions() {
        let directories = registry(&[("acme", "acme/root")]);

        let error = resolve_name(
            &request(Some("acme/codestyle")),
            &forge("x/y"),
            &directories,
        )
        .unwrap_err();

        assert_eq!(
            error,
            EngineError::ProjectNameNests {
                name: "acme/codestyle".to_owned(),
                blocker: "acme".to_owned(),
            }
        );
        assert_eq!(
            error.to_string(),
            "acme/codestyle would sit inside the project acme"
        );
        assert_eq!(
            error.envelope().remedy.as_deref(),
            Some("Choose a name outside acme, or rename acme first.")
        );

        let outward = registry(&[("acme/codestyle", "acme/codestyle")]);
        let error = resolve_name(&request(Some("acme")), &forge("x/y"), &outward).unwrap_err();

        assert_eq!(
            error,
            EngineError::ProjectNameNests {
                name: "acme".to_owned(),
                blocker: "acme/codestyle".to_owned(),
            }
        );
        assert_eq!(
            error.to_string(),
            "acme would contain the project acme/codestyle"
        );
    }

    fn request(name: Option<&str>) -> ProjectAddRequest {
        ProjectAddRequest {
            url: "git@gitlab.com:acme/codestyle.git".to_owned(),
            name: name.map(str::to_owned),
        }
    }

    /// A source url with a host, so the whole path after it is the default
    /// name.
    fn forge(path: &str) -> ProjectSource {
        ProjectSource::Owned {
            url: format!("git@gitlab.com:{path}.git"),
        }
    }

    /// A registry of `(name, the forge path it was registered from)`.
    fn registry(projects: &[(&str, &str)]) -> Directories {
        let dir = scratch_dir("project-add-names");
        let directories = Directories::new(dir.join("planes"), dir.join("projects"));

        for (name, from) in projects {
            let name = ProjectName::parse(name).expect("a fixture name is a name");
            let project = ProjectDirectory::of(&directories, name.clone());
            std::fs::create_dir_all(project.path()).unwrap();
            project
                .write_project_file(&ProjectFile::new(name, forge(from)))
                .unwrap();
        }

        directories
    }
}
