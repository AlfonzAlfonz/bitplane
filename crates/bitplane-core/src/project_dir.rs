//! The project directory: the registry, one directory at a time.
//!
//! `<projects-dir>/<name>/` holds `project.toml`, the source repo at
//! `repo.git` for an **owned** project, and `bin/` — which is the user's to
//! create and which bitplane never writes to, because that is what keeps the
//! shell-alias trust posture true.
//!
//! **A project is exactly a directory containing `project.toml`**, the mirror
//! of "a plane is exactly a directory containing a plane file". That is what
//! makes the set of registered projects self-describing: there is no registry
//! file, no index and no database (ADR-0002). A directory holding a `repo.git`
//! and no `project.toml` — what an interrupted `project add` leaves — is
//! therefore simply not a project, and nothing lists it.
//!
//! **One sentinel per project directory**, at `.bitplane/lock`, guarding both
//! `project.toml` and every git command that writes to the source repo. On the
//! sentinel and never on the data file, because writing by atomic rename
//! replaces the inode (ADR-0002).
//!
//! Unlike a plane directory, this one is **not claimed by atomic `mkdir`**. A
//! failed `project add` keeps its object store so the retry can reuse it
//! (ADR-0005), so an existing directory is a resumption rather than a
//! collision; what makes the name unique is `project.toml`, checked under the
//! lock.

use std::fs;
use std::path::{Path, PathBuf};

use crate::directories::Directories;
use crate::error::EngineError;
use crate::fsio::{self, write_atomically};
use crate::lock::{self, Lock};
use crate::member::{ProjectName, RESERVED_SEGMENT};
use crate::project_file::{PROJECT_FILE_NAME, ProjectFile};

/// The bare source repo of an **owned** project, inside its project directory.
pub const REPO_DIR_NAME: &str = "repo.git";

/// The directory a project ships its own executables in. Prepended to `PATH`
/// for that project's scripts; never written to by bitplane.
pub const BIN_DIR_NAME: &str = "bin";

/// One project's directory, whether or not it is a project yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDirectory {
    name: ProjectName,
    path: PathBuf,
}

impl ProjectDirectory {
    /// The directory `name` would occupy on this host.
    pub fn of(directories: &Directories, name: ProjectName) -> ProjectDirectory {
        let path = directories.projects().join(name.as_str());

        ProjectDirectory { name, path }
    }

    pub fn name(&self) -> &ProjectName {
        &self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where `project.toml` is.
    pub fn project_file(&self) -> PathBuf {
        self.path.join(PROJECT_FILE_NAME)
    }

    /// The bare source repo bitplane builds for an **owned** project. An
    /// adopted project's source repo is the user's checkout and is not in here.
    pub fn source_repo(&self) -> PathBuf {
        self.path.join(REPO_DIR_NAME)
    }

    /// Whether this directory is a project — which is exactly whether it holds
    /// a `project.toml`.
    pub fn is_registered(&self) -> bool {
        self.project_file().exists()
    }

    /// Creates the directory, and the projects directory above it, if they are
    /// not there. An existing directory is reused: a failed `project add` keeps
    /// its objects for exactly that.
    pub fn make(&self, directories: &Directories) -> Result<(), EngineError> {
        fs::create_dir_all(&self.path).map_err(|err| EngineError::io(&self.path, err))?;

        fsio::sync_directory(directories.projects())
    }

    /// Takes the project's lock, on the sentinel rather than on any file it
    /// guards.
    pub fn lock(&self, on_contended: impl FnOnce(&Path)) -> Result<Lock, EngineError> {
        let bitplane = self.path.join(RESERVED_SEGMENT);
        fs::create_dir_all(&bitplane).map_err(|err| EngineError::io(&bitplane, err))?;

        Lock::acquire(
            &bitplane.join(lock::SENTINEL_NAME),
            lock::DEFAULT_TIMEOUT,
            on_contended,
        )
    }

    /// Writes the registration. The last step of `add` and of `adopt`: until it
    /// lands, the directory is not a project.
    pub fn write_project_file(&self, file: &ProjectFile) -> Result<(), EngineError> {
        write_atomically(&self.project_file(), &file.render())
    }

    /// Reads the registration.
    pub fn read_project_file(&self) -> Result<ProjectFile, EngineError> {
        ProjectFile::read(&self.project_file())
    }

    /// Whether the source repo this project names is on this host.
    ///
    /// One `stat`, never a stored field: "is it cloned here?" is a question the
    /// filesystem answers and a `project.toml` could only answer stalely
    /// (ADR-0007).
    pub fn source_repo_present(&self, file: &ProjectFile) -> bool {
        source_repo_of(self, file).is_dir()
    }
}

/// The repo a project's worktrees come from: `repo.git` when bitplane built it,
/// the user's own checkout when it merely points at one.
pub fn source_repo_of(directory: &ProjectDirectory, file: &ProjectFile) -> PathBuf {
    match &file.source {
        crate::project_file::ProjectSource::Owned { .. } => directory.source_repo(),
        crate::project_file::ProjectSource::Adopted { path } => path.clone(),
    }
}

/// Every project registered on this host, in name order.
///
/// One `readdir`, and nothing else: a directory without a `project.toml` is not
/// a project and is skipped, which is what makes the leftovers of an
/// interrupted `add` invisible here and `doctor`'s business instead.
///
/// A projects directory that does not exist yet is an empty registry rather
/// than a failure — nothing has been registered, which is not an error.
pub fn registered(directories: &Directories) -> Result<Vec<ProjectDirectory>, EngineError> {
    let entries = match fs::read_dir(directories.projects()) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(EngineError::io(directories.projects(), err)),
    };

