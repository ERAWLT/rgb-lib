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
    wallet.sync_colored_payments(online, vec![]).unwrap();
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

    wallet.sync_colored_payments(online, vec![]).unwrap();
    assert!(lists_envelope(&mut wallet, online, &txid, false));
    let colored = wallet.get_btc_balance(None, true).unwrap().colored;
    assert_eq!((colored.settled, colored.future), (0, amount));

    // the address is used now; the TX is followed through its colored output
    chain.mine(1);
    wallet.sync_colored_payments(online, vec![]).unwrap();
    let colored = wallet.get_btc_balance(None, true).unwrap().colored;
    assert_eq!((colored.settled, colored.future), (amount, amount));
    // and once confirmed it is asked about no more
    assert_eq!(fast_sync_lookups(&mut wallet, online, &chain), 0);
    chain.assert_all_matched();
}

/// The fast sync asks about the addresses `get_colored_address` handed out, and about no other
/// unused colored script: here one revealed by rgb-lib itself, as a send's change is.
#[cfg(any(feature = "electrum", feature = "esplora"))]
#[test]
#[parallel]
fn the_fast_sync_asks_about_the_addresses_handed_out_only() {
    let chain = ScriptedChain::start();
    let mut wallet = get_test_wallet(false, None);
    let online = wallet.go_online(online_options(&chain)).unwrap();
    {
        let (bdk_wallet, bdk_db) = wallet.bdk_wallet_db_mut();
        bdk_wallet.reveal_next_address(KeychainKind::External);
        bdk_wallet.persist(bdk_db).unwrap();
    }
    assert_eq!(fast_sync_lookups(&mut wallet, online, &chain), 0);

    let address = wallet.get_colored_address().unwrap();
    assert_eq!(fast_sync_lookups(&mut wallet, online, &chain), 1);
    let txid = chain.fund(&address, UTXO_SATS as u64).to_string();
    chain.mine(1);
    assert!(lists_envelope(&mut wallet, online, &txid, true));
    // paid and confirmed: asked about no more
    assert_eq!(fast_sync_lookups(&mut wallet, online, &chain), 0);
    chain.assert_all_matched();
}

/// The watched addresses live in the process. After a restart a payment that comes later is
/// seen once the host names the address, and followed from then on.
#[cfg(any(feature = "electrum", feature = "esplora"))]
#[test]
#[parallel]
fn a_payment_awaited_across_a_restart_is_seen_once_the_host_names_it() {
    let chain = ScriptedChain::start();
    let mut wallet = get_test_wallet(false, None);
    let address = wallet.get_colored_address().unwrap();
    let data_dir = wallet.get_wallet_data().data_dir;
    let fingerprint = wallet.get_keys().master_fingerprint;
    drop(wallet);

    let mut wallet = Wallet::load(&data_dir, &fingerprint, None).unwrap();
    let online = wallet.go_online(online_options(&chain)).unwrap();
    let txid = chain.fund(&address, UTXO_SATS as u64).to_string();
    assert!(!lists_envelope(&mut wallet, online, &txid, true));

    wallet
        .sync_colored_payments(online, vec![address.clone()])
        .unwrap();
    assert!(lists_envelope(&mut wallet, online, &txid, false));
    let colored = wallet.get_btc_balance(None, true).unwrap().colored;
    assert_eq!(colored.settled, 0);
    // watched now: the fast sync follows it to its confirmation
    chain.mine(1);
    wallet.get_btc_balance(Some(online), false).unwrap();
    let colored = wallet.get_btc_balance(None, true).unwrap().colored;
    assert_eq!(colored.settled, UTXO_SATS as u64);
    chain.assert_all_matched();
}

/// An address that is not one of this wallet's colored ones is refused, and nothing is synced.
#[cfg(any(feature = "electrum", feature = "esplora"))]
#[test]
#[parallel]
fn the_colored_payment_sync_refuses_another_address() {
    let chain = ScriptedChain::start();
    let mut wallet = get_test_wallet(false, None);
    let online = wallet.go_online(online_options(&chain)).unwrap();
    let vanilla = wallet.get_address().unwrap();
    let other = get_test_wallet(false, None).get_colored_address().unwrap();
    for address in [vanilla, other, s!("not an address")] {
        chain.clear_requests();
        let result = wallet.sync_colored_payments(online, vec![address]);
        assert_matches!(result, Err(Error::InvalidAddress { .. }));
        assert!(chain.requests().is_empty());
    }
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
