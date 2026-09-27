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
//!
//! A refusal of the forwarder (403 with `X-Era-Forward-Refused`) must come out as
//! `Error::ForwarderRefused` where rgb-lib would otherwise report a proxy out of reach, and must
//! change nothing where it reads a failed request as something else (`consignment.get`: no
//! consignment yet). A forwarder that is down keeps today's errors.

use mockito::{Matcher, Mock, Server, ServerGuard};
use serde_json::{Value as Json, json};

use super::*;
use crate::api::forwarder::{
    FORWARD_KIND_HEADER, FORWARD_KIND_REJECT_LIST, FORWARD_KIND_RGB_PROXY, FORWARD_REFUSED_HEADER,
    FORWARD_TARGET_HEADER, tests::Untouchable,
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

    /// The proxy request `method` arriving at the forwarder, meant for `target`, refused by it
    /// with `reason`.
    fn expect_refusal(&mut self, target: &str, method: &str, reason: &str) -> Mock {
        self.forwarder
            .mock("POST", FORWARDER_PATH)
            .match_header(FORWARD_TARGET_HEADER, target)
            .match_header(FORWARD_KIND_HEADER, FORWARD_KIND_RGB_PROXY)
            // in a JSON-RPC body and in a multipart one alike
            .match_body(Matcher::Regex(regex::escape(method)))
            .with_status(403)
            .with_header(FORWARD_REFUSED_HEADER, reason)
            .with_body("refused")
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
fn send_waiting_for_ack(wallet: &Wallet, recipient_id: &str, endpoint: &str) -> i32 {
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
    batch_transfer_idx
}

/// The invoice of another wallet's witness receive on `endpoints`.
fn invoice_on(endpoints: &[String]) -> InvoiceData {
    let mut recipient = get_test_wallet(false, None);
    let receive_data = recipient
        .witness_receive(
            None,
            Assignment::Any,
            default_rcv_expiration(),
            endpoints.to_vec(),
            MIN_CONFIRMATIONS,
        )
        .unwrap();
    Invoice::new(receive_data.invoice).unwrap().invoice_data()
}

/// What `send_begin` probes for the `n`th endpoint of `invoice`: the endpoint as rgb-lib reads
/// it, recipient nonce included.
fn probed(invoice: &InvoiceData, n: usize) -> String {
    TransportEndpoint::new(invoice.transport_endpoints[n].clone())
        .unwrap()
        .endpoint
}

fn server_info() -> Json {
    json!({"protocol_version": "0.2", "version": "0.2.1", "uptime": 1})
}

/// `send_begin` of the asset to `invoice`, the recipient built from the invoice as the app does.
fn send_begin_to(
    wallet: &mut Wallet,
    online: Online,
    asset_id: &str,
    invoice: &InvoiceData,
) -> Result<SendBeginResult, Error> {
    let recipient_map = HashMap::from([(
        asset_id.to_string(),
        vec![Recipient {
            recipient_id: invoice.recipient_id.clone(),
            witness_data: Some(WitnessData {
                amount_sat: 1000,
                blinding: None,
            }),
            assignment: Assignment::Fungible(AMOUNT),
            transport_endpoints: invoice.transport_endpoints.clone(),
        }],
    )]);
    wallet.send_begin(
        online,
        recipient_map,
        false,
        FEE_RATE,
        MIN_CONFIRMATIONS,
        default_send_expiration(),
        false,
        None,
    )
}

const CONSIGNMENT_BYTES: &str = "consignment bytes";
const MEDIA_BYTES: &str = "media bytes";

/// What `send_end` hands to `post_transfer_data`: the consignment it wrote, the asset's media and
/// a recipient whose invoice endpoint (on `target`, with a recipient nonce) passed the probe in
/// `send_begin`.
struct SendEndInputs {
    recipients: Vec<LocalRecipient>,
    asset_transfer_dir: PathBuf,
    media: Media,
    // the routing id the recipient nonce derives, which the consignment is posted under
    proxy_rid: String,
}

impl SendEndInputs {
    fn new(wallet: &Wallet, target: &str) -> Self {
        let asset_transfer_dir = wallet.get_transfer_dir("forwarder").join("asset");
        let consignment_path = wallet.get_send_consignment_path_impl(&asset_transfer_dir);
        fs::create_dir_all(consignment_path.parent().unwrap()).unwrap();
        fs::write(&consignment_path, CONSIGNMENT_BYTES).unwrap();
        let digest = hash_bytes_hex(MEDIA_BYTES.as_bytes());
        let media_path = wallet.get_media_dir().join(&digest);
        fs::write(&media_path, MEDIA_BYTES).unwrap();
        let recipient_id = s!("recipient");
        let nonce = [0x5a; 8];
        Self {
            recipients: vec![LocalRecipient {
                recipient_id: recipient_id.clone(),
                local_recipient_data: LocalRecipientData::Witness(LocalWitnessData {
                    amount_sat: 1000,
                    blinding: None,
                    vout: 1,
                }),
                assignment: Assignment::Fungible(AMOUNT),
                transport_endpoints: vec![LocalTransportEndpoint {
                    transport_type: TransportType::JsonRpc,
                    endpoint: append_recipient_nonce(target, &nonce),
                    used: false,
                    usable: true,
                }],
            }],
            asset_transfer_dir,
            media: Media {
                file_path: media_path.to_string_lossy().to_string(),
                digest,
                mime: s!("text/plain"),
            },
            proxy_rid: derive_proxy_recipient_id(&recipient_id, &nonce),
        }
    }

    fn post(&mut self, wallet: &Wallet) -> Result<(), Error> {
        wallet.post_transfer_data(
            &mut self.recipients,
            self.asset_transfer_dir.clone(),
            FAKE_TXID.to_string(),
            HashSet::from([self.media.clone()]),
        )
    }

    fn posted(&self) -> bool {
        self.recipients[0].transport_endpoints[0].used
    }
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
    let invoice = invoice_on(&[services.endpoint()]);
    assert!(probed(&invoice, 0).starts_with(&services.target()));

    let direct = Untouchable::on(&mut services.proxy);
    let probe = services.expect_rpc(
        &probed(&invoice, 0),
        "server.info",
        Json::Null,
        server_info(),
    );
    let result = send_begin_to(&mut wallet, online, &asset_id, &invoice);
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
    let mut inputs = SendEndInputs::new(&wallet, &target);

    let direct = Untouchable::on(&mut services.proxy);
    // posted to the proxy URL without the nonce, under the routing id the nonce derives
    let consignment = services.expect_upload(
        &target,
        "consignment.post",
        &format!(
            r#"{{"recipient_id":"{}","txid":"{FAKE_TXID}","vout":1}}"#,
            inputs.proxy_rid
        ),
        CONSIGNMENT_BYTES.as_bytes(),
    );
    let media_post = services.expect_upload(
        &target,
        "media.post",
        &format!(r#"{{"attachment_id":"{}"}}"#, inputs.media.digest),
        MEDIA_BYTES.as_bytes(),
    );
    inputs.post(&wallet).unwrap();
    consignment.assert();
    media_post.assert();
    direct.assert();
    assert!(inputs.posted());
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

#[test]
#[parallel]
fn send_begin_reports_the_forwarders_refusal() {
    let mut services = Services::start();
    let (mut wallet, online) = services.online_wallet();
    let asset_id = asset_in_db(&wallet);
    let invoice = invoice_on(&[services.endpoint()]);

    let direct = Untouchable::on(&mut services.proxy);
    let refusal = services.expect_refusal(&probed(&invoice, 0), "server.info", "unknown-proxy");
    let result = send_begin_to(&mut wallet, online, &asset_id, &invoice);
    // not "no valid transport endpoints", which reads as a proxy out of reach
    assert_eq!(
        result.unwrap_err(),
        Error::ForwarderRefused {
            target: probed(&invoice, 0),
            reason: s!("unknown-proxy"),
        }
    );
    refusal.assert();
    direct.assert();
}

#[test]
#[parallel]
fn send_begin_goes_on_with_a_usable_endpoint_next_to_a_refused_one() {
    let mut services = Services::start();
    let mut unknown = Server::new();
    let unknown_endpoint = format!("rpc://{}/json-rpc", unknown.host_with_port());
    let (mut wallet, online) = services.online_wallet();
    let asset_id = asset_in_db(&wallet);
    let invoice = invoice_on(&[unknown_endpoint, services.endpoint()]);

    let direct = [
        Untouchable::on(&mut unknown),
        Untouchable::on(&mut services.proxy),
    ];
    let refusal = services.expect_refusal(&probed(&invoice, 0), "server.info", "unknown-proxy");
    let probe = services.expect_rpc(
        &probed(&invoice, 1),
        "server.info",
        Json::Null,
        server_info(),
    );
    let result = send_begin_to(&mut wallet, online, &asset_id, &invoice);
    // one endpoint is usable, so the refusal of the other is no error: send_begin goes on to
    // input selection
    assert!(
        matches!(result, Err(Error::InsufficientAssignments { .. })),
        "{result:?}"
    );
    refusal.assert();
    probe.assert();
    direct.iter().for_each(Untouchable::assert);
}

#[test]
#[parallel]
fn send_begin_keeps_todays_error_when_the_forwarder_is_down() {
    let mut services = Services::start();
    let mut wallet = get_test_wallet(false, None);
    // nothing listens on port 1
    let online = wallet
        .go_online(services.options(Some(s!("http://127.0.0.1:1"))))
        .unwrap();
    let asset_id = asset_in_db(&wallet);
    let invoice = invoice_on(&[services.endpoint()]);

    let direct = Untouchable::on(&mut services.proxy);
    let result = send_begin_to(&mut wallet, online, &asset_id, &invoice);
    assert_eq!(
        result.unwrap_err(),
        Error::InvalidTransportEndpoints {
            details: s!("no valid transport endpoints"),
        }
    );
    direct.assert();
}

#[test]
#[parallel]
fn send_end_reports_the_forwarders_refusal() {
    let mut services = Services::start();
    let (wallet, _online) = services.online_wallet();
    let target = services.target();
    let mut inputs = SendEndInputs::new(&wallet, &target);

    let direct = Untouchable::on(&mut services.proxy);
    let refusal = services.expect_refusal(&target, "consignment.post", "consent-expired");
    // not NoValidTransportEndpoint, which reads as a proxy out of reach
    assert_eq!(
        inputs.post(&wallet).unwrap_err(),
        Error::ForwarderRefused {
            target: target.clone(),
            reason: s!("consent-expired"),
        }
    );
    assert!(!inputs.posted());
    refusal.assert();

    // a forwarder that is down: the error of today
    let mut down = get_test_wallet(false, None);
    down.go_online(services.options(Some(s!("http://127.0.0.1:1"))))
        .unwrap();
    let mut inputs = SendEndInputs::new(&down, &target);
    assert_eq!(
        inputs.post(&down).unwrap_err(),
        Error::NoValidTransportEndpoint
    );
    direct.assert();
}

#[test]
#[parallel]
fn a_refused_ack_poll_is_the_sends_refresh_failure() {
    let mut services = Services::start();
    let (mut wallet, online) = services.online_wallet();
    let target = services.target();
    let batch_transfer_idx = send_waiting_for_ack(&wallet, "recipient", &target);

    let direct = Untouchable::on(&mut services.proxy);
    let refusal = services.expect_refusal(&target, "ack.get", "consent-expired");
    let result = wallet.refresh(online, None, vec![], true).unwrap();
    // where a proxy out of reach would be Error::Proxy
    assert_eq!(
        result[&batch_transfer_idx].failure,
        Some(Error::ForwarderRefused {
            target,
            reason: s!("consent-expired"),
        })
    );
    refusal.assert();
    direct.assert();
}

#[test]
#[parallel]
fn a_refused_consignment_download_leaves_the_receive_waiting() {
    let mut services = Services::start();
    let (mut wallet, online) = services.online_wallet();
    let target = services.target();
    let (_, transfer) = receive(&mut wallet, &services.endpoint());
    let idx = transfer.batch_transfer_idx;

    let direct = Untouchable::on(&mut services.proxy);
    let refusal = services.expect_refusal(&target, "consignment.get", "not-allowlisted");
    // read as "no consignment yet", like every failure of consignment.get upstream: no error in
    // the refresh result and the receive keeps waiting
    let result = wallet.refresh(online, None, vec![], true).unwrap();
    assert_eq!(
        result[&idx],
        RefreshedTransfer {
            updated_status: None,
            failure: None,
        }
    );
    refusal.assert();
    refusal.remove();
    // which keeps it failable: fail_transfers refreshes a transfer before failing it, and an
    // error there would stop it
    let refusal = services.expect_refusal(&target, "consignment.get", "not-allowlisted");
    assert!(
        wallet
            .fail_transfers(online, Some(idx), false, true)
            .unwrap()
    );
    let status = wallet
        .list_transfers(AssetFilter::AnyOrNone, None)
        .unwrap()
        .into_iter()
        .find(|t| t.batch_transfer_idx == idx)
        .unwrap()
        .status;
    assert_eq!(status, TransferStatus::Failed);
    refusal.assert();
    direct.assert();
}

#[test]
#[parallel]
fn send_begin_refuses_an_endpoint_with_userinfo_or_a_fragment() {
    let mut services = Services::start();
    let (mut wallet, online) = services.online_wallet();
    let asset_id = asset_in_db(&wallet);
    let mut invoice = invoice_on(&[services.endpoint()]);
    let usable = invoice.transport_endpoints[0].clone();

    let direct = Untouchable::on(&mut services.proxy);
    for bad in [
        // a trusted-looking name before the real host, evil.example
        s!("rpcs://rgb-proxy.utexo.com@evil.example/json-rpc"),
        format!("{}#fragment", services.endpoint()),
    ] {
        // refused before any request
        invoice.transport_endpoints = vec![bad.clone()];
        let never = Untouchable::on(&mut services.forwarder);
        let result = send_begin_to(&mut wallet, online, &asset_id, &invoice);
        assert!(
            matches!(result, Err(Error::InvalidForwardTarget { .. })),
            "{bad}: {result:?}"
        );
        never.assert();
        never.remove();

        // an invoice carrying one is refused even next to a usable endpoint (probed first here)
        invoice.transport_endpoints = vec![usable.clone(), bad.clone()];
        let probe = services.expect_rpc(
            &probed(&invoice, 0),
            "server.info",
            Json::Null,
            server_info(),
        );
        let result = send_begin_to(&mut wallet, online, &asset_id, &invoice);
        assert!(
            matches!(result, Err(Error::InvalidForwardTarget { .. })),
            "{bad}: {result:?}"
        );
        probe.assert();
        probe.remove();
    }
    direct.assert();
}
