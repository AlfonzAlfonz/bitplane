//! `bp project add`, `list` and `fetch`, end to end through the binary.
//!
//! What the core tests cannot check: the rows a person reads, the same run's
//! machine rendering, and the exit codes — including `3`, which is the one code
//! that means the command **worked** and there is something to look at.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bitplane_core::testing::{git, repository_with_one_commit, scratch_dir};

#[test]
fn a_registered_project_prints_its_kind_and_where_to_find_it() {
    let host = Host::new("cli-project-add");
    let forge = host.forge("codestyle");

    let run = host.bp(&["project", "add", &forge.display().to_string()]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        stdout(&run),
        format!(
            concat!(
                "@codestyle  owned\n",
                "  source     {forge}\n",
                "  directory  {projects}/codestyle\n",
                "  default    main\n",
            ),
            forge = forge.display(),
            projects = host.projects().display(),
        )
    );
    assert_eq!(stderr(&run), "");
}

#[test]
fn the_same_run_under_json_puts_the_response_on_stdout_and_nothing_on_stderr() {
    let host = Host::new("cli-project-add-json");
    let forge = host.forge("codestyle");

    let run = host.bp(&["project", "add", &forge.display().to_string(), "--json"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(stderr(&run), "");

    let response: serde_json::Value = serde_json::from_str(&stdout(&run)).unwrap();
    assert_eq!(response["action"], "project_add");
    assert_eq!(response["name"], "codestyle");
    assert_eq!(response["source"]["type"], "owned");
    assert_eq!(response["default_branch"], "main");
}

#[test]
fn a_taken_default_name_exits_2_with_the_suggestion_in_the_remedy() {
    let host = Host::new("cli-project-add-taken");
    host.bp(&[
        "project",
        "add",
        &host.forge_at("acme/codestyle").display().to_string(),
    ]);

    let run = host.bp(&[
        "project",
        "add",
        &host.forge_at("other/codestyle").display().to_string(),
    ]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "", "stdout carries results only");
    assert_eq!(
        stderr(&run),
        concat!(
            "error[project_name_taken]: codestyle is already a project\n",
            "\n",
            "remedy: other-codestyle is free; re-run with --name other-codestyle.\n",
        )
    );
}

#[test]
fn a_fetch_that_never_reached_the_forge_exits_1_naming_the_kept_objects() {
    let host = Host::new("cli-project-add-aborted");
    let forge = host.root.join("forges").join("codestyle");
    fs::create_dir_all(&forge).unwrap();

    let run = host.bp(&["project", "add", &forge.display().to_string()]);

    assert_eq!(run.status.code(), Some(1));
    let reported = stderr(&run);
    assert!(
        reported.starts_with("error[project_add_aborted]: @codestyle was not registered\n"),
        "got:\n{reported}"
    );
    assert!(
        reported.contains(&format!(
            "remedy: The objects fetched so far were kept at {}/codestyle/repo.git; \
             re-running bp project add will reuse them.\n",
            host.projects().display()
        )),
        "got:\n{reported}"
    );
}

#[test]
fn an_empty_registry_lists_nothing_and_exits_0() {
    let host = Host::new("cli-project-list-empty");

    let run = host.bp(&["project", "list"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(stdout(&run), "");
    assert_eq!(stderr(&run), "");
}

#[test]
fn a_listing_prints_one_row_per_project() {
    let host = Host::new("cli-project-list");
    let api = host.forge("api");
    let codestyle = host.forge("codestyle");
    host.bp(&["project", "add", &api.display().to_string()]);
    host.bp(&["project", "add", &codestyle.display().to_string()]);

    let run = host.bp(&["project", "list"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        stdout(&run),
        format!(
            "  @api        owned  {api}\n  @codestyle  owned  {codestyle}\n",
            api = api.display(),
            codestyle = codestyle.display(),
        )
    );
}

#[test]
fn a_project_file_that_will_not_parse_is_a_row_and_the_listing_exits_3() {
    let host = Host::new("cli-project-list-drift");
    host.bp(&["project", "add", &host.forge("api").display().to_string()]);
    let broken = host.projects().join("codestyle");
    fs::create_dir_all(&broken).unwrap();
    fs::write(
        broken.join("project.toml"),
        "version = 1\nname = \"codestyle\"\ndefault_branch = \"main\"\n",
    )
    .unwrap();

    let run = host.bp(&["project", "list"]);

    assert_eq!(
        run.status.code(),
        Some(3),
        "the command worked and there is a finding; stderr: {}",
        stderr(&run)
    );
    let listed = stdout(&run);
    assert!(listed.contains("  @api        owned"), "got:\n{listed}");
    assert!(
        listed.contains("@codestyle  ") && listed.contains("unknown key \"default_branch\""),
        "got:\n{listed}"
    );
    assert_eq!(
        stderr(&run),
        "",
        "drift is a state bp reports, not an error: the result is on stdout \
         and only the exit code says to look at it"
    );
}

#[test]
fn a_fetch_prints_one_row_per_project() {
    let host = Host::new("cli-project-fetch");
    let forge = host.forge("codestyle");
    host.bp(&["project", "add", &forge.display().to_string()]);
    commit(&forge, "more");

    let run = host.bp(&["project", "fetch", "@codestyle"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(stdout(&run), "  @codestyle  fetched  1 ref updated\n");
    assert_eq!(stderr(&run), "");

    let again = host.bp(&["project", "fetch"]);
    assert_eq!(stdout(&again), "  @codestyle  fetched  up to date\n");
}

#[test]
fn a_forge_that_will_not_answer_is_a_row_on_stdout_and_a_failure_on_stderr() {
    let host = Host::new("cli-project-fetch-failed");
    let api = host.forge("api");
    let codestyle = host.forge("codestyle");
    host.bp(&["project", "add", &api.display().to_string()]);
    host.bp(&["project", "add", &codestyle.display().to_string()]);
    fs::remove_dir_all(&codestyle).unwrap();

    let run = host.bp(&["project", "fetch"]);

    assert_eq!(run.status.code(), Some(1));
    let rows = stdout(&run);
    assert!(
        rows.starts_with("  @api        fetched  up to date\n"),
        "got:\n{rows}"
    );
    assert!(rows.contains("  @codestyle  failed   "), "got:\n{rows}");
    assert!(
        stderr(&run)
            .starts_with("error[fetch_failed]: 1 of 2 fetchable projects could not be fetched\n"),
        "got:\n{}",
        stderr(&run)
    );
    assert!(
        stderr(&run).contains("  @codestyle  "),
        "got:\n{}",
        stderr(&run)
    );
}

#[test]
fn no_such_project_exits_2_pointing_at_the_listing() {
    let host = Host::new("cli-project-fetch-unknown");

    let run = host.bp(&["project", "fetch", "@codestyle"]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "");
    assert_eq!(
        stderr(&run),
        concat!(
            "error[project_not_found]: there is no project called codestyle\n",
            "\n",
            "remedy: Run bp project list to see what projects exist.\n",
        )
    );
}

#[test]
fn the_project_verbs_name_every_flag_the_reference_documents() {
    let host = Host::new("cli-project-help");

    let add = stdout(&host.bp(&["project", "add", "--help"]));
    assert!(add.contains("--name"), "got:\n{add}");

    // Clap spells an optional positional `[project]...`; the reference writes
    // it `<project>...`. The slot is what matters, not the brackets.
    let fetch = stdout(&host.bp(&["project", "fetch", "--help"]));
    assert!(fetch.contains("[project]..."), "got:\n{fetch}");

    let project = stdout(&host.bp(&["project", "--help"]));
    for verb in ["add", "list", "fetch"] {
        assert!(
            project.contains(verb),
            "{verb} is undocumented in:\n{project}"
        );
    }
}

fn commit(repository: &Path, message: &str) {
    fs::write(repository.join(message), message).unwrap();
    git(repository, &["add", "."]);
    git(repository, &["commit", "--quiet", "--message", message]);
}

fn stdout(run: &Output) -> String {
    String::from_utf8(run.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(run: &Output) -> String {
    String::from_utf8(run.stderr.clone()).expect("stderr is utf-8")
}

/// A host of its own, so no test can see another's projects.
struct Host {
    root: PathBuf,
}

impl Host {
    fn new(label: &str) -> Host {
        Host {
            root: scratch_dir(label),
        }
    }

    /// A repository standing in for a forge. A path is a git URL.
    fn forge(&self, name: &str) -> PathBuf {
        self.forge_at(&format!("forges/{name}"))
    }

    fn forge_at(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        repository_with_one_commit(&path);
        path.canonicalize().expect("the forge was just made")
    }

    fn projects(&self) -> PathBuf {
        self.root.join("projects")
    }

    fn bp(&self, arguments: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bp"));
        command
            .args(arguments)
            .arg("--planes-dir")
            .arg(self.root.join("planes"))
            .arg("--projects-dir")
            .arg(self.projects());
        sealed_off(&mut command, &self.root);

        command.output().expect("run bp")
    }
}

/// Cuts the run off from the developer's own environment: a real
/// `~/.config/bitplane/config.toml` would otherwise be read whatever the flags
/// say, and one key bitplane does not know would fail every test here with an
/// unrelated `parse_error`.
fn sealed_off(command: &mut Command, root: &Path) {
    command
        .env("BITPLANE_CONFIG", root.join("no-such-config.toml"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env_remove("BITPLANE_PLANES_DIR")
        .env_remove("BITPLANE_PROJECTS_DIR");
}
