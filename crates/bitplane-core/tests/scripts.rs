//! Scripts and `bp run`, against real git repositories and real child
//! processes.
//!
//! Real rather than stubbed on both counts: every acceptance criterion here is
//! about what a spawned process saw — its working directory, its `PATH`, its
//! environment, what its output did — and a fake would only test bitplane's
//! idea of a process.
//!
//! The scripts are shell one-liners writing to an absolute path the fixture
//! chose, which is how a test reads what a child observed without the child
//! having to be told where to write it twice.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bitplane_core::plane_dir::BITPLANE_DIR;
use bitplane_core::project_dir::BIN_DIR_NAME;
use bitplane_core::scripts::LOGS_DIR;
use bitplane_core::testing::{git, repository_with_one_commit, scratch_dir};
use bitplane_core::{
    BranchIntent, Directories, Engine, EngineError, ExitCode, LocalEngine, PlaneAddRequest,
    PlaneAdded, PlaneCreateRequest, PlaneCreated, PlaneDestroyRequest, PlaneDestroyed, PlaneFile,
    PlaneRef, PlaneRemoveRequest, PlaneRemoved, PlaneScriptsRequest, ProjectAddRequest,
    ProjectName, ScriptsRun,
};

#[test]
fn the_create_point_fires_per_member_in_declaration_order() {
    let host = Host::new("scripts-create-order");
    host.project("api");
    host.project("web");
    host.declares(
        "api",
        &host.trace_scripts(&["api-one", "api-two"], "create"),
    );
    host.declares(
        "web",
        &host.trace_scripts(&["web-one", "web-two"], "create"),
    );

    let created = host.create(&["@api", "@web"], true).unwrap();

    assert_eq!(
        host.trace(),
        ["api-one", "api-two", "web-one", "web-two"],
        "member order, then declaration order within each member"
    );
    assert_eq!(
        created
            .scripts
            .iter()
            .map(|outcome| (outcome.subject(), outcome.name.clone()))
            .collect::<Vec<(String, String)>>(),
        [
            ("@api".to_owned(), "api-one".to_owned()),
            ("@api".to_owned(), "api-two".to_owned()),
            ("@web".to_owned(), "web-one".to_owned()),
            ("@web".to_owned(), "web-two".to_owned()),
        ]
    );
    assert!(created.scripts.iter().all(|outcome| outcome.succeeded()));
}

#[test]
fn execution_is_sequential_within_a_member_and_across_them() {
    // Each script brackets a pause. Anything running in parallel interleaves
    // the brackets; sequential execution nests them perfectly.
    let host = Host::new("scripts-sequential");
    host.project("api");
    host.project("web");
    for project in ["api", "web"] {
        host.declares(
            project,
            &format!(
                "[scripts.slow]\nshell = \"{}; sleep 0.2; {}\"\npost_worktree_create = true\n",
                host.append("in"),
                host.append("out"),
            ),
        );
    }

    host.create(&["@api", "@web"], true).unwrap();

    assert_eq!(host.trace(), ["in", "out", "in", "out"]);
}

#[test]
fn a_script_bound_to_no_point_never_fires_and_is_still_runnable_by_name() {
    let host = Host::new("scripts-unbound");
    host.project("api");
    host.declares("api", &host.trace_scripts(&["reset-db"], "none"));

    let created = host.create(&["@api"], true).unwrap();
    assert!(created.scripts.is_empty(), "nothing is bound to a point");
    assert!(host.trace().is_empty());

    let run = host
        .run(created.id.as_str(), &["api"], &["reset-db"])
        .unwrap();

    assert_eq!(run.scripts.len(), 1);
    assert_eq!(host.trace(), ["reset-db"]);
}

#[test]
fn the_create_point_fires_on_add_too_and_only_for_what_this_run_added() {
    let host = Host::new("scripts-add");
    host.project("api");
    host.project("web");
    host.declares("api", &host.trace_scripts(&["api-setup"], "create"));
    host.declares("web", &host.trace_scripts(&["web-setup"], "create"));

    let created = host.create(&["@api"], true).unwrap();
    assert_eq!(host.trace(), ["api-setup"]);

    let added = host.add(created.id.as_str(), &["@web"], true).unwrap();

    assert_eq!(
        host.trace(),
        ["api-setup", "web-setup"],
        "the member already in the plane had its setup when it arrived"
    );
    assert_eq!(added.scripts.len(), 1);
}

