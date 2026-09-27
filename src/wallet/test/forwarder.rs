//! ERA fork: a wallet online with a forwarder (`OnlineOptions::forwarder_url`) sends its RGB proxy
//! and reject-list traffic to the forwarder, while invoices and stored transport endpoints keep
//! the real proxy.
//!
//! No regtest services: the Esplora indexer, the RGB proxy, the reject-list host and the
//! forwarder are local mock servers, and the indexer only answers the genesis lookup `go_online`
//! makes. Every request rgb-lib sends to a proxy or a reject list is sent here by the wallet code
//! that sends it in production, the call sites of `WalletOnline::proxy_client`,
//! `reject_list_client` and `check_proxy_endpoint`, and has to arrive at the forwarder, at the
//! path of `forwarder_url`, with both `X-Era-Forward-*` headers, while the real host is never
//! contacted:
//!
//! | request | sent by | the test drives |
//! |---|---|---|
//! | `server.info` | `send_begin`, probing each recipient endpoint | `send_begin` |
//! | `consignment.get` | `refresh` of a pending receive | `refresh` |
//! | `ack.post` (NACK) | `refresh` refusing a received consignment | `refresh` |
//! | `ack.get` | `refresh` of a send waiting for the recipient | `refresh` |
//! | `consignment.post`, `media.post` | `send_end` | `post_transfer_data` |
//! | `ack.post` (ACK) | `refresh` accepting a received consignment | `ack_consignment` |
//! | `media.get` | `refresh` receiving an asset it does not know | `fetch_and_save_attachments` |
//! | reject list `GET` | `refresh` and `send_begin` of an IFA asset | `get_reject_list` |
//!
//! The last four cannot be reached through the public API offline: `send_end` needs a PSBT that
//! spends real RGB allocations, and the ACK, the media and the reject list come after a
//! consignment has validated against a chain. Their tests call the wallet method that holds the
//! call site, on a wallet online through a forwarder. `refresh` always runs with `skip_sync`, and
//! `send_begin` on a wallet with no allocations stops at input selection, after its probe.

use mockito::{Matcher, Mock, Server, ServerGuard};
use serde_json::{Value as Json, json};

use super::*;
use crate::api::forwarder::{
    FORWARD_KIND_HEADER, FORWARD_KIND_REJECT_LIST, FORWARD_KIND_RGB_PROXY, FORWARD_TARGET_HEADER,
    tests::Untouchable,
};
use crate::utils::{append_recipient_nonce, derive_proxy_recipient_id};

