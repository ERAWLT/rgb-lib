//! ERA fork: CC-99, an own colored spend whose record was lost, completed by `go_online`
//! (`OnlineOptions::complete_unrecorded_spends`), on the scripted chain (`scripted_chain`): no
//! regtest services.
//!
//! The three ways a spend loses its record are made for real: S1 by the scripted indexer (a
//! relayed `POST /tx` whose answer, and the lookup after it, are lost), S2 by the crash hooks, S3
//! by putting back a copy of the wallet directory taken earlier. The W numbers are those of the
//! design (ERA.md, section 6).

use super::scripted_chain::*;
use super::*;
use crate::wallet::test::era_fixtures::dir_listing;

fn completing_options(chain: &ScriptedChain) -> OnlineOptions {
    OnlineOptions {
        complete_unrecorded_spends: true,
        ..online_options(chain)
    }
}

/// S1: the next broadcast reaches the network, its answer and the lookup of `txid` after it are
/// lost.
fn lose_the_answer(chain: &ScriptedChain, txid: &str) {
    chain.fault("POST", "/tx", 502, true, 1);
    chain.fault("GET", &format!("/tx/{txid}/status"), 502, false, 1);
}

/// The wallet as the app reopens it after a pause: offline, then online again.
fn reopen(chain: &ScriptedChain, party: &mut Issuer, options: OnlineOptions) -> Result<(), Error> {
    party.wallet.go_offline();
    chain.clear_requests();
    party.online = party.wallet.go_online(options)?;
    Ok(())
}

fn status_lookups(chain: &ScriptedChain, txid: &str) -> usize {
    let lookup = format!("GET /tx/{txid}/status");
    chain.requests().iter().filter(|r| **r == lookup).count()
}

/// A donation of `AMOUNT_SMALL` whose `send_end` met S1: returns its batch transfer and TXID.
fn donation_answer_lost(chain: &ScriptedChain, party: &mut Issuer) -> (i32, String) {
    let (begin, signed, _) = begin_send(chain, party, AMOUNT_SMALL, true);
    let txid = psbt_txid(&signed);
    lose_the_answer(chain, &txid);
    let result = party.wallet.send_end(party.online, signed);
    assert_matches!(result, Err(Error::Indexer { .. }));
    assert!(chain.knows(&txid));
    let idx = begin.batch_transfer_idx.unwrap();
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);
    (idx, txid)
}

/// The digests of the database and of the RGB stash, which a refusal must leave as they were.
fn state_digest(party: &Issuer) -> String {
    let wallet_dir = party.wallet.get_wallet_dir();
    let data_dir = wallet_dir.parent().unwrap();
    let name = wallet_dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    dir_listing(data_dir, &name)
        .lines()
        .filter(|l| l.ends_with("/rgb_lib_db") || l.contains("/rgb/"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn stash_digest(party: &Issuer) -> String {
    state_digest(party)
        .lines()
        .filter(|l| l.contains("/rgb/"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    if !from.exists() {
        return;
    }
    for entry in WalkDir::new(from) {
        let entry = entry.unwrap();
        let target = to.join(entry.path().strip_prefix(from).unwrap());
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).unwrap();
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A copy of the wallet directory, as a seal or a VSS backup keeps it.
fn snapshot(party: &Issuer) -> tempfile::TempDir {
    let copy = tempfile::tempdir().unwrap();
    copy_dir(&party.wallet.get_wallet_dir(), copy.path());
    copy
}

/// Put a copy back in place of the wallet directory and open the wallet again, offline.
fn restore(party: Issuer, copy: &tempfile::TempDir) -> Issuer {
    let wallet_data = party.wallet.get_wallet_data();
    let keys = party.wallet.get_keys();
    let wallet_dir = party.wallet.get_wallet_dir();
    drop(party.wallet);
    fs::remove_dir_all(&wallet_dir).unwrap();
    copy_dir(copy.path(), &wallet_dir);
    Issuer {
        wallet: Wallet::new(wallet_data, keys).unwrap(),
        ..party
    }
}

/// The same wallet opened from a copy of its directory under another data dir: a control that
/// runs what the wallet under test lost.
fn control(party: &Issuer) -> (Issuer, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().unwrap();
    let wallet_dir = party.wallet.get_wallet_dir();
    copy_dir(
        &wallet_dir,
        &data_dir.path().join(wallet_dir.file_name().unwrap()),
    );
    let wallet_data = WalletData {
        data_dir: data_dir.path().to_string_lossy().to_string(),
        ..party.wallet.get_wallet_data()
    };
    let wallet = Wallet::new(wallet_data, party.wallet.get_keys()).unwrap();
    (
        Issuer {
            wallet,
            online: party.online,
            asset_id: party.asset_id.clone(),
        },
        data_dir,
    )
}

fn stash_witness(party: &Issuer, txid: &str) -> Option<PubWitness> {
    party
        .wallet
        .rgb_runtime()
        .unwrap()
        .stash_pub_witness(RgbTxid::from_str(txid).unwrap())
}

/// The stash's witness is the TX this wallet signed (as a donation's send_end consumes it).
fn is_signed(witness: &Option<PubWitness>) -> bool {
    matches!(witness, Some(PubWitness::Tx(tx)) if tx.input.iter().all(|i| !i.witness.is_empty()))
}

/// The stash's witness is the TX as the fascia stored it at send_begin, without signatures (as
/// every other send consumes it).
fn is_unsigned(witness: &Option<PubWitness>) -> bool {
    matches!(witness, Some(PubWitness::Tx(tx)) if tx.input.iter().all(|i| i.witness.is_empty()))
}

fn settle(chain: &ScriptedChain, party: &mut Issuer) {
    chain.mine(1);
    party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
}

// W1: a donation whose broadcast answer was lost is completed by the next go_online, in the
// mempool and in a block, and afterwards behaves as one whose send_end went through
#[test]
#[parallel]
fn a_donation_whose_answer_was_lost_completes_at_go_online() {
    for confirmed in [false, true] {
        let chain = ScriptedChain::start();
        let mut party = issuer(&chain, vec![AMOUNT]);
        let (idx, txid) = donation_answer_lost(&chain, &mut party);
        if confirmed {
            chain.mine(1);
        }

        reopen(&chain, &mut party, completing_options(&chain)).unwrap();
        assert_eq!(
            party.wallet.completed_spends(),
            vec![CompletedSpend {
                txid: txid.clone(),
                batch_transfer_idx: Some(idx),
                previous_status: Some(TransferStatus::Initiated),
                donation: true,
                confirmations: u64::from(confirmed),
            }]
        );
        assert_eq!(status_lookups(&chain, &txid), 1);
        assert_eq!(
            status_of(&party.wallet, idx),
            TransferStatus::WaitingConfirmations
        );
        // the stash holds the transition, witnessed by the TX this wallet signed, as send_end
        // consumes a donation
        assert!(is_signed(&stash_witness(&party, &txid)));
        assert!(chain.proxy_methods().iter().all(|m| m != "ack.get"));

        // the next go_online finds nothing to complete and asks nothing about the TX, and going
        // offline drops the report
        reopen(&chain, &mut party, completing_options(&chain)).unwrap();
        assert!(party.wallet.completed_spends().is_empty());
        assert_eq!(status_lookups(&chain, &txid), 0);
        party.wallet.go_offline();
        assert!(party.wallet.completed_spends().is_empty());
        party.online = party.wallet.go_online(completing_options(&chain)).unwrap();

        // it settles and its change is spent by the next send, as after a send_end that went
        // through
        settle(&chain, &mut party);
        assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
        assert_eq!(
            spendable(&party.wallet, &party.asset_id),
            AMOUNT - AMOUNT_SMALL
        );
        let (_, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
        let next = party.wallet.send_end(party.online, signed).unwrap();
        assert!(chain.knows(&next.txid));
        settle(&chain, &mut party);
        assert_eq!(
            spendable(&party.wallet, &party.asset_id),
            AMOUNT - 2 * AMOUNT_SMALL
        );
    }
}

// a completion is an operation: the next backup has to carry it
#[test]
#[parallel]
fn a_completion_asks_for_a_backup() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (_, _) = donation_answer_lost(&chain, &mut party);
    let backup_dir = tempfile::tempdir().unwrap();
    party
        .wallet
        .backup(
            &backup_dir.path().join("backup").to_string_lossy(),
            PASSWORD,
        )
        .unwrap();
    assert!(!party.wallet.backup_info().unwrap());

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(party.wallet.completed_spends().len(), 1);
    assert!(party.wallet.backup_info().unwrap());
}

// W2: the refresh that broadcasts an ACKed send meets S1; it records the failure and commits the
// ACK, and the next go_online completes the send without asking the RGB proxy anything
#[test]
#[parallel]
fn an_acked_send_whose_answer_was_lost_completes_at_go_online() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (begin, signed, recipient) = begin_send(&chain, &mut party, AMOUNT_SMALL, false);
    let idx = begin.batch_transfer_idx.unwrap();
    let txid = psbt_txid(&signed);
    party.wallet.send_end(party.online, signed).unwrap();
    chain.set_ack(&recipient.recipient_id, true);
    lose_the_answer(&chain, &txid);
    let result = party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    assert_matches!(result[&idx].failure, Some(Error::Indexer { .. }));
    assert!(chain.knows(&txid));
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingCounterparty
    );

    let proxy_calls = chain.proxy_methods().len();
    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(chain.proxy_methods().len(), proxy_calls);
    assert_eq!(
        party.wallet.completed_spends(),
        vec![CompletedSpend {
            txid: txid.clone(),
            batch_transfer_idx: Some(idx),
            previous_status: Some(TransferStatus::WaitingCounterparty),
            donation: false,
            confirmations: 0,
        }]
    );
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingConfirmations
    );
    // a non-donation send consumes its fascia as stored, witnessed by the unsigned TX
    assert!(is_unsigned(&stash_witness(&party, &txid)));
    settle(&chain, &mut party);
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
}

// W3: one refresh broadcasts two ACKed sends and loses both answers; one go_online completes both
// in one commit, and the one after it has nothing to do
#[test]
#[parallel]
fn two_sends_whose_answers_were_lost_complete_together() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT, AMOUNT]);
    let mut sends = vec![];
    for _ in 0..2 {
        let (begin, signed, recipient) = begin_send(&chain, &mut party, AMOUNT_SMALL, false);
        let txid = psbt_txid(&signed);
        party.wallet.send_end(party.online, signed).unwrap();
        chain.set_ack(&recipient.recipient_id, true);
        lose_the_answer(&chain, &txid);
        sends.push((txid, begin.batch_transfer_idx.unwrap()));
    }
    let result = party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    for (txid, idx) in &sends {
        assert_matches!(result[idx].failure, Some(Error::Indexer { .. }));
        assert!(chain.knows(txid));
    }

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    sends.sort();
    let completed: Vec<(String, i32)> = party
        .wallet
        .completed_spends()
        .into_iter()
        .map(|c| (c.txid, c.batch_transfer_idx.unwrap()))
        .collect();
    assert_eq!(completed, sends);
    for (_, idx) in &sends {
        assert_eq!(
            status_of(&party.wallet, *idx),
            TransferStatus::WaitingConfirmations
        );
    }

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert!(party.wallet.completed_spends().is_empty());
    assert!(
        sends
            .iter()
            .all(|(txid, _)| status_lookups(&chain, txid) == 0)
    );
}

// W4: a drain of empty colored UTXOs meets S1; go_online completes it and releases its
// reservations
#[test]
#[parallel]
fn a_drain_whose_answer_was_lost_completes_at_go_online() {
    let chain = ScriptedChain::start();
    let (wallet, online) = funded(&chain, UTXOS);
    let mut party = Issuer {
        wallet,
        online,
        asset_id: s!(""),
    };
    let address = get_test_wallet(false, None).get_address().unwrap();
    let psbt = party
        .wallet
        .drain_to_begin(party.online, address, FEE_RATE, false)
        .unwrap();
    let signed = party.wallet.sign_psbt(psbt, None).unwrap();
    let txid = psbt_txid(&signed);
    lose_the_answer(&chain, &txid);
    let result = party.wallet.drain_to_end(party.online, signed);
    assert_matches!(result, Err(Error::Indexer { .. }));
    assert!(chain.knows(&txid));
    assert_eq!(party.wallet.list_pending_vanilla_txs().unwrap().len(), 1);

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(
        party.wallet.completed_spends(),
        vec![CompletedSpend {
            txid,
            batch_transfer_idx: None,
            previous_status: None,
            donation: false,
            confirmations: 0,
        }]
    );
    assert!(party.wallet.list_pending_vanilla_txs().unwrap().is_empty());
    assert!(
        party
            .wallet
            .list_unspents(None, false, true)
            .unwrap()
            .is_empty()
    );
}

