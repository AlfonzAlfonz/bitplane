//! `project_list`: every project registered on this host.
//!
//! A **read**. No lock, no file written, nothing repaired — which is what makes
//! it safe to run from a script or in a loop (ADR-0002).
//!
//! The projects directory *is* the registry: one `readdir`, and a directory
//! without a `project.toml` is not a project and is skipped. That includes the
//! leftovers of an interrupted `project add`, which hold `repo.git` and nothing
//! else.
//!
//! **A file that will not parse is a row, not a failure.** The scan continues
//! and the run reports drift, because a listing that dies on one bad file tells
//! the user nothing about the other nine — and nothing is auto-repaired, since
//! a file bitplane cannot read is a file it has no business rewriting.

use crate::directories::Directories;
use crate::error::EngineError;
use crate::outcome::PerProject;
use crate::project_dir::{self, ProjectDirectory};
use crate::project_file::ProjectFile;
use crate::wire::{ProjectListing, ProjectSummary};

/// Every project, in name order.
pub fn project_list(directories: &Directories) -> Result<ProjectListing, EngineError> {
    let projects = project_dir::registered(directories)?
        .into_iter()
        .map(|project| match project.read_project_file() {
            Ok(file) => PerProject::ok(project.name().clone(), summarise(&project, &file)),
            Err(error) => PerProject::failed(project.name().clone(), error),
        })
        .collect();

    Ok(ProjectListing { projects })
}

fn summarise(project: &ProjectDirectory, file: &ProjectFile) -> ProjectSummary {
    ProjectSummary {
        source_repo_present: project.source_repo_present(file),
        source: file.source.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::member::ProjectName;
    use crate::project_file::{PROJECT_FILE_NAME, ProjectSource};
    use crate::testing::scratch_dir;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn an_empty_registry_is_an_empty_listing_and_not_a_failure() {
        let dir = scratch_dir("project-list-empty");
        let directories = Directories::new(dir.join("planes"), dir.join("projects"));

        assert!(project_list(&directories).unwrap().projects.is_empty());
    }

    #[test]
    fn every_registered_project_is_a_row_and_a_leftover_is_not() {
        let (directories, _) = registry("project-list-rows");
        register(&directories, "api", &owned("api"));
        register(&directories, "codestyle", &owned("codestyle"));
        let leftover = ProjectDirectory::of(&directories, name("half-added"));
        fs::create_dir_all(leftover.source_repo()).unwrap();

        let listing = project_list(&directories).unwrap();

        assert_eq!(
            listing
                .projects
                .iter()
                .map(|row| row.project.to_string())
                .collect::<Vec<String>>(),
            ["api", "codestyle"]
        );
    }

    #[test]
    fn whether_the_source_repo_is_here_is_answered_by_a_stat() {
        let (directories, _) = registry("project-list-stat");
        register(&directories, "api", &owned("api"));
        register(&directories, "codestyle", &owned("codestyle"));
        fs::create_dir_all(ProjectDirectory::of(&directories, name("codestyle")).source_repo())
            .unwrap();

        let listing = project_list(&directories).unwrap();

        assert!(!listing.projects[0].value().unwrap().source_repo_present);
        assert!(listing.projects[1].value().unwrap().source_repo_present);
    }

    #[test]
    fn an_adopted_project_reports_its_checkout_as_the_source_repo() {
        let (directories, dir) = registry("project-list-adopted");
        let checkout = dir.join("projects-of-mine").join("bitplane");
        fs::create_dir_all(&checkout).unwrap();
        register(
            &directories,
            "bitplane",
            &ProjectFile::new(
                name("bitplane"),
                ProjectSource::Adopted {
                    path: checkout.clone(),
                },
            ),
        );

        let listing = project_list(&directories).unwrap();

        let summary = listing.projects[0].value().unwrap();
        assert_eq!(summary.source, ProjectSource::Adopted { path: checkout });
        assert!(summary.source_repo_present, "the checkout is right there");
    }

    #[test]
    fn a_file_that_will_not_parse_is_a_row_and_the_scan_carries_on() {
        let (directories, _) = registry("project-list-unreadable");
        register(&directories, "api", &owned("api"));
        let broken = ProjectDirectory::of(&directories, name("codestyle"));
        fs::create_dir_all(broken.path()).unwrap();
        fs::write(
            broken.project_file(),
            "version = 1\nname = \"codestyle\"\ndefault_branch = \"main\"\n",
        )
        .unwrap();
        register(&directories, "web", &owned("web"));

        let listing = project_list(&directories).unwrap();

        assert_eq!(listing.projects.len(), 3, "the scan continues");
        assert!(listing.projects[1].is_failure());
        assert!(listing.has_unreadable(), "an unreadable file is drift");
        assert!(
            broken.project_file().exists(),
            "a file bp cannot read is one it has no business rewriting"
        );
    }

    fn registry(label: &str) -> (Directories, PathBuf) {
        let dir = scratch_dir(label);
        let directories = Directories::new(dir.join("planes"), dir.join("projects"));
        fs::create_dir_all(directories.projects()).unwrap();
        (directories, dir)
    }

    fn register(directories: &Directories, of: &str, file: &ProjectFile) {
        let project = ProjectDirectory::of(directories, name(of));
        fs::create_dir_all(project.path()).unwrap();
        fs::write(project.path().join(PROJECT_FILE_NAME), file.render()).unwrap();
    }

    fn name(name: &str) -> ProjectName {
        ProjectName::parse(name).unwrap()
    }

    fn owned(of: &str) -> ProjectFile {
        ProjectFile::new(
            name(of),
            ProjectSource::Owned {
                url: format!("git@gitlab.com:acme/{of}.git"),
            },
        )
    }
}
