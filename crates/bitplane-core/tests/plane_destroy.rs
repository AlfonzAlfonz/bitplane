//! `bp destroy` and `bp rm`, against real git repositories.
//!
//! Real rather than stubbed, because every acceptance criterion here is about
//! what git actually refuses and what is actually left on disk — the refusal
//! pass, the worktrees, the plane file, and what a re-run finds.
//!
//! Every fixture repository has an **origin that already holds `main`**. The
//! `unpushed` predicate is *"the tip is contained in some `refs/remotes/origin/*`"*,
//! so a repository with no origin has every branch unpushed; that is correct, and
//! it would make `unpushed` the answer to every test rather than to its own.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

use bitplane_core::plane_dir::{BITPLANE_DIR, LATCH_NAME};
use bitplane_core::testing::{
    git, repository_with_an_origin, repository_with_one_commit, scratch_dir,
};
use bitplane_core::{
    BranchDisposition, BranchIntent, Directories, Engine, EngineError, ExitCode, Interrupt,
    LocalEngine, MemberRef, Outcome, PlaneCreateRequest, PlaneCreated, PlaneDestroyRequest,
    PlaneDestroyed, PlaneFile, PlaneRef, PlaneRemoveRequest, PlaneRemoved, ProjectAddRequest,
    Reason, SkipReason, Teardown,
};

#[test]
fn a_plane_destroys_to_nothing() {
    let host = Host::new("destroy-whole");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha, &beta], "feat-login");

    let destroyed = host.destroy(by_id(&plane), &[]).unwrap();

    assert_eq!(destroyed.members.len(), 2, "one row per member");
    for row in &destroyed.members {
        let removed = row.value().expect("the worktree went");
        assert_eq!(removed.branch.as_deref(), Some("feat-login"));
        assert!(removed.waived.is_empty(), "nothing had to be waived");
    }

    assert!(!plane.directory.exists(), "the plane directory goes too");
    for repository in [&alpha, &beta] {
        assert_eq!(
            worktrees_of(repository),
            1,
            "only {repository:?}'s own worktree should remain"
        );
    }
}

#[test]
fn an_owned_projects_branch_goes_with_its_worktree() {
    let host = Host::new("destroy-owned-branch");
    host.project("codestyle");
    let plane = host.project_plane(&["@codestyle"], "feat-login");

    let destroyed = host.destroy(by_id(&plane), &[]).unwrap();

    let removed = destroyed.members[0].value().unwrap();
    assert_eq!(removed.branch.as_deref(), Some("feat-login"));
    assert_eq!(removed.disposition, BranchDisposition::Deleted);
    assert!(
        !branches_of(&host.source_repo("codestyle")).contains(&"feat-login".to_owned()),
        "for a repo bitplane built, refs/heads/* holds exactly the plane branches"
    );
}

#[test]
fn the_branch_deleted_is_the_one_the_worktree_is_actually_on() {
    // Nothing records the branch a worktree was created on, so `destroy`
    // deletes what it finds — and the branch the user switched away from is
    // left behind with no record of it, which is `doctor`'s business.
    let host = Host::new("destroy-owned-switched");
    host.project("codestyle");
    let plane = host.project_plane(&["@codestyle"], "feat-login");
    let worktree = plane.directory.join("forges/codestyle");
    git(&worktree, &["checkout", "--quiet", "-b", "somewhere-else"]);

    let destroyed = host.destroy(by_id(&plane), &[]).unwrap();

    let removed = destroyed.members[0].value().unwrap();
    assert_eq!(removed.branch.as_deref(), Some("somewhere-else"));
    assert_eq!(removed.disposition, BranchDisposition::Deleted);

    let left = branches_of(&host.source_repo("codestyle"));
    assert!(!left.contains(&"somewhere-else".to_owned()));
    assert!(
        left.contains(&"feat-login".to_owned()),
        "the namespace is clean but not self-pruning"
    );
}

