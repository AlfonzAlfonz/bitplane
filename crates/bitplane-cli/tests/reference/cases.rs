//! One case per documented example: the world it assumes, and whether the
//! binary is expected to agree with the page yet.
//!
//! This file is the hand-written half of the suite. A page says what an
//! invocation prints; only a person can say what has to exist for it to print
//! that — which projects are registered, which branch the forge already has,
//! which worktree was deleted behind bitplane's back. The case is that sentence
//! in Rust.
//!
//! **Adding a command means adding its cases.** An example with no case fails
//! the suite, so there is no way to document an invocation and leave it
//! unasserted.

use std::fs;
use std::path::Path;

use crate::world::{World, git};

/// One documented example, staged.
pub struct Case {
    /// The example's heading on the page, and `#2` where a heading holds more
    /// than one.
    pub example: &'static str,
    pub expectation: Expectation,
    /// Everything that has to exist before the command is run.
    pub build: fn(&mut World),
}

/// What the suite does with the result.
pub enum Expectation {
    /// The binary prints what the page says, and a difference is a failure.
    Matches,
    /// The page describes something that is not built yet, so the case is
    /// expected to **disagree**. When the ticket that builds it lands, the case
    /// starts agreeing and the suite fails until this marker comes off — which
    /// is what keeps the gap visible and shrinking.
    NotBuilt(&'static str),
    /// Nothing a test process can stage, with the reason. Present rather than
    /// omitted, so the example is accounted for rather than forgotten.
    Unstageable(&'static str),
}

/// The cases for one page.
pub fn of(page: &str) -> Vec<Case> {
    match page {
        "exit-codes.md" | "global-flags.md" => vec![],
        "member-syntax.md" => member_syntax(),
        "refusals-and-waivers.md" => refusals(),
        "plane/add.md" => plane_add(),
        "plane/create.md" => plane_create(),
        "plane/destroy.md" => plane_destroy(),
        "plane/doctor.md" => plane_doctor(),
        "plane/list.md" => plane_list(),
        "plane/rename.md" => plane_rename(),
        "plane/repair.md" => plane_repair(),
        "plane/rm.md" => plane_rm(),
        "plane/run.md" => plane_run(),
        "plane/show.md" => plane_show(),
        "plane/status.md" => plane_status(),
        "project/add.md" => project_add(),
        "project/adopt.md" => project_adopt(),
        "project/fetch.md" => project_fetch(),
        "project/list.md" => project_list(),
        "project/rename.md" => project_rename(),
        "project/rm.md" => project_rm(),
        "project/show.md" => project_show(),
        other => panic!("no cases are registered for {other}"),
    }
}

const API: &str = "git@gitlab.com:acme/api.git";
const WEB: &str = "git@gitlab.com:acme/web.git";
const DOCS: &str = "git@gitlab.com:acme/docs.git";
const CODESTYLE: &str = "git@gitlab.com:acme/codestyle.git";
const BITPLANE: &str = "/Users/alfonz/projects/bitplane";

fn member_syntax() -> Vec<Case> {
    vec![Case {
        example: "The one path a suffix cannot express",
        expectation: Expectation::Matches,
        build: |world| {
            // The ambiguity is only real where both readings could be: a
            // directory at the whole spec, and a branch suffix on the rest.
            world.checkout("/Users/alfonz/projects/weird:name");
        },
    }]
}

fn refusals() -> Vec<Case> {
    vec![Case {
        example: "What a refusal prints",
        expectation: Expectation::Matches,
        build: |world| {
            world.project_with(API, &["feat-login"]);
            world.project_with(WEB, &["feat-login"]);
            world.project_with(DOCS, &["feat-login"]);

            let plane = world.anonymous_plane(&["@api", "@web", "@docs"], "feat-login");
            dirty(&plane.join("acme/api"));
            commit(&plane.join("acme/api"), "later.md");
            fs::write(plane.join("acme/web/notes.md"), "new\n").expect("write an untracked file");
            world.cd(plane);
        },
    }]
}

fn plane_add() -> Vec<Case> {
    vec![
        Case {
            example: "Output",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);
                world.project_with(DOCS, &["feat-login"]);

                let plane = world.anonymous_plane(&["@api", "@web"], "feat-login");
                world.cd(plane);
            },
        },
        Case {
            example: "The member is already in the plane",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                let plane = world.anonymous_plane(&["@api"], "feat-login");
                world.cd(plane);
            },
        },
        Case {
            example: "Another bp holds the plane",
            expectation: Expectation::Unstageable(
                "the lock blocks for 120 seconds before it times out, which is \
                 longer than a test suite may take; the contended path is \
                 covered by the core's own lock tests",
            ),
            build: |_| {},
        },
    ]
}

