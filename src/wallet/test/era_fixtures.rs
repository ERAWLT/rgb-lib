//! ERA fork: fixtures that pin the backups an older rev wrote, so a change to how rgb-lib reads a
//! backup cannot quietly stop those backups from restoring.
//!
//! [`generate_backup_fixtures`] writes them. It is run by hand, on a checkout of the rev the
//! fixtures stand for, with this file and its `mod` line in `wallet/test/mod.rs` copied in (the
//! calls it makes have the same signatures there):
//!
//! ```sh
//! ERA_FIXTURE_OUT=<dir> SKIP_INIT=1 cargo test --locked --lib --features esplora,vss -- \
//!   --ignored --exact wallet::test::era_fixtures::generate_backup_fixtures
//! ```
//!
//! `fixtures/d82e21a/` comes from `d82e21a`, the rev the ERA app pinned when the decryption
//! started refusing a stream that ends without its final block (ERA.md, section 5). Read by
//! `wallet::vss::tests::d82e21a_*` and `wallet::backup::tests::d82e21a_*`:
//!
//! - `vss_stream_vectors.txt`: `encrypt_data` of [`stream_plaintext`] for every length in
//!   [`STREAM_VECTOR_LENGTHS`], under [`fixture_signing_key`] and [`fixture_stream_metadata`], one
//!   `<length> <ciphertext hex>` line each;
//! - `vss_backup_*`: the four values a VSS server holds for a backup uploaded with encryption on
//!   (manifest, data, encryption metadata, fingerprint), for a new wallet, encrypted under
//!   [`fixture_signing_key`], and `vss_backup_listing.txt`, what that backup's zip holds;
//! - `file_backup.bin`: `Wallet::backup` of the same wallet with [`FIXTURE_PASSWORD`] (the app's
//!   seal), and `file_backup_listing.txt`, what that rev's own `restore_backup` made of it.
//!
//! A listing has one line per entry, sorted: `<sha256 hex>  <path>` for a file, `dir  <path>/` for
//! a directory, paths from the target directory (so they start with the wallet's fingerprint).

#[cfg(feature = "vss")]
use std::io::Read as _;

#[cfg(feature = "vss")]
use bdk_wallet::bitcoin::secp256k1::SecretKey;
use walkdir::WalkDir;

use super::*;
use crate::utils::hash_bytes_hex;
#[cfg(feature = "vss")]
use crate::wallet::vss::{
    BackupManifest, VSS_CHUNK_SIZE, VssEncryptionMetadata, create_backup_data, encrypt_data,
};

/// The password of `file_backup.bin`.
pub(crate) const FIXTURE_PASSWORD: &str = "era-fixture-seal-password";

#[cfg(feature = "vss")]
/// The lengths of the stream vectors: empty, one byte, a block less one, exactly one and two
/// blocks (whose final block holds no plaintext, only its tag), a block and one byte, and longer.
pub(crate) const STREAM_VECTOR_LENGTHS: [usize; 9] = [0, 1, 238, 239, 240, 477, 478, 479, 1000];

#[cfg(feature = "vss")]
/// The signing key of every VSS fixture.
pub(crate) fn fixture_signing_key() -> SecretKey {
    SecretKey::from_slice(&[0x11; 32]).unwrap()
}

#[cfg(feature = "vss")]
/// The encryption metadata of the stream vectors (a backup draws its own at random).
pub(crate) fn fixture_stream_metadata() -> VssEncryptionMetadata {
    VssEncryptionMetadata {
        salt: hex::encode([0x22u8; 32]),
        nonce: hex::encode([0x33u8; 19]),
        version: 1,
    }
}

#[cfg(feature = "vss")]
/// The plaintext of the stream vector of `len` bytes.
pub(crate) fn stream_plaintext(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 + 3) as u8).collect()
}

/// The directory holding the fixtures of `rev`.
pub(crate) fn fixture_dir(rev: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/wallet/test/fixtures")
        .join(rev)
}

fn listing(mut lines: Vec<String>) -> String {
    lines.sort_by(|a, b| {
        a.split_once("  ")
            .unwrap()
            .1
            .cmp(b.split_once("  ").unwrap().1)
    });
    lines.into_iter().map(|line| format!("{line}\n")).collect()
}