#[test]
fn an_owned_projects_branch_is_gated_by_the_unpushed_check_and_by_nothing_else() {
    let host = Host::new("destroy-owned-unpushed");
    host.project("codestyle");
    let plane = host.project_plane(&["@codestyle"], "feat-login");
    commit(&plane.directory.join("forges/codestyle"), "work");

    let refused = host.destroy(by_id(&plane), &[]).unwrap_err();
    assert!(
        matches!(refused, EngineError::Refused { .. }),
        "a branch with commits nobody else has is not bitplane's to delete: {refused:?}"
    );
    assert!(branches_of(&host.source_repo("codestyle")).contains(&"feat-login".to_owned()));

    // No new waiver and no second gate: the one the refusal named is the one
    // that lets the branch go with the worktree.
    let destroyed = host.destroy(by_id(&plane), &[Reason::Unpushed]).unwrap();

    assert_eq!(
        destroyed.members[0].value().unwrap().disposition,
        BranchDisposition::Deleted
    );
    assert!(!branches_of(&host.source_repo("codestyle")).contains(&"feat-login".to_owned()));
}

#[test]
fn an_owned_branch_is_still_weighed_when_the_worktree_directory_has_been_deleted() {
    // The commits at risk are the branch's, not the working tree's, and the
    // branch is about to go with it — so a directory the user removed by hand
    // must not be a way past the veto.
    let host = Host::new("destroy-owned-missing-directory");
    host.project("codestyle");
    let plane = host.project_plane(&["@codestyle"], "feat-login");
    let worktree = plane.directory.join("forges/codestyle");
    commit(&worktree, "work");
    fs::remove_dir_all(&worktree).unwrap();

    let refused = host.destroy(by_id(&plane), &[]).unwrap_err();

    assert!(
        matches!(refused, EngineError::Refused { .. }),
        "got {refused:?}"
    );
    assert!(
        refused
            .envelope()
            .problems
            .iter()
            .any(|problem| problem.message.contains("not on origin")),
        "got {:?}",
        refused.envelope().problems
    );
    assert!(branches_of(&host.source_repo("codestyle")).contains(&"feat-login".to_owned()));
}

#[test]
fn an_ad_hoc_members_missing_directory_refuses_over_nothing_because_its_branch_stays() {
    let host = Host::new("destroy-ad-hoc-missing-directory");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    let worktree = plane.directory.join("repos/alpha");
    commit(&worktree, "work");
    fs::remove_dir_all(&worktree).unwrap();

    let destroyed = host.destroy(by_id(&plane), &[]).unwrap();

    assert_eq!(
        destroyed.members[0].value().unwrap().disposition,
        BranchDisposition::Kept
    );
    assert!(
        branches_of(&alpha).contains(&"feat-login".to_owned()),
        "nothing was at risk, so there was nothing to refuse over"
    );
}

#[test]
fn a_branch_something_else_already_deleted_is_reported_as_deleted_rather_than_as_a_failure() {
    let host = Host::new("destroy-branch-already-gone");
    host.project("codestyle");
    let plane = host.project_plane(&["@codestyle"], "feat-login");

    // What a concurrent run, or a `git branch -D` by hand, leaves behind: the
    // worktree registered and the branch already gone.
    git(
        &host.source_repo("codestyle"),
        &["update-ref", "-d", "refs/heads/feat-login"],
    );

    let destroyed = host
        .destroy(by_id(&plane), &[Reason::Uncommitted, Reason::Unpushed])
        .unwrap();

    assert_eq!(
        destroyed.members[0].value().unwrap().disposition,
        BranchDisposition::Deleted,
        "the state asked for is the state reached; converging says so"
    );
}

#[test]
fn rm_deletes_an_owned_projects_branch_and_keeps_an_ad_hoc_members() {
    let host = Host::new("rm-mixed-branches");
    host.project("codestyle");
    let alpha = host.repository("alpha");
    let plane = host.project_plane(&["@codestyle", &alpha.display().to_string()], "feat-login");

    let removed = host
        .remove_specs(
            by_id(&plane),
            &["@codestyle", &alpha.display().to_string()],
            &[],
        )
        .unwrap();

    assert_eq!(
        removed
            .members
            .iter()
            .map(|row| row.value().unwrap().disposition)
            .collect::<Vec<BranchDisposition>>(),
        vec![BranchDisposition::Deleted, BranchDisposition::Kept],
        "the response says what became of each member's branch"
    );
    assert!(!branches_of(&host.source_repo("codestyle")).contains(&"feat-login".to_owned()));
    assert!(branches_of(&alpha).contains(&"feat-login".to_owned()));
}

