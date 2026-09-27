//! ERA fork: a wallet online with a forwarder (`OnlineOptions::forwarder_url`) sends its RGB proxy
//! and reject-list traffic to the forwarder, while invoices and stored transport endpoints keep
//! the real proxy.
//!
//! No regtest services: the Esplora indexer, the RGB proxy, the reject-list host and the
//! forwarder are local mock servers. The indexer only answers the genesis lookup `go_online`
//! makes, and every `refresh` is run with `skip_sync`, so the network requests are the ones under
//! test: `consignment.get` for a pending witness receive, and the reject list, fetched through
//! the wallet's own `get_reject_list` (its callers validate an IFA consignment or build an IFA
//! send, and neither exists without a chain). The client-level tests in `api::forwarder` cover
//! the other proxy methods.

use mockito::{Matcher, Mock, Server, ServerGuard};
use serde_json::json;

use super::*;
use crate::api::forwarder::{
    FORWARD_KIND_HEADER, FORWARD_KIND_REJECT_LIST, FORWARD_KIND_RGB_PROXY, FORWARD_TARGET_HEADER,
    tests::Untouchable,
};

const REGTEST_GENESIS: &str = "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206";
// what a proxy answers when it holds no consignment for the recipient yet
const NO_CONSIGNMENT: &str = r#"{"jsonrpc":"2.0","id":null,"result":null,"error":null}"#;

struct Services {
    esplora: ServerGuard,
    proxy: ServerGuard,
    // the host of an asset's reject list
    issuer: ServerGuard,
    forwarder: ServerGuard,
    _genesis: Mock,
}

impl Services {
    fn start() -> Self {
        let mut esplora = Server::new();
        let genesis = esplora
            .mock("GET", "/block-height/0")
            .with_body(REGTEST_GENESIS)
            .create();
        Self {
            esplora,
            proxy: Server::new(),
            issuer: Server::new(),
            forwarder: Server::new(),
            _genesis: genesis,
        }
    }

    /// The URL of a reject list, as an asset contract names it.
    fn reject_list_url(&self) -> String {
        format!("{}/lists/usdt.txt", self.issuer.url())
    }

    /// The reject list arriving at the forwarder, meant for the issuer, answered with `status`.
    fn expect_reject_list(&mut self, status: usize, body: &str) -> Mock {
        let target = self.reject_list_url();
        self.forwarder
            .mock("GET", "/")
            .match_header(FORWARD_TARGET_HEADER, target.as_str())
            .match_header(FORWARD_KIND_HEADER, FORWARD_KIND_REJECT_LIST)
            .with_status(status)
            .with_body(body)
            .expect(1)
            .create()
    }

    /// The transport endpoint as it goes into an invoice.
    fn endpoint(&self) -> String {
        format!("rpc://{}/json-rpc", self.proxy.host_with_port())
    }

    /// The URL rgb-lib derives from [`Self::endpoint`] and would request.
    fn target(&self) -> String {
        format!("http://{}/json-rpc", self.proxy.host_with_port())
    }

    fn options(&self, forwarder_url: Option<String>) -> OnlineOptions {
        OnlineOptions {
            indexer_url: self.esplora.url(),
            skip_consistency_check: true,
            vanilla_sync_lookback: 1,
            forwarder_url,
        }
    }

    fn forwarder_url(&self) -> Option<String> {
        Some(self.forwarder.url())
    }

    /// `consignment.get` for `proxy_rid` arriving at the forwarder, meant for the proxy.
    fn expect_forwarded(&mut self, proxy_rid: &str, hits: usize) -> Mock {
        let target = self.target();
        self.forwarder
            .mock("POST", "/")
            .match_header(FORWARD_TARGET_HEADER, target.as_str())
            .match_header(FORWARD_KIND_HEADER, FORWARD_KIND_RGB_PROXY)
            .match_body(Matcher::PartialJson(json!({
                "method": "consignment.get",
                "params": {"recipient_id": proxy_rid},
            })))
            .with_body(NO_CONSIGNMENT)
            .expect(hits)
            .create()
    }

