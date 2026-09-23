//! The world a documented example is run in, and the normalisation that makes
//! its output comparable with the page.
//!
//! A reference example is written as if you were the user in the docs: your
//! home is `/Users/alfonz`, your planes are in `~/planes`, and `@api` is a
//! project cloned from `git@gitlab.com:acme/api.git`. None of that can be true
//! in a test, so the world is built under a scratch directory and an **alias**
//! is recorded for each of those names. Arguments are translated on the way in
//! and output on the way back out, which is what lets the page's own bytes be
//! the expectation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub use bitplane_core::testing::git;
use bitplane_core::testing::{repository_with_an_origin, repository_with_one_commit, scratch_dir};

/// What one invocation produced.
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    /// stderr as it came, un-normalised. Normalisation rewrites the text
    /// bitplane quotes from git, which is right for a page and fatal for a line
    /// of JSON.
    pub raw_stderr: String,
}

/// The scratch host an example runs against.
pub struct World {
    root: PathBuf,
    /// The same directory with every symlink followed. On macOS the scratch
    /// root is under `/var`, which is a symlink to `/private/var`, so bitplane
    /// reports paths the fixture never spelled.
    canonical_root: PathBuf,
    home: PathBuf,
    /// Real text to the name the documentation gives it, longest real first.
    output: Vec<(String, String)>,
    /// The name the documentation gives it to real text, for arguments.
    input: Vec<(String, String)>,
    cwd: PathBuf,
    /// A directory put in front of `PATH`, where a shim stands in for git.
    shim: Option<PathBuf>,
}

impl World {
    /// A host of its own, with the directories the reference assumes.
    pub fn new(label: &str) -> World {
        let root = scratch_dir(label);
        let canonical_root = root.canonicalize().expect("the scratch root was just made");
        let home = root.join("home");
        fs::create_dir_all(&home).expect("create the fixture home");

        let mut world = World {
            root,
            canonical_root,
            home,
            output: Vec::new(),
            input: Vec::new(),
            cwd: PathBuf::new(),
            shim: None,
        };
        world.cwd = world.home.clone();

        // Longest first, so the planes directory is not rewritten as the home
        // directory it happens to sit inside.
        let planes = world.planes();
        let projects = world.projects();
        world.alias(&planes, "~/planes");
        world.alias(&projects, "~/.local/share/bitplane/projects");
        let home = world.home.clone();
        world.alias(&home, "/Users/alfonz");

        world
    }

    pub fn planes(&self) -> PathBuf {
        self.home.join("planes")
    }

    pub fn projects(&self) -> PathBuf {
        self.home.join(".local/share/bitplane/projects")
    }

    /// Where the next command runs from. The plane every read resolves comes
    /// from here, exactly as it does for a person at a shell.
    pub fn cd(&mut self, directory: impl AsRef<Path>) -> &mut World {
        self.cwd = directory.as_ref().to_path_buf();
        self
    }

    /// A repository standing in for the forge behind `url`, laid out so the
    /// worktree path bitplane derives is the one the page prints.
    ///
    /// `git@gitlab.com:acme/api.git` derives `acme/api`, so the stand-in lives
    /// at `<root>/forges/acme/api` and derives the same.
    pub fn forge(&mut self, url: &str) -> PathBuf {
        let path = self.inside(&self.root.join("forges").join(url_tail(url)));
        repository_with_one_commit(&path);
        // More than one tracked file, so a page that shows `3 modified` has
        // three files to modify.
        for file in ["a.md", "b.md", "c.md"] {
            fs::write(path.join(file), "fixture\n").expect("write a tracked file");
        }
        git(&path, &["add", "."]);
        git(&path, &["commit", "--quiet", "--message", "files"]);
        let path = path.canonicalize().expect("the forge was just made");

        self.alias(&path, url);
        path
    }

    /// A forge, registered as an owned project. Returns the project directory.
    pub fn project(&mut self, url: &str) -> PathBuf {
        self.project_with(url, &[])
    }

    /// The same, with `branches` already on the forge — which is the difference
    /// between a member the page prints as `created` and one it prints as
    /// `created (new branch)`.
    pub fn project_with(&mut self, url: &str, branches: &[&str]) -> PathBuf {
        let forge = self.forge(url);
        for branch in branches {
            git(&forge, &["branch", branch]);
        }

        self.register(&forge);
        self.projects()
            .join(url_tail(url).rsplit('/').next().expect("a last segment"))
    }