#[test]
fn no_branch_is_deleted_in_a_repo_bitplane_does_not_own() {
    // The sharpest asymmetry in the design: bitplane owns the refs in a repo it
    // built and owns nothing in a repo it merely pointed at (ADR-0006).
    let host = Host::new("destroy-keeps-branches");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");

    let destroyed = host.destroy(by_id(&plane), &[]).unwrap();

    assert_eq!(
        destroyed.members[0].value().unwrap().disposition,
        BranchDisposition::Kept,
        "bitplane owns no ref in an ad-hoc member's repo"
    );
    assert!(
        branches_of(&alpha).contains(&"feat-login".to_owned()),
        "the branch is the user's; bitplane owns nothing here"
    );
}

#[test]
fn anything_in_the_plane_directory_that_no_member_names_goes_with_it() {
    let host = Host::new("destroy-scratch");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    fs::write(plane.directory.join("notes.md"), "mine\n").unwrap();

    host.destroy(by_id(&plane), &[]).unwrap();

    assert!(!plane.directory.exists());
}

#[test]
fn every_refusal_across_every_member_comes_back_in_one_error_with_nothing_touched() {
    let host = Host::new("destroy-refused");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let gamma = host.repository("gamma");
    let plane = host.plane(&[&alpha, &beta, &gamma], "feat-login");

    dirty(&worktree(&plane, 0));
    scribble(&worktree(&plane, 1));

    let error = host.destroy(by_id(&plane), &[]).unwrap_err();

    let EngineError::Refused {
        operation,
        blocking,
        considered,
        refusals,
        ..
    } = &error
    else {
        panic!("expected refused, got {error:?}");
    };
    assert_eq!(*operation, Teardown::Destroy);
    assert_eq!((*blocking, *considered), (2, 3));
    assert_eq!(
        reasons(refusals),
        vec![Reason::Uncommitted, Reason::Untracked],
        "every reason across every member, in one pass"
    );

    let envelope = error.envelope();
    assert_eq!(envelope.error, "refused");
    assert_eq!(error.exit_code(), ExitCode::Failure);
    assert_eq!(
        envelope.problems.len(),
        2,
        "a refusal per member per reason"
    );
    assert!(
        envelope.message.contains("2 of 3 members"),
        "got {}",
        envelope.message
    );

    // A half-destroyed plane is worse than a refused one.
    assert!(plane.directory.exists());
    for index in 0..3 {
        assert!(
            worktree(&plane, index).exists(),
            "member {index} was touched"
        );
    }
}

#[test]
fn the_remedy_names_exactly_the_reasons_that_were_raised_and_can_be_pasted() {
    let host = Host::new("destroy-remedy");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    scribble(&worktree(&plane, 0));

    let error = host.destroy(by_id(&plane), &[]).unwrap_err();

    let remedy = error.envelope().remedy.expect("a remedy");
    assert!(remedy.contains("--waive untracked"), "got {remedy}");
    assert!(
        !remedy.contains("--waive uncommitted"),
        "it never names a reason that did not come up: {remedy}"
    );
}

#[test]
fn a_waiver_covers_one_reason_and_the_others_still_refuse() {
    let host = Host::new("destroy-one-waiver");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    dirty(&worktree(&plane, 0));
    scribble(&worktree(&plane, 0));

    let error = host
        .destroy(by_id(&plane), &[Reason::Uncommitted])
        .unwrap_err();

    let EngineError::Refused { refusals, .. } = &error else {
        panic!("expected refused, got {error:?}");
    };
    assert_eq!(
        reasons(refusals),
        vec![Reason::Untracked],
        "there is no blanket force: waiving one says nothing about the rest"
    );
    assert!(plane.directory.exists());
}

#[test]
fn a_waived_reason_is_echoed_per_member_so_a_forced_destruction_is_in_the_transcript() {
    let host = Host::new("destroy-waived-echo");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    dirty(&worktree(&plane, 0));

    let destroyed = host
        .destroy(by_id(&plane), &[Reason::Uncommitted, Reason::Unpushed])
        .unwrap();

    assert_eq!(
        destroyed.members[0].value().unwrap().waived,
        vec![Reason::Uncommitted],
        "only the reasons actually raised are echoed"
    );
    assert!(!plane.directory.exists());
}

