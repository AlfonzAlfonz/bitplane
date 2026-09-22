//! The two renderings.
//!
//! **The stream is decided by what the output is, the rendering by the mode**
//! (ADR-0003, as ticket 04 amended it). Results go to stdout and failures to
//! stderr in both modes; only the rendering changes, and `--json` is the only
//! switch. There is no TTY detection, so redirecting never changes the bytes.
//!
//! Every line of the human envelope maps to exactly one field — `error[<tag>]:
//! <message>`, one indented row per problem, `remedy: <sentence>` — which is
//! what keeps the two renderings from drifting apart and lets one fixture
//! assert both. `code` is not printed: it is the exit status, and a number on
//! screen that duplicates `$?` is noise.
//!
//! One grammar is used everywhere something is listed: a header line, then
//! indented rows with every column but the last padded to its widest cell. A
//! fan-out **never prints a count in place of the rows**.

use bitplane_core::member::PROJECT_SIGIL;
use bitplane_core::{
    CreatedMember, ErrorEnvelope, Fetched, Finding, Head, MemberView, MemberWork, Outcome,
    PerMember, PerProject, PlaneCreated, PlaneHealth, PlaneList, PlaneStatus, PlaneView, Problem,
    ProjectAdded, ProjectFetched, ProjectListing, ProjectSummary, Response,
};

/// Whether output is for a person or a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rendering {
    Human,
    Json,
}

/// A response, for stdout.
pub fn response(response: &Response, rendering: Rendering) -> String {
    match rendering {
        Rendering::Json => {
            let mut json =
                serde_json::to_string(response).expect("a response is always serialisable");
            json.push('\n');
            json
        }
        Rendering::Human => match response {
            Response::PlaneCreate(created) => plane(created),
            Response::PlaneList(list) => listing(list),
            Response::PlaneShow(plane) => detail(plane),
            Response::PlaneStatus(status) => fan_out(status),
            Response::ProjectAdd(added) => project(added),
            Response::ProjectList(listing) => projects(listing),
            Response::ProjectFetch(fetched) => fetches(fetched),
        },
    }
}

/// A failure, for stderr.
pub fn envelope(envelope: &ErrorEnvelope, rendering: Rendering) -> String {
    match rendering {
        Rendering::Json => format!("{}\n", envelope.to_json()),
        Rendering::Human => human_envelope(envelope),
    }
}

/// A plane, then one row per member in the order they were named.
fn plane(created: &PlaneCreated) -> String {
    let rows: Vec<Vec<String>> = created.members.iter().map(member_row).collect();

    let mut text = format!(
        "{}  {}\n\n{}",
        created.id,
        created.directory.display(),
        columns(&rows, INDENT)
    );

    // An interrupt that could not take the whole plane back has left something,
    // and the rows alone do not say so.
    if created.remnant {
        text.push_str(&format!(
            "\n{} could not be fully removed; run bp destroy -p {} to clear the remnant.\n",
            created.id, created.id
        ));
    }

    text
}

