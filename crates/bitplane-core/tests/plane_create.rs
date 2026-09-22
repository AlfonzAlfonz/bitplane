//! `bp create`, against real git repositories.
//!
//! Real rather than stubbed, because every acceptance criterion here is about
//! what is left on disk after git has run — the claim, the latch, the plane
//! file, the worktrees, and what survives an abort. A fake git would only test
//! bitplane's idea of git.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

use bitplane_core::plane_dir::{BITPLANE_DIR, LATCH_NAME};
use bitplane_core::testing::{git, repository_with_one_commit, scratch_dir};
use bitplane_core::{
    BranchIntent, Directories, Engine, EngineError, ExitCode, Interrupt, LocalEngine, MemberRef,
    Occupant, Outcome, PlaneCreateRequest, PlaneCreated, PlaneFile, ProjectAddRequest, SkipReason,
};

#[test]
fn a_plane_of_two_ad_hoc_members_lands_a_worktree_each() {
    let host = Host::new("create-two");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");

    let created = host
        .create(request(&[&alpha, &beta], "feat-login"))
        .unwrap();

    assert_eq!(created.directory, host.planes().join(created.id.as_str()));
    assert_eq!(
        created.members.len(),
        2,
        "one row per member, never a count"
    );

    for (member, repository) in created.members.iter().zip([&alpha, &beta]) {
        let landed = member.value().expect("the member was created");
        assert_eq!(landed.branch, "feat-login");
        assert!(landed.created_branch, "feat-login did not exist yet");

        let worktree = created.directory.join(landed.path.as_path());
        assert!(
            worktree.join(".git").exists(),
            "no worktree at {worktree:?}"
        );
        assert_eq!(
            git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
            "feat-login"
        );
        assert_eq!(member.member, MemberRef::Repo(repository.clone()));
    }
}

#[test]
fn a_plane_mixes_registered_projects_and_ad_hoc_members() {
    let host = Host::new("create-mixed");
    host.project("codestyle");
    let alpha = host.repository("alpha");

    let created = host
        .create(PlaneCreateRequest {
            members: vec!["@codestyle".to_owned(), alpha.display().to_string()],
            ..request(&[], "feat-login")
        })
        .unwrap();

    assert_eq!(
        created.members[0].member,
        MemberRef::parse("@codestyle").unwrap(),
        "a project name never carries the sigil in memory"
    );

    let file = PlaneFile::read(&created.directory.join("plane.toml")).unwrap();
    assert_eq!(
        file.members
            .iter()
            .map(|member| (member.path.to_string(), member.source.to_string()))
            .collect::<Vec<(String, String)>>(),
        vec![
            ("forges/codestyle".to_owned(), "@codestyle".to_owned()),
            ("repos/alpha".to_owned(), alpha.display().to_string()),
        ],
        "the sigil form is what the plane file stores, and the only place it is stored"
    );
    assert!(created.directory.join("forges/codestyle/.git").exists());
}

#[test]
fn an_owned_project_lands_where_its_url_says_and_not_where_its_bare_repo_is() {
    let host = Host::new("create-owned-layout");
    host.project("codestyle");

    let created = host
        .create(PlaneCreateRequest {
            members: vec!["@codestyle".to_owned()],
            ..request(&[], "feat-login")
        })
        .unwrap();

    assert_eq!(
        created.members[0].value().unwrap().path.to_string(),
        "forges/codestyle",
        "derived from the project's source, not from <project-dir>/repo.git"
    );
}

#[test]
fn a_branch_pushed_after_registration_is_joined_because_the_fetch_ran_first() {
    let host = Host::new("create-fetch-first");
    let forge = host.project("codestyle");

    // The colleague pushes after `bp project add`, so only a fetch can make
    // this branch visible to the source repo.
    git(&forge, &["branch", "colleagues-branch"]);
    let theirs = git(&forge, &["rev-parse", "colleagues-branch"]);

    let created = host
        .create(PlaneCreateRequest {
            members: vec!["@codestyle:colleagues-branch".to_owned()],
            ..request(&[], "unused")
        })
        .unwrap();

    let landed = created.members[0].value().unwrap();
    assert!(
        !landed.created_branch,
        "the branch is the colleague's, not a new one of the same name"
    );
    assert_eq!(
        git(
            &created.directory.join(landed.path.as_path()),
            &["rev-parse", "HEAD"]
        ),
        theirs,
        "resolving against a stale source repo would have cut a new branch off main"
    );
}

