//! Wallet utilities.
//!
//! This module defines some utility methods and structures of the wallet.

use super::*;

pub use crate::no_wallet::{recipient_id_from_script_buf, script_buf_from_recipient_id};

const TIMESTAMP_FORMAT: &[time::format_description::BorrowedFormatItem] = time::macros::format_description!(
    "[year]-[month]-[day]T[hour repr:24]:[minute]:[second].[subsecond digits:3]+00"
);

#[cfg(feature = "rgb_fs_store_migration")]
pub(crate) const RGB_RUNTIME_DIR: &str = "rgb";

/// Where the legacy filesystem RGB store is moved to be deleted, once migrated into the DB, so
/// that retiring it is atomic. Only ever on disk between that rename and the delete: a crash in
/// between leaves this dir, which the next wallet open sweeps up and every backup skips.
pub(crate) const RGB_RUNTIME_DELETING_DIR: &str = "rgb.deleting";
pub(crate) const LOG_FILE: &str = "log";

pub(crate) const PURPOSE: u8 = 86;
pub(crate) const COIN_RGB_MAINNET: u32 = 827166;
pub(crate) const COIN_RGB_TESTNET: u32 = 827167;
pub(crate) const ACCOUNT: u8 = 0;
pub(crate) const KEYCHAIN_RGB: u8 = 0;
pub(crate) const KEYCHAIN_BTC: u8 = 0;

#[cfg(any(feature = "electrum", feature = "esplora"))]
pub(crate) const INDEXER_STOP_GAP: usize = 20;
#[cfg(feature = "electrum")]
pub(crate) const INDEXER_BATCH_SIZE: usize = 5;
#[cfg(feature = "esplora")]
pub(crate) const INDEXER_PARALLEL_REQUESTS: usize = 5;

#[cfg(feature = "_online")]
const PROXY_PROTOCOL_VERSION: &str = "1.0";

// sea-orm with runtime-tokio needs a tokio runtime for connection pool management
static TOKIO_RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().expect("failed to create the runtime"));

