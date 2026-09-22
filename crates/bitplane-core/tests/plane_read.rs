//! The read surface, against real planes built by `plane_create`.
//!
//! Every acceptance criterion here is about what is *on disk* and what the read
//! does or does not do to it, so the planes are real ones: real repositories,
//! real worktrees, real `HEAD` files. A stubbed filesystem would only test
//! bitplane's idea of one.
//!
//! Two properties are asserted structurally rather than by inspection, because
//! they are the ones that would rot silently:
//!
//! - **A read never writes.** The whole plane directory is walked before and
//!   after every read, and the two listings must be identical — no lock
//!   sentinel, no rewritten `plane.toml`, nothing repaired.
//! - **A cheap read starts no git.** The engine is pointed at a stub `git` that
//!   records every invocation, and the recording must stay empty.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use bitplane_core::plane_dir::{BITPLANE_DIR, LATCH_NAME};
use bitplane_core::testing::{git, repository_with_one_commit, scratch_dir};
use bitplane_core::{
    BranchIntent, Directories, Engine, EngineError, ExitCode, Finding, Head, HealthCheck,
    LocalEngine, MemberRef, MemberWork, PlaneCreateRequest, PlaneCreated, PlaneList,
    PlaneListRequest, PlaneRef, PlaneShowRequest, PlaneStatus, PlaneStatusRequest, PlaneView,
    Reader,
};

#[test]
fn every_plane_is_listed_with_its_members_and_their_live_branches() {
    let host = Host::new("list-two-planes");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");

    host.create(&[&alpha], "feat-a", Some("auth-work"));
    host.create(&[&beta], "feat-b", Some("billing"));

    let listed = host.list(HealthCheck::Cheap);

    let ids: Vec<&str> = listed
        .planes
        .iter()
        .map(|plane| plane.id.as_str())
        .collect();
    assert_eq!(ids, ["auth-work", "billing"], "listed in directory order");

    let first = &listed.planes[0];
    // Canonical however it was reached: one plane is one path, so `bp list`
    // and a `bp show` resolved by walking up from a worktree agree.
    assert_eq!(first.directory, host.planes_as_resolved().join("auth-work"));
    assert_eq!(first.members.len(), 1, "one row per member, never a count");
    assert_eq!(first.members[0].member, MemberRef::Repo(alpha));
    assert_eq!(first.members[0].path.to_string(), "repos/alpha");
    assert_eq!(branch(first), ["feat-a"]);
    assert!(!first.health.has_findings());
}

#[test]
fn the_live_branch_is_whatever_the_worktree_is_on_right_now() {
    let host = Host::new("list-live-branch");
    let alpha = host.repository("alpha");
    let created = host.create(&[&alpha], "feat-a", Some("auth-work"));

    // Ordinary use: the user owns the worktree's HEAD. `plane.toml` records no
    // branch, so the read has to go and look.
    let worktree = created.directory.join("repos/alpha");
    git(&worktree, &["switch", "--quiet", "--create", "feat-b"]);

    assert_eq!(branch(&host.list(HealthCheck::Cheap).planes[0]), ["feat-b"]);
}

#[test]
fn a_detached_head_is_reported_as_detached_and_is_not_a_finding() {
    let host = Host::new("list-detached");
    let alpha = host.repository("alpha");
    let created = host.create(&[&alpha], "feat-a", Some("auth-work"));

    let worktree = created.directory.join("repos/alpha");
    git(&worktree, &["switch", "--quiet", "--detach"]);

    let plane = &host.list(HealthCheck::Cheap).planes[0];

    assert!(
        matches!(plane.members[0].head, Some(Head::Detached { .. })),
        "got {:?}",
        plane.members[0].head
    );
    assert!(
        !plane.health.has_findings(),
        "detaching is ordinary use, not drift"
    );
}

#[test]
fn a_cheap_listing_starts_no_git_process_at_all() {
    let host = Host::new("list-no-git");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", Some("auth-work"));

    let recording = recording_git();
    let listed = host
        .engine_using_git_at(&recording)
        .plane_list(PlaneListRequest {
            health: HealthCheck::Cheap,
        })
        .unwrap();

    assert_eq!(branch(&listed.planes[0]), ["feat-a"], "branches were read");
    assert!(
        !recording.join("calls").exists(),
        "bp list must not spawn git per member: {}",
        fs::read_to_string(recording.join("calls")).unwrap_or_default()
    );
}

