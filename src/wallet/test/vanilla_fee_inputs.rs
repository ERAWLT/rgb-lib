//! ERA fork (spike, simple mode): a send whose colored UTXOs cannot pay its bitcoin part (the fee
//! and the witness output) adds confirmed vanilla UTXOs to its inputs when the host allows it
//! (`Wallet::set_vanilla_fee_inputs`), so a user with bitcoin in the wallet sends tokens without
//! preparing envelopes first and waiting for them to confirm.
//!
//! What is pinned here, on the scripted chain:
//! - off (the default) nothing changes: the send fails as upstream's does;
//! - on, the PSBT spends the asset's colored UTXO and one vanilla UTXO; a new 1000-sat colored
//!   output takes the RGB change and the rest of the bitcoin drains back to the vanilla keychain;
//! - the vanilla input stays reserved until the send is broadcast: `create_utxos`, `send_btc` and
//!   the vanilla balance leave it out (the reservation is kept as a colored one by the send);
//! - an unconfirmed vanilla UTXO is never taken;
//! - free colored UTXOs are spent first: with envelopes enough, no vanilla input;
//! - a vanilla-funded send whose broadcast answer is lost is completed by `go_online` (CC-99) as
//!   any send is.

// (explicit: the shared test helpers have their own, for the electrum harness)
use super::scripted_chain::*;
use super::scripted_chain::{psbt_txid, witness_recipient};
use super::*;

/// The single envelope of a thin issuer: below a witness send's 1000-sat output, so its own sats
/// can never pay the bitcoin part.
const THIN_SATS: u32 = 600;

/// An issuer whose only colored UTXO is a `THIN_SATS` envelope holding the asset, with the rest of
/// its funding back on the vanilla keychain as one UTXO (`create_utxos` spends every vanilla UTXO),
/// confirmed if `mine_envelope`.
fn thin_issuer(chain: &ScriptedChain, mine_envelope: bool) -> Issuer {
    let mut wallet = get_test_wallet(true, None);
    let online = wallet.go_online(online_options(chain)).unwrap();
    chain.fund(&wallet.get_address().unwrap(), FUNDING);
    chain.mine(1);
    wallet
        .create_utxos(online, false, Some(1), Some(THIN_SATS), FEE_RATE, false)
        .unwrap();
    if mine_envelope {
        chain.mine(1);
    }
    // sync both keychains, as a host refreshing its balance does
    wallet.get_btc_balance(Some(online), false).unwrap();
    let asset_id = issue(&wallet, vec![AMOUNT]);
    Issuer {
        wallet,
        online,
        asset_id,
    }
}

fn try_send(
    chain: &ScriptedChain,
    party: &mut Issuer,
    donation: bool,
    dry_run: bool,
) -> Result<SendBeginResult, Error> {
    let recipient = witness_recipient(chain, AMOUNT_SMALL);
    party.wallet.send_begin(
        party.online,
        HashMap::from([(party.asset_id.clone(), vec![recipient])]),
        donation,
        FEE_RATE,
        MIN_CONFIRMATIONS,
        (now().unix_timestamp() + DURATION_SEND_TRANSFER as i64) as u64,
        Some(chain.proxy_endpoint()),
        dry_run,
    )
}

/// The keychain BDK derives `spk` on, `None` for a script of someone else.
fn keychain_of(wallet: &Wallet, spk: &ScriptBuf) -> Option<KeychainKind> {
    wallet
        .bdk_wallet()
        .derivation_of_spk(spk.clone())
        .map(|(k, _)| k)
}

/// The keychains of the inputs, sorted (the fork collects the inputs in a hash set, so their order
/// in the TX is arbitrary): colored (`External`) first.
fn input_keychains(wallet: &Wallet, psbt: &Psbt) -> Vec<KeychainKind> {
    let mut keychains: Vec<KeychainKind> = psbt
        .unsigned_tx
        .input
        .iter()
        .map(|i| {
            wallet
                .bdk_wallet()
                .get_utxo(i.previous_output)
                .expect("an input of the wallet")
                .keychain
        })
        .collect();
    keychains.sort_by_key(|k| *k == KeychainKind::Internal);
    keychains
}