#[test]
fn an_unpushed_branch_refuses_and_a_pushed_one_does_not_even_with_the_wrong_upstream() {
    // Row two of ADR-0006's table, which is the whole reason the predicate is
    // containment and not `git branch -d`: a plane branch is cut from
    // `origin/main` and keeps it as its upstream, so `-d` compares against the
    // wrong ref and refuses a branch whose commits are already on the forge.
    let host = Host::new("destroy-unpushed");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    let at = worktree(&plane, 0);

    commit(&at, "work.txt");
    let error = host.destroy(by_id(&plane), &[]).unwrap_err();
    let EngineError::Refused { refusals, .. } = &error else {
        panic!("expected refused, got {error:?}");
    };
    assert_eq!(reasons(refusals), vec![Reason::Unpushed]);
    assert!(
        refusals[0].detail.contains("not on origin"),
        "got {}",
        refusals[0].detail
    );

    // Pushed to `origin/feat-login`, with `branch.feat-login.merge` still
    // pointing at `main` — exactly the state `git branch -d` gets wrong.
    git(&at, &["push", "--quiet", "origin", "feat-login"]);
    assert_eq!(
        git(&at, &["config", "--get", "branch.feat-login.merge"]).trim(),
        "refs/heads/main",
        "the upstream is untouched; only containment changed"
    );

    host.destroy(by_id(&plane), &[])
        .expect("the work is on the forge, so there is nothing to lose");
}

#[test]
fn destroy_performs_no_implicit_fetch() {
    // A stale remote-tracking ref can only make the check more conservative —
    // a false refusal the user waives — never more permissive, which is what
    // lets `destroy` skip the fetch entirely (ADR-0006).
    let host = Host::new("destroy-no-fetch");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    let at = worktree(&plane, 0);

    commit(&at, "work.txt");
    // Pushed with the tracking ref then rewound, so the forge has the commit
    // and this repo's `refs/remotes/origin/*` does not. A fetch would notice.
    git(&at, &["push", "--quiet", "origin", "feat-login"]);
    let before = origin_refs(&alpha);
    git(
        &alpha,
        &["update-ref", "-d", "refs/remotes/origin/feat-login"],
    );

    let error = host.destroy(by_id(&plane), &[]).unwrap_err();

    let EngineError::Refused { refusals, .. } = &error else {
        panic!("expected refused, got {error:?}");
    };
    assert_eq!(
        reasons(refusals),
        vec![Reason::Unpushed],
        "the stale ref made the answer conservative, and nothing corrected it"
    );
    assert_ne!(
        origin_refs(&alpha),
        before,
        "the fixture's rewind must not have been undone by a fetch"
    );
}

#[test]
fn a_locked_worktree_refuses_with_gits_own_reason_and_only_its_own_waiver_removes_it() {
    let host = Host::new("destroy-locked");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    let at = worktree(&plane, 0);
    git(
        &alpha,
        &[
            "worktree",
            "lock",
            "--reason",
            "migration running",
            &at.display().to_string(),
        ],
    );

    let error = host.destroy(by_id(&plane), &[]).unwrap_err();
    let EngineError::Refused { refusals, .. } = &error else {
        panic!("expected refused, got {error:?}");
    };
    assert_eq!(reasons(refusals), vec![Reason::LockedWorktree]);
    assert_eq!(
        refusals[0].detail, "the worktree is locked: migration running",
        "git's own reason, carried through"
    );

    // Waiving something else gets nowhere: the refusal is per reason.
    assert!(host.destroy(by_id(&plane), &[Reason::Uncommitted]).is_err());

    host.destroy(by_id(&plane), &[Reason::LockedWorktree])
        .expect("its own waiver removes it");
    assert!(!plane.directory.exists());
    assert_eq!(worktrees_of(&alpha), 1);
}

#[test]
fn a_missing_source_repo_is_a_reason_of_its_own_rather_than_folded_into_uncommitted() {
    // It means *I cannot check at all*, not *I accept losing this diff*.
    // Folding it in would let a waiver granted for work the user looked at
    // authorise deleting a directory nobody looked at.
    let host = Host::new("destroy-source-gone");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    fs::remove_dir_all(&alpha).unwrap();

    let error = host.destroy(by_id(&plane), &[]).unwrap_err();
    let EngineError::Refused { refusals, .. } = &error else {
        panic!("expected refused, got {error:?}");
    };
    assert_eq!(reasons(refusals), vec![Reason::SourceRepoMissing]);
    assert!(
        host.destroy(by_id(&plane), &[Reason::Uncommitted]).is_err(),
        "the waiver for work you looked at does not cover work nobody can see"
    );

    host.destroy(by_id(&plane), &[Reason::SourceRepoMissing])
        .expect("its own waiver clears it");
    assert!(!plane.directory.exists());
}

