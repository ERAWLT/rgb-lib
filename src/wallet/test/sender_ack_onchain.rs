//! ERA fork: a send waiting for its ACKs whose sender proxy fails `ack.get` (an error of the
//! proxy's own, an answer with neither a result nor an error, one that is not JSON-RPC, the
//! forwarder's refusal) completes at the refresh if the indexer knows its own TX, mempool
//! included, and otherwise keeps waiting with the proxy's error, as upstream does
//! (`WalletOnline::complete_on_chain_or`).
//!
//! Live on UTEXO's signet sandbox (25.11.2026), `ack.get` answered the proxy's own database error
//! ("no such column: fetched") while the recipient had ACKed, got the TX released and broadcast
//! it: the TX confirmed and the sender stayed `WaitingCounterparty`, its change locked, until the
//! transfer would expire. On the scripted chain; `era_regtest` runs the same send on the regtest
//! RGB proxy. See the fork's `ERA.md`.

use super::scripted_chain::*;
// (explicit: the shared test helpers have their own, for the electrum harness)
use super::scripted_chain::psbt_txid;
use super::*;
use crate::wallet::online::SIGNED_TX_FILE;

/// What UTEXO's sandbox answered (its error code was not recorded; any code rgb-lib does not map
/// reads the same, as `Error::Proxy` with the message).
const SANDBOX_ERROR: &str = "no such column: fetched";

/// How the sender proxy fails `ack.get`.
#[derive(Clone, Copy, Debug)]
enum Failure {
    /// an error of the proxy's own (the sandbox's)
    RpcError,
    /// an answer with neither a result nor an error
    NoResult,
    /// an answer that is not JSON-RPC (a 502 page of whatever is in front of the proxy)
    NotJsonRpc,
    /// the forwarder refuses the request (the wallet routed through it, as the app is)
    Refused,
}

const FAILURES: [Failure; 4] = [
    Failure::RpcError,
    Failure::NoResult,
    Failure::NotJsonRpc,
    Failure::Refused,
];

impl Failure {
    /// The next `times` `ack.get` requests fail so.
    fn script(self, chain: &ScriptedChain, times: usize) {
        match self {
            Failure::RpcError => chain.answer(
                "ack.get",
                serde_json::json!({"jsonrpc": "2.0", "id": null, "result": null,
                    "error": {"code": -32603, "message": SANDBOX_ERROR}}),
                times,
            ),
            Failure::NoResult => chain.answer(
                "ack.get",
                serde_json::json!({"jsonrpc": "2.0", "id": null, "result": null, "error": null}),
                times,
            ),
            // the refreshes below ask the proxy nothing but ack.get
            Failure::NotJsonRpc => chain.fault("POST", PROXY_PATH, 502, false, times),
            Failure::Refused => chain.refuse("ack.get", "consent-expired", times),
        }
    }

    fn options(self, chain: &ScriptedChain) -> OnlineOptions {
        match self {
            Failure::Refused => OnlineOptions {
                forwarder_url: Some(chain.forwarder_url()),
                ..online_options(chain)
            },
            _ => online_options(chain),
        }
    }

    /// Whether `error` is the one this failure makes the refresh report.
    fn reported(self, error: &Option<Error>) -> bool {
        match (self, error) {
            (Failure::RpcError, Some(Error::Proxy { details })) => details == SANDBOX_ERROR,
            (Failure::NoResult, Some(Error::Proxy { details })) => {
                details == "missing ack.get result"
            }
            (Failure::NotJsonRpc, Some(Error::Proxy { .. })) => true,
            (Failure::Refused, Some(Error::ForwarderRefused { reason, .. })) => {
                reason == "consent-expired"
            }
            _ => false,
        }
    }
}

/// An issuer (see `scripted_chain::issuer`) online with `failure`'s options.
fn sender(chain: &ScriptedChain, failure: Failure) -> Issuer {
    let mut party = issuer(chain, vec![AMOUNT, AMOUNT]);
    party.wallet.go_offline();
    party.online = party.wallet.go_online(failure.options(chain)).unwrap();
    party
}

/// A send of `AMOUNT_SMALL` waiting for its recipient's ACK: its batch transfer, TXID, signed PSBT
/// and recipient.
fn waiting_send(chain: &ScriptedChain, party: &mut Issuer) -> (i32, String, String, Recipient) {
    let (begin, signed, recipient) = begin_send(chain, party, AMOUNT_SMALL, false);
    let idx = begin.batch_transfer_idx.unwrap();
    let txid = psbt_txid(&signed);
    party.wallet.send_end(party.online, signed.clone()).unwrap();
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingCounterparty
    );
    assert!(!chain.knows(&txid));
    (idx, txid, signed, recipient)
}

