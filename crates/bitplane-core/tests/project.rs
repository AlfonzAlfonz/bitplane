//! `bp project add`, `list` and `fetch`, against real git repositories.
//!
//! Real rather than stubbed, because every acceptance criterion here is about
//! what git actually does with the repo bitplane built — which refs land where,
//! what a fetch writes, and what survives a failure. A fake git would only test
//! bitplane's idea of git, and the whole point of ADR-0005 is that bitplane's
//! idea of git was wrong twice.
//!
//! The "forge" in these tests is an ordinary local repository. A path is a git
//! URL, so `init --bare` + `remote add` + `fetch` against one exercises exactly
//! the code path a real forge would.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bitplane_core::lock::{DEFAULT_TIMEOUT, Lock};
use bitplane_core::member::RESERVED_SEGMENT;
use bitplane_core::project_add::CLONE_SHAPED_REFSPEC;
use bitplane_core::project_dir::{ProjectDirectory, REPO_DIR_NAME};
use bitplane_core::testing::{git, repository_with_one_commit, scratch_dir};
use bitplane_core::{
    Directories, Engine, EngineError, ExitCode, LocalEngine, Outcome, ProjectAddRequest,
    ProjectAdded, ProjectFetchRequest, ProjectFetched, ProjectListing, ProjectName, ProjectSource,
    Reader, SkipReason,
};

#[test]
fn the_source_repo_is_built_by_hand_with_the_clone_shaped_refspec() {
    let host = Host::new("project-add-shape");
    let forge = host.forge("codestyle");

    let added = host.add(&forge, None).unwrap();

    let repo = host.source_repo("codestyle");
    assert!(repo.join("HEAD").is_file(), "no bare repo at {repo:?}");
    assert_eq!(config(&repo, "core.bare"), "true");
    assert_eq!(
        config(&repo, "remote.origin.url"),
        forge.display().to_string()
    );
    assert_eq!(config(&repo, "remote.origin.fetch"), CLONE_SHAPED_REFSPEC);
    assert_eq!(
        config(&repo, "remote.origin.mirror"),
        "",
        "clone --mirror is forbidden: a worktree inherits mirror = true and \
         an ordinary git push then force-pushes the forge"
    );
    assert!(
        git(
            &repo,
            &["rev-parse", "--verify", "refs/remotes/origin/HEAD"]
        )
        .trim()
        .len()
            >= 40,
        "origin's head was never set"
    );
    assert_eq!(added.default_branch.as_deref(), Some("main"));
}

#[test]
fn the_local_branch_namespace_is_empty_after_the_fetch_and_holds_one_after_a_worktree() {
    let host = Host::new("project-add-namespace");
    let forge = host.forge("codestyle");
    git(&forge, &["branch", "release"]);

    host.add(&forge, None).unwrap();

    let repo = host.source_repo("codestyle");
    assert_eq!(
        local_branches(&repo),
        Vec::<String>::new(),
        "clone --bare would have copied the forge's branches in here"
    );
    assert_eq!(
        remote_branches(&repo),
        ["origin/main", "origin/release"],
        "the forge's branches live in refs/remotes/origin/*"
    );

    git(
        &repo,
        &[
            "worktree",
            "add",
            &host.root.join("wt").display().to_string(),
            "-b",
            "feat-login",
            "origin/main",
        ],
    );

    assert_eq!(
        local_branches(&repo),
        ["feat-login"],
        "refs/heads/* is exactly the branches plane members were created on"
    );
}

#[test]
fn push_default_is_set_to_current_on_the_repo_bitplane_built() {
    let host = Host::new("project-add-push-default");
    let forge = host.forge("codestyle");

    host.add(&forge, None).unwrap();

    assert_eq!(
        config(&host.source_repo("codestyle"), "push.default"),
        "current",
        "a plane branch keeps origin/main as its upstream, so git's default \
         push.default = simple would make a plain git push fail"
    );
}