#[test]
fn a_member_git_will_not_answer_about_is_refused_once_and_then_converges() {
    // A source repo replaced rather than removed: the directory is a
    // repository, but it has never heard of this worktree, so every question
    // about it fails. Letting that propagate as a whole-operation failure
    // would make the plane undestroyable — no rows, and no waiver that helps.
    let host = Host::new("destroy-unreachable");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");

    fs::remove_dir_all(alpha.join(".git")).unwrap();
    repository_with_an_origin(&alpha, &host.root.join("forge").join("alpha-again"));

    let error = host.destroy(by_id(&plane), &[]).unwrap_err();
    let EngineError::Refused { refusals, .. } = &error else {
        panic!("expected refused, got {error:?}");
    };
    assert_eq!(
        reasons(refusals),
        vec![Reason::SourceRepoMissing],
        "git would not answer, which is *cannot check at all* and nothing else"
    );

    host.destroy(by_id(&plane), &[Reason::SourceRepoMissing])
        .expect("the waiver for what cannot be checked clears it");
    assert!(!plane.directory.exists());
}

#[test]
fn a_worktree_deleted_by_hand_is_still_taken_off_gits_books() {
    // Git records the path it resolved at `worktree add` time, so a comparison
    // that falls back to the path as written misses under a symlinked ancestor
    // — `/tmp` on macOS, or a planes directory the user symlinked. The entry
    // would survive, still holding the branch, and the next `bp create` on that
    // branch would be refused with nothing to point at.
    let host = Host::new("destroy-hand-deleted");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");

    fs::remove_dir_all(worktree(&plane, 0)).unwrap();

    let destroyed = host.destroy(by_id(&plane), &[]).unwrap();

    assert!(destroyed.members[0].value().is_some(), "it was removed");
    assert_eq!(
        worktrees_of(&alpha),
        1,
        "the stale entry must not survive holding the branch"
    );
}

#[test]
fn a_latched_plane_destroys_with_no_refusal_checks_at_all() {
    // The latch means *nothing in here is yours*, so every refusal reason is
    // structurally impossible — which is what makes `destroy` the one way out.
    let host = Host::new("destroy-latched");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    let at = worktree(&plane, 0);

    dirty(&at);
    scribble(&at);
    commit(&at, "unpushed.txt");
    relatch(&plane);

    let destroyed = host.destroy(by_id(&plane), &[]).unwrap();

    assert!(destroyed.incomplete, "the response says why it did not ask");
    assert!(!plane.directory.exists());
    assert_eq!(worktrees_of(&alpha), 1);
}

#[test]
fn re_running_after_a_half_done_destroy_finishes_and_reports_the_gone_ones() {
    let host = Host::new("destroy-converges");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha, &beta], "feat-login");

    // What an interrupted run leaves: one member out, the rest still there.
    git(
        &alpha,
        &[
            "worktree",
            "remove",
            &worktree(&plane, 0).display().to_string(),
        ],
    );

    let destroyed = host.destroy(by_id(&plane), &[]).unwrap();

    assert_eq!(
        destroyed.members[0].outcome,
        Outcome::AlreadyDone,
        "the gone one is reported, not failed over"
    );
    assert!(destroyed.members[1].value().is_some());
    assert!(!plane.directory.exists());
}

#[test]
fn an_interrupted_destroy_leaves_the_plane_findable_and_converges_on_a_re_run() {
    static RAISED: AtomicUsize = AtomicUsize::new(0);
    let interrupt = Interrupt::on(&RAISED);

    let host = Host::new("destroy-interrupted");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");

    interrupt.raise();
    let destroyed = host
        .interrupted_by(interrupt)
        .destroy(by_id(&plane), &[])
        .unwrap();

    assert!(destroyed.interrupted);
    assert_eq!(
        destroyed.members[0].outcome,
        Outcome::Skipped(SkipReason::Interrupted)
    );
    assert!(
        plane.directory.join("plane.toml").exists(),
        "the plane stays visible: it still exists"
    );

    host.destroy(by_id(&plane), &[])
        .expect("re-running finishes it");
    assert!(!plane.directory.exists());
}

