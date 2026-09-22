//! The engine that answers for this machine.

use std::path::{Path, PathBuf};

use crate::add::{self, AddContext};
use crate::create::{self, CreateContext};
use crate::destroy::{self, TeardownContext};
use crate::directories::{Directories, Environment, SystemEnvironment};
use crate::engine::{Engine, Reader};
use crate::error::EngineError;
use crate::git::{GitPrerequisite, GitVersion, SystemGit};
use crate::interrupt::Interrupt;
use crate::project_add::{self, ProjectContext};
use crate::project_fetch::{self, FetchContext};
use crate::project_list;
use crate::read::{self, ReadContext};
use crate::repo::Git;
use crate::wire::{
    PlaneAddRequest, PlaneAdded, PlaneCreateRequest, PlaneCreated, PlaneDestroyRequest,
    PlaneDestroyed, PlaneList, PlaneListRequest, PlaneRemoveRequest, PlaneRemoved,
    PlaneShowRequest, PlaneStatus, PlaneStatusRequest, PlaneView, ProjectAddRequest, ProjectAdded,
    ProjectFetchRequest, ProjectFetched, ProjectListing,
};

/// bitplane on the local host.
///
/// Holds the git version check, which is why it is constructed once per process
/// and shared: the check runs on first use and never again (ADR-0001).
pub struct LocalEngine {
    directories: Directories,
    prerequisite: GitPrerequisite<SystemGit>,
    git: Git,
    home: Option<PathBuf>,
    interrupt: Interrupt,
    on_lock_wait: Box<dyn Fn(&Path) + Send + Sync>,
}

impl LocalEngine {
    /// An engine over `directories`, using the git on the process's own `PATH`.
    pub fn new(directories: Directories) -> LocalEngine {
        LocalEngine {
            directories,
            prerequisite: GitPrerequisite::new(SystemGit::new()),
            git: Git::new(),
            home: SystemEnvironment.home(),
            interrupt: Interrupt::never(),
            on_lock_wait: Box::new(|_| {}),
        }
    }

    /// Uses the git on `search_path` instead of the process's own — the seam a
    /// test drives the version check through without touching the ambient
    /// environment.
    pub fn with_git_search_path(mut self, search_path: impl Into<PathBuf>) -> LocalEngine {
        let search_path = search_path.into();
        self.prerequisite = GitPrerequisite::new(SystemGit::on_path(&search_path));
        self.git = Git::on_path(search_path);
        self
    }

    /// Honours `interrupt` while fanning out. Without one, nothing stops a run
    /// early.
    pub fn with_interrupt(mut self, interrupt: Interrupt) -> LocalEngine {
        self.interrupt = interrupt;
        self
    }

    /// What to say when a lock is contended. The engine cannot print; this is
    /// how the surface does it without the common case saying anything.
    pub fn announcing_lock_waits(
        mut self,
        announce: impl Fn(&Path) + Send + Sync + 'static,
    ) -> LocalEngine {
        self.on_lock_wait = Box::new(announce);
        self
    }

    /// What a member path's leading `~` means.
    pub fn with_home(mut self, home: Option<PathBuf>) -> LocalEngine {
        self.home = home;
        self
    }

    /// Fails unless there is a git new enough to run bitplane. Every command
    /// goes through here before it touches anything.
    pub fn ensure_git_supported(&self) -> Result<GitVersion, EngineError> {
        self.prerequisite.ensure_supported()
    }

    /// The two directories this engine answers over.
    pub fn directories(&self) -> &Directories {
        &self.directories
    }

    fn reading(&self) -> ReadContext<'_> {
        ReadContext {
            directories: &self.directories,
            git: &self.git,
        }
    }
}

impl Reader for LocalEngine {
    fn plane_list(&self, request: PlaneListRequest) -> Result<PlaneList, EngineError> {
        read::plane_list(&request, &self.reading())
    }

    fn plane_show(&self, request: PlaneShowRequest) -> Result<PlaneView, EngineError> {
        read::plane_show(&request, &self.reading())
    }

    fn plane_status(&self, request: PlaneStatusRequest) -> Result<PlaneStatus, EngineError> {
        read::plane_status(&request, &self.reading())
    }

    fn project_list(&self) -> Result<ProjectListing, EngineError> {
        project_list::project_list(&self.directories)
    }
}

impl LocalEngine {
    /// What the teardown verbs need that is not in the request. Both of them
    /// take the same context, because they carry the same rules.
    fn teardown(&self) -> TeardownContext<'_> {
        TeardownContext {
            directories: &self.directories,
            git: &self.git,
            home: self.home.as_deref(),
            interrupt: self.interrupt,
            on_lock_wait: &*self.on_lock_wait,
        }
    }
}

impl Engine for LocalEngine {
    fn plane_create(&self, request: PlaneCreateRequest) -> Result<PlaneCreated, EngineError> {
        create::plane_create(
            &request,
            &CreateContext {
                directories: &self.directories,
                git: &self.git,
                home: self.home.as_deref(),
                interrupt: self.interrupt,
                on_lock_wait: &*self.on_lock_wait,
            },
        )
    }

    fn plane_add(&self, request: PlaneAddRequest) -> Result<PlaneAdded, EngineError> {
        add::plane_add(
            &request,
            &AddContext {
                directories: &self.directories,
                git: &self.git,
                home: self.home.as_deref(),
                interrupt: self.interrupt,
                on_lock_wait: &*self.on_lock_wait,
            },
        )
    }

    fn project_add(&self, request: ProjectAddRequest) -> Result<ProjectAdded, EngineError> {
        project_add::project_add(
            &request,
            &ProjectContext {
                directories: &self.directories,
                git: &self.git,
                on_lock_wait: &*self.on_lock_wait,
            },
        )
    }

    fn project_fetch(&self, request: ProjectFetchRequest) -> Result<ProjectFetched, EngineError> {
        project_fetch::project_fetch(
            &request,
            &FetchContext {
                directories: &self.directories,
                git: &self.git,
                interrupt: self.interrupt,
                on_lock_wait: &*self.on_lock_wait,
            },
        )
    }

    fn plane_destroy(&self, request: PlaneDestroyRequest) -> Result<PlaneDestroyed, EngineError> {
        destroy::plane_destroy(&request, &self.teardown())
    }

    fn plane_remove(&self, request: PlaneRemoveRequest) -> Result<PlaneRemoved, EngineError> {
        destroy::plane_remove(&request, &self.teardown())
    }
}

impl std::fmt::Debug for LocalEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalEngine")
            .field("directories", &self.directories)
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}