#[test]
fn the_project_file_is_a_version_a_name_and_an_owned_source() {
    let host = Host::new("project-add-file");
    let forge = host.forge("codestyle");

    let added = host.add(&forge, None).unwrap();

    assert_eq!(added.name.as_str(), "codestyle", "the last path segment");
    assert_eq!(added.directory, host.projects().join("codestyle"));

    let file = host.project_file("codestyle");
    assert_eq!(file.version, 1);
    assert_eq!(file.name.as_str(), "codestyle");
    assert_eq!(
        file.source,
        ProjectSource::Owned {
            url: forge.display().to_string()
        }
    );
    assert!(file.source.is_owned());
}

#[test]
fn a_name_the_user_chose_beats_the_one_derived_from_the_url() {
    let host = Host::new("project-add-named");
    let forge = host.forge("codestyle");

    let added = host.add(&forge, Some("style")).unwrap();

    assert_eq!(added.name.as_str(), "style");
    assert!(host.projects().join("style").join("repo.git").is_dir());
}

#[test]
fn a_taken_default_name_is_refused_with_a_suggestion_and_never_disambiguated() {
    let host = Host::new("project-add-collision");
    let first = host.forge_at("acme/codestyle");
    let second = host.forge_at("other/codestyle");

    host.add(&first, None).unwrap();
    let error = host.add(&second, None).unwrap_err();

    assert_eq!(
        error,
        EngineError::ProjectNameTaken {
            name: "codestyle".to_owned(),
            suggestion: Some("other-codestyle".to_owned()),
        }
    );
    assert_eq!(error.exit_code(), ExitCode::Usage);
    assert_eq!(error.to_string(), "codestyle is already a project");
    assert_eq!(
        error.envelope().remedy.as_deref(),
        Some("other-codestyle is free; re-run with --name other-codestyle.")
    );
    assert_eq!(
        host.list().unwrap().projects.len(),
        1,
        "the second add must not have registered anything"
    );
}

#[test]
fn a_repo_landing_on_the_reserved_segment_is_refused_before_anything_is_created() {
    let host = Host::new("project-add-reserved");

    let error = host
        .add_url("git@gitlab.com:.bitplane/tools.git", None)
        .unwrap_err();

    assert_eq!(
        error,
        EngineError::ReservedPathSegment {
            path: ".bitplane/tools".to_owned()
        }
    );
    assert_eq!(error.exit_code(), ExitCode::Usage);
    assert!(
        !host.projects().join("tools").exists(),
        "nothing is created for a repo no plane could ever hold"
    );
}

#[test]
fn a_failed_add_keeps_the_object_store_and_unwinds_only_the_registration() {
    let host = Host::new("project-add-aborted");
    // A directory that is not a repository: `init --bare`, `remote add` and the
    // refspec all land, and the fetch is what gives up.
    let forge = host.root.join("forges").join("codestyle");
    fs::create_dir_all(&forge).unwrap();

    let error = host.add(&forge, None).unwrap_err();

    let kept = host.source_repo("codestyle");
    let EngineError::ProjectAddAborted {
        name, kept: left, ..
    } = &error
    else {
        panic!("not an aborted add: {error:?}");
    };
    assert_eq!(name, "codestyle");
    assert_eq!(
        left.as_ref(),
        Some(&kept),
        "the object store is what is kept"
    );
    assert_eq!(error.exit_code(), ExitCode::Failure);
    assert_eq!(error.to_string(), "@codestyle was not registered");

    let envelope = error.envelope();
    assert_eq!(envelope.problems.len(), 1);
    assert_eq!(envelope.problems[0].subject.as_deref(), Some("origin"));
    assert!(
        envelope.problems[0]
            .message
            .starts_with("could not be fetched: "),
        "got {}",
        envelope.problems[0].message
    );
    assert_eq!(
        envelope.remedy.as_deref(),
        Some(
            format!(
                "The objects fetched so far were kept at {}; \
                 re-running bp project add will reuse them.",
                kept.display()
            )
            .as_str()
        )
    );

    assert!(kept.is_dir(), "the object store is the expensive part");
    assert!(
        !host
            .projects()
            .join("codestyle")
            .join("project.toml")
            .exists(),
        "no project file was written"
    );
}

