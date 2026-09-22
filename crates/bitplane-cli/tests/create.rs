//! `bp create`, end to end through the binary.
//!
//! What the core tests cannot check: the rows a person reads, the same run's
//! machine rendering, and the exit codes — including `130`, which only exists
//! once there is a process to signal.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use bitplane_core::testing::{git, repository_with_one_commit, scratch_dir};

#[test]
fn a_plane_prints_a_header_and_one_row_per_member() {
    let host = Host::new("cli-create");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");

    let run = host.bp(&[
        "create",
        &alpha.display().to_string(),
        &beta.display().to_string(),
        "-b",
        "feat-login",
        "--id",
        "auth-work",
    ]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        stdout(&run),
        format!(
            concat!(
                "auth-work  {planes}/auth-work\n",
                "\n",
                "  {alpha}  feat-login  created (new branch)  repos/alpha\n",
                "  {beta}   feat-login  created (new branch)  repos/beta\n",
            ),
            planes = host.planes().display(),
            alpha = alpha.display(),
            beta = beta.display(),
        )
    );
    assert_eq!(stderr(&run), "");
}

#[test]
fn the_same_run_under_json_puts_the_response_on_stdout_and_nothing_on_stderr() {
    let host = Host::new("cli-create-json");
    let alpha = host.repository("alpha");

    let run = host.bp(&[
        "create",
        &alpha.display().to_string(),
        "-b",
        "feat-login",
        "--json",
    ]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(stderr(&run), "");

    let response: serde_json::Value = serde_json::from_str(&stdout(&run)).unwrap();
    assert_eq!(response["action"], "plane_create");
    assert_eq!(response["members"][0]["outcome"], "ok");
    assert_eq!(response["members"][0]["detail"]["branch"], "feat-login");
}

#[test]
fn a_taken_id_exits_2_with_a_remedy_naming_what_is_there() {
    let host = Host::new("cli-create-taken");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");

    host.bp(&[
        "create",
        &alpha.display().to_string(),
        "-b",
        "feat-x",
        "--id",
        "auth-work",
    ]);
    let run = host.bp(&[
        "create",
        &beta.display().to_string(),
        "-b",
        "feat-y",
        "--id",
        "auth-work",
    ]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "", "stdout carries results only");
    assert_eq!(
        stderr(&run),
        concat!(
            "error[plane_id_in_use]: auth-work is already a plane\n",
            "\n",
            "remedy: Choose another id, or destroy the existing plane first.\n",
        )
    );
}

#[test]
fn an_occupied_branch_exits_1_and_says_so_in_bitplanes_own_words() {
    let host = Host::new("cli-create-occupied");
    let alpha = host.repository("alpha");

    let run = host.bp(&["create", &alpha.display().to_string(), "-b", "main"]);

    assert_eq!(run.status.code(), Some(1));
    assert_eq!(
        stderr(&run),
        format!(
            concat!(
                "error[branch_occupied]: main is already checked out in {alpha}\n",
                "\n",
                "  {alpha}  its working tree is on main\n",
                "\n",
                "remedy: Check out a different branch there, or give this member a branch \
                 no worktree holds.\n",
            ),
            alpha = alpha.display()
        )
    );
}

#[test]
fn ctrl_c_unwinds_prints_the_partial_vector_and_exits_130() {
    let host = Host::new("cli-create-interrupt");

    // Six members, four workers, and a checkout that takes its time: the
    // interrupt lands while the first batch is in flight and the rest have not
    // been scheduled.
    let members: Vec<String> = (0..6)
        .map(|index| {
            let repository = host.repository(&format!("slow{index}"));
            dawdle_on_checkout(&repository);
            repository.display().to_string()
        })
        .collect();

    let mut arguments: Vec<&str> = vec!["create"];
    arguments.extend(members.iter().map(String::as_str));
    arguments.extend(["-b", "feat-login", "--id", "auth-work"]);

    let running = host.spawn(&arguments);
    // Wait until a worktree is genuinely in flight, so the interrupt lands
    // mid-batch rather than before the fan-out started.
    wait_for(|| {
        fs::read_dir(host.planes().join("auth-work").join("repos"))
            .is_ok_and(|entries| entries.count() > 0)
    });
    interrupt(&running);

    let run = running.wait_with_output().expect("bp finished");

    assert_eq!(run.status.code(), Some(130), "stderr: {}", stderr(&run));

    let printed = stdout(&run);
    assert!(printed.starts_with("auth-work  "), "got:\n{printed}");
    assert_eq!(
        printed
            .lines()
            .filter(|line| line.starts_with("  "))
            .count(),
        6,
        "the partial vector is the repair instruction:\n{printed}"
    );
    assert!(
        printed.contains("skipped: interrupted"),
        "scheduling must stop:\n{printed}"
    );
    assert!(
        printed.contains("removed"),
        "the children already running must be waited for, then unwound:\n{printed}"
    );
    assert!(
        !host.planes().join("auth-work").exists(),
        "the abort path runs on an interrupt too"
    );
}

#[test]
fn a_second_ctrl_c_exits_immediately_and_leaves_what_was_in_flight_where_it_fell() {
    let host = Host::new("cli-create-interrupt-twice");
    let members: Vec<String> = (0..4)
        .map(|index| {
            let repository = host.repository(&format!("slow{index}"));
            dawdle_on_checkout(&repository);
            repository.display().to_string()
        })
        .collect();

    let mut arguments: Vec<&str> = vec!["create"];
    arguments.extend(members.iter().map(String::as_str));
    arguments.extend(["-b", "feat-login", "--id", "auth-work"]);

    let running = host.spawn(&arguments);
    wait_for(|| {
        fs::read_dir(host.planes().join("auth-work").join("repos"))
            .is_ok_and(|entries| entries.count() > 0)
    });

    interrupt(&running);
    std::thread::sleep(Duration::from_millis(100));
    let asked_twice = Instant::now();
    interrupt(&running);

    let run = running.wait_with_output().expect("bp finished");

    assert_eq!(run.status.code(), Some(130), "stderr: {}", stderr(&run));
    assert!(
        asked_twice.elapsed() < CHECKOUT_DAWDLE,
        "the second Ctrl-C must not wait for the children the first one is waiting for"
    );
    assert_eq!(stdout(&run), "", "there was no time to print anything");
    assert!(
        host.planes().join("auth-work").exists(),
        "whatever was in flight is left where it fell"
    );
}

/// How long the `post-checkout` hook takes, and therefore how long the first
/// interrupt spends waiting for the children already running.
const CHECKOUT_DAWDLE: Duration = Duration::from_secs(2);

/// Makes `git worktree add` slow, deterministically, without a sleep in the
/// test itself: git runs `post-checkout` in the new worktree.
fn dawdle_on_checkout(repository: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let hooks = repository.join(".git").join("hooks");
    fs::create_dir_all(&hooks).unwrap();

    let hook = hooks.join("post-checkout");
    fs::write(
        &hook,
        format!("#!/bin/sh\nsleep {}\n", CHECKOUT_DAWDLE.as_secs()),
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
}

/// SIGINT to `bp` alone. The real Ctrl-C reaches the whole process group and
/// the children too, which is precisely why bitplane cannot protect them —
/// signalling only the parent is the stricter test of what bitplane controls.
fn interrupt(running: &Child) {
    // SAFETY: `kill(2)` against a pid this test owns.
    unsafe {
        libc::kill(running.id() as libc::pid_t, libc::SIGINT);
    }
}

fn wait_for(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);

    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    panic!("bp never got as far as claiming the plane");
}

/// Cuts the run off from the developer's own environment.
///
/// `--planes-dir` and `--projects-dir` are not enough on their own: the config
/// file is resolved and read whatever the flags say, so a real
/// `~/.config/bitplane/config.toml` with a key bitplane does not know would
/// fail every test here with an unrelated `parse_error`.
fn sealed_off(command: &mut Command, root: &Path) {
    command
        .env("BITPLANE_CONFIG", root.join("no-such-config.toml"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env_remove("BITPLANE_PLANES_DIR")
        .env_remove("BITPLANE_PROJECTS_DIR");
}

fn stdout(run: &Output) -> String {
    String::from_utf8(run.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(run: &Output) -> String {
    String::from_utf8(run.stderr.clone()).expect("stderr is utf-8")
}

/// A host of its own, so no test can see another's planes.
struct Host {
    root: PathBuf,
}

impl Host {
    fn new(label: &str) -> Host {
        Host {
            root: scratch_dir(label),
        }
    }

    fn repository(&self, name: &str) -> PathBuf {
        let path = self.root.join("repos").join(name);
        repository_with_one_commit(&path);
        // A branch for a plane to land on that the checkout does not occupy.
        git(&path, &["branch", "spare"]);
        path.canonicalize().expect("the repository was just made")
    }

    fn planes(&self) -> PathBuf {
        self.root.join("planes")
    }

    fn bp(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().expect("run bp")
    }

    fn spawn(&self, arguments: &[&str]) -> Child {
        self.command(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn bp")
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bp"));
        command
            .args(arguments)
            .arg("--planes-dir")
            .arg(self.planes())
            .arg("--projects-dir")
            .arg(self.root.join("projects"));
        sealed_off(&mut command, &self.root);
        command
    }
}
