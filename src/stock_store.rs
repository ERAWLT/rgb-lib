//! ERA fork (CC-101): the RGB stock on disk.
//!
//! rgb-ops keeps a wallet's RGB state in three files, `rgb/stash.dat`, `state.dat` and
//! `index.dat` (its `FsBinStore`), and stores each by truncating it in place and writing it
//! field by field, with no sync. A store that fails (a full disk) or is interrupted (the process
//! killed) leaves a file cut short, which no later load can read. And rgb-lib created an empty
//! stock whenever one of the three was missing, which overwrote the two that survived.
//!
//! [`StockStore`] replaces `FsBinStore` as the stock's persistence provider:
//! - A file is written whole to `<name>.new` next to it, synced, renamed over it, and the
//!   directory synced. On disk a file is always a version that was completely written: the
//!   previous one until the rename, the new one after. A store whose bytes equal the file's is
//!   skipped (the one at the start of every rgb-ops transaction rewrites unchanged data).
//! - A store that fails is not returned to rgb-ops as an error. rgb-ops answers a failed store
//!   at the end of a transaction by rolling back its in-memory providers, whose rollback is
//!   `unreachable!()`: the process would panic. The failure is kept instead, per file, until a
//!   later store of that file succeeds, and `RgbRuntime` reports it as `Error::IO` after the call
//!   that stored. The file keeps its previous version.
//! - Since rgb-ops then goes on with the files after it, the stash is held back while the index or
//!   the state is behind: the stash is never newer than either.
//!
//! [`open_stock`] loads the three files only as a whole set; see [`StockFiles`].

use std::sync::{Arc, Mutex};

use amplify::confinement::U32 as U32MAX;
use nonasync::persistence::{PersistenceError, PersistenceProvider};
use rgbstd::persistence::{MemIndex, MemStash, MemState};
use strict_encoding::{StrictDeserialize, StrictSerialize};

use super::*;
use crate::utils::RGB_RUNTIME_DIR;

/// The files of the stock, in the order rgb-ops loads them.
pub(crate) const STOCK_FILES: [&str; 3] = ["stash.dat", "state.dat", "index.dat"];
/// Where a new stock is made before it is renamed into place, next to `rgb/`.
pub(crate) const STOCK_STAGING_DIR: &str = "rgb.new";
const NEW_SUFFIX: &str = ".new";

/// The stock's persistence provider: the three files of one directory, each stored atomically.
#[derive(Clone, Debug)]
pub(crate) struct StockStore {
    dir: PathBuf,
    // the files whose latest store failed, with the reason
    failed: Arc<Mutex<BTreeMap<String, String>>>,
}

impl StockStore {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            failed: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// `Err(IO)` while the latest store of any file failed: the file on disk is behind the stock
    /// in memory.
    pub(crate) fn check_stored(&self) -> Result<(), String> {
        let failed = self.failed.lock().expect("not poisoned");
        if failed.is_empty() {
            return Ok(());
        }
        Err(failed
            .iter()
            .map(|(name, why)| format!("unable to store {name}: {why}"))
            .collect::<Vec<_>>()
            .join("; "))
    }

    fn load_file<T: StrictDeserialize>(&self, name: &str) -> Result<T, PersistenceError> {
        T::strict_deserialize_from_file::<U32MAX>(self.dir.join(name))
            .map_err(|e| PersistenceError::with(LoadFailure::from_deserialize(name, e)))
    }

    fn store_file<T: StrictSerialize>(
        &self,
        name: &str,
        object: &T,
    ) -> Result<(), PersistenceError> {
        // The stash stays the oldest of the three: rgb-ops commits index, state and stash in that
        // order, and a stash that holds a bundle is how a consume is known to have happened
        // (fascia_unknown_part). While the latest store of the index or the state failed, the
        // stash is not written: a consume whose index or state is not on disk is then consumed
        // again, instead of being skipped with its state missing.
        let behind = (name == STOCK_FILES[0])
            .then(|| {
                self.failed
                    .lock()
                    .expect("not poisoned")
                    .keys()
                    .find(|other| other.as_str() != name)
                    .cloned()
            })
            .flatten();
        let result = match behind {
            Some(other) => Err(format!("held back: {other} is not stored")),
            None => {
                let pending = self.failed.lock().expect("not poisoned").contains_key(name);
                object
                    .to_strict_serialized::<U32MAX>()
                    .map_err(|e| e.to_string())
                    .and_then(|data| replace_file(&self.dir, name, data.as_slice(), pending))
            }
        };
        let mut failed = self.failed.lock().expect("not poisoned");
        match result {
            Ok(()) => {
                failed.remove(name);
            }
            Err(why) => {
                failed.insert(name.to_string(), why);
            }
        }
        Ok(())
    }
}