#[test]
fn a_directory_without_a_plane_file_is_silently_skipped() {
    let host = Host::new("list-not-a-plane");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", Some("auth-work"));

    fs::create_dir_all(host.planes().join("scratch").join("deep")).unwrap();
    fs::write(host.planes().join("notes.txt"), "not a plane\n").unwrap();

    let listed = host.list(HealthCheck::Cheap);

    assert_eq!(listed.planes.len(), 1);
    assert_eq!(listed.planes[0].id, "auth-work");
}

#[test]
fn a_plane_file_that_will_not_parse_is_a_row_and_the_scan_carries_on() {
    let host = Host::new("list-unreadable");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", Some("zzz-healthy"));
    host.plane_file_saying("broken", "version = 1\nid = \"broken\"\nhost = \"local\"\n");

    let listed = host.list(HealthCheck::Cheap);

    assert_eq!(
        listed.planes.len(),
        2,
        "the scan was not aborted by one file"
    );

    let broken = &listed.planes[0];
    assert_eq!(broken.id, "broken");
    assert!(broken.members.is_empty());
    let [Finding::Unreadable { error }] = &broken.health.findings[..] else {
        panic!("expected one unreadable finding, got {:?}", broken.health);
    };
    let envelope = error.envelope();
    assert_eq!(envelope.error, "parse_error");
    assert!(
        envelope.message.contains("host"),
        "got {}",
        envelope.message
    );
    assert!(
        envelope.message.contains("plane.toml"),
        "the file is named: {}",
        envelope.message
    );

    assert!(!listed.planes[1].health.has_findings(), "the other is fine");
}

#[test]
fn every_read_leaves_the_planes_directory_byte_for_byte_as_it_found_it() {
    let host = Host::new("read-writes-nothing");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let created = host.create(&[&alpha, &beta], "feat-a", Some("auth-work"));

    // Something to be tempted to repair: a member whose worktree is gone, and
    // a stored id that disagrees with the directory name.
    fs::remove_dir_all(created.directory.join("repos/beta")).unwrap();
    host.rewrite_id("auth-work", "bp-a3f9c2e1");

    let before = tree(&host.planes());

    host.list(HealthCheck::Full);
    host.show(PlaneRef::Id {
        id: "auth-work".to_owned(),
    })
    .unwrap();
    host.status(PlaneRef::Id {
        id: "auth-work".to_owned(),
    })
    .unwrap();

    assert_eq!(
        tree(&host.planes()),
        before,
        "a read takes no lock, writes no file and repairs nothing"
    );
}

#[test]
fn the_cheap_findings_are_each_detected_by_a_read_that_never_starts_git() {
    let host = Host::new("cheap-findings");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let created = host.create(&[&alpha, &beta], "feat-a", Some("auth-work"));

    fs::remove_dir_all(created.directory.join("repos/alpha")).unwrap();
    fs::remove_dir_all(&beta).unwrap();
    fs::create_dir_all(created.directory.join(BITPLANE_DIR)).unwrap();
    fs::write(
        created.directory.join(BITPLANE_DIR).join(LATCH_NAME),
        "2026-09-18T09:41:07Z\n",
    )
    .unwrap();
    host.rewrite_id("auth-work", "bp-a3f9c2e1");

    let recording = recording_git();
    let plane = host
        .engine_using_git_at(&recording)
        .plane_show(PlaneShowRequest {
            plane: PlaneRef::Id {
                id: "auth-work".to_owned(),
            },
            health: HealthCheck::Cheap,
        })
        .unwrap();

    let tags: Vec<&str> = plane.health.findings.iter().map(Finding::tag).collect();
    assert_eq!(
        tags,
        [
            "id_mismatch",
            "create_never_completed",
            "member_worktree_missing",
            "source_repo_missing",
        ]
    );
    assert!(
        !recording.join("calls").exists(),
        "the cheap tier is filesystem only"
    );

    let latched = plane
        .health
        .findings
        .iter()
        .find(|finding| finding.tag() == "create_never_completed")
        .unwrap();
    assert!(
        latched.to_string().starts_with("create never completed"),
        "got {latched}"
    );
    assert!(
        latched
            .remedy()
            .unwrap()
            .contains("bp destroy -p auth-work")
    );
}