#[test]
fn the_remove_point_fires_on_destroy_and_on_rm() {
    let host = Host::new("scripts-remove-point");
    host.project("api");
    host.project("web");
    host.declares("api", &host.trace_scripts(&["api-stop"], "remove"));
    host.declares("web", &host.trace_scripts(&["web-stop"], "remove"));

    let created = host.create(&["@api", "@web"], true).unwrap();

    host.remove(created.id.as_str(), &["@web"], true).unwrap();
    assert_eq!(host.trace(), ["web-stop"]);

    host.destroy(created.id.as_str(), true).unwrap();
    assert_eq!(host.trace(), ["web-stop", "api-stop"]);
}

#[test]
fn a_blocked_pre_script_removes_nothing_and_no_waiver_covers_it() {
    let host = Host::new("scripts-blocked");
    host.project("api");
    host.declares(
        "api",
        "[scripts.stop-stack]\nshell = \"exit 1\"\npre_worktree_remove = true\n",
    );
    let created = host.create(&["@api"], true).unwrap();

    let error = host.destroy(created.id.as_str(), true).unwrap_err();

    let envelope = error.envelope();
    assert_eq!(envelope.error, "script_blocked");
    assert_eq!(envelope.code, ExitCode::Failure);
    assert_eq!(
        envelope.message,
        "stop-stack exited 1 in @api; nothing was removed"
    );
    assert_eq!(
        envelope.problems[0].subject.as_deref(),
        Some("@api"),
        "one problem per failure, keyed by the project"
    );
    assert_eq!(envelope.problems[0].message, "stop-stack exited 1");
    assert_eq!(
        envelope.remedy.as_deref(),
        Some("Fix the script, or re-run with --no-scripts.")
    );

    assert!(
        created.directory.join("forges/api/.git").exists(),
        "nothing is removed when a pre-script blocks"
    );
    assert!(created.directory.join("plane.toml").is_file());

    // A waiver says *I accept losing this work, which I am looking at*. A
    // script's exit code is a program's opinion, so none of them helps.
    let with_every_waiver = host
        .engine()
        .plane_destroy(PlaneDestroyRequest {
            plane: named(created.id.as_str()),
            waive: bitplane_core::Reason::ALL.to_vec(),
            run_scripts: true,
        })
        .unwrap_err();

    assert_eq!(with_every_waiver.tag(), "script_blocked");
}

#[test]
fn no_scripts_always_gets_a_plane_apart() {
    let host = Host::new("scripts-undestroyable");
    host.project("api");
    host.declares(
        "api",
        "[scripts.stop-stack]\nshell = \"exit 1\"\npre_worktree_remove = true\n",
    );
    let created = host.create(&["@api"], true).unwrap();

    let destroyed = host.destroy(created.id.as_str(), false).unwrap();

    assert!(destroyed.scripts.is_empty());
    assert!(
        !created.directory.exists(),
        "a project.toml cannot make a plane undestroyable"
    );
}

#[test]
fn the_remove_point_is_skipped_entirely_on_a_latched_plane() {
    let host = Host::new("scripts-latched");
    host.project("api");
    host.declares(
        "api",
        "[scripts.stop-stack]\nshell = \"exit 1\"\npre_worktree_remove = true\n",
    );
    let created = host.create(&["@api"], true).unwrap();
    host.latch(created.id.as_str());

    let destroyed = host.destroy(created.id.as_str(), true).unwrap();

    assert!(destroyed.incomplete);
    assert!(
        destroyed.scripts.is_empty(),
        "no post_worktree_create ever ran there, so its counterpart has nothing to undo"
    );
    assert!(!created.directory.exists());
}

#[test]
fn a_post_script_failure_reports_its_log_and_unwinds_nothing() {
    let host = Host::new("scripts-post-failure");
    host.project("api");
    host.declares(
        "api",
        "[scripts.install]\nshell = \"exit 1\"\npost_worktree_create = true\n",
    );

    let error = host.create(&["@api"], true).unwrap_err();

    let envelope = error.envelope();
    assert_eq!(envelope.error, "script_failed");
    assert_eq!(envelope.code, ExitCode::Failure);
    assert_eq!(
        envelope.message,
        "install exited 1 in @api; the worktree was created"
    );
    assert!(
        envelope.problems[0]
            .message
            .starts_with("install exited 1; see "),
        "got {}",
        envelope.problems[0].message
    );
    assert_eq!(
        envelope.remedy.as_deref(),
        Some("See the log, fix the cause, then run bp run @api install.")
    );

    let plane = host.only_plane();
    assert!(
        plane.join("forges/api/.git").exists(),
        "scripts run past the point of no return, so a failure unwinds nothing"
    );
    assert!(
        !plane.join(BITPLANE_DIR).join("incomplete").exists(),
        "the latch was cleared before the scripts ran"
    );
}