impl PersistenceProvider<MemStash> for StockStore {
    fn load(&self) -> Result<MemStash, PersistenceError> {
        self.load_file(STOCK_FILES[0])
    }

    fn store(&self, object: &MemStash) -> Result<(), PersistenceError> {
        self.store_file(STOCK_FILES[0], object)
    }
}

impl PersistenceProvider<MemState> for StockStore {
    fn load(&self) -> Result<MemState, PersistenceError> {
        self.load_file(STOCK_FILES[1])
    }

    fn store(&self, object: &MemState) -> Result<(), PersistenceError> {
        self.store_file(STOCK_FILES[1], object)
    }
}

impl PersistenceProvider<MemIndex> for StockStore {
    fn load(&self) -> Result<MemIndex, PersistenceError> {
        self.load_file(STOCK_FILES[2])
    }

    fn store(&self, object: &MemIndex) -> Result<(), PersistenceError> {
        self.store_file(STOCK_FILES[2], object)
    }
}

/// Replace `dir/name` with `data`: written whole to `dir/name.new`, synced, renamed over the
/// file, then the directory synced. Nothing is written when the file already holds `data`; with
/// a failure `pending` for the file, the directory is still synced then (the failure may have
/// been that sync, after a rename that made the file what it is). A failure before the rename
/// leaves the file as it was (and removes the new one if it can).
fn replace_file(dir: &Path, name: &str, data: &[u8], pending: bool) -> Result<(), String> {
    let target = dir.join(name);
    if fs::read(&target).is_ok_and(|current| current == data) {
        if pending {
            return sync_dir(dir, name).map_err(|e| e.to_string());
        }
        return Ok(());
    }
    let new = dir.join(format!("{name}{NEW_SUFFIX}"));
    let write = || -> io::Result<()> {
        let mut file = fs::File::create(&new)?;
        if let Err(e) = inject(name, InjectedFailure::HalfWritten) {
            file.write_all(&data[..data.len() / 2])?;
            return Err(e);
        }
        file.write_all(data)?;
        sync_file(name, file)?;
        inject(name, InjectedFailure::BeforeRename)?;
        rename(name, &new, &target)
    };
    if let Err(e) = write() {
        let _ = fs::remove_file(&new);
        return Err(e.to_string());
    }
    sync_dir(dir, name).map_err(|e| e.to_string())
}

fn sync_file(name: &str, file: fs::File) -> io::Result<()> {
    record(StoreEvent::Sync(name.to_string()));
    file.sync_all()
}

fn rename(name: &str, from: &Path, to: &Path) -> io::Result<()> {
    record(StoreEvent::Rename(name.to_string()));
    fs::rename(from, to)
}

/// Sync a directory after renaming `name` into it, so the entry is on disk. Where a directory
/// cannot be opened as a file (Windows), there is nothing to sync this way.
fn sync_dir(dir: &Path, name: &str) -> io::Result<()> {
    inject(name, InjectedFailure::DirSync)?;
    record(StoreEvent::SyncDir(name.to_string()));
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// Why a file of the stock could not be loaded.
#[derive(Debug)]
pub(crate) enum LoadFailure {
    /// The file is not a whole stock file: cut short, longer than its content, or not decoding
    Damaged(String),
    /// The file system refused the read (permissions, I/O)
    Io(String),
}

impl LoadFailure {
    fn from_deserialize(name: &str, e: DeserializeError) -> Self {
        match e {
            DeserializeError::Decode(DecodeError::Io(io)) => match io.kind() {
                ErrorKind::UnexpectedEof => LoadFailure::Damaged(format!("{name} is cut short")),
                ErrorKind::NotFound => LoadFailure::Damaged(format!("{name} is missing")),
                _ => LoadFailure::Io(format!("{name}: {io}")),
            },
            DeserializeError::DataNotEntirelyConsumed => {
                LoadFailure::Damaged(format!("{name} is longer than its content"))
            }
            DeserializeError::Decode(e) => {
                LoadFailure::Damaged(format!("{name} does not decode: {e}"))
            }
        }
    }
}

impl fmt::Display for LoadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadFailure::Damaged(details) | LoadFailure::Io(details) => f.write_str(details),
        }
    }
}