#[test]
fn the_plane_is_resolved_from_a_directory_inside_it() {
    let host = Host::new("destroy-from-here");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");

    let destroyed = host
        .destroy(
            PlaneRef::ContainingPath {
                path: worktree(&plane, 0),
            },
            &[],
        )
        .unwrap();

    assert_eq!(
        destroyed.id,
        plane.id.to_string(),
        "a path resolves to a plane"
    );
    assert!(!plane.directory.exists());
}

#[test]
fn a_plane_that_is_not_there_is_a_usage_failure_naming_what_to_do() {
    let host = Host::new("destroy-not-found");
    let elsewhere = host.root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();

    let by_name = host
        .destroy(
            PlaneRef::Id {
                id: "auth-work".to_owned(),
            },
            &[],
        )
        .unwrap_err();
    assert_eq!(by_name.envelope().error, "plane_not_found");
    assert_eq!(by_name.exit_code(), ExitCode::Usage);
    assert!(by_name.to_string().contains("auth-work"));
    assert!(
        by_name
            .envelope()
            .remedy
            .is_some_and(|remedy| remedy.contains("bp list"))
    );

    let from_here = host
        .destroy(PlaneRef::ContainingPath { path: elsewhere }, &[])
        .unwrap_err();
    assert!(
        from_here
            .envelope()
            .remedy
            .is_some_and(|remedy| remedy.contains("--plane")),
        "standing nowhere, the way out is to name one"
    );
}

#[test]
fn rm_takes_one_member_out_and_leaves_the_rest_of_the_plane_alone() {
    let host = Host::new("rm-one");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha, &beta], "feat-login");

    let removed = host.remove(by_id(&plane), &[&alpha], &[]).unwrap();

    assert_eq!(removed.members.len(), 1, "one row per member *named*");
    assert_eq!(removed.members[0].member, MemberRef::Repo(alpha.clone()));

    assert!(!worktree(&plane, 0).exists());
    assert!(worktree(&plane, 1).exists(), "the rest is untouched");
    assert!(plane.directory.join("plane.toml").exists());

    let file = PlaneFile::read(&plane.directory.join("plane.toml")).unwrap();
    assert_eq!(
        file.members
            .iter()
            .map(|member| member.source.to_string())
            .collect::<Vec<String>>(),
        vec![beta.display().to_string()],
        "the plane file is rewritten without it"
    );
}

#[test]
fn rm_preserves_what_a_user_wrote_in_the_plane_file() {
    let host = Host::new("rm-comments");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha, &beta], "feat-login");

    let at = plane.directory.join("plane.toml");
    let annotated = format!(
        "# do not reap, long-running migration\n{}",
        fs::read_to_string(&at).unwrap()
    );
    fs::write(&at, &annotated).unwrap();

    host.remove(by_id(&plane), &[&alpha], &[]).unwrap();

    let rewritten = fs::read_to_string(&at).unwrap();
    assert!(
        rewritten.starts_with("# do not reap, long-running migration"),
        "hand-editing is tolerated, so a comment survives a rewrite: {rewritten}"
    );
    assert!(!rewritten.contains(&alpha.display().to_string()));
}

#[test]
fn rm_carries_the_same_rules_as_destroy() {
    // Removing a member destroys exactly as much work as destroying a
    // one-member plane, so a lighter rule here would be a hole in that one.
    let host = Host::new("rm-refuses");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha, &beta], "feat-login");
    dirty(&worktree(&plane, 0));

    let error = host
        .remove(by_id(&plane), &[&alpha, &beta], &[])
        .unwrap_err();

    let EngineError::Refused {
        operation,
        blocking,
        considered,
        ..
    } = &error
    else {
        panic!("expected refused, got {error:?}");
    };
    assert_eq!(*operation, Teardown::Remove);
    assert_eq!(
        (*blocking, *considered),
        (1, 2),
        "the count is over the members you named, not over the plane"
    );
    assert!(worktree(&plane, 0).exists(), "nothing has been touched");
    assert!(worktree(&plane, 1).exists());

    host.remove(by_id(&plane), &[&alpha, &beta], &[Reason::Uncommitted])
        .expect("the waiver covers it");
}

