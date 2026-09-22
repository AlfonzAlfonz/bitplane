//! The published reference, read as data.
//!
//! The pages under `website/docs/reference/` are the specification, so the
//! snapshot suite takes its expectations **out of them** rather than out of a
//! parallel set of golden files. A golden file is a second place to edit, and a
//! second place to edit is where a documentation suite starts agreeing with
//! itself instead of with the docs.
//!
//! An **example** is a fenced block whose first line starts with `$ bp `,
//! followed by the block or two that hold what it printed. That is the shape
//! every page already uses; nothing was reformatted to make this parser work.

use std::fs;
use std::path::{Path, PathBuf};

/// What a page's own admonition says about how much of it is real.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Implemented,
    InProgress,
    NotImplemented,
    /// A shared page — exit codes, global flags — that documents no one command.
    Unstated,
}

/// One reference page.
#[derive(Debug, Clone)]
pub struct Page {
    /// Relative to the reference root: `plane/list.md`.
    pub path: String,
    pub status: Status,
    /// The synopsis line under the `# ` heading: `bp list [--health …]`.
    pub synopsis: String,
    /// Every flag the `## Flags` table names, `-p` and `--plane` alike.
    pub flags: Vec<String>,
    /// The flags the page's own admonition says are not accepted yet.
    ///
    /// An `:::in-progress` page names what is missing in its body, in one
    /// shape: **`--no-scripts` is not accepted.** Reading it here is what lets
    /// the suite hold the binary to everything the page does not except — and
    /// notice when an exception stops being true.
    pub not_accepted: Vec<String>,
    pub examples: Vec<Example>,
}

/// One runnable example: a command, and what the page says it printed.
#[derive(Debug, Clone)]
pub struct Example {
    /// The heading it sits under, without its `#`s or backticks.
    pub section: String,
    /// Which example it is within that section, from 1.
    pub nth: usize,
    /// The command with its `$ ` gone: `bp list --health full`.
    pub command: String,
    /// Whatever else the command block held — a `^C`, a line of live stderr.
    pub annotations: Vec<String>,
    pub stdout: String,
    pub stderr: String,
    /// From the nearest `Exit `N`` in the prose above it; `0` where none says.
    pub exit: i32,
}

impl Page {
    /// The example a case names, by the label it prints under.
    pub fn example(&self, label: &str) -> Option<&Example> {
        self.examples
            .iter()
            .find(|example| example.label() == label)
    }
}

impl Example {
    /// How the case registry and a failure message name it.
    pub fn label(&self) -> String {
        match self.nth {
            1 => self.section.clone(),
            nth => format!("{} #{nth}", self.section),
        }
    }
}

/// Every reference page, in path order.
pub fn pages() -> Vec<Page> {
    let root = reference_root();
    let mut paths = Vec::new();
    collect(&root, &root, &mut paths);
    paths.sort();

    paths
        .iter()
        .map(|relative| read(&root.join(relative), relative))
        .collect()
}

/// The page at `relative`, which must exist — a case naming a page that was
/// deleted is a broken case, not an empty one.
pub fn page(relative: &str) -> Page {
    let path = reference_root().join(relative);
    assert!(path.is_file(), "no reference page at {}", path.display());
    read(&path, relative)
}

/// Where the published reference lives.
pub fn reference_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/docs/reference")
        .canonicalize()
        .expect("the reference is part of the repository")
}