impl std::error::Error for LoadFailure {}

impl From<LoadFailure> for Error {
    fn from(failure: LoadFailure) -> Self {
        match failure {
            LoadFailure::Damaged(details) => Error::RgbStockDamaged { details },
            LoadFailure::Io(details) => Error::IO { details },
        }
    }
}

/// Which of the stock's files a directory holds.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StockFiles {
    /// all three
    Whole,
    /// none: a new stock, for a wallet that has none yet
    None,
    /// some, not all: never a stock rgb-lib wrote
    Partial(Vec<&'static str>),
}

impl StockFiles {
    pub(crate) fn of(dir: &Path) -> Result<Self, Error> {
        let mut missing = vec![];
        for name in STOCK_FILES {
            match fs::symlink_metadata(dir.join(name)) {
                Ok(metadata) if metadata.is_file() => {}
                Ok(_) => {
                    return Err(Error::RgbStockDamaged {
                        details: format!("{name} is not a file"),
                    });
                }
                Err(e) if e.kind() == ErrorKind::NotFound => missing.push(name),
                Err(e) => {
                    return Err(Error::IO {
                        details: format!("{name}: {e}"),
                    });
                }
            }
        }
        Ok(match missing.len() {
            0 => StockFiles::Whole,
            n if n == STOCK_FILES.len() => StockFiles::None,
            _ => StockFiles::Partial(missing),
        })
    }
}

/// Load the stock of `rgb_dir` with `store` as its provider.
///
/// All three files: loaded, and any that is cut short, longer than its content or not decoding
/// is [`Error::RgbStockDamaged`] (a file the system refuses to read is `Error::IO`). Some but not
/// all: [`Error::RgbStockDamaged`], nothing written. None: a new, empty stock when
/// `new_allowed` (a wallet that has none yet), made in [`STOCK_STAGING_DIR`] and renamed into
/// place whole, so that a stock is never seen half made; [`Error::RgbStockDamaged`] otherwise.
/// A `.new` file a store left behind is removed first; the caller holds the runtime's lock.
pub(crate) fn open_stock(
    wallet_dir: &Path,
    rgb_dir: &Path,
    new_allowed: bool,
) -> Result<(Stock, StockStore), Error> {
    let staging = wallet_dir.join(STOCK_STAGING_DIR);
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    if rgb_dir.is_dir() {
        for name in STOCK_FILES {
            let new = rgb_dir.join(format!("{name}{NEW_SUFFIX}"));
            if new.exists() {
                fs::remove_file(new)?;
            }
        }
        // what the stock is built on is on disk: a process killed between a rename and the sync
        // of the directory after it left a rename that may not be
        sync_dir(rgb_dir, RGB_RUNTIME_DIR)?;
    }
    match StockFiles::of(rgb_dir)? {
        StockFiles::Whole => {}
        StockFiles::Partial(missing) => {
            return Err(Error::RgbStockDamaged {
                details: format!("{} missing", missing.join(", ")),
            });
        }
        StockFiles::None if !new_allowed => {
            return Err(Error::RgbStockDamaged {
                details: s!("no stock file for a wallet that has a manifest"),
            });
        }
        StockFiles::None => make_new_stock(wallet_dir, rgb_dir, &staging)?,
    }
    let store = StockStore::new(rgb_dir.to_path_buf());
    let stock =
        Stock::load(store.clone(), true).map_err(|e| match e.0.downcast::<LoadFailure>() {
            Ok(failure) => Error::from(*failure),
            Err(e) => Error::IO {
                details: e.to_string(),
            },
        })?;
    Ok((stock, store))
}

