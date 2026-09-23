//! The CLI reference, run as a test suite.
//!
//! Every example on every page under `website/docs/reference/` is an
//! invocation and the bytes it printed. This suite stages the world each one
//! assumes, runs the command **as the page writes it**, and asserts the page's
//! own text against what came back — so a page and the binary cannot disagree
//! without CI saying so.
//!
//! Three things make that possible without rewriting the docs:
//!
//! - a [`World`](world::World) whose scratch directories carry the names the
//!   documentation gives them, so `~/planes` and `git@gitlab.com:acme/api.git`
//!   mean something;
//! - [normalisation](world::World::normalise) of the values that are volatile
//!   by design — generated plane ids, timestamps, object names, durations and
//!   the temporary paths the fixture lives at — applied to the page and the run
//!   alike;
//! - a [case](cases::Case) per example, which is the one thing that has to be
//!   written by hand: the page says what an invocation prints, and only a
//!   person can say what has to exist for it to print that.
//!
//! **An example with no case fails this suite.** Adding a command means adding
//! its cases; there is no way to document an invocation and leave it
//! unasserted.
//!
//! A case for something that is not built yet is marked
//! [`NotBuilt`](cases::Expectation::NotBuilt) and is expected to **disagree**.
//! When the ticket that builds it lands, the case starts agreeing and this
//! suite fails until the marker is removed — which is what keeps the gap
//! between the documentation and the binary visible, and shrinking.

#[path = "reference/cases.rs"]
mod cases;
#[path = "reference/docs.rs"]
mod docs;
#[path = "reference/world.rs"]
mod world;

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;

use cases::{Case, Expectation};
use docs::{Example, Page};
use world::World;

macro_rules! pages {
    ($($name:ident => $path:literal,)*) => {
        $(
            #[test]
            fn $name() {
                check($path);
            }
        )*

        /// The list above is the list this suite covers. A reference page that
        /// is not on it is a page nothing asserts, so the roll call is itself a
        /// test.
        const COVERED: &[&str] = &[$($path,)*];
    };
}

/// Every test the suite runs, under one module, so its names all begin
/// `reference::`.
///
/// That prefix is what lets CI report the documentation separately from the
/// code without keeping a second list of targets: the generic job runs
/// `cargo test --workspace -- --skip reference::` and this one runs
/// `cargo test --test reference`. Locally `cargo test --workspace` is still
/// everything.
mod reference {
    use super::*;

    pages! {
        exit_codes => "exit-codes.md",
        global_flags => "global-flags.md",
        member_syntax => "member-syntax.md",
        refusals_and_waivers => "refusals-and-waivers.md",
        plane_add => "plane/add.md",
        plane_create => "plane/create.md",
        plane_destroy => "plane/destroy.md",
        plane_doctor => "plane/doctor.md",
        plane_list => "plane/list.md",
        plane_rename => "plane/rename.md",
        plane_repair => "plane/repair.md",
        plane_rm => "plane/rm.md",
        plane_run => "plane/run.md",
        plane_show => "plane/show.md",
        plane_status => "plane/status.md",
        project_add => "project/add.md",
        project_adopt => "project/adopt.md",
        project_fetch => "project/fetch.md",
        project_list => "project/list.md",
        project_rename => "project/rename.md",
        project_rm => "project/rm.md",
        project_show => "project/show.md",
    }

    #[test]
    fn every_reference_page_is_covered_by_a_test() {
        let missing: Vec<String> = docs::pages()
            .iter()
            .map(|page| page.path.clone())
            .filter(|path| !COVERED.contains(&path.as_str()))
            .collect();

        assert!(
            missing.is_empty(),
            "these reference pages have no snapshot test:\n  {}\n\
             Add them to the `pages!` list in this file.",
            missing.join("\n  ")
        );
    }