#[test]
fn rm_tolerates_a_member_whose_worktree_is_already_gone_and_drops_its_entry() {
    // The repair for a half-unwound `bp add` is `bp rm`, so the naive
    // implementation — `git worktree remove` on a path that is not there —
    // would fail on exactly the case it exists to fix (ADR-0004).
    let host = Host::new("rm-already-gone");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha, &beta], "feat-login");
    git(
        &alpha,
        &[
            "worktree",
            "remove",
            &worktree(&plane, 0).display().to_string(),
        ],
    );

    let removed = host.remove(by_id(&plane), &[&alpha], &[]).unwrap();

    assert_eq!(removed.members[0].outcome, Outcome::AlreadyDone);

    let file = PlaneFile::read(&plane.directory.join("plane.toml")).unwrap();
    assert_eq!(file.members.len(), 1, "the entry is dropped all the same");
}

#[test]
fn rm_names_a_member_whose_repo_has_been_deleted_the_way_the_plane_file_spells_it() {
    // The one case `--waive source_repo_missing` exists for, and the one where
    // `canonicalize` cannot help: the file holds the resolved path, the user
    // types the path they know, and on a host with a symlinked ancestor the two
    // differ unless as much of the path as exists is resolved.
    let host = Host::new("rm-deleted-repo");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let plane = host.plane(&[&alpha, &beta], "feat-login");
    fs::remove_dir_all(&alpha).unwrap();

    let removed = host
        .remove(by_id(&plane), &[&alpha], &[Reason::SourceRepoMissing])
        .expect("the member is nameable, and the waiver covers it");

    assert_eq!(removed.members.len(), 1);
    assert!(!worktree(&plane, 0).exists());
    assert!(worktree(&plane, 1).exists());
}

#[test]
fn a_directory_git_does_not_record_as_a_worktree_is_still_removable() {
    // `git worktree remove` refuses a path it has no entry for, and the
    // prunable fallback cannot fire because there is no entry to be prunable —
    // so without a path of its own this is a member no waiver can shift.
    let host = Host::new("destroy-unregistered");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    let at = worktree(&plane, 0);

    // The entry unregistered, and a repository of its own left at that path:
    // still listed in `plane.toml`, and nothing git will answer for.
    git(
        &alpha,
        &["worktree", "remove", "--force", &at.display().to_string()],
    );
    repository_with_an_origin(&at, &host.root.join("forge").join("impostor"));

    host.destroy(by_id(&plane), &[Reason::Unpushed])
        .expect("an unregistered directory still goes");

    assert!(!plane.directory.exists());
    assert_eq!(worktrees_of(&alpha), 1);
}

#[test]
fn rm_on_a_latched_plane_declines_and_points_at_the_one_verb_that_clears_it() {
    let host = Host::new("rm-latched");
    let alpha = host.repository("alpha");
    let plane = host.plane(&[&alpha], "feat-login");
    relatch(&plane);

    let error = host.remove(by_id(&plane), &[&alpha], &[]).unwrap_err();

    let envelope = error.envelope();
    assert_eq!(envelope.error, "plane_incomplete");
    assert_eq!(error.exit_code(), ExitCode::Failure);
    assert!(
        envelope
            .remedy
            .is_some_and(|remedy| remedy.contains("bp destroy -p")),
        "whittling a latched plane down one member at a time is meaningless work"
    );
    assert!(worktree(&plane, 0).exists());
}

#[test]
fn rm_of_a_member_that_is_not_in_the_plane_is_a_usage_failure() {
    let host = Host::new("rm-stranger");
    let alpha = host.repository("alpha");
    let stranger = host.repository("stranger");
    let plane = host.plane(&[&alpha], "feat-login");

    let error = host.remove(by_id(&plane), &[&stranger], &[]).unwrap_err();

    assert_eq!(error.envelope().error, "project_not_in_plane");
    assert_eq!(error.exit_code(), ExitCode::Usage);
    assert!(worktree(&plane, 0).exists());
}

// ---------------------------------------------------------------- fixtures --

fn by_id(plane: &PlaneCreated) -> PlaneRef {
    PlaneRef::Id {
        id: plane.id.to_string(),
    }
}

fn worktree(plane: &PlaneCreated, index: usize) -> PathBuf {
    plane.directory.join(
        plane.members[index]
            .value()
            .expect("a built member")
            .path
            .as_path(),
    )
}