fn vanilla_unspents(wallet: &Wallet) -> Vec<BdkOutPoint> {
    wallet
        .bdk_wallet()
        .list_unspent()
        .filter(|u| u.keychain == KeychainKind::Internal)
        .map(|u| u.outpoint)
        .collect()
}

#[test]
#[parallel]
fn off_by_default_a_send_without_free_envelopes_fails_as_upstream() {
    let chain = ScriptedChain::start();
    let mut party = thin_issuer(&chain, true);
    let result = try_send(&chain, &mut party, false, false);
    assert_matches!(result, Err(Error::InsufficientAllocationSlots));
}

#[test]
#[parallel]
fn a_send_without_free_envelopes_pays_its_bitcoin_part_from_a_vanilla_utxo() {
    let chain = ScriptedChain::start();
    let mut party = thin_issuer(&chain, true);
    let vanilla_before = vanilla_unspents(&party.wallet);
    assert_eq!(vanilla_before.len(), 1);
    let vanilla_sats = party
        .wallet
        .get_btc_balance(None, true)
        .unwrap()
        .vanilla
        .spendable;
    party.wallet.set_vanilla_fee_inputs(true);

    let (begin, signed, recipient) = begin_send(&chain, &mut party, AMOUNT_SMALL, false);
    let psbt = Psbt::from_str(&begin.psbt).unwrap();

    // inputs: the envelope holding the asset and the vanilla UTXO
    assert_eq!(
        input_keychains(&party.wallet, &psbt),
        vec![KeychainKind::External, KeychainKind::Internal]
    );
    assert!(
        psbt.unsigned_tx
            .input
            .iter()
            .any(|i| i.previous_output == vanilla_before[0])
    );
    // outputs: the commitment, the recipient's witness output, the colored output taking the RGB
    // change, the vanilla change
    let outputs = &psbt.unsigned_tx.output;
    assert_eq!(outputs.len(), 4);
    assert!(outputs[0].script_pubkey.is_op_return());
    assert_eq!(outputs[1].value.to_sat(), WITNESS_SATS);
    assert_eq!(keychain_of(&party.wallet, &outputs[1].script_pubkey), None);
    assert_eq!(
        keychain_of(&party.wallet, &outputs[2].script_pubkey),
        Some(KeychainKind::External)
    );
    assert_eq!(outputs[2].value.to_sat(), 1000);
    assert_eq!(
        keychain_of(&party.wallet, &outputs[3].script_pubkey),
        Some(KeychainKind::Internal)
    );
    // the fee is the rate's, not the vanilla UTXO: everything but the fee and the witness output
    // comes back
    let fee =
        THIN_SATS as u64 + vanilla_sats - outputs.iter().map(|o| o.value.to_sat()).sum::<u64>();
    assert!(fee < 1000, "fee {fee}");
    assert_eq!(
        outputs[3].value.to_sat(),
        vanilla_sats + THIN_SATS as u64 - WITNESS_SATS - 1000 - fee
    );

    // until the TX is broadcast the vanilla UTXO is the send's: the vanilla operations and the
    // vanilla balance leave it out
    assert_eq!(
        party
            .wallet
            .get_btc_balance(None, true)
            .unwrap()
            .vanilla
            .spendable,
        0
    );
    let other = get_test_wallet(false, None).get_address().unwrap();
    let send_btc = party
        .wallet
        .send_btc_begin(party.online, other, 10_000, FEE_RATE, true, true);
    assert_matches!(send_btc, Err(Error::InsufficientBitcoins { .. }));
    let create_utxos = party.wallet.create_utxos_begin(
        party.online,
        false,
        Some(1),
        Some(1000),
        FEE_RATE,
        true,
        true,
    );
    assert_matches!(create_utxos, Err(Error::InsufficientBitcoins { .. }));

    // the send goes through as any send: ACK, broadcast, settled
    let idx = begin.batch_transfer_idx.unwrap();
    let txid = psbt_txid(&signed);
    party.wallet.send_end(party.online, signed).unwrap();
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingCounterparty
    );
    chain.set_ack(&recipient.recipient_id, true);
    party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingConfirmations
    );
    assert!(chain.knows(&txid));
    chain.mine(1);
    party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
    assert_eq!(
        spendable(&party.wallet, &party.asset_id),
        AMOUNT - AMOUNT_SMALL
    );

    // the RGB change is on the new colored output; the vanilla change is ordinary bitcoin again
    let unspents = party.wallet.list_unspents(None, false, true).unwrap();
    let colored: Vec<&Unspent> = unspents.iter().filter(|u| u.utxo.colorable).collect();
    assert_eq!(colored.len(), 1);
    assert_eq!(colored[0].utxo.outpoint.txid, txid);
    assert_eq!(colored[0].utxo.outpoint.vout, 2);
    assert_eq!(colored[0].rgb_allocations.len(), 1);
    let balance = party
        .wallet
        .get_btc_balance(Some(party.online), false)
        .unwrap();
    assert_eq!(balance.vanilla.spendable, outputs[3].value.to_sat());
    assert_eq!(balance.colored.settled, 1000);

    // the wallet passes the consistency check, and a second send from the change works the same
    party.wallet.go_offline();
    party.online = party.wallet.go_online(online_options(&chain)).unwrap();
    let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
    let psbt = Psbt::from_str(&begin.psbt).unwrap();
    assert_eq!(
        input_keychains(&party.wallet, &psbt),
        vec![KeychainKind::External, KeychainKind::Internal]
    );
    let result = party.wallet.send_end(party.online, signed).unwrap();
    assert!(chain.knows(&result.txid));
    chain.mine(1);
    party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    assert_eq!(
        spendable(&party.wallet, &party.asset_id),
        AMOUNT - 2 * AMOUNT_SMALL
    );
    chain.assert_all_matched();
}

