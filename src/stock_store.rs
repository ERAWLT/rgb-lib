//! ERA fork (CC-101): the RGB stock on disk.
//!
//! rgb-ops keeps a wallet's RGB state in three files, `rgb/stash.dat`, `state.dat` and
//! `index.dat` (its `FsBinStore`), and stores each by truncating it in place and writing it
//! field by field, with no sync. A store that fails (a full disk) or is interrupted (the process
//! killed) leaves a file cut short, which no later load can read.
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
//!

use std::sync::{Arc, Mutex};

use amplify::confinement::U32 as U32MAX;
use nonasync::persistence::{PersistenceError, PersistenceProvider};
use rgbstd::persistence::{MemIndex, MemStash, MemState};
use strict_encoding::{StrictDeserialize, StrictSerialize};

use super::*;

/// The files of the stock, in the order rgb-ops loads them.
pub(crate) const STOCK_FILES: [&str; 3] = ["stash.dat", "state.dat", "index.dat"];
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
            .map_err(PersistenceError::with)
    }

    fn store_file<T: StrictSerialize>(
        &self,
        name: &str,
        object: &T,
    ) -> Result<(), PersistenceError> {
        let result = object
            .to_strict_serialized::<U32MAX>()
            .map_err(|e| e.to_string())
            .and_then(|data| replace_file(&self.dir, name, data.as_slice()));
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
/// file, then the directory synced. Nothing is written when the file already holds `data`. A
/// failure before the rename leaves the file as it was (and removes the new one if it can).
fn replace_file(dir: &Path, name: &str, data: &[u8]) -> Result<(), String> {
    let target = dir.join(name);
    if fs::read(&target).is_ok_and(|current| current == data) {
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

    use crate::utils::RGB_RUNTIME_DIR;

    use super::*;

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
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
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

    #[test]
    #[parallel]
    fn a_new_stock_is_made_whole_and_reads_back() {
        let dir = with_stock();
        assert_eq!(names(dir.path()), stock_names());
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
        assert_eq!(
            files(dir.path()).get("rgb/stash.dat"),
            before.get("rgb/stash.dat")
        );
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

    // every store: the new file written and synced, renamed, then the directory synced; a store of
    // unchanged bytes writes nothing
    #[test]
    #[parallel]
    fn a_store_syncs_before_and_after_the_rename() {
        let dir = with_stock();
        STORE_EVENTS.with_borrow_mut(|e| e.clear());
        let mut runtime = load_rgb_runtime(dir.path()).unwrap();
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
