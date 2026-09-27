//! VSS (Versioned Storage Service) backup module
//!
//! This module provides cloud backup functionality using VSS servers.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bdk_wallet::bitcoin::secp256k1::SecretKey;
// Note: this is the RustCrypto crate (XChaCha20). The similarly named
// `chacha20-poly1305` (rust-bitcoin, stateless-only) is a transitive dep
// of vss-client-ng via bitreq — they are not interchangeable.
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, aead::Aead};
use hkdf::Hkdf;

use serde::{Deserialize, Serialize};
use sha2::Sha256;
use slog::{Logger, debug, info};
use time::OffsetDateTime;
use vss_client::client::VssClient;
use vss_client::error::VssError;
use vss_client::headers::sigs_auth::SigsAuthProvider;
use vss_client::types::{GetObjectRequest, KeyValue, PutObjectRequest};
use vss_client::util::retry::{
    ExponentialBackoffRetryPolicy, MaxAttemptsRetryPolicy, MaxTotalDelayRetryPolicy, RetryPolicy,
};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

use crate::error::Error;
use crate::utils::LOG_FILE;
use crate::utils::setup_logger;
use crate::wallet::backup::stream_be32_nonce;
use crate::wallet::core::WALLET_MANIFEST_FILE;

/// Whether auto-backup uploads block the calling operation or run asynchronously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VssBackupMode {
    /// Upload runs on a spawned tokio task (fire-and-forget). Default.
    #[default]
    Async,
    /// Upload blocks the calling operation until the backup is persisted.
    Blocking,
}

/// Type alias for our retry policy
type VssRetryPolicy =
    MaxTotalDelayRetryPolicy<MaxAttemptsRetryPolicy<ExponentialBackoffRetryPolicy<VssError>>>;

// Encryption constants (matching backup.rs)
const BACKUP_BUFFER_LEN_ENCRYPT: usize = 239;
const BACKUP_BUFFER_LEN_DECRYPT: usize = BACKUP_BUFFER_LEN_ENCRYPT + 16;
const BACKUP_KEY_LENGTH: usize = 32;
const BACKUP_NONCE_LENGTH: usize = 19;
const VSS_BACKUP_VERSION: u8 = 1;

/// Default chunk size for large backups
/// Kept under the upstream vss-client-ng 10-second HTTP timeout
/// so that each chunk PUT completes reliably
pub(crate) const VSS_CHUNK_SIZE: usize = 1024 * 1024; // 1MB

/// Key prefix for backup data
const BACKUP_KEY_DATA: &str = "backup/data";
/// Key prefix for backup metadata (encryption params)
const BACKUP_KEY_METADATA: &str = "backup/metadata";
/// Key prefix for backup manifest (chunk info)
const BACKUP_KEY_MANIFEST: &str = "backup/manifest";
/// Key prefix for backup chunks
const BACKUP_KEY_CHUNK_PREFIX: &str = "backup/chunk/";
/// Key for storing wallet fingerprint separately
const BACKUP_KEY_FINGERPRINT: &str = "backup/fingerprint";
/// Generic directory name used in sanitized (plaintext) backups
const SANITIZED_DIR_NAME: &str = "wallet";
/// Marker dropped in a restored wallet dir until its first successful
/// consistency check, so a failure there can be attributed to the restore.
pub(crate) const VSS_RESTORE_MARKER: &str = ".vss_restored";
/// BDK database filename
const BDK_DB_NAME: &str = "bdk_db";
/// BDK watch-only database filename
const BDK_DB_WO_NAME: &str = "bdk_db_watch_only";

/// Salt length for HKDF key derivation (32 bytes, hex-encoded = 64 chars)
const BACKUP_SALT_LENGTH: usize = 32;

/// Encryption metadata stored alongside encrypted backups
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VssEncryptionMetadata {
    /// Salt used for HKDF key derivation (hex encoded)
    pub salt: String,
    /// Nonce used for encryption
    pub nonce: String,
    /// Version of the encryption format
    pub version: u8,
}

impl Default for VssEncryptionMetadata {
    fn default() -> Self {
        Self::new()
    }
}

impl VssEncryptionMetadata {
    /// Create new encryption metadata with random salt and nonce
    pub fn new() -> Self {
        let salt: [u8; BACKUP_SALT_LENGTH] = rand::random();
        let nonce: [u8; BACKUP_NONCE_LENGTH] = rand::random();

        Self {
            salt: hex::encode(salt),
            nonce: hex::encode(nonce),
            version: VSS_BACKUP_VERSION,
        }
    }

    fn nonce_bytes(&self) -> Result<[u8; BACKUP_NONCE_LENGTH], Error> {
        let bytes = hex::decode(&self.nonce).map_err(|e| Error::Internal {
            details: format!("Invalid nonce hex: {e}"),
        })?;
        // ERA fork: a shorter nonce is an error, not a panic (it can come from a VSS server)
        bytes
            .get(..BACKUP_NONCE_LENGTH)
            .and_then(|prefix| prefix.try_into().ok())
            .ok_or_else(|| Error::Internal {
                details: "Invalid nonce length".to_string(),
            })
    }

    /// ERA fork: `Ok` for metadata as [`Self::new`] makes it (a salt of `BACKUP_SALT_LENGTH`
    /// bytes, a nonce of `BACKUP_NONCE_LENGTH`, both hex), which is all a restore takes from a
    /// server: anything else is not rgb-lib's.
    fn check(&self) -> Result<(), Error> {
        let length = |hex_value: &str| hex::decode(hex_value).map(|bytes| bytes.len()).ok();
        if length(&self.salt) != Some(BACKUP_SALT_LENGTH)
            || length(&self.nonce) != Some(BACKUP_NONCE_LENGTH)
        {
            return Err(Error::VssError {
                details: "the backup's encryption metadata is malformed".to_string(),
            });
        }
        Ok(())
    }
}

/// Configuration for VSS backup service
#[derive(Clone)]
pub struct VssBackupConfig {
    /// VSS server URL
    pub(crate) server_url: String,
    /// Store ID (namespace for this wallet's data)
    pub(crate) store_id: String,
    /// Private key for signing requests and deriving encryption key
    pub(crate) signing_key: SecretKey,
    /// Whether to encrypt data before uploading (default: true)
    pub(crate) encryption_enabled: bool,
    /// Whether to automatically back up after state-changing operations (default: false)
    pub(crate) auto_backup: bool,
    /// Whether auto-backup uploads block the caller or run asynchronously (default: Async)
    pub(crate) backup_mode: VssBackupMode,
}

impl VssBackupConfig {
    /// Create a new VSS backup configuration
    ///
    /// Encryption is enabled by default. The encryption key is derived from the
    /// signing key using HKDF-SHA256, so no separate password is needed.
    pub fn new(server_url: String, store_id: String, signing_key: SecretKey) -> Self {
        Self {
            server_url,
            store_id,
            signing_key,
            encryption_enabled: true,
            auto_backup: false,
            backup_mode: VssBackupMode::default(),
        }
    }

    /// Set encryption enabled/disabled
    pub fn with_encryption(mut self, enabled: bool) -> Self {
        self.encryption_enabled = enabled;
        self
    }

    /// Enable or disable automatic backups after state-changing operations
    pub fn with_auto_backup(mut self, enabled: bool) -> Self {
        self.auto_backup = enabled;
        self
    }

    /// Set the auto-backup mode (Async or Blocking)
    pub fn with_backup_mode(mut self, mode: VssBackupMode) -> Self {
        self.backup_mode = mode;
        self
    }
}

/// Backup manifest for tracking chunked backups
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupManifest {
    /// Number of chunks
    pub chunk_count: usize,
    /// Total size in bytes
    pub total_size: usize,
    /// Whether the backup is encrypted
    pub encrypted: bool,
    /// Backup version
    pub version: u8,
}

/// VSS backup client wrapper
pub struct VssBackupClient {
    client: VssClient<VssRetryPolicy>,
    store_id: String,
    encryption_enabled: bool,
    signing_key: SecretKey,
    auto_backup: bool,
    backup_mode: VssBackupMode,
    runtime: Option<tokio::runtime::Runtime>,
    last_auto_backup_error: std::sync::Mutex<Option<String>>,
}