    /// `consignment.get` for `proxy_rid` arriving at the proxy itself, with no forwarding header.
    fn expect_direct(&mut self, proxy_rid: &str, hits: usize) -> Mock {
        self.proxy
            .mock("POST", "/json-rpc")
            .match_header(FORWARD_TARGET_HEADER, Matcher::Missing)
            .match_header(FORWARD_KIND_HEADER, Matcher::Missing)
            .match_body(Matcher::PartialJson(json!({
                "method": "consignment.get",
                "params": {"recipient_id": proxy_rid},
            })))
            .with_body(NO_CONSIGNMENT)
            .expect(hits)
            .create()
    }
}

/// A witness receive on the proxy; returns its transfer.
fn receive(wallet: &mut Wallet, endpoint: &str) -> (ReceiveData, Transfer) {
    let receive_data = wallet
        .witness_receive(
            None,
            Assignment::Any,
            default_rcv_expiration(),
            vec![endpoint.to_string()],
            MIN_CONFIRMATIONS,
        )
        .unwrap();
    let transfer = wallet
        .list_transfers(AssetFilter::AnyOrNone, None)
        .unwrap()
        .into_iter()
        .find(|t| t.recipient_id.as_deref() == Some(receive_data.recipient_id.as_str()))
        .unwrap();
    (receive_data, transfer)
}

fn refresh(wallet: &mut Wallet, online: Online) {
    let result = wallet.refresh(online, None, vec![], true).unwrap();
    assert!(
        result.values().all(|r| r.failure.is_none()),
        "refresh failed: {result:?}"
    );
}

#[test]
#[parallel]
fn proxy_traffic_goes_through_the_forwarder() {
    let mut services = Services::start();
    let endpoint = services.endpoint();
    let target = services.target();
    let mut wallet = get_test_wallet(false, None);
    let online = wallet
        .go_online(services.options(services.forwarder_url()))
        .unwrap();

    let (receive_data, transfer) = receive(&mut wallet, &endpoint);

    // the invoice and the stored transfer carry the real proxy, not the forwarder
    let invoice = Invoice::new(receive_data.invoice.clone()).unwrap();
    // (a nonce, which rgb-lib adds for a pinned witness script, is a query parameter on the same
    // endpoint)
    let invoice_endpoints = invoice.invoice_data().transport_endpoints;
    assert_eq!(invoice_endpoints.len(), 1);
    assert_eq!(
        crate::utils::extract_recipient_nonce(&invoice_endpoints[0]).0,
        endpoint
    );
    assert!(
        !receive_data
            .invoice
            .contains(&services.forwarder.host_with_port())
    );
    assert_eq!(transfer.transport_endpoints.len(), 1);
    assert_eq!(transfer.transport_endpoints[0].endpoint, target);

    let proxy_rid = transfer.proxy_recipient_id.clone().unwrap();
    let direct = Untouchable::on(&mut services.proxy);
    let forwarded = services.expect_forwarded(&proxy_rid, 2);
    refresh(&mut wallet, online);
    refresh(&mut wallet, online);
    forwarded.assert();
    direct.assert();

    // the transfer still waits, on the real proxy
    let transfer_after = wallet
        .list_transfers(AssetFilter::AnyOrNone, None)
        .unwrap()
        .into_iter()
        .find(|t| t.idx == transfer.idx)
        .unwrap();
    assert_eq!(transfer_after.status, TransferStatus::WaitingCounterparty);
    assert_eq!(transfer_after.transport_endpoints[0].endpoint, target);
    assert!(!transfer_after.transport_endpoints[0].used);
}

#[test]
#[parallel]
fn without_a_forwarder_the_proxy_is_contacted_directly() {
    let mut services = Services::start();
    let endpoint = services.endpoint();
    let mut wallet = get_test_wallet(false, None);
    let online = wallet.go_online(services.options(None)).unwrap();

    let (_, transfer) = receive(&mut wallet, &endpoint);
    let proxy_rid = transfer.proxy_recipient_id.clone().unwrap();
    let forwarder = Untouchable::on(&mut services.forwarder);
    let direct = services.expect_direct(&proxy_rid, 1);
    refresh(&mut wallet, online);
    direct.assert();
    forwarder.assert();
}

