//! ERA fork: `Wallet::get_colored_address`, and the colored fast sync that sees a TX paying such
//! an address from outside the wallet (on the scripted chain: no regtest services).

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

/// The colored scripts a colored fast sync asks the indexer about.
#[cfg(any(feature = "electrum", feature = "esplora"))]
fn fast_sync_lookups(wallet: &mut Wallet, online: Online, chain: &ScriptedChain) -> usize {
    chain.clear_requests();
    wallet.sync_colored_payments(online, false).unwrap();
    chain
        .requests()
        .iter()
        .filter(|r| r.starts_with("GET /scripthash/"))
        .count()
}

/// The fast sync every syncing call makes sees a payment to an address handed out, and an invoice
/// can then be issued on the envelope.
#[cfg(any(feature = "electrum", feature = "esplora"))]
#[test]
#[parallel]
fn the_fast_sync_sees_a_payment_from_outside() {
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

    // list_unspents with a sync is a fast sync, as in every *_begin
    assert!(lists_envelope(&mut wallet, online, &txid, true));
    let receive = blind_receive(&mut wallet, &chain).unwrap();
    assert!(!receive.invoice.is_empty());
    chain.assert_all_matched();
}

/// A payment the indexer has in its mempool only is seen too, the envelope usable as soon as the
/// TX is known (as `create_utxos_end` makes its own), and the fast sync follows the TX until it
/// confirms although it spends nothing of the wallet.
#[cfg(any(feature = "electrum", feature = "esplora"))]
#[test]
#[parallel]
fn the_fast_sync_follows_a_payment_from_outside_to_its_confirmation() {
    let chain = ScriptedChain::start();
    let mut wallet = get_test_wallet(false, None);
    let online = wallet.go_online(online_options(&chain)).unwrap();
    let address = wallet.get_colored_address().unwrap();
    let amount = UTXO_SATS as u64;
    let txid = chain.fund(&address, amount).to_string();
    assert!(chain.knows(&txid) && !chain.is_confirmed(&txid));

    wallet.sync_colored_payments(online, false).unwrap();
    assert!(lists_envelope(&mut wallet, online, &txid, false));
    let colored = wallet.get_btc_balance(None, true).unwrap().colored;
    assert_eq!((colored.settled, colored.future), (0, amount));

    // the address is used now; the TX is followed through its colored output
    chain.mine(1);
    wallet.sync_colored_payments(online, false).unwrap();
    let colored = wallet.get_btc_balance(None, true).unwrap().colored;
    assert_eq!((colored.settled, colored.future), (amount, amount));
    // and once confirmed it is asked about no more
    assert_eq!(fast_sync_lookups(&mut wallet, online, &chain), 0);
    chain.assert_all_matched();
}

/// The fast sync asks about the `COLORED_SYNC_RECENT_UNUSED` most recently revealed and unused
/// colored scripts, no more: a payment to an address with as many unused ones revealed after it
/// is left to `sync_colored_payments(.., true)` and to `go_online`'s full scan.
#[cfg(any(feature = "electrum", feature = "esplora"))]
#[test]
#[parallel]
fn the_fast_sync_asks_about_the_most_recent_unused_addresses_only() {
    let chain = ScriptedChain::start();
    let mut wallet = get_test_wallet(false, None);
    let online = wallet.go_online(online_options(&chain)).unwrap();
    assert_eq!(fast_sync_lookups(&mut wallet, online, &chain), 0);

    let old = wallet.get_colored_address().unwrap();
    for _ in 0..COLORED_SYNC_RECENT_UNUSED {
        wallet.get_colored_address().unwrap();
    }
    assert_eq!(
        fast_sync_lookups(&mut wallet, online, &chain),
        COLORED_SYNC_RECENT_UNUSED
    );
    let txid = chain.fund(&old, UTXO_SATS as u64).to_string();
    chain.mine(1);
    assert!(!lists_envelope(&mut wallet, online, &txid, true));

    wallet.sync_colored_payments(online, true).unwrap();
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
