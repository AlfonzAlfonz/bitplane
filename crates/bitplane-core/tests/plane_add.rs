//! `bp add`, against real git repositories and a real registry.
//!
//! Real rather than stubbed, because every acceptance criterion here is about
//! what is left on disk after a failure: which worktrees survive, which refs
//! survive, what `plane.toml` says, and — the one that matters most — that the
//! incomplete latch is never written.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bitplane_core::plane_dir::{BITPLANE_DIR, LATCH_NAME};
use bitplane_core::testing::{git, repository_with_one_commit, scratch_dir};
use bitplane_core::{
    BranchIntent, Directories, Engine, EngineError, ExitCode, LocalEngine, MemberRef,
    PlaneAddRequest, PlaneAdded, PlaneCreateRequest, PlaneCreated, PlaneFile, PlaneRef,
    ProjectAddRequest,
};

#[test]
fn a_member_joins_a_plane_that_already_has_one() {
    let host = Host::new("add-one");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha], "feat-login");

    let added = host.add(request(&[&beta], "feat-login")).unwrap();

    assert_eq!(added.id, plane.id.to_string());
    assert_eq!(added.members.len(), 1, "one row per member named");
    let landed = added.members[0].value().expect("beta was added");
    assert_eq!(landed.branch, "feat-login");
    assert_eq!(landed.path.to_string(), "repos/beta");
    assert_eq!(
        git(
            &plane.directory.join("repos/beta"),
            &["rev-parse", "--abbrev-ref", "HEAD"]
        )
        .trim(),
        "feat-login"
    );
    assert!(
        plane.directory.join("repos/alpha").is_dir(),
        "the members already there are left alone"
    );
}

#[test]
fn the_new_entry_is_appended_and_everything_around_it_survives() {
    let host = Host::new("add-plane-file");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha], "feat-login");

    // Hand-editing is tolerated, which is why the membership is a read-modify-
    // write rather than a re-render.
    let file = plane.directory.join("plane.toml");
    let text = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        format!("# do not reap, long-running migration\n{text}"),
    )
    .unwrap();

    host.add(request(&[&beta], "feat-login")).unwrap();

    let rewritten = fs::read_to_string(&file).unwrap();
    assert!(
        rewritten.starts_with("# do not reap, long-running migration"),
        "got {rewritten}"
    );

    let membership = PlaneFile::parse(&file, &rewritten).unwrap();
    assert_eq!(
        membership
            .members
            .iter()
            .map(|member| member.source.to_string())
            .collect::<Vec<String>>(),
        vec![alpha.display().to_string(), beta.display().to_string()],
        "the new entry lands last, which is the order every fan-out prints"
    );
}

#[test]
fn a_registered_project_joins_a_plane_of_ad_hoc_members() {
    let host = Host::new("add-project");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    host.project("codestyle");

    let added = host
        .add(PlaneAddRequest {
            members: vec!["@codestyle:feat-login".to_owned()],
            ..request(&[], "feat-login")
        })
        .unwrap();

    assert_eq!(
        added.members[0].member,
        MemberRef::parse("@codestyle").unwrap()
    );

    let membership = PlaneFile::read(&plane.directory.join("plane.toml")).unwrap();
    assert_eq!(
        membership
            .members
            .iter()
            .map(|member| (member.path.to_string(), member.source.to_string()))
            .collect::<Vec<(String, String)>>(),
        vec![
            ("repos/alpha".to_owned(), alpha.display().to_string()),
            ("forges/codestyle".to_owned(), "@codestyle".to_owned()),
        ],
        "the sigil form is what the plane file stores"
    );
}

#[test]
fn a_member_with_no_branch_is_refused_rather_than_given_one() {
    let host = Host::new("add-no-branch");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.plane(&[&alpha], "feat-login");

    let error = host
        .add(PlaneAddRequest {
            branch: None,
            ..request(&[&beta], "unused")
        })
        .unwrap_err();

    assert!(
        matches!(error, EngineError::BranchUnspecified { .. }),
        "nothing is derived from what the other members happen to be on: {error:?}"
    );
    assert_eq!(error.exit_code(), ExitCode::Usage);
}