#[test]
#[parallel]
fn an_unconfirmed_vanilla_utxo_is_never_taken() {
    let chain = ScriptedChain::start();
    // the envelope and the vanilla change of the same create_utxos, both in the mempool
    let mut party = thin_issuer(&chain, false);
    party.wallet.set_vanilla_fee_inputs(true);
    let result = try_send(&chain, &mut party, false, true);
    assert_matches!(result, Err(Error::InsufficientAllocationSlots));

    // once it confirms (and the host has synced the vanilla keychain), it pays
    chain.mine(1);
    party
        .wallet
        .get_btc_balance(Some(party.online), false)
        .unwrap();
    let begin = try_send(&chain, &mut party, false, true).unwrap();
    let psbt = Psbt::from_str(&begin.psbt).unwrap();
    assert_eq!(
        input_keychains(&party.wallet, &psbt),
        vec![KeychainKind::External, KeychainKind::Internal]
    );
}

#[test]
#[parallel]
fn several_vanilla_utxos_pay_when_one_is_not_enough() {
    let chain = ScriptedChain::start();
    let mut party = thin_issuer(&chain, true);
    // leave ~1700 sats of the funding on the vanilla keychain, then add a 900-sat UTXO: neither
    // pays the witness output, the colored change output and the fee alone
    let vanilla = party
        .wallet
        .get_btc_balance(None, true)
        .unwrap()
        .vanilla
        .spendable;
    let away = get_test_wallet(false, None).get_address().unwrap();
    party
        .wallet
        .send_btc(party.online, away, vanilla - 2000, FEE_RATE, false)
        .unwrap();
    chain.fund(&party.wallet.get_address().unwrap(), 900);
    chain.mine(1);
    party
        .wallet
        .get_btc_balance(Some(party.online), false)
        .unwrap();
    let vanilla_before = vanilla_unspents(&party.wallet);
    assert_eq!(vanilla_before.len(), 2);

    party.wallet.set_vanilla_fee_inputs(true);
    let begin = try_send(&chain, &mut party, false, false).unwrap();
    let psbt = Psbt::from_str(&begin.psbt).unwrap();
    assert_eq!(
        input_keychains(&party.wallet, &psbt),
        vec![
            KeychainKind::External,
            KeychainKind::Internal,
            KeychainKind::Internal
        ]
    );
    // both vanilla UTXOs are the send's until it is broadcast
    let balance = party.wallet.get_btc_balance(None, true).unwrap();
    assert_eq!(balance.vanilla.spendable, 0);
    let outputs = &psbt.unsigned_tx.output;
    assert_eq!(
        keychain_of(&party.wallet, &outputs[2].script_pubkey),
        Some(KeychainKind::External)
    );
    // what is left over the fee and the two outputs comes back, unless it is dust BDK gives to
    // the miners (then only three outputs)
    if outputs.len() == 4 {
        assert_eq!(
            keychain_of(&party.wallet, &outputs[3].script_pubkey),
            Some(KeychainKind::Internal)
        );
    } else {
        assert_eq!(outputs.len(), 3);
    }
}