/// Block on a future, spawning a new thread if already inside a Tokio runtime.
pub fn block_on<F>(future: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        // avoid blocking the Tokio runtime thread; spawn a new thread
        std::thread::scope(|s| {
            s.spawn(|| TOKIO_RUNTIME.block_on(future))
                .join()
                .expect("rgb-lib block_on thread panicked")
        })
    } else {
        TOKIO_RUNTIME.block_on(future)
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn adjust_canonicalization<P: AsRef<Path>>(p: P) -> String {
    p.as_ref().display().to_string()
}

#[cfg(target_os = "windows")]
pub(crate) fn adjust_canonicalization<P: AsRef<Path>>(p: P) -> String {
    const VERBATIM_PREFIX: &str = r#"\\?\"#;
    let p = p.as_ref().display().to_string();
    if p.starts_with(VERBATIM_PREFIX) {
        p[VERBATIM_PREFIX.len()..].to_string()
    } else {
        p
    }
}

fn deserialize_str_or_number<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + Copy,
    T::Err: fmt::Display,
{
    struct StringOrNumberVisitor<T>(std::marker::PhantomData<T>);

    impl<T> Visitor<'_> for StringOrNumberVisitor<T>
    where
        T: FromStr + Copy,
        T::Err: fmt::Display,
    {
        type Value = Option<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string, a number, or null")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            T::from_str(&value.to_string())
                .map(Some)
                .map_err(de::Error::custom)
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            T::from_str(&value.to_string())
                .map(Some)
                .map_err(de::Error::custom)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            value.parse::<T>().map(Some).map_err(|e| {
                de::Error::invalid_value(Unexpected::Str(value), &e.to_string().as_str())
            })
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_any(StringOrNumberVisitor(std::marker::PhantomData))
}

pub(crate) fn from_str_or_number_mandatory<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + Copy,
    T::Err: fmt::Display,
{
    match deserialize_str_or_number(deserializer)? {
        Some(val) => Ok(val),
        None => Err(de::Error::custom("expected a number but got null")),
    }
}

pub(crate) fn from_str_or_number_optional<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr + Copy,
    T::Err: fmt::Display,
{
    deserialize_str_or_number(deserializer)
}

pub(crate) fn str_to_xpub(xpub: &str, network_kind: &NetworkKind) -> Result<Xpub, Error> {
    let pubkey_btc = Xpub::from_str(xpub)?;
    let extended_key_btc: ExtendedKey = ExtendedKey::from(pubkey_btc);
    Ok(extended_key_btc.into_xpub(*network_kind, &Secp256k1::new()))
}

pub(crate) fn get_coin_type(bitcoin_network: &BitcoinNetwork, rgb: bool) -> u32 {
    match (bitcoin_network, rgb) {
        (BitcoinNetwork::Mainnet, true) => COIN_RGB_MAINNET,
        (_, true) => COIN_RGB_TESTNET,
        (_, false) => u32::from(*bitcoin_network != BitcoinNetwork::Mainnet),
    }
}

/// Where the two sides of a singlesig wallet live, resolved against the wallet network.
///
/// The default is rgb-lib's standard layout: the colored side on its own coin type
/// (827166' on mainnet, 827167' elsewhere) at keychain 0, the vanilla side on the standard
/// Bitcoin coin type (0'/1') at keychain 0 or the requested `vanilla_keychain`.
///
/// Hosts whose signer can only export the standard BIP-86 account (hardware wallets) can put
/// both sides under that one account by overriding the colored coin type and choosing two
/// distinct keychain indexes (see [`KeychainLayoutOverrides`]). The layout is purely local:
/// invoices, consignments, ACKs and witness transactions never carry derivation paths, so
/// singlesig transfers interoperate with wallets that use the default layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeychainLayout {
    pub(crate) colored_coin_type: u32,
    pub(crate) vanilla_coin_type: u32,
    pub(crate) colored_keychain: u8,
    pub(crate) vanilla_keychain: u8,
}

impl KeychainLayout {
    pub(crate) fn resolve(
        bitcoin_network: &BitcoinNetwork,
        overrides: &KeychainLayoutOverrides,
        vanilla_keychain: Option<u8>,
    ) -> Result<Self, Error> {
        let layout = Self {
            colored_coin_type: overrides
                .colored_coin_type
                .unwrap_or(get_coin_type(bitcoin_network, true)),
            vanilla_coin_type: overrides
                .vanilla_coin_type
                .unwrap_or(get_coin_type(bitcoin_network, false)),
            colored_keychain: overrides.colored_keychain.unwrap_or(KEYCHAIN_RGB),
            vanilla_keychain: vanilla_keychain.unwrap_or(KEYCHAIN_BTC),
        };
        for (side, coin_type) in [
            ("colored", layout.colored_coin_type),
            ("vanilla", layout.vanilla_coin_type),
        ] {
            if ChildNumber::from_hardened_idx(coin_type).is_err() {
                return Err(Error::InvalidKeychainLayout {
                    details: format!("{side} coin type {coin_type} is not a valid hardened index"),
                });
            }
        }
        if layout.colored_coin_type == layout.vanilla_coin_type
            && layout.colored_keychain == layout.vanilla_keychain
        {
            // one keychain would feed both BDK keychains: colored UTXOs would be spendable as
            // vanilla ones and RGB allocations could be burnt by a plain BTC send
            return Err(Error::InvalidKeychainLayout {
                details: s!("colored and vanilla sides resolve to the same keychain"),
            });
        }
        Ok(layout)
    }
}

pub(crate) fn get_account_derivation_children(
    witness_version: WitnessVersion,
    coin_type: u32,
) -> Vec<ChildNumber> {
    vec![
        ChildNumber::from_hardened_idx(witness_version.purpose()).unwrap(),
        ChildNumber::from_hardened_idx(coin_type).unwrap(),
        ChildNumber::from_hardened_idx(ACCOUNT as u32).unwrap(),
    ]
}

fn derive_account_xprv_from_mnemonic(
    bitcoin_network: &BitcoinNetwork,
    mnemonic: &str,
    coin_type: u32,
    witness_version: WitnessVersion,
) -> Result<(Xpriv, Fingerprint), Error> {
    let account_derivation_children = get_account_derivation_children(witness_version, coin_type);
    let mnemonic = Mnemonic::parse_in(Language::English, mnemonic.to_string())?;
    let master_xprv = Xpriv::new_master(*bitcoin_network, &mnemonic.to_seed("")).unwrap();
    let master_xpub = Xpub::from_priv(&Secp256k1::new(), &master_xprv);
    let master_fingerprint = master_xpub.fingerprint();
    let account_xprv = master_xprv.derive_priv(&Secp256k1::new(), &account_derivation_children)?;
    Ok((account_xprv, master_fingerprint))
}

fn get_xpub_from_xprv(xprv: &Xpriv) -> Xpub {
    Xpub::from_priv(&Secp256k1::new(), xprv)
}

/// Get the account-level xPriv and xPub for the given mnemonic, Bitcoin network and witness
/// version, based on the requested wallet side (colored or vanilla).
pub fn get_account_data(
    bitcoin_network: &BitcoinNetwork,
    mnemonic: &str,
    rgb: bool,
    witness_version: WitnessVersion,
) -> Result<(Xpriv, Xpub, Fingerprint), Error> {
    get_account_data_at_coin_type(
        bitcoin_network,
        mnemonic,
        get_coin_type(bitcoin_network, rgb),
        witness_version,
    )
}

/// Get the account-level xPriv and xPub for the given mnemonic, Bitcoin network, coin type and
/// witness version.
///
/// Use this to prepare [`crate::wallet::SinglesigKeys`] for a non-default keychain layout
/// (see `colored_coin_type` / `vanilla_coin_type`).
pub fn get_account_data_at_coin_type(
    bitcoin_network: &BitcoinNetwork,
    mnemonic: &str,
    coin_type: u32,
    witness_version: WitnessVersion,
) -> Result<(Xpriv, Xpub, Fingerprint), Error> {
    let (account_xprv, master_fingerprint) =
        derive_account_xprv_from_mnemonic(bitcoin_network, mnemonic, coin_type, witness_version)?;
    let account_xpub = get_xpub_from_xprv(&account_xprv);
    Ok((account_xprv, account_xpub, master_fingerprint))
}

pub(crate) fn get_account_xpubs(
    bitcoin_network: &BitcoinNetwork,
    mnemonic: &str,
    witness_version: WitnessVersion,
) -> Result<(Xpub, Xpub), Error> {
    let (_, account_xpub_vanilla, _) =
        get_account_data(bitcoin_network, mnemonic, false, witness_version)?;
    let (_, account_xpub_colored, _) =
        get_account_data(bitcoin_network, mnemonic, true, witness_version)?;
    Ok((account_xpub_vanilla, account_xpub_colored))
}

fn derive_descriptor(
    bitcoin_network: &BitcoinNetwork,
    mnemonic: &str,
    coin_type: u32,
    keychain: u8,
    expected_xpub: &Xpub,
    witness_version: WitnessVersion,
) -> Result<String, Error> {
    let (account_xprv, account_xpub, master_fingerprint) =
        get_account_data_at_coin_type(bitcoin_network, mnemonic, coin_type, witness_version)?;
    if account_xpub != *expected_xpub {
        return Err(Error::InvalidBitcoinKeys);
    }
    calculate_descriptor_from_xprv(
        &master_fingerprint,
        coin_type,
        account_xprv,
        keychain,
        witness_version,
    )
}

pub(crate) fn get_descriptors(
    bitcoin_network: &BitcoinNetwork,
    mnemonic: &str,
    layout: &KeychainLayout,
    expected_xpub_btc: &Xpub,
    expected_xpub_rgb: &Xpub,
    witness_version: WitnessVersion,
) -> Result<WalletDescriptors, Error> {
    let colored = derive_descriptor(
        bitcoin_network,
        mnemonic,
        layout.colored_coin_type,
        layout.colored_keychain,
        expected_xpub_rgb,
        witness_version,
    )?;
    let vanilla = derive_descriptor(
        bitcoin_network,
        mnemonic,
        layout.vanilla_coin_type,
        layout.vanilla_keychain,
        expected_xpub_btc,
        witness_version,
    )?;
    Ok(WalletDescriptors { colored, vanilla })
}

pub(crate) fn get_descriptors_from_xpubs(
    master_fingerprint: &str,
    xpub_rgb: &Xpub,
    xpub_btc: &Xpub,
    layout: &KeychainLayout,
    witness_version: WitnessVersion,
) -> Result<WalletDescriptors, Error> {
    let master_fingerprint =
        Fingerprint::from_str(master_fingerprint).map_err(|_| Error::InvalidFingerprint)?;
    let colored = calculate_descriptor_from_xpub(
        &master_fingerprint,
        layout.colored_coin_type,
        xpub_rgb,
        layout.colored_keychain,
        witness_version,
    )?;
    let vanilla = calculate_descriptor_from_xpub(
        &master_fingerprint,
        layout.vanilla_coin_type,
        xpub_btc,
        layout.vanilla_keychain,
        witness_version,
    )?;
    Ok(WalletDescriptors { colored, vanilla })
}

pub(crate) fn parse_address_str(
    address: &str,
    bitcoin_network: BitcoinNetwork,
) -> Result<BdkAddress, Error> {
    BdkAddress::from_str(address)
        .map_err(|e| Error::InvalidAddress {
            details: e.to_string(),
        })?
        .require_network(bitcoin_network.into())
        .map_err(|_| Error::InvalidAddress {
            details: s!("belongs to another network"),
        })
}

fn get_derivation_path(keychain: u8) -> DerivationPath {
    let derivation_path = vec![ChildNumber::from_normal_idx(keychain as u32).unwrap()];
    DerivationPath::from_iter(derivation_path.clone())
}

pub(crate) fn get_extended_derivation_path(
    mut account_derivation_children: Vec<ChildNumber>,
    keychain: u8,
) -> DerivationPath {
    let keychain_child = ChildNumber::from_normal_idx(keychain as u32).unwrap();
    account_derivation_children.push(keychain_child);
    DerivationPath::from_iter(account_derivation_children.clone())
}

pub(crate) fn calculate_descriptor_from_xprv(
    master_fingerprint: &Fingerprint,
    coin_type: u32,
    xprv: Xpriv,
    keychain: u8,
    witness_version: WitnessVersion,
) -> Result<String, Error> {
    // derive final xpub from account-level xpub
    let path = get_derivation_path(keychain);
    let der_xprv = &xprv
        .derive_priv(&Secp256k1::new(), &path)
        .expect("provided path should be derivable in an xprv");
    // derive descriptor with master fingerprint and full derivation path
    let account_derivation_children = get_account_derivation_children(witness_version, coin_type);
    let full_path = get_extended_derivation_path(account_derivation_children, keychain);
    let origin_prv: KeySource = (*master_fingerprint, full_path.clone());
    let der_xprv_desc_key: DescriptorKey<Segwitv0> = der_xprv
        .into_descriptor_key(Some(origin_prv), DerivationPath::default())
        .expect("should be able to convert xprv in a descriptor key");
    let Secret(key, _, _) = der_xprv_desc_key else {
        unreachable!("into_descriptor_key on an Xpriv always yields a Secret variant")
    };
    Ok(format!("{}({key})", witness_version.descriptor_fn()))
}

pub(crate) fn calculate_descriptor_from_xpub(
    master_fingerprint: &Fingerprint,
    coin_type: u32,
    xpub: &Xpub,
    keychain: u8,
    witness_version: WitnessVersion,
) -> Result<String, Error> {
    // derive final xpub from account-level xpub
    let path = get_derivation_path(keychain);
    let der_xpub = xpub
        .derive_pub(&Secp256k1::new(), &path)
        .expect("provided path should be derivable in an xpub");
    // derive descriptor with master fingerprint and full derivation path
    let account_derivation_children = get_account_derivation_children(witness_version, coin_type);
    let full_path = get_extended_derivation_path(account_derivation_children, keychain);
    let origin_pub: KeySource = (*master_fingerprint, full_path);
    let der_xpub_desc_key: DescriptorKey<Segwitv0> = der_xpub
        .into_descriptor_key(Some(origin_pub), DerivationPath::default())
        .expect("should be able to convert xpub in a descriptor key");
    let Public(key, _, _) = der_xpub_desc_key else {
        unreachable!("into_descriptor_key on an Xpub always yields a Public variant")
    };
    Ok(format!("{}({key})", witness_version.descriptor_fn()))
}

/// The retention grace advertised by a proxy (`retention_grace_secs`), i.e. how long after an
/// expiration it keeps the data awaiting retrieval. A proxy not advertising it is considered to
/// grant none.
#[cfg(feature = "_online")]
pub(crate) fn proxy_retention_grace(info: &ServerInfoResponse) -> u32 {
    #[cfg(test)]
    if let Some(grace) = mock_retention_grace() {
        return grace;
    }
    info.retention_grace_secs
        .as_ref()
        .and_then(|grace| grace.parse::<u32>().ok())
        .unwrap_or(0)
}

/// The max number of media per transfer advertised by a proxy (`max_media`), `None` if it doesn't
/// advertise a valid one.
#[cfg(feature = "_online")]
pub(crate) fn proxy_max_media(info: &ServerInfoResponse) -> Option<usize> {
    info.max_media.as_ref().and_then(|max| max.parse().ok())
}

/// Check that the proxy at `proxy_url` runs the supported protocol version, returning its info.
#[cfg(feature = "_online")]
pub(crate) fn check_proxy(proxy_url: &str) -> Result<ServerInfoResponse, Error> {
    check_proxy_routed(proxy_url, None)
}

/// ERA fork: [`check_proxy`], through `forwarder` when one is given. The forwarder's refusal and a
/// URL it may not be given are [`Error::ForwarderRefused`] and [`Error::InvalidForwardTarget`],
/// not "unable to connect to proxy".
#[cfg(feature = "_online")]
pub(crate) fn check_proxy_routed(
    proxy_url: &str,
    forwarder: Option<&Forwarder>,
) -> Result<ServerInfoResponse, Error> {
    let proxy_client = ProxyClient::new_routed(proxy_url, forwarder)?;
    let server_info = proxy_client.get_info()?;
    if let Some(info) = server_info.result {
        if info.protocol_version == *PROXY_PROTOCOL_VERSION {
            return Ok(info);
        } else {
            return Err(Error::InvalidProxyProtocol {
                version: info.protocol_version,
            });
        }
    }
    let details = match server_info.error {
        Some(err) => err.message,
        None => s!("proxy response has neither result nor error"),
    };
    Err(Error::Proxy { details })
}

#[cfg(feature = "_online")]
pub(crate) fn get_indexer_and_resolver(
    indexer_config: &IndexerConfig,
    bitcoin_network: BitcoinNetwork,
) -> Result<(Indexer, AnyResolver), Error> {
    #[cfg_attr(not(feature = "bitcoind"), allow(unused_mut))]
    let mut indexer = build_indexer(indexer_config)?;
    // this also checks connectivity (and credentials for bitcoind)
    indexer.block_hash(0).map_err(|e| Error::InvalidIndexer {
        details: unreachable_indexer_details(indexer_config, &e),
    })?;
    #[allow(irrefutable_let_patterns)]
    #[cfg(feature = "bitcoind")]
    if let Indexer::Bitcoind(client) = &mut indexer {
        client.block_filters = checked_bitcoind_indexes(&client.inner)?.block_filters;
    }

    let resolver = get_resolver(indexer_config, bitcoin_network)?;

    Ok((indexer, resolver))
}

/// Build the indexer client described by `indexer_config`.
///
/// Fails if support for the configured indexer is not compiled in. The returned client has not
/// contacted the service yet, except for electrum, whose client connects on creation.
#[cfg(feature = "_online")]
pub(crate) fn build_indexer(indexer_config: &IndexerConfig) -> Result<Indexer, Error> {
    match indexer_config {
        #[cfg(feature = "bitcoind")]
        IndexerConfig::Bitcoind { url, auth } => {
            let client =
                BitcoindRpcClient::new(url, auth.into()).map_err(|e| Error::InvalidIndexer {
                    details: unreachable_indexer_details(indexer_config, &e),
                })?;
            Ok(Indexer::Bitcoind(Box::new(BitcoindClient {
                inner: client,
                block_filters: false,
            })))
        }
        #[cfg(feature = "electrum")]
        IndexerConfig::Electrum { url } => {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let client = ElectrumClient::from_config(url, electrum_config()).map_err(|e| {
                Error::InvalidIndexer {
                    details: unreachable_indexer_details(indexer_config, &e),
                }
            })?;
            Ok(Indexer::Electrum(Box::new(BdkElectrumClient::new(client))))
        }
        #[cfg(feature = "esplora")]
        IndexerConfig::Esplora { url } => Ok(Indexer::Esplora(Box::new(
            EsploraClient::from_builder(esplora_config(url)),
        ))),
        #[cfg(not(all(feature = "bitcoind", feature = "electrum", feature = "esplora")))]
        _ => {
            let feature = match indexer_config {
                IndexerConfig::Bitcoind { .. } => "bitcoind",
                IndexerConfig::Electrum { .. } => "electrum",
                IndexerConfig::Esplora { .. } => "esplora",
            };
            Err(Error::InvalidIndexer {
                details: format!(
                    "{feature} support is not enabled (requires the `{feature}` feature)"
                ),
            })
        }
    }
}

/// Detect which indexer is served at `indexer_url`: a Bitcoin Core node when `bitcoind_auth` is
/// provided, otherwise an electrum or esplora server (detected by trying electrum first).
///
/// Returns `None` if no supported indexer can be detected.
#[cfg(feature = "_online")]
pub(crate) fn detect_indexer(
    indexer_url: &str,
    bitcoind_auth: Option<&BitcoindAuth>,
) -> Option<IndexerConfig> {
    if let Some(auth) = bitcoind_auth {
        return cfg!(feature = "bitcoind").then(|| IndexerConfig::Bitcoind {
            url: indexer_url.to_string(),
            auth: auth.clone(),
        });
    }
    let electrum = IndexerConfig::Electrum {
        url: indexer_url.to_string(),
    };
    if build_indexer(&electrum).is_ok() {
        return Some(electrum);
    }
    cfg!(feature = "esplora").then(|| IndexerConfig::Esplora {
        url: indexer_url.to_string(),
    })
}

pub(crate) fn hash_bytes(data: &[u8]) -> Vec<u8> {
    sha256_digest(data).to_vec()
}

pub(crate) fn hash_bytes_hex(data: &[u8]) -> String {
    hex::encode(hash_bytes(data))
}

/// Remove the RGB data (operations, consumers and commitment data) from the given PSBT, once they
/// are committed: the commitment is in its TX and the operations are in the fascia, so they aren't
/// needed to sign it and broadcast its TX, and, spending UTXOs with many allocations, they could
/// make the PSBT too large to be parsed back (see [`check_psbt_parsable`]).
#[cfg(feature = "wallet")]
pub(crate) fn strip_rgb_data(psbt: &mut Psbt) {
    let is_rgb = |key: &bdk_wallet::bitcoin::psbt::raw::ProprietaryKey| {
        [
            &PSBT_RGB_PREFIX[..],
            &PSBT_MPC_PREFIX[..],
            &PSBT_OPRET_PREFIX[..],
            &PSBT_TAPRET_PREFIX[..],
        ]
        .contains(&key.prefix.as_slice())
    };
    psbt.proprietary.retain(|key, _| !is_rgb(key));
    for input in &mut psbt.inputs {
        input.proprietary.retain(|key, _| !is_rgb(key));
    }
    for output in &mut psbt.outputs {
        output.proprietary.retain(|key, _| !is_rgb(key));
    }
}

/// Check that the given PSBT can be parsed back, which fails if its global map exceeds
/// MAX_VEC_SIZE bytes. Such a PSBT can't be signed by an external signer nor be read back to
/// broadcast its TX.
///
/// This is a safety check, run before anything is saved or posted: once its RGB data are removed
/// (see [`strip_rgb_data`]), the PSBT of an operation stays far below that bound.
#[cfg(feature = "wallet")]
pub(crate) fn check_psbt_parsable(psbt: &Psbt) -> Result<(), Error> {
    Psbt::deserialize(&psbt.serialize())
        .map(|_| ())
        .map_err(|e| Error::Internal {
            details: format!("PSBT can't be parsed back: {e}"),
        })
}

/// Write a fascia file atomically, so a crash mid-write cannot leave a torn one behind.
#[cfg(any(feature = "_online", feature = "rgb_fs_store_migration"))]
pub(crate) fn write_fascia(path: &Path, fascia: &Fascia) -> Result<(), Error> {
    let bytes = fascia
        .to_strict_serialized::<FASCIA_MAX_LEN>()
        .map_err(|e| Error::Internal {
            details: format!("cannot serialize fascia {}: {e}", path.display()),
        })?;
    atomic_write(path, &bytes)
}

#[cfg(feature = "_online")]
pub(crate) fn hash_file(path: &Path) -> Result<String, Error> {
    let mut file = fs::File::open(path)?;
    let mut engine = sha256::HashEngine::default();
    let mut buffer = [0u8; 8192];
    while let Ok(n) = file.read(&mut buffer) {
        if n == 0 {
            break;
        }
        engine.input(&buffer[..n]);
    }
    Ok(sha256::Hash::from_engine(engine).to_string())
}

fn log_timestamp(io: &mut dyn io::Write) -> io::Result<()> {
    let now: time::OffsetDateTime = now();
    write!(
        io,
        "{}",
        now.format(TIMESTAMP_FORMAT)
            .expect("OffsetDateTime::format with a static format description is infallible")
    )
}

pub(crate) fn setup_logger<P: AsRef<Path>>(
    log_path: P,
    log_name: Option<&str>,
) -> Result<(Logger, AsyncGuard), Error> {
    let log_file = log_name.unwrap_or(LOG_FILE);
    let log_filepath = log_path.as_ref().join(log_file);
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_filepath)?;

    let decorator = PlainDecorator::new(file);
    let drain = FullFormat::new(decorator)
        .use_custom_timestamp(log_timestamp)
        .use_file_location();
    let (drain, async_guard) = slog_async::Async::new(drain.build().fuse()).build_with_guard();
    let logger = Logger::root(drain.fuse(), o!());

    Ok((logger, async_guard))
}

