//! ERA fork: CC-99, an own colored spend whose record was lost, on the scripted chain
//! (`scripted_chain`), no regtest services.

use super::scripted_chain::*;
use super::*;

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
