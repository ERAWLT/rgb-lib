//! ERA fork: a scripted Esplora indexer and RGB proxy, for wallet tests that need a chain without
//! the regtest services.
//!
//! One local HTTP server answers the Esplora API rgb-lib and BDK use (blocks, script hash
//! histories, transactions, their status and spends, broadcast, fee estimates) from an in-memory
//! chain, and the RGB proxy's JSON-RPC at [`PROXY_PATH`]. Transactions are what the wallets under
//! test broadcast plus funding transactions the test makes up; nothing checks signatures or
//! amounts, since only the wallet reads this chain. Blocks are mined when the test says so.
//!
//! What makes it scripted rather than a mock of one request:
//! - a fault answers the next matching requests with a status of the test's choice, optionally
//!   after the request took effect (a relayed `POST /tx` whose answer is lost);
//! - a withheld transaction is one the indexer does not know (never listed, looked up as unknown,
//!   never mined) until the test releases it: a broadcast that never reached the network, or a
//!   donation its recipient broadcasts later;
//! - an evicted transaction is gone from every mempool;
//! - every request is logged, and one no route answers is recorded as unmatched, which
//!   [`ScriptedChain::assert_all_matched`] (and dropping the chain) turns into a test failure, so
//!   a dependency upgrade that changes what the wallet asks shows up instead of silently covering
//!   less.

use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use bdk_wallet::bitcoin::{
    Amount, BlockHash, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid,
    Witness,
    absolute::LockTime,
    consensus::{deserialize, serialize},
    constants::genesis_block,
    hashes::{Hash, sha256, sha256d},
    transaction::Version,
};
use serde_json::{Value as Json, json};

/// Where the scripted RGB proxy answers.
pub(crate) const PROXY_PATH: &str = "/json-rpc";

const BLOCK_INTERVAL: u64 = 600;
const PAGE: usize = 25;

struct Block {
    hash: BlockHash,
    time: u64,
    txids: Vec<Txid>,
}

struct Fault {
    method: String,
    path: String,
    status: u16,
    // the request takes effect before the fault answers
    relay: bool,
    remaining: usize,
}

#[derive(Default)]
struct Proxy {
    acks: HashMap<String, bool>,
    posted: Vec<String>,
    methods: Vec<String>,
}

struct State {
    blocks: Vec<Block>,
    mempool: Vec<Txid>,
    txs: HashMap<Txid, Transaction>,
    confirmed_at: HashMap<Txid, u32>,
    // outputs the chain never saw created: the inputs of funding transactions
    foreign: HashMap<OutPoint, TxOut>,
    withheld: HashSet<Txid>,
    withhold_broadcasts: usize,
    faults: Vec<Fault>,
    log: Vec<String>,
    unmatched: Vec<String>,
    proxy: Proxy,
    funding_nonce: u32,
}

impl State {
    fn new() -> Self {
        let genesis = genesis_block(Network::Regtest);
        Self {
            blocks: vec![Block {
                hash: genesis.block_hash(),
                time: genesis.header.time as u64,
                txids: vec![],
            }],
            mempool: vec![],
            txs: HashMap::new(),
            confirmed_at: HashMap::new(),
            foreign: HashMap::new(),
            withheld: HashSet::new(),
            withhold_broadcasts: 0,
            faults: vec![],
            log: vec![],
            unmatched: vec![],
            proxy: Proxy::default(),
            funding_nonce: 0,
        }
    }

    fn tip(&self) -> u32 {
        (self.blocks.len() - 1) as u32
    }

    fn visible(&self, txid: &Txid) -> bool {
        self.txs.contains_key(txid) && !self.withheld.contains(txid)
    }

    fn prevout(&self, outpoint: &OutPoint) -> Option<TxOut> {
        if let Some(txout) = self.foreign.get(outpoint) {
            return Some(txout.clone());
        }
        self.txs
            .get(&outpoint.txid)
            .and_then(|tx| tx.output.get(outpoint.vout as usize).cloned())
    }