fn make_new_stock(wallet_dir: &Path, rgb_dir: &Path, staging: &Path) -> Result<(), Error> {
    fs::create_dir_all(staging)?;
    let store = StockStore::new(staging.to_path_buf());
    let mut stock = Stock::in_memory();
    stock
        .make_persistent(store.clone(), true)
        .map_err(|e| Error::IO {
            details: e.to_string(),
        })?;
    store
        .check_stored()
        .map_err(|details| Error::IO { details })?;
    // what `rgb/` holds is no stock (checked by the caller): at most a store's leftovers
    if rgb_dir.exists() {
        fs::remove_dir_all(rgb_dir)?;
    }
    inject(STOCK_STAGING_DIR, InjectedFailure::BeforeRename)?;
    rename(STOCK_STAGING_DIR, staging, rgb_dir)?;
    sync_dir(wallet_dir, STOCK_STAGING_DIR)?;
    Ok(())
}

// Test hooks: what a store does to the disk, recorded, and failures a test injects. Stores run on
// the thread that calls into the stock, so each test sees only its own.

/// What a store does that ordering matters for, by file (or staging directory) name.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StoreEvent {
    /// the new file synced
    Sync(String),
    /// the new file (or the staging directory) renamed into place
    Rename(String),
    /// its directory synced after the rename
    SyncDir(String),
}

/// Where an injected failure stops a store.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InjectedFailure {
    /// half of the new file written (a full disk)
    HalfWritten,
    /// the new file written and synced, not renamed (the process killed)
    BeforeRename,
    /// the directory sync after the rename
    DirSync,
}

#[cfg(test)]
thread_local! {
    pub(crate) static STORE_EVENTS: std::cell::RefCell<Vec<StoreEvent>> =
        const { std::cell::RefCell::new(vec![]) };
    // (file name, where, how many stores it still fails)
    pub(crate) static STORE_FAILURES: std::cell::RefCell<Vec<(String, InjectedFailure, usize)>> =
        const { std::cell::RefCell::new(vec![]) };
}

fn record(_event: StoreEvent) {
    #[cfg(test)]
    STORE_EVENTS.with_borrow_mut(|events| events.push(_event));
}