    /// Registers a forge as an owned project.
    pub fn register(&self, forge: &Path) {
        let run = self.run(&format!("bp project add {}", forge.display()));
        assert_eq!(
            run.code,
            0,
            "the fixture project at {} was not registered: {}",
            forge.display(),
            run.stderr
        );
    }

    /// An adopted project, written rather than adopted.
    ///
    /// `bp project adopt` is not built yet, and the read surface is: a project
    /// is exactly a directory holding a `project.toml`, so staging the file is
    /// staging the project. When adopt lands this becomes a call to it.
    pub fn adopted(&mut self, name: &str, checkout: &Path) {
        let directory = self.projects().join(name);
        fs::create_dir_all(&directory).expect("create the project directory");
        fs::write(
            directory.join("project.toml"),
            format!(
                "version = 1\nname = \"{name}\"\n\n[source]\ntype = \"adopted\"\npath = \"{}\"\n",
                checkout.display()
            ),
        )
        .expect("write the project file");
    }

    /// Appends a `[scripts]` table to a registered project's `project.toml`.
    ///
    /// Written rather than declared through a command, because there is no
    /// command that declares one: a script is there because the user typed it
    /// into their own data directory, and bitplane never writes one on their
    /// behalf. That is the whole of the shell-alias trust posture.
    pub fn declares(&self, project: &str, scripts: &str) {
        let file = self.projects().join(project).join("project.toml");
        let text = fs::read_to_string(&file).expect("the project was registered first");

        fs::write(file, format!("{text}\n{scripts}")).expect("write the project file");
    }