/// `bp list`: one block per plane, separated by a blank line.
///
/// The headers are laid out as one table across every plane, so the directories
/// line up down the page; the members of each plane are a table of their own,
/// because a long ad-hoc path in one plane should not indent every other
/// plane's rows off the screen.
fn listing(list: &PlaneList) -> String {
    let headers: Vec<Vec<String>> = list
        .planes
        .iter()
        .map(|plane| {
            vec![
                plane.id.clone(),
                plane.directory.display().to_string(),
                match &plane.created_at {
                    Some(created) => format!("created {}", day(created)),
                    None => String::new(),
                },
            ]
        })
        .collect();

    // Zipped as rows rather than as rendered lines: a plane directory whose
    // name contains a newline would otherwise slide every following plane one
    // header out of step and drop the last one entirely.
    aligned(&headers, "")
        .into_iter()
        .zip(&list.planes)
        .map(|(header, plane)| {
            format!(
                "{header}{}{}",
                columns(&member_rows(plane), INDENT),
                notes(&plane.health),
            )
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// `bp show`: the plane's own facts, its members, then what was found.
fn detail(plane: &PlaneView) -> String {
    let mut facts = vec![vec![
        "directory".to_owned(),
        plane.directory.display().to_string(),
    ]];
    if let Some(created) = &plane.created_at {
        facts.push(vec!["created".to_owned(), minute(created)]);
    }
    facts.push(vec!["members".to_owned(), plane.members.len().to_string()]);
    if plane.health.has_findings() {
        facts.push(vec!["health".to_owned(), "broken".to_owned()]);
    }

    let rows: Vec<Vec<String>> = plane
        .members
        .iter()
        .map(|member| {
            vec![
                member.member.to_string(),
                branch(member.head.as_ref()),
                member.path.to_string(),
                labels(&plane.health, member),
            ]
        })
        .collect();

    // A plane with no members prints no blank line and no block, rather than
    // the separator for a table that is not there.
    let members = match rows.is_empty() {
        true => String::new(),
        false => format!("\n{}", columns(&rows, INDENT)),
    };

    format!(
        "{}\n{}{members}{}",
        plane.id,
        columns(&facts, INDENT),
        findings(&plane.health),
    )
}

/// `bp status`: git's answer per member, then what was found.
fn fan_out(status: &PlaneStatus) -> String {
    let rows: Vec<Vec<String>> = status
        .members
        .iter()
        .map(|member| {
            vec![
                member.member.to_string(),
                branch(member.head.as_ref()),
                work(&member.work),
            ]
        })
        .collect();

    format!(
        "{}  {}\n\n{}{}",
        status.id,
        status.directory.display(),
        columns(&rows, INDENT),
        findings(&status.health),
    )
}

/// A registered project: what kind it is, and the three things a user needs to
/// find it again.
fn project(added: &ProjectAdded) -> String {
    let default = added
        .default_branch
        .clone()
        .unwrap_or_else(|| "unspecified".to_owned());

    format!(
        "{PROJECT_SIGIL}{}  {}\n{}",
        added.name,
        added.source.kind(),
        columns(
            &[
                vec!["source".to_owned(), added.source.to_string()],
                vec![
                    "directory".to_owned(),
                    added.directory.display().to_string()
                ],
                vec!["default".to_owned(), default],
            ],
            "  ",
        )
    )
}

/// One row per project, in name order.
///
/// A project whose file would not parse is a row too, naming the file and the
/// error — the scan reported it rather than dying on it, and printing a count
/// of the readable ones instead would hide exactly the project that needs
/// looking at.
fn projects(listing: &ProjectListing) -> String {
    columns(
        &listing.projects.iter().map(project_row).collect::<Vec<_>>(),
        "  ",
    )
}

/// `<project>  <kind>  <source>  [repo.git missing]`, or `<project>  <error>`.
fn project_row(row: &PerProject<ProjectSummary>) -> Vec<String> {
    let name = format!("{PROJECT_SIGIL}{}", row.project);

    match &row.outcome {
        Outcome::Ok(summary) => vec![
            name,
            summary.source.kind().to_owned(),
            summary.source.to_string(),
            if summary.source_repo_present {
                String::new()
            } else {
                "repo.git missing".to_owned()
            },
        ],
        Outcome::AlreadyDone => vec![name, "unchanged".to_owned()],
        Outcome::Skipped(reason) => vec![name, format!("skipped: {}", reason.reason())],
        Outcome::Failed(error) => vec![name, error.to_string()],
    }
}

/// One row per project, in the order they were named.
fn fetches(fetched: &ProjectFetched) -> String {
    columns(
        &fetched.projects.iter().map(fetch_row).collect::<Vec<_>>(),
        "  ",
    )
}

/// `<project>  <outcome>  <detail>`.
fn fetch_row(row: &PerProject<Fetched>) -> Vec<String> {
    let name = format!("{PROJECT_SIGIL}{}", row.project);

    match &row.outcome {
        Outcome::Ok(Fetched { updated: 0 }) => {
            vec![name, "fetched".to_owned(), "up to date".to_owned()]
        }
        Outcome::Ok(Fetched { updated }) => vec![
            name,
            "fetched".to_owned(),
            format!("{updated} {} updated", plural(*updated, "ref")),
        ],
        Outcome::AlreadyDone => vec![name, "fetched".to_owned(), "up to date".to_owned()],
        // An adopted project has nothing to fetch, which is a row rather than
        // an outcome word: there was no fetch to report on.
        Outcome::Skipped(reason) => vec![name, dash(), reason.reason().to_owned()],
        Outcome::Failed(error) => vec![name, "failed".to_owned(), error.to_string()],
    }
}

/// `<member>  <branch>  <outcome>  <path>`.
fn member_row(row: &PerMember<CreatedMember>) -> Vec<String> {
    let (branch, outcome, path) = match &row.outcome {
        Outcome::Ok(member) => (
            member.branch.clone(),
            if member.unwound {
                "removed".to_owned()
            } else if member.created_branch {
                "created (new branch)".to_owned()
            } else {
                "created".to_owned()
            },
            member.path.to_string(),
        ),
        Outcome::AlreadyDone => (dash(), "unchanged".to_owned(), dash()),
        Outcome::Skipped(reason) => (dash(), format!("skipped: {}", reason.reason()), dash()),
        Outcome::Failed(error) => (dash(), format!("failed: {error}"), dash()),
    };

    vec![row.member.to_string(), branch, outcome, path]
}

/// `<member>  <branch>  <what was found about it>`.
fn member_rows(plane: &PlaneView) -> Vec<Vec<String>> {
    plane
        .members
        .iter()
        .map(|member| {
            vec![
                member.member.to_string(),
                branch(member.head.as_ref()),
                labels(&plane.health, member),
            ]
        })
        .collect()
}

/// The findings about the plane itself, which belong to no member's row.
fn notes(health: &PlaneHealth) -> String {
    health
        .about_the_plane()
        .map(|finding| format!("{INDENT}{finding}\n"))
        .collect()
}

/// The few words naming what was found about one member, where anything was.
fn labels(health: &PlaneHealth, member: &MemberView) -> String {
    health
        .about(&member.member)
        .map(Finding::label)
        .collect::<Vec<&str>>()
        .join(", ")
}

/// The block `bp show` and `bp status` print under the rows: every finding, and
/// the command that fixes it. A read reports and never repairs.
fn findings(health: &PlaneHealth) -> String {
    if !health.has_findings() {
        return String::new();
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    for finding in &health.findings {
        rows.push(vec![
            finding
                .member()
                .map(ToString::to_string)
                .unwrap_or_default(),
            finding.to_string(),
        ]);
        if let Some(remedy) = finding.remedy() {
            rows.push(vec![String::new(), remedy]);
        }
    }

    format!("\nfindings\n{}", columns(&rows, INDENT))
}

/// What git said about one worktree, in the words the reference page uses.
///
/// `clean` is the working tree's own word, so a branch with commits to push but
/// nothing uncommitted reads `clean, 1 ahead` — none of which is drift.
fn work(work: &MemberWork) -> String {
    match work {
        MemberWork::Reported {
            modified,
            untracked,
            ahead,
        } => {
            let mut parts = Vec::new();

            if *modified > 0 {
                parts.push(format!("{modified} modified"));
            }
            if *untracked > 0 {
                parts.push(format!("{untracked} untracked"));
            }
            if parts.is_empty() {
                parts.push("clean".to_owned());
            }
            if *ahead > 0 {
                parts.push(format!("{ahead} ahead"));
            }

            parts.join(", ")
        }
        MemberWork::WorktreeMissing => "worktree missing".to_owned(),
        MemberWork::SourceRepoMissing => "source repo missing".to_owned(),
        MemberWork::Unreadable { message } => message.clone(),
    }
}

fn branch(head: Option<&Head>) -> String {
    head.map(ToString::to_string).unwrap_or_else(dash)
}

/// The date alone, for a listing where the time of day is noise.
fn day(stamp: &str) -> &str {
    stamp.get(..10).unwrap_or(stamp)
}

/// The date and the minute, for the one plane `bp show` is about.
fn minute(stamp: &str) -> String {
    match (stamp.get(..10), stamp.get(11..16)) {
        (Some(day), Some(minute)) => format!("{day} {minute}"),
        _ => stamp.to_owned(),
    }
}

fn human_envelope(envelope: &ErrorEnvelope) -> String {
    let mut text = format!("error[{}]: {}\n", envelope.error, envelope.message);

    // Three things are omitted rather than rendered empty: an envelope with no
    // problems prints no indented block, a problem with no subject leaves the
    // column blank, and an absent remedy prints no line.
    if !envelope.problems.is_empty() {
        let rows: Vec<Vec<String>> = envelope.problems.iter().map(problem_row).collect();
        text.push('\n');
        text.push_str(&columns(&rows, INDENT));
    }

    if let Some(remedy) = &envelope.remedy {
        text.push_str(&format!("\nremedy: {remedy}\n"));
    }

    text
}

fn problem_row(problem: &Problem) -> Vec<String> {
    vec![
        problem.subject.clone().unwrap_or_default(),
        problem.message.clone(),
    ]
}

/// What every row but a header is indented by.
const INDENT: &str = "  ";

/// What separates one column from the next.
const GUTTER: &str = "  ";

/// [`aligned`], as one block of text.
fn columns(rows: &[Vec<String>], indent: &str) -> String {
    aligned(rows, indent).concat()
}

/// One rendered line per row, each ending in its newline, with every column but
/// a row's own last padded to its widest cell — the one grammar every fan-out,
/// every listing and every problem block uses.
///
/// A `Vec` rather than a block, because a caller that pairs rows with the things
/// they came from must not have to split the text back up: a cell containing a
/// newline would make that split silently wrong.
///
/// **Rows may be ragged, and a row's last cell neither pads nor widens.** A
/// project listing puts a whole parse error where another row has a kind and a
/// source, and that error must not push its neighbours' sources off to the
/// right; it is a differently shaped row, not a long one. A trailing empty cell
/// is trimmed rather than printed, so a column nothing in a listing uses costs
/// no trailing whitespace. For rows that all run the full width — every other
/// caller — this is the same rule as "all but the last".
fn aligned(rows: &[Vec<String>], indent: &str) -> Vec<String> {
    let arity = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0usize; arity];

    for row in rows {
        for (column, cell) in row.iter().enumerate().take(row.len().saturating_sub(1)) {
            widths[column] = widths[column].max(cell.chars().count());
        }
    }

    rows.iter()
        .map(|row| {
            let last = row.len().saturating_sub(1);
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(column, cell)| {
                    if column == last {
                        cell.clone()
                    } else {
                        let padding = widths[column].saturating_sub(cell.chars().count());
                        format!("{cell}{}", " ".repeat(padding))
                    }
                })
                .collect();

            format!("{indent}{}\n", cells.join(GUTTER).trim_end())
        })
        .collect()
}

/// `ref` or `refs`, for a count the row prints.
fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        noun.to_owned()
    } else {
        format!("{noun}s")
    }
}