pub(crate) fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

pub(crate) struct DumbResolver;

impl ResolveWitness for DumbResolver {
    fn resolve_witness(&self, _: RgbTxid) -> Result<WitnessStatus, WitnessResolverError> {
        unreachable!()
    }

    fn check_chain_net(&self, _: ChainNet) -> Result<(), WitnessResolverError> {
        Ok(())
    }
}

/// Wrapper for the RGB stock.
///
/// Every read and write goes through the borrowed database transaction, so concurrent access is
/// serialized by the database itself (a single connection per process, SQLite locking across
/// processes) and no separate lock is needed.
// `pub` only because it appears in the signatures of `WalletOffline`/`WalletOnline` methods,
// supertraits of the public `RgbWalletOps*` traits; not part of the public API
#[doc(hidden)]
#[derive(Debug)]
pub struct RgbStock<'txn> {
    /// The RGB stock, writing through the shared database transaction borrowed for `'txn`
    stock: Stock<&'txn DbTxn>,
}

impl<'txn> RgbStock<'txn> {
    /// Build an [`RgbStock`] whose `Stock` reads and writes through the shared rgb-lib
    /// transaction. The store borrows `db_txn` for `'txn`, so the stock cannot outlive the
    /// transaction's commit.
    pub(crate) fn new(db_txn: &'txn DbTxn) -> Self {
        Self {
            stock: Stock::with(db_txn),
        }
    }

