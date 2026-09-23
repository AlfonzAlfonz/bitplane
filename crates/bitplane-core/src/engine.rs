//! The two traits every mutation and every read passes through (ADR-0003).
//!
//! The split into [`Reader`] and [`Engine`] exists so ADR-0002's *"a read never
//! writes"* is checkable by reading the trait rather than by auditing every
//! body. In Rust both reads and mutations take `&self` — the state is the
//! filesystem — so `&self`/`&mut self` cannot express the distinction.
//!
//! Sync throughout, per ADR-0001: the whole workload is spawn-subprocess-and-wait
//! plus small TOML reads and writes, so there is no IO multiplexing problem and
//! async would colour every caller for nothing.
//!
//! There is no `host` parameter. The host is chosen by picking which engine you
//! hold — [`crate::LocalEngine`] now, an SSH engine later.
//!
//! Actions arrive one slice at a time, each landing on the trait, on
//! [`crate::Request`] and in [`crate::dispatch`] together.

use crate::error::EngineError;
use crate::wire::{
    PlaneAddRequest, PlaneAdded, PlaneCreateRequest, PlaneCreated, PlaneDestroyRequest,
    PlaneDestroyed, PlaneList, PlaneListRequest, PlaneRemoveRequest, PlaneRemoved,
    PlaneScriptsRequest, PlaneShowRequest, PlaneStatus, PlaneStatusRequest, PlaneView,
    ProjectAddRequest, ProjectAdded, ProjectFetchRequest, ProjectFetched, ProjectListing,
    ScriptsRun,
};

/// The reads: `plane_list`, `plane_show`, `plane_status`, `project_list`,
/// `project_show` and `doctor`. None of them takes a lock or creates a file.
pub trait Reader {
    /// Every plane on this host, with its members, their live branches and its
    /// health.
    ///
    /// A directory without a plane file is silently skipped; one whose plane
    /// file does not parse is a row in an error state, and the scan continues.
    /// Nothing is ever auto-repaired: a file bitplane cannot read is one it has
    /// no business rewriting.
    fn plane_list(&self, request: PlaneListRequest) -> Result<PlaneList, EngineError>;

    /// One plane, resolved by id or by the directory the caller is standing in.
    ///
    /// Unlike [`Reader::plane_list`], there is exactly one file to read, so
    /// failing to read it is a failure rather than a row.
    fn plane_show(&self, request: PlaneShowRequest) -> Result<PlaneView, EngineError>;

    /// git's own status across every member of one plane.
    ///
    /// Reading is allowed; writing is not. This stores nothing, so what it
    /// reports cannot go stale — it is git's answer, rendered (ADR-0006).
    fn plane_status(&self, request: PlaneStatusRequest) -> Result<PlaneStatus, EngineError>;

    /// Every project registered on this host.
    ///
    /// One `readdir` and one small file per project. A file that will not parse
    /// is a row in an error state and the scan continues, because a listing
    /// that dies on one bad file says nothing about the other nine.
    fn project_list(&self) -> Result<ProjectListing, EngineError>;
}

/// The reads, plus every mutation.
pub trait Engine: Reader {
    /// Makes a plane and a worktree of every member named.
    ///
    /// Strict, not idempotent: it either makes a whole plane or it makes none.
    /// A failure returns `Err` carrying the per-member rows, because the
    /// envelope `Err` is for an operation that produced no durable state.
    fn plane_create(&self, request: PlaneCreateRequest) -> Result<PlaneCreated, EngineError>;

    /// Puts the members named into a plane that already exists.
    ///
    /// **Cannot abort-and-remove**, because the plane holds other members full
    /// of work: a failure force-removes only the worktrees this run created,
    /// drops only its own entries from the plane file, and leaves the plane as
    /// it found it. It never sets the incomplete latch, under any failure —
    /// that marker means *nothing in here is yours*, and a plane holding the
    /// user's work carrying it would be a trapdoor (ADR-0004).
    fn plane_add(&self, request: PlaneAddRequest) -> Result<PlaneAdded, EngineError>;

    /// Registers a project from a URL, building the source repo bitplane owns.
    ///
    /// A failure keeps the object store and unwinds only the registration: the
    /// cold fetch is the expensive step, and `init --bare` + `fetch` is
    /// resumable in a way `clone` is not (ADR-0005).
    fn project_add(&self, request: ProjectAddRequest) -> Result<ProjectAdded, EngineError>;

    /// Brings owned projects' source repos up to date with their forges.
    ///
    /// A fan-out: a forge that will not answer is one row, and every other
    /// project still gets its turn.
    fn project_fetch(&self, request: ProjectFetchRequest) -> Result<ProjectFetched, EngineError>;

    /// Takes a whole plane apart, and refuses over work that would be lost.
    ///
    /// **Converges.** You cannot un-remove a worktree, so there is no rollback:
    /// an interrupted or half-failed run is finished by running it again, with
    /// the refusals re-checked against what survived and the members already
    /// gone reported `AlreadyDone` (ADR-0004).
    fn plane_destroy(&self, request: PlaneDestroyRequest) -> Result<PlaneDestroyed, EngineError>;

    /// Takes the members named out of a plane, leaving the rest of it alone.
    ///
    /// Carries **rules identical** to [`Engine::plane_destroy`]: removing a
    /// member destroys exactly as much work as destroying a one-member plane,
    /// so a lighter rule here would be a hole in that one (ADR-0006).
    fn plane_remove(&self, request: PlaneRemoveRequest) -> Result<PlaneRemoved, EngineError>;

    /// Runs a project's declared scripts, by name, on demand.
    ///
    /// Here and not on [`Reader`] because it runs arbitrary user commands and
    /// writes logs, and *"a read never writes"* has to stay checkable by
    /// reading the trait (ADR-0003). It refuses on a latched plane: acting on
    /// one is meaningless work on a thing headed for deletion.
    fn plane_scripts(&self, request: PlaneScriptsRequest) -> Result<ScriptsRun, EngineError>;
}