// W5: S3, a copy taken after send_begin is restored while the send went out in the meantime;
// a donation (no signed.psbt in the copy) and a send that its refresh broadcast
#[test]
#[parallel]
fn a_copy_taken_after_send_begin_completes_the_send() {
    for donation in [true, false] {
        let chain = ScriptedChain::start();
        let mut party = issuer(&chain, vec![AMOUNT]);
        let (begin, signed, recipient) = begin_send(&chain, &mut party, AMOUNT_SMALL, donation);
        let idx = begin.batch_transfer_idx.unwrap();
        let txid = psbt_txid(&signed);
        let copy = snapshot(&party);
        party.wallet.send_end(party.online, signed).unwrap();
        if !donation {
            chain.set_ack(&recipient.recipient_id, true);
            party
                .wallet
                .refresh(party.online, None, vec![], false)
                .unwrap();
        }
        assert!(chain.knows(&txid));

        let mut party = restore(party, &copy);
        party.online = party.wallet.go_online(completing_options(&chain)).unwrap();
        assert_eq!(
            party.wallet.completed_spends(),
            vec![CompletedSpend {
                txid: txid.clone(),
                batch_transfer_idx: Some(idx),
                previous_status: Some(TransferStatus::Initiated),
                donation,
                confirmations: 0,
            }]
        );
        // without the signed PSBT the stash gets the fascia as stored, with the unsigned TX
        assert!(is_unsigned(&stash_witness(&party, &txid)));
        settle(&chain, &mut party);
        assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
        assert_eq!(
            spendable(&party.wallet, &party.asset_id),
            AMOUNT - AMOUNT_SMALL
        );
    }
}

// W6: a wallet restored from a VSS backup: the marker goes with the completion's commit, and stays
// when the check refuses or the commit does not happen
#[cfg(feature = "vss")]
#[test]
#[parallel]
fn the_vss_marker_goes_only_with_a_committed_completion() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let marker = party
        .wallet
        .get_wallet_dir()
        .join(crate::wallet::vss::VSS_RESTORE_MARKER);
    let (idx, _) = donation_answer_lost(&chain, &mut party);
    fs::write(&marker, "").unwrap();

    MOCK_FAIL_BEFORE_COMPLETION_COMMIT.replace(Some(()));
    let result = reopen(&chain, &mut party, completing_options(&chain));
    assert_matches!(result, Err(Error::Internal { details }) if details.contains("completion commit"));
    assert!(marker.exists());
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(party.wallet.completed_spends().len(), 1);
    assert!(!marker.exists());

    // a copy older than the send: the refusal is the restored backup's, and the marker stays
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let copy = snapshot(&party);
    let (_, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
    party.wallet.send_end(party.online, signed).unwrap();
    let mut party = restore(party, &copy);
    let marker = party
        .wallet
        .get_wallet_dir()
        .join(crate::wallet::vss::VSS_RESTORE_MARKER);
    fs::write(&marker, "").unwrap();
    let result = party.wallet.go_online(completing_options(&chain));
    let error = result.unwrap_err();
    assert_matches!(error, Error::RestoredBackupInconsistent { .. });
    assert_eq!(
        error.inconsistency_reason(),
        Some(InconsistencyReason::SpenderNotRecorded)
    );
    assert!(marker.exists());
}

// W7: a stash that holds the transition already (a send_end killed after its broadcast and
// consume; a completion that failed before its commit) is not consumed again, and ends up as the
// stash of a wallet whose send_end went through
#[test]
#[parallel]
fn a_stash_ahead_of_the_database_ends_as_after_a_send_end() {
    for crash_in_send_end in [true, false] {
        let chain = ScriptedChain::start();
        let mut party = issuer(&chain, vec![AMOUNT]);
        let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
        let idx = begin.batch_transfer_idx.unwrap();
        let txid = psbt_txid(&signed);
        let (mut control, _control_dir) = control(&party);
        control
            .wallet
            .go_online(online_options(&chain))
            .map(|online| control.online = online)
            .unwrap();

        if crash_in_send_end {
            MOCK_SEND_END_CRASH.replace(Some(()));
            let result = party.wallet.send_end(party.online, signed.clone());
            assert_matches!(result, Err(Error::Internal { .. }));
        } else {
            lose_the_answer(&chain, &txid);
            let result = party.wallet.send_end(party.online, signed.clone());
            assert_matches!(result, Err(Error::Indexer { .. }));
            MOCK_FAIL_BEFORE_COMPLETION_COMMIT.replace(Some(()));
            let result = reopen(&chain, &mut party, completing_options(&chain));
            assert_matches!(result, Err(Error::Internal { .. }));
        }
        assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);
        // the stash is ahead of the database
        assert!(is_signed(&stash_witness(&party, &txid)));

        control.wallet.send_end(control.online, signed).unwrap();
        reopen(&chain, &mut party, completing_options(&chain)).unwrap();
        assert_eq!(party.wallet.completed_spends().len(), 1);
        assert_eq!(
            status_of(&party.wallet, idx),
            TransferStatus::WaitingConfirmations
        );
        assert_eq!(stash_digest(&party), stash_digest(&control));
    }
}

// a transition the stash holds already is not consumed again: the completion goes by the stash,
// not by the file (here damaged, which a consume would refuse). On rgb-ops 0.11.1-rc.11 a second
// consume of an intact fascia changes nothing, so this is the one way to see the difference.
#[test]
#[parallel]
fn a_transition_the_stash_holds_is_not_consumed_again() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (begin, signed) = begin_two_asset_donation(&chain, &mut party);
    let idx = begin.batch_transfer_idx.unwrap();
    let txid = psbt_txid(&signed);
    MOCK_SEND_END_CRASH.replace(Some(()));
    let result = party.wallet.send_end(party.online, signed);
    assert_matches!(result, Err(Error::Internal { .. }));
    assert!(is_signed(&stash_witness(&party, &txid)));
    damage_second_bundle(&party, &txid);

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(party.wallet.completed_spends().len(), 1);
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingConfirmations
    );
}

// W8: without the option, go_online answers exactly as upstream and asks the indexer nothing
// about the spend
#[test]
#[parallel]
fn without_the_option_the_check_is_upstream() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
    let txid = psbt_txid(&signed);
    let inputs: Vec<String> = Psbt::from_str(&signed)
        .unwrap()
        .unsigned_tx
        .input
        .iter()
        .map(|i| i.previous_output.to_string())
        .collect();
    assert_eq!(inputs.len(), 1);
    lose_the_answer(&chain, &txid);
    let result = party.wallet.send_end(party.online, signed);
    assert_matches!(result, Err(Error::Indexer { .. }));
    let digest = state_digest(&party);

    let error = reopen(&chain, &mut party, online_options(&chain)).unwrap_err();
    assert_eq!(
        error,
        Error::Inconsistency {
            details: format!("spent bitcoins with another wallet: [\"{}\"]", inputs[0]),
        }
    );
    assert_eq!(error.inconsistency_reason(), None);
    assert_eq!(status_lookups(&chain, &txid), 0);
    assert_eq!(state_digest(&party), digest);
    assert_eq!(
        status_of(&party.wallet, begin.batch_transfer_idx.unwrap()),
        TransferStatus::Initiated
    );
}

// W9: with the check skipped nothing is looked up or completed, and each go_online replaces the
// report
#[test]
#[parallel]
fn a_skipped_check_completes_nothing() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (idx, txid) = donation_answer_lost(&chain, &mut party);
    let skipping = OnlineOptions {
        skip_consistency_check: true,
        ..completing_options(&chain)
    };
    reopen(&chain, &mut party, skipping.clone()).unwrap();
    assert!(party.wallet.completed_spends().is_empty());
    assert_eq!(status_lookups(&chain, &txid), 0);
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);

    // a go_online that completes, then one on the same online state that skips the check
    party.online = party.wallet.go_online(completing_options(&chain)).unwrap();
    assert_eq!(party.wallet.completed_spends().len(), 1);
    party.online = party.wallet.go_online(skipping).unwrap();
    assert!(party.wallet.completed_spends().is_empty());
}

// W10: the refusals write nothing: the database and the stash are as they were
#[test]
#[parallel]
fn refusals_write_nothing() {
    // a copy older than the send (seal before send_begin): the spender is not recorded
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let copy = snapshot(&party);
    let (_, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
    let txid = psbt_txid(&signed);
    party.wallet.send_end(party.online, signed).unwrap();
    let mut party = restore(party, &copy);
    let digest = state_digest(&party);
    let error = party
        .wallet
        .go_online(completing_options(&chain))
        .unwrap_err();
    assert_matches!(&error, Error::Inconsistency { details } if details.contains(&txid));
    assert_eq!(
        error.inconsistency_reason(),
        Some(InconsistencyReason::SpenderNotRecorded)
    );
    assert!(party.wallet.completed_spends().is_empty());
    assert_eq!(state_digest(&party), digest);

    // a copy taken after the first of two chained sends: the second, which spent the first's
    // change, is not recorded
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (_, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
    let copy = snapshot(&party);
    party.wallet.send_end(party.online, signed).unwrap();
    settle(&chain, &mut party);
    let (_, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
    party.wallet.send_end(party.online, signed).unwrap();
    let mut party = restore(party, &copy);
    let digest = state_digest(&party);
    let error = party
        .wallet
        .go_online(completing_options(&chain))
        .unwrap_err();
    assert_eq!(
        error.inconsistency_reason(),
        Some(InconsistencyReason::LaterSpendUnrecorded)
    );
    assert_eq!(state_digest(&party), digest);

    // a donation failed while its TX was unknown, then broadcast by its recipient
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
    let idx = begin.batch_transfer_idx.unwrap();
    let txid = psbt_txid(&signed);
    chain.withhold_broadcasts(1);
    chain.fault("POST", "/tx", 502, true, 1);
    let result = party.wallet.send_end(party.online, signed);
    assert_matches!(result, Err(Error::FailedBroadcast { .. }));
    assert!(
        party
            .wallet
            .fail_transfers(party.online, Some(idx), false, false)
            .unwrap()
    );
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Failed);
    chain.release(&txid);
    let digest = state_digest(&party);
    let result = reopen(&chain, &mut party, completing_options(&chain));
    assert_eq!(
        result.unwrap_err(),
        Error::UnrecordedSpend {
            txid: txid.clone(),
            reason: s!("transfer-failed"),
            batch_transfer_idx: Some(idx),
        }
    );
    assert_eq!(state_digest(&party), digest);

    // the transfer's saved data is gone
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (idx, txid) = donation_answer_lost(&chain, &mut party);
    fs::remove_file(party.wallet.get_transfers_dir().join(&txid).join("fascia")).unwrap();
    let digest = state_digest(&party);
    let result = reopen(&chain, &mut party, completing_options(&chain));
    assert_eq!(
        result.unwrap_err(),
        Error::UnrecordedSpend {
            txid: txid.clone(),
            reason: s!("transfer-data-missing"),
            batch_transfer_idx: Some(idx),
        }
    );
    assert_eq!(state_digest(&party), digest);

    // the lookup fails: the indexer's error, retryable
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (idx, txid) = donation_answer_lost(&chain, &mut party);
    chain.fault("GET", &format!("/tx/{txid}/status"), 502, false, 1);
    let digest = state_digest(&party);
    let result = reopen(&chain, &mut party, completing_options(&chain));
    assert_matches!(result, Err(Error::Indexer { .. }));
    assert_eq!(state_digest(&party), digest);
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);

    // the wallet applied its TX, which then left every mempool: the indexer does not know it
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
    let idx = begin.batch_transfer_idx.unwrap();
    let txid = psbt_txid(&signed);
    MOCK_SEND_END_CRASH.replace(Some(()));
    let result = party.wallet.send_end(party.online, signed);
    assert_matches!(result, Err(Error::Internal { .. }));
    chain.evict(&txid);
    let digest = state_digest(&party);
    let result = reopen(&chain, &mut party, completing_options(&chain));
    assert_eq!(
        result.unwrap_err(),
        Error::UnrecordedSpendUnseen {
            txid,
            batch_transfer_idx: Some(idx),
        }
    );
    assert_eq!(state_digest(&party), digest);
}

/// A donation of the issued asset and of a second one, in one batch: a fascia of two bundles.
fn begin_two_asset_donation(
    chain: &ScriptedChain,
    party: &mut Issuer,
) -> (SendBeginResult, String) {
    let other_asset = issue(&party.wallet, vec![AMOUNT]);
    let begin = party
        .wallet
        .send_begin(
            party.online,
            HashMap::from([
                (
                    party.asset_id.clone(),
                    vec![witness_recipient(chain, AMOUNT_SMALL)],
                ),
                (other_asset, vec![witness_recipient(chain, AMOUNT_SMALL)]),
            ]),
            true,
            FEE_RATE,
            MIN_CONFIRMATIONS,
            (now().unix_timestamp() + DURATION_SEND_TRANSFER as i64) as u64,
            false,
            None,
        )
        .unwrap();
    let signed = party.wallet.sign_psbt(begin.psbt.clone(), None).unwrap();
    (begin, signed)
}

/// The second bundle the stash would consume from the saved fascia names a transition its input
/// map does not: a consume stops there, after the first bundle. The bundle IDs do not change.
fn damage_second_bundle(party: &Issuer, txid: &str) {
    let fascia_path = party.wallet.get_transfers_dir().join(txid).join("fascia");
    let fascia_str = fs::read_to_string(&fascia_path).unwrap();
    let fascia: Fascia = serde_json::from_str(&fascia_str).unwrap();
    let contracts: Vec<String> = fascia.bundles().keys().map(|c| c.to_string()).collect();
    assert_eq!(contracts.len(), 2);
    let mut json: serde_json::Value = serde_json::from_str(&fascia_str).unwrap();
    let first_opid = json["bundles"][&contracts[0]]["knownTransitions"][0]["opid"].clone();
    assert!(first_opid.is_string());
    json["bundles"][&contracts[1]]["knownTransitions"][0]["opid"] = first_opid;
    fs::write(&fascia_path, json.to_string()).unwrap();
}

// a stash that refuses a spend half way (the second of its two bundles) keeps nothing of that
// spend, and the refusal has its reason instead of a panic
#[test]
#[parallel]
fn a_stash_that_refuses_a_spend_keeps_nothing_of_it() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (begin, signed) = begin_two_asset_donation(&chain, &mut party);
    let txid = psbt_txid(&signed);
    lose_the_answer(&chain, &txid);
    let result = party.wallet.send_end(party.online, signed);
    assert_matches!(result, Err(Error::Indexer { .. }));
    damage_second_bundle(&party, &txid);

    let digest = state_digest(&party);
    let result = reopen(&chain, &mut party, completing_options(&chain));
    assert_eq!(
        result.unwrap_err(),
        Error::UnrecordedSpend {
            txid,
            reason: s!("stash-refused"),
            batch_transfer_idx: begin.batch_transfer_idx,
        }
    );
    assert_eq!(state_digest(&party), digest);
}