    /// The plane a generated id named, which is the only kind a page can show
    /// as `bp-a3f9c2e1`: `bp-` is reserved, so no fixture may choose one.
    pub fn anonymous_plane(&mut self, members: &[&str], branch: &str) -> PathBuf {
        let members = members.join(" ");
        let run = self.run(&format!("bp create {members} -b {branch}"));
        assert_eq!(
            run.code, 0,
            "the fixture plane was not created: {}\n{}",
            run.stderr, run.stdout
        );

        let mut made: Vec<PathBuf> = fs::read_dir(self.planes())
            .expect("the planes directory exists by now")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("bp-"))
            })
            .collect();
        made.sort();

        made.pop().expect("create just made one")
    }

    /// A plane, built the way a user builds one.
    pub fn plane(&mut self, id: &str, members: &[&str], branch: &str) -> PathBuf {
        let members = members.join(" ");
        let run = self.run(&format!("bp create {members} -b {branch} --id {id}"));
        assert_eq!(
            run.code, 0,
            "the fixture plane {id} was not created: {}\n{}",
            run.stderr, run.stdout
        );

        self.planes().join(id)
    }

    /// A plane directory `create` claimed, latched, and never put anything in.
    ///
    /// Built rather than created-and-latched because the page that shows it
    /// goes on to create a member on a branch, and a real half-built plane
    /// would already be holding that branch.
    pub fn latched_remnant(&self, id: &str) {
        let plane = self.planes().join(id);
        fs::create_dir_all(&plane).expect("claim the plane directory");
        fs::write(
            plane.join("plane.toml"),
            format!("version = 1\nid = \"{id}\"\n\n[members]\n"),
        )
        .expect("write the plane file");
        self.latch(id);
    }

    /// Prunes a worktree's record from an owned project's source repo.
    ///
    /// What an interrupted `destroy` leaves: the worktree gone and git's record
    /// with it, and `plane.toml` still listing the member. It is the only state
    /// a page can print as `already gone` — deleting the directory by hand
    /// leaves git's record, which is a finding rather than a removal.
    pub fn forget(&self, project: &str, worktree: &Path) {
        fs::remove_dir_all(worktree).expect("drop a worktree by hand");
        git(
            &self.projects().join(project).join("repo.git"),
            &["worktree", "prune"],
        );
    }

    /// Latches a plane: `create` claimed the directory and never finished.
    pub fn latch(&self, id: &str) {
        let latch = self.planes().join(id).join(".bitplane");
        fs::create_dir_all(&latch).expect("the plane directory is there");
        fs::write(latch.join("incomplete"), "").expect("write the latch");
    }

    /// Puts a shell script called `git` in front of `PATH`.
    ///
    /// `body` runs with git's own arguments in `"$@"`; falling through to
    /// `exec /usr/bin/env -- "$REAL_GIT" "$@"` is what every shim does for the
    /// invocations it is not interested in. A shim is how a documented failure
    /// that only git can produce — a version below the floor, a `worktree add`
    /// that refuses — is staged without waiting for one to happen.
    pub fn shim_git(&mut self, body: &str) {
        let directory = self.root.join("shim");
        fs::create_dir_all(&directory).expect("create the shim directory");

        let real = which_git();
        let script = format!("#!/bin/sh\nREAL_GIT={real}\n{body}\nexec \"$REAL_GIT\" \"$@\"\n");
        let path = directory.join("git");
        fs::write(&path, script).expect("write the shim");
        executable(&path);

        self.shim = Some(directory);
    }

    /// A repository of the user's own, at the path the documentation gives it,
    /// sitting on `main`.
    pub fn checkout(&mut self, documented: &str) -> PathBuf {
        let path = self.real(documented);
        let forge = self.inside(&self.root.join("forges/checkouts").join(name_of(&path)));

        // With an origin, because the `unpushed` check asks whether a tip is
        // contained in any `refs/remotes/origin/*`: a repository with no origin
        // at all has every branch unpushed, and every page that destroys an
        // ad-hoc member would be refused.
        repository_with_an_origin(&path, &forge);
        // A branch for a plane to land on that the checkout does not occupy.
        git(&path, &["branch", "spare"]);
        path.canonicalize().expect("the checkout was just made")
    }

    /// The same, sitting on `branch` instead — which is what frees `main` for a
    /// plane member to take. Git refuses `worktree add` on a branch any
    /// worktree of the repo already holds, so a page that shows an ad-hoc
    /// member on `main` is showing a checkout that has moved off it.
    pub fn checkout_on(&mut self, documented: &str, branch: &str) -> PathBuf {
        let path = self.checkout(documented);
        git(&path, &["checkout", "--quiet", "-b", branch]);
        path
    }

    /// Records that `real` is what the documentation calls `documented`.
    ///
    /// Both directions at once: output is read through it, and so is every
    /// path a case asks for by its documented name. A documented name with no
    /// alias would resolve to itself — `/Users/alfonz/projects/bitplane` is a
    /// real path on the machine running the suite — so the two tables are
    /// filled from one call rather than from two a case could get out of step.
    pub fn alias(&mut self, real: &Path, documented: &str) {
        let raw = real.display().to_string();
        let mut spellings = vec![raw.clone()];

        // Spelled rather than canonicalised: a fixture aliases directories
        // before they exist, and `canonicalize` cannot resolve those.
        if let Ok(tail) = real.strip_prefix(&self.root) {
            spellings.push(self.canonical_root.join(tail).display().to_string());
        }
        spellings.dedup();

        for spelling in &spellings {
            if !self.output.iter().any(|(real, _)| real == spelling) {
                self.output.push((spelling.clone(), documented.to_owned()));
            }
        }

        self.output
            .sort_by_key(|(real, _)| std::cmp::Reverse(real.len()));

        self.input
            .push((documented.to_owned(), spellings[0].clone()));
        self.input
            .sort_by_key(|(documented, _)| std::cmp::Reverse(documented.len()));
    }

    /// The real path behind a documented one, with its parents made.
    pub fn real(&self, documented: &str) -> PathBuf {
        let path = PathBuf::from(self.to_real(documented));
        let path = self.inside(&path);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("make the fixture's parents");
        }
        path
    }

    /// `path`, having checked that a fixture is about to write inside its own
    /// scratch directory and nowhere else.
    ///
    /// A missing alias is otherwise silent and catastrophic: the reference
    /// writes `~/projects/bitplane` and `git@gitlab.com:acme/api.git`, and the
    /// first of those is a path that exists on the machine running the suite.
    /// A fixture that reached it would `git init` over a real checkout.
    fn inside(&self, path: &Path) -> PathBuf {
        assert!(
            path.starts_with(&self.root),
            "the fixture would write to {}, which is outside its scratch \
             directory {} — the documented name it came from has no alias",
            path.display(),
            self.root.display(),
        );

        path.to_path_buf()
    }

    /// Runs a command written exactly as the page writes it.
    pub fn run(&self, command: &str) -> Run {
        self.run_with(command, &[])
    }

    /// The same command with more arguments — `--json`, for the rendering the
    /// page does not print.
    pub fn run_with(&self, command: &str, extra: &[&str]) -> Run {
        let mut words = command.split_whitespace();
        assert_eq!(words.next(), Some("bp"), "not a bp command: {command}");

        let arguments: Vec<String> = words.map(|word| self.to_real(word)).collect();
        let output = self.command(&arguments, extra).output().expect("run bp");

        Run {
            code: code(&output),
            stdout: self.normalise(&text(&output.stdout)),
            stderr: self.normalise(&text(&output.stderr)),
            raw_stderr: text(&output.stderr),
        }
    }

    /// Everything a page's expectation has to be read through too: the page is
    /// normalised by the same function as the run, so a rule that would hide a
    /// difference has to hide it on both sides.
    pub fn normalise(&self, text: &str) -> String {
        let mut text = text.to_owned();

        for (real, documented) in &self.output {
            text = text.replace(real.as_str(), documented);
        }

        let text = generated_ids(&text);
        let text = stamps(&text);
        let text = detached_heads(&text);
        let text = durations(&text);
        let text = ages(&text);
        git_diagnostics(&text)
    }

    fn command(&self, arguments: &[String], extra: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bp"));
        command
            .args(arguments)
            .args(extra)
            .arg("--planes-dir")
            .arg(self.planes())
            .arg("--projects-dir")
            .arg(self.projects())
            .current_dir(&self.cwd)
            // The world is the whole world: a real config file, a real
            // `XDG_DATA_HOME` or a real home would all reach into the run.
            .env("HOME", &self.home)
            .env("BITPLANE_CONFIG", self.root.join("no-such-config.toml"))
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("XDG_DATA_HOME", self.home.join(".local/share"))
            .env_remove("BITPLANE_PLANES_DIR")
            .env_remove("BITPLANE_PROJECTS_DIR");

        if let Some(shim) = &self.shim {
            let path = std::env::var("PATH").unwrap_or_default();
            command.env("PATH", format!("{}:{path}", shim.display()));
        }

        command
    }

    fn to_real(&self, word: &str) -> String {
        let mut word = word.to_owned();
        for (documented, real) in &self.input {
            word = word.replace(documented.as_str(), real);
        }
        word
    }
}

