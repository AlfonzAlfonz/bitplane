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

use bitplane_core::{
    CreatedMember, ErrorEnvelope, Outcome, PerMember, PlaneCreated, Problem, Response,
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
    let rows: Vec<[String; 4]> = created.members.iter().map(member_row).collect();

    let mut text = format!(
        "{}  {}\n\n{}",
        created.id,
        created.directory.display(),
        columns(&rows, "  ")
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

/// `<member>  <branch>  <outcome>  <path>`.
fn member_row(row: &PerMember<CreatedMember>) -> [String; 4] {
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

    [row.member.to_string(), branch, outcome, path]
}

fn human_envelope(envelope: &ErrorEnvelope) -> String {
    let mut text = format!("error[{}]: {}\n", envelope.error, envelope.message);

    // Three things are omitted rather than rendered empty: an envelope with no
    // problems prints no indented block, a problem with no subject leaves the
    // column blank, and an absent remedy prints no line.
    if !envelope.problems.is_empty() {
        let rows: Vec<[String; 2]> = envelope.problems.iter().map(problem_row).collect();
        text.push('\n');
        text.push_str(&columns(&rows, "  "));
    }

    if let Some(remedy) = &envelope.remedy {
        text.push_str(&format!("\nremedy: {remedy}\n"));
    }

    text
}

fn problem_row(problem: &Problem) -> [String; 2] {
    [
        problem.subject.clone().unwrap_or_default(),
        problem.message.clone(),
    ]
}

/// Indented rows with every column but the last padded to its widest cell —
/// the one grammar every fan-out and every problem block uses.
fn columns<const N: usize>(rows: &[[String; N]], separator: &str) -> String {
    let widths: Vec<usize> = (0..N)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();

    rows.iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(column, cell)| {
                    if column + 1 == N {
                        cell.clone()
                    } else {
                        let padding = widths[column] - cell.chars().count();
                        format!("{cell}{}", " ".repeat(padding))
                    }
                })
                .collect();

            format!("  {}\n", cells.join(separator).trim_end())
        })
        .collect()
}

/// What a column holds when there is nothing to report there.
fn dash() -> String {
    "-".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitplane_core::{MemberRef, PlaneId, SkipReason, WorktreePath};
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
}