/// A failure a test injected at this point of the store of `name`.
fn inject(_name: &str, _at: InjectedFailure) -> io::Result<()> {
    #[cfg(test)]
    if STORE_FAILURES.with_borrow_mut(|failures| {
        match failures
            .iter_mut()
            .find(|(name, at, left)| name == _name && *at == _at && *left > 0)
        {
            Some((_, _, left)) => {
                *left -= 1;
                true
            }
            None => false,
        }
    }) {
        return Err(io::Error::other(format!("{_at:?} (test)")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rgbstd::persistence::fs::FsBinStore;
    use serial_test::parallel;

    use super::*;

    const NEW: bool = true;
    const EXISTING: bool = false;

    fn rgb(wallet_dir: &Path) -> PathBuf {
        wallet_dir.join(RGB_RUNTIME_DIR)
    }

    /// Every file under the wallet directory but the runtime's lock, by name, with its bytes.
    fn files(wallet_dir: &Path) -> BTreeMap<String, Vec<u8>> {
        WalkDir::new(wallet_dir)
            .into_iter()
            .map(|e| e.unwrap())
            .filter(|e| e.file_type().is_file() && e.file_name() != "rgb_runtime.lock")
            .map(|e| {
                (
                    e.path()
                        .strip_prefix(wallet_dir)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    fs::read(e.path()).unwrap(),
                )
            })
            .collect()
    }

    fn names(wallet_dir: &Path) -> Vec<String> {
        files(wallet_dir).into_keys().collect()
    }

    fn stock_names() -> Vec<String> {
        let mut names: Vec<String> = STOCK_FILES.iter().map(|n| format!("rgb/{n}")).collect();
        names.sort();
        names
    }

    /// A wallet directory with a stock that knows the NIA schema.
    fn with_stock() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = load_or_create_rgb_runtime(dir.path(), NEW).unwrap();
        AssetSchema::Nia.import_kit(&mut runtime).unwrap();
        dir
    }

    fn schemata(dir: &Path) -> usize {
        load_rgb_runtime(dir).unwrap().schemata().unwrap().len()
    }

    fn inject(name: &str, at: InjectedFailure, times: usize) {
        STORE_FAILURES.with_borrow_mut(|f| f.push((name.to_string(), at, times)));
    }

    fn clear_injected() {
        STORE_FAILURES.with_borrow_mut(|f| f.clear());
    }

    fn seal() -> GraphSeal {
        GraphSeal::new_random_vout(0)
    }

    fn damaged(result: Result<RgbRuntime, Error>) -> String {
        match result {
            Err(Error::RgbStockDamaged { details }) => details,
            Err(e) => panic!("expected RgbStockDamaged, got {e:?}"),
            Ok(_) => panic!("expected RgbStockDamaged, got a runtime"),
        }
    }

    #[test]
    #[parallel]
    fn a_new_stock_is_made_whole_and_reads_back() {
        let dir = with_stock();
        assert_eq!(names(dir.path()), stock_names());
        assert!(!dir.path().join(STOCK_STAGING_DIR).exists());
        assert_eq!(schemata(dir.path()), 1);
    }

    // the files are rgb-ops's own: a stock FsBinStore wrote loads here (a wallet of an older
    // rev), and this store writes the bytes FsBinStore would
    #[test]
    #[parallel]
    fn the_files_are_the_ones_rgb_ops_writes() {
        let ours = with_stock();
        let stock: Stock = Stock::load(FsBinStore::new(rgb(ours.path())).unwrap(), false).unwrap();
        let theirs = tempfile::tempdir().unwrap();
        let mut stock = stock;
        stock
            .make_persistent(FsBinStore::new(rgb(theirs.path())).unwrap(), false)
            .unwrap();
        drop(stock);
        assert_eq!(files(theirs.path()), files(ours.path()));
        assert_eq!(schemata(theirs.path()), 1);
    }

    // a full disk half way through writing the stash: the store is reported, the file keeps its
    // previous version whole, nothing is left next to it, and nothing panics
    #[test]
    #[parallel]
    fn a_store_stopped_half_way_keeps_the_previous_version() {
        let dir = with_stock();
        let before = files(dir.path());
        inject("stash.dat", InjectedFailure::HalfWritten, usize::MAX);
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
        let error = runtime.store_secret_seal(seal()).unwrap_err();
        assert!(matches!(&error, InternalError::StockNotStored(d) if d.contains("stash.dat")));
        let error = Error::from(error);
        assert_matches!(error, Error::IO { details } if details.contains("HalfWritten"));
        assert_matches!(runtime.persist(), Err(Error::IO { .. }));
        drop(runtime);
        clear_injected();
        assert_eq!(files(dir.path()), before);
        assert_eq!(schemata(dir.path()), 1);
    }

    // killed with the new file written and not renamed: the previous version is loaded and the
    // new file goes
    #[test]
    #[parallel]
    fn a_kill_before_the_rename_leaves_the_previous_version() {
        let dir = with_stock();
        let before = files(dir.path());
        inject("stash.dat", InjectedFailure::BeforeRename, 1);
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
        assert_matches!(
            runtime.store_secret_seal(seal()),
            Err(InternalError::StockNotStored(_))
        );
        drop(runtime);
        assert_eq!(files(dir.path()), before);
        // what a kill at that point leaves: the new file, cut short at that
        let stash = rgb(dir.path()).join("stash.dat");
        let data = fs::read(&stash).unwrap();
        fs::write(
            rgb(dir.path()).join("stash.dat.new"),
            &data[..data.len() / 3],
        )
        .unwrap();
        assert_eq!(schemata(dir.path()), 1);
        assert_eq!(files(dir.path()), before);
    }

    // a new stock whose making died before its rename leaves rgb.new/ behind: it goes at the next
    // load, whatever it holds
    #[test]
    #[parallel]
    fn a_stale_staging_directory_goes() {
        let dir = with_stock();
        let before = files(dir.path());
        let staging = dir.path().join(STOCK_STAGING_DIR);
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("stash.dat"), b"half").unwrap();
        assert_eq!(schemata(dir.path()), 1);
        assert!(!staging.exists());
        assert_eq!(files(dir.path()), before);
    }

    // a failure lasts until the file is stored again: the next store of it catches up
    #[test]
    #[parallel]
    fn a_failure_lasts_until_the_file_is_stored() {
        let dir = with_stock();
        inject("stash.dat", InjectedFailure::HalfWritten, 1);
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
        let first = seal();
        assert_matches!(
            runtime.store_secret_seal(first),
            Err(InternalError::StockNotStored(_))
        );
        assert!(runtime.store_secret_seal(seal()).unwrap());
        runtime.persist().unwrap();
        drop(runtime);
        // the stash on disk caught up with the one in memory, the first seal included
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
        assert!(
            runtime
                .seal_secret(first.to_secret_seal())
                .unwrap()
                .is_some()
        );
    }

    // after the rename, a directory that cannot be synced is reported too: the new version is in
    // place, but not known to be on disk
    #[test]
    #[parallel]
    fn a_directory_sync_that_fails_is_reported() {
        let dir = with_stock();
        inject("stash.dat", InjectedFailure::DirSync, 1);
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
        assert_matches!(
            runtime.store_secret_seal(seal()),
            Err(InternalError::StockNotStored(d)) if d.contains("DirSync")
        );
    }

    // a directory sync that failed after the rename is not forgotten by a later store of the same,
    // unchanged bytes: that store syncs the directory, and only then is the failure gone
    #[test]
    #[parallel]
    fn a_failed_directory_sync_is_made_up_for_by_the_next_store() {
        let dir = with_stock();
        inject("stash.dat", InjectedFailure::DirSync, 1);
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
        assert!(matches!(
            runtime.store_secret_seal(seal()),
            Err(InternalError::StockNotStored(_))
        ));
        STORE_EVENTS.with_borrow_mut(|e| e.clear());
        // unchanged bytes: rgb-ops stores the stash at the start and at the end of the transaction
        AssetSchema::Nia.import_kit(&mut runtime).unwrap();
        let events = STORE_EVENTS.with_borrow_mut(std::mem::take);
        assert_eq!(events, vec![StoreEvent::SyncDir(s!("stash.dat"))]);
        runtime.persist().unwrap();
    }

    // a load syncs rgb/ before a stock is built on it: a process killed between a rename and the
    // sync of the directory after it left a rename that may not be on disk
    #[test]
    #[parallel]
    fn a_load_syncs_the_stock_directory() {
        let dir = with_stock();
        STORE_EVENTS.with_borrow_mut(|e| e.clear());
        drop(load_rgb_runtime(dir.path()).unwrap());
        let events = STORE_EVENTS.with_borrow_mut(std::mem::take);
        assert_eq!(
            events,
            vec![StoreEvent::SyncDir(RGB_RUNTIME_DIR.to_string())]
        );
    }

    // every store: the new file written and synced, renamed, then the directory synced; a store of
    // unchanged bytes writes nothing
    #[test]
    #[parallel]
    fn a_store_syncs_before_and_after_the_rename() {
        let dir = with_stock();
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
        STORE_EVENTS.with_borrow_mut(|e| e.clear());
        runtime.store_secret_seal(seal()).unwrap();
        let events = STORE_EVENTS.with_borrow_mut(std::mem::take);
        let stash: Vec<&StoreEvent> = events
            .iter()
            .filter(|e| {
                matches!(e, StoreEvent::Sync(n) | StoreEvent::Rename(n) | StoreEvent::SyncDir(n)
                    if n == "stash.dat")
            })
            .collect();
        assert_eq!(
            stash,
            vec![
                &StoreEvent::Sync(s!("stash.dat")),
                &StoreEvent::Rename(s!("stash.dat")),
                &StoreEvent::SyncDir(s!("stash.dat")),
            ]
        );
        // the other two did not change
        assert_eq!(events.len(), 3);
        runtime.persist().unwrap();
        assert!(STORE_EVENTS.with_borrow(|e| e.is_empty()));
    }

    // one file missing: refused, nothing written, whether a new stock is allowed or not
    #[test]
    #[parallel]
    fn a_missing_file_is_refused_and_the_others_kept() {
        for name in STOCK_FILES {
            let dir = with_stock();
            fs::remove_file(rgb(dir.path()).join(name)).unwrap();
            let before = files(dir.path());
            for new_allowed in [EXISTING, NEW] {
                let details = damaged(load_or_create_rgb_runtime(dir.path(), new_allowed));
                assert_eq!(details, format!("{name} missing"));
                assert_eq!(files(dir.path()), before, "{name}");
                // the refused load released the runtime's lock
                assert!(!dir.path().join("rgb_runtime.lock").exists());
            }
        }
    }

    // a file cut short, empty, longer than its content, not decoding, or not a file: refused,
    // nothing written
    #[test]
    #[parallel]
    fn a_damaged_file_is_refused_and_nothing_written() {
        type Damage = fn(&Path, &str);
        let damages: [(Damage, &str); 5] = [
            (
                |path, _| {
                    let data = fs::read(path).unwrap();
                    fs::write(path, &data[..data.len() / 2]).unwrap();
                },
                "is cut short",
            ),
            (|path, _| fs::write(path, b"").unwrap(), "is cut short"),
            (
                |path, _| {
                    let mut data = fs::read(path).unwrap();
                    data.push(0);
                    fs::write(path, data).unwrap();
                },
                "is longer than its content",
            ),
            (|path, _| fs::write(path, [0xffu8; 64]).unwrap(), ""),
            (
                |path, _| {
                    fs::remove_file(path).unwrap();
                    fs::create_dir(path).unwrap();
                },
                "is not a file",
            ),
        ];
        for name in STOCK_FILES {
            for (damage, expected) in damages {
                let dir = with_stock();
                damage(&rgb(dir.path()).join(name), name);
                let before = files(dir.path());
                let details = damaged(load_rgb_runtime(dir.path()));
                assert!(details.starts_with(name), "{name}: {details}");
                assert!(details.contains(expected), "{name}: {details}");
                assert_eq!(files(dir.path()), before, "{name}: {details}");
            }
        }
    }

    // a file the system refuses to read is an I/O error, which a retry may get past
    #[cfg(unix)]
    #[test]
    #[parallel]
    fn an_unreadable_file_is_an_io_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = with_stock();
        let stash = rgb(dir.path()).join("stash.dat");
        fs::set_permissions(&stash, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&stash).is_ok() {
            // running as root
            return;
        }
        let result = load_rgb_runtime(dir.path());
        fs::set_permissions(&stash, fs::Permissions::from_mode(0o644)).unwrap();
        assert_matches!(result, Err(Error::IO { details }) if details.starts_with("stash.dat"));
    }

    // no stock file at all: a new stock only where one is allowed
    #[test]
    #[parallel]
    fn no_stock_is_new_only_where_allowed() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(rgb(dir.path())).unwrap();
        let details = damaged(load_or_create_rgb_runtime(dir.path(), EXISTING));
        assert!(details.contains("no stock file"), "{details}");
        assert!(names(dir.path()).is_empty());
        let with_stock = with_stock();
        for name in STOCK_FILES {
            fs::remove_file(rgb(with_stock.path()).join(name)).unwrap();
        }
        damaged(load_rgb_runtime(with_stock.path()));
        load_or_create_rgb_runtime(with_stock.path(), NEW).unwrap();
        assert_eq!(names(with_stock.path()), stock_names());
    }

    // a new stock is made next to rgb/ and renamed into place whole: one that fails to be made
    // leaves no stock file in rgb/, and the next attempt makes it
    #[test]
    #[parallel]
    fn a_new_stock_that_fails_leaves_none_half_made() {
        for (name, at) in [
            ("state.dat", InjectedFailure::HalfWritten),
            ("index.dat", InjectedFailure::BeforeRename),
            (STOCK_STAGING_DIR, InjectedFailure::BeforeRename),
        ] {
            let dir = tempfile::tempdir().unwrap();
            inject(name, at, 1);
            let result = load_or_create_rgb_runtime(dir.path(), NEW);
            assert!(matches!(result, Err(Error::IO { .. })), "{name}");
            assert_eq!(
                StockFiles::of(&rgb(dir.path())).unwrap(),
                StockFiles::None,
                "{name}"
            );
            let mut runtime = load_or_create_rgb_runtime(dir.path(), NEW).unwrap();
            AssetSchema::Nia.import_kit(&mut runtime).unwrap();
            drop(runtime);
            assert_eq!(names(dir.path()), stock_names(), "{name}");
            clear_injected();
        }
    }

    // killed while storing, over and over (a child process storing in a loop, SIGKILLed at
    // spread delays): every load afterwards finds the whole stock, never a file cut short
    #[test]
    #[parallel]
    fn a_kill_mid_store_never_leaves_a_file_cut_short() {
        let dir = with_stock();
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
        for schema in [AssetSchema::Cfa, AssetSchema::Uda, AssetSchema::Ifa] {
            schema.import_kit(&mut runtime).unwrap();
        }
        drop(runtime);
        let exe = std::env::current_exe().unwrap();
        let rounds = 24u32;
        let mut new_files_left = 0;
        for round in 0..rounds {
            let mut child = std::process::Command::new(&exe)
                .args([
                    "--exact",
                    "stock_store::tests::store_in_a_loop",
                    "--test-threads=1",
                ])
                .env("ERA_STOCK_STORE_LOOP", dir.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(
                150 + u64::from(round) * 25,
            ));
            child.kill().unwrap();
            child.wait().unwrap();
            if STOCK_FILES
                .iter()
                .any(|n| rgb(dir.path()).join(format!("{n}.new")).exists())
            {
                new_files_left += 1;
            }
            // what the host does with the lock of a process that died
            let _ = fs::remove_file(dir.path().join("rgb_runtime.lock"));
            let runtime = load_rgb_runtime(dir.path());
            assert!(runtime.is_ok(), "round {round}: {:?}", runtime.err());
            assert_eq!(runtime.unwrap().schemata().unwrap().len(), 4);
        }
        println!("{new_files_left} of {rounds} kills left a new file behind");
    }

    // the other calls that store report a failed store too
    #[test]
    #[parallel]
    fn every_call_that_stores_reports_a_failed_store() {
        let dir = with_stock();
        let before = files(dir.path());
        inject("stash.dat", InjectedFailure::HalfWritten, usize::MAX);
        inject("index.dat", InjectedFailure::HalfWritten, usize::MAX);
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
        assert!(matches!(
            AssetSchema::Cfa.import_kit(&mut runtime),
            Err(Error::IO { .. })
        ));
        let witness =
            RgbTxid::from_str("e5a3e577309df31bd606f48049049d2e1e02b048206ba232944fcc053a176ccb")
                .unwrap();
        assert!(matches!(
            runtime.upsert_witness(witness, WitnessOrd::Tentative),
            Err(InternalError::StockNotStored(_))
        ));
        drop(runtime);
        clear_injected();
        // each file is whole: the two whose stores failed as they were, and the set loads (state.dat,
        // stored between them, may be the newer one: the three are not written as one)
        let after = files(dir.path());
        for name in ["rgb/stash.dat", "rgb/index.dat"] {
            assert_eq!(after.get(name), before.get(name), "{name}");
        }
        assert_eq!(schemata(dir.path()), 1);
    }

    // a wallet's stock missing a file (here the index, with the stash worth keeping) is refused
    // when the wallet is opened again, and the stash is left as it was; and so is one with no
    // stock at all, since it has a manifest. A new wallet gets its stock whole.
    #[test]
    #[parallel]
    fn a_wallet_with_a_damaged_stock_is_refused() {
        use crate::wallet::{RgbWalletOpsOffline, test::get_test_wallet};
        let wallet = get_test_wallet(true, None);
        let (wallet_data, keys, wallet_dir) = (
            wallet.get_wallet_data(),
            wallet.get_keys(),
            wallet.get_wallet_dir(),
        );
        drop(wallet);
        let stock: Vec<String> = names(&wallet_dir)
            .into_iter()
            .filter(|n| n.starts_with("rgb/"))
            .collect();
        assert_eq!(stock, stock_names());

        fs::remove_file(rgb(&wallet_dir).join("index.dat")).unwrap();
        let before = files(&wallet_dir);
        let result = Wallet::new(wallet_data.clone(), keys.clone());
        assert!(matches!(
            result,
            Err(Error::RgbStockDamaged { ref details }) if details == "index.dat missing"
        ));
        let after = files(&wallet_dir);
        assert_eq!(after.get("rgb/stash.dat"), before.get("rgb/stash.dat"));

        fs::remove_dir_all(rgb(&wallet_dir)).unwrap();
        let result = Wallet::new(wallet_data, keys);
        assert!(matches!(result, Err(Error::RgbStockDamaged { .. })));
        assert!(!rgb(&wallet_dir).exists());
    }

    // the child of a_kill_mid_store_never_leaves_a_file_cut_short; a no-op unless it is that child
    #[test]
    fn store_in_a_loop() {
        let Some(dir) = std::env::var_os("ERA_STOCK_STORE_LOOP") else {
            return;
        };
        loop {
            let mut runtime = load_rgb_runtime(PathBuf::from(&dir)).unwrap();
            for _ in 0..8 {
                runtime.store_secret_seal(seal()).unwrap();
            }
        }
    }
}