#[test]
fn the_default_intent_with_no_fetch_is_refused_before_anything_is_claimed() {
    let host = Host::new("create-no-fetch");
    let alpha = host.repository("alpha");

    let error = host
        .create(PlaneCreateRequest {
            fetch: false,
            ..request(&[&alpha], "feat-x")
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
        Some("Drop --no-fetch, or pass --new-branch to create feat-x deliberately.")
    );
    assert!(host.planes_are_empty(), "nothing was touched at all");
}

#[test]
fn a_name_that_is_not_a_project_is_refused_before_anything_is_claimed() {
    let host = Host::new("create-unknown-project");

    let error = host
        .create(PlaneCreateRequest {
            members: vec!["@nowhere".to_owned()],
            ..request(&[], "feat-x")
        })
        .unwrap_err();

    assert_eq!(
        error,
        EngineError::ProjectNotFound {
            name: "nowhere".to_owned(),
        }
    );
    assert!(host.planes_are_empty());
}

#[test]
fn the_worktree_path_is_derived_from_the_source_by_the_layout_table() {
    let host = Host::new("create-layout");
    let alpha = host.repository("alpha");

    let created = host.create(request(&[&alpha], "feat-x")).unwrap();

    let landed = created.members[0].value().unwrap();
    assert_eq!(landed.path.to_string(), "repos/alpha");
}

#[test]
fn the_plane_file_is_three_keys_keyed_by_the_worktree_path() {
    let host = Host::new("create-plane-file");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");

    let created = host
        .create(request(&[&alpha, &beta], "feat-login"))
        .unwrap();

    let file = PlaneFile::read(&created.directory.join("plane.toml")).unwrap();
    assert_eq!(file.version, 1);
    assert_eq!(file.id, created.id);
    assert_eq!(
        file.members
            .iter()
            .map(|member| (member.path.to_string(), member.source.to_string()))
            .collect::<Vec<(String, String)>>(),
        vec![
            ("repos/alpha".to_owned(), alpha.display().to_string()),
            ("repos/beta".to_owned(), beta.display().to_string()),
        ]
    );
}

#[test]
fn the_latch_is_gone_once_the_last_worktree_lands() {
    let host = Host::new("create-latch-cleared");
    let alpha = host.repository("alpha");

    let created = host.create(request(&[&alpha], "feat-x")).unwrap();

    assert!(
        !created
            .directory
            .join(BITPLANE_DIR)
            .join(LATCH_NAME)
            .exists(),
        "the latch is the point of no return; it must be cleared"
    );
    assert!(created.directory.join(BITPLANE_DIR).join("lock").exists());
}

#[test]
fn a_generated_id_carries_the_reserved_prefix() {
    let host = Host::new("create-generated-id");
    let alpha = host.repository("alpha");

    let created = host.create(request(&[&alpha], "feat-x")).unwrap();

    assert!(created.id.is_generated(), "got {}", created.id);
}

#[test]
fn a_chosen_id_names_the_directory() {
    let host = Host::new("create-chosen-id");
    let alpha = host.repository("alpha");

    let created = host
        .create(PlaneCreateRequest {
            id: Some("auth-work".to_owned()),
            ..request(&[&alpha], "feat-x")
        })
        .unwrap();

    assert_eq!(created.id.as_str(), "auth-work");
    assert_eq!(created.directory, host.planes().join("auth-work"));
}

#[test]
fn a_chosen_id_already_in_use_fails_immediately_saying_what_is_there() {
    let host = Host::new("create-id-in-use");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");

    host.create(PlaneCreateRequest {
        id: Some("auth-work".to_owned()),
        ..request(&[&alpha], "feat-x")
    })
    .unwrap();

    let error = host
        .create(PlaneCreateRequest {
            id: Some("auth-work".to_owned()),
            ..request(&[&beta], "feat-y")
        })
        .unwrap_err();

    assert_eq!(
        error,
        EngineError::PlaneIdInUse {
            id: "auth-work".to_owned(),
            found: Occupant::Plane,
        }
    );
    assert_eq!(error.exit_code(), ExitCode::Usage);
}

#[test]
fn a_chosen_id_with_the_reserved_prefix_is_rejected() {
    let host = Host::new("create-reserved-id");
    let alpha = host.repository("alpha");

    let error = host
        .create(PlaneCreateRequest {
            id: Some("bp-mine".to_owned()),
            ..request(&[&alpha], "feat-x")
        })
        .unwrap_err();

    assert_eq!(error, EngineError::ReservedPlaneId);
    assert!(
        !host.planes().join("bp-mine").exists(),
        "a rejected id must claim nothing"
    );
}

#[test]
fn a_branch_already_checked_out_in_the_members_repo_is_refused_in_bitplanes_words() {
    let host = Host::new("create-branch-occupied");
    let alpha = host.repository("alpha");

    let error = host.create(request(&[&alpha], "main")).unwrap_err();

    assert_eq!(
        error,
        EngineError::BranchOccupied {
            branch: "main".to_owned(),
            repo: alpha.clone(),
            worktree: alpha.clone(),
            stale: false,
        }
    );

    let envelope = error.envelope();
    assert_eq!(envelope.error, "branch_occupied");
    assert_eq!(envelope.problems.len(), 1);
    assert!(
        !envelope.message.contains("fatal"),
        "git's own words must not leak: {}",
        envelope.message
    );
    assert!(
        host.planes().read_dir().is_err() || host.planes().read_dir().unwrap().count() == 0,
        "a refusal before the claim must leave no plane"
    );
}

#[test]
fn an_existing_branch_is_checked_out_rather_than_cut() {
    let host = Host::new("create-existing-branch");
    let alpha = host.repository("alpha");
    git(&alpha, &["branch", "feat-login"]);

    let created = host
        .create(PlaneCreateRequest {
            intent: BranchIntent::RequireExisting,
            ..request(&[&alpha], "feat-login")
        })
        .unwrap();

    let landed = created.members[0].value().unwrap();
    assert!(!landed.created_branch, "the branch was already there");
}

#[test]
fn a_branch_intent_that_cannot_be_honoured_is_refused_before_anything_is_claimed() {
    let host = Host::new("create-intent");
    let alpha = host.repository("alpha");
    git(&alpha, &["branch", "feat-login"]);

    let error = host
        .create(PlaneCreateRequest {
            intent: BranchIntent::RequireNew,
            ..request(&[&alpha], "feat-login")
        })
        .unwrap_err();

    assert!(
        matches!(error, EngineError::BranchIntentUnmet { .. }),
        "got {error:?}"
    );
    assert_eq!(error.exit_code(), ExitCode::Usage);
}

#[test]
fn a_member_named_twice_is_refused() {
    let host = Host::new("create-duplicate");
    let alpha = host.repository("alpha");

    let error = host
        .create(request(&[&alpha, &alpha], "feat-x"))
        .unwrap_err();

    assert_eq!(
        error,
        EngineError::DuplicateMember {
            member: alpha.display().to_string(),
            plane: None,
        }
    );
}

#[test]
fn two_members_that_would_land_at_the_same_path_are_refused() {
    let host = Host::new("create-path-collision");
    let one = host.repository_at("one/repos/alpha");
    let other = host.repository_at("two/repos/alpha");

    let error = host.create(request(&[&one, &other], "feat-x")).unwrap_err();

    assert!(
        matches!(error, EngineError::MemberPathCollision { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_member_with_no_branch_from_either_source_is_refused() {
    let host = Host::new("create-no-branch");
    let alpha = host.repository("alpha");

    let error = host
        .create(PlaneCreateRequest {
            branch: None,
            ..request(&[&alpha], "unused")
        })
        .unwrap_err();

    assert!(
        matches!(error, EngineError::BranchUnspecified { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_branch_suffix_beats_the_shared_branch_for_that_member_alone() {
    let host = Host::new("create-suffix");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");

    let created = host
        .create(PlaneCreateRequest {
            members: vec![
                alpha.display().to_string(),
                format!("{}:hotfix", beta.display()),
            ],
            branch: Some("feat-login".to_owned()),
            id: None,
            intent: BranchIntent::Resolve,
            fetch: true,
        })
        .unwrap();

    assert_eq!(created.members[0].value().unwrap().branch, "feat-login");
    assert_eq!(created.members[1].value().unwrap().branch, "hotfix");
}

#[test]
fn a_path_whose_last_segment_holds_a_colon_is_refused_rather_than_guessed_at() {
    let host = Host::new("create-ambiguous");
    let weird = host.repository_at("repos/weird:name");

    let error = host.create(request(&[&weird], "feat-x")).unwrap_err();

    assert_eq!(
        error,
        EngineError::MemberPathAmbiguous {
            spec: weird.display().to_string(),
        }
    );
}

#[test]
fn a_member_that_is_not_a_repository_is_refused() {
    let host = Host::new("create-not-a-repo");
    let scratch = host.root.join("not-a-repo");
    fs::create_dir_all(&scratch).unwrap();

    let error = host.create(request(&[&scratch], "feat-x")).unwrap_err();

    assert!(
        matches!(error, EngineError::MemberNotARepository { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_git_that_is_not_there_is_a_prerequisite_failure_not_a_bad_member() {
    // `git rev-parse` failing because there is no git to run is a different
    // thing from it failing because the path is not a repository, and the two
    // carry different exit codes — `4`, so a script knows to install something,
    // against `2`, which no amount of retrying fixes.
    let host = Host::new("create-no-git");
    let alpha = host.repository("alpha");
    let nowhere = host.root.join("no-git-here");
    fs::create_dir_all(&nowhere).unwrap();

    let error = LocalEngine::new(Directories::new(host.planes(), host.root.join("projects")))
        .with_git_search_path(&nowhere)
        .plane_create(request(&[&alpha], "feat-login"))
        .unwrap_err();

    assert_eq!(error, EngineError::GitMissing);
    assert_eq!(error.exit_code(), ExitCode::PrerequisiteMissing);
}

#[test]
fn a_failure_partway_through_unwinds_every_worktree_and_removes_the_plane() {
    let host = Host::new("create-abort");
    let alpha = host.repository("alpha");
    let beta = refusing_new_worktrees(host.repository("beta"));

    let error = host
        .create(request(&[&alpha, &beta], "feat-login"))
        .unwrap_err();

    let EngineError::CreateAborted {
        members,
        rollback,
        remnant,
        ..
    } = &error
    else {
        panic!("expected create_aborted, got {error:?}");
    };

    assert!(
        !remnant,
        "the unwind succeeded, so nothing is left behind: {rollback:?}"
    );
    assert_eq!(members.len(), 2, "the rows ride inside the error");
    assert!(
        members.iter().any(bitplane_core::PerMember::is_failure),
        "one row must say what went wrong"
    );
    assert!(
        rollback.iter().all(|row| !row.is_failure()),
        "every worktree this run created was taken back: {rollback:?}"
    );

    assert!(host.planes_are_empty(), "the plane directory must be gone");
    assert_eq!(error.exit_code(), ExitCode::Failure);
    assert!(error.envelope().remedy.is_some());

    // Both source repos are as they were: no worktree, and no branch this run
    // cut left dangling.
    for repository in [&alpha, &beta] {
        assert_eq!(
            worktrees_of(repository),
            1,
            "only {repository:?}'s own worktree should remain"
        );
        assert!(
            !branches_of(repository).contains(&"feat-login".to_owned()),
            "a branch this run cut must not survive the abort in {repository:?}"
        );
    }
}

#[test]
fn an_unwind_that_cannot_finish_keeps_the_latch_and_names_the_remnant() {
    let host = Host::new("create-abort-remnant");
    let alpha = host.repository("alpha");
    let beta = unwritable(host.repository("beta"));

    let error = host
        .create(request(&[&alpha, &beta], "feat-login"))
        .unwrap_err();

    let EngineError::CreateAborted {
        rollback, remnant, ..
    } = &error
    else {
        panic!("expected create_aborted, got {error:?}");
    };

    assert!(remnant, "the unwind did not finish: {rollback:?}");
    assert!(rollback.iter().any(|row| row.is_failure()));

    let envelope = error.envelope();
    assert!(
        envelope
            .remedy
            .as_deref()
            .is_some_and(|remedy| remedy.contains("bp destroy")),
        "the remedy must name the way out: {:?}",
        envelope.remedy
    );

    let plane = host.planes().join(error_plane_id(&error));
    assert!(
        plane.join(BITPLANE_DIR).join(LATCH_NAME).exists(),
        "the latch stays set, so the remnant is findable and free to discard"
    );
}

#[test]
fn a_worktree_registered_by_a_member_that_then_failed_is_still_taken_back() {
    // What a real Ctrl-C does: it reaches the whole process group, so a
    // `git worktree add` can register its admin entry and *then* die. The row
    // is `Failed`, and unwinding only the `Ok` rows would orphan that entry —
    // and the branch — in a repo bitplane does not own, pointing into a plane
    // directory about to be deleted. A `post-checkout` that exits non-zero is
    // the same shape, deterministically.
    let host = Host::new("create-abort-registered");
    let alpha = host.repository("alpha");
    let beta = failing_on_checkout(host.repository("beta"));

    let error = host
        .create(request(&[&alpha, &beta], "feat-login"))
        .unwrap_err();

    let EngineError::CreateAborted {
        members,
        rollback,
        remnant,
        ..
    } = &error
    else {
        panic!("expected create_aborted, got {error:?}");
    };

    assert!(members.iter().any(bitplane_core::PerMember::is_failure));
    assert!(!remnant, "the unwind finished: {rollback:?}");

    assert_eq!(
        worktrees_of(&beta),
        1,
        "the entry the failed member registered must be gone again"
    );
    assert!(
        !branches_of(&beta).contains(&"feat-login".to_owned()),
        "so must the branch it cut"
    );
    assert!(host.planes_are_empty());
}

#[test]
fn a_stale_worktree_entry_is_refused_with_a_remedy_the_user_can_act_on() {
    // After a hard kill, the source repo can hold an entry whose directory is
    // gone. Git calls that `prunable`, and still refuses to check the branch
    // out — so the refusal is right, but "check out a different branch there"
    // names a directory that is not there and cannot be followed.
    let host = Host::new("create-prunable");
    let alpha = host.repository("alpha");

    let abandoned = host.root.join("abandoned");
    git(
        &alpha,
        &[
            "worktree",
            "add",
            "-b",
            "feat-login",
            abandoned.to_str().unwrap(),
            "HEAD",
        ],
    );
    fs::remove_dir_all(&abandoned).unwrap();

    let error = host.create(request(&[&alpha], "feat-login")).unwrap_err();

    let EngineError::BranchOccupied {
        worktree, stale, ..
    } = &error
    else {
        panic!("expected branch_occupied, got {error:?}");
    };
    assert!(stale, "the holder's directory is gone");
    assert!(
        worktree.ends_with("abandoned"),
        "the remedy must name the holder git recorded, got {worktree:?}"
    );

    // Measured, and the reason this is a refusal rather than something to
    // ignore: git refuses the branch just as firmly when the holder is stale.
    // What changes is the remedy — the one bitplane gives has to be one the
    // user can act on, and "check out a different branch there" is not.
    let remedy = error.envelope().remedy.expect("a remedy");
    assert!(remedy.contains("git worktree prune"), "got {remedy}");
    assert!(remedy.contains(alpha.to_str().unwrap()), "got {remedy}");
    assert!(host.planes_are_empty());
}

#[test]
fn a_linked_worktree_of_a_member_is_not_a_second_member() {
    // The trap `--show-toplevel` walks into: a linked worktree *is* the root of
    // its own working tree, so "is this path a repository root?" says yes and
    // one plane ends up holding two worktrees of one repository — which the
    // layout cannot express and the parallel fan-out assumes cannot happen.
    let host = Host::new("create-linked-worktree");
    let alpha = host.repository("alpha");
    let side = host.root.join("side");
    git(
        &alpha,
        &[
            "worktree",
            "add",
            "-b",
            "side",
            side.to_str().unwrap(),
            "HEAD",
        ],
    );

    let error = host
        .create(request(&[&alpha, &side], "feat-login"))
        .unwrap_err();

    assert!(
        matches!(error, EngineError::SameRepository { .. }),
        "got {error:?}"
    );
    assert!(host.planes_are_empty());
}

#[test]
fn a_directory_inside_a_member_is_not_a_second_member() {
    // `git rev-parse` answers for the containing repository from any
    // subdirectory, so accepting one would put two worktrees of a single
    // repository into one plane — which the layout cannot express and the
    // parallel fan-out assumes cannot happen.
    let host = Host::new("create-subdirectory");
    let alpha = host.repository("alpha");
    let inside = alpha.join("src");
    fs::create_dir_all(&inside).unwrap();

    let error = host
        .create(request(&[&alpha, &inside], "feat-login"))
        .unwrap_err();

    let EngineError::MemberNotARepository { path, reason } = &error else {
        panic!("expected member_not_a_repository, got {error:?}");
    };
    assert_eq!(path, &inside);
    assert!(
        reason.contains("is inside the repository at"),
        "got {reason}"
    );
    assert!(host.planes_are_empty());
}

#[test]
fn an_interrupt_stops_scheduling_unwinds_and_answers_with_the_partial_vector() {
    static RAISED: AtomicUsize = AtomicUsize::new(0);
    let interrupt = Interrupt::on(&RAISED);
    interrupt.raise();

    let host = Host::new("create-interrupted").interrupted_by(interrupt);
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");

    let created = host
        .create(request(&[&alpha, &beta], "feat-login"))
        .unwrap();

    assert!(created.interrupted);
    assert_eq!(created.members.len(), 2, "the partial vector is the answer");
    for row in &created.members {
        assert!(
            matches!(row.outcome, Outcome::Skipped(SkipReason::Interrupted)),
            "got {:?}",
            row.outcome
        );
    }
    assert!(
        !created.directory.exists(),
        "the abort path runs on an interrupt too"
    );
}

/// A repository that will not accept a new worktree, but is otherwise writable
/// — so `worktree add` fails and the unwind can still finish.
fn refusing_new_worktrees(repository: PathBuf) -> PathBuf {
    let worktrees = repository.join(".git").join("worktrees");
    fs::create_dir_all(&worktrees).unwrap();
    read_only(&worktrees);

    repository
}

/// A repository git cannot write to at all, so even the unwind fails. The only
/// honest way to reach the remnant path.
fn unwritable(repository: PathBuf) -> PathBuf {
    read_only(&repository.join(".git"));

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

fn read_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o555);
    fs::set_permissions(path, permissions).unwrap();
}

fn worktrees_of(repository: &Path) -> usize {
    git(repository, &["worktree", "list", "--porcelain"])
        .lines()
        .filter(|line| line.starts_with("worktree "))
        .count()
}

fn branches_of(repository: &Path) -> Vec<String> {
    git(repository, &["branch", "--format=%(refname:short)"])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The plane id an aborted `create` names.
fn error_plane_id(error: &EngineError) -> String {
    match error {
        EngineError::CreateAborted { id, .. } => id.clone(),
        other => panic!("not an aborted create: {other:?}"),
    }
}

fn request(members: &[&Path], branch: &str) -> PlaneCreateRequest {
    PlaneCreateRequest {
        members: members
            .iter()
            .map(|member| member.display().to_string())
            .collect(),
        branch: Some(branch.to_owned()),
        id: None,
        intent: BranchIntent::Resolve,
        fetch: true,
    }
}

/// A host of its own: its own planes directory, its own projects directory, and
/// its own repositories, so no test can see another's.
struct Host {
    root: PathBuf,
    directories: Directories,
    interrupt: Interrupt,
}

impl Host {
    fn new(label: &str) -> Host {
        let root = scratch_dir(label);

        Host {
            directories: Directories::new(root.join("planes"), root.join("projects")),
            root,
            interrupt: Interrupt::never(),
        }
    }

    fn interrupted_by(mut self, interrupt: Interrupt) -> Host {
        self.interrupt = interrupt;
        self
    }

    fn repository(&self, name: &str) -> PathBuf {
        self.repository_at(&format!("repos/{name}"))
    }

    /// An owned project, registered the way `bp project add` registers one. A
    /// path is a git URL, so the "forge" is an ordinary local repository and
    /// this is the code path a real one takes.
    fn project(&self, name: &str) -> PathBuf {
        let forge = self.repository_at(&format!("forges/{name}"));

        LocalEngine::new(self.directories.clone())
            .project_add(ProjectAddRequest {
                url: forge.display().to_string(),
                name: Some(name.to_owned()),
            })
            .expect("the fixture project was registered");

        forge
    }

    fn repository_at(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        repository_with_one_commit(&path);
        path.canonicalize().expect("the repository was just made")
    }

    fn planes(&self) -> PathBuf {
        self.directories.planes().to_path_buf()
    }

    fn planes_are_empty(&self) -> bool {
        match fs::read_dir(self.planes()) {
            Ok(entries) => entries.count() == 0,
            Err(_) => true,
        }
    }

    fn create(&self, request: PlaneCreateRequest) -> Result<PlaneCreated, EngineError> {
        LocalEngine::new(self.directories.clone())
            .with_interrupt(self.interrupt)
            .plane_create(request)
    }
}