/// What a column holds when there is nothing to report there.
fn dash() -> String {
    "-".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitplane_core::{
        EngineError, HealthCheck, MemberRef, MemberStatus, PlaneId, ProjectName, ProjectSource,
        SkipReason, WorktreePath,
    };
    use std::path::PathBuf;

    #[test]
    fn a_created_plane_prints_a_header_and_one_row_per_member() {
        let rendered = response(&Response::PlaneCreate(created()), Rendering::Human);

        assert_eq!(
            rendered,
            concat!(
                "bp-7c1e0d44  /Users/alfonz/planes/bp-7c1e0d44\n",
                "\n",
                "  @api                             feat-login  created               acme/api\n",
                "  /Users/alfonz/projects/bitplane  feat-login  created (new branch)  projects/bitplane\n",
            )
        );
    }

    #[test]
    fn a_skipped_member_says_why_and_has_no_branch_to_report() {
        let mut plane = created();
        plane.members[1] = PerMember::skipped(
            MemberRef::Repo(PathBuf::from("/Users/alfonz/projects/bitplane")),
            SkipReason::Interrupted,
        );

        let rendered = response(&Response::PlaneCreate(plane), Rendering::Human);

        assert!(
            rendered.contains("-           skipped: interrupted"),
            "got:\n{rendered}"
        );
    }

    #[test]
    fn an_interrupt_that_left_a_remnant_says_so_and_names_the_way_out() {
        let mut plane = created();
        plane.interrupted = true;
        plane.remnant = true;

        let rendered = response(&Response::PlaneCreate(plane), Rendering::Human);

        assert!(
            rendered.ends_with(
                "bp-7c1e0d44 could not be fully removed; \
                 run bp destroy -p bp-7c1e0d44 to clear the remnant.\n"
            ),
            "got:\n{rendered}"
        );
    }

    #[test]
    fn the_json_rendering_is_one_line_of_the_response_itself() {
        let rendered = response(&Response::PlaneCreate(created()), Rendering::Json);

        assert_eq!(rendered.lines().count(), 1);
        let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(json["action"], "plane_create");
        assert_eq!(json["id"], "bp-7c1e0d44");
    }

    #[test]
    fn a_listing_lines_the_planes_up_and_gives_each_its_own_members() {
        let list = PlaneList {
            planes: vec![healthy(), drifted()],
        };

        assert_eq!(
            response(&Response::PlaneList(list), Rendering::Human),
            concat!(
                "auth-work    /planes/auth-work    created 2026-09-18\n",
                "  @api  feat-login\n",
                "  @web  feat-login\n",
                "\n",
                "bp-a3f9c2e1  /planes/bp-a3f9c2e1  created 2026-09-21\n",
                "  @api  feat-x  worktree missing\n",
                "  create never completed, started 3 days ago\n",
            )
        );
    }

    #[test]
    fn a_plane_file_that_would_not_parse_is_a_row_rather_than_the_end_of_the_scan() {
        let list = PlaneList {
            planes: vec![healthy(), unreadable()],
        };

        let rendered = response(&Response::PlaneList(list), Rendering::Human);

        assert!(rendered.contains("@api  feat-login"), "got:\n{rendered}");
        assert!(
            rendered.ends_with(
                "broken.plane  /planes/broken.plane\n  \
                 /planes/broken.plane/plane.toml: unknown key \"status\"\n"
            ),
            "got:\n{rendered}"
        );
    }

    #[test]
    fn a_plane_directory_whose_name_holds_a_newline_does_not_shift_the_listing() {
        let mut odd = healthy();
        odd.id = "aa\nbb".to_owned();

        let list = PlaneList {
            planes: vec![odd, drifted()],
        };

        let rendered = response(&Response::PlaneList(list), Rendering::Human);

        assert!(
            rendered.contains("bp-a3f9c2e1"),
            "the plane after it is still printed: got\n{rendered}"
        );
        assert!(
            rendered.contains("create never completed"),
            "and so are its findings: got\n{rendered}"
        );
    }

    #[test]
    fn a_plane_with_no_members_prints_no_separator_for_a_table_that_is_not_there() {
        let mut empty = healthy();
        empty.members = Vec::new();

        assert_eq!(
            response(&Response::PlaneShow(empty), Rendering::Human),
            concat!(
                "auth-work\n",
                "  directory  /planes/auth-work\n",
                "  created    2026-09-18 09:41\n",
                "  members    0\n",
            )
        );
    }

    #[test]
    fn no_planes_at_all_prints_nothing() {
        assert_eq!(
            response(&Response::PlaneList(PlaneList::default()), Rendering::Human),
            ""
        );
    }

    #[test]
    fn a_healthy_plane_shows_its_facts_and_its_members_and_no_findings() {
        assert_eq!(
            response(&Response::PlaneShow(healthy()), Rendering::Human),
            concat!(
                "auth-work\n",
                "  directory  /planes/auth-work\n",
                "  created    2026-09-18 09:41\n",
                "  members    2\n",
                "\n",
                "  @api  feat-login  acme/api\n",
                "  @web  feat-login  acme/web\n",
            )
        );
    }

    #[test]
    fn a_plane_with_a_finding_says_health_broken_and_names_the_way_out() {
        let rendered = response(&Response::PlaneShow(drifted()), Rendering::Human);

        assert!(
            rendered.contains("  health     broken\n"),
            "got:\n{rendered}"
        );
        assert!(
            rendered.ends_with(concat!(
                "findings\n",
                "        create never completed, started 3 days ago\n",
                "        nothing in it is yours; bp destroy -p bp-a3f9c2e1 clears it\n",
                "  @api  the worktree at acme/api is not there\n",
                "        bp rm @api drops it; bp add @api:<branch> puts it back\n",
            )),
            "got:\n{rendered}"
        );
    }

    #[test]
    fn a_status_row_reads_in_the_words_the_reference_page_uses() {
        let rendered = response(&Response::PlaneStatus(status()), Rendering::Human);

        assert_eq!(
            rendered,
            concat!(
                "bp-a3f9c2e1  /planes/bp-a3f9c2e1\n",
                "\n",
                "  @api   feat-login             3 modified, 1 untracked, 2 ahead\n",
                "  @web   feat-login             clean\n",
                "  @docs  feat-login             clean, 1 ahead\n",
                "  @ops   (detached at 9f2c1ab)  clean\n",
                "  @gone  -                      worktree missing\n",
                "\n",
                "findings\n",
                "  @gone  the worktree at acme/gone is not there\n",
                "         bp rm @gone drops it; bp add @gone:<branch> puts it back\n",
            )
        );
    }

    #[test]
    fn every_line_of_the_human_envelope_maps_to_one_field() {
        let envelope = ErrorEnvelope::usage("refused", "refusing to destroy bp-a3f9c2e1")
            .with_problems(vec![
                Problem::about("@api", "feat-login has uncommitted changes"),
                Problem::about("@web", "feat-login has untracked files"),
            ])
            .with_remedy("Inspect the members listed.");

        assert_eq!(
            super::envelope(&envelope, Rendering::Human),
            concat!(
                "error[refused]: refusing to destroy bp-a3f9c2e1\n",
                "\n",
                "  @api  feat-login has uncommitted changes\n",
                "  @web  feat-login has untracked files\n",
                "\n",
                "remedy: Inspect the members listed.\n",
            )
        );
    }

    #[test]
    fn an_envelope_with_no_problems_and_no_remedy_is_one_line() {
        let envelope = ErrorEnvelope::usage("io", "/planes/x: permission denied");

        assert_eq!(
            super::envelope(&envelope, Rendering::Human),
            "error[io]: /planes/x: permission denied\n"
        );
    }

    #[test]
    fn a_problem_with_no_subject_leaves_the_column_blank() {
        let envelope =
            ErrorEnvelope::usage("refused", "two things were wrong").with_problems(vec![
                Problem::about("@api", "is occupied"),
                Problem::new("and the plane is locked"),
            ]);

        assert_eq!(
            super::envelope(&envelope, Rendering::Human),
            concat!(
                "error[refused]: two things were wrong\n",
                "\n",
                "  @api  is occupied\n",
                "        and the plane is locked\n",
            )
        );
    }

    #[test]
    fn the_two_renderings_of_one_envelope_carry_the_same_five_fields() {
        let envelope = ErrorEnvelope::usage("plane_id_in_use", "auth-work is already a plane")
            .with_remedy("Choose another id.");

        let json: serde_json::Value =
            serde_json::from_str(&super::envelope(&envelope, Rendering::Json)).unwrap();
        let human = super::envelope(&envelope, Rendering::Human);

        assert!(human.contains(json["error"].as_str().unwrap()));
        assert!(human.contains(json["message"].as_str().unwrap()));
        assert!(human.contains(json["remedy"].as_str().unwrap()));
        assert!(!human.contains("\"code\""), "the code is the exit status");
    }

    #[test]
    fn a_registered_project_prints_its_kind_and_where_to_find_it() {
        let rendered = response(
            &Response::ProjectAdd(ProjectAdded {
                name: ProjectName::parse("codestyle").unwrap(),
                source: ProjectSource::Owned {
                    url: "git@gitlab.com:acme/codestyle.git".to_owned(),
                },
                directory: PathBuf::from("/data/bitplane/projects/codestyle"),
                default_branch: Some("main".to_owned()),
            }),
            Rendering::Human,
        );

        assert_eq!(
            rendered,
            concat!(
                "@codestyle  owned\n",
                "  source     git@gitlab.com:acme/codestyle.git\n",
                "  directory  /data/bitplane/projects/codestyle\n",
                "  default    main\n",
            )
        );
    }

    #[test]
    fn a_forge_that_named_no_default_branch_says_so_rather_than_leaving_it_blank() {
        let rendered = response(
            &Response::ProjectAdd(ProjectAdded {
                name: ProjectName::parse("empty").unwrap(),
                source: ProjectSource::Owned {
                    url: "git@gitlab.com:acme/empty.git".to_owned(),
                },
                directory: PathBuf::from("/data/bitplane/projects/empty"),
                default_branch: None,
            }),
            Rendering::Human,
        );

        assert!(
            rendered.ends_with("  default    unspecified\n"),
            "got:\n{rendered}"
        );
    }

    #[test]
    fn a_listing_prints_a_row_per_project_and_a_missing_repo_as_a_column() {
        let rendered = response(&Response::ProjectList(listing()), Rendering::Human);

        assert_eq!(
            rendered,
            concat!(
                "  @api        owned    git@gitlab.com:acme/api.git        repo.git missing\n",
                "  @bitplane   adopted  /Users/alfonz/projects/bitplane\n",
                "  @codestyle  owned    git@gitlab.com:acme/codestyle.git\n",
            )
        );
    }

    #[test]
    fn an_unreadable_project_is_a_row_that_does_not_widen_its_neighbours_columns() {
        let mut listing = listing();
        listing.projects[2] = PerProject::failed(
            ProjectName::parse("codestyle").unwrap(),
            EngineError::ParseError {
                path: PathBuf::from("/data/projects/codestyle/project.toml"),
                message: "unknown key \"default_branch\"".to_owned(),
                legal_keys: Vec::new(),
            },
        );

        let rendered = response(&Response::ProjectList(listing), Rendering::Human);

        assert_eq!(
            rendered,
            concat!(
                "  @api        owned    git@gitlab.com:acme/api.git      repo.git missing\n",
                "  @bitplane   adopted  /Users/alfonz/projects/bitplane\n",
                "  @codestyle  /data/projects/codestyle/project.toml: unknown key \"default_branch\"\n",
            )
        );
    }

    #[test]
    fn a_fetch_prints_what_each_project_did_and_an_adopted_one_has_no_outcome() {
        let rendered = response(
            &Response::ProjectFetch(ProjectFetched {
                projects: vec![
                    PerProject::ok(ProjectName::parse("api").unwrap(), Fetched { updated: 3 }),
                    PerProject::ok(ProjectName::parse("one").unwrap(), Fetched { updated: 1 }),
                    PerProject::ok(
                        ProjectName::parse("codestyle").unwrap(),
                        Fetched { updated: 0 },
                    ),
                    PerProject::skipped(
                        ProjectName::parse("bitplane").unwrap(),
                        SkipReason::NothingToFetch,
                    ),
                    PerProject::failed(
                        ProjectName::parse("web").unwrap(),
                        EngineError::GitFailed {
                            message: "git fetch exited 128: Connection refused".to_owned(),
                        },
                    ),
                ],
                interrupted: false,
            }),
            Rendering::Human,
        );

        assert_eq!(
            rendered,
            concat!(
                "  @api        fetched  3 refs updated\n",
                "  @one        fetched  1 ref updated\n",
                "  @codestyle  fetched  up to date\n",
                "  @bitplane   -        nothing to fetch (adopted)\n",
                "  @web        failed   git fetch exited 128: Connection refused\n",
            )
        );
    }

    fn listing() -> ProjectListing {
        ProjectListing {
            projects: vec![
                PerProject::ok(
                    ProjectName::parse("api").unwrap(),
                    ProjectSummary {
                        source: ProjectSource::Owned {
                            url: "git@gitlab.com:acme/api.git".to_owned(),
                        },
                        source_repo_present: false,
                    },
                ),
                PerProject::ok(
                    ProjectName::parse("bitplane").unwrap(),
                    ProjectSummary {
                        source: ProjectSource::Adopted {
                            path: PathBuf::from("/Users/alfonz/projects/bitplane"),
                        },
                        source_repo_present: true,
                    },
                ),
                PerProject::ok(
                    ProjectName::parse("codestyle").unwrap(),
                    ProjectSummary {
                        source: ProjectSource::Owned {
                            url: "git@gitlab.com:acme/codestyle.git".to_owned(),
                        },
                        source_repo_present: true,
                    },
                ),
            ],
        }
    }

    fn created() -> PlaneCreated {
        PlaneCreated {
            id: PlaneId::parse("bp-7c1e0d44").unwrap(),
            directory: PathBuf::from("/Users/alfonz/planes/bp-7c1e0d44"),
            members: vec![
                PerMember::ok(
                    MemberRef::parse("@api").unwrap(),
                    CreatedMember {
                        branch: "feat-login".to_owned(),
                        created_branch: false,
                        path: WorktreePath::parse("acme/api").unwrap(),
                        unwound: false,
                    },
                ),
                PerMember::ok(
                    MemberRef::Repo(PathBuf::from("/Users/alfonz/projects/bitplane")),
                    CreatedMember {
                        branch: "feat-login".to_owned(),
                        created_branch: true,
                        path: WorktreePath::parse("projects/bitplane").unwrap(),
                        unwound: false,
                    },
                ),
            ],
            interrupted: false,
            remnant: false,
        }
    }

    fn healthy() -> PlaneView {
        PlaneView {
            id: "auth-work".to_owned(),
            directory: PathBuf::from("/planes/auth-work"),
            created_at: Some("2026-09-18T09:41:07Z".to_owned()),
            members: vec![on_branch("@api", "acme/api"), on_branch("@web", "acme/web")],
            health: PlaneHealth::sound(HealthCheck::Cheap),
        }
    }

    fn drifted() -> PlaneView {
        let api = MemberRef::parse("@api").unwrap();

        PlaneView {
            id: "bp-a3f9c2e1".to_owned(),
            directory: PathBuf::from("/planes/bp-a3f9c2e1"),
            created_at: Some("2026-09-21T14:03:55Z".to_owned()),
            members: vec![MemberView {
                member: api.clone(),
                path: WorktreePath::parse("acme/api").unwrap(),
                head: Some(Head::Branch {
                    branch: "feat-x".to_owned(),
                }),
            }],
            health: PlaneHealth {
                checked: HealthCheck::Cheap,
                findings: vec![
                    Finding::CreateNeverCompleted {
                        plane: "bp-a3f9c2e1".to_owned(),
                        started: Some("2026-09-18T09:41:07Z".to_owned()),
                        ago: Some("3 days".to_owned()),
                    },
                    Finding::MemberWorktreeMissing {
                        member: api,
                        path: WorktreePath::parse("acme/api").unwrap(),
                    },
                ],
            },
        }
    }

    fn unreadable() -> PlaneView {
        PlaneView {
            id: "broken.plane".to_owned(),
            directory: PathBuf::from("/planes/broken.plane"),
            created_at: None,
            members: Vec::new(),
            health: PlaneHealth {
                checked: HealthCheck::Cheap,
                findings: vec![Finding::Unreadable {
                    error: bitplane_core::EngineError::ParseError {
                        path: PathBuf::from("/planes/broken.plane/plane.toml"),
                        message: "unknown key \"status\"".to_owned(),
                        legal_keys: Vec::new(),
                    },
                }],
            },
        }
    }

    fn status() -> PlaneStatus {
        let gone = MemberRef::parse("@gone").unwrap();

        PlaneStatus {
            id: "bp-a3f9c2e1".to_owned(),
            directory: PathBuf::from("/planes/bp-a3f9c2e1"),
            members: vec![
                reporting("@api", "acme/api", Some("feat-login"), 3, 1, 2),
                reporting("@web", "acme/web", Some("feat-login"), 0, 0, 0),
                reporting("@docs", "acme/docs", Some("feat-login"), 0, 0, 1),
                reporting("@ops", "acme/ops", None, 0, 0, 0),
                MemberStatus {
                    member: gone.clone(),
                    path: WorktreePath::parse("acme/gone").unwrap(),
                    head: None,
                    work: MemberWork::WorktreeMissing,
                },
            ],
            health: PlaneHealth {
                checked: HealthCheck::Cheap,
                findings: vec![Finding::MemberWorktreeMissing {
                    member: gone,
                    path: WorktreePath::parse("acme/gone").unwrap(),
                }],
            },
        }
    }

    fn on_branch(member: &str, path: &str) -> MemberView {
        MemberView {
            member: MemberRef::parse(member).unwrap(),
            path: WorktreePath::parse(path).unwrap(),
            head: Some(Head::Branch {
                branch: "feat-login".to_owned(),
            }),
        }
    }

    fn reporting(
        member: &str,
        path: &str,
        branch: Option<&str>,
        modified: usize,
        untracked: usize,
        ahead: usize,
    ) -> MemberStatus {
        MemberStatus {
            member: MemberRef::parse(member).unwrap(),
            path: WorktreePath::parse(path).unwrap(),
            head: Some(match branch {
                Some(branch) => Head::Branch {
                    branch: branch.to_owned(),
                },
                None => Head::Detached {
                    commit: "9f2c1ab0d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9".to_owned(),
                },
            }),
            work: MemberWork::Reported {
                modified,
                untracked,
                ahead,
            },
        }
    }
}