// a stash the disk will not take while a spend is completed (here the wallet's rgb/ directory
// read-only, so no store can create its new file): the go_online fails with the retryable
// Error::IO rather than a panic, nothing is committed and the stash is as it was; once the disk
// takes it again, the next go_online completes the spend and it settles
#[cfg(unix)]
#[test]
#[parallel]
fn a_stash_the_disk_will_not_take_commits_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (idx, _) = donation_answer_lost(&chain, &mut party);
    let rgb_dir = party.wallet.get_wallet_dir().join("rgb");
    let digest = state_digest(&party);
    fs::set_permissions(&rgb_dir, fs::Permissions::from_mode(0o555)).unwrap();
    if fs::File::create(rgb_dir.join("probe")).is_ok() {
        // running as root
        let _ = fs::remove_file(rgb_dir.join("probe"));
        fs::set_permissions(&rgb_dir, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let result = reopen(&chain, &mut party, completing_options(&chain));
    fs::set_permissions(&rgb_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert_matches!(result, Err(Error::IO { details }) if details.contains("stash.dat"));
    assert_eq!(state_digest(&party), digest);
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(party.wallet.completed_spends().len(), 1);
    settle(&chain, &mut party);
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
    assert_eq!(
        spendable(&party.wallet, &party.asset_id),
        AMOUNT - AMOUNT_SMALL
    );
}

// W11: try_complete_batch broadcasts only its own transfer's TX
#[test]
#[parallel]
fn a_signed_psbt_of_another_tx_is_not_broadcast() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT, AMOUNT]);
    let (first, first_signed, first_recipient) =
        begin_send(&chain, &mut party, AMOUNT_SMALL, false);
    let first_txid = psbt_txid(&first_signed);
    party.wallet.send_end(party.online, first_signed).unwrap();
    let (_, second_signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, false);
    let second_txid = psbt_txid(&second_signed);
    party.wallet.send_end(party.online, second_signed).unwrap();
    assert_ne!(first_txid, second_txid);

    // the first transfer's file now holds the second transfer's signed TX
    let transfers_dir = party.wallet.get_transfers_dir();
    fs::copy(
        transfers_dir.join(&second_txid).join("signed.psbt"),
        transfers_dir.join(&first_txid).join("signed.psbt"),
    )
    .unwrap();
    chain.set_ack(&first_recipient.recipient_id, true);
    chain.clear_requests();

    let idx = first.batch_transfer_idx.unwrap();
    let result = party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    assert_matches!(
        &result[&idx].failure,
        Some(Error::InvalidPsbt { details }) if details.contains("for another TX")
    );
    assert!(!chain.requests().contains(&s!("POST /tx")));
    assert!(!chain.knows(&first_txid) && !chain.knows(&second_txid));
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingCounterparty
    );
}

/// Two sends waiting for their ACKs, the first one's signed.psbt replaced by the second one's and
/// the first one ACKed (as in W11). `before_the_swap` runs between the two sends. Returns the
/// first send's batch transfer and TXID.
fn a_send_holding_another_tx(
    chain: &ScriptedChain,
    party: &mut Issuer,
    before_the_swap: impl FnOnce(&Issuer),
) -> (i32, String) {
    let (first, first_signed, first_recipient) = begin_send(chain, party, AMOUNT_SMALL, false);
    let first_txid = psbt_txid(&first_signed);
    party.wallet.send_end(party.online, first_signed).unwrap();
    before_the_swap(party);
    let (_, second_signed, _) = begin_send(chain, party, AMOUNT_SMALL, false);
    let second_txid = psbt_txid(&second_signed);
    party.wallet.send_end(party.online, second_signed).unwrap();
    let transfers_dir = party.wallet.get_transfers_dir();
    fs::copy(
        transfers_dir.join(&second_txid).join("signed.psbt"),
        transfers_dir.join(&first_txid).join("signed.psbt"),
    )
    .unwrap();
    chain.set_ack(&first_recipient.recipient_id, true);
    (first.batch_transfer_idx.unwrap(), first_txid)
}

// A send the W11 guard refuses at every refresh can still be failed before it expires (it was
// locked until then, its coins reserved), and nothing is broadcast
#[test]
#[parallel]
fn a_send_whose_signed_psbt_is_refused_can_be_failed() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT, AMOUNT]);
    let (idx, txid) = a_send_holding_another_tx(&chain, &mut party, |_| {});
    let before = spendable(&party.wallet, &party.asset_id);
    chain.clear_requests();

    let result = party
        .wallet
        .fail_transfers(party.online, Some(idx), false, false);
    assert_matches!(result, Ok(true));
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Failed);
    assert!(!chain.requests().contains(&s!("POST /tx")));
    assert!(!chain.knows(&txid));
    assert_eq!(spendable(&party.wallet, &party.asset_id), before + AMOUNT);
}

// ... but not when its own TX is on chain (another copy of the wallet broadcast it): that send
// happened, and failing it would stop crediting its change
#[test]
#[parallel]
fn a_send_whose_signed_psbt_is_refused_is_not_failed_with_its_tx_on_chain() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT, AMOUNT]);
    let mut copy = None;
    let (idx, txid) =
        a_send_holding_another_tx(&chain, &mut party, |party| copy = Some(control(party)));
    let (mut other, _other_dir) = copy.unwrap();
    other.online = other.wallet.go_online(online_options(&chain)).unwrap();
    other
        .wallet
        .refresh(other.online, None, vec![], false)
        .unwrap();
    assert!(chain.knows(&txid));
    chain.clear_requests();

    let result = party
        .wallet
        .fail_transfers(party.online, Some(idx), false, false);
    assert_matches!(result, Err(Error::CannotFailBatchTransfer));
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingCounterparty
    );
    assert!(!chain.requests().contains(&s!("POST /tx")));
}

// Upstream's own lock of the same kind: a signed.psbt left empty (send_end writes it without a
// sync) fails every refresh; the send can be failed, and nothing is broadcast
#[test]
#[parallel]
fn a_send_whose_signed_psbt_is_empty_can_be_failed() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT, AMOUNT]);
    let (begin, signed, recipient) = begin_send(&chain, &mut party, AMOUNT_SMALL, false);
    let txid = psbt_txid(&signed);
    party.wallet.send_end(party.online, signed).unwrap();
    let idx = begin.batch_transfer_idx.unwrap();
    fs::write(
        party
            .wallet
            .get_transfers_dir()
            .join(&txid)
            .join("signed.psbt"),
        "",
    )
    .unwrap();
    chain.set_ack(&recipient.recipient_id, true);
    let refreshed = party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    assert_matches!(&refreshed[&idx].failure, Some(Error::InvalidPsbt { .. }));
    let before = spendable(&party.wallet, &party.asset_id);
    chain.clear_requests();

    let result = party
        .wallet
        .fail_transfers(party.online, Some(idx), false, false);
    assert_matches!(result, Ok(true));
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Failed);
    assert!(!chain.requests().contains(&s!("POST /tx")));
    assert!(!chain.knows(&txid));
    assert_eq!(spendable(&party.wallet, &party.asset_id), before + AMOUNT);
}

// Planner table (design section 5.1): `plan` over a real wallet state (database, BDK view, transfer
// files), changed one way per case in a transaction that is rolled back and a copy of the
// transfer files, with the indexer's answers given by the case. Every refusal must also leave the
// plan unwritten, which `plan` guarantees by only reading; the wallet tests above check the files.

use crate::wallet::unrecorded_spends::{Plan, PlannedRecord, SpendView, TxStatusLookup, plan};

#[derive(Default)]
struct Lookup {
    answers: HashMap<String, Result<Option<u64>, Error>>,
    asked: RefCell<Vec<String>>,
}

impl TxStatusLookup for Lookup {
    fn confirmations(&self, txid: &str) -> Result<Option<u64>, Error> {
        self.asked.borrow_mut().push(txid.to_string());
        self.answers.get(txid).cloned().unwrap_or(Ok(Some(0)))
    }
}

/// A wallet state with own spends that lost their record, scanned by a go_online without the
/// option (which refused, and left BDK knowing the spends).
struct Base {
    _chain: ScriptedChain,
    party: Issuer,
    // (TXID, batch transfer, signed PSBT) of each spend, in TXID order
    spends: Vec<(String, Option<i32>, String)>,
    details: String,
}

impl Base {
    fn scan(
        chain: ScriptedChain,
        mut party: Issuer,
        mut spends: Vec<(String, Option<i32>, String)>,
    ) -> Self {
        party.wallet.go_offline();
        let details = match party.wallet.go_online(online_options(&chain)) {
            Err(Error::Inconsistency { details }) => details,
            other => panic!("expected the upstream refusal, got {other:?}"),
        };
        spends.sort();
        Self {
            _chain: chain,
            party,
            spends,
            details,
        }
    }

    /// Two donations that met S1.
    fn two_donations() -> Self {
        let chain = ScriptedChain::start();
        let mut party = issuer(&chain, vec![AMOUNT, AMOUNT]);
        let mut spends = vec![];
        for _ in 0..2 {
            let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
            let txid = psbt_txid(&signed);
            lose_the_answer(&chain, &txid);
            let result = party.wallet.send_end(party.online, signed.clone());
            assert_matches!(result, Err(Error::Indexer { .. }));
            spends.push((txid, begin.batch_transfer_idx, signed));
        }
        // an asset of the wallet that neither moves
        issue(&party.wallet, vec![AMOUNT]);
        Self::scan(chain, party, spends)
    }

    fn txid(&self, i: usize) -> &str {
        &self.spends[i].0
    }

    fn idx(&self, i: usize) -> i32 {
        self.spends[i].1.unwrap()
    }

    fn signed_tx(&self, i: usize) -> BdkTransaction {
        Psbt::from_str(&self.spends[i].2)
            .unwrap()
            .extract_tx()
            .unwrap()
    }

    /// `plan` after `change` (the database, in a transaction rolled back after the case, and a
    /// copy of the transfer files) and `answer` (BDK's view and the indexer's answers).
    fn plan(
        &self,
        change: impl FnOnce(&DbTxn, &Path),
        answer: impl FnOnce(&mut SpendView, &mut Lookup),
    ) -> (Result<Plan, Error>, Vec<String>) {
        let txn = self.party.wallet.database().begin_transaction().unwrap();
        let transfers = tempfile::tempdir().unwrap();
        copy_dir(&self.party.wallet.get_transfers_dir(), transfers.path());
        change(&txn, transfers.path());
        let bdk_wallet = self.party.wallet.bdk_wallet();
        let divergence = colored_divergence(bdk_wallet, &txn).unwrap();
        let mut view = SpendView::from_bdk(bdk_wallet, &divergence);
        let mut lookup = Lookup::default();
        answer(&mut view, &mut lookup);
        let result = plan(
            &view,
            &divergence,
            &lookup,
            &txn,
            transfers.path(),
            self.details.clone(),
        );
        let asked = lookup.asked.into_inner();
        (result, asked)
    }

    fn plan_after(&self, change: impl FnOnce(&DbTxn, &Path)) -> Result<Plan, Error> {
        self.plan(change, |_, _| {}).0
    }
}

use crate::wallet::unrecorded_spends::colored_divergence;

fn batch_transfer(txn: &DbTxn, idx: i32) -> DbBatchTransfer {
    txn.iter_batch_transfers()
        .unwrap()
        .into_iter()
        .find(|b| b.idx == idx)
        .unwrap()
}

fn update_batch_transfer(txn: &DbTxn, idx: i32, update: impl FnOnce(&mut DbBatchTransferActMod)) {
    let mut batch_transfer: DbBatchTransferActMod = batch_transfer(txn, idx).into();
    update(&mut batch_transfer);
    txn.update_batch_transfer(&mut batch_transfer).unwrap();
}

fn set_status(txn: &DbTxn, idx: i32, status: TransferStatus) {
    update_batch_transfer(txn, idx, |b| b.status = ActiveValue::Set(status));
}

fn asset_transfers_of(txn: &DbTxn, idx: i32) -> Vec<DbAssetTransfer> {
    txn.iter_asset_transfers()
        .unwrap()
        .into_iter()
        .filter(|a| a.batch_transfer_idx == idx)
        .collect()
}

fn rewrite_info(transfers: &Path, txid: &str, change: impl FnOnce(&mut InfoBatchTransfer)) {
    let path = transfers.join(txid).join(TRANSFER_DATA_FILE);
    let mut info: InfoBatchTransfer =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    change(&mut info);
    fs::write(path, serde_json::to_string(&info).unwrap()).unwrap();
}