impl Drop for VssBackupClient {
    fn drop(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

impl VssBackupClient {
    /// Create a new VSS backup client
    pub fn new(config: VssBackupConfig) -> Result<Self, Error> {
        let auth_provider = SigsAuthProvider::new(config.signing_key, HashMap::new());

        let retry_policy = ExponentialBackoffRetryPolicy::new(Duration::from_millis(100))
            .with_max_attempts(3)
            .with_max_total_delay(Duration::from_secs(5));

        let client =
            VssClient::new_with_headers(config.server_url, retry_policy, Arc::new(auth_provider));

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| Error::Internal {
                details: format!("Failed to create tokio runtime: {e}"),
            })?;

        Ok(Self {
            client,
            store_id: config.store_id,
            encryption_enabled: config.encryption_enabled,
            signing_key: config.signing_key,
            auto_backup: config.auto_backup,
            backup_mode: config.backup_mode,
            runtime: Some(runtime),
            last_auto_backup_error: std::sync::Mutex::new(None),
        })
    }

    /// Error message of the most recent failed auto-backup, cleared on the
    /// next successful one. `None` when the last auto-backup succeeded or none
    /// ran yet.
    pub fn last_auto_backup_error(&self) -> Option<String> {
        self.last_auto_backup_error.lock().unwrap().clone()
    }

    pub(crate) fn record_auto_backup_result(&self, error: Option<String>) {
        *self.last_auto_backup_error.lock().unwrap() = error;
    }

    /// Get a handle to the client's tokio runtime
    pub fn handle(&self) -> &tokio::runtime::Handle {
        self.runtime
            .as_ref()
            .expect("runtime not available")
            .handle()
    }

    /// Upload backup data to VSS server
    ///
    /// If encryption is enabled, data will be encrypted before upload.
    /// The encryption key is derived from the signing key using HKDF-SHA256.
    ///
    /// If encryption is disabled, the backup is sanitized to exclude sensitive
    /// data (master fingerprint in paths, BDK database with xpubs). The
    /// fingerprint is stored separately for use during restore.
    ///
    /// Returns the version number of the uploaded backup.
    pub async fn upload_backup(&self, data: Vec<u8>) -> Result<i64, Error> {
        // Extract fingerprint from zip data
        let fingerprint = get_fingerprint_from_zip_bytes(&data)?;

        // Encrypt or sanitize data
        let (upload_data, encryption_metadata) = if self.encryption_enabled {
            let metadata = VssEncryptionMetadata::new();
            let encrypted = encrypt_data(&data, &self.signing_key, &metadata, None)?;
            (encrypted, Some(metadata))
        } else {
            // Sanitize for plaintext: remove fingerprint from paths, exclude bdk_db
            let (sanitized, _) = sanitize_zip_for_plaintext(&data)?;
            (sanitized, None)
        };

        let total_size = upload_data.len();

        if total_size <= VSS_CHUNK_SIZE {
            // Single chunk upload
            self.upload_single(upload_data, encryption_metadata, &fingerprint)
                .await
        } else {
            // Chunked upload for large backups
            self.upload_chunked(upload_data, encryption_metadata, &fingerprint)
                .await
        }
    }

    /// Upload a single backup (non-chunked)
    async fn upload_single(
        &self,
        data: Vec<u8>,
        encryption_metadata: Option<VssEncryptionMetadata>,
        fingerprint: &str,
    ) -> Result<i64, Error> {
        // VSS versioning: version field is the expected current version (optimistic locking)
        // For new keys: version = 0 (no current version)
        // For updates: version = current_version (server will increment to current_version + 1)
        let data_version = self
            .get_current_version(BACKUP_KEY_DATA)
            .await?
            .unwrap_or(0);

        let manifest = BackupManifest {
            chunk_count: 1,
            total_size: data.len(),
            encrypted: encryption_metadata.is_some(),
            version: VSS_BACKUP_VERSION,
        };
        let manifest_json = serde_json::to_vec(&manifest).map_err(|e| Error::Internal {
            details: format!("Failed to serialize manifest: {e}"),
        })?;

        let manifest_version = self
            .get_current_version(BACKUP_KEY_MANIFEST)
            .await?
            .unwrap_or(0);

        let fingerprint_version = self
            .get_current_version(BACKUP_KEY_FINGERPRINT)
            .await?
            .unwrap_or(0);

        let mut transaction_items = vec![
            KeyValue {
                key: BACKUP_KEY_DATA.to_string(),
                version: data_version,
                value: data,
            },
            KeyValue {
                key: BACKUP_KEY_MANIFEST.to_string(),
                version: manifest_version,
                value: manifest_json,
            },
            KeyValue {
                key: BACKUP_KEY_FINGERPRINT.to_string(),
                version: fingerprint_version,
                value: fingerprint.as_bytes().to_vec(),
            },
        ];

        // Add encryption metadata if present
        if let Some(metadata) = encryption_metadata {
            let metadata_json = serde_json::to_vec(&metadata).map_err(|e| Error::Internal {
                details: format!("Failed to serialize encryption metadata: {e}"),
            })?;
            let metadata_version = self
                .get_current_version(BACKUP_KEY_METADATA)
                .await?
                .unwrap_or(0);
            transaction_items.push(KeyValue {
                key: BACKUP_KEY_METADATA.to_string(),
                version: metadata_version,
                value: metadata_json,
            });
        }

        let request = PutObjectRequest {
            store_id: self.store_id.clone(),
            global_version: None,
            transaction_items,
            delete_items: vec![],
        };

        self.client
            .put_object(&request)
            .await
            .map_err(vss_error_to_rgb_error)?;

        // Server increments version, so return expected new version
        Ok(data_version + 1)
    }

    /// Upload a chunked backup for large data
    ///
    /// Each chunk is uploaded in a separate request to avoid sending the entire
    /// backup in a single HTTP body. The manifest is uploaded last so that a
    /// partial failure leaves the previous manifest (and thus the previous
    /// valid backup) intact. Orphaned chunks from a failed partial upload are
    /// harmlessly overwritten on the next backup attempt.
    async fn upload_chunked(
        &self,
        data: Vec<u8>,
        encryption_metadata: Option<VssEncryptionMetadata>,
        fingerprint: &str,
    ) -> Result<i64, Error> {
        let chunks: Vec<&[u8]> = data.chunks(VSS_CHUNK_SIZE).collect();
        let chunk_count = chunks.len();
        let total_size = data.len();

        // Upload each chunk in a separate request
        for (i, chunk) in chunks.iter().enumerate() {
            let key = format!("{}{}", BACKUP_KEY_CHUNK_PREFIX, i);
            let chunk_version = self.get_current_version(&key).await?.unwrap_or(0);

            let request = PutObjectRequest {
                store_id: self.store_id.clone(),
                global_version: None,
                transaction_items: vec![KeyValue {
                    key,
                    version: chunk_version,
                    value: chunk.to_vec(),
                }],
                delete_items: vec![],
            };

            self.client
                .put_object(&request)
                .await
                .map_err(vss_error_to_rgb_error)?;
        }

        // Upload manifest + fingerprint + metadata atomically (small, logically related)
        let manifest = BackupManifest {
            chunk_count,
            total_size,
            encrypted: encryption_metadata.is_some(),
            version: VSS_BACKUP_VERSION,
        };
        let manifest_json = serde_json::to_vec(&manifest).map_err(|e| Error::Internal {
            details: format!("Failed to serialize manifest: {e}"),
        })?;
        let manifest_version = self
            .get_current_version(BACKUP_KEY_MANIFEST)
            .await?
            .unwrap_or(0);

        let fingerprint_version = self
            .get_current_version(BACKUP_KEY_FINGERPRINT)
            .await?
            .unwrap_or(0);

        let mut transaction_items = vec![
            KeyValue {
                key: BACKUP_KEY_MANIFEST.to_string(),
                version: manifest_version,
                value: manifest_json,
            },
            KeyValue {
                key: BACKUP_KEY_FINGERPRINT.to_string(),
                version: fingerprint_version,
                value: fingerprint.as_bytes().to_vec(),
            },
        ];

        if let Some(metadata) = encryption_metadata {
            let metadata_json = serde_json::to_vec(&metadata).map_err(|e| Error::Internal {
                details: format!("Failed to serialize encryption metadata: {e}"),
            })?;
            let metadata_version = self
                .get_current_version(BACKUP_KEY_METADATA)
                .await?
                .unwrap_or(0);
            transaction_items.push(KeyValue {
                key: BACKUP_KEY_METADATA.to_string(),
                version: metadata_version,
                value: metadata_json,
            });
        }

        let request = PutObjectRequest {
            store_id: self.store_id.clone(),
            global_version: None,
            transaction_items,
            delete_items: vec![],
        };

        self.client
            .put_object(&request)
            .await
            .map_err(vss_error_to_rgb_error)?;

        Ok(manifest_version + 1)
    }

    /// Download backup data from VSS server
    ///
    /// If the backup was encrypted, it will be decrypted using a key derived from
    /// the signing key via HKDF-SHA256.
    pub async fn download_backup(&self) -> Result<Vec<u8>, Error> {
        // First get the manifest
        let manifest = self.get_manifest().await?;
        self.download_backup_with(&manifest).await
    }

    /// ERA fork: [`Self::download_backup`] by a manifest the caller read, so a caller that also
    /// acts on the manifest acts on the same answer the download followed.
    ///
    /// The manifest is checked before anything is downloaded ([`check_manifest`]), no buffer is
    /// sized by its numbers, and what is downloaded must add up to its `total_size` exactly; an
    /// encrypted backup's metadata must be as rgb-lib writes it.
    pub(crate) async fn download_backup_with(
        &self,
        manifest: &BackupManifest,
    ) -> Result<Vec<u8>, Error> {
        check_manifest(manifest)?;

        // Download the raw data
        let raw_data = if manifest.chunk_count == 1 {
            self.download_single(manifest).await?
        } else {
            self.download_chunked(manifest).await?
        };

        // Decrypt if the backup was encrypted
        if manifest.encrypted {
            let metadata = self.get_encryption_metadata().await?;
            metadata.check()?;
            decrypt_data(&raw_data, &self.signing_key, &metadata, None)
        } else {
            Ok(raw_data)
        }
    }

    /// Download a single backup (non-chunked)
    async fn download_single(&self, manifest: &BackupManifest) -> Result<Vec<u8>, Error> {
        let request = GetObjectRequest {
            store_id: self.store_id.clone(),
            key: BACKUP_KEY_DATA.to_string(),
        };

        let response = self
            .client
            .get_object(&request)
            .await
            .map_err(vss_error_to_rgb_error)?;

        let data = response
            .value
            .map(|kv| kv.value)
            .ok_or(Error::VssBackupNotFound)?;
        // ERA fork: the manifest says how much there is
        if data.len() != manifest.total_size {
            return Err(backup_size_mismatch());
        }
        Ok(data)
    }

    /// Download a chunked backup
    async fn download_chunked(&self, manifest: &BackupManifest) -> Result<Vec<u8>, Error> {
        // ERA fork: not sized by the manifest's numbers (a server's), and never past its total
        let mut data = Vec::new();

        for i in 0..manifest.chunk_count {
            let key = format!("{}{}", BACKUP_KEY_CHUNK_PREFIX, i);
            let request = GetObjectRequest {
                store_id: self.store_id.clone(),
                key,
            };

            let response = self
                .client
                .get_object(&request)
                .await
                .map_err(vss_error_to_rgb_error)?;

            let chunk = response
                .value
                .map(|kv| kv.value)
                .ok_or(Error::VssBackupNotFound)?;

            if chunk.is_empty() || chunk.len() > manifest.total_size - data.len() {
                return Err(backup_size_mismatch());
            }
            data.extend(chunk);
        }

        if data.len() != manifest.total_size {
            return Err(backup_size_mismatch());
        }
        Ok(data)
    }

    /// Get the backup manifest
    async fn get_manifest(&self) -> Result<BackupManifest, Error> {
        let request = GetObjectRequest {
            store_id: self.store_id.clone(),
            key: BACKUP_KEY_MANIFEST.to_string(),
        };

        let response = self
            .client
            .get_object(&request)
            .await
            .map_err(vss_error_to_rgb_error)?;

        let manifest_bytes = response
            .value
            .map(|kv| kv.value)
            .ok_or(Error::VssBackupNotFound)?;

        serde_json::from_slice(&manifest_bytes).map_err(|e| Error::Internal {
            details: format!("Failed to parse backup manifest: {e}"),
        })
    }

    /// Get the encryption metadata
    async fn get_encryption_metadata(&self) -> Result<VssEncryptionMetadata, Error> {
        let request = GetObjectRequest {
            store_id: self.store_id.clone(),
            key: BACKUP_KEY_METADATA.to_string(),
        };

        let response = self
            .client
            .get_object(&request)
            .await
            .map_err(vss_error_to_rgb_error)?;

        let metadata_bytes = response
            .value
            .map(|kv| kv.value)
            .ok_or(Error::VssBackupNotFound)?;

        serde_json::from_slice(&metadata_bytes).map_err(|e| Error::Internal {
            details: format!("Failed to parse encryption metadata: {e}"),
        })
    }

    /// Get the current version of a backup
    ///
    /// Returns `None` if no backup exists, otherwise returns the version number.
    pub async fn get_backup_version(&self) -> Result<Option<i64>, Error> {
        self.get_current_version(BACKUP_KEY_MANIFEST).await
    }

    /// Get the wallet fingerprint stored on the server
    async fn get_fingerprint(&self) -> Result<String, Error> {
        let request = GetObjectRequest {
            store_id: self.store_id.clone(),
            key: BACKUP_KEY_FINGERPRINT.to_string(),
        };

        let response = self
            .client
            .get_object(&request)
            .await
            .map_err(vss_error_to_rgb_error)?;

        let fingerprint_bytes = response
            .value
            .map(|kv| kv.value)
            .ok_or(Error::VssBackupNotFound)?;

        String::from_utf8(fingerprint_bytes).map_err(|e| Error::Internal {
            details: format!("Invalid fingerprint encoding: {e}"),
        })
    }

    /// Get the current version of a key
    async fn get_current_version(&self, key: &str) -> Result<Option<i64>, Error> {
        let request = GetObjectRequest {
            store_id: self.store_id.clone(),
            key: key.to_string(),
        };

        match self.client.get_object(&request).await {
            Ok(response) => Ok(response.value.map(|kv| kv.version)),
            Err(VssError::NoSuchKeyError(_)) => Ok(None),
            Err(e) => Err(vss_error_to_rgb_error(e)),
        }
    }

    /// Delete the backup from VSS server
    pub async fn delete_backup(&self) -> Result<(), Error> {
        // Get manifest to know what to delete
        let manifest = match self.get_manifest().await {
            Ok(m) => m,
            Err(_) => return Ok(()), // No backup to delete
        };

        let mut delete_items = vec![];

        // Delete chunks if any
        if manifest.chunk_count > 1 {
            for i in 0..manifest.chunk_count {
                let key = format!("{}{}", BACKUP_KEY_CHUNK_PREFIX, i);
                if let Some(version) = self.get_current_version(&key).await? {
                    delete_items.push(KeyValue {
                        key,
                        version,
                        value: vec![],
                    });
                }
            }
        } else {
            // Delete single data key
            if let Some(version) = self.get_current_version(BACKUP_KEY_DATA).await? {
                delete_items.push(KeyValue {
                    key: BACKUP_KEY_DATA.to_string(),
                    version,
                    value: vec![],
                });
            }
        }

        // Delete manifest
        if let Some(version) = self.get_current_version(BACKUP_KEY_MANIFEST).await? {
            delete_items.push(KeyValue {
                key: BACKUP_KEY_MANIFEST.to_string(),
                version,
                value: vec![],
            });
        }

        // Delete metadata if exists
        if let Some(version) = self.get_current_version(BACKUP_KEY_METADATA).await? {
            delete_items.push(KeyValue {
                key: BACKUP_KEY_METADATA.to_string(),
                version,
                value: vec![],
            });
        }

        // Delete fingerprint if exists
        if let Some(version) = self.get_current_version(BACKUP_KEY_FINGERPRINT).await? {
            delete_items.push(KeyValue {
                key: BACKUP_KEY_FINGERPRINT.to_string(),
                version,
                value: vec![],
            });
        }

        if delete_items.is_empty() {
            return Ok(());
        }

        let request = PutObjectRequest {
            store_id: self.store_id.clone(),
            global_version: None,
            transaction_items: vec![],
            delete_items,
        };

        self.client
            .put_object(&request)
            .await
            .map_err(vss_error_to_rgb_error)?;

        Ok(())
    }

    /// Check if encryption is enabled
    #[must_use]
    pub fn encryption_enabled(&self) -> bool {
        self.encryption_enabled
    }

    /// Check if auto-backup is enabled
    #[must_use]
    pub fn auto_backup(&self) -> bool {
        self.auto_backup
    }

    /// Get the configured backup mode
    #[must_use]
    pub fn backup_mode(&self) -> VssBackupMode {
        self.backup_mode
    }
}

/// Default HKDF info string used by rgb-lib's wallet backup encryption.
///
/// Downstream callers that re-use [`derive_encryption_key`] / [`encrypt_data`] /
/// [`decrypt_data`] for a different purpose (e.g. encrypting LDK KVStore
/// values rather than wallet zips) should pass their own domain-separation tag
/// via the `info` parameter so that the derived keys differ even when the
/// signing key and salt happen to match.
const HKDF_INFO: &[u8] = b"rgb-lib-vss-backup-encryption-v1";

/// Derive an encryption key from a signing key using HKDF-SHA256.
///
/// When `info` is `None`, rgb-lib's default domain-separation tag
/// (`b"rgb-lib-vss-backup-encryption-v1"`) is used — that's the right choice
/// when the caller is encrypting data that will round-trip through rgb-lib's
/// own backup helpers. Pass `Some(tag)` to supply a custom HKDF info string
/// when encrypting unrelated data (e.g. LDK channel-state values) so that the
/// derived key is distinct from rgb-lib's wallet-backup key for the same
/// signing key and metadata.
pub fn derive_encryption_key(
    signing_key: &SecretKey,
    metadata: &VssEncryptionMetadata,
    info: Option<&[u8]>,
) -> Result<Key, Error> {
    let salt_bytes = hex::decode(&metadata.salt).map_err(|e| Error::Internal {
        details: format!("Invalid salt hex: {e}"),
    })?;

    let hk = Hkdf::<Sha256>::new(Some(&salt_bytes), &signing_key.secret_bytes());

    let mut key_bytes = [0u8; BACKUP_KEY_LENGTH];
    hk.expand(info.unwrap_or(HKDF_INFO), &mut key_bytes)
        .map_err(|e| Error::Internal {
            details: format!("HKDF expansion failed: {e}"),
        })?;

    Ok(Key::from(key_bytes))
}

/// Encrypt data using XChaCha20-Poly1305 with a key derived from `signing_key`
/// via HKDF-SHA256 (see [`derive_encryption_key`] for the `info` parameter).
///
/// The same `signing_key`, `metadata`, and `info` must be passed to
/// [`decrypt_data`] for the round-trip to succeed.
pub fn encrypt_data(
    data: &[u8],
    signing_key: &SecretKey,
    metadata: &VssEncryptionMetadata,
    info: Option<&[u8]>,
) -> Result<Vec<u8>, Error> {
    let key = derive_encryption_key(signing_key, metadata, info)?;
    let aead = XChaCha20Poly1305::new(&key);
    let nonce_prefix = metadata.nonce_bytes()?;
    let mut position: u32 = 0;
    let mut encrypted = Vec::new();
    let mut buffer = [0u8; BACKUP_BUFFER_LEN_ENCRYPT];
    let mut reader = std::io::Cursor::new(data);

    loop {
        let read_count = reader.read(&mut buffer).map_err(|e| Error::Internal {
            details: format!("Failed to read data: {e}"),
        })?;
        let is_last = read_count != BACKUP_BUFFER_LEN_ENCRYPT;
        if !is_last && position == u32::MAX {
            return Err(Error::Internal {
                details: "data too large".to_string(),
            });
        }

        let nonce = stream_be32_nonce(&nonce_prefix, position, is_last);
        let ciphertext =
            aead.encrypt(&nonce, &buffer[..read_count])
                .map_err(|e| Error::Internal {
                    details: format!("Encryption error: {e}"),
                })?;
        encrypted.extend(ciphertext);

        if is_last {
            break;
        }
        position += 1;
    }

    Ok(encrypted)
}

/// Decrypt data using XChaCha20-Poly1305 with a key derived from `signing_key`
/// via HKDF-SHA256 (see [`derive_encryption_key`] for the `info` parameter).
///
/// The `signing_key`, `metadata`, and `info` must match what was passed to
/// [`encrypt_data`]; otherwise the AEAD authentication tag check fails and an
/// `Error::VssError` is returned.
///
/// ERA fork: so is data that ends without its final block (cut short, or empty): each block is
/// authenticated with its position and whether it is the last, so what this returns is all of
/// what `encrypt_data` sealed under this key and metadata, in order. It says nothing of when: an
/// older backup, served with its own metadata, decrypts just as well.
pub fn decrypt_data(
    encrypted: &[u8],
    signing_key: &SecretKey,
    metadata: &VssEncryptionMetadata,
    info: Option<&[u8]>,
) -> Result<Vec<u8>, Error> {
    let key = derive_encryption_key(signing_key, metadata, info)?;
    let aead = XChaCha20Poly1305::new(&key);
    let nonce_prefix = metadata.nonce_bytes()?;
    let mut position: u32 = 0;
    let mut decrypted = Vec::new();
    let mut buffer = [0u8; BACKUP_BUFFER_LEN_DECRYPT];
    let mut reader = std::io::Cursor::new(encrypted);

    loop {
        let read_count = reader.read(&mut buffer).map_err(|e| Error::Internal {
            details: format!("Failed to read data: {e}"),
        })?;

        if read_count == BACKUP_BUFFER_LEN_DECRYPT {
            if position == u32::MAX {
                return Err(Error::Internal {
                    details: "data too large".to_string(),
                });
            }
            let nonce = stream_be32_nonce(&nonce_prefix, position, false);
            let cleartext =
                aead.decrypt(&nonce, buffer.as_slice())
                    .map_err(|_| Error::VssError {
                        details: "decryption failed: wrong signing key or corrupted data"
                            .to_string(),
                    })?;
            decrypted.extend(cleartext);
        } else if read_count == 0 {
            // ERA fork: the data ends without its final block, the only short one, which
            // encrypt_data always writes (a tag alone for data that fills its last block, and for
            // no data at all). What came out is a prefix of the data, not the data.
            return Err(Error::VssError {
                details: "decryption failed: the data ends without its final block (truncated)"
                    .to_string(),
            });
        } else {
            let nonce = stream_be32_nonce(&nonce_prefix, position, true);
            let cleartext =
                aead.decrypt(&nonce, &buffer[..read_count])
                    .map_err(|_| Error::VssError {
                        details: "decryption failed: wrong signing key or corrupted data"
                            .to_string(),
                    })?;
            decrypted.extend(cleartext);
            break;
        }
        position += 1;
    }

    Ok(decrypted)
}

/// ERA fork: the largest backup a restore downloads, in bytes (as stored: encrypted, or sanitized
/// plaintext). A wallet backup is a few megabytes; the cap keeps a server's manifest from making
/// the restore hold more than this in memory, and each response is capped at 1 GiB by vss-client
/// itself.
pub(crate) const MAX_VSS_BACKUP_SIZE: usize = 256 * 1024 * 1024;

/// ERA fork: `Ok` if `manifest` describes a backup rgb-lib could have uploaded and a restore will
/// download: between one byte and [`MAX_VSS_BACKUP_SIZE`], in no more chunks than bytes (an upload
/// never stores an empty chunk; no chunk at all adds up to nothing, which the download refuses).
/// Its numbers come from the server.
fn check_manifest(manifest: &BackupManifest) -> Result<(), Error> {
    if manifest.total_size == 0
        || manifest.total_size > MAX_VSS_BACKUP_SIZE
        || manifest.chunk_count > manifest.total_size
    {
        return Err(Error::VssError {
            details: format!(
                "the backup manifest describes no backup a restore takes ({} bytes in {} chunks, \
                 at most {MAX_VSS_BACKUP_SIZE} bytes)",
                manifest.total_size, manifest.chunk_count
            ),
        });
    }
    Ok(())
}

/// ERA fork: what the server holds does not add up to its manifest's `total_size`.
fn backup_size_mismatch() -> Error {
    Error::VssError {
        details: "the backup's size is not the one its manifest gives".to_string(),
    }
}

/// Convert VSS error to RGB-lib error
fn vss_error_to_rgb_error(e: VssError) -> Error {
    match e {
        VssError::NoSuchKeyError(_) => Error::VssBackupNotFound,
        VssError::ConflictError(msg) => Error::VssVersionConflict { details: msg },
        VssError::AuthError(msg) => Error::VssAuth { details: msg },
        VssError::InternalServerError(msg) => Error::VssError {
            details: format!("Server error: {msg}"),
        },
        VssError::InvalidRequestError(msg) => Error::VssError {
            details: format!("Invalid request: {msg}"),
        },
        _ => Error::VssError {
            details: e.to_string(),
        },
    }
}

/// VSS backup info returned by `vss_backup_info()`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VssBackupInfo {
    /// Whether a backup exists on the server
    pub backup_exists: bool,
    /// Server-side version of the backup
    pub server_version: Option<i64>,
    /// Whether the local wallet has changes since last backup
    pub backup_required: bool,
    /// Error of the most recent failed auto-backup, if the last one failed
    pub last_auto_backup_error: Option<String>,
}