    let mut projects: Vec<ProjectDirectory> = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|err| EngineError::io(directories.projects(), err))?;

        // The directory name *is* the name, so a directory whose name is not a
        // project name cannot be a project however it is furnished.
        let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|name| ProjectName::parse(name).ok())
        else {
            continue;
        };

        let project = ProjectDirectory::of(directories, name);
        if project.is_registered() {
            projects.push(project);
        }
    }

    projects.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(projects)
}

/// The project called `name`, or [`EngineError::ProjectNotFound`].
pub fn find(
    directories: &Directories,
    name: &ProjectName,
) -> Result<ProjectDirectory, EngineError> {
    let project = ProjectDirectory::of(directories, name.clone());

    if project.is_registered() {
        Ok(project)
    } else {
        Err(EngineError::ProjectNotFound {
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_file::ProjectSource;
    use crate::testing::scratch_dir;

    #[test]
    fn a_directory_holding_a_project_file_is_a_project_and_one_without_is_not() {
        let (directories, _) = somewhere("project-dir-registered");
        let project = ProjectDirectory::of(&directories, name("codestyle"));

        project.make(&directories).unwrap();
        fs::create_dir_all(project.source_repo()).unwrap();
        assert!(
            !project.is_registered(),
            "the leftovers of a failed add are not a project"
        );

        project.write_project_file(&owned("codestyle")).unwrap();
        assert!(project.is_registered());
    }

    #[test]
    fn the_registry_is_the_directory_and_a_leftover_is_not_in_it() {
        let (directories, _) = somewhere("project-dir-listing");

        for registered_name in ["api", "codestyle"] {
            let project = ProjectDirectory::of(&directories, name(registered_name));
            project.make(&directories).unwrap();
            project.write_project_file(&owned(registered_name)).unwrap();
        }
        let leftover = ProjectDirectory::of(&directories, name("half-added"));
        leftover.make(&directories).unwrap();
        fs::create_dir_all(leftover.source_repo()).unwrap();

        let found = registered(&directories).unwrap();

        assert_eq!(
            found
                .iter()
                .map(|project| project.name().to_string())
                .collect::<Vec<String>>(),
            ["api", "codestyle"]
        );
    }

    #[test]
    fn a_projects_directory_that_does_not_exist_yet_is_an_empty_registry() {
        let dir = scratch_dir("project-dir-absent");
        let directories = Directories::new(dir.join("planes"), dir.join("never-created"));

        assert!(registered(&directories).unwrap().is_empty());
    }

    #[test]
    fn a_directory_whose_name_is_not_a_project_name_is_not_a_project() {
        let (directories, _) = somewhere("project-dir-bad-name");
        let odd = directories.projects().join("Codestyle");
        fs::create_dir_all(&odd).unwrap();
        fs::write(odd.join(PROJECT_FILE_NAME), owned("codestyle").render()).unwrap();

        assert!(registered(&directories).unwrap().is_empty());
    }

    #[test]
    fn the_lock_is_taken_on_the_sentinel_not_on_the_project_file() {
        let (directories, _) = somewhere("project-dir-lock");
        let project = ProjectDirectory::of(&directories, name("codestyle"));
        project.make(&directories).unwrap();

        let lock = project.lock(|_| {}).unwrap();

        assert_eq!(
            lock.object(),
            project.path().join(RESERVED_SEGMENT).join("lock")
        );
        assert!(!lock.object().ends_with(PROJECT_FILE_NAME));
    }

    #[test]
    fn a_name_that_is_not_registered_is_not_found() {
        let (directories, _) = somewhere("project-dir-find");

        let error = find(&directories, &name("codestyle")).unwrap_err();

        assert_eq!(
            error,
            EngineError::ProjectNotFound {
                name: "codestyle".to_owned()
            }
        );
    }

    #[test]
    fn an_owned_source_repo_is_inside_the_project_and_an_adopted_one_is_not() {
        let (directories, _) = somewhere("project-dir-source-repo");
        let project = ProjectDirectory::of(&directories, name("codestyle"));

        assert_eq!(
            source_repo_of(&project, &owned("codestyle")),
            project.path().join(REPO_DIR_NAME)
        );
        assert_eq!(
            source_repo_of(
                &project,
                &ProjectFile::new(
                    name("codestyle"),
                    ProjectSource::Adopted {
                        path: PathBuf::from("/Users/alfonz/projects/codestyle"),
                    },
                )
            ),
            PathBuf::from("/Users/alfonz/projects/codestyle")
        );
    }

    fn somewhere(label: &str) -> (Directories, PathBuf) {
        let dir = scratch_dir(label);
        let directories = Directories::new(dir.join("planes"), dir.join("projects"));
        fs::create_dir_all(directories.projects()).unwrap();
        (directories, dir)
    }

    fn name(name: &str) -> ProjectName {
        ProjectName::parse(name).unwrap()
    }

    fn owned(name_of: &str) -> ProjectFile {
        ProjectFile::new(
            name(name_of),
            ProjectSource::Owned {
                url: format!("git@gitlab.com:acme/{name_of}.git"),
            },
        )
    }
}