    /// A flag on a page is a flag the binary takes — unless the page says it is not
    /// one yet, in which case it had better still not be.
    ///
    /// This is the half of the suite the examples cannot cover: a flag can be
    /// documented, never wired up, and never appear in an example. Reading the
    /// page's own admonition rather than a list kept here means the exception dies
    /// with the sentence that granted it.
    #[test]
    fn every_documented_flag_is_a_flag_the_binary_takes() {
        let mut problems = Vec::new();

        for page in docs::pages() {
            let Some(command) = command_of(&page) else {
                continue;
            };
            if page.status == docs::Status::NotImplemented {
                continue;
            }

            let world = World::new("reference-flags");
            let named = flags_in(&world.run(&format!("{command} --help")).stdout);

            for flag in &page.flags {
                let excused = page.not_accepted.contains(flag);
                match (named.contains(flag), excused) {
                    (false, false) => problems.push(format!(
                        "{}: `{command}` documents {flag}, which `{command} --help` does not name",
                        page.path
                    )),
                    (true, true) => problems.push(format!(
                        "{}: the admonition says {flag} is not accepted, and `{command} --help` \
                         names it.\n  The ticket that built it updates the admonition.",
                        page.path
                    )),
                    _ => {}
                }
            }
        }

        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// A marker is a claim about the world, and a claim with no reason behind it
    /// is how an expected failure becomes permanent.
    #[test]
    fn every_marked_case_says_why_it_is_marked() {
        let mut problems = Vec::new();

        for page in docs::pages() {
            for case in cases::of(&page.path) {
                let why = match case.expectation {
                    Expectation::Matches => continue,
                    Expectation::NotBuilt(why) | Expectation::Unstageable(why) => why,
                };

                if why.split_whitespace().count() < 4 {
                    problems.push(format!(
                        "{}: \"{}\" is marked with {why:?}, which does not say why",
                        page.path, case.example
                    ));
                }
            }
        }

        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}

/// One page: every example it documents, against the binary.
fn check(path: &str) {
    quietly();

    let page = docs::page(path);
    let cases = cases::of(path);
    let mut problems = Vec::new();

    for example in &page.examples {
        if !cases.iter().any(|case| case.example == example.label()) {
            problems.push(format!(
                "`$ {}` under \"{}\" has no case.\n  \
                 Add one to `cases::of(\"{path}\")` — a documented invocation \
                 nothing runs is a documented invocation that can drift.",
                example.command,
                example.label(),
            ));
        }
    }

    for case in &cases {
        let Some(example) = page.example(case.example) else {
            problems.push(format!(
                "there is no example \"{}\" on this page any more; the case is stale",
                case.example
            ));
            continue;
        };

        if let Some(problem) = verdict(case, example, &page) {
            problems.push(problem);
        }
    }

    assert!(
        problems.is_empty(),
        "{path} and the binary disagree:\n\n{}\n",
        problems.join("\n\n")
    );
}

/// What a case's outcome means, given what it said to expect.
fn verdict(case: &Case, example: &Example, page: &Page) -> Option<String> {
    let Expectation::Unstageable(_) = case.expectation else {
        let outcome = attempt(case, example, page);

        return match (&case.expectation, outcome) {
            (Expectation::Matches, Err(why)) => Some(format!("\"{}\"\n{why}", case.example)),
            (Expectation::Matches, Ok(())) => None,
            (Expectation::NotBuilt(_), Err(_)) => None,
            (Expectation::NotBuilt(why), Ok(())) => Some(format!(
                "\"{}\" is marked not-built ({why}), and it now agrees with the page.\n  \
                 The ticket that built it unmarks the case: drop `Expectation::NotBuilt` \
                 and leave `Expectation::Matches`.",
                case.example
            )),
            (Expectation::Unstageable(_), _) => None,
        };
    };

    None
}

/// Builds the world, runs the example, and compares both renderings.
fn attempt(case: &Case, example: &Example, page: &Page) -> Result<(), String> {
    EXPECTED.set(true);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        let mut world = World::new(&label(page, case));
        (case.build)(&mut world);

        if let Some(signal) = example.annotations.iter().find(|line| line.contains('^')) {
            return Err(format!(
                "  the page shows {signal} after the command, which a snapshot \
                 cannot send; mark the case Unstageable"
            ));
        }

        let run = world.run(&example.command);

        // Whatever else the command block held is prose the page says arrived
        // on stderr while the command was still running — a lock being waited
        // for, a script's output streamed through.
        for line in &example.annotations {
            if !run.stderr.contains(line.as_str()) {
                return Err(format!(
                    "  the page shows {line:?} on stderr, and bp printed:\n  {}",
                    run.stderr
                ));
            }
        }

        if run.code != example.exit {
            return Err(format!(
                "  the page says exit {}, bp exited {}\n  stdout: {}\n  stderr: {}",
                example.exit, run.code, run.stdout, run.stderr
            ));
        }

        world::same(&run.stdout, &world.normalise(&example.stdout))
            .map_err(|report| indented("stdout", &report))?;
        world::same(&run.stderr, &world.normalise(&example.stderr))
            .map_err(|report| indented("stderr", &report))?;

        match example.stderr.is_empty() {
            true => Ok(()),
            false => both_renderings(&world, example),
        }
    }));

    EXPECTED.set(false);

    caught.unwrap_or_else(|payload| Err(format!("  the case panicked: {}", panic_text(&payload))))
}