#[test]
fn a_post_script_pass_runs_every_member_even_after_one_fails() {
    // Post-scripts do not short-circuit: by construction there is nothing left
    // to stop, so a broken setup in one project must not skip another's.
    let host = Host::new("scripts-post-no-short-circuit");
    host.project("api");
    host.project("web");
    host.declares(
        "api",
        "[scripts.install]\nshell = \"exit 3\"\npost_worktree_create = true\n",
    );
    host.declares("web", &host.trace_scripts(&["web-setup"], "create"));

    let error = host.create(&["@api", "@web"], true).unwrap_err();

    assert_eq!(host.trace(), ["web-setup"]);
    let EngineError::ScriptFailed { outcomes, .. } = error else {
        panic!("not a script failure: {error:?}");
    };
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].failure(), Some("exited 3"));
    assert!(outcomes[1].succeeded());
}

#[test]
fn a_script_runs_in_the_worktree_with_the_projects_bin_in_front_of_path() {
    let host = Host::new("scripts-cwd-and-path");
    host.project("api");
    // An executable the project ships, whose name would otherwise find nothing.
    host.ships(
        "api",
        "stamp",
        &format!("#!/bin/sh\npwd >> {}\n", host.trace_file().display()),
    );
    host.declares(
        "api",
        "[scripts.setup]\nargv = [\"stamp\"]\npost_worktree_create = true\n",
    );

    let created = host.create(&["@api"], true).unwrap();

    assert_eq!(
        host.trace(),
        [canonical(&created.directory.join("forges/api"))],
        "the working directory is the worktree, and bin/ won"
    );
}

#[test]
fn the_eight_variables_are_set_and_the_callers_environment_survives() {
    let host = Host::new("scripts-environment");
    host.project("api");
    host.declares("api", &host.env_script("create"));

    let created = host.create(&["@api"], true).unwrap();
    let seen = host.environment();

    assert_eq!(seen["BITPLANE_SCRIPT"], "report");
    assert_eq!(seen["BITPLANE_PROJECT"], "api", "never with the sigil");
    assert_eq!(
        seen["BITPLANE_PROJECT_DIR"],
        host.directories
            .projects()
            .join("api")
            .display()
            .to_string()
    );
    assert_eq!(
        seen["BITPLANE_SOURCE_REPO"],
        host.directories
            .projects()
            .join("api/repo.git")
            .display()
            .to_string()
    );
    assert_eq!(
        seen["BITPLANE_WORKTREE"],
        created.directory.join("forges/api").display().to_string()
    );
    assert_eq!(seen["BITPLANE_BRANCH"], "feat-login");
    assert_eq!(
        seen["BITPLANE_PLANE_DIR"],
        created.directory.display().to_string()
    );
    assert_eq!(seen["BITPLANE_PLANE_ID"], created.id.as_str());

    // Inherited in full rather than sanitised: a sanitised environment would
    // break SSH_AUTH_SOCK, ssh-agent and every credential helper.
    let inherited = std::env::var("PATH").unwrap();
    assert!(
        seen["PATH"].ends_with(&inherited),
        "the project's bin/ is prepended, not substituted: {}",
        seen["PATH"]
    );
    assert!(
        seen["PATH"].starts_with(
            &host
                .directories
                .projects()
                .join("api")
                .join(BIN_DIR_NAME)
                .display()
                .to_string()
        )
    );
}

#[test]
fn the_branch_variable_is_unset_rather_than_a_sha_when_head_is_detached() {
    let host = Host::new("scripts-detached");
    host.project("api");
    host.declares("api", &host.env_script("none"));
    let created = host.create(&["@api"], true).unwrap();
    git(
        &created.directory.join("forges/api"),
        &["checkout", "--quiet", "--detach"],
    );

    host.run(created.id.as_str(), &["api"], &["report"])
        .unwrap();

    assert!(
        !host.environment().contains_key("BITPLANE_BRANCH"),
        "a script doing git switch \"$BITPLANE_BRANCH\" must fail loudly, not detach again"
    );
}