#[test]
fn asking_for_no_health_asks_for_nothing() {
    let host = Host::new("health-none");
    let alpha = host.repository("alpha");
    let created = host.create(&[&alpha], "feat-a", Some("auth-work"));
    fs::remove_dir_all(created.directory.join("repos/alpha")).unwrap();

    let plane = host
        .show_at(
            PlaneRef::Id {
                id: "auth-work".to_owned(),
            },
            HealthCheck::None,
        )
        .unwrap();

    assert_eq!(plane.health.checked, HealthCheck::None);
    assert!(
        !plane.health.has_findings(),
        "nothing found because nothing was looked for"
    );
    assert_eq!(plane.members.len(), 1, "membership is still reported");
}

#[test]
fn gits_own_prunable_needs_the_full_tier_and_the_cheap_one_never_asks() {
    let host = Host::new("health-full");
    let alpha = host.repository("alpha");
    let created = host.create(&[&alpha], "feat-a", Some("auth-work"));

    // The directory is gone but git's admin entry survives, which is exactly
    // what `prunable` names.
    fs::remove_dir_all(created.directory.join("repos/alpha")).unwrap();

    let cheap = host.list(HealthCheck::Cheap);
    assert_eq!(
        tags(&cheap.planes[0]),
        ["member_worktree_missing"],
        "the cheap tier cannot see what only git knows"
    );

    let full = host.list(HealthCheck::Full);
    assert_eq!(
        tags(&full.planes[0]),
        ["member_worktree_missing", "prunable"]
    );
}

#[test]
fn the_creation_time_is_the_plane_directorys_own_birth_time_or_nothing() {
    let host = Host::new("created-at");
    let alpha = host.repository("alpha");
    let created = host.create(&[&alpha], "feat-a", Some("auth-work"));

    let plane = &host.list(HealthCheck::None).planes[0];

    // `plane.toml` is disqualified: writing it by atomic rename swaps the
    // inode, so its birth time is the time of the *last* write (ADR-0008).
    match fs::metadata(&created.directory).and_then(|metadata| metadata.created()) {
        Ok(born) => assert_eq!(
            plane.created_at.as_deref(),
            Some(bitplane_core::time::Rfc3339::at(born).to_string().as_str())
        ),
        // NFS, older ext4, or a plane moved across filesystems. Nothing gates
        // on the field, so its absence is not a failure.
        Err(_) => assert_eq!(plane.created_at, None),
    }
}

#[test]
fn a_path_inside_a_worktree_resolves_to_the_plane_it_belongs_to() {
    let host = Host::new("show-from-inside");
    let alpha = host.repository("alpha");
    let created = host.create(&[&alpha], "feat-a", Some("auth-work"));

    let deep = created.directory.join("repos/alpha/nested/deeper");
    fs::create_dir_all(&deep).unwrap();

    let plane = host.show(containing(&deep)).unwrap();

    // A path resolves to a **plane**, never to a member: standing in `@alpha`
    // describes the whole plane.
    assert_eq!(plane.id, "auth-work");
    assert_eq!(plane.members.len(), 1);
}

#[test]
fn a_worktree_reached_through_a_symlink_resolves_to_its_real_plane() {
    let host = Host::new("show-symlink");
    let alpha = host.repository("alpha");
    let created = host.create(&[&alpha], "feat-a", Some("auth-work"));

    let link = host.root.join("shortcut");
    std::os::unix::fs::symlink(created.directory.join("repos/alpha"), &link).unwrap();

    assert_eq!(host.show(containing(&link)).unwrap().id, "auth-work");
}

