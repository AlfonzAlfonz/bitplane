//! The plane directory: claiming it, latching it, and taking it back.
//!
//! Its name **is** the plane's identity — the directory is the key, so renaming
//! a plane is moving it — and it is claimed by **atomic `mkdir(2)`**, which
//! fails `EEXIST`. Never probe first: an `exists()`-then-create is a TOCTOU that
//! two concurrent `bp create`s both pass (ADR-0002).
//!
//! `.bitplane/` at the root is reserved for bitplane's own per-plane files: the
//! lock sentinel and the incomplete latch. It sits outside every repo, so
//! nothing in it can be committed by accident.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::directories::Directories;
use crate::error::{EngineError, Occupant};
use crate::fsio::{self, write_atomically};
use crate::lock::{self, Lock};
use crate::plane_file::{PLANE_FILE_NAME, PlaneFile};
use crate::plane_id::{GENERATED_ID_ATTEMPTS, PlaneId};
use crate::time::Rfc3339;

/// bitplane's own directory at the root of a plane.
pub const BITPLANE_DIR: &str = ".bitplane";

/// The one-way latch: present exactly while a plane has been claimed but never
/// finished being created.
pub const LATCH_NAME: &str = "incomplete";

/// A plane directory this process claimed.
#[derive(Debug)]
pub struct ClaimedPlane {
    id: PlaneId,
    path: PathBuf,
}

impl ClaimedPlane {
    /// Claims the directory for a **user-supplied** id.
    ///
    /// `EEXIST` is fatal and immediate: `create` is strict, not idempotent, and
    /// re-running it must never silently adopt an unrelated healthy plane. The
    /// path is stat'd and classified so the remedy names what is actually there.
    pub fn claim(directories: &Directories, id: PlaneId) -> Result<ClaimedPlane, EngineError> {
        let path = directories.plane(&id);

        match mkdir(directories.planes(), &path) {
            Ok(()) => Ok(ClaimedPlane { id, path }),
            Err(Claim::Taken) => Err(EngineError::PlaneIdInUse {
                id: id.to_string(),
                found: classify(&path),
            }),
            Err(Claim::Refused(error)) => Err(error),
        }
    }

    /// Claims a directory under a generated id, retrying on collision.
    ///
    /// Bounded at [`GENERATED_ID_ATTEMPTS`]: needing a sixth means something
    /// other than collision is wrong, and a retry loop that never gives up
    /// would hide it.
    pub fn claim_generated(directories: &Directories) -> Result<ClaimedPlane, EngineError> {
        ClaimedPlane::claim_generated_by(directories, PlaneId::generate)
    }

    /// [`ClaimedPlane::claim_generated`], with the generator named.
    ///
    /// The seam exists for the bound: a test cannot make `/dev/urandom` repeat
    /// itself, and "gives up after five" is the behaviour worth pinning.
    pub fn claim_generated_by(
        directories: &Directories,
        mut generate: impl FnMut() -> Result<PlaneId, EngineError>,
    ) -> Result<ClaimedPlane, EngineError> {
        let mut last = None;

        for _ in 0..GENERATED_ID_ATTEMPTS {
            let id = generate()?;
            let path = directories.plane(&id);

            match mkdir(directories.planes(), &path) {
                Ok(()) => return Ok(ClaimedPlane { id, path }),
                Err(Claim::Taken) => last = Some(id),
                Err(Claim::Refused(error)) => return Err(error),
            }
        }

        Err(EngineError::PlaneIdInUse {
            id: last.map(|id| id.to_string()).unwrap_or_default(),
            found: Occupant::Plane,
        })
    }