fn unrecorded(txid: &str, reason: UnrecordedSpendReason, idx: Option<i32>) -> Error {
    Error::UnrecordedSpend {
        txid: txid.to_string(),
        reason: reason.code().to_string(),
        batch_transfer_idx: idx,
    }
}

fn witness_tx(plan: &Plan, i: usize) -> Option<BdkTransaction> {
    match &plan.spends[i].record {
        PlannedRecord::Send { fascia, .. } => match &fascia.seal_witness().public {
            PubWitness::Tx(tx) => Some(tx.clone()),
            PubWitness::Txid(_) => None,
        },
        PlannedRecord::Drain { .. } => None,
    }
}

#[test]
#[parallel]
fn plan_table_of_two_donations() {
    use UnrecordedSpendReason::*;
    let base = Base::two_donations();
    let (t0, t1) = (base.txid(0).to_string(), base.txid(1).to_string());
    let (i0, i1) = (base.idx(0), base.idx(1));

    // C1 x2 (C5): both completed, in TXID order, each witnessed by the TX it signed
    let (result, asked) = base.plan(
        |_, _| {},
        |_, lookup| {
            lookup.answers.insert(t1.clone(), Ok(Some(3)));
        },
    );
    let plan = result.unwrap();
    assert_eq!(asked, vec![t0.clone(), t1.clone()]);
    let planned: Vec<(&str, u64)> = plan
        .spends
        .iter()
        .map(|s| (s.txid.as_str(), s.confirmations))
        .collect();
    assert_eq!(planned, vec![(t0.as_str(), 0), (t1.as_str(), 3)]);
    for i in 0..2 {
        assert_eq!(
            witness_tx(&plan, i).unwrap().compute_wtxid(),
            base.signed_tx(i).compute_wtxid()
        );
    }

    // the witness is the signed PSBT's TX, never BDK's copy of it (here with other witness data)
    let (result, _) = base.plan(
        |_, _| {},
        |view, _| {
            let tx = view.txs.get_mut(&t0).unwrap();
            tx.input[0].witness = bdk_wallet::bitcoin::Witness::from_slice(&[[7u8; 64]]);
        },
    );
    let plan = result.unwrap();
    assert_eq!(
        witness_tx(&plan, 0).unwrap().compute_wtxid(),
        base.signed_tx(0).compute_wtxid()
    );

    // a donation copy without signed.psbt (S3): the fascia as stored, with the unsigned TX
    let plan = base
        .plan_after(|_, transfers| {
            fs::remove_file(transfers.join(&t0).join(SIGNED_PSBT_FILE)).unwrap();
        })
        .unwrap();
    let witness = witness_tx(&plan, 0).unwrap();
    assert_eq!(witness.compute_txid().to_string(), t0);
    assert!(witness.input.iter().all(|i| i.witness.is_empty()));

    // C2/C3: a send waiting for its recipient, ACKs in any state, expired: the chain decides
    for ack in [None, Some(true), Some(false)] {
        let plan = base
            .plan_after(|txn, transfers| {
                set_status(txn, i0, TransferStatus::WaitingCounterparty);
                update_batch_transfer(txn, i0, |b| {
                    b.expiration = ActiveValue::Set(Some(now().unix_timestamp() - 1))
                });
                for asset_transfer in asset_transfers_of(txn, i0) {
                    for transfer in txn
                        .iter_transfers()
                        .unwrap()
                        .into_iter()
                        .filter(|t| t.asset_transfer_idx == asset_transfer.idx)
                    {
                        let mut transfer: DbTransferActMod = transfer.into();
                        transfer.ack = ActiveValue::Set(ack);
                        txn.update_transfer(&mut transfer).unwrap();
                    }
                }
                rewrite_info(transfers, &t0, |info| info.donation = false);
            })
            .unwrap();
        assert_matches!(
            &plan.spends[0].record,
            PlannedRecord::Send { donation: false, batch_transfer, .. }
                if batch_transfer.status == TransferStatus::WaitingCounterparty
        );
        // a non-donation keeps the stored fascia
        assert!(
            witness_tx(&plan, 0)
                .unwrap()
                .input
                .iter()
                .all(|i| i.witness.is_empty())
        );
    }

    // a coin nothing spends: unexplained, so nothing is completed and nothing is asked
    let fake_txo = |txn: &DbTxn, _: &Path| {
        txn.set_txo(DbTxoActMod {
            txid: ActiveValue::Set(FAKE_TXID.to_string()),
            vout: ActiveValue::Set(7),
            btc_amount: ActiveValue::Set(s!("1000")),
            spent: ActiveValue::Set(false),
            exists: ActiveValue::Set(true),
            pending_witness: ActiveValue::Set(false),
            ..Default::default()
        })
        .unwrap();
    };
    let (result, asked) = base.plan(fake_txo, |_, lookup| {
        lookup.answers.insert(
            t0.clone(),
            Err(Error::Indexer {
                details: s!("down"),
            }),
        );
    });
    let error = result.err().unwrap();
    assert_eq!(
        error.inconsistency_reason(),
        Some(InconsistencyReason::NoCanonicalSpender)
    );
    assert_matches!(&error, Error::Inconsistency { details }
        if details.starts_with(&base.details) && details.contains(&t0) && details.contains(&t1));
    assert!(asked.is_empty());

    // a spender only an incoming batch transfer names is not this wallet's spend
    let error = base
        .plan_after(|txn, _| {
            update_batch_transfer(txn, i1, |b| b.incoming = ActiveValue::Set(true))
        })
        .err()
        .unwrap();
    assert_eq!(
        error.inconsistency_reason(),
        Some(InconsistencyReason::SpenderNotRecorded)
    );

    // an output of an own spend spent by a TX the wallet has no record of; allowed when the
    // database has it spent (a send-to-self receive, already spent)
    let change_of_t0 = |view: &mut SpendView| {
        let outpoint = BdkOutPoint::new(bdk_wallet::bitcoin::Txid::from_str(&t0).unwrap(), 9);
        view.colored_outputs
            .get_mut(&t0)
            .unwrap()
            .push((outpoint, true));
        outpoint
    };
    let (result, _) = base.plan(
        |_, _| {},
        |view, _| {
            change_of_t0(view);
        },
    );
    assert_eq!(
        result.err().unwrap().inconsistency_reason(),
        Some(InconsistencyReason::LaterSpendUnrecorded)
    );
    let (result, _) = base.plan(
        |txn, _| {
            txn.set_txo(DbTxoActMod {
                txid: ActiveValue::Set(t0.clone()),
                vout: ActiveValue::Set(9),
                btc_amount: ActiveValue::Set(s!("1000")),
                spent: ActiveValue::Set(true),
                exists: ActiveValue::Set(true),
                pending_witness: ActiveValue::Set(false),
                ..Default::default()
            })
            .unwrap();
        },
        |view, _| {
            change_of_t0(view);
        },
    );
    assert_eq!(result.unwrap().spends.len(), 2);

    // the indexer: a failed lookup is its error, an unknown TX is Unseen, and a failed lookup
    // wins over an unknown TX whatever their order
    let (result, _) = base.plan(
        |_, _| {},
        |_, lookup| {
            lookup
                .answers
                .insert(t1.clone(), Err(Error::Indexer { details: s!("502") }));
        },
    );
    assert_eq!(result.err().unwrap(), Error::Indexer { details: s!("502") });
    let (result, _) = base.plan(
        |_, _| {},
        |_, lookup| {
            lookup.answers.insert(t1.clone(), Ok(None));
        },
    );
    assert_eq!(
        result.err().unwrap(),
        Error::UnrecordedSpendUnseen {
            txid: t1.clone(),
            batch_transfer_idx: Some(i1),
        }
    );
    let (result, asked) = base.plan(
        |_, _| {},
        |_, lookup| {
            lookup.answers.insert(t0.clone(), Ok(None));
            lookup
                .answers
                .insert(t1.clone(), Err(Error::Indexer { details: s!("502") }));
        },
    );
    assert_matches!(result, Err(Error::Indexer { .. }));
    assert_eq!(asked.len(), 2);
    // Unseen wins over a record that cannot be completed
    let (result, _) = base.plan(
        |txn, _| set_status(txn, i0, TransferStatus::Failed),
        |_, lookup| {
            lookup.answers.insert(t1.clone(), Ok(None));
        },
    );
    assert_matches!(result, Err(Error::UnrecordedSpendUnseen { txid, .. }) if txid == t1);

    // statuses
    assert_eq!(
        base.plan_after(|txn, _| set_status(txn, i0, TransferStatus::Failed))
            .err()
            .unwrap(),
        unrecorded(&t0, TransferFailed, Some(i0))
    );
    for status in [
        TransferStatus::WaitingConfirmations,
        TransferStatus::Settled,
    ] {
        assert_eq!(
            base.plan_after(|txn, _| set_status(txn, i0, status))
                .err()
                .unwrap(),
            unrecorded(&t0, UnexpectedStatus, Some(i0))
        );
    }

    // more than one record
    let duplicate = |txn: &DbTxn, _: &Path| {
        let original = batch_transfer(txn, i0);
        txn.set_batch_transfer(DbBatchTransferActMod {
            txid: ActiveValue::Set(original.txid),
            status: ActiveValue::Set(TransferStatus::Initiated),
            created_at: ActiveValue::Set(original.created_at),
            expiration: ActiveValue::Set(original.expiration),
            min_confirmations: ActiveValue::Set(original.min_confirmations),
            incoming: ActiveValue::Set(false),
            ..Default::default()
        })
        .unwrap();
    };
    assert_eq!(
        base.plan_after(duplicate).err().unwrap(),
        unrecorded(&t0, RecordAmbiguous, None)
    );
    assert_eq!(
        base.plan_after(|txn, _| {
            txn.set_wallet_transaction(DbWalletTransactionActMod {
                txid: ActiveValue::Set(t0.clone()),
                r#type: ActiveValue::Set(WalletTransactionType::Drain),
                ..Default::default()
            })
            .unwrap();
        })
        .err()
        .unwrap(),
        unrecorded(&t0, RecordAmbiguous, None)
    );

    // kinds the completion does not handle
    assert_eq!(
        base.plan_after(|_, transfers| {
            rewrite_info(transfers, &t0, |info| {
                info.transfers
                    .values_mut()
                    .for_each(|t| t.main_transition = TypeOfTransition::Inflate)
            })
        })
        .err()
        .unwrap(),
        unrecorded(&t0, RecordKindUnsupported, Some(i0))
    );
    assert_eq!(
        base.plan_after(|txn, _| {
            update_batch_transfer(txn, i0, |b| b.incoming = ActiveValue::Set(true));
            txn.set_wallet_transaction(DbWalletTransactionActMod {
                txid: ActiveValue::Set(t0.clone()),
                r#type: ActiveValue::Set(WalletTransactionType::CreateUtxos),
                ..Default::default()
            })
            .unwrap();
        })
        .err()
        .unwrap(),
        unrecorded(&t0, RecordKindUnsupported, None)
    );

    // missing or unreadable transfer data
    for damage in [
        |dir: &Path| fs::remove_dir_all(dir).unwrap(),
        |dir: &Path| fs::remove_file(dir.join(TRANSFER_DATA_FILE)).unwrap(),
        |dir: &Path| fs::remove_file(dir.join("fascia")).unwrap(),
        |dir: &Path| fs::write(dir.join("fascia"), "{").unwrap(),
        |dir: &Path| fs::write(dir.join(TRANSFER_DATA_FILE), "[]").unwrap(),
        |dir: &Path| fs::write(dir.join(SIGNED_PSBT_FILE), "not a PSBT").unwrap(),
    ] {
        assert_eq!(
            base.plan_after(|_, transfers| damage(&transfers.join(&t0)))
                .err()
                .unwrap(),
            unrecorded(&t0, TransferDataMissing, Some(i0))
        );
    }

    // files and rows that do not match the TX
    let other_file = |name: &'static str| {
        let (t0, t1) = (t0.clone(), t1.clone());
        move |_: &DbTxn, transfers: &Path| {
            fs::copy(
                transfers.join(&t1).join(name),
                transfers.join(&t0).join(name),
            )
            .unwrap();
        }
    };
    for change in [other_file("fascia"), other_file(SIGNED_PSBT_FILE)] {
        assert_eq!(
            base.plan_after(change).err().unwrap(),
            unrecorded(&t0, RecordMismatch, Some(i0))
        );
    }
    // the signed PSBT of another TX in a send that is not a donation (not used as a witness, still
    // refused: it is what try_complete_batch would broadcast)
    let psbt_of_t1 = other_file(SIGNED_PSBT_FILE);
    assert_eq!(
        base.plan_after(|txn, transfers| {
            psbt_of_t1(txn, transfers);
            rewrite_info(transfers, &t0, |info| info.donation = false);
        })
        .err()
        .unwrap(),
        unrecorded(&t0, RecordMismatch, Some(i0))
    );
    // an asset the fascia does not move
    assert_eq!(
        base.plan_after(|txn, _| {
            txn.set_asset_transfer(DbAssetTransferActMod {
                user_driven: ActiveValue::Set(false),
                batch_transfer_idx: ActiveValue::Set(i0),
                asset_id: ActiveValue::Set(
                    txn.get_asset_ids()
                        .unwrap()
                        .into_iter()
                        .find(|a| *a != base.party.asset_id),
                ),
                ..Default::default()
            })
            .unwrap();
        })
        .err()
        .unwrap(),
        unrecorded(&t0, RecordMismatch, Some(i0))
    );
    // a coin the batch transfer spends that is not an input of the TX (the other spend's)
    assert_eq!(
        base.plan_after(|txn, _| {
            let other_input = txn
                .iter_colorings()
                .unwrap()
                .into_iter()
                .find(|c| {
                    c.r#type == ColoringType::Input
                        && asset_transfers_of(txn, i1)
                            .iter()
                            .any(|a| a.idx == c.asset_transfer_idx)
                })
                .unwrap()
                .txo_idx;
            txn.set_coloring(DbColoringActMod {
                txo_idx: ActiveValue::Set(other_input),
                asset_transfer_idx: ActiveValue::Set(asset_transfers_of(txn, i0)[0].idx),
                r#type: ActiveValue::Set(ColoringType::Input),
                assignment: ActiveValue::Set(Assignment::Fungible(1)),
                ..Default::default()
            })
            .unwrap();
        })
        .err()
        .unwrap(),
        unrecorded(&t0, RecordMismatch, Some(i0))
    );
}