/// Create a zip archive of a wallet directory in memory
fn zip_wallet_to_bytes(wallet_dir: &Path, logger: &Logger) -> Result<Vec<u8>, Error> {
    let mut buffer = std::io::Cursor::new(Vec::new());

    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Zstd);
        let mut file_buffer = [0u8; 4096];

        // Get the parent directory to preserve wallet fingerprint in zip
        let prefix = wallet_dir.parent().ok_or_else(|| Error::Internal {
            details: "Wallet directory has no parent".to_string(),
        })?;

        let entry_iterator = WalkDir::new(wallet_dir).into_iter().filter_map(|e| e.ok());

        for entry in entry_iterator {
            let path = entry.path();
            let name = path.strip_prefix(prefix).map_err(|e| Error::Internal {
                details: format!("Failed to strip prefix: {e}"),
            })?;
            let name_str = name.to_str().ok_or_else(|| Error::Internal {
                details: "Invalid path encoding".to_string(),
            })?;

            if path.is_file() {
                // Skip log file
                if path.ends_with(LOG_FILE) {
                    continue;
                }
                debug!(logger, "VSS backup: adding file {:?}", name);
                zip.start_file(name_str, options)
                    .map_err(|e| Error::Internal {
                        details: format!("Failed to add file to zip: {e}"),
                    })?;

                let mut f = fs::File::open(path)?;
                loop {
                    let read_count = f.read(&mut file_buffer)?;
                    if read_count != 0 {
                        zip.write_all(&file_buffer[..read_count])?;
                    } else {
                        break;
                    }
                }
            } else if !name.as_os_str().is_empty() {
                debug!(logger, "VSS backup: adding directory {:?}", name);
                zip.add_directory(name_str, options)
                    .map_err(|e| Error::Internal {
                        details: format!("Failed to add directory to zip: {e}"),
                    })?;
            }
        }

        zip.finish().map_err(|e| Error::Internal {
            details: format!("Failed to finalize zip: {e}"),
        })?;
    }

    Ok(buffer.into_inner())
}

/// Extract the wallet fingerprint from a zip archive
///
/// Wallet fingerprints are 4-byte BIP32 fingerprints displayed as 8 lowercase
/// hex characters (e.g., `"a1b2c3d4"`).
fn get_fingerprint_from_zip_bytes(data: &[u8]) -> Result<String, Error> {
    let reader = std::io::Cursor::new(data);
    let archive = zip::ZipArchive::new(reader).map_err(|e| Error::Internal {
        details: format!("Failed to read zip archive: {e}"),
    })?;

    let first_entry = archive.name_for_index(0).unwrap_or_default();
    let fingerprint = first_entry.trim_end_matches('/').to_string();

    // Validate: wallet fingerprints are 8 hex characters (4 bytes)
    if fingerprint.len() != 8 || hex::decode(&fingerprint).is_err() {
        return Err(Error::Internal {
            details: format!("Invalid wallet fingerprint in zip: '{fingerprint}'"),
        });
    }

    Ok(fingerprint)
}

/// Check if a zip entry path is a BDK database file (contains xpubs in descriptors)
///
/// Matches the `bdk_db`/`bdk_db_watch_only` stems and any recovery sidecar the store
/// leaves behind (`bdk_db.corrupt[.N]`, `bdk_db.recovering`, …): those copies hold full
/// descriptors and must never ride into a plaintext backup.
fn is_bdk_db_file(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let stem = filename.split('.').next().unwrap_or(filename);
    stem == BDK_DB_NAME || stem == BDK_DB_WO_NAME
}

/// Check if a zip entry path is the wallet manifest, or a temp copy orphaned by a crash
/// mid-write (contains xpubs and the fingerprint)
fn is_wallet_manifest_file(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    filename == WALLET_MANIFEST_FILE || filename == format!("{WALLET_MANIFEST_FILE}.tmp")
}

/// Sanitize a wallet zip for plaintext (unencrypted) backup
///
/// Removes sensitive data that should not be stored unencrypted:
/// - Replaces the master fingerprint directory name with a generic "wallet/" name
/// - Excludes BDK database files (bdk_db, bdk_db_watch_only) which contain xpubs
/// - Excludes the wallet manifest, which contains the xpubs and fingerprint in plaintext;
///   the first `Wallet::new` after restore rewrites it
///
/// Returns the sanitized zip bytes and the extracted fingerprint.
fn sanitize_zip_for_plaintext(data: &[u8]) -> Result<(Vec<u8>, String), Error> {
    let fingerprint = get_fingerprint_from_zip_bytes(data)?;

    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| Error::Internal {
        details: format!("Failed to read zip archive: {e}"),
    })?;

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip_writer = zip::ZipWriter::new(&mut buffer);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Zstd);

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| Error::Internal {
                details: format!("Failed to read zip entry: {e}"),
            })?;

            // ZIP APPNOTE mandates '/' separators; a Windows-produced archive may carry
            // backslashes ("fp\bdk_db"), which the separator-naive predicates below would miss
            // and leak unredacted. Normalize once so every predicate sees canonical names.
            let original_name = file.name().replace('\\', "/");

            // Skip files carrying xpubs or the fingerprint in their contents
            if is_bdk_db_file(&original_name) || is_wallet_manifest_file(&original_name) {
                continue;
            }

            // Replace fingerprint with generic directory name
            let sanitized_name = original_name.replacen(&fingerprint, SANITIZED_DIR_NAME, 1);

            if file.is_dir() {
                zip_writer
                    .add_directory(&sanitized_name, options)
                    .map_err(|e| Error::Internal {
                        details: format!("Failed to add directory to sanitized zip: {e}"),
                    })?;
            } else {
                zip_writer
                    .start_file(&sanitized_name, options)
                    .map_err(|e| Error::Internal {
                        details: format!("Failed to add file to sanitized zip: {e}"),
                    })?;
                std::io::copy(&mut file, &mut zip_writer)?;
            }
        }

        zip_writer.finish().map_err(|e| Error::Internal {
            details: format!("Failed to finalize sanitized zip: {e}"),
        })?;
    }

    Ok((buffer.into_inner(), fingerprint))
}

/// ERA fork: the path of zip entry `name` relative to the directory `dir_name` at the top of the
/// archive, or `None` for an entry outside it. Separators are `/` and `\` (a Windows-made archive
/// may carry the latter). An absolute name, a `..` that climbs out of the archive, a NUL, or a
/// component holding a `:` (a drive or a stream on Windows) is inside nothing.
fn entry_in_dir(name: &str, dir_name: &str) -> Option<PathBuf> {
    if name.contains('\0') || name.starts_with(['/', '\\']) {
        return None;
    }
    let mut parts: Vec<&str> = vec![];
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part if part.contains(':') => return None,
            part => parts.push(part),
        }
    }
    match parts.split_first() {
        Some((first, rest)) if *first == dir_name => Some(rest.iter().collect()),
        _ => None,
    }
}

/// ERA fork: extract the entries of the wallet directory `dir_name` at the top of the zip `data`
/// into `wallet_dir`, and nothing else (upstream extracted every entry into the target directory,
/// next to the other wallets there). Returns how many files were extracted. Regular files and
/// directories only: an entry the archive marks as a symlink is written as a file holding its
/// target, as upstream wrote it.
fn unzip_wallet_dir(
    data: &[u8],
    dir_name: &str,
    wallet_dir: &Path,
    logger: &Logger,
) -> Result<usize, Error> {
    let reader = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| Error::Internal {
        details: format!("Failed to read zip archive: {e}"),
    })?;

    let (mut files, mut skipped) = (0, 0);
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| Error::Internal {
            details: format!("Failed to read zip entry: {e}"),
        })?;

        let Some(relative) = entry_in_dir(file.name(), dir_name) else {
            skipped += 1;
            continue;
        };
        let outpath = wallet_dir.join(&relative);

        if file.name().ends_with(['/', '\\']) {
            debug!(logger, "VSS restore: creating directory {:?}", relative);
            fs::create_dir_all(&outpath)?;
        } else if relative.as_os_str().is_empty() {
            // a file where the wallet directory is
            skipped += 1;
        } else {
            debug!(
                logger,
                "VSS restore: extracting file {:?} ({} bytes)",
                relative,
                file.size()
            );
            if let Some(p) = outpath.parent() {
                fs::create_dir_all(p)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
            files += 1;
        }
    }
    if skipped > 0 {
        info!(
            logger,
            "VSS restore: {skipped} entries outside the wallet directory not extracted"
        );
    }

    Ok(files)
}

/// ERA fork: the directory a restore extracts into, next to the wallet directory it becomes once
/// the extraction is complete ([`Self::finish`]); removed with whatever it holds otherwise, so a
/// restore that fails leaves no partial wallet directory behind.
struct Staging {
    path: Option<PathBuf>,
}