/// Whether two renderings of the same text say the same thing.
///
/// Column widths are **not** compared. Every fan-out pads to its widest cell,
/// so a member called `/private/var/folders/…/projects/bitplane` in the fixture
/// and `/Users/alfonz/projects/bitplane` on the page align their neighbours
/// differently while saying the same thing. The alignment itself is asserted
/// byte for byte by the hand-written tests beside this one; what a reference
/// example is for is the words.
pub fn same(actual: &str, expected: &str) -> Result<(), String> {
    let actual: Vec<String> = actual.lines().map(canonical).collect();
    let wanted: Vec<String> = expected.lines().map(canonical).collect();

    if actual == wanted {
        return Ok(());
    }

    let mut report = String::from("the page and the binary disagree\n");
    for row in 0..actual.len().max(wanted.len()) {
        let left = wanted.get(row).map(String::as_str).unwrap_or("");
        let right = actual.get(row).map(String::as_str).unwrap_or("");
        let mark = if left == right { "  " } else { "->" };
        report.push_str(&format!("{mark} page: {left}\n{mark}  bp : {right}\n"));
    }

    Err(report)
}

/// One line as its cells, with the gutter widths gone and the fact of being
/// indented kept.
fn canonical(line: &str) -> String {
    let cells = cells(line);
    if cells.is_empty() {
        return String::new();
    }

    let indent = match line.starts_with(' ') {
        true => "  ",
        false => "",
    };
    format!("{indent}{}", cells.join("  "))
}

/// A row split on its gutters: two or more spaces separate columns, one space
/// is part of a cell.
pub fn cells(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut spaces = 0usize;

    for character in line.trim().chars() {
        if character == ' ' {
            spaces += 1;
            continue;
        }

        if spaces >= 2 && !current.is_empty() {
            cells.push(std::mem::take(&mut current));
        } else if spaces == 1 {
            current.push(' ');
        }

        current.push(character);
        spaces = 0;
    }

    if !current.is_empty() {
        cells.push(current);
    }

    cells
}

/// `bp-a3f9c2e1` and every other generated id, which is random by design.
fn generated_ids(text: &str) -> String {
    replace_runs(
        text,
        "bp-",
        8,
        8,
        |character| character.is_ascii_hexdigit(),
        |_| "bp-xxxxxxxx".to_owned(),
    )
}

/// A date, and the minute after one.
fn stamps(text: &str) -> String {
    let dated = replace_shaped(text, "dddd-dd-dd", "<date>");
    replace_shaped(&dated, "dd:dd", "<time>")
}