// the assets of a batch transfer include those moved along from the spent coins (extra
// allocations), and its fascia carries a bundle for each (design question 7)
#[test]
#[parallel]
fn plan_counts_the_assets_moved_along() {
    let chain = ScriptedChain::start();
    // one colored UTXO: both assets are issued on it
    let (wallet, online) = funded(&chain, 1);
    let asset_id = issue(&wallet, vec![AMOUNT]);
    let moved_along = issue(&wallet, vec![AMOUNT]);
    let mut party = Issuer {
        wallet,
        online,
        asset_id,
    };
    let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
    let txid = psbt_txid(&signed);
    lose_the_answer(&chain, &txid);
    let result = party.wallet.send_end(party.online, signed.clone());
    assert_matches!(result, Err(Error::Indexer { .. }));
    let base = Base::scan(chain, party, vec![(txid, begin.batch_transfer_idx, signed)]);

    let plan = base.plan_after(|_, _| {}).unwrap();
    let PlannedRecord::Send { fascia, .. } = &plan.spends[0].record else {
        panic!("a send")
    };
    let contracts: BTreeSet<String> = fascia.bundles().keys().map(|c| c.to_string()).collect();
    assert_eq!(
        contracts,
        BTreeSet::from([base.party.asset_id.clone(), moved_along.clone()])
    );
    // without the asset moved along, the batch transfer does not match its fascia
    let result = base.plan_after(|txn, _| {
        let moved = asset_transfers_of(txn, base.idx(0))
            .into_iter()
            .find(|a| a.asset_id.as_deref() == Some(moved_along.as_str()))
            .unwrap();
        let mut moved: DbAssetTransferActMod = moved.into();
        moved.asset_id = ActiveValue::Set(Some(base.party.asset_id.clone()));
        txn.update_asset_transfer(&mut moved).unwrap();
    });
    assert_eq!(
        result.err().unwrap(),
        unrecorded(
            base.txid(0),
            UnrecordedSpendReason::RecordMismatch,
            Some(base.idx(0))
        )
    );
}

#[test]
#[parallel]
fn plan_table_of_a_drain() {
    let chain = ScriptedChain::start();
    let (wallet, online) = funded(&chain, UTXOS);
    let mut party = Issuer {
        wallet,
        online,
        asset_id: s!(""),
    };
    let address = get_test_wallet(false, None).get_address().unwrap();
    let psbt = party
        .wallet
        .drain_to_begin(party.online, address, FEE_RATE, false)
        .unwrap();
    let signed = party.wallet.sign_psbt(psbt, None).unwrap();
    let txid = psbt_txid(&signed);
    lose_the_answer(&chain, &txid);
    let result = party.wallet.drain_to_end(party.online, signed.clone());
    assert_matches!(result, Err(Error::Indexer { .. }));
    let base = Base::scan(chain, party, vec![(txid.clone(), None, signed)]);

    let plan = base.plan_after(|_, _| {}).unwrap();
    assert_matches!(&plan.spends[0].record, PlannedRecord::Drain { reservations } if reservations.len() == plan.spends[0].tx.input.len());

    let reservations = |txn: &DbTxn| {
        txn.get_wallet_transaction_with_reserved_txos_by_txid(&txid)
            .unwrap()
            .unwrap()
    };
    // reservations that are not the TX's inputs
    assert_eq!(
        base.plan_after(|txn, _| {
            let (_, reserved) = reservations(txn);
            txn.del_reserved_txos(&reserved[..1]).unwrap();
        })
        .err()
        .unwrap(),
        unrecorded(&txid, UnrecordedSpendReason::RecordMismatch, None)
    );
    // a drain that is no longer pending
    assert_eq!(
        base.plan_after(|txn, _| {
            let (_, reserved) = reservations(txn);
            txn.del_reserved_txos(&reserved).unwrap();
        })
        .err()
        .unwrap(),
        unrecorded(&txid, UnrecordedSpendReason::UnexpectedStatus, None)
    );
    // a vanilla TX of another kind
    assert_eq!(
        base.plan_after(|txn, _| {
            let (wallet_transaction, reserved) = reservations(txn);
            txn.del_wallet_transaction(wallet_transaction.idx).unwrap();
            let idx = txn
                .set_wallet_transaction(DbWalletTransactionActMod {
                    txid: ActiveValue::Set(txid.clone()),
                    r#type: ActiveValue::Set(WalletTransactionType::SendBtc),
                    ..Default::default()
                })
                .unwrap();
            txn.set_reserved_txos(
                reserved
                    .into_iter()
                    .map(|r| DbReservedTxoActMod {
                        txid: ActiveValue::Set(r.txid),
                        vout: ActiveValue::Set(r.vout),
                        reserved_for: ActiveValue::Set(Some(idx)),
                        ..Default::default()
                    })
                    .collect(),
            )
            .unwrap();
        })
        .err()
        .unwrap(),
        unrecorded(&txid, UnrecordedSpendReason::RecordKindUnsupported, None)
    );
}

// P7: whatever the plan, the completion never reports success while a divergence is left (here a
// plan that covers nothing)
#[test]
#[parallel]
fn apply_refuses_to_leave_a_divergence() {
    let mut base = Base::two_donations();
    let txn = base.party.wallet.database().begin_transaction().unwrap();
    let mut runtime = base.party.wallet.rgb_runtime().unwrap();
    let result = crate::wallet::unrecorded_spends::apply(
        &mut base.party.wallet,
        &txn,
        &mut runtime,
        Plan {
            spends: vec![],
            gone_envelopes: vec![],
        },
    );
    assert_matches!(result, Err(Error::Internal { details }) if details.contains("left a divergence"));
}

// the record the bridge's drain guard reads: none for a dry run or an aborted drain, pending
// until the drain ends or is completed, kept after; the same signed PSBT ended again after a
// completion answers with its TX
#[test]
#[parallel]
fn a_drain_record_says_whether_it_is_pending() {
    let chain = ScriptedChain::start();
    let (wallet, online) = funded(&chain, UTXOS);
    let mut party = Issuer {
        wallet,
        online,
        asset_id: s!(""),
    };
    let address = get_test_wallet(false, None).get_address().unwrap();
    let dry_run = party
        .wallet
        .drain_to_begin(party.online, address.clone(), FEE_RATE, true)
        .unwrap();
    assert_eq!(
        party.wallet.vanilla_tx_record(psbt_txid(&dry_run)).unwrap(),
        None
    );
    let aborted = party
        .wallet
        .drain_to_begin(party.online, address.clone(), FEE_RATE, false)
        .unwrap();
    party
        .wallet
        .abort_pending_vanilla_tx(psbt_txid(&aborted))
        .unwrap();
    assert_eq!(
        party.wallet.vanilla_tx_record(psbt_txid(&aborted)).unwrap(),
        None
    );

    let psbt = party
        .wallet
        .drain_to_begin(party.online, address, FEE_RATE, false)
        .unwrap();
    let signed = party.wallet.sign_psbt(psbt, None).unwrap();
    let txid = psbt_txid(&signed);
    let record = |party: &Issuer| party.wallet.vanilla_tx_record(txid.clone()).unwrap();
    let pending = |pending| {
        Some(VanillaTxRecord {
            txid: txid.clone(),
            r#type: WalletTransactionType::Drain,
            pending,
        })
    };
    assert_eq!(record(&party), pending(true));
    lose_the_answer(&chain, &txid);
    let result = party.wallet.drain_to_end(party.online, signed.clone());
    assert_matches!(result, Err(Error::Indexer { .. }));
    assert_eq!(record(&party), pending(true));

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(party.wallet.completed_spends().len(), 1);
    assert_eq!(record(&party), pending(false));
    assert_eq!(
        party.wallet.drain_to_end(party.online, signed).unwrap(),
        txid
    );
    assert_eq!(record(&party), pending(false));
    assert_eq!(
        party
            .wallet
            .vanilla_tx_record(FAKE_TXID.to_string())
            .unwrap(),
        None
    );
}

// CC-101: a stash the disk refuses while a spend is completed is an I/O error, which a retry can
// get past, not the stash's verdict on the transitions; the next go_online completes the spend
#[test]
#[parallel]
fn a_stash_the_disk_refuses_is_an_io_error() {
    use crate::stock_store::{InjectedFailure, STORE_FAILURES};
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (idx, _) = donation_answer_lost(&chain, &mut party);
    STORE_FAILURES
        .with_borrow_mut(|f| f.push((s!("stash.dat"), InjectedFailure::HalfWritten, usize::MAX)));
    let result = reopen(&chain, &mut party, completing_options(&chain));
    STORE_FAILURES.with_borrow_mut(|f| f.clear());
    assert_matches!(result, Err(Error::IO { details }) if details.contains("stash.dat"));
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(party.wallet.completed_spends().len(), 1);
}

// CC-101: an issuance whose contract the disk refuses is an I/O error, not a panic
#[test]
#[parallel]
fn an_issuance_the_disk_refuses_is_an_io_error() {
    use crate::stock_store::{InjectedFailure, STORE_FAILURES};
    let chain = ScriptedChain::start();
    let (wallet, _online) = funded(&chain, UTXOS);
    STORE_FAILURES
        .with_borrow_mut(|f| f.push((s!("stash.dat"), InjectedFailure::HalfWritten, usize::MAX)));
    let result = wallet.issue_asset_nia(
        TICKER.to_string(),
        NAME.to_string(),
        PRECISION,
        vec![AMOUNT],
    );
    STORE_FAILURES.with_borrow_mut(|f| f.clear());
    assert_matches!(result, Err(Error::IO { .. }));
    assert!(
        wallet
            .list_assets(vec![])
            .unwrap()
            .nia
            .unwrap_or_default()
            .is_empty()
    );
}

// the report is the last go_online's, even when that call failed before its check: a refused
// forwarder URL and a failed probe of a new indexer both leave the online state in place (the
// second keeps the previous session usable), and neither may leave the previous report with it
#[test]
#[parallel]
fn a_failed_go_online_leaves_no_report() {
    for refused_forwarder in [true, false] {
        let chain = ScriptedChain::start();
        let mut party = issuer(&chain, vec![AMOUNT]);
        let (_, _) = donation_answer_lost(&chain, &mut party);
        reopen(&chain, &mut party, completing_options(&chain)).unwrap();
        assert_eq!(party.wallet.completed_spends().len(), 1);
        let failing = if refused_forwarder {
            OnlineOptions {
                forwarder_url: Some(s!("http://localhost:1/secret/rgb")),
                ..completing_options(&chain)
            }
        } else {
            OnlineOptions {
                indexer_url: s!("http://127.0.0.1:1"),
                ..completing_options(&chain)
            }
        };
        let result = party.wallet.go_online(failing);
        if refused_forwarder {
            assert_matches!(result, Err(Error::InvalidForwarderUrl { .. }));
        } else {
            assert_matches!(result, Err(Error::InvalidIndexer { .. }));
            // upstream stays online on the previous indexer
            party
                .wallet
                .list_unspents(Some(party.online), false, false)
                .unwrap();
        }
        assert!(party.wallet.completed_spends().is_empty());
    }
}

// the text of a failed lookup stays out of the wallet's log: a forwarder's error page names the
// path it was asked for, and the path carries the session's secret
#[test]
#[parallel]
fn a_failed_lookup_leaves_its_text_out_of_the_log() {
    const SECRET: &str = "5ec2e7a1b0c4d9f86e3a";
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (_, txid) = donation_answer_lost(&chain, &mut party);
    let path = format!("/tx/{txid}/status");
    chain.fault_answering(
        "GET",
        &path,
        502,
        false,
        1,
        &format!("no upstream for /{SECRET}/esplora{path}"),
    );
    let result = reopen(&chain, &mut party, completing_options(&chain));
    // the caller gets the error as it came (the host scrubs what it passes on)
    assert_matches!(result, Err(Error::Indexer { details }) if details.contains(SECRET));
    let log_path = party.wallet.get_wallet_dir().join("log");
    // the log is written by a thread of its own, flushed when the wallet goes
    drop(party);
    let log = fs::read_to_string(log_path).unwrap();
    assert!(log.contains("CC-99: not completing the spends of divergent coins"));
    assert!(!log.contains(SECRET));
}