    fn status_json(&self, txid: &Txid) -> Json {
        match self.confirmed_at.get(txid) {
            Some(height) => {
                let block = &self.blocks[*height as usize];
                json!({
                    "confirmed": true,
                    "block_height": height,
                    "block_hash": block.hash.to_string(),
                    "block_time": block.time,
                })
            }
            None => json!({"confirmed": false}),
        }
    }

    fn tx_json(&self, txid: &Txid) -> Json {
        let tx = &self.txs[txid];
        let mut value_in = 0u64;
        let vin: Vec<Json> = tx
            .input
            .iter()
            .map(|input| {
                let prevout = self.prevout(&input.previous_output);
                if let Some(prevout) = &prevout {
                    value_in += prevout.value.to_sat();
                }
                json!({
                    "txid": input.previous_output.txid.to_string(),
                    "vout": input.previous_output.vout,
                    "prevout": prevout.map(|p| json!({
                        "value": p.value.to_sat(),
                        "scriptpubkey": hex::encode(p.script_pubkey.as_bytes()),
                    })),
                    "scriptsig": hex::encode(input.script_sig.as_bytes()),
                    "witness": input.witness.iter().map(hex::encode).collect::<Vec<_>>(),
                    "sequence": input.sequence.0,
                    "is_coinbase": false,
                })
            })
            .collect();
        let value_out: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
        json!({
            "txid": txid.to_string(),
            "version": tx.version.0,
            "locktime": tx.lock_time.to_consensus_u32(),
            "vin": vin,
            "vout": tx.output.iter().map(|o| json!({
                "value": o.value.to_sat(),
                "scriptpubkey": hex::encode(o.script_pubkey.as_bytes()),
            })).collect::<Vec<_>>(),
            "size": serialize(tx).len(),
            "weight": tx.weight().to_wu(),
            "status": self.status_json(txid),
            "fee": value_in.saturating_sub(value_out),
        })
    }

    fn block_json(&self, height: u32) -> Json {
        let block = &self.blocks[height as usize];
        json!({
            "id": block.hash.to_string(),
            "height": height,
            "version": 0x2000_0000,
            "timestamp": block.time,
            "tx_count": block.txids.len() + 1,
            "size": 285,
            "weight": 1140,
            "merkle_root": "0000000000000000000000000000000000000000000000000000000000000000",
            "previousblockhash": height
                .checked_sub(1)
                .map(|h| self.blocks[h as usize].hash.to_string()),
            "mediantime": block.time,
            "nonce": 0,
            "bits": 0x207f_ffff,
            "difficulty": 0.0,
        })
    }

    /// The visible transactions that pay to or spend from a script, as Esplora lists them:
    /// the mempool first, newest first, then the confirmed ones, newest first.
    fn history(&self, scripthash: &str) -> (Vec<Txid>, Vec<Txid>) {
        let involves = |txid: &Txid| {
            let tx = &self.txs[txid];
            tx.output
                .iter()
                .any(|o| script_hash(&o.script_pubkey) == scripthash)
                || tx.input.iter().any(|i| {
                    self.prevout(&i.previous_output)
                        .is_some_and(|p| script_hash(&p.script_pubkey) == scripthash)
                })
        };
        let mempool = self
            .mempool
            .iter()
            .rev()
            .filter(|t| self.visible(t) && involves(t))
            .copied()
            .collect();
        let chain = self
            .blocks
            .iter()
            .rev()
            .flat_map(|b| b.txids.iter().rev())
            .filter(|t| self.visible(t) && involves(t))
            .copied()
            .collect();
        (mempool, chain)
    }

    fn spender(&self, outpoint: &OutPoint) -> Option<(Txid, usize)> {
        let mut candidates: Vec<&Txid> = self
            .blocks
            .iter()
            .flat_map(|b| b.txids.iter())
            .chain(self.mempool.iter())
            .collect();
        candidates.retain(|t| self.visible(t));
        candidates.into_iter().find_map(|txid| {
            self.txs[txid]
                .input
                .iter()
                .position(|i| i.previous_output == *outpoint)
                .map(|vin| (*txid, vin))
        })
    }