fn signed_tx_of(signed: &str) -> BdkTransaction {
    Psbt::from_str(signed).unwrap().extract_tx().unwrap()
}

fn refresh(party: &mut Issuer) -> RefreshResult {
    party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap()
}

fn broadcasts(chain: &ScriptedChain) -> usize {
    chain
        .requests()
        .iter()
        .filter(|r| r.as_str() == "POST /tx")
        .count()
}

// the live case: the recipient ACKed and broadcast the TX, the sender proxy fails ack.get; the
// refresh finds the TX (in the mempool or in a block), completes the send, and it settles as one
// whose ACK came in
#[test]
#[parallel]
fn a_send_whose_tx_is_on_chain_completes_when_ack_get_fails() {
    for failure in FAILURES {
        for confirmed in [false, true] {
            let chain = ScriptedChain::start();
            let mut party = sender(&chain, failure);
            let (idx, txid, signed, recipient) = waiting_send(&chain, &mut party);
            chain.set_ack(&recipient.recipient_id, true);
            chain.publish(&signed_tx_of(&signed));
            if confirmed {
                chain.mine(1);
            }
            failure.script(&chain, 1);
            chain.clear_requests();

            let result = refresh(&mut party);
            let refreshed = &result[&idx];
            assert!(refreshed.failure.is_none(), "{failure:?}: {refreshed:?}");
            assert_eq!(
                refreshed.updated_status,
                Some(TransferStatus::WaitingConfirmations),
                "{failure:?}"
            );
            assert!(
                chain.requests().contains(&format!("GET /tx/{txid}/status")),
                "{failure:?}"
            );
            assert_eq!(chain.is_confirmed(&txid), confirmed);

            if !confirmed {
                chain.mine(1);
            }
            refresh(&mut party);
            assert_eq!(
                status_of(&party.wallet, idx),
                TransferStatus::Settled,
                "{failure:?}"
            );
            assert_eq!(
                spendable(&party.wallet, &party.asset_id),
                2 * AMOUNT - AMOUNT_SMALL,
                "{failure:?}"
            );
            chain.assert_all_matched();
        }
    }
}

// nobody broadcast the TX: the refresh reports the proxy's error as upstream does, broadcasts
// nothing and changes nothing; once the proxy answers again the send goes on as any send
#[test]
#[parallel]
fn a_send_whose_tx_is_not_on_chain_keeps_waiting_with_the_proxys_error() {
    for failure in FAILURES {
        let chain = ScriptedChain::start();
        let mut party = sender(&chain, failure);
        let (idx, txid, _, recipient) = waiting_send(&chain, &mut party);
        chain.set_ack(&recipient.recipient_id, true);
        failure.script(&chain, 1);
        chain.clear_requests();

        let result = refresh(&mut party);
        let refreshed = &result[&idx];
        assert!(
            failure.reported(&refreshed.failure),
            "{failure:?}: {refreshed:?}"
        );
        assert_eq!(refreshed.updated_status, None);
        assert_eq!(
            status_of(&party.wallet, idx),
            TransferStatus::WaitingCounterparty
        );
        assert!(
            chain.requests().contains(&format!("GET /tx/{txid}/status")),
            "{failure:?}"
        );
        assert_eq!(broadcasts(&chain), 0, "{failure:?}");
        assert!(!chain.knows(&txid));

        let result = refresh(&mut party);
        assert!(result[&idx].failure.is_none(), "{failure:?}");
        assert_eq!(
            status_of(&party.wallet, idx),
            TransferStatus::WaitingConfirmations
        );
        assert!(chain.knows(&txid));
    }
}

// only the send's own TX completes it: another TX spending its coins (its own replaced, or
// another install's on the same seed) is not looked for, and the send keeps waiting
#[test]
#[parallel]
fn another_tx_spending_the_coins_does_not_complete_the_send() {
    let chain = ScriptedChain::start();
    let mut party = sender(&chain, Failure::RpcError);
    let (idx, txid, signed, _) = waiting_send(&chain, &mut party);
    chain.publish(&signed_tx_of(&signed));
    let address = get_test_wallet(false, None).get_address().unwrap();
    let other = chain.replace(&txid, &[(&address, 50_000)]);
    assert!(chain.knows(&other.to_string()) && !chain.knows(&txid));
    Failure::RpcError.script(&chain, 1);
    chain.clear_requests();

    let result = refresh(&mut party);
    assert!(Failure::RpcError.reported(&result[&idx].failure));
    assert_eq!(result[&idx].updated_status, None);
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingCounterparty
    );
    assert_eq!(broadcasts(&chain), 0);
}