#[test]
#[parallel]
fn the_forwarder_follows_the_latest_go_online() {
    let mut services = Services::start();
    let endpoint = services.endpoint();
    let mut wallet = get_test_wallet(false, None);
    let online = wallet.go_online(services.options(None)).unwrap();
    let (_, transfer) = receive(&mut wallet, &endpoint);
    let proxy_rid = transfer.proxy_recipient_id.clone().unwrap();

    // same indexer, forwarder added: the online object is kept, the traffic moves
    let online_fwd = wallet
        .go_online(services.options(services.forwarder_url()))
        .unwrap();
    assert_eq!(online_fwd, online);
    let direct = Untouchable::on(&mut services.proxy);
    let forwarded = services.expect_forwarded(&proxy_rid, 1);
    refresh(&mut wallet, online);
    forwarded.assert();
    direct.assert();
    forwarded.remove();
    direct.remove();

    // a refused forwarder URL is an error and changes nothing: still through the forwarder
    for bad in [
        "http://10.0.2.2:8080",
        "https://127.0.0.1:8080",
        "http://localhost:8080",
    ] {
        let err = wallet
            .go_online(services.options(Some(bad.to_string())))
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidForwarderUrl { .. }),
            "{bad}: {err:?}"
        );
    }
    let direct = Untouchable::on(&mut services.proxy);
    let forwarded = services.expect_forwarded(&proxy_rid, 1);
    refresh(&mut wallet, online);
    forwarded.assert();
    direct.assert();
    direct.remove();
    forwarded.remove();

    // forwarder removed by the host: direct again, as upstream
    let online_direct = wallet.go_online(services.options(None)).unwrap();
    assert_eq!(online_direct, online);
    let forwarder = Untouchable::on(&mut services.forwarder);
    let direct = services.expect_direct(&proxy_rid, 1);
    refresh(&mut wallet, online);
    direct.assert();
    forwarder.assert();
}

#[test]
#[parallel]
fn a_forwarder_that_is_down_fails_the_request_without_a_fallback() {
    let mut services = Services::start();
    let endpoint = services.endpoint();
    let mut wallet = get_test_wallet(false, None);
    // nothing listens on port 1
    let online = wallet
        .go_online(services.options(Some(s!("http://127.0.0.1:1"))))
        .unwrap();
    let (_, transfer) = receive(&mut wallet, &endpoint);
    let direct = Untouchable::on(&mut services.proxy);
    let result = wallet.refresh(online, None, vec![], true).unwrap();
    // an unreachable proxy reads as "no consignment yet" in wait_consignment, as upstream: the
    // transfer stays pending and nothing went around the forwarder
    assert!(result.values().all(|r| r.updated_status.is_none()));
    direct.assert();
    let transfer_after = wallet
        .list_transfers(AssetFilter::AnyOrNone, None)
        .unwrap()
        .into_iter()
        .find(|t| t.idx == transfer.idx)
        .unwrap();
    assert_eq!(transfer_after.status, TransferStatus::WaitingCounterparty);
}

fn opout(byte: u8) -> Opout {
    Opout::new(rgbstd::OpId::from([byte; 32]), OS_ASSET, 0)
}

#[test]
#[parallel]
fn the_reject_list_goes_through_the_forwarder_and_fails_closed() {
    let mut services = Services::start();
    let list_url = services.reject_list_url();
    let mut wallet = get_test_wallet(false, None);
    wallet
        .go_online(services.options(services.forwarder_url()))
        .unwrap();
    let direct = Untouchable::on(&mut services.issuer);

    // a 2xx answer is the list, parsed as upstream parses it
    let (rejected, allowed) = (opout(1), opout(2));
    let list = services.expect_reject_list(200, &format!("{rejected}\n!{allowed}\nnot an opout\n"));
    let (reject_opouts, allow_opouts) = wallet.get_reject_list(&list_url).unwrap();
    assert_eq!(reject_opouts, HashSet::from([rejected]));
    assert_eq!(allow_opouts, HashSet::from([allowed]));
    list.assert();
    list.remove();

    // anything else is an error, never an empty list the consignment would pass against
    for status in [403, 503] {
        let answer = services.expect_reject_list(status, "<html>error page</html>\n");
        let result = wallet.get_reject_list(&list_url);
        assert!(
            matches!(&result, Err(Error::RejectListService { details })
                if details.contains(&status.to_string())),
            "{status}: {result:?}"
        );
        answer.assert();
        answer.remove();
    }
    direct.assert();
}