    fn outspend_json(&self, outpoint: &OutPoint) -> Json {
        match self.spender(outpoint) {
            Some((txid, vin)) => json!({
                "spent": true,
                "txid": txid.to_string(),
                "vin": vin,
                "status": self.status_json(&txid),
            }),
            None => json!({"spent": false}),
        }
    }

    fn broadcast(&mut self, body: &[u8]) -> Response {
        let Ok(raw) = hex::decode(String::from_utf8_lossy(body).trim()) else {
            return Response::text(400, "invalid hex");
        };
        let Ok(tx) = deserialize::<Transaction>(&raw) else {
            return Response::text(400, "TX decode failed");
        };
        let txid = tx.compute_txid();
        if self.txs.contains_key(&txid) {
            if self.confirmed_at.contains_key(&txid) {
                return Response::text(400, "Transaction outputs already in utxo set");
            }
            return Response::text(200, &txid.to_string());
        }
        for input in &tx.input {
            if self.prevout(&input.previous_output).is_none() {
                return Response::text(400, "bad-txns-inputs-missingorspent");
            }
            if self.spender(&input.previous_output).is_some() {
                return Response::text(400, "txn-mempool-conflict");
            }
        }
        self.txs.insert(txid, tx);
        self.mempool.push(txid);
        if self.withhold_broadcasts > 0 {
            self.withhold_broadcasts -= 1;
            self.withheld.insert(txid);
        }
        Response::text(200, &txid.to_string())
    }

    fn esplora(&mut self, method: &str, path: &str, body: &[u8]) -> Option<Response> {
        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let txid_at = |i: usize| Txid::from_str(segments[i]).ok();
        Some(match (method, segments.as_slice()) {
            ("GET", ["block-height", height]) => {
                let height: u32 = height.parse().ok()?;
                match self.blocks.get(height as usize) {
                    Some(block) => Response::text(200, &block.hash.to_string()),
                    None => Response::text(404, "Block not found"),
                }
            }
            ("GET", ["blocks", "tip", "height"]) => Response::text(200, &self.tip().to_string()),
            ("GET", ["blocks", "tip", "hash"]) => {
                Response::text(200, &self.blocks[self.tip() as usize].hash.to_string())
            }
            ("GET", ["blocks"]) | ("GET", ["blocks", _]) => {
                let start = match segments.get(1) {
                    Some(h) => h.parse::<u32>().ok()?.min(self.tip()),
                    None => self.tip(),
                };
                let blocks: Vec<Json> = (0..=start)
                    .rev()
                    .take(10)
                    .map(|h| self.block_json(h))
                    .collect();
                Response::json(200, &Json::Array(blocks))
            }
            ("GET", ["scripthash", scripthash, "txs"]) => {
                let (mempool, chain) = self.history(scripthash);
                let txs: Vec<Json> = mempool
                    .iter()
                    .chain(chain.iter().take(PAGE))
                    .map(|t| self.tx_json(t))
                    .collect();
                Response::json(200, &Json::Array(txs))
            }
            ("GET", ["scripthash", scripthash, "txs", "chain", last]) => {
                let last = Txid::from_str(last).ok()?;
                let (_, chain) = self.history(scripthash);
                let txs: Vec<Json> = chain
                    .iter()
                    .skip_while(|t| **t != last)
                    .skip(1)
                    .take(PAGE)
                    .map(|t| self.tx_json(t))
                    .collect();
                Response::json(200, &Json::Array(txs))
            }
            ("GET", ["scripthash", scripthash, "txs", "mempool"]) => {
                let (mempool, _) = self.history(scripthash);
                let txs: Vec<Json> = mempool.iter().map(|t| self.tx_json(t)).collect();
                Response::json(200, &Json::Array(txs))
            }
            ("GET", ["tx", _]) => {
                let txid = txid_at(1)?;
                if self.visible(&txid) {
                    Response::json(200, &self.tx_json(&txid))
                } else {
                    Response::text(404, "Transaction not found")
                }
            }
            // Esplora answers "unconfirmed" for a TX it does not know, as for one in its mempool
            ("GET", ["tx", _, "status"]) => {
                let txid = txid_at(1)?;
                Response::json(200, &self.status_json(&txid))
            }
            ("GET", ["tx", _, "raw"]) => {
                let txid = txid_at(1)?;
                if self.visible(&txid) {
                    Response::bytes(200, serialize(&self.txs[&txid]))
                } else {
                    Response::text(404, "Transaction not found")
                }
            }
            ("GET", ["tx", _, "hex"]) => {
                let txid = txid_at(1)?;
                if self.visible(&txid) {
                    Response::text(200, &hex::encode(serialize(&self.txs[&txid])))
                } else {
                    Response::text(404, "Transaction not found")
                }
            }
            ("GET", ["tx", _, "outspends"]) => {
                let txid = txid_at(1)?;
                if !self.visible(&txid) {
                    return Some(Response::json(200, &json!([])));
                }
                let outspends: Vec<Json> = (0..self.txs[&txid].output.len())
                    .map(|vout| self.outspend_json(&OutPoint::new(txid, vout as u32)))
                    .collect();
                Response::json(200, &Json::Array(outspends))
            }
            ("GET", ["tx", _, "outspend", vout]) => {
                let txid = txid_at(1)?;
                let vout: u32 = vout.parse().ok()?;
                Response::json(200, &self.outspend_json(&OutPoint::new(txid, vout)))
            }
            ("POST", ["tx"]) => self.broadcast(body),
            ("GET", ["fee-estimates"]) => Response::json(
                200,
                &json!({"1": 2.0, "2": 2.0, "3": 2.0, "6": 1.0, "144": 1.0}),
            ),
            _ => return None,
        })
    }