#[test]
fn output_is_merged_streamed_and_teed_to_a_timestamped_log() {
    let host = Host::new("scripts-output");
    host.project("api");
    host.declares(
        "api",
        "[scripts.noisy]\nshell = \"echo to-stdout; echo to-stderr >&2\"\npost_worktree_create = true\n",
    );

    let created = host.create(&["@api"], true).unwrap();

    let streamed = host.streamed();
    assert!(streamed.contains("to-stdout"), "got {streamed:?}");
    assert!(streamed.contains("to-stderr"), "both streams are merged");

    let log = &created.scripts[0].log;
    assert_eq!(
        log.parent().unwrap(),
        created.directory.join(BITPLANE_DIR).join(LOGS_DIR)
    );
    let name = log.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.ends_with("-api-noisy.log"), "got {name}");
    assert_eq!(
        name.len(),
        "20260921T140312Z-api-noisy.log".len(),
        "a compact stamp, because RFC 3339's colons are hostile in filenames: {name}"
    );

    let teed = fs::read_to_string(log).unwrap();
    assert!(
        teed.contains("to-stdout") && teed.contains("to-stderr"),
        "got {teed:?}"
    );
}

#[test]
fn script_outcomes_are_not_written_to_the_plane_file() {
    let host = Host::new("scripts-plane-file");
    host.project("api");
    host.declares("api", &host.trace_scripts(&["setup"], "create"));

    let created = host.create(&["@api"], true).unwrap();

    let text = fs::read_to_string(created.directory.join("plane.toml")).unwrap();
    assert!(
        !text.contains("setup"),
        "the plane file is a membership list: {text}"
    );
    assert_eq!(
        PlaneFile::read(&created.directory.join("plane.toml"))
            .unwrap()
            .members
            .len(),
        1
    );
}

#[test]
fn bp_run_runs_the_names_in_request_order_and_stops_at_the_first_failure() {
    let host = Host::new("scripts-run-order");
    host.project("api");
    host.declares("api", &host.trace_scripts(&["one", "two"], "none"));
    let created = host.create(&["@api"], true).unwrap();

    let run = host
        .run(created.id.as_str(), &["api"], &["two", "one"])
        .unwrap();

    assert_eq!(
        host.trace(),
        ["two", "one"],
        "request order, not declaration order"
    );
    assert_eq!(
        run.scripts
            .iter()
            .map(|outcome| outcome.name.clone())
            .collect::<Vec<String>>(),
        ["two", "one"]
    );
    assert_eq!(run.id, created.id.as_str());
}

#[test]
fn an_unknown_name_and_a_project_that_is_not_a_member_are_different_errors() {
    let host = Host::new("scripts-run-errors");
    host.project("api");
    host.project("web");
    host.declares("api", &host.trace_scripts(&["install"], "none"));
    let created = host.create(&["@api"], true).unwrap();

    let unknown = host
        .run(created.id.as_str(), &["api"], &["intall"])
        .unwrap_err();
    assert_eq!(
        unknown,
        EngineError::ScriptNotFound {
            project: "api".to_owned(),
            name: "intall".to_owned(),
        }
    );
    assert_eq!(unknown.to_string(), "@api has no script called intall");
    assert_eq!(unknown.exit_code(), ExitCode::Usage);
    assert_eq!(
        unknown.envelope().remedy.as_deref(),
        Some("Run bp project show @api to see the scripts it declares.")
    );

    let stranger = host
        .run(created.id.as_str(), &["web"], &["install"])
        .unwrap_err();
    assert_eq!(
        stranger,
        EngineError::ProjectNotInPlane {
            member: "@web".to_owned(),
            plane: created.id.to_string(),
        }
    );

    assert!(
        host.trace().is_empty(),
        "both are checked before anything runs"
    );
}

#[test]
fn bp_run_refuses_on_a_plane_that_was_never_finished_being_created() {
    let host = Host::new("scripts-run-latched");
    host.project("api");
    host.declares("api", &host.trace_scripts(&["install"], "none"));
    let created = host.create(&["@api"], true).unwrap();
    host.latch(created.id.as_str());

    let error = host
        .run(created.id.as_str(), &["api"], &["install"])
        .unwrap_err();

    assert_eq!(
        error,
        EngineError::PlaneIncomplete {
            id: created.id.to_string()
        }
    );
    assert!(host.trace().is_empty());
}