impl Staging {
    fn new(target_dir: &Path, fingerprint: &str) -> Result<Self, Error> {
        let path = target_dir.join(format!(
            ".vss_restore_{fingerprint}_{}",
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::create_dir(&path)?;
        Ok(Self { path: Some(path) })
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("set until finish")
    }

    /// Rename the staging directory to `wallet_dir`.
    fn finish(mut self, wallet_dir: &Path) -> Result<(), Error> {
        let path = self.path.take().expect("set until finish");
        if let Err(e) = fs::rename(&path, wallet_dir) {
            let _ = fs::remove_dir_all(&path);
            return Err(e.into());
        }
        Ok(())
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// Restore a wallet from VSS backup
///
/// Downloads the backup from the VSS server and extracts it to the target directory.
/// For encrypted backups, the fingerprint is embedded in the zip entry paths.
/// For plaintext backups, the fingerprint is fetched from a separate server key
/// and the sanitized "wallet/" directory is renamed to the actual fingerprint.
///
/// ERA fork: the manifest is read once, so the decryption and the extraction follow the same
/// answer. With encryption enabled in `config` (the default), a backup the manifest marks as
/// unencrypted is [`Error::VssBackupUnencrypted`], before any of it is downloaded: decrypting is
/// what authenticates a backup (to restore a plaintext one, disable encryption in `config`). The
/// name the server gives the wallet is taken only if it is a wallet fingerprint (8 hex
/// characters); anything else is [`Error::VssError`] before any of the backup is written. An
/// encrypted backup must name that same wallet inside (ignoring case), else
/// [`Error::FingerprintMismatch`]. Only the archive's wallet directory is extracted, into a
/// staging directory renamed once complete to `<target_dir>/<name>`, the name being the one the
/// backup carries (inside an encrypted backup, the server's for a plaintext one): an entry outside
/// it is not written anywhere. [`restore_from_vss_expecting`] also checks whose wallet it is.
///
/// Returns the path to the restored wallet directory.
///
/// ERA fork: deprecated in the fork, kept for the bindings and upstream's own tests. A host that
/// knows which wallet it restores uses [`restore_from_vss_expecting`], which also checks that the
/// backup is that wallet's, and before it reads any of it.
#[deprecated(
    note = "ERA fork: use restore_from_vss_expecting, which checks whose wallet the backup is"
)]
pub async fn restore_from_vss(config: VssBackupConfig, target_dir: &str) -> Result<PathBuf, Error> {
    restore_from_vss_impl(config, target_dir, None).await
}

/// ERA fork: [`restore_from_vss`] for a host that knows which wallet it restores.
///
/// `expected_fingerprint` is that wallet's master fingerprint: 8 hex characters, anything else
/// being [`Error::InvalidFingerprint`] before anything is written or requested, compared ignoring
/// case (rgb-lib names a wallet directory after the master fingerprint as the host gave it, and the
/// restored directory keeps the name the backup carries). The server's word is checked against
/// it, not trusted, before any of the backup reaches the disk:
/// - the wallet the server names (`backup/fingerprint`) must be this one, else
///   [`Error::FingerprintMismatch`];
/// - with encryption enabled in `config` (the default), a backup the manifest marks as
///   unencrypted is [`Error::VssBackupUnencrypted`], as for [`restore_from_vss`];
/// - an encrypted backup names its wallet inside, where the server cannot change it: that must be
///   this one too, else [`Error::FingerprintMismatch`].
///
/// Each of those is read once, so a server cannot pass a check with one answer and have the
/// restore act on another. As for [`restore_from_vss`], only the wallet directory of the archive
/// is extracted.
pub async fn restore_from_vss_expecting(
    config: VssBackupConfig,
    target_dir: &str,
    expected_fingerprint: &str,
) -> Result<PathBuf, Error> {
    restore_from_vss_impl(config, target_dir, Some(expected_fingerprint)).await
}

/// ERA fork: whether `name` is a wallet fingerprint, as the server's `backup/fingerprint` and a
/// backup's own directory name it: 8 hex characters, so it can be a directory name and nothing
/// else. The case is the host's: rgb-lib names a wallet directory after the master fingerprint
/// exactly as the host gives it, so fingerprints are compared ignoring case, and a restore uses the
/// name the backup carries.
fn is_fingerprint(name: &str) -> bool {
    name.len() == 8 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn restore_from_vss_impl(
    config: VssBackupConfig,
    target_dir: &str,
    expected_fingerprint: Option<&str>,
) -> Result<PathBuf, Error> {
    // ERA fork: the expected fingerprint names a directory, so it is checked before anything
    if let Some(expected) = expected_fingerprint
        && !is_fingerprint(expected)
    {
        return Err(Error::InvalidFingerprint);
    }

    // ERA fork: a restore that fails leaves nothing behind that it made: not its log, and not the
    // target directory (nor a parent of it) if it made them, and nothing that was there before
    let target_dir_path = PathBuf::from(target_dir);
    let created = create_dirs(&target_dir_path)?;
    let result = match new_log_file(&target_dir_path).and_then(|log_name| {
        let logger = setup_logger(&target_dir_path, Some(&log_name));
        if logger.is_err() {
            let _ = fs::remove_file(target_dir_path.join(&log_name));
        }
        Ok((log_name, logger?))
    }) {
        Ok((log_name, (logger, logger_guard))) => {
            let result =
                restore_logged(config, &target_dir_path, expected_fingerprint, &logger).await;
            // (the log is complete and closed once both are gone)
            drop(logger);
            drop(logger_guard);
            if result.is_err() {
                let _ = fs::remove_file(target_dir_path.join(&log_name));
            }
            result
        }
        Err(e) => Err(e),
    };
    if result.is_err() {
        remove_dirs(&created);
    }
    result
}

/// ERA fork: create `dir` and its missing parents, as `fs::create_dir_all` does, and return the
/// directories this call created, in order. Only a directory `fs::create_dir` made counts: one
/// that was there, whatever path reaches it (`missing/../there`), is not this call's.
fn create_dirs(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut created = vec![];
    let mut path = PathBuf::new();
    for component in dir.components() {
        path.push(component);
        match fs::create_dir(&path) {
            Ok(()) => created.push(path.clone()),
            // there already (a root answers with another error than AlreadyExists on some systems)
            Err(_) if path.is_dir() => {}
            Err(e) => {
                remove_dirs(&created);
                return Err(e.into());
            }
        }
    }
    Ok(created)
}

/// ERA fork: remove `created` (from [`create_dirs`]), deepest first, each only if it is empty.
fn remove_dirs(created: &[PathBuf]) {
    for dir in created.iter().rev() {
        let _ = fs::remove_dir(dir);
    }
}

/// ERA fork: create the restore's log file in `dir`, a file of its own (`create_new`): named
/// `vss_restore_<unix time>` as upstream names it, with a suffix when a file of that name is there
/// already, which is then neither written to nor removed. Returns its name.
fn new_log_file(dir: &Path) -> Result<String, Error> {
    let name = format!("vss_restore_{}", OffsetDateTime::now_utc().unix_timestamp());
    for n in 0..100 {
        let candidate = match n {
            0 => name.clone(),
            n => format!("{name}_{n}"),
        };
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(&candidate))
        {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::IO {
        details: format!("no free name for the restore log next to {name}"),
    })
}

async fn restore_logged(
    config: VssBackupConfig,
    target_dir_path: &Path,
    expected_fingerprint: Option<&str>,
    logger: &Logger,
) -> Result<PathBuf, Error> {
    // ERA fork: with encryption on, only an encrypted backup is taken: decrypting is what
    // authenticates a backup, and a server could otherwise hand over any plaintext it likes
    let require_encrypted = config.encryption_enabled;

    // ERA fork: neither the server URL (a host's route to it may carry a session secret) nor the
    // store ID goes into the log file
    info!(logger, "Starting VSS restore...");

    // Create VSS client and download backup
    let client = VssBackupClient::new(config)?;

    // Check manifest to determine if backup is encrypted
    // ERA fork: read once; the download below decrypts by this same answer
    let manifest = client.get_manifest().await?;
    if require_encrypted && !manifest.encrypted {
        return Err(Error::VssBackupUnencrypted);
    }

    // Get fingerprint from server (stored during upload)
    // ERA fork: before the download, and checked before it is used as a directory name
    let fingerprint = client.get_fingerprint().await?;
    match expected_fingerprint {
        Some(expected) if !fingerprint.eq_ignore_ascii_case(expected) => {
            return Err(Error::FingerprintMismatch);
        }
        None if !is_fingerprint(&fingerprint) => {
            return Err(Error::VssError {
                details: "the backup on the server names no wallet fingerprint".to_string(),
            });
        }
        _ => {}
    }
    info!(logger, "Wallet fingerprint: {}", fingerprint);

    info!(logger, "Downloading backup from VSS server...");
    let backup_data = client.download_backup_with(&manifest).await?;
    info!(
        logger,
        "Downloaded {} bytes ({:.2} MB)",
        backup_data.len(),
        backup_data.len() as f64 / 1_000_000.0
    );
    // ERA fork: the wallet directory in the archive, and the name it is restored under. An
    // encrypted backup names its wallet inside, where the server cannot change it: that must be
    // the wallet the server named (ignoring case), and is the name. A plaintext one is sanitized,
    // its directory being "wallet/" whatever the wallet (upstream extracted it under that name and
    // renamed it afterwards), and the server's name for it is the name.
    let (dir_in_zip, dir_name) = if manifest.encrypted {
        let named = get_fingerprint_from_zip_bytes(&backup_data)?;
        if !named.eq_ignore_ascii_case(&fingerprint) {
            return Err(Error::FingerprintMismatch);
        }
        (named.clone(), named)
    } else {
        (SANITIZED_DIR_NAME.to_string(), fingerprint)
    };

    // Check if wallet already exists
    let wallet_dir = target_dir_path.join(&dir_name);
    if wallet_dir.exists() {
        return Err(Error::WalletDirAlreadyExists {
            path: wallet_dir.to_string_lossy().to_string(),
        });
    }

    // Extract backup
    // ERA fork: only the wallet directory, into a staging directory renamed into place once the
    // extraction is complete
    info!(logger, "Extracting backup to {:?}", wallet_dir);
    let staging = Staging::new(target_dir_path, &dir_name)?;
    if unzip_wallet_dir(&backup_data, &dir_in_zip, staging.path(), logger)? == 0 {
        return Err(Error::VssError {
            details: "the backup holds no file of the wallet".to_string(),
        });
    }

    if let Err(e) = fs::write(staging.path().join(VSS_RESTORE_MARKER), b"") {
        info!(logger, "Could not write restore marker: {e}");
    }
    staging.finish(&wallet_dir)?;

    info!(logger, "VSS restore completed successfully");
    Ok(wallet_dir)
}

/// Create backup data from a wallet directory
///
/// This is a helper function used by Wallet::vss_backup()
pub fn create_backup_data(wallet_dir: &Path, logger: &Logger) -> Result<Vec<u8>, Error> {
    info!(logger, "Creating VSS backup data from {:?}", wallet_dir);
    let data = zip_wallet_to_bytes(wallet_dir, logger)?;
    info!(
        logger,
        "Backup data created: {} bytes ({:.2} MB)",
        data.len(),
        data.len() as f64 / 1_000_000.0
    );
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::bitcoin::secp256k1::{Secp256k1, rand::rngs::OsRng};

    fn test_signing_key() -> SecretKey {
        let secp = Secp256k1::new();
        let (sk, _) = secp.generate_keypair(&mut OsRng);
        sk
    }

    #[test]
    fn test_backup_manifest_serialization() {
        let manifest = BackupManifest {
            chunk_count: 5,
            total_size: 20_000_000,
            encrypted: true,
            version: 1,
        };

        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: BackupManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.chunk_count, 5);
        assert_eq!(parsed.total_size, 20_000_000);
        assert!(parsed.encrypted);
        assert_eq!(parsed.version, 1);
    }

    #[test]
    fn test_encryption_metadata_serialization() {
        let metadata = VssEncryptionMetadata::new();

        let json = serde_json::to_string(&metadata).unwrap();
        let parsed: VssEncryptionMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, VSS_BACKUP_VERSION);
        assert_eq!(parsed.nonce.len(), BACKUP_NONCE_LENGTH * 2);
        assert!(!parsed.salt.is_empty());
        // Salt should be valid hex (64 chars for 32 bytes)
        assert_eq!(parsed.salt.len(), BACKUP_SALT_LENGTH * 2);
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let original_data = b"Hello, this is test data for encryption!".to_vec();
        let key = test_signing_key();

        let metadata = VssEncryptionMetadata::new();

        // Encrypt
        let encrypted = encrypt_data(&original_data, &key, &metadata, None).unwrap();
        assert_ne!(encrypted, original_data);

        // Decrypt
        let decrypted = decrypt_data(&encrypted, &key, &metadata, None).unwrap();
        assert_eq!(decrypted, original_data);
    }

    #[test]
    fn test_encrypt_decrypt_large_data() {
        // Test with data larger than buffer size
        let original_data: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
        let key = test_signing_key();

        let metadata = VssEncryptionMetadata::new();

        let encrypted = encrypt_data(&original_data, &key, &metadata, None).unwrap();
        let decrypted = decrypt_data(&encrypted, &key, &metadata, None).unwrap();

        assert_eq!(decrypted, original_data);
    }

    #[test]
    fn test_wrong_key_fails() {
        let original_data = b"Secret data".to_vec();
        let correct_key = test_signing_key();
        let wrong_key = test_signing_key();

        let metadata = VssEncryptionMetadata::new();

        let encrypted = encrypt_data(&original_data, &correct_key, &metadata, None).unwrap();
        let result = decrypt_data(&encrypted, &wrong_key, &metadata, None);

        assert!(result.is_err());
    }

    #[test]
    fn test_encrypt_decrypt_empty_data() {
        let original_data: Vec<u8> = vec![];
        let key = test_signing_key();

        let metadata = VssEncryptionMetadata::new();

        let encrypted = encrypt_data(&original_data, &key, &metadata, None).unwrap();
        let decrypted = decrypt_data(&encrypted, &key, &metadata, None).unwrap();

        assert_eq!(decrypted, original_data);
    }

    #[test]
    fn test_encrypt_decrypt_exact_buffer_size() {
        // Test with data exactly at buffer boundary
        let original_data: Vec<u8> = (0..BACKUP_BUFFER_LEN_ENCRYPT)
            .map(|i| (i % 256) as u8)
            .collect();
        let key = test_signing_key();

        let metadata = VssEncryptionMetadata::new();

        let encrypted = encrypt_data(&original_data, &key, &metadata, None).unwrap();
        let decrypted = decrypt_data(&encrypted, &key, &metadata, None).unwrap();

        assert_eq!(decrypted, original_data);
    }

    #[test]
    fn test_encrypt_decrypt_multiple_buffer_sizes() {
        // Test with data that spans multiple buffer reads
        let original_data: Vec<u8> = (0..(BACKUP_BUFFER_LEN_ENCRYPT * 3 + 50))
            .map(|i| (i % 256) as u8)
            .collect();
        let key = test_signing_key();

        let metadata = VssEncryptionMetadata::new();

        let encrypted = encrypt_data(&original_data, &key, &metadata, None).unwrap();
        let decrypted = decrypt_data(&encrypted, &key, &metadata, None).unwrap();

        assert_eq!(decrypted, original_data);
    }

    #[test]
    fn test_decrypt_corrupted_data_fails() {
        let original_data = b"Test data for corruption test".to_vec();
        let key = test_signing_key();

        let metadata = VssEncryptionMetadata::new();

        let mut encrypted = encrypt_data(&original_data, &key, &metadata, None).unwrap();

        // Corrupt the encrypted data
        if !encrypted.is_empty() {
            let mid = encrypted.len() / 2;
            encrypted[mid] ^= 0xFF;
        }

        let result = decrypt_data(&encrypted, &key, &metadata, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_different_salts_produce_different_keys() {
        let key = test_signing_key();
        let metadata1 = VssEncryptionMetadata::new();
        let metadata2 = VssEncryptionMetadata::new();

        let derived1 = derive_encryption_key(&key, &metadata1, None).unwrap();
        let derived2 = derive_encryption_key(&key, &metadata2, None).unwrap();

        // Different random salts should produce different derived keys
        assert_ne!(derived1, derived2);
    }

    #[test]
    fn test_backup_manifest_chunk_calculation() {
        // Test that chunk count calculation is correct
        let small_size = VSS_CHUNK_SIZE - 1;
        let exact_size = VSS_CHUNK_SIZE;
        let large_size = VSS_CHUNK_SIZE * 2 + 100;

        assert_eq!(small_size.div_ceil(VSS_CHUNK_SIZE), 1);
        assert_eq!(exact_size.div_ceil(VSS_CHUNK_SIZE), 1);
        assert_eq!(large_size.div_ceil(VSS_CHUNK_SIZE), 3);
    }

    #[test]
    fn test_is_bdk_db_file() {
        assert!(is_bdk_db_file("bdk_db"));
        assert!(is_bdk_db_file("bdk_db_watch_only"));
        assert!(is_bdk_db_file("abc123/bdk_db"));
        assert!(is_bdk_db_file("abc123/bdk_db_watch_only"));
        assert!(is_bdk_db_file("some/deep/path/bdk_db"));

        // recovery sidecars carry full descriptors and must be excluded (HIGH-3)
        assert!(is_bdk_db_file("abc123/bdk_db.corrupt"));
        assert!(is_bdk_db_file("abc123/bdk_db.corrupt.1"));
        assert!(is_bdk_db_file("abc123/bdk_db.recovering"));
        assert!(is_bdk_db_file("abc123/bdk_db_watch_only.corrupt"));

        assert!(!is_bdk_db_file("bdk_db_other"));
        assert!(!is_bdk_db_file("not_bdk_db"));
        assert!(!is_bdk_db_file("abc123/some_file.txt"));
        assert!(!is_bdk_db_file(""));
    }

    /// Helper: create a test zip with a fingerprint directory structure
    fn create_test_zip(fingerprint: &str) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

            // Add fingerprint directory
            zip.add_directory(format!("{fingerprint}/"), options)
                .unwrap();

            // Add a normal file
            zip.start_file(format!("{fingerprint}/some_file.txt"), options)
                .unwrap();
            zip.write_all(b"test content").unwrap();

            // Add bdk_db file
            zip.start_file(format!("{fingerprint}/bdk_db"), options)
                .unwrap();
            zip.write_all(b"xpub_sensitive_data").unwrap();

            // Add bdk_db_watch_only file
            zip.start_file(format!("{fingerprint}/bdk_db_watch_only"), options)
                .unwrap();
            zip.write_all(b"xpub_watch_only_data").unwrap();

            // Add a nested file
            zip.add_directory(format!("{fingerprint}/subdir/"), options)
                .unwrap();
            zip.start_file(format!("{fingerprint}/subdir/nested.dat"), options)
                .unwrap();
            zip.write_all(b"nested data").unwrap();

            // Add a wallet manifest with xpubs and the fingerprint
            zip.start_file(format!("{fingerprint}/{WALLET_MANIFEST_FILE}"), options)
                .unwrap();
            zip.write_all(
                format!(
                    r#"{{"account_xpub_vanilla":"tpubFakeVanilla","account_xpub_colored":"tpubFakeColored","master_fingerprint":"{fingerprint}"}}"#
                )
                .as_bytes(),
            )
            .unwrap();

            // Add an orphaned manifest temp file left by a crash mid-write
            zip.start_file(format!("{fingerprint}/{WALLET_MANIFEST_FILE}.tmp"), options)
                .unwrap();
            zip.write_all(br#"{"account_xpub_vanilla":"tpubFakeVanilla"}"#)
                .unwrap();

            zip.finish().unwrap();
        }
        buffer.into_inner()
    }

    #[test]
    fn test_sanitize_zip_for_plaintext() {
        let fingerprint = "a1b2c3d4";
        let zip_data = create_test_zip(fingerprint);

        let (sanitized, extracted_fp) = sanitize_zip_for_plaintext(&zip_data).unwrap();
        assert_eq!(extracted_fp, fingerprint);

        // Inspect sanitized zip
        let reader = std::io::Cursor::new(&sanitized);
        let mut archive = zip::ZipArchive::new(reader).unwrap();

        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();

        // Fingerprint should be replaced with "wallet"
        assert!(names.iter().any(|n| n.starts_with("wallet/")));
        assert!(!names.iter().any(|n| n.contains(fingerprint)));

        // BDK files should be excluded
        assert!(!names.iter().any(|n| n.ends_with("bdk_db")));
        assert!(!names.iter().any(|n| n.ends_with("bdk_db_watch_only")));

        // Normal files should be present
        assert!(names.contains(&"wallet/some_file.txt".to_string()));
        assert!(names.contains(&"wallet/subdir/nested.dat".to_string()));

        // Verify file content is preserved
        let mut some_file = archive.by_name("wallet/some_file.txt").unwrap();
        let mut content = String::new();
        some_file.read_to_string(&mut content).unwrap();
        assert_eq!(content, "test content");
    }

    #[test]
    fn test_sanitize_zip_excludes_wallet_manifest() {
        let fingerprint = "a1b2c3d4";
        let zip_data = create_test_zip(fingerprint);

        let (sanitized, _) = sanitize_zip_for_plaintext(&zip_data).unwrap();

        let reader = std::io::Cursor::new(&sanitized);
        let mut archive = zip::ZipArchive::new(reader).unwrap();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).unwrap();
            let filename = file.name().rsplit('/').next().unwrap().to_string();
            assert_ne!(filename, WALLET_MANIFEST_FILE);
            assert_ne!(filename, format!("{WALLET_MANIFEST_FILE}.tmp"));
            let mut content = Vec::new();
            file.read_to_end(&mut content).unwrap();
            let content = String::from_utf8_lossy(&content);
            assert!(!content.contains("tpubFakeVanilla"));
            assert!(!content.contains("tpubFakeColored"));
            assert!(!content.contains(fingerprint));
        }
    }