    /// ERA fork (CC-99): `fascia` without the bundles the stash already holds, `None` when it holds
    /// them all.
    ///
    /// Completing a spend consumes a fascia the stash may hold already. On beta.8 the stock writes
    /// through the rgb-lib transaction, so a consume that did not commit leaves nothing behind and
    /// a held bundle means a committed one; the check stays so that a completion never consumes a
    /// bundle twice, whatever the store does with a second consume.
    #[cfg(feature = "_online")]
    pub(crate) fn fascia_unknown_part(&self, fascia: Fascia) -> Option<Fascia> {
        let seal_witness = fascia.seal_witness().clone();
        let store = self.stock.as_store();
        let unknown: BTreeMap<ContractId, rgbstd::TransitionBundle> = fascia
            .into_bundles()
            .into_iter()
            .filter(|(_, bundle)| !matches!(store.bundle(bundle.bundle_id()), Ok(Some(_))))
            .collect();
        Confined::try_from(unknown)
            .ok()
            .map(|bundles| Fascia::new(seal_witness, bundles))
    }

    #[cfg(feature = "_online")]
    pub(crate) fn accept_transfer<R: ResolveWitness>(
        &mut self,
        contract: ValidTransfer,
        resolver: &R,
    ) -> Result<(), Error> {
        self.stock
            .accept_transfer(contract, resolver)
            .map_err(Error::from)
    }

    pub(crate) fn consume_fascia(
        &mut self,
        fascia: Fascia,
        witness_ord: Option<WitnessOrd>,
    ) -> Result<(), Error> {
        struct FasciaResolver {
            witness_id: RgbTxid,
            witness_ord: WitnessOrd,
        }
        impl WitnessOrdProvider for FasciaResolver {
            fn witness_ord(&self, witness_id: RgbTxid) -> Result<WitnessOrd, WitnessResolverError> {
                debug_assert_eq!(witness_id, self.witness_id);
                Ok(self.witness_ord)
            }
        }

        let resolver = FasciaResolver {
            witness_id: fascia.witness_id(),
            witness_ord: witness_ord.unwrap_or(WitnessOrd::Tentative),
        };

        self.stock
            .consume_fascia(fascia, resolver)
            .map_err(Error::from)
    }

    pub(crate) fn contract_wrapper<C: IssuerWrapper>(
        &self,
        contract_id: ContractId,
    ) -> Result<C::Wrapper<ContractStateSnapshot>, Error> {
        self.stock
            .contract_wrapper::<C>(contract_id)
            .map_err(Error::from)
    }

    /// The assignments on the given outpoints of every contract assigning state there, in a
    /// single store query
    #[cfg(feature = "_online")]
    pub(crate) fn assignments_by_contract(
        &self,
        outpoints: impl IntoIterator<Item = impl Into<OutPoint>>,
    ) -> Result<BTreeMap<ContractId, ContractAssignments>, Error> {
        self.stock
            .assignments_by_contract(outpoints)
            .map_err(Error::from)
    }

    pub(crate) fn genesis(&self, contract_id: ContractId) -> Result<Genesis, Error> {
        self.stock
            .as_store()
            .genesis(contract_id)?
            .ok_or_else(|| Error::Internal {
                details: format!("genesis {contract_id} not found"),
            })
    }

    pub(crate) fn import_contract<R: ResolveWitness>(
        &mut self,
        contract: ValidContract,
        resolver: &R,
    ) -> Result<(), Error> {
        self.stock
            .import_contract(contract, resolver)
            .map_err(Error::from)
    }

    pub(crate) fn import_schema_definition(
        &mut self,
        schema_def: SchemaDefinition,
    ) -> Result<(), Error> {
        self.stock
            .import_schema_definition(schema_def)
            .map_err(Error::from)
    }

    pub(crate) fn consignment_data<const TRANSFER: bool>(
        &self,
        consignment: &ValidConsignment<TRANSFER>,
    ) -> Result<ContractData<FilteredContractState>, Error> {
        self.stock
            .consignment_data(consignment)
            .map_err(Error::from)
    }

    pub(crate) fn contract_assignments_for(
        &self,
        contract_id: ContractId,
        outpoints: impl IntoIterator<Item = impl Into<OutPoint>>,
    ) -> Result<ContractAssignments, Error> {
        self.stock
            .contract_assignments_for(contract_id, outpoints)
            .map_err(Error::from)
    }

    #[cfg(feature = "_online")]
    pub(crate) fn contract_schema(&self, contract_id: ContractId) -> Result<Schema, Error> {
        self.stock
            .as_store()
            .contract_schema(contract_id)?
            .ok_or_else(|| Error::Internal {
                details: format!("schema for contract {contract_id} not found"),
            })
    }

    pub(crate) fn schemata(&self) -> Result<Vec<SchemaInfo>, Error> {
        self.stock
            .schemata()
            .map(|r| r.map_err(Error::from))
            .collect()
    }

    pub(crate) fn seal_of_secret(
        &mut self,
        secret: SecretSeal,
    ) -> Result<Option<GraphSeal>, Error> {
        self.stock.as_store().seal_of_secret(secret)
    }

    pub(crate) fn store_secret_seal(&mut self, seal: GraphSeal) -> Result<(), Error> {
        self.stock.store_secret_seal(seal).map_err(Error::from)
    }

    pub(crate) fn store_spv_proofs(&mut self, proofs: RetrievedSpvProofs) -> Result<(), Error> {
        if !proofs.is_empty() {
            self.stock.store_spv_proofs(proofs)?;
        }
        Ok(())
    }

    pub(crate) fn transfer(
        &self,
        contract_id: ContractId,
        outputs: impl AsRef<[OutputSeal]>,
        secret_seals: impl AsRef<[SecretSeal]>,
        witness_id: Option<RgbTxid>,
        spv_resolver: Option<&dyn ResolveSpvProof>,
    ) -> Result<(RgbTransfer, RetrievedSpvProofs), Error> {
        self.stock
            .transfer(
                contract_id,
                outputs,
                secret_seals,
                [],
                witness_id,
                spv_resolver,
            )
            .map_err(Error::from)
    }

    pub(crate) fn transfer_from_fascia(
        &self,
        contract_id: ContractId,
        outputs: impl AsRef<[OutputSeal]>,
        secret_seals: impl AsRef<[SecretSeal]>,
        opids: impl IntoIterator<Item = OpId>,
        fascia: &Fascia,
        spv_resolver: Option<&dyn ResolveSpvProof>,
    ) -> Result<(RgbTransfer, RetrievedSpvProofs), Error> {
        self.stock
            .transfer_from_fascia(
                contract_id,
                outputs,
                secret_seals,
                opids,
                fascia,
                spv_resolver,
            )
            .map_err(Error::from)
    }