    fn proxy(&mut self, content_type: &str, body: &[u8]) -> Response {
        let (method, params): (String, Json) = if content_type.starts_with("application/json") {
            let Ok(request) = serde_json::from_slice::<Json>(body) else {
                return Response::text(400, "invalid JSON");
            };
            (
                request["method"].as_str().unwrap_or_default().to_string(),
                request["params"].clone(),
            )
        } else {
            let fields = multipart_fields(content_type, body);
            (
                fields.get("method").cloned().unwrap_or_default(),
                fields
                    .get("params")
                    .and_then(|p| serde_json::from_str(p).ok())
                    .unwrap_or(Json::Null),
            )
        };
        self.proxy.methods.push(method.clone());
        let recipient_id = params["recipient_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let result = match method.as_str() {
            "server.info" => json!({"protocol_version": "0.2", "version": "0.2.1", "uptime": 1}),
            "ack.get" => match self.proxy.acks.get(&recipient_id) {
                Some(ack) => json!(ack),
                None => Json::Null,
            },
            "consignment.post" => {
                self.proxy.posted.push(recipient_id);
                json!(true)
            }
            "media.post" | "ack.post" => json!(true),
            "consignment.get" | "media.get" => Json::Null,
            _ => {
                self.unmatched.push(format!("proxy {method}"));
                return Response::text(404, "unknown method");
            }
        };
        Response::json(
            200,
            &json!({"jsonrpc": "2.0", "id": null, "result": result, "error": null}),
        )
    }

    fn handle(&mut self, method: &str, path: &str, content_type: &str, body: &[u8]) -> Response {
        let line = format!("{method} {path}");
        if path != PROXY_PATH {
            self.log.push(line.clone());
        }
        let fault = self
            .faults
            .iter_mut()
            .find(|f| f.remaining > 0 && f.method == method && f.path == path)
            .map(|f| {
                f.remaining -= 1;
                (f.status, f.relay)
            });
        if let Some((status, false)) = fault {
            return Response::text(status, "scripted fault");
        }
        let response = if path == PROXY_PATH && method == "POST" {
            self.proxy(content_type, body)
        } else {
            match self.esplora(method, path, body) {
                Some(response) => response,
                None => {
                    self.unmatched.push(line);
                    Response::text(404, "no scripted route")
                }
            }
        };
        match fault {
            Some((status, true)) => Response::text(status, "scripted fault"),
            _ => response,
        }
    }
}