fn plane_create() -> Vec<Case> {
    vec![
        Case {
            example: "Output",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);
            },
        },
        Case {
            example: "Output #2",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.checkout(BITPLANE);
            },
        },
        Case {
            example: "The id is already taken",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.latched_remnant("auth-work");
            },
        },
        Case {
            example: "git is too old",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.shim_git(
                    "if [ \"$1\" = --version ]; then echo 'git version 2.35.9'; exit 0; fi",
                );
            },
        },
        Case {
            example: "A member fails partway through",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);
                world.project_with(DOCS, &["feat-login"]);
                // git refusing one member's worktree, and only that one: the
                // page's example is a failure between two members that worked.
                world.shim_git(
                    "case \" $* \" in *acme/web*worktree*|*worktree*acme/web*) \
                     echo 'fatal: invalid reference: feat-login' >&2; exit 128;; esac",
                );
            },
        },
        Case {
            example: "Ctrl-C",
            expectation: Expectation::Unstageable(
                "which member the first interrupt lands between is a race a \
                 snapshot cannot pin; tests/create.rs drives the signal itself \
                 and asserts the same rows",
            ),
            build: |_| {},
        },
    ]
}

fn plane_destroy() -> Vec<Case> {
    vec![
        Case {
            example: "Output",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);
                world.project_with(DOCS, &["feat-login"]);
                world.checkout(BITPLANE);

                let plane = world.anonymous_plane(
                    &["@api", "@web", "~/projects/bitplane", "@docs"],
                    "feat-login",
                );
                world.forget("docs", &plane.join("acme/docs"));
                world.cd(plane);
            },
        },
        Case {
            example: "Refused",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);
                world.project_with(DOCS, &["feat-login"]);

                let plane = world.anonymous_plane(&["@api", "@web", "@docs"], "feat-login");
                dirty(&plane.join("acme/api"));
                commit(&plane.join("acme/api"), "later.md");
                fs::write(plane.join("acme/web/notes.md"), "new\n")
                    .expect("write an untracked file");
                world.cd(plane);
            },
        },
        Case {
            example: "A plane that was never finished being created",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.plane("auth-work", &["@api"], "feat-login");
                world.latch("auth-work");
            },
        },
    ]
}

fn plane_doctor() -> Vec<Case> {
    const WHY: &str = "bp doctor is ticket 16";

    vec![
        Case {
            example: "Output",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
        Case {
            example: "Nothing is wrong",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
        Case {
            example: "A latched plane",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
        Case {
            example: "An orphaned branch",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
        Case {
            example: "--footprint",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
    ]
}

fn plane_list() -> Vec<Case> {
    vec![
        Case {
            example: "Output",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login", "feat-x"]);
                world.project_with(WEB, &["feat-login"]);
                world.checkout_on(BITPLANE, "workbench");

                world.plane("auth-work", &["@api", "@web"], "feat-login");
                world.anonymous_plane(&["@api", "~/projects/bitplane:main"], "feat-x");
            },
        },
        Case {
            example: "Everything is fine",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);
                world.anonymous_plane(&["@api", "@web"], "feat-login");
            },
        },
        Case {
            example: "Something has drifted",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);

                let auth = world.plane("auth-work", &["@api", "@web"], "feat-login");
                fs::remove_dir_all(auth.join("acme/web")).expect("drop a worktree by hand");

                // Written rather than created: a second plane really holding
                // @api on feat-login would be refused, and a latched plane
                // never got as far as holding anything.
                world.latched_remnant("bp-7c1e0d44");

                let broken = world.planes().join("broken.plane");
                fs::create_dir_all(&broken).expect("make a plane directory");
                fs::write(
                    broken.join("plane.toml"),
                    "version = 1\nid = \"broken.plane\"\nstatus = \"fine\"\n",
                )
                .expect("write a plane file bitplane will not read");
            },
        },
    ]
}

fn plane_rename() -> Vec<Case> {
    vec![Case {
        example: "Output",
        expectation: Expectation::NotBuilt("bp rename is ticket 14"),
        build: |_| {},
    }]
}

