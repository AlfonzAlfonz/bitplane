//! `project_fetch`: bringing an owned project's source repo up to date.
//!
//! ```sh
//! git -C repo.git fetch --prune origin
//! ```
//!
//! **The configured refspec, in full** — not "only what some plane needs".
//! Fetching selectively saves nothing on one round trip and would turn a
//! colleague's new branch into a second fetch.
//!
//! `--prune` is safe **because of the refspec** `project_add` set:
//! `refs/remotes/origin/*` is a pure cache of the forge and holds nothing a
//! user owns, so pruning cannot lose work. It would have been unsafe under
//! `+refs/*:refs/*`. **Tags are left at git's default and never pruned** —
//! `refs/tags/*` is a namespace shared with every worktree bitplane does not
//! own.
//!
//! A fetch **succeeds while a plane worktree on a branch of the same name is
//! live**, because it writes only to `refs/remotes/origin/*`, which nothing
//! ever checks out. That is the whole reason the refspec is not left at a
//! default: under `+refs/*:refs/*` git exits 128 and fetches nothing the moment
//! any plane branch also exists on the forge.
//!
//! An **adopted** project has nothing to fetch — its source repo is the user's
//! checkout, and bitplane writes nothing there. Naming one is not an error; it
//! is a row, and never a denominator.
//!
//! The fan-out is parallel across projects, which is sound because the lock
//! attaches to the **project directory**: fetches of different projects can
//! never contend.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::directories::Directories;
use crate::error::EngineError;
use crate::interrupt::Interrupt;
use crate::member::{MemberRef, ProjectName};
use crate::outcome::{Outcome, PerProject, SkipReason};
use crate::project_dir::{self, ProjectDirectory};

use crate::repo::Git;
use crate::wire::{Fetched, ProjectFetchRequest, ProjectFetched};

/// How many projects are fetched at once. The cap exists because a host with
/// thirty projects should not open thirty connections to a forge.
const MAX_WORKERS: usize = 4;

/// Everything `project_fetch` needs that is not in the request.
pub struct FetchContext<'a> {
    pub directories: &'a Directories,
    pub git: &'a Git,
    pub interrupt: Interrupt,
    /// Called once per contended project lock.
    pub on_lock_wait: &'a (dyn Fn(&Path) + Sync),
}

/// Fetches the projects named, or every registered project where none is.
pub fn project_fetch(
    request: &ProjectFetchRequest,
    context: &FetchContext<'_>,
) -> Result<ProjectFetched, EngineError> {
    let projects = chosen(request, context.directories)?;
    let rows = fetch_all(&projects, context);

    Ok(ProjectFetched {
        interrupted: context.interrupt.is_raised(),
        projects: rows,
    })
}

/// Which projects this run is about.
///
/// A name that is not a project stops the run before anything is fetched — the
/// envelope `Err` is for a request that could not be acted on, and fetching
/// three projects and then reporting that a fourth does not exist would make
/// the typo cost a round trip.
fn chosen(
    request: &ProjectFetchRequest,
    directories: &Directories,
) -> Result<Vec<ProjectDirectory>, EngineError> {
    if request.projects.is_empty() {
        return project_dir::registered(directories);
    }

    let mut chosen: Vec<ProjectDirectory> = Vec::new();

    for spec in &request.projects {
        let project = project_dir::find(directories, &named(spec)?)?;

        // Named twice is one project, not two. A second turn would take the
        // lock again, print a second row for one fetch, and inflate the
        // denominator the failure count is reported against.
        if !chosen
            .iter()
            .any(|earlier| earlier.name() == project.name())
        {
            chosen.push(project);
        }
    }

    Ok(chosen)
}

/// A project as the user wrote it. The `@` sigil is **optional** here, because
/// no path is accepted in this slot and there is nothing to disambiguate.
fn named(spec: &str) -> Result<ProjectName, EngineError> {
    match MemberRef::parse(spec)? {
        MemberRef::Project(name) => Ok(name),
        MemberRef::Repo(_) => ProjectName::parse(spec),
    }
}