// two spends in one plan, the stash refusing one of them: rgb-ops commits each fascia on its own,
// so a spend consumed before the refused one stays in the stash while the database rolls back
// (S2's state), and one refused first leaves the stash as it was. Either way the refusal is the
// same on every go_online while the damage lasts, and once the fascia is whole the next one
// completes both, skipping what the stash holds
#[test]
#[parallel]
fn a_stash_refusal_keeps_the_spends_consumed_before_it() {
    for refused in [1, 0] {
        let chain = ScriptedChain::start();
        let (wallet, online) = funded(&chain, 8);
        let asset_id = issue(&wallet, vec![AMOUNT, AMOUNT]);
        let mut party = Issuer {
            wallet,
            online,
            asset_id,
        };
        let mut spends = vec![];
        for _ in 0..2 {
            let (begin, signed) = begin_two_asset_donation(&chain, &mut party);
            let txid = psbt_txid(&signed);
            lose_the_answer(&chain, &txid);
            let result = party.wallet.send_end(party.online, signed);
            assert_matches!(result, Err(Error::Indexer { .. }));
            spends.push((txid, begin.batch_transfer_idx.unwrap()));
        }
        // the plan consumes them in TXID order
        spends.sort();
        let fascia_path = party
            .wallet
            .get_transfers_dir()
            .join(&spends[refused].0)
            .join("fascia");
        let whole = fs::read_to_string(&fascia_path).unwrap();
        damage_second_bundle(&party, &spends[refused].0);
        let db_digest = |party: &Issuer| {
            state_digest(party)
                .lines()
                .filter(|l| l.ends_with("/rgb_lib_db"))
                .collect::<String>()
        };
        let (db_before, stash_before) = (db_digest(&party), stash_digest(&party));

        let refusal = Error::UnrecordedSpend {
            txid: spends[refused].0.clone(),
            reason: s!("stash-refused"),
            batch_transfer_idx: Some(spends[refused].1),
        };
        let result = reopen(&chain, &mut party, completing_options(&chain));
        assert_eq!(result.unwrap_err(), refusal);
        assert_eq!(db_digest(&party), db_before);
        for (_, idx) in &spends {
            assert_eq!(status_of(&party.wallet, *idx), TransferStatus::Initiated);
        }
        assert!(stash_witness(&party, &spends[refused].0).is_none());
        if refused == 1 {
            assert!(is_signed(&stash_witness(&party, &spends[0].0)));
        } else {
            assert_eq!(stash_digest(&party), stash_before);
        }
        let result = reopen(&chain, &mut party, completing_options(&chain));
        assert_eq!(result.unwrap_err(), refusal);

        fs::write(&fascia_path, whole).unwrap();
        reopen(&chain, &mut party, completing_options(&chain)).unwrap();
        assert_eq!(party.wallet.completed_spends().len(), 2);
        for (txid, idx) in &spends {
            assert!(is_signed(&stash_witness(&party, txid)));
            assert_eq!(
                status_of(&party.wallet, *idx),
                TransferStatus::WaitingConfirmations
            );
        }
        settle(&chain, &mut party);
        assert_eq!(
            spendable(&party.wallet, &party.asset_id),
            2 * AMOUNT - 2 * AMOUNT_SMALL
        );
    }
}

// P1, CC-27 offline: this wallet's own TX spent a coin and left every mempool, while another
// install of the same wallet spent that coin with another TX. BDK's graph holds both; only the
// canonical one spends the coin, and it is not this wallet's record, so the check refuses as an
// unrecorded spend (not as a spend of its own the indexer does not know yet, which would be
// retried forever)
#[test]
#[parallel]
fn a_conflicting_spend_of_another_install_is_not_completed() {
    for mined in [false, true] {
        let chain = ScriptedChain::start();
        let mut party = issuer(&chain, vec![AMOUNT]);
        let (mut other, _other_dir) = control(&party);
        other.online = other.wallet.go_online(online_options(&chain)).unwrap();

        let (_, signed_a, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
        let txid_a = psbt_txid(&signed_a);
        MOCK_SEND_END_CRASH.replace(Some(()));
        let crashed = party.wallet.send_end(party.online, signed_a);
        assert_matches!(crashed, Err(Error::Internal { .. }));
        chain.evict(&txid_a);

        let (_, signed_b, _) = begin_send(&chain, &mut other, AMOUNT_SMALL * 2, true);
        let txid_b = psbt_txid(&signed_b);
        other.wallet.send_end(other.online, signed_b).unwrap();
        assert!(chain.knows(&txid_b));
        if mined {
            chain.mine(1);
        }

        let digest = state_digest(&party);
        let error = reopen(&chain, &mut party, completing_options(&chain)).unwrap_err();
        assert_eq!(
            error.inconsistency_reason(),
            Some(InconsistencyReason::SpenderNotRecorded),
            "{error:?}"
        );
        assert_matches!(&error, Error::Inconsistency { details } if details.contains(&txid_b));
        assert_eq!(state_digest(&party), digest);

        let txn = party.wallet.database().begin_transaction().unwrap();
        let bdk_wallet = party.wallet.bdk_wallet();
        let divergence = colored_divergence(bdk_wallet, &txn).unwrap();
        let view = SpendView::from_bdk(bdk_wallet, &divergence);
        assert!(
            bdk_wallet
                .tx_graph()
                .get_tx(bdk_wallet::bitcoin::Txid::from_str(&txid_a).unwrap())
                .is_some()
        );
        assert_eq!(view.txs.keys().cloned().collect::<Vec<_>>(), vec![txid_b]);
    }
}

// a completion beside a send of this wallet that is pending and not broadcast (its coins exist
// and are unspent everywhere): only the lost spend is completed, and both settle
#[test]
#[parallel]
fn a_completion_beside_a_pending_send() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT, AMOUNT]);
    let (pending, signed, recipient) = begin_send(&chain, &mut party, AMOUNT_SMALL, false);
    let pending_idx = pending.batch_transfer_idx.unwrap();
    let pending_txid = psbt_txid(&signed);
    party.wallet.send_end(party.online, signed).unwrap();
    assert_eq!(
        status_of(&party.wallet, pending_idx),
        TransferStatus::WaitingCounterparty
    );
    let (idx, txid) = donation_answer_lost(&chain, &mut party);

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(
        party
            .wallet
            .completed_spends()
            .into_iter()
            .map(|c| c.txid)
            .collect::<Vec<_>>(),
        vec![txid]
    );
    assert_eq!(
        status_of(&party.wallet, pending_idx),
        TransferStatus::WaitingCounterparty
    );
    chain.set_ack(&recipient.recipient_id, true);
    party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    assert!(chain.knows(&pending_txid));
    settle(&chain, &mut party);
    settle(&chain, &mut party);
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
    assert_eq!(
        status_of(&party.wallet, pending_idx),
        TransferStatus::Settled
    );
    assert_eq!(
        spendable(&party.wallet, &party.asset_id),
        2 * AMOUNT - 2 * AMOUNT_SMALL
    );
}

// the option off, or on with nothing to complete, is upstream's go_online: it asks for no backup
#[test]
#[parallel]
fn a_clean_go_online_asks_for_no_backup() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let backup_dir = tempfile::tempdir().unwrap();
    party
        .wallet
        .backup(
            &backup_dir.path().join("backup").to_string_lossy(),
            PASSWORD,
        )
        .unwrap();
    assert!(!party.wallet.backup_info().unwrap());
    for options in [online_options(&chain), completing_options(&chain)] {
        reopen(&chain, &mut party, options).unwrap();
        assert!(party.wallet.completed_spends().is_empty());
        assert!(!party.wallet.backup_info().unwrap());
    }
}

// the option is off when a host's options do not name it (the C-FFI binding reads them from JSON)
#[test]
#[parallel]
fn the_option_is_off_unless_named() {
    let options: OnlineOptions = serde_json::from_str(
        r#"{"indexer_url":"http://x","skip_consistency_check":false,"vanilla_sync_lookback":1}"#,
    )
    .unwrap();
    assert!(!options.complete_unrecorded_spends);
}

// a transfer file the system refuses to read is the retryable Error::IO, not a refusal of the
// spend: once it can be read, the next go_online completes it
#[cfg(unix)]
#[test]
#[parallel]
fn an_unreadable_transfer_file_is_an_io_error() {
    use std::os::unix::fs::PermissionsExt;
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (idx, txid) = donation_answer_lost(&chain, &mut party);
    let fascia = party.wallet.get_transfers_dir().join(&txid).join("fascia");
    fs::set_permissions(&fascia, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&fascia).is_ok() {
        // running as root
        fs::set_permissions(&fascia, fs::Permissions::from_mode(0o644)).unwrap();
        return;
    }
    let result = reopen(&chain, &mut party, completing_options(&chain));
    fs::set_permissions(&fascia, fs::Permissions::from_mode(0o644)).unwrap();
    assert_matches!(result, Err(Error::IO { .. }));
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);
    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(party.wallet.completed_spends().len(), 1);
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingConfirmations
    );
}

// upstream's asset check refuses a stock without an asset the database has, with the option or
// without it. Since the stock loads only as a whole set, such a stock is a whole one (here a fresh
// wallet's); a missing one is refused before (the upstream tests of this check reach it the same
// way now)
#[test]
#[parallel]
fn the_asset_check_refuses_a_stock_without_the_asset() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let fresh = get_test_wallet(true, None);
    let fresh_rgb_dir = fresh.get_wallet_dir().join(crate::utils::RGB_RUNTIME_DIR);
    drop(fresh);
    let rgb_dir = party
        .wallet
        .get_wallet_dir()
        .join(crate::utils::RGB_RUNTIME_DIR);
    party.wallet.go_offline();
    fs::remove_dir_all(&rgb_dir).unwrap();
    copy_dir(&fresh_rgb_dir, &rgb_dir);
    for options in [online_options(&chain), completing_options(&chain)] {
        let result = party.wallet.go_online(options);
        assert_matches!(
            &result,
            Err(Error::Inconsistency { details })
                if details == "DB assets do not match with ones stored in RGB"
        );
    }
}

// upstream's media check, the last one, still runs after a completion, and still refuses: nothing
// is committed
#[test]
#[parallel]
fn the_media_check_runs_after_a_completion() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (idx, _) = donation_answer_lost(&chain, &mut party);
    let txn = party.wallet.database().begin_transaction().unwrap();
    txn.set_media(DbMediaActMod {
        digest: ActiveValue::Set("ab".repeat(32)),
        mime: ActiveValue::Set(s!("text/plain")),
        ..Default::default()
    })
    .unwrap();
    txn.commit().unwrap();
    let result = reopen(&chain, &mut party, completing_options(&chain));
    assert_matches!(&result, Err(Error::Inconsistency { details }) if details.contains("media"));
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);
    assert!(party.wallet.completed_spends().is_empty());
}

/// Spend the change of the issued asset once more, and settle: the stash knows every transition
/// the change comes from.
fn spend_the_change(chain: &ScriptedChain, party: &mut Issuer, spent_before: u64) {
    let (_, signed, _) = begin_send(chain, party, AMOUNT_SMALL, true);
    let next = party.wallet.send_end(party.online, signed).unwrap();
    assert!(chain.knows(&next.txid));
    settle(chain, party);
    assert_eq!(
        spendable(&party.wallet, &party.asset_id),
        AMOUNT - spent_before - AMOUNT_SMALL
    );
}

// CC-101: one of the three stock files failing to store while the stash consumes a spend, during
// the completion or during send_end itself: Error::IO, the stash no newer than the file that
// failed (it is held back), the next go_online completes the spend, it settles, and its change
// is spent again. Were the stash written after a failed index or state, the completion would
// skip the consume and the change could never be spent.
#[test]
#[parallel]
fn a_stock_file_that_fails_to_store_keeps_the_stash_behind() {
    use crate::stock_store::{InjectedFailure, STORE_FAILURES};
    for in_send_end in [false, true] {
        for name in ["index.dat", "state.dat", "stash.dat"] {
            let chain = ScriptedChain::start();
            let mut party = issuer(&chain, vec![AMOUNT]);
            let stash_path = party.wallet.get_wallet_dir().join("rgb").join("stash.dat");
            let (idx, stash_before) = if in_send_end {
                let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
                let stash_before = fs::read(&stash_path).unwrap();
                STORE_FAILURES.with_borrow_mut(|f| {
                    f.push((name.to_string(), InjectedFailure::HalfWritten, usize::MAX))
                });
                let result = party.wallet.send_end(party.online, signed.clone());
                STORE_FAILURES.with_borrow_mut(|f| f.clear());
                assert!(
                    matches!(result, Err(Error::IO { .. })),
                    "{name}: {result:?}"
                );
                assert!(chain.knows(&psbt_txid(&signed)));
                (begin.batch_transfer_idx.unwrap(), stash_before)
            } else {
                let (idx, _) = donation_answer_lost(&chain, &mut party);
                let stash_before = fs::read(&stash_path).unwrap();
                STORE_FAILURES.with_borrow_mut(|f| {
                    f.push((name.to_string(), InjectedFailure::HalfWritten, usize::MAX))
                });
                let result = reopen(&chain, &mut party, completing_options(&chain));
                STORE_FAILURES.with_borrow_mut(|f| f.clear());
                assert!(
                    matches!(result, Err(Error::IO { .. })),
                    "{name}: {result:?}"
                );
                (idx, stash_before)
            };
            assert_eq!(fs::read(&stash_path).unwrap(), stash_before, "{name}");
            assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);

            reopen(&chain, &mut party, completing_options(&chain)).unwrap();
            assert_eq!(party.wallet.completed_spends().len(), 1, "{name}");
            settle(&chain, &mut party);
            assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
            spend_the_change(&chain, &mut party, AMOUNT_SMALL);
        }
    }
}