    #[test]
    fn test_get_fingerprint_from_zip_bytes() {
        let fingerprint = "deadbeef";
        let zip_data = create_test_zip(fingerprint);

        let extracted = get_fingerprint_from_zip_bytes(&zip_data).unwrap();
        assert_eq!(extracted, fingerprint);
    }

    /// Zip carrying a Windows backslash-separated bdk_db plus BDK recovery sidecars, all with
    /// full descriptors. Guards HIGH-4 (separator leak) end-to-end.
    fn create_test_zip_with_leaky_sidecars(fingerprint: &str) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

            zip.add_directory(format!("{fingerprint}/"), options)
                .unwrap();
            zip.start_file(format!("{fingerprint}/some_file.txt"), options)
                .unwrap();
            zip.write_all(b"harmless").unwrap();

            // Windows-style backslash separator on the primary bdk_db
            zip.start_file(format!("{fingerprint}\\bdk_db"), options)
                .unwrap();
            zip.write_all(b"tpubLeakBackslash").unwrap();

            // recovery sidecars reached through a backslash-separated directory
            zip.start_file(format!("{fingerprint}\\bdk_db.corrupt"), options)
                .unwrap();
            zip.write_all(format!("tr([{fingerprint}/86'/1'/0']tpubLeakCorrupt/0/*)").as_bytes())
                .unwrap();
            zip.start_file(format!("{fingerprint}\\bdk_db.recovering"), options)
                .unwrap();
            zip.write_all(b"tpubLeakRecovering").unwrap();