fn plane_repair() -> Vec<Case> {
    const WHY: &str = "bp repair is ticket 14";

    vec![
        Case {
            example: "Output",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
        Case {
            example: "Output #2",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
        Case {
            example: "A member's worktree is missing entirely",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
    ]
}

fn plane_rm() -> Vec<Case> {
    vec![
        Case {
            example: "Branch deletion",
            expectation: Expectation::Matches,
            build: |world| {
                world.checkout(BITPLANE);
                let plane = world.anonymous_plane(&["~/projects/bitplane"], "feat-login");
                world.cd(plane);
            },
        },
        Case {
            example: "Output",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(DOCS, &["feat-login"]);
                let plane = world.anonymous_plane(&["@docs"], "feat-login");
                world.cd(plane);
            },
        },
        Case {
            example: "Output #2",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(DOCS, &["feat-login"]);
                let plane = world.anonymous_plane(&["@docs"], "feat-login");
                world.forget("docs", &plane.join("acme/docs"));
                world.cd(plane);
            },
        },
        Case {
            example: "Refused",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);

                let plane = world.anonymous_plane(&["@api", "@web"], "feat-login");
                dirty(&plane.join("acme/api"));
                fs::write(plane.join("acme/web/notes.md"), "new\n")
                    .expect("write an untracked file");
                world.cd(plane);
            },
        },
        Case {
            example: "Waived",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);

                let plane = world.anonymous_plane(&["@api", "@web"], "feat-login");
                dirty(&plane.join("acme/api"));
                fs::write(plane.join("acme/web/notes.md"), "new\n")
                    .expect("write an untracked file");
                world.cd(plane);
            },
        },
    ]
}

fn plane_run() -> Vec<Case> {
    const WHY: &str = "scripts and bp run are ticket 13";

    vec![
        Case {
            example: "Running a script",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
        Case {
            example: "Several scripts, in the order you named them",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
    ]
}

fn plane_show() -> Vec<Case> {
    vec![
        Case {
            example: "Output",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);
                world.checkout_on(BITPLANE, "workbench");

                let plane = world
                    .anonymous_plane(&["@api", "@web", "~/projects/bitplane:main"], "feat-login");
                world.cd(plane);
            },
        },
        Case {
            example: "There is no plane here",
            expectation: Expectation::Matches,
            build: |world| {
                let src = world.real("/Users/alfonz/src");
                fs::create_dir_all(&src).expect("make a directory that is in no plane");
                world.cd(src);
            },
        },
        Case {
            example: "The plane has a finding",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);

                let plane = world.plane("auth-work", &["@api", "@web"], "feat-login");
                fs::remove_dir_all(plane.join("acme/web")).expect("drop a worktree by hand");
            },
        },
        Case {
            example: "The plane was moved outside bp",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);

                let plane = world.anonymous_plane(&["@api", "@web"], "feat-login");
                let moved = world.planes().join("auth-work");
                fs::rename(&plane, &moved).expect("move a plane the way a person would");
            },
        },
    ]
}

fn plane_status() -> Vec<Case> {
    vec![
        Case {
            example: "Output",
            expectation: Expectation::Matches,
            build: |world| {
                busy_plane(world, true);
            },
        },
        Case {
            example: "A plane with work in it",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);

                let plane = world.anonymous_plane(&["@api", "@web"], "feat-login");
                let api = plane.join("acme/api");
                commit(&api, "one.md");
                commit(&api, "two.md");
                modify(&api, &["a.md", "b.md", "c.md"]);
                fs::write(api.join("scratch.md"), "new\n").expect("write an untracked file");
                world.cd(plane);
            },
        },
        Case {
            example: "A member whose worktree is gone",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);
                world.project_with(WEB, &["feat-login"]);

                let plane = world.anonymous_plane(&["@api", "@web"], "feat-login");
                let api = plane.join("acme/api");
                modify(&api, &["a.md", "b.md", "c.md"]);
                fs::write(api.join("scratch.md"), "new\n").expect("write an untracked file");
                fs::remove_dir_all(plane.join("acme/web")).expect("drop a worktree by hand");
                world.cd(plane);
            },
        },
        Case {
            example: "A detached HEAD",
            expectation: Expectation::Matches,
            build: |world| {
                world.project_with(API, &["feat-login"]);

                let plane = world.anonymous_plane(&["@api"], "feat-login");
                git(
                    &plane.join("acme/api"),
                    &["checkout", "--quiet", "--detach"],
                );
                world.cd(plane);
            },
        },
    ]
}

fn project_add() -> Vec<Case> {
    vec![
        Case {
            example: "Output",
            expectation: Expectation::Matches,
            build: |world| {
                world.forge(CODESTYLE);
            },
        },
        Case {
            example: "The default name is taken",
            expectation: Expectation::Matches,
            build: |world| {
                world.project(CODESTYLE);
            },
        },
        Case {
            example: "The URL derives a name the charset will not take",
            expectation: Expectation::Matches,
            build: |world| {
                world.forge("git@gitlab.com:acme/MyProject.git");
            },
        },
    ]
}