#[test]
fn a_member_the_plane_already_holds_is_a_duplicate_and_never_an_adoption() {
    let host = Host::new("add-duplicate");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");

    let error = host.add(request(&[&alpha], "feat-other")).unwrap_err();

    assert_eq!(
        error,
        EngineError::DuplicateMember {
            member: alpha.display().to_string(),
            plane: Some(plane.id.to_string()),
        }
    );
    assert_eq!(error.exit_code(), ExitCode::Usage);
    assert_eq!(
        error.envelope().remedy,
        Some(format!(
            "Run bp rm {alpha}, then bp add {alpha}:<branch>.",
            alpha = alpha.display()
        )),
        "the fix is two commands, so the removal is visible instead of implied"
    );
}

#[test]
fn a_listed_member_whose_worktree_is_gone_is_still_a_duplicate() {
    let host = Host::new("add-duplicate-missing");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    fs::remove_dir_all(plane.directory.join("repos/alpha")).unwrap();

    let error = host.add(request(&[&alpha], "feat-other")).unwrap_err();

    assert!(
        matches!(error, EngineError::DuplicateMember { .. }),
        "add will not quietly take over a listed member: {error:?}"
    );
}

#[test]
fn a_failed_add_returns_the_plane_to_exactly_what_it_was() {
    let host = Host::new("add-aborted");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let doomed = failing_on_checkout(host.repository("doomed"));
    let plane = host.plane(&[&alpha], "feat-login");
    let before = fs::read_to_string(plane.directory.join("plane.toml")).unwrap();

    let error = host
        .add(request(&[&beta, &doomed], "feat-new"))
        .unwrap_err();

    let EngineError::AddAborted {
        plane: named,
        members,
        remnant,
        ..
    } = &error
    else {
        panic!("not an aborted add: {error:?}");
    };
    assert_eq!(named, &plane.id.to_string());
    assert!(!remnant, "the unwind had nothing to leave behind");
    assert_eq!(members.len(), 2, "the rows ride inside the error");

    assert_eq!(
        fs::read_to_string(plane.directory.join("plane.toml")).unwrap(),
        before,
        "only this run's entries are dropped, and it wrote two"
    );
    assert!(
        !plane.directory.join("repos/beta").exists(),
        "the worktree this run created is force-removed"
    );
    assert!(
        plane.directory.join("repos/alpha").is_dir(),
        "the member that was already there is untouched"
    );
    assert_eq!(
        branches_of(&beta),
        ["main"],
        "the branch this run cut goes with the worktree it cut it for"
    );
}

#[test]
fn add_never_sets_the_incomplete_latch() {
    let host = Host::new("add-no-latch");
    let alpha = host.repository("alpha");
    let doomed = failing_on_checkout(host.repository("doomed"));
    let plane = host.plane(&[&alpha], "feat-login");

    host.add(request(&[&doomed], "feat-new")).unwrap_err();

    assert!(
        !plane.directory.join(BITPLANE_DIR).join(LATCH_NAME).exists(),
        "the latch means nothing in here is yours; a plane full of work \
         carrying it would be a trapdoor"
    );
}

#[test]
fn a_plane_that_was_never_finished_being_created_is_declined() {
    let host = Host::new("add-latched");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha], "feat-login");
    fs::write(plane.directory.join(BITPLANE_DIR).join(LATCH_NAME), "").unwrap();

    let error = host.add(request(&[&beta], "feat-login")).unwrap_err();

    assert_eq!(
        error,
        EngineError::PlaneIncomplete {
            id: plane.id.to_string(),
        }
    );
    assert!(
        !plane.directory.join("repos/beta").exists(),
        "nothing was added to a plane headed for deletion"
    );
}