    #[cfg(feature = "_online")]
    pub(crate) fn transfer_from_fascia_with_dag(
        &self,
        contract_id: ContractId,
        outputs: impl AsRef<[OutputSeal]>,
        secret_seals: impl AsRef<[SecretSeal]>,
        opids: impl IntoIterator<Item = OpId>,
        fascia: &Fascia,
    ) -> Result<(RgbTransfer, OpsDag, RetrievedSpvProofs), Error> {
        self.stock
            .transfer_from_fascia_with_dag(contract_id, outputs, secret_seals, opids, fascia, None)
            .map_err(Error::from)
    }

    pub(crate) fn transition_builder(
        &self,
        contract_id: ContractId,
        transition_name: impl Into<FieldName>,
    ) -> Result<TransitionBuilder, Error> {
        self.stock
            .transition_builder(contract_id, transition_name)
            .map_err(Error::from)
    }

    #[cfg(feature = "_online")]
    pub(crate) fn transition_builder_raw(
        &self,
        contract_id: ContractId,
        transition_type: TransitionType,
    ) -> Result<TransitionBuilder, Error> {
        self.stock
            .transition_builder_raw(contract_id, transition_type)
            .map_err(Error::from)
    }

    #[cfg(feature = "_online")]
    pub(crate) fn update_witnesses<R: ResolveWitness>(
        &mut self,
        resolver: &R,
        after_height: u32,
        force_witnesses: Vec<RgbTxid>,
    ) -> Result<UpdateRes, Error> {
        self.stock
            .update_witnesses(resolver, after_height, force_witnesses)
            .map_err(Error::from)
    }

    pub(crate) fn upsert_witness(
        &mut self,
        witness_id: RgbTxid,
        witness_ord: WitnessOrd,
    ) -> Result<(), Error> {
        self.stock.upsert_witness(witness_id, witness_ord)?;
        Ok(())
    }

    #[cfg(all(test, feature = "electrum"))]
    pub(crate) fn witness_spv_proof(&self, witness_id: RgbTxid) -> Result<Option<SpvProof>, Error> {
        self.stock.as_store().witness_spv_proof(witness_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "_online")]
    #[test]
    fn reject_list_parse() {
        let opid = "a8a9fb2e43bd4d1d02c6c4c5e1a8dd4ce9a4f9ea1e04a4dd4a1d4bdd4b0c4c4d";
        let rejected = Opout::from_str(&format!("{opid}/4000/0")).unwrap();
        let allowed = Opout::from_str(&format!("{opid}/4000/1")).unwrap();
        let list = format!("\n  {rejected} \n\t!{allowed}\t\n\n# a comment\n{opid}\n");
        let (reject_list, invalid) = RejectList::parse(&list);
        assert!(reject_list.rejects(&rejected));
        assert!(!reject_list.allows(&rejected));
        assert!(reject_list.allows(&allowed));
        assert!(!reject_list.rejects(&allowed));
        assert_eq!(invalid, 2);
    }

    #[derive(Debug, Deserialize)]
    struct MandatoryField {
        #[serde(deserialize_with = "from_str_or_number_mandatory")]
        val: u64,
    }

    #[derive(Debug, Deserialize)]
    struct OptionalField {
        #[serde(deserialize_with = "from_str_or_number_optional")]
        val: Option<u64>,
    }