// CC-101, second line: a stash that holds a spend's bundle while the index does not (as the
// index and state were written before the stash, a set older than the hold-back could be left
// so) counts as not holding it: the completion consumes again, and the change is spent
#[test]
#[parallel]
fn a_bundle_the_index_does_not_know_is_consumed_again() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (idx, _) = donation_answer_lost(&chain, &mut party);
    let index_path = party.wallet.get_wallet_dir().join("rgb").join("index.dat");
    let old_index = fs::read(&index_path).unwrap();
    // the completion consumes (all three files stored) and does not commit
    MOCK_FAIL_BEFORE_COMPLETION_COMMIT.replace(Some(()));
    let result = reopen(&chain, &mut party, completing_options(&chain));
    assert_matches!(result, Err(Error::Internal { .. }));
    party.wallet.go_offline();
    fs::write(&index_path, old_index).unwrap();

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(party.wallet.completed_spends().len(), 1);
    settle(&chain, &mut party);
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
    spend_the_change(&chain, &mut party, AMOUNT_SMALL);
}

// ERA fork (T1.1b, the second review's probe): after S1 the fast sync and the colored payment
// sync, with an address handed out and paid meanwhile, never ask about the lost donation's change
// address, so BDK does not learn its TX. If the TX then leaves every mempool, go_online goes
// through, as it did before the fork watched colored addresses: had BDK learnt the TX, it would
// keep it (nothing tells it of an eviction), and every go_online would refuse with
// UnrecordedSpendUnseen.
#[test]
#[parallel]
fn a_lost_spend_that_leaves_the_mempool_lets_go_online_through() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (_idx, txid) = donation_answer_lost(&chain, &mut party);

    let address = party.wallet.get_colored_address().unwrap();
    chain.fund(&address, UTXO_SATS as u64);
    party
        .wallet
        .list_unspents(Some(party.online), false, false)
        .unwrap();
    party
        .wallet
        .sync_colored_payments(party.online, vec![address])
        .unwrap();
    let bdk_txid = crate::bitcoin::Txid::from_str(&txid).unwrap();
    assert!(party.wallet.bdk_wallet().get_tx(bdk_txid).is_none());

    chain.evict(&txid);
    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert!(party.wallet.completed_spends().is_empty());
}

// ERA fork (T1.1b, the third review's probe): an envelope paid from outside, still unconfirmed,
// carries an asset and is spent by a donation whose answer is lost. The indexer answers a script
// with the TXs spending from it too, so had the wallet kept watching the paid address until it
// confirmed, the next sync would have taught BDK the lost spend, and once it left the mempool
// every go_online would refuse. A paid address is watched no more: go_online goes through.
#[test]
#[parallel]
fn a_lost_spend_of_an_envelope_paid_from_outside_lets_go_online_through() {
    let chain = ScriptedChain::start();
    let mut wallet = get_test_wallet(true, None);
    let online = wallet.go_online(online_options(&chain)).unwrap();
    chain.fund(&wallet.get_address().unwrap(), FUNDING);
    chain.mine(1);
    let address = wallet.get_colored_address().unwrap();
    let funding = chain.fund(&address, UTXO_SATS as u64).to_string();
    wallet.sync_colored_payments(online, vec![]).unwrap();
    let asset_id = issue(&wallet, vec![AMOUNT]);
    let mut party = Issuer {
        wallet,
        online,
        asset_id,
    };
    let (_idx, txid) = donation_answer_lost(&chain, &mut party);
    assert!(!chain.is_confirmed(&funding));

    party
        .wallet
        .list_unspents(Some(party.online), false, false)
        .unwrap();
    let bdk_txid = crate::bitcoin::Txid::from_str(&txid).unwrap();
    assert!(party.wallet.bdk_wallet().get_tx(bdk_txid).is_none());

    chain.evict(&txid);
    chain.mine(1);
    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
}

// ERA fork (T1.1b, the review's probe): why the host must not use the public sync for that. Its
// orphan reconcile marks the donation's inputs spent with no transition, so the next go_online
// finds no divergence and completes nothing: the donation stays Initiated and its transition
// never reaches the stash
#[test]
#[parallel]
fn the_public_sync_after_s1_preempts_the_completion() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (idx, txid) = donation_answer_lost(&chain, &mut party);
    party
        .wallet
        .sync(
            party.online,
            SyncOptions {
                keychain: SyncKeychain::Colored,
                strategy: SyncStrategy::FullSync,
            },
        )
        .unwrap();
    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert!(party.wallet.completed_spends().is_empty());
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);
    assert!(stash_witness(&party, &txid).is_none());
}

// CC-115: an envelope paid from outside the wallet whose payment is replaced (a fee bump, or
// another wallet on the same seed spending the same coins) before it confirms. Once BDK learns
// the replacement, from go_online's full scan of the colored keychain, it no longer holds the
// first payment, and the rows the colored payment sync recorded are divergent coins that nothing
// spends. One that holds nothing is marked as not existing and comes back with its payment; one
// that holds anything is refused as before.

/// A payment of envelopes from outside: the wallet as the bridge opens it (the completion on, one
/// allocation per UTXO), `count` colored addresses handed out and paid in one TX, which the
/// colored payment sync recorded while unconfirmed.
struct EnvelopePayment {
    chain: ScriptedChain,
    wallet: Wallet,
    online: Online,
    addresses: Vec<String>,
    txid: String,
}

impl EnvelopePayment {
    fn new(count: usize) -> Self {
        let chain = ScriptedChain::start();
        let mut wallet = get_test_wallet(false, Some(1));
        let online = wallet.go_online(completing_options(&chain)).unwrap();
        let addresses: Vec<String> = (0..count)
            .map(|_| wallet.get_colored_address().unwrap())
            .collect();
        let txid = chain.pay(&paying(&addresses, UTXO_SATS as u64)).to_string();
        wallet.sync_colored_payments(online, vec![]).unwrap();
        let mut payment = Self {
            chain,
            wallet,
            online,
            addresses,
            txid: txid.clone(),
        };
        assert_eq!(payment.envelopes(&txid), count);
        payment
    }

    /// Replace the payment by one paying `outputs`. BDK orders unconfirmed conflicts by when it
    /// last saw each, in seconds, a tie going by TXID: the replacement is seen a second later at
    /// least, as it is in the field.
    fn replace(&self, outputs: &[(&str, u64)]) -> String {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        self.chain.replace(&self.txid, outputs).to_string()
    }

    /// The same addresses paid again, for less: a fee bump.
    fn bump(&self) -> String {
        self.replace(&paying(&self.addresses, UTXO_SATS as u64 - 500))
    }

    /// Offline, then online again with the completion on, as the bridge reopens a wallet.
    fn reopen(&mut self) -> Result<(), Error> {
        self.wallet.go_offline();
        self.online = self.wallet.go_online(completing_options(&self.chain))?;
        Ok(())
    }

    /// The RGB-ready UTXOs of `txid` the wallet lists, as the bridge counts them (colorable and
    /// existing).
    fn envelopes(&mut self, txid: &str) -> usize {
        self.wallet
            .list_unspents(None, false, true)
            .unwrap()
            .into_iter()
            .filter(|u| u.utxo.colorable && u.utxo.exists && u.utxo.outpoint.txid == txid)
            .count()
    }

    /// The database rows of the outputs of `txid`.
    fn rows(&self, txid: &str) -> Vec<DbTxo> {
        let txn = self.wallet.database().begin_transaction().unwrap();
        txn.iter_txos()
            .unwrap()
            .into_iter()
            .filter(|t| t.txid == txid)
            .collect()
    }

    /// A blind invoice on an envelope of the wallet: the TXID of the UTXO it took.
    fn invoice(&mut self) -> Result<String, Error> {
        let receive = self.wallet.blind_receive(
            None,
            Assignment::Any,
            default_rcv_expiration(),
            vec![self.chain.proxy_endpoint()],
            MIN_CONFIRMATIONS,
        )?;
        let utxo = self
            .wallet
            .list_transfers(AssetFilter::AnyOrNone, None)
            .unwrap()
            .into_iter()
            .find(|t| t.batch_transfer_idx == receive.batch_transfer_idx)
            .unwrap()
            .receive_utxo
            .unwrap();
        Ok(utxo.txid)
    }

    fn refused_as_no_canonical_spender(&mut self) {
        let error = self.reopen().err().unwrap();
        assert_matches!(error, Error::Inconsistency { .. });
        assert_eq!(
            error.inconsistency_reason(),
            Some(InconsistencyReason::NoCanonicalSpender)
        );
    }
}

fn paying(addresses: &[String], amount: u64) -> Vec<(&str, u64)> {
    addresses.iter().map(|a| (a.as_str(), amount)).collect()
}

// the replaced payment's empty envelopes are marked as not existing and count as envelopes no
// more; the replacement's outputs on the same addresses, which go_online's full scan records, are
// the envelopes now. Upstream's check, without the option, still refuses.
#[test]
#[parallel]
fn a_replaced_payment_of_empty_envelopes_lets_go_online_through() {
    let mut payment = EnvelopePayment::new(2);
    let first = payment.txid.clone();
    let replacement = payment.bump();

    payment.wallet.go_offline();
    let result = payment.wallet.go_online(online_options(&payment.chain));
    assert_matches!(result, Err(Error::Inconsistency { .. }));
    assert_eq!(result.err().unwrap().inconsistency_reason(), None);
    assert!(payment.rows(&first).iter().all(|t| t.exists));

    payment.reopen().unwrap();
    assert!(payment.wallet.completed_spends().is_empty());
    let bdk = payment.wallet.bdk_wallet();
    let first_id = crate::bitcoin::Txid::from_str(&first).unwrap();
    assert!(bdk.get_tx(first_id).is_none() && bdk.tx_graph().get_tx(first_id).is_some());
    let rows = payment.rows(&first);
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|t| !t.exists && !t.spent && !t.pending_witness)
    );
    assert_eq!(payment.envelopes(&first), 0);
    assert_eq!(payment.envelopes(&replacement), 2);
    // one invoice per envelope: the replacement's two, and none on the first payment's
    assert_eq!(payment.invoice().unwrap(), replacement);
    assert_eq!(payment.invoice().unwrap(), replacement);
    assert_matches!(payment.invoice(), Err(Error::InsufficientAllocationSlots));

    // nothing left to mark: the next go_online changes nothing
    payment.reopen().unwrap();
    assert_eq!(payment.rows(&first), rows);
    payment.chain.mine(1);
    payment.reopen().unwrap();
    assert_eq!(payment.envelopes(&replacement), 2);
    payment.chain.assert_all_matched();
}

// a replacement paying other colored addresses of the wallet, within the full scan's stop gap
// (another install on the same seed paying the addresses it handed out): seen, and the first
// payment's envelopes are marked the same way
#[test]
#[parallel]
fn a_payment_replaced_by_one_to_other_addresses_lets_go_online_through() {
    let mut payment = EnvelopePayment::new(1);
    let first = payment.txid.clone();
    let others: Vec<String> = (0..2)
        .map(|_| payment.wallet.get_colored_address().unwrap())
        .collect();
    let replacement = payment.replace(&paying(&others, UTXO_SATS as u64 - 500));

    payment.reopen().unwrap();
    assert!(payment.rows(&first).iter().all(|t| !t.exists));
    assert_eq!(payment.envelopes(&first), 0);
    assert_eq!(payment.envelopes(&replacement), 2);
}

// the limit of the rule: a replacement that pays none of the wallet's scripts never reaches BDK,
// which keeps holding the first payment as canonical (the requests carry no expected TXIDs, so
// nothing tells it of an eviction). No divergence, no refusal, and the envelope stays listed
// although its TX is gone: the host's rule for when a payment's envelopes are ready covers it
// (ERA.md, section 9)
#[test]
#[parallel]
fn a_payment_replaced_out_of_the_wallets_sight_stays_listed() {
    let mut payment = EnvelopePayment::new(1);
    let first = payment.txid.clone();
    let elsewhere = get_test_wallet(false, None).get_address().unwrap();
    payment.replace(&[(elsewhere.as_str(), UTXO_SATS as u64 - 500)]);
    assert!(!payment.chain.knows(&first));

    payment.reopen().unwrap();
    let first_id = crate::bitcoin::Txid::from_str(&first).unwrap();
    assert!(payment.wallet.bdk_wallet().get_tx(first_id).is_some());
    assert_eq!(payment.envelopes(&first), 1);
}

// the payment comes back (the replacement dropped, the first payment broadcast again and mined):
// its rows exist again, the same rows, and the replacement's are marked in turn
#[test]
#[parallel]
fn a_replaced_payment_that_comes_back_brings_its_envelopes_back() {
    let mut payment = EnvelopePayment::new(2);
    let first = payment.txid.clone();
    let rows = payment.rows(&first);
    let replacement = payment.bump();
    payment.reopen().unwrap();
    assert_eq!(payment.envelopes(&first), 0);
    assert_eq!(payment.envelopes(&replacement), 2);

    payment.chain.evict(&replacement);
    payment.chain.readmit(&first);
    payment.chain.mine(1);
    payment.reopen().unwrap();
    assert!(payment.wallet.completed_spends().is_empty());
    assert_eq!(payment.rows(&first), rows);
    assert_eq!(payment.envelopes(&first), 2);
    assert_eq!(payment.envelopes(&replacement), 0);
    assert!(payment.rows(&replacement).iter().all(|t| !t.exists));
    assert_eq!(payment.invoice().unwrap(), first);
    assert_eq!(payment.invoice().unwrap(), first);
    assert_matches!(payment.invoice(), Err(Error::InsufficientAllocationSlots));
    payment.chain.assert_all_matched();
}