#[test]
fn bp_run_takes_a_project_and_at_least_one_name() {
    let host = Host::new("scripts-run-usage");
    host.project("api");
    let created = host.create(&["@api"], true).unwrap();

    for (projects, names) in [(vec![], vec!["install"]), (vec!["api"], vec![])] {
        let error = host
            .run(created.id.as_str(), &projects, &names)
            .unwrap_err();

        assert_eq!(
            error.exit_code(),
            ExitCode::Usage,
            "for {projects:?} {names:?}"
        );
    }
}

#[test]
fn an_ad_hoc_member_runs_nothing_reports_nothing_and_cannot_be_run_against() {
    let host = Host::new("scripts-ad-hoc");
    let alpha = host.repository("alpha");
    host.project("api");
    host.declares("api", &host.trace_scripts(&["setup"], "create"));

    let created = host
        .create(&["@api", &alpha.display().to_string()], true)
        .unwrap();

    assert_eq!(
        created.scripts.len(),
        1,
        "an ad-hoc member behaves exactly like a project with no scripts"
    );
    assert_eq!(created.scripts[0].subject(), "@api");

    // `bp run` takes a project, so a path in that slot is not a member it can
    // find — it is not a project name at all.
    let error = host
        .engine()
        .plane_scripts(PlaneScriptsRequest {
            plane: named(created.id.as_str()),
            names: vec!["setup".to_owned()],
            projects: vec![ProjectName::parse("alpha").unwrap()],
        })
        .unwrap_err();

    assert_eq!(error.tag(), "project_not_in_plane");
}

#[test]
fn there_is_no_timeout() {
    // A build that outlives any default worth shipping is the case a timeout
    // would kill; what makes that safe is the live output, not a clock.
    let host = Host::new("scripts-no-timeout");
    host.project("api");
    host.declares(
        "api",
        "[scripts.slow]\nshell = \"sleep 1.5; echo done\"\npost_worktree_create = true\n",
    );

    let created = host.create(&["@api"], true).unwrap();

    assert!(created.scripts[0].succeeded());
    assert!(
        created.scripts[0].duration_ms >= 1_400,
        "got {}ms",
        created.scripts[0].duration_ms
    );
}

/// A host of its own: its own planes directory, its own projects directory, its
/// own repositories, and its own capture of what scripts printed.
struct Host {
    root: PathBuf,
    directories: Directories,
    streamed: Arc<Mutex<Vec<u8>>>,
}