#[cfg(feature = "vss")]
/// The listing of a zip archive's entries.
pub(crate) fn zip_listing(zip: &[u8]) -> String {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
    let mut lines = vec![];
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().to_string();
        if entry.is_dir() {
            lines.push(format!("dir  {name}"));
        } else {
            let mut content = vec![];
            entry.read_to_end(&mut content).unwrap();
            lines.push(format!("{}  {name}", hash_bytes_hex(&content)));
        }
    }
    listing(lines)
}

/// The listing of `target_dir/wallet_name/`, with the restore marker left out (a VSS restore adds
/// it, `vss::VSS_RESTORE_MARKER`; it is not part of any backup).
pub(crate) fn dir_listing(target_dir: &Path, wallet_name: &str) -> String {
    let mut lines = vec![];
    for entry in WalkDir::new(target_dir.join(wallet_name)) {
        let entry = entry.unwrap();
        let name = entry
            .path()
            .strip_prefix(target_dir)
            .unwrap()
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");
        if entry.file_type().is_dir() {
            lines.push(format!("dir  {name}/"));
        } else if entry.file_name() != ".vss_restored" {
            lines.push(format!(
                "{}  {name}",
                hash_bytes_hex(&fs::read(entry.path()).unwrap())
            ));
        }
    }
    listing(lines)
}

#[cfg(feature = "vss")]
#[test]
#[ignore = "writes the fixtures; run by hand on the rev they stand for"]
fn generate_backup_fixtures() {
    let out = PathBuf::from(std::env::var("ERA_FIXTURE_OUT").expect("ERA_FIXTURE_OUT"));
    fs::create_dir_all(&out).unwrap();
    let key = fixture_signing_key();

    let metadata = fixture_stream_metadata();
    let vectors: String = STREAM_VECTOR_LENGTHS
        .iter()
        .map(|&len| {
            let sealed = encrypt_data(&stream_plaintext(len), &key, &metadata, None).unwrap();
            format!("{len} {}\n", hex::encode(sealed))
        })
        .collect();
    fs::write(out.join("vss_stream_vectors.txt"), vectors).unwrap();

    let wallet = get_test_wallet(false, None);
    let wallet_dir = wallet.get_wallet_dir();
    let fingerprint = wallet_dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();

    // what VssBackupClient::upload_backup stores with encryption on (a single chunk)
    let logger = slog::Logger::root(slog::Discard, slog::o!());
    let zip = create_backup_data(&wallet_dir, &logger).unwrap();
    let vss_metadata = VssEncryptionMetadata::new();
    let data = encrypt_data(&zip, &key, &vss_metadata, None).unwrap();
    assert!(data.len() <= VSS_CHUNK_SIZE);
    let manifest = BackupManifest {
        chunk_count: 1,
        total_size: data.len(),
        encrypted: true,
        version: 1,
    };
    fs::write(
        out.join("vss_backup_manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(out.join("vss_backup_data.bin"), &data).unwrap();
    fs::write(
        out.join("vss_backup_metadata.json"),
        serde_json::to_vec(&vss_metadata).unwrap(),
    )
    .unwrap();
    fs::write(out.join("vss_backup_fingerprint.txt"), &fingerprint).unwrap();
    fs::write(out.join("vss_backup_listing.txt"), zip_listing(&zip)).unwrap();

    // the app's seal: Wallet::backup with the default scrypt parameters, and what this rev's own
    // restore_backup makes of it
    let backup = out.join("file_backup.bin");
    wallet
        .backup(backup.to_str().unwrap(), FIXTURE_PASSWORD)
        .unwrap();
    let staging = tempfile::tempdir().unwrap();
    let copy = staging.path().join("seal").join("file_backup.bin");
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(&backup, &copy).unwrap();
    let restored = staging.path().join("restored");
    crate::wallet::restore_backup(
        copy.to_str().unwrap(),
        FIXTURE_PASSWORD,
        restored.to_str().unwrap(),
    )
    .unwrap();
    fs::write(
        out.join("file_backup_listing.txt"),
        dir_listing(&restored, &fingerprint),
    )
    .unwrap();
}