const REGTEST_GENESIS: &str = "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206";
// what a proxy answers when it holds no consignment for the recipient yet
const NO_CONSIGNMENT: &str = r#"{"jsonrpc":"2.0","id":null,"result":null,"error":null}"#;
// the path of forwarder_url: requests go to it exactly (it may carry a per-session secret)
const FORWARDER_PATH: &str = "/session-6c1f0e/rgb";

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
            .mock("GET", FORWARDER_PATH)
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
        Some(format!("{}{FORWARDER_PATH}", self.forwarder.url()))
    }

    /// A wallet online through the forwarder.
    fn online_wallet(&self) -> (Wallet, Online) {
        let mut wallet = get_test_wallet(false, None);
        let online = wallet
            .go_online(self.options(self.forwarder_url()))
            .unwrap();
        (wallet, online)
    }

    /// The JSON-RPC `method` arriving at the forwarder, meant for `target`, with `params`;
    /// answered with `result`.
    fn expect_rpc(&mut self, target: &str, method: &str, params: Json, result: Json) -> Mock {
        self.forwarder
            .mock("POST", FORWARDER_PATH)
            .match_header(FORWARD_TARGET_HEADER, target)
            .match_header(FORWARD_KIND_HEADER, FORWARD_KIND_RGB_PROXY)
            .match_header("content-type", "application/json")
            .match_body(Matcher::PartialJson(json!({
                "method": method,
                "jsonrpc": "2.0",
                "params": params,
            })))
            .with_header("content-type", "application/json")
            .with_body(json!({"jsonrpc": "2.0", "id": null, "result": result}).to_string())
            .expect(1)
            .create()
    }

    /// The multipart upload `method` arriving at the forwarder, meant for `target`, with `params`
    /// (as rgb-lib serializes them) and `file`; answered with `true`.
    fn expect_upload(&mut self, target: &str, method: &str, params: &str, file: &[u8]) -> Mock {
        let part = |name: &str, value: &str| {
            Matcher::Regex(regex::escape(&format!(
                "name=\"{name}\"\r\n\r\n{value}\r\n"
            )))
        };
        let file = String::from_utf8(file.to_vec()).unwrap();
        self.forwarder
            .mock("POST", FORWARDER_PATH)
            .match_header(FORWARD_TARGET_HEADER, target)
            .match_header(FORWARD_KIND_HEADER, FORWARD_KIND_RGB_PROXY)
            .match_header(
                "content-type",
                Matcher::Regex(s!("^multipart/form-data; boundary=")),
            )
            .match_body(Matcher::AllOf(vec![
                part("method", method),
                part("jsonrpc", "2.0"),
                part("params", params),
                Matcher::Regex(s!("name=\"file\"")),
                Matcher::Regex(regex::escape(&format!("\r\n\r\n{file}\r\n"))),
            ]))
            .with_header("content-type", "application/json")
            .with_body(json!({"jsonrpc": "2.0", "id": null, "result": true}).to_string())
            .expect(1)
            .create()
    }

    /// `consignment.get` for `proxy_rid` arriving at the forwarder, meant for the proxy.
    fn expect_forwarded(&mut self, proxy_rid: &str, hits: usize) -> Mock {
        let target = self.target();
        self.forwarder
            .mock("POST", FORWARDER_PATH)
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

/// An NIA asset in the wallet's database only, so `send_begin` gets as far as its recipients.
fn asset_in_db(wallet: &Wallet) -> String {
    let asset_id = ContractId::from([7u8; 32]).to_string();
    let now = now().unix_timestamp();
    let txn = wallet.database().begin_transaction().unwrap();
    txn.set_asset(DbAssetActMod {
        id: ActiveValue::Set(asset_id.clone()),
        schema: ActiveValue::Set(AssetSchema::Nia),
        added_at: ActiveValue::Set(now),
        initial_supply: ActiveValue::Set(AMOUNT.to_string()),
        name: ActiveValue::Set(NAME.to_string()),
        precision: ActiveValue::Set(PRECISION),
        ticker: ActiveValue::Set(Some(TICKER.to_string())),
        timestamp: ActiveValue::Set(now),
        ..Default::default()
    })
    .unwrap();
    txn.commit().unwrap();
    asset_id
}

/// A send as `send_end` leaves it: waiting for the recipient's ACK, with `endpoint` (an invoice
/// endpoint, recipient nonce included) as the one the consignment was posted to.
fn send_waiting_for_ack(wallet: &Wallet, recipient_id: &str, endpoint: &str) {
    let now = now().unix_timestamp();
    let txn = wallet.database().begin_transaction().unwrap();
    let batch_transfer_idx = txn
        .set_batch_transfer(DbBatchTransferActMod {
            txid: ActiveValue::Set(Some(FAKE_TXID.to_string())),
            status: ActiveValue::Set(TransferStatus::WaitingCounterparty),
            expiration: ActiveValue::Set(Some(now + DURATION_SEND_TRANSFER as i64)),
            created_at: ActiveValue::Set(now),
            min_confirmations: ActiveValue::Set(MIN_CONFIRMATIONS),
            incoming: ActiveValue::Set(false),
            ..Default::default()
        })
        .unwrap();
    let asset_transfer_idx = txn
        .set_asset_transfer(DbAssetTransferActMod {
            user_driven: ActiveValue::Set(true),
            batch_transfer_idx: ActiveValue::Set(batch_transfer_idx),
            ..Default::default()
        })
        .unwrap();
    let transfer_idx = txn
        .set_transfer(DbTransferActMod {
            asset_transfer_idx: ActiveValue::Set(asset_transfer_idx),
            requested_assignment: ActiveValue::Set(Some(Assignment::Fungible(AMOUNT))),
            recipient_id: ActiveValue::Set(Some(recipient_id.to_string())),
            ..Default::default()
        })
        .unwrap();
    wallet
        .save_transfer_transport_endpoint(
            &txn,
            transfer_idx,
            &LocalTransportEndpoint {
                transport_type: TransportType::JsonRpc,
                endpoint: endpoint.to_string(),
                used: true,
                usable: true,
            },
        )
        .unwrap();
    txn.commit().unwrap();
}

#[test]
#[parallel]
fn proxy_traffic_goes_through_the_forwarder() {
    let mut services = Services::start();
    let endpoint = services.endpoint();
    let target = services.target();
    let (mut wallet, online) = services.online_wallet();

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
fn send_begin_probes_the_recipient_proxy_through_the_forwarder() {
    let mut services = Services::start();
    let (mut wallet, online) = services.online_wallet();
    let asset_id = asset_in_db(&wallet);
    // the invoice of another wallet, on the same proxy
    let mut recipient = get_test_wallet(false, None);
    let (receive_data, _) = receive(&mut recipient, &services.endpoint());
    let invoice_data = Invoice::new(receive_data.invoice).unwrap().invoice_data();
    // what send_begin probes: the invoice endpoint as rgb-lib reads it, recipient nonce included
    let probed = TransportEndpoint::new(invoice_data.transport_endpoints[0].clone())
        .unwrap()
        .endpoint;
    assert!(probed.starts_with(&services.target()));

    let direct = Untouchable::on(&mut services.proxy);
    let probe = services.expect_rpc(
        &probed,
        "server.info",
        Json::Null,
        json!({"protocol_version": "0.2", "version": "0.2.1", "uptime": 1}),
    );
    let recipient_map = HashMap::from([(
        asset_id,
        vec![Recipient {
            recipient_id: invoice_data.recipient_id,
            witness_data: Some(WitnessData {
                amount_sat: 1000,
                blinding: None,
            }),
            assignment: Assignment::Fungible(AMOUNT),
            transport_endpoints: invoice_data.transport_endpoints,
        }],
    )]);
    let result = wallet.send_begin(
        online,
        recipient_map,
        false,
        FEE_RATE,
        MIN_CONFIRMATIONS,
        default_send_expiration(),
        false,
        None,
    );
    // the endpoint passed its probe, so send_begin went on to input selection and stopped there,
    // as it does for a wallet that holds none of the asset
    assert!(
        matches!(result, Err(Error::InsufficientAssignments { .. })),
        "{result:?}"
    );
    probe.assert();
    direct.assert();
}

#[test]
#[parallel]
fn send_end_posts_the_consignment_and_media_through_the_forwarder() {
    let mut services = Services::start();
    let (wallet, _online) = services.online_wallet();
    let target = services.target();

    // what send_end hands to post_transfer_data: the consignment it wrote, the asset's media and a
    // recipient whose invoice endpoint passed the probe in send_begin
    let asset_transfer_dir = wallet.get_transfer_dir("forwarder").join("asset");
    let consignment_path = wallet.get_send_consignment_path_impl(&asset_transfer_dir);
    fs::create_dir_all(consignment_path.parent().unwrap()).unwrap();
    fs::write(&consignment_path, "consignment bytes").unwrap();
    let media_bytes = "media bytes";
    let digest = hash_bytes_hex(media_bytes.as_bytes());
    let media_path = wallet.get_media_dir().join(&digest);
    fs::write(&media_path, media_bytes).unwrap();
    let media = Media {
        file_path: media_path.to_string_lossy().to_string(),
        digest: digest.clone(),
        mime: s!("text/plain"),
    };
    let recipient_id = s!("recipient");
    let nonce = [0x5a; 8];
    let mut recipients = vec![LocalRecipient {
        recipient_id: recipient_id.clone(),
        local_recipient_data: LocalRecipientData::Witness(LocalWitnessData {
            amount_sat: 1000,
            blinding: None,
            vout: 1,
        }),
        assignment: Assignment::Fungible(AMOUNT),
        transport_endpoints: vec![LocalTransportEndpoint {
            transport_type: TransportType::JsonRpc,
            endpoint: append_recipient_nonce(&target, &nonce),
            used: false,
            usable: true,
        }],
    }];

    let direct = Untouchable::on(&mut services.proxy);
    // posted to the proxy URL without the nonce, under the routing id the nonce derives
    let proxy_rid = derive_proxy_recipient_id(&recipient_id, &nonce);
    let consignment = services.expect_upload(
        &target,
        "consignment.post",
        &format!(r#"{{"recipient_id":"{proxy_rid}","txid":"{FAKE_TXID}","vout":1}}"#),
        b"consignment bytes",
    );
    let media_post = services.expect_upload(
        &target,
        "media.post",
        &format!(r#"{{"attachment_id":"{digest}"}}"#),
        media_bytes.as_bytes(),
    );
    wallet
        .post_transfer_data(
            &mut recipients,
            asset_transfer_dir,
            FAKE_TXID.to_string(),
            HashSet::from([media]),
        )
        .unwrap();
    consignment.assert();
    media_post.assert();
    direct.assert();
    assert!(recipients[0].transport_endpoints[0].used);
}

#[test]
#[parallel]
fn refresh_polls_the_ack_through_the_forwarder() {
    let mut services = Services::start();
    let (mut wallet, online) = services.online_wallet();
    let target = services.target();
    let recipient_id = s!("recipient");
    let nonce = [0xc3; 8];
    send_waiting_for_ack(
        &wallet,
        &recipient_id,
        &append_recipient_nonce(&target, &nonce),
    );

    let direct = Untouchable::on(&mut services.proxy);
    // no answer yet: the send keeps waiting
    let ack = services.expect_rpc(
        &target,
        "ack.get",
        json!({"recipient_id": derive_proxy_recipient_id(&recipient_id, &nonce)}),
        Json::Null,
    );
    refresh(&mut wallet, online);
    ack.assert();
    direct.assert();
}

#[test]
#[parallel]
fn a_refused_consignment_is_nacked_through_the_forwarder() {
    let mut services = Services::start();
    let (mut wallet, online) = services.online_wallet();
    let target = services.target();
    let (_, transfer) = receive(&mut wallet, &services.endpoint());
    let proxy_rid = transfer.proxy_recipient_id.clone().unwrap();

    let direct = Untouchable::on(&mut services.proxy);
    // a consignment that is not even base64: the wallet refuses it and posts a NACK
    let get = services.expect_rpc(
        &target,
        "consignment.get",
        json!({"recipient_id": proxy_rid}),
        json!({"consignment": "not base64!", "txid": FAKE_TXID, "vout": 1, "validated": null}),
    );
    let nack = services.expect_rpc(
        &target,
        "ack.post",
        json!({"recipient_id": proxy_rid, "ack": false}),
        json!(true),
    );
    let result = wallet.refresh(online, None, vec![], true).unwrap();
    assert_eq!(
        result[&transfer.batch_transfer_idx].updated_status,
        Some(TransferStatus::Failed)
    );
    get.assert();
    nack.assert();
    direct.assert();
}

#[test]
#[parallel]
fn an_accepted_consignment_is_acked_through_the_forwarder() {
    let mut services = Services::start();
    let (mut wallet, _online) = services.online_wallet();
    let target = services.target();
    let (_, transfer) = receive(&mut wallet, &services.endpoint());
    let proxy_rid = transfer.proxy_recipient_id.clone().unwrap();

    let direct = Untouchable::on(&mut services.proxy);
    let ack = services.expect_rpc(
        &target,
        "ack.post",
        json!({"recipient_id": proxy_rid, "ack": true}),
        json!(true),
    );
    let txn = wallet.database().begin_transaction().unwrap();
    let batch_transfer = txn
        .get_db_data(false)
        .unwrap()
        .batch_transfers
        .into_iter()
        .find(|bt| bt.idx == transfer.batch_transfer_idx)
        .unwrap();
    let mut updated_batch_transfer: DbBatchTransferActMod = batch_transfer.clone().into();
    let updated = wallet
        .ack_consignment(
            &txn,
            &batch_transfer,
            proxy_rid,
            &mut updated_batch_transfer,
            &ReceiveMode::Proxy { proxy_url: target },
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, TransferStatus::WaitingBroadcast);
    ack.assert();
    direct.assert();
}

#[test]
#[parallel]
fn media_are_fetched_through_the_forwarder() {
    let mut services = Services::start();
    let (wallet, _online) = services.online_wallet();
    let target = services.target();
    let media_bytes = b"media of an asset the wallet receives for the first time";
    let digest = hash_bytes(media_bytes);
    let attachment = Attachment {
        ty: MediaType::with("text/plain"),
        digest: Bytes32::try_from(digest.as_slice()).unwrap(),
    };

    let direct = Untouchable::on(&mut services.proxy);
    let get = services.expect_rpc(
        &target,
        "media.get",
        json!({"attachment_id": hex::encode(&digest)}),
        json!(general_purpose::STANDARD.encode(media_bytes)),
    );
    let saved = wallet
        .fetch_and_save_attachments(vec![attachment], &ReceiveMode::Proxy { proxy_url: target })
        .unwrap();
    assert!(saved);
    assert_eq!(
        fs::read(wallet.get_media_dir().join(hex::encode(&digest))).unwrap(),
        media_bytes
    );
    get.assert();
    direct.assert();
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
    let (wallet, _online) = services.online_wallet();
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