#[test]
fn the_default_intent_with_no_fetch_is_refused_before_anything_is_touched() {
    let host = Host::new("add-no-fetch");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha], "feat-login");

    let error = host
        .add(PlaneAddRequest {
            fetch: false,
            ..request(&[&beta], "feat-x")
        })
        .unwrap_err();

    assert_eq!(
        error,
        EngineError::BranchIntentRequiresFetch {
            branch: Some("feat-x".to_owned()),
        }
    );
    assert_eq!(error.exit_code(), ExitCode::Usage);
    assert_eq!(
        error.envelope().remedy.as_deref(),
        Some("Drop --no-fetch, or pass --new-branch to create feat-x deliberately."),
        "the remedy names both ways out"
    );
    assert!(!plane.directory.join("repos/beta").exists());
}

#[test]
fn an_explicit_intent_may_switch_the_fetch_off() {
    let host = Host::new("add-no-fetch-explicit");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha], "feat-login");

    host.add(PlaneAddRequest {
        fetch: false,
        intent: BranchIntent::RequireNew,
        ..request(&[&beta], "feat-x")
    })
    .unwrap();

    assert!(plane.directory.join("repos/beta").is_dir());
}

#[test]
fn a_member_landing_where_one_already_is_is_refused_by_both_names() {
    let host = Host::new("add-path-collision");
    let alpha = host.repository_at("one/repos/alpha");
    let other = host.repository_at("two/repos/alpha");
    let plane = host.plane(&[&alpha], "feat-login");

    let error = host.add(request(&[&other], "feat-login")).unwrap_err();

    let EngineError::MemberPathCollision {
        first,
        second,
        path,
    } = &error
    else {
        panic!("not a collision: {error:?}");
    };
    assert_eq!(first, &alpha.display().to_string());
    assert_eq!(second, &other.display().to_string());
    assert_eq!(path, "repos/alpha");
    assert!(plane.directory.join("repos/alpha").is_dir());
}

#[test]
fn a_second_worktree_of_a_repository_the_plane_already_holds_is_refused() {
    let host = Host::new("add-same-repository");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");

    // A linked worktree of `alpha` is a different path and a different derived
    // worktree path, but the same repository — and a plane holds at most one
    // worktree per repository.
    let linked = host.root.join("elsewhere/alpha-side");
    fs::create_dir_all(linked.parent().unwrap()).unwrap();
    git(
        &alpha,
        &[
            "worktree",
            "add",
            "-b",
            "side",
            &linked.display().to_string(),
        ],
    );

    let error = host.add(request(&[&linked], "feat-other")).unwrap_err();

    assert!(
        matches!(error, EngineError::SameRepository { .. }),
        "got {error:?}"
    );
    assert!(!plane.directory.join("alpha-side").exists());
}

#[test]
fn a_branch_the_forge_already_has_is_joined_rather_than_started() {
    let host = Host::new("add-forge-branch");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha], "feat-login");
    let forge = host.project("codestyle");

    // A colleague pushes after the project was registered, so the source repo
    // has not seen the branch yet.
    git(&forge, &["branch", "colleagues-branch"]);

    let added = host
        .add(PlaneAddRequest {
            members: vec!["@codestyle:colleagues-branch".to_owned()],
            ..request(&[], "unused")
        })
        .unwrap();

    let landed = added.members[0].value().expect("codestyle was added");
    assert_eq!(landed.branch, "colleagues-branch");
    assert!(
        !landed.created_branch,
        "joining a colleague's work is not starting your own"
    );
    assert_eq!(
        git(
            &host.planes().join(added.id).join(landed.path.as_path()),
            &[
                "rev-parse",
                "colleagues-branch",
                "refs/remotes/origin/colleagues-branch"
            ],
        )
        .lines()
        .collect::<std::collections::BTreeSet<&str>>()
        .len(),
        1,
        "the worktree is on the forge's branch, not on an unrelated namesake"
    );
}