/// The same invocation under `--json`, against the same fixture.
///
/// Every line of the human envelope maps to exactly one field of the JSON one,
/// so the two are asserted against each other rather than against a second
/// expectation — there is no second expectation to keep in step.
fn both_renderings(world: &World, example: &Example) -> Result<(), String> {
    let run = world.run_with(&example.command, &["--json"]);

    if !run.stdout.is_empty() {
        return Err(format!("  --json put a failure on stdout: {}", run.stdout));
    }

    // The bytes, not the normalisation: rewriting what bitplane quotes from git
    // is right for a page and would cut a line of JSON in half.
    let envelope: serde_json::Value =
        serde_json::from_str(run.raw_stderr.trim()).map_err(|err| {
            format!(
                "  --json did not render an envelope ({err}): {}",
                run.stderr
            )
        })?;

    if run.code != example.exit {
        return Err(format!(
            "  --json exited {} where the human rendering exited {}",
            run.code, example.exit
        ));
    }
    if envelope["code"] != example.exit {
        return Err(format!(
            "  the envelope's code is {} and the process exited {}",
            envelope["code"], example.exit
        ));
    }

    let human: Vec<String> = world
        .normalise(&example.stderr)
        .lines()
        .map(str::to_owned)
        .collect();
    let head = format!(
        "error[{}]: {}",
        text(&envelope["error"]),
        text(&envelope["message"])
    );

    if human.first().map(String::as_str) != Some(world.normalise(&head).trim_end()) {
        return Err(format!(
            "  the envelope's error and message render as\n    {}\n  and the page opens with\n    {}",
            world.normalise(&head),
            human.first().cloned().unwrap_or_default()
        ));
    }

    let rows: Vec<Vec<String>> = human
        .iter()
        .filter(|line| line.starts_with("  ") && !line.trim().is_empty())
        .map(|line| world::cells(line))
        .collect();
    let problems = envelope["problems"]
        .as_array()
        .ok_or("  the envelope has no problems array")?;

    if rows.len() != problems.len() {
        return Err(format!(
            "  the page shows {} indented rows and the envelope carries {} problems",
            rows.len(),
            problems.len()
        ));
    }

    for (row, problem) in rows.iter().zip(problems) {
        let subject = text(&problem["subject"]);
        let message = world.normalise(&text(&problem["message"]));
        let mut expected: Vec<String> = Vec::new();
        if !subject.is_empty() {
            expected.push(world.normalise(&subject).trim_end().to_owned());
        }
        expected.extend(world::cells(&message));

        if *row != expected {
            return Err(format!(
                "  the page's row {row:?} is not the envelope's problem {expected:?}"
            ));
        }
    }

    let remedy = human
        .iter()
        .find_map(|line| line.strip_prefix("remedy: "))
        .map(str::to_owned);
    let carried = envelope["remedy"]
        .as_str()
        .map(|remedy| world.normalise(remedy).trim_end().to_owned());

    match remedy == carried {
        true => Ok(()),
        false => Err(format!(
            "  the page's remedy is {remedy:?} and the envelope's is {carried:?}"
        )),
    }
}

/// Every flag a help screen names, as whole words.
///
/// Containment would not do: `-p` is inside `--plane`, so a short flag could
/// never fail the check that is supposed to catch it.
fn flags_in(help: &str) -> Vec<String> {
    help.split([' ', ',', '\n', '\t', '[', ']'])
        .filter(|word| word.starts_with('-') && word.len() > 1)
        .map(str::to_owned)
        .collect()
}

fn command_of(page: &Page) -> Option<String> {
    let synopsis = page.synopsis.lines().next()?;
    let words: Vec<&str> = synopsis.split_whitespace().collect();

    match words.as_slice() {
        ["bp", "project", verb, ..] => Some(format!("bp project {verb}")),
        ["bp", verb, ..] if !verb.starts_with('-') && !verb.starts_with('<') => {
            Some(format!("bp {verb}"))
        }
        _ => None,
    }
}

fn label(page: &Page, case: &Case) -> String {
    let page = page.path.replace(['/', '.'], "-");
    let case: String = case
        .example
        .chars()
        .map(|character| match character.is_ascii_alphanumeric() {
            true => character.to_ascii_lowercase(),
            false => '-',
        })
        .collect();

    format!("ref-{page}-{case}")
}

fn indented(stream: &str, report: &str) -> String {
    let body: Vec<String> = report.lines().map(|line| format!("  {line}")).collect();

    format!("  on {stream}, {}", body.join("\n").trim_start())
}

fn text(value: &serde_json::Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn panic_text(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
        })
        .unwrap_or_else(|| "(no message)".to_owned())
}

thread_local! {
    /// Whether a panic on this thread is one the suite is deliberately
    /// provoking.
    static EXPECTED: Cell<bool> = const { Cell::new(false) };
}

/// Silences the panics a case's own failure makes, and nothing else.
///
/// A case that is expected to fail fails by panicking, and a page full of them
/// would bury the report under backtraces nobody asked for. The suite's own
/// assertions still print: only a panic raised inside [`attempt`], on this
/// thread, is swallowed — and its message is carried into the report instead.
fn quietly() {
    static ONCE: Once = Once::new();

    ONCE.call_once(|| {
        let loud = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !EXPECTED.get() {
                loud(info);
            }
        }));
    });
}