#[test]
fn what_a_failed_add_left_is_not_a_project_and_the_retry_reuses_it() {
    let host = Host::new("project-add-retry");
    let forge = host.root.join("forges").join("codestyle");
    fs::create_dir_all(&forge).unwrap();

    host.add(&forge, None).unwrap_err();

    assert!(
        host.list().unwrap().projects.is_empty(),
        "a directory without a project file is not a project"
    );

    // A marker inside the object store the retry must not throw away.
    let kept = host.source_repo("codestyle");
    fs::write(kept.join("kept-by-the-first-attempt"), "objects\n").unwrap();

    repository_with_one_commit(&forge);
    let added = host.add(&forge, None).unwrap();

    assert_eq!(added.name.as_str(), "codestyle");
    assert!(
        kept.join("kept-by-the-first-attempt").exists(),
        "init --bare + fetch is resumable in a way clone is not"
    );
    assert_eq!(host.list().unwrap().projects.len(), 1);
}

#[test]
fn one_sentinel_per_project_directory_guards_the_file_and_every_write_to_the_repo() {
    let host = Host::new("project-lock-one-sentinel");
    let forge = host.forge("codestyle");

    let sentinel = host
        .projects()
        .join("codestyle")
        .join(RESERVED_SEGMENT)
        .join("lock");
    fs::create_dir_all(sentinel.parent().unwrap()).unwrap();

    // The add takes it before writing `project.toml`, and says so when it is
    // contended.
    held_while(&sentinel, || host.add(&forge, None).unwrap());
    assert_eq!(host.announcements(), vec![sentinel.clone()]);

    // And the fetch takes the very same one before writing to the source repo,
    // which is what "one sentinel guards both" means.
    held_while(&sentinel, || host.fetch(&[]).unwrap());
    assert_eq!(host.announcements(), vec![sentinel]);
}

#[test]
fn the_listing_is_the_projects_directory_and_a_leftover_is_not_in_it() {
    let host = Host::new("project-list");
    host.add(&host.forge("api"), None).unwrap();
    host.add(&host.forge("codestyle"), None).unwrap();
    fs::create_dir_all(host.projects().join("half-added").join(REPO_DIR_NAME)).unwrap();

    let listing = host.list().unwrap();

    assert_eq!(
        listing
            .projects
            .iter()
            .map(|row| row.project.to_string())
            .collect::<Vec<String>>(),
        ["api", "codestyle"],
        "a directory without a project file is not a project and is skipped"
    );
    assert!(!listing.has_unreadable());
    assert!(
        listing
            .projects
            .iter()
            .all(|row| row.value().unwrap().source_repo_present)
    );
}

#[test]
fn a_fetch_updates_the_remote_refs_prunes_a_deleted_branch_and_never_prunes_tags() {
    let host = Host::new("project-fetch-prune");
    let forge = host.forge("codestyle");
    git(&forge, &["branch", "doomed"]);
    git(&forge, &["tag", "v1"]);

    host.add(&forge, None).unwrap();
    let repo = host.source_repo("codestyle");
    assert_eq!(remote_branches(&repo), ["origin/doomed", "origin/main"]);
    assert_eq!(tags(&repo), ["v1"]);

    // Up to date: nothing moved on the forge.
    assert_eq!(updates(&host.fetch(&["@codestyle"]).unwrap()), [0]);

    commit(&forge, "more");
    git(&forge, &["branch", "-D", "doomed"]);
    git(&forge, &["tag", "-d", "v1"]);

    assert_eq!(
        updates(&host.fetch(&["codestyle"]).unwrap()),
        [2],
        "main moved and doomed was pruned; the sigil is optional here"
    );
    assert_eq!(remote_branches(&repo), ["origin/main"]);
    assert_eq!(
        tags(&repo),
        ["v1"],
        "refs/tags/* is a namespace bitplane does not own"
    );
}