#[test]
fn the_fan_out_is_genuinely_parallel_across_members() {
    // Per-source-repo locks cannot contend within one plane — a plane holds at
    // most one worktree per repository — so nothing serialises the build but
    // the worker cap, which is four.
    let host = Host::new("add-parallel");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha], "feat-login");

    let dawdlers: Vec<PathBuf> = ["one", "two", "three", "four"]
        .iter()
        .map(|name| dawdle_on_checkout(host.repository(name)))
        .collect();
    let members: Vec<&Path> = dawdlers.iter().map(PathBuf::as_path).collect();

    let started = Instant::now();
    host.add(request(&members, "feat-new")).unwrap();
    let took = started.elapsed();

    assert!(
        took < DAWDLE * 3,
        "four members that each dawdle {DAWDLE:?} took {took:?}; \
         serially that would be {:?}",
        DAWDLE * 4
    );
}

/// Long enough that four of them in series cannot be mistaken for four in
/// parallel, and short enough that the test is not itself a dawdle.
const DAWDLE: Duration = Duration::from_secs(2);

/// A repository whose `post-checkout` sleeps, so the time a build takes is
/// dominated by how many of them run at once. Run in the *new worktree*, which
/// is why the fixture writes it into the source repo's hooks.
fn dawdle_on_checkout(repository: PathBuf) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let hook = repository.join(".git").join("hooks").join("post-checkout");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(&hook, format!("#!/bin/sh\nsleep {}\n", DAWDLE.as_secs())).unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

    repository
}

/// A repository whose `post-checkout` fails, so `git worktree add` registers
/// its admin entry, cuts its branch, and *then* exits non-zero.
fn failing_on_checkout(repository: PathBuf) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let hook = repository.join(".git").join("hooks").join("post-checkout");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

    repository
}

fn branches_of(repository: &Path) -> Vec<String> {
    git(repository, &["branch", "--format=%(refname:short)"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn request(members: &[&Path], branch: &str) -> PlaneAddRequest {
    PlaneAddRequest {
        plane: PlaneRef::Id {
            id: "auth-work".to_owned(),
        },
        members: members
            .iter()
            .map(|member| member.display().to_string())
            .collect(),
        branch: Some(branch.to_owned()),
        intent: BranchIntent::Resolve,
        fetch: true,
    }
}

/// A host of its own: its own planes directory, its own projects directory, and
/// its own repositories, so no test can see another's.
struct Host {
    root: PathBuf,
    directories: Directories,
}

impl Host {
    fn new(label: &str) -> Host {
        let root = scratch_dir(label);

        Host {
            directories: Directories::new(root.join("planes"), root.join("projects")),
            root,
        }
    }

    fn repository(&self, name: &str) -> PathBuf {
        self.repository_at(&format!("repos/{name}"))
    }

    fn repository_at(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        repository_with_one_commit(&path);
        path.canonicalize().expect("the repository was just made")
    }

    /// An owned project, registered the way `bp project add` registers one. A
    /// path is a git URL, so the "forge" is an ordinary local repository and
    /// this is the code path a real one takes.
    fn project(&self, name: &str) -> PathBuf {
        let forge = self.repository_at(&format!("forges/{name}"));

        self.engine()
            .project_add(ProjectAddRequest {
                url: forge.display().to_string(),
                name: Some(name.to_owned()),
            })
            .expect("the fixture project was registered");

        forge
    }

    fn planes(&self) -> PathBuf {
        self.directories.planes().to_path_buf()
    }

    fn plane(&self, members: &[&Path], branch: &str) -> PlaneCreated {
        self.engine()
            .plane_create(PlaneCreateRequest {
                members: members
                    .iter()
                    .map(|member| member.display().to_string())
                    .collect(),
                branch: Some(branch.to_owned()),
                id: Some("auth-work".to_owned()),
                intent: BranchIntent::Resolve,
                fetch: true,
            })
            .expect("the fixture plane was created")
    }

    fn add(&self, request: PlaneAddRequest) -> Result<PlaneAdded, EngineError> {
        self.engine().plane_add(request)
    }

    fn engine(&self) -> LocalEngine {
        LocalEngine::new(self.directories.clone())
    }
}