#[test]
#[parallel]
fn free_envelopes_pay_before_any_vanilla_utxo() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    party.wallet.set_vanilla_fee_inputs(true);
    let begin = try_send(&chain, &mut party, false, true).unwrap();
    let psbt = Psbt::from_str(&begin.psbt).unwrap();
    assert!(
        input_keychains(&party.wallet, &psbt)
            .iter()
            .all(|k| *k == KeychainKind::External)
    );
    // upstream's shape: the commitment, the witness output, the colored change
    assert_eq!(psbt.unsigned_tx.output.len(), 3);
    assert_eq!(
        keychain_of(&party.wallet, &psbt.unsigned_tx.output[2].script_pubkey),
        Some(KeychainKind::External)
    );
}

// CC-99 S1 on a vanilla-funded donation: the broadcast reaches the network, its answer and the
// lookup after it are lost, send_end rolls back; go_online with the completion completes it, the
// vanilla input included
#[test]
#[parallel]
fn a_vanilla_funded_send_whose_broadcast_answer_is_lost_is_completed_by_go_online() {
    let chain = ScriptedChain::start();
    let mut party = thin_issuer(&chain, true);
    party.wallet.set_vanilla_fee_inputs(true);
    let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
    let idx = begin.batch_transfer_idx.unwrap();
    let txid = psbt_txid(&signed);
    let vanilla_change = Psbt::from_str(&begin.psbt).unwrap().unsigned_tx.output[3]
        .value
        .to_sat();
    chain.fault("POST", "/tx", 502, true, 1);
    chain.fault("GET", &format!("/tx/{txid}/status"), 502, false, 1);
    let ended = party.wallet.send_end(party.online, signed);
    assert_matches!(ended, Err(Error::Indexer { .. }));
    assert!(chain.knows(&txid));
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);

    party.wallet.go_offline();
    party.online = party
        .wallet
        .go_online(OnlineOptions {
            complete_unrecorded_spends: true,
            ..online_options(&chain)
        })
        .unwrap();
    let completed = party.wallet.completed_spends();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].txid, txid);
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingConfirmations
    );
    chain.mine(1);
    party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
    let balance = party
        .wallet
        .get_btc_balance(Some(party.online), false)
        .unwrap();
    assert_eq!(balance.vanilla.spendable, vanilla_change);
    assert_eq!(
        spendable(&party.wallet, &party.asset_id),
        AMOUNT - AMOUNT_SMALL
    );
}