#[test]
fn nested_planes_resolve_to_the_innermost_one() {
    let host = Host::new("show-nested");
    let alpha = host.repository("alpha");
    let outer = host.create(&[&alpha], "feat-a", Some("outer"));

    // A plane directory inside another plane's directory. Contrived, but it is
    // the only case where "innermost wins" is observable.
    let inner = outer.directory.join("repos/alpha/inner");
    fs::create_dir_all(&inner).unwrap();
    fs::write(
        inner.join("plane.toml"),
        "version = 1\nid = \"inner\"\n\n[members]\n",
    )
    .unwrap();

    assert_eq!(host.show(containing(&inner)).unwrap().id, "inner");
}

#[test]
fn a_plane_outside_the_planes_directory_still_resolves() {
    let host = Host::new("show-outside");
    let elsewhere = host.root.join("not-the-planes-directory");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(
        elsewhere.join("plane.toml"),
        "version = 1\nid = \"not-the-planes-directory\"\n\n[members]\n",
    )
    .unwrap();

    // Being outside is something `bp list` reports, not a reason to refuse to
    // find it — otherwise `bp repair` could never be aimed at one from inside.
    let plane = host.show(containing(&elsewhere)).unwrap();

    // The directory's name is the identity, wherever the directory happens to be.
    assert_eq!(plane.id, "not-the-planes-directory");
    assert!(!plane.health.has_findings());
}

#[test]
fn a_directory_a_listing_reports_can_always_be_named_back_to_show() {
    let host = Host::new("show-hand-made-name");
    // A directory a user made by hand, whose name is not a well-formed plane
    // id. The name is still the plane's identity, so a listing reports it and
    // `-p` has to be able to reach it — the disagreement with the stored id is
    // a finding, not a reason to refuse to look.
    host.plane_file_saying(
        "Auth-Work",
        "version = 1\nid = \"auth-work\"\n\n[members]\n",
    );

    assert_eq!(host.list(HealthCheck::Cheap).planes[0].id, "Auth-Work");

    let plane = host
        .show(PlaneRef::Id {
            id: "Auth-Work".to_owned(),
        })
        .unwrap();

    assert_eq!(plane.id, "Auth-Work");
    assert_eq!(tags(&plane), ["id_mismatch"]);
}

#[test]
fn a_plane_id_that_is_not_one_path_segment_is_refused() {
    let host = Host::new("show-escaping-id");
    fs::create_dir_all(host.planes()).unwrap();
    fs::write(
        host.root.join("plane.toml"),
        "version = 1\nid = \"outside\"\n\n[members]\n",
    )
    .unwrap();

    // Joining this onto the planes directory would aim every later command
    // outside it.
    for id in ["..", ".", "", "../", "sub/plane", "/absolute"] {
        let error = host
            .show(PlaneRef::Id { id: id.to_owned() })
            .expect_err(&format!("{id:?} should not resolve"));

        assert_eq!(error.envelope().error, "invalid_plane_id", "for {id:?}");
        assert_eq!(error.exit_code(), ExitCode::Usage, "for {id:?}");
    }
}

#[test]
fn a_plane_directory_reached_through_a_symlink_keeps_the_name_it_has_here() {
    let host = Host::new("list-symlinked-plane");
    let elsewhere = host.root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(
        elsewhere.join("plane.toml"),
        "version = 1\nid = \"auth-work\"\n\n[members]\n",
    )
    .unwrap();

    fs::create_dir_all(host.planes()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, host.planes().join("auth-work")).unwrap();

    let plane = &host.list(HealthCheck::Cheap).planes[0];

    assert_eq!(plane.id, "auth-work");
    assert!(
        !plane.health.has_findings(),
        "the link's own name is the identity, so there is no mismatch to report"
    );
}

#[test]
fn standing_in_no_plane_says_so_and_names_the_way_out() {
    let host = Host::new("show-nowhere");

    let error = host.show(containing(&host.root)).unwrap_err();

    assert_eq!(error.envelope().error, "plane_not_found");
    assert_eq!(error.exit_code(), ExitCode::Usage);
    assert!(
        error.to_string().starts_with("no plane contains "),
        "got {error}"
    );
    assert_eq!(
        error.envelope().remedy.as_deref(),
        Some("cd into a plane, or name one with --plane.")
    );
}

