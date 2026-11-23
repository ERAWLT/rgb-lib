//! ERA fork: a wallet online with a forwarder (`OnlineOptions::forwarder_url`) sends its RGB proxy
//! traffic to the forwarder, on proxy protocol 1.0 (beta.8), while invoices and stored transport
//! endpoints keep the real proxy.
//!
//! On the scripted chain, which serves the forwarder too: it records each request's
//! `X-Era-Forward-Target` and `X-Era-Forward-Kind` and relays it to the chain's own proxy, or
//! refuses it as a test tells it to (403 with `X-Era-Forward-Refused`). Every proxy request the
//! sender of a whole send makes, from `send_begin` to the refresh that settles it, has to arrive
//! there, and none at the proxy directly.
//!
//! The 0.2 module (`forwarder.rs`, still behind `era_beta8_pending`) also drives the receiver's
//! requests, the media and the reject list with mock servers per method; on 1.0 those are the
//! regtest's (tests/regtest.sh) and the client-level tests' of `api::forwarder`.

use super::scripted_chain::*;
// (explicit: the shared test helpers have their own, for the electrum harness)
use super::scripted_chain::witness_recipient;
use super::*;

/// The options of `chain` with its forwarder.
fn forwarded_options(chain: &ScriptedChain) -> OnlineOptions {
    OnlineOptions {
        forwarder_url: Some(chain.forwarder_url()),
        ..online_options(chain)
    }
}

/// The URL rgb-lib derives from the chain's proxy endpoint and sends as the forward target.
fn proxy_target(chain: &ScriptedChain) -> String {
    chain.proxy_endpoint().replacen("rpc://", "http://", 1)
}

/// An issuer (see `scripted_chain::issuer`), back online through the forwarder.
fn routed_issuer(chain: &ScriptedChain) -> Issuer {
    let mut party = issuer(chain, vec![AMOUNT]);
    party.wallet.go_offline();
    party.online = party.wallet.go_online(forwarded_options(chain)).unwrap();
    party
}

#[test]
#[parallel]
fn a_send_goes_through_the_forwarder_from_begin_to_settled() {
    let chain = ScriptedChain::start();
    let mut party = routed_issuer(&chain);
    let (direct, forwarded, served) = (
        chain.direct_proxy_methods().len(),
        chain.forwarded().len(),
        chain.proxy_methods().len(),
    );

    let (begin, signed, recipient) = begin_send(&chain, &mut party, AMOUNT_SMALL, false);
    let idx = begin.batch_transfer_idx.unwrap();
    party.wallet.send_end(party.online, signed).unwrap();
    chain.set_ack(&recipient.recipient_id, true);
    party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    chain.mine(1);
    party
        .wallet
        .refresh(party.online, None, vec![], false)
        .unwrap();
    assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);

    // nothing reached the proxy but through the forwarder, every request meant for the proxy
    // the invoice names, as an RGB proxy request
    assert_eq!(chain.direct_proxy_methods()[direct..], [] as [String; 0]);
    let forwarded = chain.forwarded()[forwarded..].to_vec();
    let target = proxy_target(&chain);
    assert!(
        forwarded
            .iter()
            .all(|(t, kind, _)| *t == target && kind == "rgb-proxy"),
        "{forwarded:?}"
    );
    // and the proxy served exactly what the forwarder relayed, from the probe to the ACK
    let methods: Vec<String> = forwarded.into_iter().map(|(_, _, m)| m).collect();
    assert_eq!(chain.proxy_methods()[served..], methods[..]);
    for method in ["server.info", "ack.prepare", "consignment.post", "ack.get"] {
        assert!(methods.iter().any(|m| m == method), "{method}: {methods:?}");
    }
    chain.assert_all_matched();
}

#[test]
#[parallel]
fn send_begin_reports_the_forwarders_refusal() {
    let chain = ScriptedChain::start();
    let mut party = routed_issuer(&chain);
    chain.refuse("server.info", "unknown-proxy", 1);
    let result = party.wallet.send_begin(
        party.online,
        HashMap::from([(
            party.asset_id.clone(),
            vec![witness_recipient(&chain, AMOUNT_SMALL)],
        )]),
        false,
        FEE_RATE,
        MIN_CONFIRMATIONS,
        (now().unix_timestamp() + DURATION_SEND_TRANSFER as i64) as u64,
        Some(chain.proxy_endpoint()),
        false,
    );
    let target = proxy_target(&chain);
    assert_matches!(
        result,
        Err(Error::ForwarderRefused { target: t, reason }) if t == target && reason == "unknown-proxy"
    );
}

#[test]
#[parallel]
fn send_end_reports_the_forwarders_refusal() {
    let chain = ScriptedChain::start();
    let mut party = routed_issuer(&chain);
    let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, false);
    chain.refuse("consignment.post", "consent-expired", 1);
    let result = party.wallet.send_end(party.online, signed);
    assert_matches!(
        result,
        Err(Error::ForwarderRefused { reason, .. }) if reason == "consent-expired"
    );
    // nothing reached the network: the send is still to be ended
    assert_eq!(
        status_of(&party.wallet, begin.batch_transfer_idx.unwrap()),
        TransferStatus::Initiated
    );
    assert!(chain.direct_proxy_methods().is_empty());
}

// review N4: a go_online that names no forwarder does not take a routed wallet off it; going
// offline first does
#[test]
#[parallel]
fn a_routed_wallet_goes_direct_only_after_going_offline() {
    let chain = ScriptedChain::start();
    let mut party = routed_issuer(&chain);

    let result = party.wallet.go_online(online_options(&chain));
    assert_matches!(result, Err(Error::InvalidForwarderUrl { .. }));
    // offline: the previous session is gone, not kept direct
    assert_matches!(
        party.wallet.refresh(party.online, None, vec![], false),
        Err(Error::Offline)
    );

    party.wallet.go_offline();
    party.online = party.wallet.go_online(online_options(&chain)).unwrap();
    let forwarded = chain.forwarded().len();
    begin_send(&chain, &mut party, AMOUNT_SMALL, false);
    assert_eq!(chain.forwarded().len(), forwarded);
    assert!(
        chain
            .direct_proxy_methods()
            .iter()
            .any(|m| m == "server.info")
    );
}