#[test]
fn a_fetch_succeeds_while_a_plane_worktree_on_a_branch_of_the_same_name_is_live() {
    let host = Host::new("project-fetch-live-worktree");
    let forge = host.forge("codestyle");

    host.add(&forge, None).unwrap();
    let repo = host.source_repo("codestyle");

    // A plane branch called `main`, checked out — the exact case that makes a
    // mirror-shaped refspec exit 128 and fetch nothing.
    git(
        &repo,
        &[
            "worktree",
            "add",
            &host.root.join("wt").display().to_string(),
            "-b",
            "main",
            "origin/main",
        ],
    );
    commit(&forge, "pushed by a colleague");

    let fetched = host.fetch(&[]).unwrap();

    assert_eq!(updates(&fetched), [1]);
    assert!(fetched.failure().is_none());
    assert_eq!(
        local_branches(&repo),
        ["main"],
        "the fetch writes only to refs/remotes/origin/*"
    );
}

#[test]
fn with_no_project_named_every_registered_project_is_taken_in_turn() {
    let host = Host::new("project-fetch-all");
    host.add(&host.forge("api"), None).unwrap();
    host.add(&host.forge("codestyle"), None).unwrap();

    let fetched = host.fetch(&[]).unwrap();

    assert_eq!(
        fetched
            .projects
            .iter()
            .map(|row| row.project.to_string())
            .collect::<Vec<String>>(),
        ["api", "codestyle"]
    );
    assert!(fetched.failure().is_none());
}

#[test]
fn an_adopted_project_is_a_row_and_never_a_denominator() {
    let host = Host::new("project-fetch-adopted");
    host.add(&host.forge("api"), None).unwrap();
    let checkout = host.forge("bitplane");
    host.register_adopted("bitplane", &checkout);

    let fetched = host.fetch(&[]).unwrap();

    assert_eq!(
        fetched.projects[1].outcome,
        Outcome::Skipped(SkipReason::NothingToFetch)
    );
    assert!(fetched.failure().is_none(), "nothing failed");
    assert_eq!(updates(&fetched), [0], "only @api had anything to fetch");
}

#[test]
fn one_unreachable_forge_is_a_row_and_every_other_project_still_gets_its_turn() {
    let host = Host::new("project-fetch-failure");
    host.add(&host.forge("api"), None).unwrap();
    let codestyle = host.forge("codestyle");
    host.add(&codestyle, None).unwrap();
    host.register_adopted("bitplane", &host.forge("bitplane"));
    fs::remove_dir_all(&codestyle).unwrap();

    // In name order: api, bitplane, codestyle.
    let fetched = host.fetch(&[]).unwrap();

    assert_eq!(updates(&fetched), [0], "api was up to date");
    assert_eq!(
        fetched.projects[1].outcome,
        Outcome::Skipped(SkipReason::NothingToFetch)
    );
    assert!(fetched.projects[2].is_failure());

    let error = fetched.failure().expect("one fetch failed");
    assert_eq!(
        error.to_string(),
        "1 of 2 fetchable projects could not be fetched",
        "an adopted project is a row, never a denominator"
    );
    assert_eq!(error.exit_code(), ExitCode::Failure);
    assert_eq!(
        error.envelope().problems[0].subject.as_deref(),
        Some("@codestyle")
    );
}

#[test]
fn a_name_that_is_not_a_project_stops_the_run_before_anything_is_fetched() {
    let host = Host::new("project-fetch-unknown");
    host.add(&host.forge("api"), None).unwrap();

    let error = host.fetch(&["@codestyle"]).unwrap_err();

    assert_eq!(
        error,
        EngineError::ProjectNotFound {
            name: "codestyle".to_owned()
        }
    );
    assert_eq!(error.exit_code(), ExitCode::Usage);
}

fn config(repo: &Path, key: &str) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--get", key])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn local_branches(repo: &Path) -> Vec<String> {
    for_each_ref(repo, "refs/heads")
}

fn remote_branches(repo: &Path) -> Vec<String> {
    for_each_ref(repo, "refs/remotes/origin")
        .into_iter()
        // `refs/remotes/origin/HEAD` shortens to a bare `origin`. It is the
        // recorded default, not one of the forge's branches.
        .filter(|reference| reference != "origin")
        .collect()
}

fn tags(repo: &Path) -> Vec<String> {
    for_each_ref(repo, "refs/tags")
}