#[test]
fn a_plane_id_naming_nothing_says_so_in_different_words() {
    let host = Host::new("show-no-such-id");

    let error = host
        .show(PlaneRef::Id {
            id: "auth-work".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.to_string(), "there is no plane called auth-work");
    assert_eq!(
        error.envelope().remedy.as_deref(),
        Some("Run bp list to see what planes exist.")
    );
}

#[test]
fn a_show_of_a_plane_file_that_will_not_parse_fails_rather_than_reporting_a_row() {
    let host = Host::new("show-unreadable");
    host.plane_file_saying("broken", "version = 1\nid = \"broken\"\nhost = \"local\"\n");

    let error = host
        .show(PlaneRef::Id {
            id: "broken".to_owned(),
        })
        .unwrap_err();

    // A listing tolerates a file it cannot read; a show has exactly one file.
    assert_eq!(error.envelope().error, "parse_error");
    assert_eq!(error.exit_code(), ExitCode::Failure);
}

#[test]
fn status_reports_what_git_says_about_each_member() {
    let host = Host::new("status-work");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let created = host.create(&[&alpha, &beta], "feat-a", Some("auth-work"));

    let worktree = created.directory.join("repos/alpha");
    fs::write(worktree.join("README.md"), "changed\n").unwrap();
    fs::write(worktree.join("notes.md"), "new\n").unwrap();

    let status = host
        .status(PlaneRef::Id {
            id: "auth-work".to_owned(),
        })
        .unwrap();

    let MemberWork::Reported {
        modified,
        untracked,
        ahead,
    } = status.members[0].work
    else {
        panic!("expected git to answer, got {:?}", status.members[0].work);
    };
    assert_eq!((modified, untracked), (1, 1));
    // The fixture has no `origin`, so every commit is contained in no
    // `refs/remotes/origin/*` — which is exactly what the predicate says.
    assert_eq!(ahead, 1);

    assert!(
        matches!(
            status.members[1].work,
            MemberWork::Reported {
                modified: 0,
                untracked: 0,
                ..
            }
        ),
        "got {:?}",
        status.members[1].work
    );
    assert!(
        !status.health.has_findings(),
        "uncommitted work is ordinary use, never drift"
    );
}

#[test]
fn a_member_whose_worktree_is_gone_is_a_row_and_a_finding_not_a_failed_read() {
    let host = Host::new("status-gone");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    let created = host.create(&[&alpha, &beta], "feat-a", Some("auth-work"));
    fs::remove_dir_all(created.directory.join("repos/beta")).unwrap();

    let status = host
        .status(PlaneRef::Id {
            id: "auth-work".to_owned(),
        })
        .unwrap();

    assert!(matches!(
        status.members[0].work,
        MemberWork::Reported { .. }
    ));
    assert_eq!(status.members[1].work, MemberWork::WorktreeMissing);
    assert_eq!(status.members[1].head, None);
    assert_eq!(
        status
            .health
            .findings
            .iter()
            .map(Finding::tag)
            .collect::<Vec<&str>>(),
        ["member_worktree_missing"],
        "git has nothing to say about a directory that is not there"
    );
}

#[test]
fn a_member_whose_source_repo_is_gone_says_so_rather_than_passing_gits_confusion_on() {
    let host = Host::new("status-source-gone");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", Some("auth-work"));
    fs::remove_dir_all(&alpha).unwrap();

    let status = host
        .status(PlaneRef::Id {
            id: "auth-work".to_owned(),
        })
        .unwrap();

    assert_eq!(status.members[0].work, MemberWork::SourceRepoMissing);
    assert_eq!(tags_of(&status), ["source_repo_missing"]);
}

fn containing(path: &Path) -> PlaneRef {
    PlaneRef::ContainingPath {
        path: path.to_path_buf(),
    }
}

fn branch(plane: &PlaneView) -> Vec<String> {
    plane
        .members
        .iter()
        .map(|member| match &member.head {
            Some(head) => head.to_string(),
            None => "-".to_owned(),
        })
        .collect()
}

fn tags(plane: &PlaneView) -> Vec<&'static str> {
    plane.health.findings.iter().map(Finding::tag).collect()
}

