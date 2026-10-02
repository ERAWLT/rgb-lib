//! ERA fork: `Wallet::get_colored_address`, and which sync sees a TX that pays it from outside
//! the wallet (on the scripted chain: no regtest services).

use super::*;

#[cfg(any(feature = "electrum", feature = "esplora"))]
use super::scripted_chain::*;

/// The colored (BDK external) index `address` derives at in `wallet`, if any.
fn colored_index(wallet: &Wallet, address: &str) -> Option<u32> {
    let script = BdkAddress::from_str(address)
        .unwrap()
        .assume_checked()
        .script_pubkey();
    match wallet.bdk_wallet().derivation_of_spk(script) {
        Some((KeychainKind::External, index)) => Some(index),
        _ => None,
    }
}

#[test]
#[parallel]
fn success() {
    let mut party = offline_party!(get_test_wallet(false, None));
    assert!(party.db_backup_info_opt().is_none());

    let first = party.wallet.get_colored_address().unwrap();
    // a mutation, as get_address is: the next backup is owed
    assert!(
        party
            .db_backup_info()
            .last_operation_timestamp
            .parse::<i128>()
            .unwrap()
            > 0
    );
    let second = party.wallet.get_colored_address().unwrap();
    assert_eq!(colored_index(&party.wallet, &first), Some(0));
    assert_eq!(colored_index(&party.wallet, &second), Some(1));
    // the vanilla keychain is another one
    let vanilla = party.wallet.get_address().unwrap();
    assert_eq!(colored_index(&party.wallet, &vanilla), None);

    // the index is persisted: a reload hands out the next one
    let data_dir = party.wallet.get_wallet_data().data_dir;
    let fingerprint = party.wallet.get_keys().master_fingerprint;
    drop(party);
    let mut loaded = Wallet::load(&data_dir, &fingerprint, None).unwrap();
    let third = loaded.get_colored_address().unwrap();
    assert_eq!(colored_index(&loaded, &third), Some(2));
}

/// Whether `wallet` lists an RGB-ready UTXO of `txid`, syncing first (fast) when `sync`.
#[cfg(any(feature = "electrum", feature = "esplora"))]
fn lists_envelope(wallet: &mut Wallet, online: Online, txid: &str, sync: bool) -> bool {
    wallet
        .list_unspents(Some(online), false, !sync)
        .unwrap()
        .into_iter()
        .any(|u| u.utxo.outpoint.txid == txid && u.utxo.colorable && u.utxo.exists)
}

#[cfg(any(feature = "electrum", feature = "esplora"))]
fn blind_receive(wallet: &mut Wallet, chain: &ScriptedChain) -> Result<ReceiveData, Error> {
    wallet.blind_receive(
        None,
        Assignment::Any,
        default_rcv_expiration(),
        vec![chain.proxy_endpoint()],
        MIN_CONFIRMATIONS,
    )
}

#[cfg(any(feature = "electrum", feature = "esplora"))]
#[test]
#[parallel]
fn a_full_sync_sees_a_payment_from_outside() {
    let chain = ScriptedChain::start();
    let mut wallet = get_test_wallet(false, None);
    let online = wallet.go_online(online_options(&chain)).unwrap();
    // no colored UTXO yet: nothing to receive on
    assert_matches!(
        blind_receive(&mut wallet, &chain),
        Err(Error::InsufficientAllocationSlots)
    );

    let address = wallet.get_colored_address().unwrap();
    let txid = chain.fund(&address, UTXO_SATS as u64).to_string();
    chain.mine(1);
    assert!(chain.is_confirmed(&txid));

    // The fast sync of every other call does not ask about the address. If this starts to hold,
    // rgb-lib's fast sync has learnt to and the host's full sync after a payment can go.
    assert!(!lists_envelope(&mut wallet, online, &txid, true));
    assert_matches!(
        blind_receive(&mut wallet, &chain),
        Err(Error::InsufficientAllocationSlots)
    );

    wallet
        .sync(
            online,
            SyncOptions {
                keychain: SyncKeychain::Colored,
                strategy: SyncStrategy::FullSync,
            },
        )
        .unwrap();
    assert!(lists_envelope(&mut wallet, online, &txid, false));
    // and an invoice can now be issued on it
    let receive = blind_receive(&mut wallet, &chain).unwrap();
    assert!(!receive.invoice.is_empty());
    chain.assert_all_matched();
}

/// A payment the indexer has in its mempool only is seen too: the envelope is usable as soon as
/// the TX is known, as `create_utxos_end` makes its own.
#[cfg(any(feature = "electrum", feature = "esplora"))]
#[test]
#[parallel]
fn a_full_sync_sees_an_unconfirmed_payment() {
    let chain = ScriptedChain::start();
    let mut wallet = get_test_wallet(false, None);
    let online = wallet.go_online(online_options(&chain)).unwrap();
    let address = wallet.get_colored_address().unwrap();
    let txid = chain.fund(&address, UTXO_SATS as u64).to_string();
    assert!(chain.knows(&txid) && !chain.is_confirmed(&txid));

    wallet
        .sync(
            online,
            SyncOptions {
                keychain: SyncKeychain::Colored,
                strategy: SyncStrategy::FullSync,
            },
        )
        .unwrap();
    assert!(lists_envelope(&mut wallet, online, &txid, false));
    chain.assert_all_matched();
}

/// `go_online` scans the colored keychain in full: a payment made while the wallet was closed is
/// there once it is back online, with no sync of its own.
#[cfg(any(feature = "electrum", feature = "esplora"))]
#[test]
#[parallel]
fn go_online_sees_a_payment_from_outside() {
    let chain = ScriptedChain::start();
    let mut wallet = get_test_wallet(false, None);
    let address = wallet.get_colored_address().unwrap();
    let txid = chain.fund(&address, UTXO_SATS as u64).to_string();
    chain.mine(1);

    let online = wallet.go_online(online_options(&chain)).unwrap();
    assert!(lists_envelope(&mut wallet, online, &txid, false));
    chain.assert_all_matched();
}