/// The short object name a detached `HEAD` is reported at.
fn detached_heads(text: &str) -> String {
    replace_runs(
        text,
        "detached at ",
        4,
        40,
        |character| character.is_ascii_hexdigit(),
        |_| "detached at <sha>".to_owned(),
    )
}

/// How long a script took.
fn durations(text: &str) -> String {
    replace_shaped(text, "d.ds", "<duration>")
}

/// How long ago something happened.
fn ages(text: &str) -> String {
    let mut text = text.replace("less than a minute ago", "<age> ago");

    for unit in ["seconds", "minutes", "hours", "days", "weeks", "months"] {
        let mut out = String::new();
        let mut rest = text.as_str();

        while let Some(at) = rest.find(&format!(" {unit} ago")) {
            let head = &rest[..at];
            let digits = head.len() - head.trim_end_matches(|c: char| c.is_ascii_digit()).len();
            out.push_str(&head[..head.len() - digits]);
            out.push_str("<age> ago");
            rest = &rest[at + format!(" {unit} ago").len()..];
        }

        out.push_str(rest);
        text = out;
    }

    text
}

/// Replaces every `prefix` followed by between `least` and `most` characters
/// `is_body` accepts.
fn replace_runs(
    text: &str,
    prefix: &str,
    least: usize,
    most: usize,
    is_body: impl Fn(char) -> bool,
    with: impl Fn(&str) -> String,
) -> String {
    let mut out = String::new();
    let mut rest = text;

    while let Some(at) = rest.find(prefix) {
        out.push_str(&rest[..at]);
        let after = &rest[at + prefix.len()..];
        let run: String = after.chars().take_while(|c| is_body(*c)).collect();

        match run.len() >= least && run.len() <= most {
            true => {
                out.push_str(&with(&run));
                rest = &after[run.len()..];
            }
            false => {
                out.push_str(prefix);
                rest = after;
            }
        }
    }

    out.push_str(rest);
    out
}

/// Replaces every run matching a shape, where `d` is a digit and every other
/// character stands for itself.
fn replace_shaped(text: &str, shape: &str, with: &str) -> String {
    let shape: Vec<char> = shape.chars().collect();
    let characters: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut at = 0;

    while at < characters.len() {
        let matched = at + shape.len() <= characters.len()
            && shape.iter().enumerate().all(|(offset, wanted)| {
                let found = characters[at + offset];
                match wanted {
                    'd' => found.is_ascii_digit(),
                    _ => found == *wanted,
                }
            });

        match matched {
            true => {
                out.push_str(with);
                at += shape.len();
            }
            false => {
                out.push(characters[at]);
                at += 1;
            }
        }
    }

    out
}

/// What git said, where bitplane is quoting it.
///
/// The reference documents that `git_failed` and a failed fetch carry git's
/// own diagnostic, which makes the wording git's to change and not this
/// project's to assert. What bitplane itself chose — the subcommand, and the
/// clause that introduces the quote — is left alone.
fn git_diagnostics(text: &str) -> String {
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            for marker in [" exited ", "could not fetch"] {
                if let Some(at) = line.find(marker)
                    && let Some(colon) = line[at..].find(": ")
                {
                    return format!("{}: <git says>", &line[..at + colon]);
                }
            }
            line.to_owned()
        })
        .collect();

    match text.ends_with('\n') && !lines.is_empty() {
        true => format!("{}\n", lines.join("\n")),
        false => lines.join("\n"),
    }
}

/// The real git, found before a shim is put in front of it.
fn which_git() -> String {
    let found = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("look for git");

    text(&found.stdout).trim().to_owned()
}

fn executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).expect("the shim is there").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make the shim executable");
}

/// The last two segments of a git URL's path, without its transport or `.git`.
fn url_tail(url: &str) -> String {
    let path = match url.split_once("://") {
        Some((_, after)) => after.split_once('/').map(|(_, path)| path).unwrap_or(""),
        None => match (url.find(':'), url.find('/')) {
            (Some(colon), None) => &url[colon + 1..],
            (Some(colon), Some(slash)) if colon < slash => &url[colon + 1..],
            _ => url,
        },
    };

    let path = path.trim_end_matches('/');

    path.strip_suffix(".git").unwrap_or(path).to_owned()
}

/// A path's last segment, for naming a directory after it.
fn name_of(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo")
        .to_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("bp was not killed by a signal")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("bp printed utf-8")
}