fn script_hash(script: &ScriptBuf) -> String {
    format!("{:x}", sha256::Hash::hash(script.as_bytes()))
}

/// The text fields of a `multipart/form-data` body (the file part is skipped).
fn multipart_fields(content_type: &str, body: &[u8]) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let Some(boundary) = content_type.split("boundary=").nth(1) else {
        return fields;
    };
    let delimiter = format!("--{}", boundary.trim_matches('"'));
    let body = String::from_utf8_lossy(body);
    for part in body.split(delimiter.as_str()) {
        let Some((headers, value)) = part.split_once("\r\n\r\n") else {
            continue;
        };
        let Some(name) = headers
            .split("name=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
        else {
            continue;
        };
        if headers.contains("filename=") {
            continue;
        }
        fields.insert(name.to_string(), value.trim_end_matches("\r\n").to_string());
    }
    fields
}

struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain",
            body: body.as_bytes().to_vec(),
        }
    }

    fn json(status: u16, body: &Json) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.to_string().into_bytes(),
        }
    }

    fn bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: "application/octet-stream",
            body,
        }
    }
}

fn read_body(reader: &mut BufReader<TcpStream>, headers: &HashMap<String, String>) -> Vec<u8> {
    if headers
        .get("transfer-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case("chunked"))
    {
        let mut body = vec![];
        loop {
            let mut size = String::new();
            if reader.read_line(&mut size).unwrap_or(0) == 0 {
                break;
            }
            let size = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16)
                .unwrap_or(0);
            if size == 0 {
                let mut trailer = String::new();
                let _ = reader.read_line(&mut trailer);
                break;
            }
            let mut chunk = vec![0; size];
            if reader.read_exact(&mut chunk).is_err() {
                break;
            }
            body.extend(chunk);
            let mut crlf = [0; 2];
            let _ = reader.read_exact(&mut crlf);
        }
        body
    } else {
        let length = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0; length];
        let _ = reader.read_exact(&mut body);
        body
    }
}

fn serve(stream: TcpStream, state: Arc<Mutex<State>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return;
    };
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let body = read_body(&mut reader, &headers);
    let path = target.split('?').next().unwrap_or(target);
    let content_type = headers.get("content-type").cloned().unwrap_or_default();
    let response = state
        .lock()
        .unwrap()
        .handle(method, path, &content_type, &body);
    let mut stream = stream;
    let head = format!(
        "HTTP/1.1 {} Scripted\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&response.body);
    let _ = stream.flush();
}

/// A scripted chain and RGB proxy, served until dropped.
pub(crate) struct ScriptedChain {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

impl ScriptedChain {
    pub(crate) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_state, thread_stop) = (state.clone(), stop.clone());
        thread::spawn(move || {
            for stream in listener.incoming() {
                if thread_stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let state = thread_state.clone();
                thread::spawn(move || serve(stream, state));
            }
        });
        Self { addr, state, stop }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// The indexer URL.
    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The RGB proxy as a transport endpoint (what an invoice carries).
    pub(crate) fn proxy_endpoint(&self) -> String {
        format!("rpc://{}{PROXY_PATH}", self.addr)
    }