            zip.finish().unwrap();
        }
        buffer.into_inner()
    }

    #[test]
    fn test_sanitize_zip_excludes_bdk_db_recovery_sidecars() {
        let fingerprint = "a1b2c3d4";
        let zip_data = create_test_zip_with_leaky_sidecars(fingerprint);

        let (sanitized, _) = sanitize_zip_for_plaintext(&zip_data).unwrap();

        let reader = std::io::Cursor::new(&sanitized);
        let mut archive = zip::ZipArchive::new(reader).unwrap();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).unwrap();
            let mut content = Vec::new();
            file.read_to_end(&mut content).unwrap();
            let content = String::from_utf8_lossy(&content);
            assert!(!content.contains("tpubLeakBackslash"));
            assert!(!content.contains("tpubLeakCorrupt"));
            assert!(!content.contains("tpubLeakRecovering"));
            assert!(!content.contains(fingerprint));
        }
    }
    // ERA fork: decrypt_data refuses data that ends without its final block, and the backups
    // written before that (fixtures from d82e21a, see wallet::test::era_fixtures) still decrypt

    #[test]
    fn decrypt_data_refuses_data_cut_short() {
        let key = test_signing_key();
        let metadata = VssEncryptionMetadata::new();
        // no data, a byte, one and two whole blocks (their final block a tag alone), more
        for len in [0usize, 1, 239, 478, 1000] {
            let plain: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            let sealed = encrypt_data(&plain, &key, &metadata, None).unwrap();
            assert_eq!(decrypt_data(&sealed, &key, &metadata, None).unwrap(), plain);
            for at in 0..sealed.len() {
                let result = decrypt_data(&sealed[..at], &key, &metadata, None);
                let details = match result {
                    Err(Error::VssError { details }) => details,
                    other => panic!("{len} cut at {at}: {other:?}"),
                };
                // after whole blocks, every one authenticates and the final one is missing; a
                // partial block does not authenticate
                assert_eq!(
                    details.contains("truncated"),
                    at % BACKUP_BUFFER_LEN_DECRYPT == 0,
                    "{len} cut at {at}: {details}"
                );
            }
        }
    }

    #[test]
    fn d82e21a_stream_vectors_still_decrypt() {
        use crate::wallet::test::era_fixtures::{
            STREAM_VECTOR_LENGTHS, fixture_dir, fixture_signing_key, fixture_stream_metadata,
            stream_plaintext,
        };
        let key = fixture_signing_key();
        let metadata = fixture_stream_metadata();
        let vectors =
            fs::read_to_string(fixture_dir("d82e21a").join("vss_stream_vectors.txt")).unwrap();
        let mut lengths = vec![];
        for line in vectors.lines() {
            let (len, sealed) = line.split_once(' ').unwrap();
            let len: usize = len.parse().unwrap();
            let sealed = hex::decode(sealed).unwrap();
            let plain = stream_plaintext(len);
            assert_eq!(
                decrypt_data(&sealed, &key, &metadata, None).unwrap(),
                plain,
                "{len}"
            );
            // and today's encrypt_data writes the same bytes
            assert_eq!(
                encrypt_data(&plain, &key, &metadata, None).unwrap(),
                sealed,
                "{len}"
            );
            lengths.push(len);
        }
        assert_eq!(lengths, STREAM_VECTOR_LENGTHS);
    }

    #[test]
    fn d82e21a_vss_backup_still_restores() {
        use crate::wallet::test::era_fixtures::{dir_listing, fixture_dir, fixture_signing_key};
        // what a VSS server holds for a backup uploaded at d82e21a, the rev the ERA app pinned
        // before the check above
        let fixtures = fixture_dir("d82e21a");
        let read = |name: &str| fs::read(fixtures.join(name)).unwrap();
        let fingerprint = String::from_utf8(read("vss_backup_fingerprint.txt")).unwrap();
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![read("vss_backup_manifest.json")]),
            (
                BACKUP_KEY_FINGERPRINT,
                vec![fingerprint.clone().into_bytes()],
            ),
            (BACKUP_KEY_DATA, vec![read("vss_backup_data.bin")]),
            (BACKUP_KEY_METADATA, vec![read("vss_backup_metadata.json")]),
        ]);
        let listing = fs::read_to_string(fixtures.join("vss_backup_listing.txt")).unwrap();
        for expected in [Some(fingerprint.as_str()), None] {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            let restored =
                restore(config(&server, fixture_signing_key()), &target, expected).unwrap();
            assert_eq!(restored, target.join(&fingerprint));
            assert_eq!(dir_listing(&target, &fingerprint), listing, "{expected:?}");
        }
    }

    /// ERA fork: a VSS server whose getObject answers follow a script: the n-th read of a key
    /// gets the n-th value listed for it (the last one again after that), so a server that
    /// answers two reads of one key differently can be played. A key with no values is not
    /// there (HTTP 500). Every read is counted.
    /// The values a [`VssScript`] answers, per key.
    type Answers<'a> = Vec<(&'a str, Vec<Vec<u8>>)>;

    struct VssScript {
        server: mockito::ServerGuard,
        reads: Arc<std::sync::Mutex<HashMap<String, usize>>>,
        _mock: mockito::Mock,
    }

    impl VssScript {
        fn start(answers: Vec<(&str, Vec<Vec<u8>>)>) -> Self {
            use vss_client::prost::Message;
            use vss_client::types::GetObjectResponse;
            let key_of = |request: &mockito::Request| {
                GetObjectRequest::decode(&request.body().unwrap()[..])
                    .map(|r| r.key)
                    .unwrap_or_default()
            };
            let answers: Arc<HashMap<String, Vec<Vec<u8>>>> = Arc::new(
                answers
                    .into_iter()
                    .map(|(key, values)| (key.to_string(), values))
                    .collect(),
            );
            let reads = Arc::new(std::sync::Mutex::new(HashMap::new()));
            let (known, counter) = (answers.clone(), reads.clone());
            let mut server = mockito::Server::new();
            let mock = server
                .mock("POST", mockito::Matcher::Regex("/getObject$".to_string()))
                .with_status_code_from_request(move |request| {
                    if known.get(&key_of(request)).is_some_and(|v| !v.is_empty()) {
                        200
                    } else {
                        500
                    }
                })
                .with_body_from_request(move |request| {
                    let key = key_of(request);
                    let mut reads = counter.lock().unwrap();
                    let n = reads.entry(key.clone()).or_insert(0);
                    *n += 1;
                    match answers.get(&key) {
                        Some(values) if !values.is_empty() => GetObjectResponse {
                            value: Some(KeyValue {
                                key,
                                version: 1,
                                value: values[(*n - 1).min(values.len() - 1)].clone(),
                            }),
                        }
                        .encode_to_vec(),
                        _ => vec![],
                    }
                })
                .create();
            Self {
                server,
                reads,
                _mock: mock,
            }
        }

        fn url(&self) -> String {
            self.server.url()
        }

        fn reads(&self, key: &str) -> usize {
            self.reads.lock().unwrap().get(key).copied().unwrap_or(0)
        }
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(future)
    }

    /// The text of every restore log in `dir`.
    fn restore_logs(dir: &Path) -> String {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("vss_restore_")
            })
            .map(|path| fs::read_to_string(path).unwrap())
            .collect()
    }

    #[test]
    fn the_restore_log_names_neither_the_server_nor_the_store() {
        // a restore that completes keeps its log in the target directory, as upstream
        let key = test_signing_key();
        let [manifest, data, metadata] = uploaded(EXPECTED, &key, true);
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest]),
            (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
            (BACKUP_KEY_DATA, vec![data]),
            (BACKUP_KEY_METADATA, vec![metadata]),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let store_id = "store-6c1f0e";
        let server_url = format!("{}/session-secret/vss", server.url());
        let config = VssBackupConfig::new(server_url.clone(), store_id.to_string(), key);
        restore(config, dir.path(), None).unwrap();
        let logs = restore_logs(dir.path());
        assert!(logs.contains("VSS restore completed"), "{logs}");
        for secret in [server_url.as_str(), "session-secret", store_id] {
            assert!(!logs.contains(secret), "{secret} in {logs}");
        }
    }

    // ERA fork: a restore that fails leaves nothing behind: not its log, and not the target
    // directory if it made it

    /// What a refused restore leaves under a root that held nothing.
    const NOTHING: [&str; 0] = [];

    #[test]
    fn a_failed_restore_leaves_nothing_behind() {
        let key = test_signing_key();
        let [manifest, data, metadata] = uploaded(EXPECTED, &key, true);
        // the server names another wallet
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest]),
            (BACKUP_KEY_FINGERPRINT, vec![b"deadbeef".to_vec()]),
            (BACKUP_KEY_DATA, vec![data]),
            (BACKUP_KEY_METADATA, vec![metadata]),
        ]);
        for expected in [Some(EXPECTED), None] {
            // a target the restore makes, parents included: all of it goes
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("a").join("b").join("data");
            let result = restore(config(&server, key), &target, expected);
            assert!(result.is_err(), "{expected:?}: {result:?}");
            assert!(
                fs::read_dir(root.path()).unwrap().next().is_none(),
                "{expected:?}"
            );

            // a target that was there, holding another wallet: that is all it holds after
            let root = tempfile::tempdir().unwrap();
            let target = target_with_another_wallet(root.path());
            let result = restore(config(&server, key), &target, expected);
            assert!(result.is_err(), "{expected:?}: {result:?}");
            assert_eq!(restore_logs(&target), "");
            let left: Vec<String> = fs::read_dir(&target)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
                .collect();
            assert_eq!(left, ["deadbeef"], "{expected:?}");
        }
    }
    // ERA fork: restore_from_vss(_expecting) against a server whose answers change between reads
    // (restore TOCTOU): the host reads the fingerprint and the manifest first and is told the truth,
    // rgb-lib's own reads are told something else.

    const EXPECTED: &str = "a1b2c3d4";

    fn manifest_json(encrypted: bool, data: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&BackupManifest {
            chunk_count: 1,
            total_size: data.len(),
            encrypted,
            version: VSS_BACKUP_VERSION,
        })
        .unwrap()
    }

    /// What the server holds for a backup of `fingerprint`'s wallet uploaded as rgb-lib uploads
    /// it: (manifest, data, encryption metadata), encrypted with `key` or sanitized plaintext.
    fn uploaded(fingerprint: &str, key: &SecretKey, encrypted: bool) -> [Vec<u8>; 3] {
        let zip = create_test_zip(fingerprint);
        if encrypted {
            let metadata = VssEncryptionMetadata::new();
            let data = encrypt_data(&zip, key, &metadata, None).unwrap();
            [
                manifest_json(true, &data),
                data,
                serde_json::to_vec(&metadata).unwrap(),
            ]
        } else {
            let (data, _) = sanitize_zip_for_plaintext(&zip).unwrap();
            [manifest_json(false, &data), data, vec![]]
        }
    }

    fn config(server: &VssScript, key: SecretKey) -> VssBackupConfig {
        VssBackupConfig::new(server.url(), "store".to_string(), key)
    }

    /// Every entry under `root`, relative, restore logs left out.
    fn tree(root: &Path) -> Vec<String> {
        let mut entries: Vec<String> = WalkDir::new(root)
            .min_depth(1)
            .into_iter()
            .map(Result::unwrap)
            // (a staging directory left behind, ".vss_restore_…", is not a log)
            .filter(|entry| {
                !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("vss_restore_")
            })
            .map(|entry| {
                entry
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        entries.sort();
        entries
    }

    // (restore_from_vss is deprecated in the fork, and tested all the same)
    #[allow(deprecated)]
    fn restore(
        config: VssBackupConfig,
        target: &Path,
        expected: Option<&str>,
    ) -> Result<PathBuf, Error> {
        let target = target.to_str().unwrap();
        block_on(async {
            match expected {
                Some(expected) => restore_from_vss_expecting(config, target, expected).await,
                None => restore_from_vss(config, target).await,
            }
        })
    }

    #[test]
    fn restore_expecting_refuses_an_expected_fingerprint_that_is_not_one() {
        let server = VssScript::start(vec![]);
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("data");
        for expected in [
            "",
            "a1b2c3d",
            "a1b2c3d4e",
            "../a1b2c",
            "a1b2/c3d",
            "a1b2c3dz",
            " a1b2c3d",
        ] {
            let result = restore(config(&server, test_signing_key()), &target, Some(expected));
            assert!(
                matches!(result, Err(Error::InvalidFingerprint)),
                "{expected:?}: {result:?}"
            );
        }
        // nothing asked of the server, nothing written
        assert_eq!(server.reads(BACKUP_KEY_MANIFEST), 0);
        assert!(!target.exists());
    }

    #[test]
    fn restore_expecting_restores_the_expected_wallet() {
        let key = test_signing_key();
        let [manifest, data, metadata] = uploaded(EXPECTED, &key, true);
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest]),
            (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
            (BACKUP_KEY_DATA, vec![data]),
            (BACKUP_KEY_METADATA, vec![metadata]),
        ]);
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("data");
        let restored = restore(config(&server, key), &target, Some(EXPECTED)).unwrap();
        assert_eq!(restored, target.join(EXPECTED));
        assert!(restored.join("some_file.txt").is_file());
        // one read each: nothing is decided by one answer and done by another
        assert_eq!(server.reads(BACKUP_KEY_MANIFEST), 1);
        assert_eq!(server.reads(BACKUP_KEY_FINGERPRINT), 1);
    }

    #[test]
    fn restore_expecting_refuses_another_wallet_named_on_the_second_read() {
        let key = test_signing_key();
        let [manifest, data, metadata] = uploaded(EXPECTED, &key, true);
        for second in ["../escape", "deadbeef"] {
            let server = VssScript::start(vec![
                (BACKUP_KEY_MANIFEST, vec![manifest.clone()]),
                (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into(), second.into()]),
                (BACKUP_KEY_DATA, vec![data.clone()]),
                (BACKUP_KEY_METADATA, vec![metadata.clone()]),
            ]);
            // the host's own read is told the expected wallet
            let host = VssBackupClient::new(config(&server, key)).unwrap();
            assert_eq!(block_on(host.get_fingerprint()).unwrap(), EXPECTED);
            drop(host);

            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            let result = restore(config(&server, key), &target, Some(EXPECTED));
            assert!(
                matches!(result, Err(Error::FingerprintMismatch)),
                "{second}: {result:?}"
            );
            // nothing of the backup was fetched or written, in the target or next to it
            assert_eq!(server.reads(BACKUP_KEY_DATA), 0);
            assert_eq!(tree(root.path()), NOTHING);
        }
    }

    #[test]
    fn restore_expecting_refuses_a_backup_marked_plaintext_on_the_second_read() {
        let key = test_signing_key();
        let [encrypted_manifest, _, _] = uploaded(EXPECTED, &key, true);
        // a plaintext "backup" of the server's making, with the wallet's name on it
        let [plaintext_manifest, plaintext, _] = uploaded(EXPECTED, &key, false);
        let server = VssScript::start(vec![
            (
                BACKUP_KEY_MANIFEST,
                vec![encrypted_manifest, plaintext_manifest],
            ),
            (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
            (BACKUP_KEY_DATA, vec![plaintext]),
        ]);
        // the host's own read is told the backup is encrypted
        let host = VssBackupClient::new(config(&server, key)).unwrap();
        assert!(block_on(host.get_manifest()).unwrap().encrypted);
        drop(host);

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("data");
        let result = restore(config(&server, key), &target, Some(EXPECTED));
        assert!(
            matches!(result, Err(Error::VssBackupUnencrypted)),
            "{result:?}"
        );
        assert_eq!(server.reads(BACKUP_KEY_DATA), 0);
        assert_eq!(tree(root.path()), NOTHING);

        // a host that has encryption off restores it, as upstream would
        let target = root.path().join("plain");
        let restored = restore(
            config(&server, key).with_encryption(false),
            &target,
            Some(EXPECTED),
        )
        .unwrap();
        assert_eq!(restored, target.join(EXPECTED));
    }

    #[test]
    fn restore_refuses_a_plaintext_backup_when_encryption_is_on() {
        // upstream's restore_from_vss took a backup the manifest marks as plaintext as it came,
        // whatever the config said: a wallet of the server's making, its manifest file included
        let key = test_signing_key();
        let planted = zip_of(&[
            ("wallet/", None),
            ("wallet/wallet_manifest.json", Some(b"{\"planted\":true}")),
            ("wallet/rgb_lib_db", Some(b"the server's database")),
        ]);
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest_json(false, &planted)]),
            (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
            (BACKUP_KEY_DATA, vec![planted]),
        ]);
        for expected in [None, Some(EXPECTED)] {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            let result = restore(config(&server, key), &target, expected);
            assert!(
                matches!(result, Err(Error::VssBackupUnencrypted)),
                "{expected:?}: {result:?}"
            );
            assert!(!target.exists());
        }
        assert_eq!(server.reads(BACKUP_KEY_DATA), 0);
        // a config with encryption off takes it, as upstream did
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("data");
        restore(config(&server, key).with_encryption(false), &target, None).unwrap();
        assert!(target.join(EXPECTED).join("rgb_lib_db").is_file());
    }

    #[test]
    fn every_backup_the_server_makes_is_refused_with_encryption_on() {
        // the second review's probe (B1): what a server can serve for the expected wallet, short
        // of one of its genuine backups
        let key = test_signing_key();
        let fake = zip_of(&[
            ("wallet/", None),
            ("wallet/wallet_manifest.json", Some(b"{\"fake\":true}")),
            ("wallet/rgb_lib_db", Some(b"server db")),
        ]);
        let fake_named = zip_of(&[
            ("a1b2c3d4/", None),
            ("a1b2c3d4/wallet_manifest.json", Some(b"{\"fake\":true}")),
            ("a1b2c3d4/rgb_lib_db", Some(b"server db")),
        ]);
        let [_, _, good_metadata] = uploaded(EXPECTED, &key, true);
        let [_, other_data, other_metadata] = uploaded(EXPECTED, &test_signing_key(), true);
        let [_, a_data, a_metadata] = uploaded(EXPECTED, &key, true);
        let [_, b_data, _] = uploaded(EXPECTED, &key, true);
        let half = a_data.len() / 2;
        let chunked = |encrypted: bool, count: usize, total: usize| {
            vec![chunked_manifest(encrypted, count, total)]
        };
        let cases: Vec<(&str, Answers)> = vec![
            (
                "marked plaintext",
                vec![
                    (BACKUP_KEY_MANIFEST, vec![manifest_json(false, &fake)]),
                    (BACKUP_KEY_DATA, vec![fake.clone()]),
                ],
            ),
            (
                "marked encrypted, plaintext wallet/",
                vec![
                    (BACKUP_KEY_MANIFEST, vec![manifest_json(true, &fake)]),
                    (BACKUP_KEY_DATA, vec![fake.clone()]),
                    (BACKUP_KEY_METADATA, vec![good_metadata.clone()]),
                ],
            ),
            (
                "marked encrypted, plaintext a1b2c3d4/",
                vec![
                    (BACKUP_KEY_MANIFEST, vec![manifest_json(true, &fake_named)]),
                    (BACKUP_KEY_DATA, vec![fake_named.clone()]),
                    (BACKUP_KEY_METADATA, vec![good_metadata.clone()]),
                ],
            ),
            (
                "no manifest",
                vec![(BACKUP_KEY_DATA, vec![fake_named.clone()])],
            ),
            (
                "a manifest that is not JSON",
                vec![
                    (BACKUP_KEY_MANIFEST, vec![b"plaintext".to_vec()]),
                    (BACKUP_KEY_DATA, vec![fake_named.clone()]),
                ],
            ),
            (
                "encrypted under another key",
                vec![
                    (BACKUP_KEY_MANIFEST, vec![manifest_json(true, &other_data)]),
                    (BACKUP_KEY_DATA, vec![other_data.clone()]),
                    (BACKUP_KEY_METADATA, vec![other_metadata.clone()]),
                ],
            ),
            (
                "no metadata",
                vec![
                    (BACKUP_KEY_MANIFEST, vec![manifest_json(true, &a_data)]),
                    (BACKUP_KEY_DATA, vec![a_data.clone()]),
                ],
            ),
            (
                "chunks of two backups",
                vec![
                    (BACKUP_KEY_MANIFEST, chunked(true, 2, a_data.len())),
                    ("backup/chunk/0", vec![a_data[..half].to_vec()]),
                    ("backup/chunk/1", vec![b_data[half..].to_vec()]),
                    (BACKUP_KEY_METADATA, vec![a_metadata.clone()]),
                ],
            ),
            (
                "ciphertext, then plaintext",
                vec![
                    (
                        BACKUP_KEY_MANIFEST,
                        chunked(true, 2, half + fake_named.len()),
                    ),
                    ("backup/chunk/0", vec![a_data[..half].to_vec()]),
                    ("backup/chunk/1", vec![fake_named.clone()]),
                    (BACKUP_KEY_METADATA, vec![a_metadata.clone()]),
                ],
            ),
            (
                "plaintext, then ciphertext",
                vec![
                    (
                        BACKUP_KEY_MANIFEST,
                        chunked(true, 2, fake_named.len() + a_data.len() - half),
                    ),
                    ("backup/chunk/0", vec![fake_named.clone()]),
                    ("backup/chunk/1", vec![a_data[half..].to_vec()]),
                    (BACKUP_KEY_METADATA, vec![a_metadata.clone()]),
                ],
            ),
            (
                "no chunk, marked encrypted",
                vec![
                    (BACKUP_KEY_MANIFEST, chunked(true, 0, 0)),
                    (BACKUP_KEY_METADATA, vec![a_metadata.clone()]),
                ],
            ),
            (
                "plaintext chunks, marked plaintext",
                vec![
                    (BACKUP_KEY_MANIFEST, chunked(false, 2, fake.len())),
                    ("backup/chunk/0", vec![fake[..10].to_vec()]),
                    ("backup/chunk/1", vec![fake[10..].to_vec()]),
                ],
            ),
        ];
        for (name, mut answers) in cases {
            answers.push((BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]));
            let server = VssScript::start(answers);
            for expected in [Some(EXPECTED), None] {
                let root = tempfile::tempdir().unwrap();
                let target = root.path().join("data");
                let result = restore(config(&server, key), &target, expected);
                assert!(result.is_err(), "{name}, {expected:?}: {result:?}");
                assert!(!target.exists(), "{name}, {expected:?}");
            }
        }
    }

    #[test]
    fn restore_expecting_refuses_an_encrypted_backup_of_another_wallet() {
        let key = test_signing_key();
        let [manifest, data, metadata] = uploaded("deadbeef", &key, true);
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest]),
            // the server names the expected wallet, the backup is another one
            (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
            (BACKUP_KEY_DATA, vec![data]),
            (BACKUP_KEY_METADATA, vec![metadata]),
        ]);
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("data");
        let result = restore(config(&server, key), &target, Some(EXPECTED));
        assert!(
            matches!(result, Err(Error::FingerprintMismatch)),
            "{result:?}"
        );
        assert_eq!(tree(root.path()), NOTHING);
    }

    #[test]
    fn restore_refuses_a_server_fingerprint_that_is_not_one() {
        // upstream's restore_from_vss: the name the server gives the wallet became a directory
        // name as it came, so "../escape" put the restored wallet next to the target
        let key = test_signing_key();
        let [manifest, data, _] = uploaded(EXPECTED, &key, false);
        for named in ["../escape", "a1b2c3d4/", "", "/tmp/era-escape", "a1b2c3dz"] {
            let server = VssScript::start(vec![
                (BACKUP_KEY_MANIFEST, vec![manifest.clone()]),
                (BACKUP_KEY_FINGERPRINT, vec![named.into()]),
                (BACKUP_KEY_DATA, vec![data.clone()]),
            ]);
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            let result = restore(config(&server, key).with_encryption(false), &target, None);
            assert!(
                matches!(result, Err(Error::VssError { .. })),
                "{named:?}: {result:?}"
            );
            assert_eq!(server.reads(BACKUP_KEY_DATA), 0, "{named:?}");
            assert_eq!(tree(root.path()), NOTHING, "{named:?}");
        }
    }

    // ERA fork: only the archive's wallet directory is extracted, and only whole

    /// A zip of `entries`: a name and its content, or a directory (None).
    fn zip_of(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            for (name, content) in entries {
                match content {
                    None => zip.add_directory(*name, options).unwrap(),
                    Some(content) => {
                        zip.start_file(*name, options).unwrap();
                        zip.write_all(content).unwrap();
                    }
                }
            }
            zip.finish().unwrap();
        }
        buffer.into_inner()
    }

    /// What the server holds for `zip` uploaded with encryption on, under `key`.
    fn sealed(zip: &[u8], key: &SecretKey) -> [Vec<u8>; 3] {
        let metadata = VssEncryptionMetadata::new();
        let data = encrypt_data(zip, key, &metadata, None).unwrap();
        [
            manifest_json(true, &data),
            data,
            serde_json::to_vec(&metadata).unwrap(),
        ]
    }

    /// A target directory already holding another wallet, `deadbeef`.
    fn target_with_another_wallet(root: &Path) -> PathBuf {
        let target = root.join("data");
        fs::create_dir_all(target.join("deadbeef")).unwrap();
        fs::write(target.join("deadbeef/rgb_lib_db"), b"another wallet").unwrap();
        target
    }

    #[test]
    fn a_restore_extracts_the_wallet_directory_and_nothing_else() {
        // entries outside a1b2c3d4/ in an archive encrypted with the user's key: upstream wrote
        // every one of them into the target directory (or next to it), over another wallet too
        let key = test_signing_key();
        let odd = zip_of(&[
            ("a1b2c3d4/", None),
            ("a1b2c3d4/rgb_lib_db", Some(b"db")),
            ("a1b2c3d4\\media_files\\m1", Some(b"media")),
            ("../outside.txt", Some(b"x")),
            ("/abs.txt", Some(b"x")),
            ("deadbeef/rgb_lib_db", Some(b"overwritten")),
            ("wallet/y.txt", Some(b"x")),
            ("a1b2c3d4/../z.txt", Some(b"x")),
            ("a1b2c3d4/../../up.txt", Some(b"x")),
            ("a1b2c3d4/C:/drive.txt", Some(b"x")),
        ]);
        let [manifest, data, metadata] = sealed(&odd, &key);
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest]),
            (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
            (BACKUP_KEY_DATA, vec![data]),
            (BACKUP_KEY_METADATA, vec![metadata]),
        ]);
        for expected in [Some(EXPECTED), None] {
            let root = tempfile::tempdir().unwrap();
            let target = target_with_another_wallet(root.path());
            let restored = restore(config(&server, key), &target, expected).unwrap();
            assert_eq!(restored, target.join(EXPECTED));
            assert_eq!(
                tree(root.path()),
                [
                    "data",
                    "data/a1b2c3d4",
                    "data/a1b2c3d4/.vss_restored",
                    "data/a1b2c3d4/media_files",
                    "data/a1b2c3d4/media_files/m1",
                    "data/a1b2c3d4/rgb_lib_db",
                    "data/deadbeef",
                    "data/deadbeef/rgb_lib_db",
                ],
                "{expected:?}"
            );
            assert_eq!(
                fs::read(target.join("deadbeef/rgb_lib_db")).unwrap(),
                b"another wallet"
            );
        }
    }

    #[test]
    fn a_plaintext_restore_extracts_the_sanitized_directory_and_nothing_else() {
        // encryption off in the config: the server's plaintext is taken, but only its wallet/
        // directory, as the wallet's
        let key = test_signing_key();
        let plain = zip_of(&[
            ("wallet/", None),
            ("wallet/rgb_lib_db", Some(b"server db")),
            ("deadbeef/rgb_lib_db", Some(b"overwritten by the server")),
            ("a1b2c3d4/planted", Some(b"x")),
            ("../outside.txt", Some(b"x")),
        ]);
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest_json(false, &plain)]),
            (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
            (BACKUP_KEY_DATA, vec![plain]),
        ]);
        for expected in [Some(EXPECTED), None] {
            let root = tempfile::tempdir().unwrap();
            let target = target_with_another_wallet(root.path());
            restore(
                config(&server, key).with_encryption(false),
                &target,
                expected,
            )
            .unwrap();
            assert_eq!(
                tree(root.path()),
                [
                    "data",
                    "data/a1b2c3d4",
                    "data/a1b2c3d4/.vss_restored",
                    "data/a1b2c3d4/rgb_lib_db",
                    "data/deadbeef",
                    "data/deadbeef/rgb_lib_db",
                ],
                "{expected:?}"
            );
            assert_eq!(
                fs::read(target.join("deadbeef/rgb_lib_db")).unwrap(),
                b"another wallet"
            );
        }
    }

    #[test]
    fn restore_refuses_an_encrypted_backup_the_server_names_for_another_wallet() {
        // upstream's restore_from_vss extracted it under its own name and returned the server's,
        // a directory that did not exist
        let key = test_signing_key();
        let [manifest, data, metadata] = uploaded("deadbeef", &key, true);
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest]),
            (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
            (BACKUP_KEY_DATA, vec![data]),
            (BACKUP_KEY_METADATA, vec![metadata]),
        ]);
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("data");
        let result = restore(config(&server, key), &target, None);
        assert!(
            matches!(result, Err(Error::FingerprintMismatch)),
            "{result:?}"
        );
        assert_eq!(tree(root.path()), NOTHING);
    }

    #[test]
    fn a_backup_without_a_wallet_file_or_whose_extraction_fails_leaves_nothing() {
        let key = test_signing_key();
        let damaged = {
            let mut zip = zip_of(&[
                ("wallet/", None),
                ("wallet/rgb_lib_db", Some(b"a database")),
                ("wallet/stash.dat", Some(b"damaged-in-transit")),
            ]);
            // (stored, so the content is there as it is: its checksum no longer matches)
            let at = zip
                .windows(18)
                .position(|w| w == b"damaged-in-transit")
                .unwrap();
            zip[at] = b'D';
            zip
        };
        for (name, zip, encrypted) in [
            (
                "no file",
                zip_of(&[("a1b2c3d4/", None), ("other/file", Some(b"x"))]),
                true,
            ),
            ("a damaged entry", damaged, false),
        ] {
            let (manifest, data, metadata) = if encrypted {
                let [manifest, data, metadata] = sealed(&zip, &key);
                (manifest, data, metadata)
            } else {
                (manifest_json(false, &zip), zip, vec![])
            };
            let mut answers = vec![
                (BACKUP_KEY_MANIFEST, vec![manifest]),
                (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
                (BACKUP_KEY_DATA, vec![data]),
            ];
            if encrypted {
                answers.push((BACKUP_KEY_METADATA, vec![metadata]));
            }
            let server = VssScript::start(answers);
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            let result = restore(
                config(&server, key).with_encryption(encrypted),
                &target,
                Some(EXPECTED),
            );
            assert!(result.is_err(), "{name}: {result:?}");
            // no wallet directory, whole or partial, and no staging directory
            assert_eq!(tree(root.path()), NOTHING, "{name}");
        }
    }

    #[test]
    fn restore_reads_the_manifest_once() {
        // upstream's restore_from_vss read the manifest twice, deciding the rename by the first
        // answer and the decryption by the second
        let key = test_signing_key();
        let [manifest, data, metadata] = uploaded(EXPECTED, &key, true);
        let [plaintext_manifest, _, _] = uploaded(EXPECTED, &key, false);
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest, plaintext_manifest]),
            (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
            (BACKUP_KEY_DATA, vec![data]),
            (BACKUP_KEY_METADATA, vec![metadata]),
        ]);
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("data");
        let restored = restore(config(&server, key), &target, None).unwrap();
        assert!(restored.join("some_file.txt").is_file());
        assert_eq!(server.reads(BACKUP_KEY_MANIFEST), 1);
    }

    // ERA fork: what a server writes into the manifest and the encryption metadata cannot crash a
    // restore, size its buffers, or make it take more or less than the manifest says

    fn chunked_manifest(encrypted: bool, chunk_count: usize, total_size: usize) -> Vec<u8> {
        serde_json::to_vec(&BackupManifest {
            chunk_count,
            total_size,
            encrypted,
            version: VSS_BACKUP_VERSION,
        })
        .unwrap()
    }

    #[test]
    fn malformed_encryption_metadata_is_an_error() {
        let key = test_signing_key();
        let [manifest, data, _] = uploaded(EXPECTED, &key, true);
        let good = VssEncryptionMetadata::new();
        for metadata in [
            // a nonce shorter than 19 bytes panicked
            serde_json::json!({"salt": good.salt, "nonce": "00", "version": 1}),
            serde_json::json!({"salt": good.salt, "nonce": "", "version": 1}),
            serde_json::json!({"salt": good.salt, "nonce": format!("{}00", good.nonce), "version": 1}),
            serde_json::json!({"salt": "00", "nonce": good.nonce, "version": 1}),
            serde_json::json!({"salt": good.salt, "nonce": "zz", "version": 1}),
        ] {
            let server = VssScript::start(vec![
                (BACKUP_KEY_MANIFEST, vec![manifest.clone()]),
                (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
                (BACKUP_KEY_DATA, vec![data.clone()]),
                (
                    BACKUP_KEY_METADATA,
                    vec![serde_json::to_vec(&metadata).unwrap()],
                ),
            ]);
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            let result = restore(config(&server, key), &target, Some(EXPECTED));
            assert!(
                matches!(result, Err(Error::VssError { .. })),
                "{metadata}: {result:?}"
            );
            assert!(!target.exists());
        }
        // nor does the short nonce panic the public helpers
        let short = VssEncryptionMetadata {
            salt: good.salt.clone(),
            nonce: "00".to_string(),
            version: VSS_BACKUP_VERSION,
        };
        assert!(encrypt_data(b"data", &key, &short, None).is_err());
        assert!(decrypt_data(&[0u8; 32], &key, &short, None).is_err());
    }

    #[test]
    fn a_manifest_no_upload_could_write_is_refused_before_any_download() {
        let key = test_signing_key();
        let [_, data, metadata] = uploaded(EXPECTED, &key, true);
        for (chunk_count, total_size) in [
            // Vec::with_capacity(total_size) panicked on this one, and a large valid size aborted
            (2, usize::MAX),
            (1, MAX_VSS_BACKUP_SIZE + 1),
            (0, 0),
            (0, data.len()),
            (1, 0),
            (3, 2),
        ] {
            let server = VssScript::start(vec![
                (
                    BACKUP_KEY_MANIFEST,
                    vec![chunked_manifest(true, chunk_count, total_size)],
                ),
                (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
                (BACKUP_KEY_DATA, vec![data.clone()]),
                ("backup/chunk/0", vec![data.clone()]),
                (BACKUP_KEY_METADATA, vec![metadata.clone()]),
            ]);
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            let result = restore(config(&server, key), &target, Some(EXPECTED));
            assert!(
                matches!(result, Err(Error::VssError { .. })),
                "{chunk_count} chunks, {total_size} bytes: {result:?}"
            );
            // none of the backup asked for
            assert_eq!(
                server.reads(BACKUP_KEY_DATA) + server.reads("backup/chunk/0"),
                0
            );
        }
    }

    #[test]
    fn data_that_does_not_add_up_to_its_manifest_is_refused() {
        let key = test_signing_key();
        let [_, data, metadata] = uploaded(EXPECTED, &key, true);
        let half = data.len() / 2;
        let restored = |answers: Vec<(&str, Vec<Vec<u8>>)>| {
            let mut answers = answers;
            answers.push((BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]));
            answers.push((BACKUP_KEY_METADATA, vec![metadata.clone()]));
            let server = VssScript::start(answers);
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            let result = restore(config(&server, key), &target, Some(EXPECTED))
                // (checked here: the directory goes with root)
                .map(|wallet_dir| wallet_dir.join("some_file.txt").is_file());
            let chunk_reads: Vec<usize> = (0..3)
                .map(|i| server.reads(&format!("{BACKUP_KEY_CHUNK_PREFIX}{i}")))
                .collect();
            (result, chunk_reads)
        };

        // one chunk, not the size the manifest gives
        for total_size in [data.len() + 1, data.len() - 1] {
            let (result, _) = restored(vec![
                (
                    BACKUP_KEY_MANIFEST,
                    vec![chunked_manifest(true, 1, total_size)],
                ),
                (BACKUP_KEY_DATA, vec![data.clone()]),
            ]);
            assert!(
                matches!(result, Err(Error::VssError { .. })),
                "{total_size}: {result:?}"
            );
        }

        // chunks, as an upload splits a larger backup: restored
        let manifest = chunked_manifest(true, 2, data.len());
        let (result, reads) = restored(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest.clone()]),
            ("backup/chunk/0", vec![data[..half].to_vec()]),
            ("backup/chunk/1", vec![data[half..].to_vec()]),
        ]);
        assert!(result.unwrap());
        assert_eq!(reads, [1, 1, 0]);

        // chunks adding up to more (stopped at the chunk that goes past: of three, the third is
        // never read), to fewer, or an empty one
        let mut longer = data[half..].to_vec();
        longer.push(0);
        for (chunk_count, chunks, expected_reads) in [
            (2, vec![data[..half].to_vec(), longer], [1, 1, 0]),
            (
                2,
                vec![data[..half].to_vec(), data[half..data.len() - 1].to_vec()],
                [1, 1, 0],
            ),
            (3, vec![data.clone(), vec![0], vec![0]], [1, 1, 0]),
            (2, vec![vec![], data.clone()], [1, 0, 0]),
        ] {
            let mut answers = vec![(
                BACKUP_KEY_MANIFEST,
                vec![chunked_manifest(true, chunk_count, data.len())],
            )];
            let keys = ["backup/chunk/0", "backup/chunk/1", "backup/chunk/2"];
            for (key, chunk) in keys.into_iter().zip(chunks) {
                answers.push((key, vec![chunk]));
            }
            let (result, reads) = restored(answers);
            assert!(matches!(result, Err(Error::VssError { .. })), "{result:?}");
            assert_eq!(reads, expected_reads);
        }
    }

    // ERA fork: a fingerprint's case is the host's (rgb-lib names a wallet directory after the
    // master fingerprint as given): compared ignoring case, restored under the backup's own name

    const UPPER: &str = "928E8C83";

    #[test]
    fn a_backup_named_in_upper_case_restores_under_its_own_name() {
        let key = test_signing_key();
        // encrypted, the server naming it as the upload did (from the archive), or in another case
        let [manifest, data, metadata] = uploaded(UPPER, &key, true);
        for named in [UPPER, "928e8c83"] {
            let server = VssScript::start(vec![
                (BACKUP_KEY_MANIFEST, vec![manifest.clone()]),
                (BACKUP_KEY_FINGERPRINT, vec![named.into()]),
                (BACKUP_KEY_DATA, vec![data.clone()]),
                (BACKUP_KEY_METADATA, vec![metadata.clone()]),
            ]);
            for expected in [None, Some("928e8c83"), Some(UPPER), Some("928e8C83")] {
                let root = tempfile::tempdir().unwrap();
                let target = root.path().join("data");
                let restored = restore(config(&server, key), &target, expected).unwrap();
                assert_eq!(restored, target.join(UPPER), "{named} {expected:?}");
                assert!(restored.join("some_file.txt").is_file());
            }
        }
        // plaintext (encryption off): the server's name for it
        let [manifest, data, _] = uploaded(UPPER, &key, false);
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest]),
            (BACKUP_KEY_FINGERPRINT, vec![UPPER.into()]),
            (BACKUP_KEY_DATA, vec![data]),
        ]);
        for expected in [None, Some("928e8c83")] {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            let restored = restore(
                config(&server, key).with_encryption(false),
                &target,
                expected,
            )
            .unwrap();
            assert_eq!(restored, target.join(UPPER), "{expected:?}");
        }
        // another wallet is another wallet, whatever the case
        let root = tempfile::tempdir().unwrap();
        let result = restore(
            config(&server, key).with_encryption(false),
            &root.path().join("data"),
            Some("928e8c84"),
        );
        assert!(
            matches!(result, Err(Error::FingerprintMismatch)),
            "{result:?}"
        );
    }

    /// A VSS server that keeps what it is given (putObjects) and answers getObject from it.
    struct VssStore {
        server: mockito::ServerGuard,
        _mocks: [mockito::Mock; 2],
    }

    impl VssStore {
        fn start() -> Self {
            use vss_client::prost::Message;
            use vss_client::types::{
                ErrorCode, ErrorResponse, GetObjectResponse, PutObjectRequest, PutObjectResponse,
            };
            // key: (version, value)
            type Kept = HashMap<String, (i64, Vec<u8>)>;
            let state: Arc<std::sync::Mutex<Kept>> = Arc::default();
            let key_of = |request: &mockito::Request| {
                GetObjectRequest::decode(&request.body().unwrap()[..])
                    .unwrap()
                    .key
            };
            let (known, stored, kept) = (state.clone(), state.clone(), state);
            let mut server = mockito::Server::new();
            let get = server
                .mock("POST", mockito::Matcher::Regex("/getObject$".to_string()))
                .with_status_code_from_request(move |request| {
                    if known.lock().unwrap().contains_key(&key_of(request)) {
                        200
                    } else {
                        404
                    }
                })
                .with_body_from_request(move |request| {
                    let key = key_of(request);
                    match stored.lock().unwrap().get(&key) {
                        Some((version, value)) => GetObjectResponse {
                            value: Some(KeyValue {
                                key,
                                version: *version,
                                value: value.clone(),
                            }),
                        }
                        .encode_to_vec(),
                        None => ErrorResponse {
                            error_code: ErrorCode::NoSuchKeyException as i32,
                            message: "no such key".to_string(),
                        }
                        .encode_to_vec(),
                    }
                })
                .create();
            let put = server
                .mock("POST", mockito::Matcher::Regex("/putObjects$".to_string()))
                .with_body_from_request(move |request| {
                    let request = PutObjectRequest::decode(&request.body().unwrap()[..]).unwrap();
                    let mut kept = kept.lock().unwrap();
                    for item in request.transaction_items {
                        let version = kept.get(&item.key).map_or(0, |(v, _)| *v);
                        kept.insert(item.key, (version + 1, item.value));
                    }
                    for item in request.delete_items {
                        kept.remove(&item.key);
                    }
                    PutObjectResponse {}.encode_to_vec()
                })
                .create();
            Self {
                server,
                _mocks: [get, put],
            }
        }

        fn config(&self, key: SecretKey) -> VssBackupConfig {
            VssBackupConfig::new(self.server.url(), "store".to_string(), key)
        }
    }

    #[test]
    fn a_wallet_named_in_upper_case_restores_from_its_own_upload() {
        use crate::keys::{WitnessVersion, generate_keys};
        use crate::wallet::SinglesigKeys;
        use crate::wallet::offline::RgbWalletOpsOffline;
        use crate::wallet::test::era_fixtures::dir_listing;
        use crate::wallet::test::get_test_wallet_raw;
        use crate::{BitcoinNetwork, utils::LOG_FILE};
        // a watch-only wallet the host names with an upper-case master fingerprint, which rgb-lib
        // takes as given
        let keys = generate_keys(BitcoinNetwork::Regtest, WitnessVersion::Taproot);
        let mut wallet_keys = SinglesigKeys::from_keys_no_mnemonic(&keys, None);
        wallet_keys.master_fingerprint = keys.master_fingerprint.to_uppercase();
        let wallet = get_test_wallet_raw(&wallet_keys, None, BitcoinNetwork::Regtest);
        let wallet_dir = wallet.get_wallet_dir();
        let name = wallet_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert_eq!(name, wallet_keys.master_fingerprint);
        let key = test_signing_key();
        let store = VssStore::start();
        let client = VssBackupClient::new(store.config(key)).unwrap();
        block_on(wallet.vss_backup(&client)).unwrap();
        drop(client);
        // what the restore must reproduce (the wallet's log is not backed up)
        let listing: String = dir_listing(wallet_dir.parent().unwrap(), &name)
            .lines()
            .filter(|line| !line.ends_with(&format!("/{LOG_FILE}")))
            .map(|line| format!("{line}\n"))
            .collect();
        for expected in [None, Some(name.to_lowercase()), Some(name.clone())] {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            let restored = restore(store.config(key), &target, expected.as_deref()).unwrap();
            assert_eq!(restored, target.join(&name), "{expected:?}");
            assert_eq!(dir_listing(&target, &name), listing, "{expected:?}");
        }
    }

    #[test]
    fn a_restore_never_writes_into_an_existing_wallet_directory() {
        let key = test_signing_key();
        let [manifest, data, metadata] = uploaded(EXPECTED, &key, true);
        let server = VssScript::start(vec![
            (BACKUP_KEY_MANIFEST, vec![manifest]),
            (BACKUP_KEY_FINGERPRINT, vec![EXPECTED.into()]),
            (BACKUP_KEY_DATA, vec![data]),
            (BACKUP_KEY_METADATA, vec![metadata]),
        ]);
        for existing in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("data");
            // the wallet's directory already there, holding a file or empty
            fs::create_dir_all(target.join(EXPECTED)).unwrap();
            if existing {
                fs::write(target.join(EXPECTED).join("rgb_lib_db"), b"the wallet").unwrap();
            }
            let result = restore(config(&server, key), &target, Some(EXPECTED));
            assert!(
                matches!(result, Err(Error::WalletDirAlreadyExists { .. })),
                "{result:?}"
            );
            let left: Vec<String> = fs::read_dir(target.join(EXPECTED))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
                .collect();
            let expected: &[&str] = if existing { &["rgb_lib_db"] } else { &[] };
            assert_eq!(left, expected);
        }
    }

    #[test]
    fn a_failed_restore_removes_only_what_it_made() {
        // no backup on the server: the restore fails once its log is set up
        let server = VssScript::start(vec![]);
        let key = test_signing_key();

        // a target reached through a directory that is not there and "..", naming one that is
        // (and is empty): the one the restore made goes, the one that was there stays
        let root = tempfile::tempdir().unwrap();
        let there = root.path().join("there");
        fs::create_dir(&there).unwrap();
        let target = root.path().join("missing").join("..").join("there");
        assert!(restore(config(&server, key), &target, Some(EXPECTED)).is_err());
        assert!(there.is_dir());
        assert!(!root.path().join("missing").exists());
        assert_eq!(tree(root.path()), ["there"]);

        // a target that cannot be made (a file on its way): what was made on the way goes too
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("file"), b"a file").unwrap();
        let target = root.path().join("new").join("..").join("file").join("data");
        let result = restore(config(&server, key), &target, Some(EXPECTED));
        assert!(matches!(result, Err(Error::IO { .. })), "{result:?}");
        assert_eq!(tree(root.path()), ["file"]);

        // files of the restore log's names already in the target: neither written to nor removed
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("data");
        fs::create_dir(&target).unwrap();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let names: Vec<String> = (0..3)
            .flat_map(|i| {
                let name = format!("vss_restore_{}", now + i);
                [name.clone(), format!("{name}_1")]
            })
            .collect();
        for name in &names {
            fs::write(target.join(name), b"someone else's file").unwrap();
        }
        assert!(restore(config(&server, key), &target, Some(EXPECTED)).is_err());
        let mut left: Vec<String> = fs::read_dir(&target)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        let mut expected = names.clone();
        expected.sort();
        assert_eq!(left, expected);
        for name in &names {
            assert_eq!(fs::read(target.join(name)).unwrap(), b"someone else's file");
        }
    }
}