fn tags_of(status: &PlaneStatus) -> Vec<&'static str> {
    status.health.findings.iter().map(Finding::tag).collect()
}

/// Every file under `root`, with its contents, so two snapshots can be compared
/// outright. Symlinks are recorded by their target rather than followed.
fn tree(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };

        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let kind = entry.file_type().expect("stat an entry just listed");

            if kind.is_dir() {
                found.insert(path.clone(), String::from("<directory>"));
                pending.push(path);
            } else if kind.is_symlink() {
                found.insert(
                    path.clone(),
                    format!("<symlink to {:?}>", fs::read_link(&path).ok()),
                );
            } else {
                found.insert(
                    path.clone(),
                    fs::read_to_string(&path).unwrap_or_else(|_| "<binary>".to_owned()),
                );
            }
        }
    }

    found
}

/// A directory holding a `git` that records every invocation and does nothing
/// else, so "no git process was started" is checkable rather than asserted.
fn recording_git() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    let dir = scratch_dir(&format!(
        "recording-git-{}",
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let git = dir.join("git");
    fs::write(
        &git,
        format!(
            "#!/bin/sh\necho \"$@\" >> {}\nexit 1\n",
            dir.join("calls").display()
        ),
    )
    .expect("write the recording git");
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).expect("chmod the recording git");

    dir
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
        let path = self.root.join("repos").join(name);
        repository_with_one_commit(&path);
        path.canonicalize().expect("the repository was just made")
    }

    fn planes(&self) -> PathBuf {
        self.directories.planes().to_path_buf()
    }

    /// The planes directory as a read reports it.
    fn planes_as_resolved(&self) -> PathBuf {
        self.planes()
            .canonicalize()
            .expect("the planes directory exists by now")
    }

    fn engine(&self) -> LocalEngine {
        LocalEngine::new(self.directories.clone())
    }

    fn engine_using_git_at(&self, search_path: &Path) -> LocalEngine {
        self.engine().with_git_search_path(search_path)
    }

    fn create(&self, members: &[&Path], branch: &str, id: Option<&str>) -> PlaneCreated {
        self.engine()
            .plane_create(PlaneCreateRequest {
                members: members
                    .iter()
                    .map(|member| member.display().to_string())
                    .collect(),
                branch: Some(branch.to_owned()),
                id: id.map(ToOwned::to_owned),
                intent: BranchIntent::Resolve,
            })
            .expect("the fixture plane was created")
    }

    fn list(&self, health: HealthCheck) -> PlaneList {
        self.engine()
            .plane_list(PlaneListRequest { health })
            .expect("a listing never fails on a plane it cannot read")
    }

    fn show(&self, plane: PlaneRef) -> Result<PlaneView, EngineError> {
        self.show_at(plane, HealthCheck::Cheap)
    }

    fn show_at(&self, plane: PlaneRef, health: HealthCheck) -> Result<PlaneView, EngineError> {
        self.engine().plane_show(PlaneShowRequest { plane, health })
    }

    fn status(&self, plane: PlaneRef) -> Result<PlaneStatus, EngineError> {
        self.engine().plane_status(PlaneStatusRequest { plane })
    }

    /// A plane directory holding exactly the plane file given — the shape of
    /// something a user hand-edited into a state bitplane cannot read.
    fn plane_file_saying(&self, id: &str, text: &str) {
        let directory = self.planes().join(id);
        fs::create_dir_all(&directory).expect("create the plane directory");
        fs::write(directory.join("plane.toml"), text).expect("write the plane file");
    }

    /// Rewrites `plane.toml`'s `id` without moving the directory, which is what
    /// a plane moved outside bitplane looks like from the inside.
    fn rewrite_id(&self, directory: &str, recorded: &str) {
        let path = self.planes().join(directory).join("plane.toml");
        let text = fs::read_to_string(&path).expect("read the plane file");

        fs::write(
            &path,
            text.replace(
                &format!("id = {directory:?}"),
                &format!("id = {recorded:?}"),
            ),
        )
        .expect("rewrite the plane file");
    }
}