    /// Pay `amount` sats to `address` from outside the wallet (in the mempool until mined).
    pub(crate) fn fund(&self, address: &str, amount: u64) -> Txid {
        let script = BdkAddress::from_str(address)
            .unwrap()
            .assume_checked()
            .script_pubkey();
        let mut state = self.state();
        state.funding_nonce += 1;
        let foreign = OutPoint::new(
            Txid::from_byte_array(
                sha256d::Hash::hash(&state.funding_nonce.to_le_bytes()).to_byte_array(),
            ),
            0,
        );
        state.foreign.insert(
            foreign,
            TxOut {
                value: Amount::from_sat(amount + 1000),
                script_pubkey: ScriptBuf::new_op_return([0u8; 4]),
            },
        );
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: foreign,
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::from_slice(&[[0u8; 64]]),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(amount),
                script_pubkey: script,
            }],
        };
        let txid = tx.compute_txid();
        state.txs.insert(txid, tx);
        state.mempool.push(txid);
        txid
    }

    /// Mine `count` blocks, the first with every transaction the indexer knows in its mempool.
    pub(crate) fn mine(&self, count: u32) {
        let mut state = self.state();
        for _ in 0..count {
            let height = state.blocks.len() as u32;
            let (mined, kept): (Vec<Txid>, Vec<Txid>) = std::mem::take(&mut state.mempool)
                .into_iter()
                .partition(|t| !state.withheld.contains(t));
            state.mempool = kept;
            for txid in &mined {
                state.confirmed_at.insert(*txid, height);
            }
            let previous = state.blocks.last().unwrap();
            let hash = BlockHash::from_byte_array(
                sha256d::Hash::hash(&[previous.hash.as_byte_array(), &b"scripted"[..]].concat())
                    .to_byte_array(),
            );
            let time = previous.time + BLOCK_INTERVAL;
            state.blocks.push(Block {
                hash,
                time,
                txids: mined,
            });
        }
    }

    /// Whether the indexer knows `txid` (in its mempool or in a block).
    pub(crate) fn knows(&self, txid: &str) -> bool {
        self.state().visible(&Txid::from_str(txid).unwrap())
    }

    /// Whether `txid` is in a block.
    pub(crate) fn is_confirmed(&self, txid: &str) -> bool {
        self.state()
            .confirmed_at
            .contains_key(&Txid::from_str(txid).unwrap())
    }

    /// The next `count` broadcasts are accepted but never reach the indexer.
    pub(crate) fn withhold_broadcasts(&self, count: usize) {
        self.state().withhold_broadcasts = count;
    }

    /// The indexer starts knowing a withheld transaction (its mempool).
    pub(crate) fn release(&self, txid: &str) {
        self.state().withheld.remove(&Txid::from_str(txid).unwrap());
    }

    /// A transaction in the mempool leaves every mempool.
    pub(crate) fn evict(&self, txid: &str) {
        let txid = Txid::from_str(txid).unwrap();
        let mut state = self.state();
        assert!(
            !state.confirmed_at.contains_key(&txid),
            "a mined TX cannot be evicted"
        );
        state.mempool.retain(|t| *t != txid);
        state.txs.remove(&txid);
    }

    /// Answer the next `times` requests `method path` with `status`; with `relay`, after the
    /// request took effect.
    pub(crate) fn fault(&self, method: &str, path: &str, status: u16, relay: bool, times: usize) {
        self.state().faults.push(Fault {
            method: method.to_string(),
            path: path.to_string(),
            status,
            relay,
            remaining: times,
        });
    }

    /// The RGB proxy's ACK (or NACK) for a recipient.
    pub(crate) fn set_ack(&self, recipient_id: &str, ack: bool) {
        self.state()
            .proxy
            .acks
            .insert(recipient_id.to_string(), ack);
    }

    /// The requests the indexer received, as `METHOD /path` (the proxy's are not among them).
    pub(crate) fn requests(&self) -> Vec<String> {
        self.state().log.clone()
    }

    /// Forget the requests received so far.
    pub(crate) fn clear_requests(&self) {
        self.state().log.clear();
    }

    /// The JSON-RPC methods the RGB proxy received.
    pub(crate) fn proxy_methods(&self) -> Vec<String> {
        self.state().proxy.methods.clone()
    }

    pub(crate) fn assert_all_matched(&self) {
        let unmatched = self.state().unmatched.clone();
        assert!(
            unmatched.is_empty(),
            "requests no scripted route answers: {unmatched:?}"
        );
    }
}

impl Drop for ScriptedChain {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // wake the accept loop so that it sees the flag
        let _ = TcpStream::connect(self.addr);
        if !thread::panicking() {
            self.assert_all_matched();
        }
    }
}

// Wallets on the scripted chain: an issuer of an NIA asset and its sends, as the fork's tests of
// CC-99 and the self-tests below use them

pub(crate) const FUNDING: u64 = 100_000_000;
pub(crate) const WITNESS_SATS: u64 = 1000;
pub(crate) const UTXOS: u8 = 4;
pub(crate) const UTXO_SATS: u32 = 100_000;