/// The reasons raised, deduplicated in the order the table lists them.
fn reasons(refusals: &[bitplane_core::Refusal]) -> Vec<Reason> {
    Reason::ALL
        .into_iter()
        .filter(|reason| refusals.iter().any(|refusal| refusal.reason == *reason))
        .collect()
}

/// A tracked file, changed: `uncommitted`.
fn dirty(worktree: &Path) {
    fs::write(worktree.join("README.md"), "changed\n").unwrap();
}

/// A file git does not track: `untracked`.
fn scribble(worktree: &Path) {
    fs::write(worktree.join("scratch.txt"), "mine\n").unwrap();
}

/// A commit the forge has never seen: `unpushed`.
fn commit(worktree: &Path, name: &str) {
    fs::write(worktree.join(name), "work\n").unwrap();
    git(worktree, &["add", name]);
    git(worktree, &["commit", "--quiet", "--message", "work"]);
}

/// Puts the latch back, which is the state an interrupted `create` leaves.
fn relatch(plane: &PlaneCreated) {
    let bitplane = plane.directory.join(BITPLANE_DIR);
    fs::create_dir_all(&bitplane).unwrap();
    fs::write(bitplane.join(LATCH_NAME), "2026-09-22T00:00:00Z\n").unwrap();
}

fn origin_refs(repository: &Path) -> String {
    git(
        repository,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/remotes/origin/",
        ],
    )
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

/// A host of its own: its own planes directory, its own forge, and its own
/// repositories, so no test can see another's.
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

    fn interrupted_by(&self, interrupt: Interrupt) -> Host {
        Host {
            root: self.root.clone(),
            directories: self.directories.clone(),
            interrupt,
        }
    }

    fn repository(&self, name: &str) -> PathBuf {
        let path = self.root.join("repos").join(name);
        repository_with_an_origin(&path, &self.root.join("forge").join(name));
        path.canonicalize().expect("the repository was just made")
    }

    /// An owned project, registered the way `bp project add` registers one.
    fn project(&self, name: &str) -> PathBuf {
        let forge = self.root.join("forges").join(name);
        repository_with_one_commit(&forge);

        self.engine()
            .project_add(ProjectAddRequest {
                url: forge.display().to_string(),
                name: Some(name.to_owned()),
            })
            .expect("the fixture project was registered");

        forge.canonicalize().expect("the forge was just made")
    }

    fn source_repo(&self, name: &str) -> PathBuf {
        self.directories.projects().join(name).join("repo.git")
    }

    fn plane(&self, members: &[&Path], branch: &str) -> PlaneCreated {
        self.project_plane(
            &members
                .iter()
                .map(|member| member.display().to_string())
                .collect::<Vec<String>>()
                .iter()
                .map(String::as_str)
                .collect::<Vec<&str>>(),
            branch,
        )
    }

    fn project_plane(&self, members: &[&str], branch: &str) -> PlaneCreated {
        self.engine()
            .plane_create(PlaneCreateRequest {
                members: members.iter().map(|member| (*member).to_owned()).collect(),
                branch: Some(branch.to_owned()),
                id: None,
                intent: BranchIntent::Resolve,
                fetch: true,
                run_scripts: true,
            })
            .expect("the fixture plane was built")
    }

    fn destroy(&self, plane: PlaneRef, waive: &[Reason]) -> Result<PlaneDestroyed, EngineError> {
        self.engine().plane_destroy(PlaneDestroyRequest {
            plane,
            waive: waive.to_vec(),
            run_scripts: true,
        })
    }

    fn remove(
        &self,
        plane: PlaneRef,
        members: &[&Path],
        waive: &[Reason],
    ) -> Result<PlaneRemoved, EngineError> {
        let members: Vec<String> = members
            .iter()
            .map(|member| member.display().to_string())
            .collect();

        self.remove_specs(
            plane,
            &members.iter().map(String::as_str).collect::<Vec<&str>>(),
            waive,
        )
    }

    fn remove_specs(
        &self,
        plane: PlaneRef,
        members: &[&str],
        waive: &[Reason],
    ) -> Result<PlaneRemoved, EngineError> {
        self.engine().plane_remove(PlaneRemoveRequest {
            plane,
            members: members.iter().map(|member| (*member).to_owned()).collect(),
            waive: waive.to_vec(),
            run_scripts: true,
        })
    }

    fn engine(&self) -> LocalEngine {
        LocalEngine::new(self.directories.clone()).with_interrupt(self.interrupt)
    }
}