    pub fn id(&self) -> &PlaneId {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Takes the plane's lock.
    ///
    /// On the **sentinel**, never on `plane.toml`: writing by atomic rename
    /// replaces the inode, so a lock on the data file would guard a file that
    /// no longer exists (ADR-0002).
    pub fn lock(&self, on_contended: impl FnOnce(&Path)) -> Result<Lock, EngineError> {
        lock_plane(&self.path, on_contended)
    }

    /// Writes the latch, **before** the plane file.
    ///
    /// It holds the timestamp of the claim and nothing else, and it is
    /// diagnostic only: nothing parses it for control flow, so a corrupt or
    /// empty file still means "incomplete" (ADR-0004).
    pub fn latch(&self) -> Result<(), EngineError> {
        self.make_bitplane_dir()?;
        write_atomically(&self.latch_path(), &format!("{}\n", Rfc3339::now()))
    }

    /// Clears the latch — the point of no return, once the last worktree lands.
    ///
    /// A one-way latch: set once at birth, cleared once, never set again. A
    /// crash before clearing leaves it set, which is the truthful reading, so it
    /// can only fail in the safe direction.
    pub fn unlatch(&self) -> Result<(), EngineError> {
        let path = self.latch_path();

        match fs::remove_file(&path) {
            Ok(()) => fsio::sync_directory(&self.path.join(BITPLANE_DIR)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(EngineError::io(&path, err)),
        }
    }

    /// Writes the membership.
    pub fn write_plane_file(&self, file: &PlaneFile) -> Result<(), EngineError> {
        write_atomically(&self.path.join(PLANE_FILE_NAME), &file.render())
    }

    /// Throws the whole claim away.
    ///
    /// Only ever called inside the abort window, which by construction holds
    /// nothing the user has touched.
    pub fn discard(&self) -> Result<(), EngineError> {
        match fs::remove_dir_all(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(EngineError::io(&self.path, err)),
        }
    }

    fn latch_path(&self) -> PathBuf {
        latch_path(&self.path)
    }

    fn make_bitplane_dir(&self) -> Result<(), EngineError> {
        make_bitplane_dir(&self.path)
    }
}

/// A plane directory that is already there.
///
/// The counterpart to [`ClaimedPlane`]: `create` claims a directory that must
/// not exist, everything else opens one that must. Opening takes no lock and
/// creates nothing — the caller decides whether it is about to write.
#[derive(Debug, Clone)]
pub struct OpenPlane {
    id: String,
    path: PathBuf,
}

impl OpenPlane {
    /// The plane at an **already resolved** directory.
    ///
    /// Resolution itself is [`crate::read::resolve`]'s, not this type's: the
    /// walk up to `plane.toml` is the same one `bp show` and `bp status` do, and
    /// two implementations of "which plane is this?" would be two answers.
    pub fn at(path: PathBuf) -> OpenPlane {
        // The **directory's name**, which is the plane's identity — and held as
        // a string rather than a `PlaneId` for the reason `read::named` gives:
        // a directory a user made by hand need not be a well-formed id, and
        // refusing to name one would make it impossible to destroy.
        let id = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        OpenPlane { id, path }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Takes the plane's lock, on the sentinel rather than on `plane.toml`.
    pub fn lock(&self, on_contended: impl FnOnce(&Path)) -> Result<Lock, EngineError> {
        lock_plane(&self.path, on_contended)
    }

    /// Whether `create` claimed this plane and never finished.
    ///
    /// The latch means *nothing in here is yours*, which is what licenses a
    /// `destroy` with no refusal checks at all — and what makes every other
    /// verb decline and point at that one.
    pub fn is_latched(&self) -> bool {
        latch_path(&self.path).exists()
    }

    pub fn plane_file_path(&self) -> PathBuf {
        self.path.join(PLANE_FILE_NAME)
    }

    /// The membership, where there is a file holding one.
    ///
    /// Absent rather than an error for a claimed directory with no plane file:
    /// that is a state `create` can leave behind, and `destroy` has to be able
    /// to clear it.
    pub fn read_plane_file(&self) -> Result<Option<PlaneFile>, EngineError> {
        let path = self.plane_file_path();

        if !path.exists() {
            return Ok(None);
        }

        PlaneFile::read(&path).map(Some)
    }

    /// Writes the membership back, atomically.
    pub fn write_plane_file_text(&self, text: &str) -> Result<(), EngineError> {
        write_atomically(&self.plane_file_path(), text)
    }

    /// Unlinks `plane.toml` — the **last** thing removed, so the window in
    /// which a plane is visible strictly contains the window in which it
    /// exists (ADR-0004).
    pub fn remove_plane_file(&self) -> Result<(), EngineError> {
        let path = self.plane_file_path();

        match fs::remove_file(&path) {
            Ok(()) => fsio::sync_directory(&self.path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(EngineError::io(&path, err)),
        }
    }

    /// Removes the plane directory and everything left in it.
    ///
    /// Everything: anything no member names — scratch files, a notes file —
    /// goes with it. The plane directory is the user's between `create` and
    /// `destroy`, and `destroy` is the end of that.
    pub fn remove_directory(&self) -> Result<(), EngineError> {
        match fs::remove_dir_all(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(EngineError::io(&self.path, err)),
        }
    }
}

fn lock_plane(plane: &Path, on_contended: impl FnOnce(&Path)) -> Result<Lock, EngineError> {
    make_bitplane_dir(plane)?;
    Lock::acquire(
        &plane.join(BITPLANE_DIR).join(lock::SENTINEL_NAME),
        lock::DEFAULT_TIMEOUT,
        on_contended,
    )
}

fn latch_path(plane: &Path) -> PathBuf {
    plane.join(BITPLANE_DIR).join(LATCH_NAME)
}

fn make_bitplane_dir(plane: &Path) -> Result<(), EngineError> {
    let path = plane.join(BITPLANE_DIR);

    fs::create_dir_all(&path).map_err(|err| EngineError::io(&path, err))
}

/// What is at a plane directory that could not be claimed.
///
/// One `stat` of a path bitplane already knows, so the remedy can say *"exists
/// but was never completed"* rather than the useless "already in use".
pub fn classify(path: &Path) -> Occupant {
    if path.join(BITPLANE_DIR).join(LATCH_NAME).exists() {
        Occupant::LatchedRemnant
    } else if path.join(PLANE_FILE_NAME).exists() {
        Occupant::Plane
    } else {
        Occupant::ClaimWithoutPlaneFile
    }
}

/// Why a claim did not land.
enum Claim {
    /// Something is already there.
    Taken,
    /// The filesystem said no.
    Refused(EngineError),
}

/// The claim itself: one atomic `mkdir`, and the planes directory `fsync`'d so
/// the claim survives a crash.
fn mkdir(planes: &Path, plane: &Path) -> Result<(), Claim> {
    fs::create_dir_all(planes).map_err(|err| Claim::Refused(EngineError::io(planes, err)))?;

    match fs::create_dir(plane) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Err(Claim::Taken),
        Err(err) => return Err(Claim::Refused(EngineError::io(plane, err))),
    }

    fsio::sync_directory(planes).map_err(Claim::Refused)
}

/// The timeout a lock on a plane sentinel is given.
pub const LOCK_TIMEOUT: Duration = lock::DEFAULT_TIMEOUT;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_dir;

    #[test]
    fn a_claim_creates_the_directory_the_id_names() {
        let (directories, _) = somewhere("claim");
        let id = PlaneId::parse("auth-work").unwrap();

        let claimed = ClaimedPlane::claim(&directories, id).unwrap();

        assert!(claimed.path().is_dir());
        assert_eq!(claimed.path(), directories.planes().join("auth-work"));
    }

    #[test]
    fn a_second_claim_on_the_same_id_fails_immediately_saying_what_is_there() {
        let (directories, _) = somewhere("claim-twice");
        let id = PlaneId::parse("auth-work").unwrap();

        let first = ClaimedPlane::claim(&directories, id.clone()).unwrap();
        first.latch().unwrap();
        first
            .write_plane_file(&PlaneFile::new(id.clone(), Vec::new()))
            .unwrap();

        let error = ClaimedPlane::claim(&directories, id).unwrap_err();

        assert_eq!(
            error,
            EngineError::PlaneIdInUse {
                id: "auth-work".to_owned(),
                found: Occupant::LatchedRemnant,
            }
        );
        assert_eq!(error.exit_code(), crate::ExitCode::Usage);
    }

    #[test]
    fn each_thing_that_can_be_at_a_claimed_path_is_classified_apart() {
        let dir = scratch_dir("classify");

        let bare = dir.join("bare");
        fs::create_dir(&bare).unwrap();
        assert_eq!(classify(&bare), Occupant::ClaimWithoutPlaneFile);

        let healthy = dir.join("healthy");
        fs::create_dir(&healthy).unwrap();
        fs::write(healthy.join(PLANE_FILE_NAME), "version = 1\n").unwrap();
        assert_eq!(classify(&healthy), Occupant::Plane);

        let latched = dir.join("latched");
        fs::create_dir_all(latched.join(BITPLANE_DIR)).unwrap();
        fs::write(latched.join(BITPLANE_DIR).join(LATCH_NAME), "").unwrap();
        fs::write(latched.join(PLANE_FILE_NAME), "version = 1\n").unwrap();
        assert_eq!(
            classify(&latched),
            Occupant::LatchedRemnant,
            "a latch outranks a plane file: the plane never worked"
        );
    }

    #[test]
    fn a_generated_claim_lands_under_a_generated_id() {
        let (directories, _) = somewhere("claim-generated");

        let claimed = ClaimedPlane::claim_generated(&directories).unwrap();

        assert!(claimed.id().is_generated(), "got {}", claimed.id());
        assert!(claimed.path().is_dir());
    }

    #[test]
    fn a_generated_claim_retries_a_collision_and_keeps_the_id_that_landed() {
        let (directories, _) = somewhere("claim-generated-retry");
        fs::create_dir_all(directories.planes().join("bp-00000000")).unwrap();

        let mut offered = ["bp-00000000", "bp-11111111"].into_iter();
        let claimed = ClaimedPlane::claim_generated_by(&directories, || {
            Ok(PlaneId::parse(offered.next().unwrap()).unwrap())
        })
        .unwrap();

        assert_eq!(claimed.id().as_str(), "bp-11111111");
    }

    #[test]
    fn a_generated_claim_gives_up_after_five_collisions() {
        let (directories, _) = somewhere("claim-generated-bound");
        let mut attempts = 0;

        let error = ClaimedPlane::claim_generated_by(&directories, || {
            attempts += 1;
            let id = PlaneId::parse("bp-deadbeef").unwrap();
            fs::create_dir_all(directories.plane(&id)).unwrap();
            Ok(id)
        })
        .unwrap_err();

        assert_eq!(
            attempts, GENERATED_ID_ATTEMPTS,
            "needing a sixth means something other than collision is wrong"
        );
        assert!(
            matches!(error, EngineError::PlaneIdInUse { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn the_latch_is_written_before_the_plane_file_and_cleared_once() {
        let (directories, _) = somewhere("latch");
        let id = PlaneId::parse("auth-work").unwrap();
        let claimed = ClaimedPlane::claim(&directories, id.clone()).unwrap();

        claimed.latch().unwrap();
        assert_eq!(classify(claimed.path()), Occupant::LatchedRemnant);

        claimed
            .write_plane_file(&PlaneFile::new(id, Vec::new()))
            .unwrap();
        assert_eq!(classify(claimed.path()), Occupant::LatchedRemnant);

        claimed.unlatch().unwrap();
        assert_eq!(classify(claimed.path()), Occupant::Plane);

        claimed
            .unlatch()
            .expect("clearing a cleared latch is not a failure");
    }

    #[test]
    fn the_latch_holds_a_timestamp_and_nothing_else() {
        let (directories, _) = somewhere("latch-contents");
        let claimed =
            ClaimedPlane::claim(&directories, PlaneId::parse("auth-work").unwrap()).unwrap();

        claimed.latch().unwrap();

        let written = fs::read_to_string(claimed.latch_path()).unwrap();
        assert!(written.trim().ends_with('Z'), "got {written:?}");
        assert_eq!(written.lines().count(), 1, "got {written:?}");
    }

    #[test]
    fn the_lock_is_taken_on_the_sentinel_not_on_the_plane_file() {
        let (directories, _) = somewhere("lock-sentinel");
        let claimed =
            ClaimedPlane::claim(&directories, PlaneId::parse("auth-work").unwrap()).unwrap();

        let lock = claimed.lock(|_| {}).unwrap();

        assert_eq!(
            lock.object(),
            claimed.path().join(BITPLANE_DIR).join(lock::SENTINEL_NAME)
        );
        assert!(!lock.object().ends_with(PLANE_FILE_NAME));
    }

    #[test]
    fn discarding_takes_the_whole_claim_back() {
        let (directories, _) = somewhere("discard");
        let claimed =
            ClaimedPlane::claim(&directories, PlaneId::parse("auth-work").unwrap()).unwrap();
        claimed.latch().unwrap();

        claimed.discard().unwrap();

        assert!(!claimed.path().exists());
        claimed
            .discard()
            .expect("discarding twice is not a failure");
    }

    // Resolution itself — by id, by walking up from a path, innermost winning
    // — is `read::resolve`'s and is tested against it in `tests/plane_read.rs`.
    // What is left here is what `OpenPlane` itself owns.

    #[test]
    fn an_opened_plane_takes_its_id_from_the_directorys_own_name() {
        let (directories, _) = somewhere("open-at");
        let id = PlaneId::parse("auth-work").unwrap();
        let claimed = ClaimedPlane::claim(&directories, id.clone()).unwrap();
        claimed
            .write_plane_file(&PlaneFile::new(id, Vec::new()))
            .unwrap();

        let open = OpenPlane::at(claimed.path().to_path_buf());

        assert_eq!(open.id(), "auth-work");
        assert_eq!(open.path(), claimed.path());
        assert_eq!(open.read_plane_file().unwrap().unwrap().members.len(), 0);
    }

    #[test]
    fn an_opened_plane_reports_the_latch_destroy_is_the_only_way_out_of() {
        let (directories, _) = somewhere("open-latched");
        let id = PlaneId::parse("auth-work").unwrap();
        let claimed = ClaimedPlane::claim(&directories, id.clone()).unwrap();
        claimed.latch().unwrap();
        claimed
            .write_plane_file(&PlaneFile::new(id, Vec::new()))
            .unwrap();

        let open = OpenPlane::at(claimed.path().to_path_buf());

        assert!(open.is_latched());

        claimed.unlatch().unwrap();
        assert!(!OpenPlane::at(claimed.path().to_path_buf()).is_latched());
    }

    fn somewhere(label: &str) -> (Directories, PathBuf) {
        let dir = scratch_dir(label);
        let directories = Directories::new(dir.join("planes"), dir.join("projects"));
        (directories, dir)
    }
}