// an envelope that holds something is refused as before, nothing committed: a blind invoice
// waiting on it, an asset issued on it, a drain begun on it (once the drain is aborted the
// envelope holds nothing, and go_online goes through)
#[test]
#[parallel]
fn a_replaced_payment_whose_envelope_holds_something_is_refused() {
    // a blind invoice
    let mut payment = EnvelopePayment::new(1);
    let first = payment.txid.clone();
    assert_eq!(payment.invoice().unwrap(), first);
    let replacement = payment.bump();
    payment.refused_as_no_canonical_spender();
    assert!(payment.rows(&first).iter().all(|t| t.exists && !t.spent));
    assert!(payment.rows(&replacement).is_empty());
    payment.refused_as_no_canonical_spender();

    // an asset issued on it
    let mut payment = EnvelopePayment::new(1);
    issue(&payment.wallet, vec![AMOUNT]);
    let first = payment.txid.clone();
    payment.bump();
    payment.refused_as_no_canonical_spender();
    assert!(payment.rows(&first).iter().all(|t| t.exists));

    // a drain begun on it, then aborted
    let mut payment = EnvelopePayment::new(1);
    let first = payment.txid.clone();
    let address = get_test_wallet(false, None).get_address().unwrap();
    let psbt = payment
        .wallet
        .drain_to_begin(payment.online, address, FEE_RATE, false)
        .unwrap();
    payment.bump();
    payment.refused_as_no_canonical_spender();
    assert!(payment.rows(&first).iter().all(|t| t.exists));
    payment
        .wallet
        .abort_pending_vanilla_tx(psbt_txid(&psbt))
        .unwrap();
    payment.reopen().unwrap();
    assert!(payment.rows(&first).iter().all(|t| !t.exists));
}

// a blind invoice on the envelope that failed before the payment was replaced holds nothing, by
// the wallet's own accounting of a UTXO's slots: go_online goes through, the transfer stays failed
#[test]
#[parallel]
fn a_replaced_payment_whose_invoice_failed_lets_go_online_through() {
    let mut payment = EnvelopePayment::new(1);
    let first = payment.txid.clone();
    assert_eq!(payment.invoice().unwrap(), first);
    let idx = payment
        .wallet
        .list_transfers(AssetFilter::AnyOrNone, None)
        .unwrap()[0]
        .batch_transfer_idx;
    assert!(
        payment
            .wallet
            .fail_transfers(payment.online, Some(idx), false, false)
            .unwrap()
    );
    let replacement = payment.bump();

    payment.reopen().unwrap();
    assert!(payment.rows(&first).iter().all(|t| !t.exists));
    assert_eq!(status_of(&payment.wallet, idx), TransferStatus::Failed);
    assert_eq!(payment.invoice().unwrap(), replacement);
}

// with an own spend whose record was lost in the same go_online: one plan, all or nothing. A
// lookup that fails refuses the whole of it and marks nothing; the next go_online completes the
// spend and marks the envelope
#[test]
#[parallel]
fn a_replaced_payment_and_a_lost_spend_go_together() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (idx, txid) = donation_answer_lost(&chain, &mut party);
    let address = party.wallet.get_colored_address().unwrap();
    let first = chain.fund(&address, UTXO_SATS as u64).to_string();
    party
        .wallet
        .sync_colored_payments(party.online, vec![])
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    chain.replace(&first, &[(address.as_str(), UTXO_SATS as u64 - 500)]);
    let exists = |party: &Issuer| {
        let txn = party.wallet.database().begin_transaction().unwrap();
        txn.iter_txos()
            .unwrap()
            .into_iter()
            .find(|t| t.txid == first)
            .unwrap()
            .exists
    };

    chain.fault("GET", &format!("/tx/{txid}/status"), 502, false, 1);
    let result = reopen(&chain, &mut party, completing_options(&chain));
    assert_matches!(result, Err(Error::Indexer { .. }));
    assert!(exists(&party));
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);

    reopen(&chain, &mut party, completing_options(&chain)).unwrap();
    assert_eq!(party.wallet.completed_spends().len(), 1);
    assert!(!exists(&party));
    settle(&chain, &mut party);
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
}

impl Base {
    /// An issuer's envelope paid from outside and recorded unconfirmed, the payment then replaced
    /// by one to the same address: returns the base and the two TXIDs.
    fn replaced_payment() -> (Self, String, String) {
        let chain = ScriptedChain::start();
        let party = issuer(&chain, vec![AMOUNT]);
        let mut wallet = party.wallet;
        let address = wallet.get_colored_address().unwrap();
        let first = chain.fund(&address, UTXO_SATS as u64).to_string();
        wallet.sync_colored_payments(party.online, vec![]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let replacement = chain
            .replace(&first, &[(address.as_str(), UTXO_SATS as u64 - 500)])
            .to_string();
        let party = Issuer { wallet, ..party };
        (Self::scan(chain, party, vec![]), first, replacement)
    }
}

// CC-115 in the planner table: the envelope is set aside only while its TX is one BDK saw and no
// longer holds as canonical and the wallet's records leave it empty; anything else, and any other
// coin nothing spends beside it, is refused as before, with nothing asked
#[test]
#[parallel]
fn plan_table_of_a_replaced_payment() {
    let (base, first, replacement) = Base::replaced_payment();
    let row = |txn: &DbTxn| {
        txn.iter_txos()
            .unwrap()
            .into_iter()
            .find(|t| t.txid == first)
            .unwrap()
    };
    let refused = |(result, asked): (Result<Plan, Error>, Vec<String>)| {
        let error = result.err().unwrap();
        assert_eq!(
            error.inconsistency_reason(),
            Some(InconsistencyReason::NoCanonicalSpender)
        );
        assert!(asked.is_empty());
    };
    let unchanged = |_: &mut SpendView, _: &mut Lookup| {};

    // as it is: nothing to complete, the envelope gone, nothing asked
    let (result, asked) = base.plan(|_, _| {}, unchanged);
    let plan = result.unwrap();
    assert!(plan.spends.is_empty());
    assert_eq!(
        plan.gone_envelopes
            .iter()
            .map(|t| t.outpoint())
            .collect::<Vec<_>>(),
        vec![Outpoint {
            txid: first.clone(),
            vout: 0
        }]
    );
    assert!(asked.is_empty());

    // its TX canonical after all
    refused(base.plan(
        |_, _| {},
        |view, _| {
            assert!(view.non_canonical_creators.contains(&first));
            view.non_canonical_creators.clear();
        },
    ));
    // a witness receive landing on it
    refused(base.plan(
        |txn, _| {
            let mut txo: DbTxoActMod = row(txn).into();
            txo.pending_witness = ActiveValue::Set(true);
            txn.update_txo(txo).unwrap();
        },
        unchanged,
    ));
    // reserved (a drain or another vanilla TX begun on it)
    refused(base.plan(
        |txn, _| {
            txn.set_reserved_txos(vec![DbReservedTxoActMod {
                txid: ActiveValue::Set(first.clone()),
                vout: ActiveValue::Set(0),
                reserved_for: ActiveValue::Set(None),
                ..Default::default()
            }])
            .unwrap();
        },
        unchanged,
    ));
    // an allocation on it (the issuance's, moved there); one of a failed transfer holds nothing
    let allocate = |txn: &DbTxn| {
        let issuance = txn
            .iter_asset_transfers()
            .unwrap()
            .into_iter()
            .find(|a| a.asset_id.as_deref() == Some(base.party.asset_id.as_str()))
            .unwrap();
        txn.set_coloring(DbColoringActMod {
            txo_idx: ActiveValue::Set(row(txn).idx),
            asset_transfer_idx: ActiveValue::Set(issuance.idx),
            r#type: ActiveValue::Set(ColoringType::Issue),
            assignment: ActiveValue::Set(Assignment::Fungible(AMOUNT)),
            ..Default::default()
        })
        .unwrap();
        issuance.batch_transfer_idx
    };
    refused(base.plan(
        |txn, _| {
            allocate(txn);
        },
        unchanged,
    ));
    let plan = base
        .plan_after(|txn, _| {
            let issuance = allocate(txn);
            set_status(txn, issuance, TransferStatus::Failed);
        })
        .unwrap();
    assert_eq!(plan.gone_envelopes.len(), 1);
    // its TX named by a transfer of this wallet still in play (the issuance's batch transfer,
    // given that TXID: a send's change is named so); one that failed does not count
    let name_it = |txn: &DbTxn, status: TransferStatus| {
        let issuance = txn
            .iter_asset_transfers()
            .unwrap()
            .into_iter()
            .find(|a| a.asset_id.as_deref() == Some(base.party.asset_id.as_str()))
            .unwrap()
            .batch_transfer_idx;
        update_batch_transfer(txn, issuance, |b| {
            b.txid = ActiveValue::Set(Some(first.clone()));
            b.status = ActiveValue::Set(status);
        });
    };
    for status in [
        TransferStatus::WaitingConfirmations,
        TransferStatus::Settled,
    ] {
        refused(base.plan(|txn, _| name_it(txn, status), unchanged));
    }
    let plan = base
        .plan_after(|txn, _| name_it(txn, TransferStatus::Failed))
        .unwrap();
    assert_eq!(plan.gone_envelopes.len(), 1);
    // beside it, a row of a TX BDK holds as canonical (the replacement) that BDK does not list
    // and nothing spends; or one of a TX BDK never saw
    for txid in [replacement.as_str(), FAKE_TXID] {
        refused(base.plan(
            |txn, _| {
                txn.set_txo(DbTxoActMod {
                    txid: ActiveValue::Set(txid.to_string()),
                    vout: ActiveValue::Set(9),
                    btc_amount: ActiveValue::Set(s!("1000")),
                    spent: ActiveValue::Set(false),
                    exists: ActiveValue::Set(true),
                    pending_witness: ActiveValue::Set(false),
                    ..Default::default()
                })
                .unwrap();
            },
            unchanged,
        ));
    }
}

// From the review of the CC-115 change (its probes, made regression tests)

// the BTC change of an own send that moved a whole allocation goes to a new colored address and
// holds nothing, while the transfer waits on the send's TX. When another install spending the
// same coins replaces that TX, the change is no envelope of a payment from outside: go_online
// refuses as before, rather than leave the transfer waiting for a TX that will not come
#[test]
#[parallel]
fn a_send_whose_tx_is_replaced_is_still_refused() {
    let chain = ScriptedChain::start();
    let mut party = issuer(&chain, vec![AMOUNT]);
    let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT, true);
    let idx = begin.batch_transfer_idx.unwrap();
    let txid = psbt_txid(&signed);
    party.wallet.send_end(party.online, signed).unwrap();
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingConfirmations
    );
    let rows = |party: &Issuer| -> Vec<DbTxo> {
        let txn = party.wallet.database().begin_transaction().unwrap();
        txn.iter_txos()
            .unwrap()
            .into_iter()
            .filter(|t| t.txid == txid)
            .collect()
    };
    let change = rows(&party);
    assert!(!change.is_empty() && change.iter().all(|t| t.exists && !t.spent));
    assert!(
        party
            .wallet
            .list_unspents(None, false, true)
            .unwrap()
            .iter()
            .filter(|u| u.utxo.outpoint.txid == txid)
            .all(|u| u.rgb_allocations.is_empty())
    );
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let address = party.wallet.get_colored_address().unwrap();
    chain.replace(&txid, &[(address.as_str(), 5_000)]);

    let error = reopen(&chain, &mut party, completing_options(&chain))
        .err()
        .unwrap();
    assert_eq!(
        error.inconsistency_reason(),
        Some(InconsistencyReason::NoCanonicalSpender)
    );
    assert_eq!(rows(&party), change);
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingConfirmations
    );
}

// a vanilla record does not refuse: the envelopes create_utxos made, its TX replaced by another
// install spending the same coins (BDK learns that spend from the vanilla full scan), are marked
// like those of a payment from outside; nothing waits on that TX once create_utxos ended
#[test]
#[parallel]
fn the_envelopes_of_a_replaced_create_utxos_are_marked_too() {
    let chain = ScriptedChain::start();
    let mut wallet = get_test_wallet(true, Some(1));
    let online = wallet.go_online(completing_options(&chain)).unwrap();
    chain.fund(&wallet.get_address().unwrap(), FUNDING);
    chain.mine(1);
    wallet
        .create_utxos(online, false, Some(2), Some(UTXO_SATS), FEE_RATE, false)
        .unwrap();
    let txid = wallet
        .list_unspents(None, false, true)
        .unwrap()
        .into_iter()
        .find(|u| u.utxo.colorable)
        .unwrap()
        .utxo
        .outpoint
        .txid;
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let elsewhere = get_test_wallet(false, None).get_address().unwrap();
    chain.replace(&txid, &[(elsewhere.as_str(), FUNDING - 10_000)]);

    wallet.go_offline();
    wallet.go_online(completing_options(&chain)).unwrap();
    let txn = wallet.database().begin_transaction().unwrap();
    let rows: Vec<DbTxo> = txn
        .iter_txos()
        .unwrap()
        .into_iter()
        .filter(|t| t.txid == txid)
        .collect();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|t| !t.exists && !t.spent));
}