fn collect(root: &Path, directory: &Path, into: &mut Vec<String>) {
    let entries = fs::read_dir(directory).expect("read the reference directory");

    for entry in entries {
        let path = entry.expect("read a reference entry").path();

        if path.is_dir() {
            collect(root, &path, into);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            let relative = path.strip_prefix(root).expect("under the root");
            into.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn read(path: &Path, relative: &str) -> Page {
    let text = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {relative}: {err}"));

    let mut page = Page {
        path: relative.to_owned(),
        status: Status::Unstated,
        synopsis: String::new(),
        flags: Vec::new(),
        not_accepted: Vec::new(),
        examples: Vec::new(),
    };

    let mut section = String::from("(top)");
    let mut counted = 0usize;
    let mut prose = String::new();
    // What the enclosing `##` said, so a `###` that states no code of its own
    // inherits one rather than defaulting to success.
    let mut baseline = 0;
    let mut at_top = true;
    let mut pending: Option<(String, Vec<String>, i32)> = None;
    let mut fence: Option<Fence> = None;

    for line in text.lines() {
        if let Some(open) = fence.as_mut() {
            if line == "```" {
                let Fence { language, body } = fence.take().expect("just borrowed");
                let exit = exit_in(&prose).unwrap_or(baseline);
                close(
                    &mut page,
                    &mut pending,
                    &mut counted,
                    &section,
                    &language,
                    &body,
                    exit,
                );
            } else {
                open.body.push(line.to_owned());
            }
            continue;
        }

        if let Some(language) = line.strip_prefix("```") {
            fence = Some(Fence {
                language: language.to_owned(),
                body: Vec::new(),
            });
            continue;
        }

        if let Some(heading) = line.strip_prefix("### ") {
            section = heading.replace('`', "");
            counted = 0;
            at_top = false;
            prose.clear();
            pending = None;
            continue;
        }

        if let Some(heading) = line.strip_prefix("## ") {
            section = heading.replace('`', "");
            counted = 0;
            at_top = true;
            baseline = 0;
            prose.clear();
            pending = None;
            continue;
        }

        if page.status == Status::Unstated {
            page.status = match line {
                ":::implemented" => Status::Implemented,
                ":::in-progress" => Status::InProgress,
                ":::not-implemented" => Status::NotImplemented,
                _ => Status::Unstated,
            };
        }

        prose.push_str(line);
        prose.push('\n');

        if line.starts_with("| `-") {
            page.flags.extend(flags_in(line));
        }

        if let Some(flag) = line
            .strip_prefix("- **`")
            .and_then(|rest| rest.split_once("` is not accepted.**"))
            .map(|(flag, _)| flag)
        {
            page.not_accepted.push(flag.to_owned());
        }

        if at_top && let Some(code) = exit_in(&prose) {
            baseline = code;
        }
    }

    let mut seen = Vec::new();
    page.flags.retain(|flag| match seen.contains(flag) {
        true => false,
        false => {
            seen.push(flag.clone());
            true
        }
    });
    page
}

/// A fenced block being read.
struct Fence {
    language: String,
    body: Vec<String>,
}

/// The state a closed fence changes: a command block arms the next output
/// block, an output block completes the example, and an error block that
/// follows one already completed is that same run's stderr.
fn close(
    page: &mut Page,
    pending: &mut Option<(String, Vec<String>, i32)>,
    counted: &mut usize,
    section: &str,
    language: &str,
    body: &[String],
    exit: i32,
) {
    if !language.is_empty() {
        return;
    }

    let first = body.first().map(String::as_str).unwrap_or_default();

    if page.synopsis.is_empty() && first.starts_with("bp ") && section == "(top)" {
        page.synopsis = body.join("\n");
    }

    if let Some(command) = first.strip_prefix("$ ") {
        *pending = Some((command.to_owned(), body[1..].to_vec(), exit));
        return;
    }

    // An example that printed nothing is still an example: `bp list` with no
    // planes says so with an empty block.
    let text = match body.is_empty() {
        true => String::new(),
        false => format!("{}\n", body.join("\n")),
    };

    if let Some((command, annotations, exit)) = pending.take() {
        *counted += 1;
        let (stdout, stderr) = match text.starts_with("error[") {
            true => (String::new(), text),
            false => (text, String::new()),
        };

        page.examples.push(Example {
            section: section.to_owned(),
            nth: *counted,
            command,
            annotations,
            stdout,
            stderr,
            exit,
        });
        return;
    }

    // A second block, holding the same run's envelope: `bp project fetch`
    // prints its rows on stdout and then fails on stderr.
    if text.starts_with("error[")
        && let Some(last) = page.examples.last_mut()
        && last.section == section
        && last.stderr.is_empty()
    {
        last.stderr = text;
    }
}

/// The last exit code the prose since the heading states, however it spells it.
fn exit_in(prose: &str) -> Option<i32> {
    let mut found = None;

    for marker in ["Exit `", "Exit code is `"] {
        let mut rest = prose;

        while let Some(at) = rest.find(marker) {
            rest = &rest[at + marker.len()..];
            if let Some(end) = rest.find('`') {
                found = rest[..end].parse().ok().or(found);
            }
        }
    }

    found
}

/// Every `-x` and `--flag` a flags-table row names, without its value.
fn flags_in(row: &str) -> Vec<String> {
    let cell = row.trim_start_matches("| ").split('|').next().unwrap_or("");

    cell.split('`')
        .filter(|token| token.starts_with('-'))
        .map(|token| {
            token
                .split_whitespace()
                .next()
                .unwrap_or(token)
                .trim_end_matches(',')
                .to_owned()
        })
        .collect()
}