fn project_adopt() -> Vec<Case> {
    vec![Case {
        example: "Output",
        expectation: Expectation::NotBuilt("bp project adopt is ticket 11"),
        build: |world| {
            world.checkout(BITPLANE);
        },
    }]
}

fn project_fetch() -> Vec<Case> {
    vec![
        Case {
            example: "Output",
            expectation: Expectation::Matches,
            build: |world| {
                let registry = registry(world);
                grow(&registry, &["one", "two", "three"]);
            },
        },
        Case {
            example: "One project",
            expectation: Expectation::Matches,
            build: |world| {
                let registry = registry(world);
                grow(&registry, &["one", "two", "three"]);
            },
        },
        Case {
            example: "The forge is unreachable",
            expectation: Expectation::NotBuilt(
                "a failed fetch is reported as git_failed, quoting git's own \
                 `git fetch --quiet exited 128`, rather than in the \
                 `could not fetch origin:` words this page and \
                 `bp project add` both use",
            ),
            build: |world| {
                let registry = registry(world);
                fs::remove_dir_all(&registry).expect("take the forge away");
            },
        },
    ]
}

fn project_list() -> Vec<Case> {
    vec![
        Case {
            example: "Output",
            expectation: Expectation::Matches,
            build: |world| {
                registry(world);
            },
        },
        Case {
            example: "A project file will not parse",
            expectation: Expectation::Matches,
            build: |world| {
                registry(world);
                let file = world.projects().join("codestyle/project.toml");
                let text = fs::read_to_string(&file).expect("the project was just registered");
                fs::write(
                    file,
                    text.replace("\n\n[source]", "\ndefault_branch = \"main\"\n\n[source]"),
                )
                .expect("write a project file bitplane will not read");
            },
        },
    ]
}

fn project_rename() -> Vec<Case> {
    const WHY: &str = "bp project rename is ticket 15";

    vec![
        Case {
            example: "Output",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
        Case {
            example: "Output #2",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
    ]
}

fn project_rm() -> Vec<Case> {
    const WHY: &str = "bp project rm is ticket 15";

    vec![
        Case {
            example: "Output",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
        Case {
            example: "A plane still holds it",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
    ]
}

fn project_show() -> Vec<Case> {
    const WHY: &str = "bp project show is ticket 11";

    vec![
        Case {
            example: "Output",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
        Case {
            example: "An adopted project",
            expectation: Expectation::NotBuilt(WHY),
            build: |_| {},
        },
    ]
}

/// The three projects the `project` pages list: two owned and one adopted.
/// Returns `@api`'s forge, which is the one those pages fetch from.
fn registry(world: &mut World) -> std::path::PathBuf {
    let api = world.forge(API);
    world.register(&api);
    world.project(CODESTYLE);

    let checkout = world.checkout(BITPLANE);
    world.adopted("bitplane", &checkout);

    api
}

/// The plane `bp status` reports on: three members, one of them busy.
fn busy_plane(world: &mut World, ahead: bool) {
    world.project_with(API, &["feat-login"]);
    world.project_with(WEB, &["feat-login"]);
    world.project_with(DOCS, &["feat-login"]);

    let plane = world.anonymous_plane(&["@api", "@web", "@docs"], "feat-login");
    let api = plane.join("acme/api");
    commit(&api, "one.md");
    commit(&api, "two.md");
    modify(&api, &["a.md", "b.md", "c.md"]);
    fs::write(api.join("scratch.md"), "new\n").expect("write an untracked file");

    if ahead {
        commit(&plane.join("acme/docs"), "note.md");
    }

    world.cd(plane);
}

/// More branches on a forge, so the next fetch has something to report.
fn grow(forge: &Path, branches: &[&str]) {
    for branch in branches {
        git(forge, &["branch", branch]);
    }
}

/// A tracked file with changes in it.
fn dirty(worktree: &Path) {
    fs::write(worktree.join("README.md"), "changed\n").expect("change a tracked file");
}

/// Three tracked files with changes in them.
fn modify(worktree: &Path, files: &[&str]) {
    for file in files {
        fs::write(worktree.join(file), "changed\n").expect("change a tracked file");
    }
}

/// A commit this worktree has and its origin does not.
fn commit(worktree: &Path, file: &str) {
    fs::write(worktree.join(file), "new\n").expect("write a file to commit");
    git(worktree, &["add", file]);
    git(worktree, &["commit", "--quiet", "--message", file]);
}