    #[test]
    fn test_block_on_inside_tokio_runtime() {
        // calling block_on from within an active Tokio runtime takes the thread-spawn path
        // to avoid blocking the runtime thread
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { block_on(async { 42u32 }) });
        assert_eq!(result, 42);
    }

    #[test]
    fn test_from_str_or_number_mandatory() {
        // integer value
        let result: MandatoryField = serde_json::from_str(r#"{"val": 42}"#).unwrap();
        assert_eq!(result.val, 42);

        // float value (visit_f64 path)
        let result: MandatoryField = serde_json::from_str(r#"{"val": 42.0}"#).unwrap();
        assert_eq!(result.val, 42);

        // string value
        let result: MandatoryField = serde_json::from_str(r#"{"val": "99"}"#).unwrap();
        assert_eq!(result.val, 99);

        // null -> error
        let err = serde_json::from_str::<MandatoryField>(r#"{"val": null}"#).unwrap_err();
        assert!(
            err.to_string().contains("expected a number but got null"),
            "unexpected error message: {err}"
        );

        // invalid string -> parse error
        let err = serde_json::from_str::<MandatoryField>(r#"{"val": "abc"}"#).unwrap_err();
        assert!(
            err.to_string().contains("invalid value"),
            "unexpected error message: {err}"
        );

        // unexpected type (bool) -> error names the expected types via `expecting`
        let err = serde_json::from_str::<MandatoryField>(r#"{"val": true}"#).unwrap_err();
        assert!(
            err.to_string().contains("a string, a number, or null"),
            "unexpected error message: {err}"
        );
    }

    #[test]
    fn test_from_str_or_number_optional() {
        // integer value
        let result: OptionalField = serde_json::from_str(r#"{"val": 42}"#).unwrap();
        assert_eq!(result.val, Some(42));

        // float value (visit_f64 path)
        let result: OptionalField = serde_json::from_str(r#"{"val": 42.0}"#).unwrap();
        assert_eq!(result.val, Some(42));

        // string value
        let result: OptionalField = serde_json::from_str(r#"{"val": "99"}"#).unwrap();
        assert_eq!(result.val, Some(99));

        // null -> None (visit_unit path)
        let result: OptionalField = serde_json::from_str(r#"{"val": null}"#).unwrap();
        assert_eq!(result.val, None);

        // invalid string -> parse error
        let err = serde_json::from_str::<OptionalField>(r#"{"val": "abc"}"#).unwrap_err();
        assert!(
            err.to_string().contains("invalid value"),
            "unexpected error message: {err}"
        );

        // visit_none path: triggered by deserializer formats that signal absence via visit_none
        // rather than visit_unit (serde_json uses visit_unit for null, but other formats differ)
        struct VisitNoneDeserializer;
        impl<'de> Deserializer<'de> for VisitNoneDeserializer {
            type Error = serde::de::value::Error;
            fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
                visitor.visit_none()
            }
            serde::forward_to_deserialize_any! {
                bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string
                bytes byte_buf option unit unit_struct newtype_struct seq tuple
                tuple_struct map struct enum identifier ignored_any
            }
        }
        let result: Option<u64> = from_str_or_number_optional(VisitNoneDeserializer).unwrap();
        assert_eq!(result, None);
    }

    #[cfg(feature = "_online")]
    #[test]
    fn test_check_proxy_json_rpc_error() {
        // server returns HTTP 200 with result=null and a JSON-RPC error field
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"jsonrpc":"2.0","id":null,"result":null,"error":{"code":-32601,"message":"method not found"}}"#,
            )
            .create();
        let result = check_proxy(&server.url());
        assert_matches!(result, Err(Error::Proxy { details }) if details == "method not found");
        mock.assert();
    }

    #[cfg(feature = "_online")]
    #[test]
    fn test_check_proxy_invalid_body_error_details() {
        // server returns HTTP 200 with a body that is not a JSON-RPC response: the transport
        // error (body decoding) must be reported, not the generic connection message
        let mut server = mockito::Server::new();
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("not json")
            .create();
        let result = check_proxy(&server.url());
        let expected = format!("error decoding response body for url ({}/)", server.url());
        assert_matches!(result, Err(Error::Proxy { details }) if details == expected);
        mock.assert();
    }

    #[cfg(feature = "_online")]
    #[test]
    fn test_check_proxy_connection_error_details() {
        // bind then drop a listener to get a port that refuses connections: the transport error
        // (connection refused) must be reported, not the generic connection message
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let result = check_proxy(&format!("http://127.0.0.1:{port}"));
        let expected = format!("error sending request for url (http://127.0.0.1:{port}/)");
        assert_matches!(result, Err(Error::Proxy { details }) if details == expected);
    }

    // The None return from detect_indexer is only reachable when electrum is enabled but esplora
    // is not, and the URL is not a valid electrum server. With esplora enabled detection always
    // falls back to Some(IndexerConfig::Esplora) instead.
    #[cfg(all(feature = "electrum", not(feature = "esplora")))]
    #[test]
    fn test_detect_indexer_invalid_url_returns_none() {
        let result = detect_indexer("not_a_valid_url", None);
        assert!(result.is_none());
    }

    #[cfg(feature = "_online")]
    #[test]
    fn test_build_indexer_invalid_url() {
        let result = build_indexer(&IndexerConfig::Electrum {
            url: s!("not_a_valid_url"),
        });
        #[cfg(feature = "electrum")]
        let details = "not a valid electrum server";
        #[cfg(not(feature = "electrum"))]
        let details = "electrum support is not enabled (requires the `electrum` feature)";
        // Indexer is not Debug, so the Ok variant cannot be printed by assert_matches
        assert!(matches!(result, Err(Error::InvalidIndexer { details: m }) if m == details));
    }

    #[test]
    fn test_bitcoin_network_str_roundtrip() {
        // mainnet
        let network = BitcoinNetwork::Mainnet;
        let network_str = network.to_string();
        let network_from_str = BitcoinNetwork::from_str(&network_str).unwrap();
        assert_eq!(network, network_from_str);

        // testnet3
        let network = BitcoinNetwork::Testnet;
        let network_str = network.to_string();
        let network_from_str = BitcoinNetwork::from_str(&network_str).unwrap();
        assert_eq!(network, network_from_str);

        // testnet4
        let network = BitcoinNetwork::Testnet4;
        let network_str = network.to_string();
        let network_from_str = BitcoinNetwork::from_str(&network_str).unwrap();
        assert_eq!(network, network_from_str);

        // signet
        let network = BitcoinNetwork::Signet;
        let network_str = network.to_string();
        let network_from_str = BitcoinNetwork::from_str(&network_str).unwrap();
        assert_eq!(network, network_from_str);

        // regtest
        let network = BitcoinNetwork::Regtest;
        let network_str = network.to_string();
        let network_from_str = BitcoinNetwork::from_str(&network_str).unwrap();
        assert_eq!(network, network_from_str);

        // signet custom
        let network = BitcoinNetwork::SignetCustom;
        let network_str = network.to_string();
        let network_from_str = BitcoinNetwork::from_str(&network_str).unwrap();
        assert_eq!(network, network_from_str);

        // invalid network
        let network_str = "invalid";
        let result = BitcoinNetwork::from_str(network_str).unwrap_err();
        assert_matches!(result, Error::InvalidBitcoinNetwork { network } if network == "invalid");

        // signet- prefix with invalid hash
        let network_str = "signet-notahash";
        let result = BitcoinNetwork::from_str(network_str).unwrap_err();
        assert_matches!(result, Error::InvalidBitcoinNetwork { network } if network == "signet-notahash");
    }

    #[test]
    fn test_bitcoin_network_chain_net_roundtrip() {
        // mainnet
        let network = BitcoinNetwork::Mainnet;
        let chain_net = ChainNet::BitcoinMainnet;
        let network_from_chain_net = BitcoinNetwork::try_from(chain_net).unwrap();
        assert_eq!(network, network_from_chain_net);
        let chain_net_from_network = ChainNet::from(network);
        assert_eq!(chain_net, chain_net_from_network);

        // testnet3
        let network = BitcoinNetwork::Testnet;
        let chain_net = ChainNet::BitcoinTestnet3;
        let network_from_chain_net = BitcoinNetwork::try_from(chain_net).unwrap();
        assert_eq!(network, network_from_chain_net);
        let chain_net_from_network = ChainNet::from(network);
        assert_eq!(chain_net, chain_net_from_network);

        // testnet4
        let network = BitcoinNetwork::Testnet4;
        let chain_net = ChainNet::BitcoinTestnet4;
        let network_from_chain_net = BitcoinNetwork::try_from(chain_net).unwrap();
        assert_eq!(network, network_from_chain_net);
        let chain_net_from_network = ChainNet::from(network);
        assert_eq!(chain_net, chain_net_from_network);

        // signet
        let network = BitcoinNetwork::Signet;
        let chain_net = ChainNet::BitcoinSignet;
        let network_from_chain_net = BitcoinNetwork::try_from(chain_net).unwrap();
        assert_eq!(network, network_from_chain_net);
        let chain_net_from_network = ChainNet::from(network);
        assert_eq!(chain_net, chain_net_from_network);

        // regtest
        let network = BitcoinNetwork::Regtest;
        let chain_net = ChainNet::BitcoinRegtest;
        let network_from_chain_net = BitcoinNetwork::try_from(chain_net).unwrap();
        assert_eq!(network, network_from_chain_net);
        let chain_net_from_network = ChainNet::from(network);
        assert_eq!(chain_net, chain_net_from_network);

        // signet custom
        let network = BitcoinNetwork::SignetCustom;
        let chain_net = ChainNet::BitcoinSignetCustom;
        let network_from_chain_net = BitcoinNetwork::try_from(chain_net).unwrap();
        assert_eq!(network, network_from_chain_net);
        let chain_net_from_network = ChainNet::from(network);
        assert_eq!(chain_net, chain_net_from_network);

        // invalid chain net
        let chain_net = ChainNet::LiquidMainnet;
        let result = BitcoinNetwork::try_from(chain_net).unwrap_err();
        assert_matches!(result, Error::UnsupportedLayer1 { layer_1 } if layer_1 == "liquid");
    }

    #[test]
    fn test_bitcoin_network_try_from_rust_bitcoin_network() {
        // mainnet
        let network = BitcoinNetwork::Mainnet;
        let rust_bitcoin_network = bitcoin::Network::from(network);
        assert_eq!(rust_bitcoin_network, bitcoin::Network::Bitcoin);

        // testnet3
        let network = BitcoinNetwork::Testnet;
        let rust_bitcoin_network = bitcoin::Network::from(network);
        assert_eq!(rust_bitcoin_network, bitcoin::Network::Testnet);

        // testnet4
        let network = BitcoinNetwork::Testnet4;
        let rust_bitcoin_network = bitcoin::Network::from(network);
        assert_eq!(rust_bitcoin_network, bitcoin::Network::Testnet4);

        // signet
        let network = BitcoinNetwork::Signet;
        let rust_bitcoin_network = bitcoin::Network::from(network);
        assert_eq!(rust_bitcoin_network, bitcoin::Network::Signet);

        // regtest
        let network = BitcoinNetwork::Regtest;
        let rust_bitcoin_network = bitcoin::Network::from(network);
        assert_eq!(rust_bitcoin_network, bitcoin::Network::Regtest);

        // signet custom
        let network = BitcoinNetwork::SignetCustom;
        let rust_bitcoin_network = bitcoin::Network::from(network);
        assert_eq!(rust_bitcoin_network, bitcoin::Network::Signet);
    }

    #[cfg(feature = "electrum")]
    #[test]
    fn get_indexer_and_resolver_resolver_connection_refused() {
        // the indexer connects, then the resolver connection is refused
        let url = start_single_connection_electrum_mock();
        let result =
            get_indexer_and_resolver(&IndexerConfig::Electrum { url }, BitcoinNetwork::Regtest);
        // Indexer is not Debug, so the Ok variant cannot be printed by assert_matches
        assert!(matches!(result, Err(Error::InvalidIndexer { .. })));
    }
}

#[cfg(test)]
mod tests_keychain_layout {
    use super::*;
    use crate::keys::generate_keys;
    use crate::wallet::SinglesigKeys;
    use bdk_wallet::descriptor::ExtendedDescriptor;

    fn norm(desc: &str) -> String {
        desc.replace('h', "'")
    }

    // both sides under the standard BIP-86 account at coin type 0, colored on keychain 9 (the
    // vanilla one, 10, is SinglesigKeys::vanilla_keychain)
    fn single_account() -> KeychainLayoutOverrides {
        KeychainLayoutOverrides {
            colored_keychain: Some(9),
            colored_coin_type: Some(0),
            vanilla_coin_type: Some(0),
        }
    }

    #[test]
    fn default_layout_matches_standard_rgb_lib_layout() {
        let none = KeychainLayoutOverrides::default();
        let mainnet = KeychainLayout::resolve(&BitcoinNetwork::Mainnet, &none, None).unwrap();
        assert_eq!(
            mainnet,
            KeychainLayout {
                colored_coin_type: COIN_RGB_MAINNET,
                vanilla_coin_type: 0,
                colored_keychain: KEYCHAIN_RGB,
                vanilla_keychain: KEYCHAIN_BTC,
            }
        );
        let signet = KeychainLayout::resolve(&BitcoinNetwork::Signet, &none, Some(10)).unwrap();
        assert_eq!(signet.colored_coin_type, COIN_RGB_TESTNET);
        assert_eq!(signet.vanilla_coin_type, 1);
        assert_eq!(signet.vanilla_keychain, 10);
    }

    #[test]
    fn layout_rejects_shared_keychain() {
        // colored moved onto the vanilla coin type without a distinct keychain
        let mut overrides = KeychainLayoutOverrides {
            colored_coin_type: Some(0),
            ..Default::default()
        };
        let err = KeychainLayout::resolve(&BitcoinNetwork::Mainnet, &overrides, None).unwrap_err();
        assert!(matches!(err, Error::InvalidKeychainLayout { .. }));
        overrides.colored_keychain = Some(9);
        let err =
            KeychainLayout::resolve(&BitcoinNetwork::Mainnet, &overrides, Some(9)).unwrap_err();
        assert!(matches!(err, Error::InvalidKeychainLayout { .. }));
    }

    #[test]
    fn layout_rejects_non_hardenable_coin_type() {
        let overrides = KeychainLayoutOverrides {
            colored_coin_type: Some(0x8000_0000),
            colored_keychain: Some(9),
            ..Default::default()
        };
        let err =
            KeychainLayout::resolve(&BitcoinNetwork::Mainnet, &overrides, Some(10)).unwrap_err();
        assert!(matches!(err, Error::InvalidKeychainLayout { .. }));
    }

    #[test]
    fn single_account_layout_from_xpubs() {
        // hardware-wallet layout: both sides under the one exported BIP-86 account
        let keys = generate_keys(BitcoinNetwork::Mainnet, WitnessVersion::Taproot);
        let net = BitcoinNetwork::Mainnet;
        let (_, account_xpub, _) =
            get_account_data_at_coin_type(&net, &keys.mnemonic, 0, WitnessVersion::Taproot)
                .unwrap();
        let layout = KeychainLayout::resolve(&net, &single_account(), Some(10)).unwrap();
        let descs = get_descriptors_from_xpubs(
            &keys.master_fingerprint,
            &account_xpub,
            &account_xpub,
            &layout,
            WitnessVersion::Taproot,
        )
        .unwrap();
        let fp = &keys.master_fingerprint;
        assert!(
            norm(&descs.colored).contains(&format!("[{fp}/86'/0'/0'/9]")),
            "{}",
            descs.colored
        );
        assert!(
            norm(&descs.vanilla).contains(&format!("[{fp}/86'/0'/0'/10]")),
            "{}",
            descs.vanilla
        );
        assert_ne!(descs.colored, descs.vanilla);

        // the mnemonic path derives exactly the same descriptors (public parts)
        let with_mnemonic = get_descriptors(
            &net,
            &keys.mnemonic,
            &layout,
            &account_xpub,
            &account_xpub,
            WitnessVersion::Taproot,
        )
        .unwrap();
        assert!(norm(&with_mnemonic.colored).contains(&format!("[{fp}/86'/0'/0'/9]")));
        assert!(norm(&with_mnemonic.vanilla).contains(&format!("[{fp}/86'/0'/0'/10]")));
    }

    #[test]
    fn default_layout_descriptors_unchanged() {
        let keys = generate_keys(BitcoinNetwork::Regtest, WitnessVersion::Taproot);
        let net = BitcoinNetwork::Regtest;
        let nk = net.network_kind();
        let xpub_rgb = str_to_xpub(&keys.account_xpub_colored, &nk).unwrap();
        let xpub_btc = str_to_xpub(&keys.account_xpub_vanilla, &nk).unwrap();
        let layout =
            KeychainLayout::resolve(&net, &KeychainLayoutOverrides::default(), None).unwrap();
        let descs = get_descriptors_from_xpubs(
            &keys.master_fingerprint,
            &xpub_rgb,
            &xpub_btc,
            &layout,
            WitnessVersion::Taproot,
        )
        .unwrap();
        let fp = &keys.master_fingerprint;
        assert!(norm(&descs.colored).contains(&format!("[{fp}/86'/827167'/0'/0]")));
        assert!(norm(&descs.vanilla).contains(&format!("[{fp}/86'/1'/0'/0]")));
    }

    fn inconsistent_xpubs(res: Result<(WalletDescriptors, bool), Error>) -> bool {
        matches!(
            res,
            Err(Error::InvalidKeychainLayout { details })
                if details == "account xpubs are inconsistent with the configured coin types"
        )
    }

    #[test]
    fn account_xpubs_must_match_the_coin_types() {
        for net in [BitcoinNetwork::Mainnet, BitcoinNetwork::Signet] {
            let keys = generate_keys(net, WitnessVersion::Taproot);
            let (_, account_xpub, _) =
                get_account_data_at_coin_type(&net, &keys.mnemonic, 0, WitnessVersion::Taproot)
                    .unwrap();
            let account_xpub = account_xpub.to_string();
            for with_mnemonic in [false, true] {
                let default_keys = if with_mnemonic {
                    SinglesigKeys::from_keys(&keys, None)
                } else {
                    SinglesigKeys::from_keys_no_mnemonic(&keys, None)
                };

                // default layout: two coin types, two account xPubs
                default_keys.build_descriptors(&net).unwrap();

                // both sides under one account: one coin type, the same account xPub twice
                let mut single = default_keys.clone().with_keychain_layout(single_account());
                single.vanilla_keychain = Some(10);
                single.account_xpub_colored = account_xpub.clone();
                single.account_xpub_vanilla = account_xpub.clone();
                single.build_descriptors(&net).unwrap();

                // same xPub for both sides but the colored coin type left to its default: the
                // colored descriptor would claim 827166'/827167' for a key at 0'
                let mut forgot_colored_coin = single.clone();
                forgot_colored_coin.keychain_layout.colored_coin_type = None;
                assert!(inconsistent_xpubs(
                    forgot_colored_coin.build_descriptors(&net)
                ));

                // off mainnet the vanilla default is 1', so the vanilla coin type is needed too
                let mut forgot_vanilla_coin = single.clone();
                forgot_vanilla_coin.keychain_layout.vanilla_coin_type = None;
                let res = forgot_vanilla_coin.build_descriptors(&net);
                if net == BitcoinNetwork::Mainnet {
                    res.unwrap();
                } else {
                    assert!(inconsistent_xpubs(res));
                }

                // default coin types with the same xPub for both sides
                let mut same_xpub = default_keys.clone();
                same_xpub.account_xpub_colored = same_xpub.account_xpub_vanilla.clone();
                assert!(inconsistent_xpubs(same_xpub.build_descriptors(&net)));

                // one coin type, but the colored xPub is still the RGB account's
                let mut other_xpub = single.clone();
                other_xpub.account_xpub_colored = keys.account_xpub_colored.clone();
                assert!(inconsistent_xpubs(other_xpub.build_descriptors(&net)));
            }
        }
    }

    #[test]
    fn account_xpubs_compared_as_keys() {
        let net = BitcoinNetwork::Signet;
        let keys = generate_keys(net, WitnessVersion::Taproot);
        let (_, account_xpub, _) =
            get_account_data_at_coin_type(&net, &keys.mnemonic, 0, WitnessVersion::Taproot)
                .unwrap();
        let mut single = SinglesigKeys::from_keys_no_mnemonic(&keys, Some(10))
            .with_keychain_layout(single_account());
        single.account_xpub_colored = account_xpub.to_string();
        single.account_xpub_vanilla = account_xpub.to_string();
        let (descs, _) = single.build_descriptors(&net).unwrap();

        // the same key encoded with mainnet version bytes (xpub) on one side
        let mut mainnet_encoded = account_xpub;
        mainnet_encoded.network = NetworkKind::Main;
        assert!(mainnet_encoded.to_string().starts_with("xpub"));
        let mut mixed = single.clone();
        mixed.account_xpub_colored = mainnet_encoded.to_string();
        assert_eq!(mixed.build_descriptors(&net).unwrap().0, descs);

        // the same key with other metadata, as a host that only has the public key and chain
        // code would build it: accepted, and it derives the same scripts
        let mut relabelled = account_xpub;
        relabelled.depth = 0;
        relabelled.parent_fingerprint = Fingerprint::default();
        relabelled.child_number = ChildNumber::from_normal_idx(0).unwrap();
        let mut bare = single.clone();
        bare.account_xpub_colored = relabelled.to_string();
        let (bare_descs, _) = bare.build_descriptors(&net).unwrap();
        let script_at = |desc: &str, index: u32| {
            ExtendedDescriptor::from_str(desc)
                .unwrap()
                .at_derivation_index(index)
                .unwrap()
                .script_pubkey()
        };
        for index in [0, 1, 100] {
            assert_eq!(
                script_at(&bare_descs.colored, index),
                script_at(&descs.colored, index)
            );
        }

        // and it is still the same key when the coin types differ
        bare.keychain_layout = KeychainLayoutOverrides::default();
        bare.account_xpub_vanilla = account_xpub.to_string();
        assert!(inconsistent_xpubs(bare.build_descriptors(&net)));
    }

    // Serialized shapes written before SinglesigKeys grouped the overrides into
    // KeychainLayoutOverrides, for the key of "abandon ... about" on signet. The account xPub at
    // m/86'/0'/0' serves both sides of the single-account layout; the other two are the default
    // layout's accounts (m/86'/827167'/0' and m/86'/1'/0').
    const TPUB_COIN_0: &str = "tpubDC3pD7UZXnsgh3EBjbtBQiB1FnLask7UHBSunZ1DPK4dCFFZoFRkgxHB8gt42FvLzx1DpxfHWxAsYaY6b643RVcGjDxXxns7wKKYnnfEcbB";
    const TPUB_COLORED: &str = "tpubDCtpoJs6YJcjLnr9gq6jYriYNMuWEu8mSDvEQU5st3ZkJbFqqzwpHUiPvxqD2366ciFAfpehk1k2d7Tyk7AJEr8uZva7KfnX4RpsiVSoEcZ";
    const TPUB_VANILLA: &str = "tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX";

    #[cfg(not(feature = "camel_case"))]
    const KEYS_JSON: &str = r#"{"account_xpub_vanilla":"TPUB","account_xpub_colored":"TPUB","vanilla_keychain":10,"colored_keychain":9,"colored_coin_type":0,"vanilla_coin_type":0,"master_fingerprint":"73c5da0a","mnemonic":null,"witness_version":"Taproot"}"#;
    #[cfg(feature = "camel_case")]
    const KEYS_JSON: &str = r#"{"accountXpubVanilla":"TPUB","accountXpubColored":"TPUB","vanillaKeychain":10,"coloredKeychain":9,"coloredCoinType":0,"vanillaCoinType":0,"masterFingerprint":"73c5da0a","mnemonic":null,"witnessVersion":"Taproot"}"#;
    // the shape before the layout existed at all
    #[cfg(not(feature = "camel_case"))]
    const KEYS_JSON_NO_LAYOUT: &str = r#"{"account_xpub_vanilla":"TPUB_V","account_xpub_colored":"TPUB_C","vanilla_keychain":null,"master_fingerprint":"73c5da0a","mnemonic":null,"witness_version":"Taproot"}"#;
    #[cfg(feature = "camel_case")]
    const KEYS_JSON_NO_LAYOUT: &str = r#"{"accountXpubVanilla":"TPUB_V","accountXpubColored":"TPUB_C","vanillaKeychain":null,"masterFingerprint":"73c5da0a","mnemonic":null,"witnessVersion":"Taproot"}"#;

    #[test]
    fn singlesig_keys_json_keeps_its_flat_shape() {
        let json = KEYS_JSON.replace("TPUB", TPUB_COIN_0);
        let keys: SinglesigKeys = serde_json::from_str(&json).unwrap();
        assert_eq!(keys.keychain_layout, single_account());
        assert_eq!(keys.vanilla_keychain, Some(10));
        keys.build_descriptors(&BitcoinNetwork::Signet).unwrap();
        // written back field for field, byte for byte
        assert_eq!(serde_json::to_string(&keys).unwrap(), json);

        let json = KEYS_JSON_NO_LAYOUT
            .replace("TPUB_V", TPUB_VANILLA)
            .replace("TPUB_C", TPUB_COLORED);
        let keys: SinglesigKeys = serde_json::from_str(&json).unwrap();
        assert_eq!(keys.keychain_layout, KeychainLayoutOverrides::default());
        keys.build_descriptors(&BitcoinNetwork::Signet).unwrap();

        // numeric strings are still accepted inside the flattened struct, and a bad value still
        // fails the whole deserialization instead of falling back to the default layout
        let json = KEYS_JSON.replace("TPUB", TPUB_COIN_0);
        let as_strings = json.replace(":9,", r#":"9","#).replace(":0,", r#":"0","#);
        let keys: SinglesigKeys = serde_json::from_str(&as_strings).unwrap();
        assert_eq!(keys.keychain_layout, single_account());
        let bad = json.replace(":9,", r#":"nine","#);
        assert!(serde_json::from_str::<SinglesigKeys>(&bad).is_err());
    }

    const MANIFEST_JSON: &str = r#"{
  "version": 1,
  "bitcoin_network": "Signet",
  "database_type": "Sqlite",
  "max_allocations_per_utxo": 1,
  "supported_schemas": [
    "Nia"
  ],
  "reuse_addresses": false,
  "account_xpub_vanilla": "TPUB",
  "account_xpub_colored": "TPUB",
  "vanilla_keychain": 10,
  "colored_keychain": 9,
  "colored_coin_type": 0,
  "vanilla_coin_type": 0,
  "master_fingerprint": "73c5da0a",
  "witness_version": "Taproot"
}"#;
    // the four settings apart, so a mix-up between them shows
    const MANIFEST_JSON_DISTINCT: &str = r#"{
  "version": 1,
  "bitcoin_network": "Signet",
  "database_type": "Sqlite",
  "max_allocations_per_utxo": 1,
  "supported_schemas": [
    "Nia"
  ],
  "reuse_addresses": false,
  "account_xpub_vanilla": "TPUB_V",
  "account_xpub_colored": "TPUB_C",
  "vanilla_keychain": 10,
  "colored_keychain": 9,
  "colored_coin_type": 0,
  "vanilla_coin_type": 5,
  "master_fingerprint": "73c5da0a",
  "witness_version": "Taproot"
}"#;
    const MANIFEST_JSON_DEFAULT: &str = r#"{
  "version": 1,
  "bitcoin_network": "Signet",
  "database_type": "Sqlite",
  "max_allocations_per_utxo": 1,
  "supported_schemas": [
    "Nia"
  ],
  "reuse_addresses": false,
  "account_xpub_vanilla": "TPUB_V",
  "account_xpub_colored": "TPUB_C",
  "vanilla_keychain": 0,
  "master_fingerprint": "73c5da0a",
  "witness_version": "Taproot"
}"#;

    #[test]
    fn manifest_keeps_its_shape() {
        use crate::wallet::WalletManifest;

        let json = MANIFEST_JSON.replace("TPUB", TPUB_COIN_0);
        let manifest: WalletManifest = serde_json::from_str(&json).unwrap();
        let (wallet_data, keys) = manifest.into_parts(s!("data_dir"), None);
        assert_eq!(keys.keychain_layout, single_account());
        assert_eq!(keys.vanilla_keychain, Some(10));
        keys.build_descriptors(&wallet_data.bitcoin_network)
            .unwrap();
        let rewritten = WalletManifest::new(&wallet_data, &keys);
        // a manifest of the previous base (UTEXO's reuse_addresses) reads, and is written back in
        // beta.8's shape, the layout keys as they were
        assert_eq!(
            serde_json::to_string_pretty(&rewritten).unwrap(),
            json.replace(
                "\"reuse_addresses\": false",
                "\"allow_reckless_schemas\": false"
            )
        );

        let json = MANIFEST_JSON_DISTINCT
            .replace("TPUB_V", TPUB_VANILLA)
            .replace("TPUB_C", TPUB_COIN_0);
        let manifest: WalletManifest = serde_json::from_str(&json).unwrap();
        let (wallet_data, keys) = manifest.into_parts(s!("data_dir"), None);
        assert_eq!(
            keys.keychain_layout,
            KeychainLayoutOverrides {
                colored_keychain: Some(9),
                colored_coin_type: Some(0),
                vanilla_coin_type: Some(5),
            }
        );
        let rewritten = WalletManifest::new(&wallet_data, &keys);
        // a manifest of the previous base (UTEXO's reuse_addresses) reads, and is written back in
        // beta.8's shape, the layout keys as they were
        assert_eq!(
            serde_json::to_string_pretty(&rewritten).unwrap(),
            json.replace(
                "\"reuse_addresses\": false",
                "\"allow_reckless_schemas\": false"
            )
        );

        let json = MANIFEST_JSON_DEFAULT
            .replace("TPUB_V", TPUB_VANILLA)
            .replace("TPUB_C", TPUB_COLORED);
        let manifest: WalletManifest = serde_json::from_str(&json).unwrap();
        let (wallet_data, keys) = manifest.into_parts(s!("data_dir"), None);
        assert_eq!(keys.keychain_layout, KeychainLayoutOverrides::default());
        keys.build_descriptors(&wallet_data.bitcoin_network)
            .unwrap();
        let rewritten = WalletManifest::new(&wallet_data, &keys);
        // a manifest of the previous base (UTEXO's reuse_addresses) reads, and is written back in
        // beta.8's shape, the layout keys as they were
        assert_eq!(
            serde_json::to_string_pretty(&rewritten).unwrap(),
            json.replace(
                "\"reuse_addresses\": false",
                "\"allow_reckless_schemas\": false"
            )
        );
    }
}