/// A funded wallet with colored UTXOs and an issued NIA asset, online on `chain`.
pub(crate) struct Issuer {
    pub(crate) wallet: Wallet,
    pub(crate) online: Online,
    pub(crate) asset_id: String,
}

pub(crate) fn online_options(chain: &ScriptedChain) -> OnlineOptions {
    OnlineOptions {
        indexer_url: chain.url(),
        skip_consistency_check: false,
        vanilla_sync_lookback: INDEXER_SYNC_LOOKBACK as u32,
        forwarder_url: None,
    }
}

pub(crate) fn issuer(chain: &ScriptedChain, amounts: Vec<u64>) -> Issuer {
    let mut wallet = get_test_wallet(true, None);
    let online = wallet.go_online(online_options(chain)).unwrap();
    chain.fund(&wallet.get_address().unwrap(), FUNDING);
    chain.mine(1);
    wallet
        .create_utxos(online, false, Some(UTXOS), Some(UTXO_SATS), FEE_RATE, false)
        .unwrap();
    chain.mine(1);
    let asset_id = wallet
        .issue_asset_nia(TICKER.to_string(), NAME.to_string(), PRECISION, amounts)
        .unwrap()
        .asset_id;
    Issuer {
        wallet,
        online,
        asset_id,
    }
}

/// A witness recipient on an address of a fresh wallet.
pub(crate) fn witness_recipient(chain: &ScriptedChain, amount: u64) -> Recipient {
    let mut other = get_test_wallet(false, None);
    let script = BdkAddress::from_str(&other.get_address().unwrap())
        .unwrap()
        .assume_checked()
        .script_pubkey();
    Recipient {
        recipient_id: recipient_id_from_script_buf(script, BitcoinNetwork::Regtest),
        witness_data: Some(WitnessData {
            amount_sat: WITNESS_SATS,
            blinding: None,
        }),
        assignment: Assignment::Fungible(amount),
        transport_endpoints: vec![chain.proxy_endpoint()],
    }
}

/// `send_begin` of `amount` of the issued asset to a fresh witness recipient, signed.
pub(crate) fn begin_send(
    chain: &ScriptedChain,
    party: &mut Issuer,
    amount: u64,
    donation: bool,
) -> (SendBeginResult, String, Recipient) {
    let recipient = witness_recipient(chain, amount);
    let begin = party
        .wallet
        .send_begin(
            party.online,
            HashMap::from([(party.asset_id.clone(), vec![recipient.clone()])]),
            donation,
            FEE_RATE,
            MIN_CONFIRMATIONS,
            (now().unix_timestamp() + DURATION_SEND_TRANSFER as i64) as u64,
            false,
            None,
        )
        .unwrap();
    let signed = party.wallet.sign_psbt(begin.psbt.clone(), None).unwrap();
    (begin, signed, recipient)
}

pub(crate) fn psbt_txid(psbt: &str) -> String {
    Psbt::from_str(psbt)
        .unwrap()
        .unsigned_tx
        .compute_txid()
        .to_string()
}

pub(crate) fn status_of(wallet: &Wallet, batch_transfer_idx: i32) -> TransferStatus {
    wallet
        .list_transfers(AssetFilter::AnyOrNone, None)
        .unwrap()
        .into_iter()
        .find(|t| t.batch_transfer_idx == batch_transfer_idx)
        .unwrap()
        .status
}

pub(crate) fn spendable(wallet: &Wallet, asset_id: &str) -> u64 {
    wallet
        .get_asset_balance(asset_id.to_string())
        .unwrap()
        .spendable
}

mod tests {
    use super::*;