// the send's TX is on chain but its stored signed TX is another transfer's (as in W11): the
// completion is refused as when its ACK comes in, with that error rather than the proxy's, and
// nothing is broadcast
#[test]
#[parallel]
fn a_completion_the_signed_tx_guard_refuses_reports_the_guard() {
    let chain = ScriptedChain::start();
    let mut party = sender(&chain, Failure::RpcError);
    let (first, first_txid, first_signed, _) = waiting_send(&chain, &mut party);
    let (second, second_txid, _, _) = waiting_send(&chain, &mut party);
    let transfers_dir = party.wallet.get_transfers_dir();
    fs::copy(
        transfers_dir.join(&second_txid).join(SIGNED_TX_FILE),
        transfers_dir.join(&first_txid).join(SIGNED_TX_FILE),
    )
    .unwrap();
    chain.publish(&signed_tx_of(&first_signed));
    Failure::RpcError.script(&chain, 2);
    chain.clear_requests();

    let result = refresh(&mut party);
    assert_matches!(
        &result[&first].failure,
        Some(Error::InvalidPsbt { details }) if details.contains("for another TX")
    );
    assert!(Failure::RpcError.reported(&result[&second].failure));
    assert_eq!(broadcasts(&chain), 0);
    assert!(!chain.knows(&second_txid));
    assert_eq!(
        status_of(&party.wallet, first),
        TransferStatus::WaitingCounterparty
    );
}

// the lookup fails too: the refresh reports the proxy's error, not the indexer's, and the wallet's
// log names neither the lookup's text (a forwarder's error page naming the session's secret) nor
// anything of it; the next refresh, the indexer back, completes the send
#[test]
#[parallel]
fn a_failed_lookup_keeps_the_proxys_error_and_its_text_out_of_the_log() {
    const SECRET: &str = "5ec2e7a1b0c4d9f86e3a";
    let chain = ScriptedChain::start();
    let mut party = sender(&chain, Failure::RpcError);
    let (idx, txid, signed, _) = waiting_send(&chain, &mut party);
    chain.publish(&signed_tx_of(&signed));
    let path = format!("/tx/{txid}/status");
    chain.fault_answering(
        "GET",
        &path,
        502,
        false,
        1,
        &format!("no upstream for /{SECRET}/esplora{path}"),
    );
    Failure::RpcError.script(&chain, 2);

    let result = refresh(&mut party);
    assert!(
        Failure::RpcError.reported(&result[&idx].failure),
        "{:?}",
        result[&idx]
    );
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingCounterparty
    );

    let result = refresh(&mut party);
    assert!(result[&idx].failure.is_none());
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingConfirmations
    );

    let log_path = party.wallet.get_wallet_dir().join("log");
    // the log is written by a thread of its own, flushed when the wallet goes
    drop(party);
    let log = fs::read_to_string(log_path).unwrap();
    assert!(log.contains("the indexer or the network failed the TX lookup: waiting"));
    assert!(log.contains("but the TX is on-chain: completed"));
    assert!(log.contains(SANDBOX_ERROR));
    assert!(!log.contains(SECRET));
}

// failing the send meets the same refresh: the send is completed and not failed
// (CannotFailBatchTransfer, as for a TX the indexer knows), where upstream returned the proxy's
// error
#[test]
#[parallel]
fn failing_a_send_whose_tx_is_on_chain_completes_it() {
    let chain = ScriptedChain::start();
    let mut party = sender(&chain, Failure::RpcError);
    let (idx, _, signed, _) = waiting_send(&chain, &mut party);
    chain.publish(&signed_tx_of(&signed));
    Failure::RpcError.script(&chain, 1);

    let result = party
        .wallet
        .fail_transfers(party.online, Some(idx), false, false);
    assert_matches!(result, Err(Error::CannotFailBatchTransfer));
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingConfirmations
    );
}

// without a failure the proxy's answer decides, and the indexer is not asked: the lookup is the
// failing proxy's alone
#[test]
#[parallel]
fn an_answering_proxy_costs_no_lookup() {
    let chain = ScriptedChain::start();
    let mut party = sender(&chain, Failure::RpcError);
    let (idx, txid, _, _) = waiting_send(&chain, &mut party);
    chain.clear_requests();

    let result = refresh(&mut party);
    assert!(result[&idx].failure.is_none());
    assert_eq!(
        status_of(&party.wallet, idx),
        TransferStatus::WaitingCounterparty
    );
    assert!(!chain.requests().contains(&format!("GET /tx/{txid}/status")));
}