fn fetch_all(
    projects: &[ProjectDirectory],
    context: &FetchContext<'_>,
) -> Vec<PerProject<Fetched>> {
    let next = AtomicUsize::new(0);
    let rows: Vec<Mutex<Option<Outcome<Fetched>>>> =
        projects.iter().map(|_| Mutex::new(None)).collect();

    std::thread::scope(|threads| {
        for _ in 0..projects.len().min(MAX_WORKERS) {
            threads.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(project) = projects.get(index) else {
                        break;
                    };

                    // An earlier failure stops nothing here: every project the
                    // user named still gets its turn, because one unreachable
                    // forge says nothing about the next one.
                    let outcome = if context.interrupt.is_raised() {
                        Outcome::Skipped(SkipReason::Interrupted)
                    } else {
                        match fetch_one(project, context) {
                            Ok(outcome) => outcome,
                            Err(error) => Outcome::Failed(error),
                        }
                    };

                    *rows[index].lock().expect("no worker panics holding this") = Some(outcome);
                }
            });
        }
    });

    projects
        .iter()
        .zip(rows)
        .map(|(project, row)| {
            let outcome = row
                .into_inner()
                .expect("no worker panics holding this")
                .expect("every project is either fetched or skipped");
            PerProject::new(project.name().clone(), outcome)
        })
        .collect()
}

fn fetch_one(
    project: &ProjectDirectory,
    context: &FetchContext<'_>,
) -> Result<Outcome<Fetched>, EngineError> {
    let file = project.read_project_file()?;
    if !file.source.is_owned() {
        return Ok(Outcome::Skipped(SkipReason::NothingToFetch));
    }

    let repo = project.source_repo();
    if !repo.is_dir() {
        return Err(EngineError::Io {
            path: repo,
            message: "the source repo is missing".to_owned(),
        });
    }

    // One sentinel per project directory, guarding both `project.toml` and
    // every git command that writes to the source repo.
    let _lock = project.lock(context.on_lock_wait)?;

    let before = refs_of(&repo, context.git)?;
    context.git.fetch(&repo, true)?;
    let after = refs_of(&repo, context.git)?;

    Ok(Outcome::Ok(Fetched {
        updated: changed(&before, &after),
    }))
}

/// Git reports what a fetch did as prose on stderr, so the count comes from the
/// refs themselves: taken before and after, a ref added, removed or moved all
/// count alike.
fn refs_of(repo: &Path, git: &Git) -> Result<BTreeMap<String, String>, EngineError> {
    Ok(git.remote_refs(repo)?.into_iter().collect())
}

fn changed(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> usize {
    let moved_or_added = after
        .iter()
        .filter(|(reference, object)| before.get(*reference) != Some(object))
        .count();
    let pruned = before
        .keys()
        .filter(|reference| !after.contains_key(*reference))
        .count();

    moved_or_added + pruned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_may_be_named_with_or_without_the_sigil() {
        assert_eq!(named("@codestyle").unwrap().as_str(), "codestyle");
        assert_eq!(named("codestyle").unwrap().as_str(), "codestyle");
    }

    #[test]
    fn something_that_is_not_a_project_name_is_refused_in_that_slot() {
        for spec in ["@Codestyle", "/Users/alfonz/projects/bitplane", ""] {
            assert!(named(spec).is_err(), "{spec} should not be a project name");
        }
    }

    #[test]
    fn a_project_named_twice_is_fetched_once() {
        let dir = crate::testing::scratch_dir("project-fetch-duplicates");
        let directories = Directories::new(dir.join("planes"), dir.join("projects"));
        for name in ["api", "codestyle"] {
            let project = ProjectDirectory::of(&directories, ProjectName::parse(name).unwrap());
            std::fs::create_dir_all(project.path()).unwrap();
            std::fs::write(project.project_file(), "version = 1\n").unwrap();
        }

        let chosen = chosen(
            &ProjectFetchRequest {
                projects: ["@api", "api", "codestyle", "@api"]
                    .map(str::to_owned)
                    .to_vec(),
            },
            &directories,
        )
        .unwrap();

        assert_eq!(
            chosen
                .iter()
                .map(|project| project.name().to_string())
                .collect::<Vec<String>>(),
            ["api", "codestyle"],
            "a second turn would lock again, print a second row and inflate the \
             denominator the failure count is reported against"
        );
    }

    #[test]
    fn a_ref_added_moved_or_pruned_each_count_as_one_update() {
        let before = map(&[("origin/main", "aaa"), ("origin/gone", "bbb")]);
        let after = map(&[("origin/main", "ccc"), ("origin/feat", "ddd")]);

        assert_eq!(
            changed(&before, &after),
            3,
            "one moved, one added, one pruned"
        );
        assert_eq!(changed(&before, &before), 0, "up to date");
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }
}