    #[test]
    #[parallel]
    fn carries_a_donation_to_settled() {
        let chain = ScriptedChain::start();
        let mut party = issuer(&chain, vec![AMOUNT]);
        assert_eq!(spendable(&party.wallet, &party.asset_id), AMOUNT);

        let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
        let idx = begin.batch_transfer_idx.unwrap();
        let result = party.wallet.send_end(party.online, signed).unwrap();
        assert!(chain.knows(&result.txid));
        assert_eq!(
            status_of(&party.wallet, idx),
            TransferStatus::WaitingConfirmations
        );
        assert!(chain.proxy_methods().contains(&s!("consignment.post")));

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

        // the change is spendable again: a second send from it goes through
        let (_, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
        let result = party.wallet.send_end(party.online, signed).unwrap();
        assert!(chain.knows(&result.txid));
        chain.assert_all_matched();
    }

    #[test]
    #[parallel]
    fn broadcasts_a_send_once_its_recipient_acks() {
        let chain = ScriptedChain::start();
        let mut party = issuer(&chain, vec![AMOUNT]);
        let (begin, signed, recipient) = begin_send(&chain, &mut party, AMOUNT_SMALL, false);
        let idx = begin.batch_transfer_idx.unwrap();
        let txid = psbt_txid(&signed);
        party.wallet.send_end(party.online, signed).unwrap();
        assert_eq!(
            status_of(&party.wallet, idx),
            TransferStatus::WaitingCounterparty
        );
        assert!(!chain.knows(&txid));

        // no ACK yet: refresh asks and waits
        party
            .wallet
            .refresh(party.online, None, vec![], false)
            .unwrap();
        assert_eq!(
            status_of(&party.wallet, idx),
            TransferStatus::WaitingCounterparty
        );
        assert!(chain.proxy_methods().contains(&s!("ack.get")));

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
        assert!(chain.is_confirmed(&txid));
        party
            .wallet
            .refresh(party.online, None, vec![], false)
            .unwrap();
        assert_eq!(status_of(&party.wallet, idx), TransferStatus::Settled);
    }

    // CC-99 S1, as upstream rgb-lib meets it: the broadcast reaches the network, its answer and the
    // lookup that follows are lost, send_end rolls back, and the next go_online refuses the wallet
    #[test]
    #[parallel]
    fn loses_a_relayed_broadcast_answer() {
        let chain = ScriptedChain::start();
        let mut party = issuer(&chain, vec![AMOUNT]);
        let (begin, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
        let idx = begin.batch_transfer_idx.unwrap();
        let txid = psbt_txid(&signed);
        chain.fault("POST", "/tx", 502, true, 1);
        chain.fault("GET", &format!("/tx/{txid}/status"), 502, false, 1);
        chain.clear_requests();

        let result = party.wallet.send_end(party.online, signed);
        assert_matches!(result, Err(Error::Indexer { .. }));
        assert_eq!(
            chain.requests(),
            vec![s!("POST /tx"), format!("GET /tx/{txid}/status")]
        );
        assert!(chain.knows(&txid));
        assert_eq!(status_of(&party.wallet, idx), TransferStatus::Initiated);

        party.wallet.go_offline();
        let result = party.wallet.go_online(online_options(&chain));
        assert_matches!(result, Err(Error::Inconsistency { .. }));
    }

    #[test]
    #[parallel]
    fn withholds_releases_and_evicts() {
        let chain = ScriptedChain::start();
        let mut party = issuer(&chain, vec![AMOUNT]);

        // a broadcast that never reaches the indexer, until released
        let (_, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
        let withheld = psbt_txid(&signed);
        chain.withhold_broadcasts(1);
        party.wallet.send_end(party.online, signed).unwrap();
        assert!(!chain.knows(&withheld));
        chain.mine(1);
        assert!(!chain.is_confirmed(&withheld));
        chain.release(&withheld);
        assert!(chain.knows(&withheld));
        chain.mine(1);
        assert!(chain.is_confirmed(&withheld));

        // a TX that leaves every mempool
        party
            .wallet
            .refresh(party.online, None, vec![], false)
            .unwrap();
        let (_, signed, _) = begin_send(&chain, &mut party, AMOUNT_SMALL, true);
        let evicted = psbt_txid(&signed);
        party.wallet.send_end(party.online, signed).unwrap();
        assert!(chain.knows(&evicted));
        chain.evict(&evicted);
        assert!(!chain.knows(&evicted));
    }
}