fn for_each_ref(repo: &Path, namespace: &str) -> Vec<String> {
    let mut found: Vec<String> = git(
        repo,
        &["for-each-ref", "--format=%(refname:short)", namespace],
    )
    .lines()
    .map(str::to_owned)
    .collect();
    found.sort();
    found
}

fn commit(repository: &Path, message: &str) {
    fs::write(repository.join(message), message).unwrap();
    git(repository, &["add", "."]);
    git(repository, &["commit", "--quiet", "--message", message]);
}

/// How many refs each row that actually fetched reports moving.
fn updates(fetched: &ProjectFetched) -> Vec<usize> {
    fetched
        .projects
        .iter()
        .filter_map(|row| row.value().map(|fetched| fetched.updated))
        .collect()
}

/// Holds `sentinel` for long enough that `run` has to wait for it.
fn held_while<T: Send>(sentinel: &Path, run: impl FnOnce() -> T + Send) -> T {
    let held = Lock::acquire(sentinel, DEFAULT_TIMEOUT, |_| {}).unwrap();

    std::thread::scope(|threads| {
        threads.spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(held);
        });
        run()
    })
}

/// A host of its own: its own projects directory and its own forges, so no test
/// can see another's.
struct Host {
    root: PathBuf,
    directories: Directories,
    announced: Arc<Mutex<Vec<PathBuf>>>,
}

impl Host {
    fn new(label: &str) -> Host {
        let root = scratch_dir(label);

        Host {
            directories: Directories::new(root.join("planes"), root.join("projects")),
            root,
            announced: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A repository standing in for a forge. A path is a git URL, so this is the
    /// same code path a real one takes.
    fn forge(&self, name: &str) -> PathBuf {
        self.forge_at(&format!("forges/{name}"))
    }

    fn forge_at(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        repository_with_one_commit(&path);
        path.canonicalize().expect("the forge was just made")
    }

    /// Every lock wait this host's runs have announced, taken once.
    fn announcements(&self) -> Vec<PathBuf> {
        self.announced.lock().unwrap().drain(..).collect()
    }

    fn projects(&self) -> PathBuf {
        self.directories.projects().to_path_buf()
    }

    fn source_repo(&self, name: &str) -> PathBuf {
        self.projects().join(name).join(REPO_DIR_NAME)
    }

    fn project_file(&self, name: &str) -> bitplane_core::ProjectFile {
        ProjectDirectory::of(&self.directories, ProjectName::parse(name).unwrap())
            .read_project_file()
            .unwrap()
    }

    /// Registers an adopted project by hand. `bp project adopt` is ticket 11;
    /// the fetch side of the asymmetry is testable without it.
    fn register_adopted(&self, name: &str, checkout: &Path) {
        let project = ProjectDirectory::of(&self.directories, ProjectName::parse(name).unwrap());
        fs::create_dir_all(project.path()).unwrap();
        project
            .write_project_file(&bitplane_core::ProjectFile::new(
                ProjectName::parse(name).unwrap(),
                ProjectSource::Adopted {
                    path: checkout.to_path_buf(),
                },
            ))
            .unwrap();
    }

    fn add(&self, forge: &Path, name: Option<&str>) -> Result<ProjectAdded, EngineError> {
        self.add_url(&forge.display().to_string(), name)
    }

    fn add_url(&self, url: &str, name: Option<&str>) -> Result<ProjectAdded, EngineError> {
        self.engine().project_add(ProjectAddRequest {
            url: url.to_owned(),
            name: name.map(str::to_owned),
        })
    }

    fn fetch(&self, projects: &[&str]) -> Result<ProjectFetched, EngineError> {
        self.engine().project_fetch(ProjectFetchRequest {
            projects: projects.iter().map(|name| (*name).to_owned()).collect(),
        })
    }

    fn list(&self) -> Result<ProjectListing, EngineError> {
        self.engine().project_list()
    }

    fn engine(&self) -> LocalEngine {
        let announced = Arc::clone(&self.announced);

        LocalEngine::new(self.directories.clone()).announcing_lock_waits(move |object| {
            announced.lock().unwrap().push(object.to_path_buf());
        })
    }
}