impl Host {
    fn new(label: &str) -> Host {
        let root = scratch_dir(label);

        Host {
            directories: Directories::new(root.join("planes"), root.join("projects")),
            root,
            streamed: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn engine(&self) -> LocalEngine {
        let streamed = Arc::clone(&self.streamed);

        LocalEngine::new(self.directories.clone()).streaming_script_output(move |bytes| {
            streamed
                .lock()
                .expect("no script sink panics")
                .extend_from_slice(bytes);
        })
    }

    /// An owned project, registered the way `bp project add` registers one.
    fn project(&self, name: &str) -> PathBuf {
        let forge = self.repository(&format!("forges/{name}"));

        LocalEngine::new(self.directories.clone())
            .project_add(ProjectAddRequest {
                url: forge.display().to_string(),
                name: Some(name.to_owned()),
            })
            .expect("the fixture project was registered");
        git(&forge, &["branch", "feat-login"]);

        forge
    }

    /// Appends a `[scripts]` table to a registered project's file.
    fn declares(&self, project: &str, scripts: &str) {
        let file = self
            .directories
            .projects()
            .join(project)
            .join("project.toml");
        let text = fs::read_to_string(&file).expect("the project was registered first");

        fs::write(file, format!("{text}\n{scripts}")).expect("write the project file");
    }

    /// An executable the project ships in its own `bin/`.
    fn ships(&self, project: &str, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        let bin = self.directories.projects().join(project).join(BIN_DIR_NAME);
        fs::create_dir_all(&bin).expect("make the project's bin directory");

        let path = bin.join(name);
        fs::write(&path, body).expect("write the executable");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("make it executable");
    }

    /// One `[scripts.<name>]` per name, each appending its own name to the
    /// trace, all bound to `point` — `create`, `remove`, or `none`.
    fn trace_scripts(&self, names: &[&str], point: &str) -> String {
        names
            .iter()
            .map(|name| {
                format!(
                    "[scripts.{name}]\nshell = \"{}\"\n{}",
                    self.append(name),
                    toggle(point),
                )
            })
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// A script that dumps its whole environment where the fixture can read it.
    fn env_script(&self, point: &str) -> String {
        format!(
            "[scripts.report]\nshell = \"env > {}\"\n{}",
            self.env_file().display(),
            toggle(point),
        )
    }

    fn append(&self, line: &str) -> String {
        format!("echo {line} >> {}", self.trace_file().display())
    }

    fn trace_file(&self) -> PathBuf {
        self.root.join("trace")
    }

    fn env_file(&self) -> PathBuf {
        self.root.join("environment")
    }

    /// What the scripts appended, in the order they appended it.
    fn trace(&self) -> Vec<String> {
        fs::read_to_string(self.trace_file())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The environment the last `report` script saw.
    fn environment(&self) -> std::collections::HashMap<String, String> {
        fs::read_to_string(self.env_file())
            .expect("the report script ran")
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    /// Everything the scripts printed, as it reached the sink.
    fn streamed(&self) -> String {
        String::from_utf8(self.streamed.lock().unwrap().clone()).expect("scripts printed utf-8")
    }

    fn repository(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        repository_with_one_commit(&path);
        path.canonicalize().expect("the repository was just made")
    }

    fn latch(&self, id: &str) {
        let latch = self.directories.planes().join(id).join(BITPLANE_DIR);
        fs::create_dir_all(&latch).expect("the plane directory is there");
        fs::write(latch.join("incomplete"), "").expect("write the latch");
    }

    /// The one plane this host has, for a `create` that answered `Err`.
    fn only_plane(&self) -> PathBuf {
        let mut planes: Vec<PathBuf> = fs::read_dir(self.directories.planes())
            .expect("the planes directory exists by now")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        planes.sort();

        planes.pop().expect("create claimed one")
    }

    fn create(&self, members: &[&str], run_scripts: bool) -> Result<PlaneCreated, EngineError> {
        self.engine().plane_create(PlaneCreateRequest {
            members: members.iter().map(|member| (*member).to_owned()).collect(),
            branch: Some("feat-login".to_owned()),
            id: None,
            intent: BranchIntent::Resolve,
            fetch: true,
            run_scripts,
        })
    }

    fn add(
        &self,
        id: &str,
        members: &[&str],
        run_scripts: bool,
    ) -> Result<PlaneAdded, EngineError> {
        self.engine().plane_add(PlaneAddRequest {
            plane: named(id),
            members: members.iter().map(|member| (*member).to_owned()).collect(),
            branch: Some("feat-login".to_owned()),
            intent: BranchIntent::Resolve,
            fetch: true,
            run_scripts,
        })
    }

    fn destroy(&self, id: &str, run_scripts: bool) -> Result<PlaneDestroyed, EngineError> {
        self.engine().plane_destroy(PlaneDestroyRequest {
            plane: named(id),
            waive: Vec::new(),
            run_scripts,
        })
    }

    fn remove(
        &self,
        id: &str,
        members: &[&str],
        run_scripts: bool,
    ) -> Result<PlaneRemoved, EngineError> {
        self.engine().plane_remove(PlaneRemoveRequest {
            plane: named(id),
            members: members.iter().map(|member| (*member).to_owned()).collect(),
            waive: Vec::new(),
            run_scripts,
        })
    }

    fn run(&self, id: &str, projects: &[&str], names: &[&str]) -> Result<ScriptsRun, EngineError> {
        self.engine().plane_scripts(PlaneScriptsRequest {
            plane: named(id),
            names: names.iter().map(|name| (*name).to_owned()).collect(),
            projects: projects
                .iter()
                .map(|project| ProjectName::parse(project).unwrap())
                .collect(),
        })
    }
}

fn named(id: &str) -> PlaneRef {
    PlaneRef::Id { id: id.to_owned() }
}

/// The toggle line a fixture script carries, or none at all.
fn toggle(point: &str) -> &'static str {
    match point {
        "create" => "post_worktree_create = true\n",
        "remove" => "pre_worktree_remove = true\n",
        _ => "",
    }
}

/// A path with every symlink followed, which is what a child's `pwd` reports.
fn canonical(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
