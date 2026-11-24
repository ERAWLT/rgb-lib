//! ERA fork: the fork's own paths on upstream's regtest services (`tests/regtest.sh`: bitcoind,
//! Esplora, the RGB proxy 0.4.0), run by era.yml's `era-regtest` job: the completion of an own
//! spend whose record was lost (CC-99, `OnlineOptions::complete_unrecorded_spends`), the
//! consistency check's excuse of an interrupted send (D63), a send the `proxy_v1` migration
//! failed while its TX was on chain (review N5), a replaced payment of envelopes (CC-115), and a
//! received BFA transfer waiting for an EVM chain whose endpoint is down (S2,
//! `OnlineOptions::defer_unavailable_evm_chains`).
//! The scripted chain (`unrecorded_spends`) tests the same verdicts without services; these run
//! them on a real node, indexer and proxy. The wallets use Esplora, as the ERA app does, and go
//! online with the consistency check, as the app's bridge does (upstream's tests skip it).
//! See the fork's `ERA.md`.

use super::*;

/// The options the app's bridge goes online with: Esplora, the consistency check, and the
/// completion as `complete` says.
fn era_options(complete: bool) -> OnlineOptions {
    OnlineOptions {
        skip_consistency_check: false,
        complete_unrecorded_spends: complete,
        ..test_go_online_options(Some(ESPLORA_URL))
    }
}

/// The wallet as the app reopens it after a pause: offline, then online again.
fn reopen(party: &mut SinglesigParty, complete: bool) -> Result<(), Error> {
    party.wallet.go_offline();
    party.online = party.wallet.go_online(era_options(complete))?;
    Ok(())
}

fn funded() -> SinglesigParty {
    let mut party = get_funded_party!(ESPLORA_URL.to_string());
    party.wallet.go_offline();
    party.online = party.wallet.go_online(era_options(true)).unwrap();
    party
}

/// Wait until the regtest Esplora knows `txid` (in its mempool or in a block), or does not.
fn wait_for_esplora(party: &SinglesigParty, txid: &str, known: bool) {
    let found = wait_for_function(
        || {
            party
                .wallet
                .indexer()
                .get_tx_confirmations(txid)
                .is_ok_and(|c| c.is_some() == known)
        },
        60,
        500,
    );
    assert!(found, "Esplora does not come to know={known} TX {txid}");
}

fn batch_transfer_status(party: &SinglesigParty, idx: i32) -> TransferStatus {
    party
        .wallet
        .list_transfers(AssetFilter::AnyOrNone, TransferScope::All)
        .unwrap()
        .into_iter()
        .find(|t| t.batch_transfer_idx == idx)
        .unwrap()
        .status
}

/// The one spend the last go_online completed (its confirmations depend on the blocks other
/// tests mine meanwhile, so they are not compared).
fn assert_completed(party: &SinglesigParty, txid: &str, idx: Option<i32>, donation: bool) {
    let completed = party.wallet.completed_spends();
    assert_eq!(completed.len(), 1, "{completed:?}");
    let spend = &completed[0];
    assert_eq!(spend.txid, txid);
    assert_eq!(spend.batch_transfer_idx, idx);
    assert_eq!(
        spend.previous_status,
        idx.map(|_| TransferStatus::Initiated)
    );
    assert_eq!(spend.donation, donation);
}

/// `send_begin` (persisted) of `amount` of `asset_id` to a blind receive of `rcv_party`, signed:
/// (batch transfer, signed PSBT, TXID, recipient ID).
fn begin_send(
    party: &mut SinglesigParty,
    rcv_party: &mut SinglesigParty,
    asset_id: &str,
    amount: u64,
    donation: bool,
) -> (i32, String, String, String) {
    let receive_data = rcv_party.blind_receive();
    let begin = party
        .wallet
        .send_begin(
            party.online,
            blind_recipient_map(asset_id, &receive_data, Assignment::Fungible(amount)),
            donation,
            FEE_RATE,
            MIN_CONFIRMATIONS,
            default_send_expiration(),
            Some(PROXY_ENDPOINT.clone()),
            false,
        )
        .unwrap();
    let idx = begin.batch_transfer_idx.unwrap();
    let signed = party.wallet.sign_psbt(begin.psbt, None).unwrap();
    let txid = psbt_txid(&signed);
    (idx, signed, txid, receive_data.recipient_id)
}

/// `send_end` dies after its last step and before its commit (the crash hook): a donation's TX is
/// broadcast, a send's signed TX is on the sender proxy and its consignment posted.
fn crash_send_end(party: &mut SinglesigParty, signed: &str, idx: i32) {
    MOCK_SEND_END_CRASH.replace(Some(()));
    let result = party.wallet.send_end(party.online, signed.to_string());
    assert_matches!(result, Err(Error::Internal { .. }));
    assert_eq!(batch_transfer_status(party, idx), TransferStatus::Initiated);
}

/// Refresh the receive of `recipient_id` until it is in `status`.
fn refresh_receive_until(party: &mut SinglesigParty, recipient_id: &str, status: TransferStatus) {
    let reached = wait_for_function(
        || {
            let _ = party.wallet.refresh(party.online, None, vec![], false);
            party.transfer_status_recipient(recipient_id) == status
        },
        90,
        1000,
    );
    assert!(
        reached,
        "the receive is {:?}, not {status:?}",
        party.transfer_status_recipient(recipient_id)
    );
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
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

// (a) CC-99 S2: a donation whose send_end broadcast its TX and died before its commit. Without the
// option, go_online lets the spend of the donation's own TX through (D63) and the donation stays
// Initiated; the first go_online with the option completes it, the recipient receives it, and both
// settle. The go_online after that has nothing to complete.
#[test]
#[parallel]
fn era_lost_donation_completes_at_go_online() {
    initialize();

    let amount: u64 = 66;
    let mut party = funded();
    let mut rcv_party = funded();
    let asset = party.issue_asset_nia(None);
    let (idx, signed, txid, recipient_id) =
        begin_send(&mut party, &mut rcv_party, &asset.asset_id, amount, true);
    crash_send_end(&mut party, &signed, idx);
    wait_for_esplora(&party, &txid, true);

    reopen(&mut party, false).unwrap();
    assert!(party.wallet.completed_spends().is_empty());
    assert_eq!(
        batch_transfer_status(&party, idx),
        TransferStatus::Initiated
    );

    reopen(&mut party, true).unwrap();
    assert_completed(&party, &txid, Some(idx), true);
    assert_eq!(
        batch_transfer_status(&party, idx),
        TransferStatus::WaitingConfirmations
    );

    refresh_receive_until(
        &mut rcv_party,
        &recipient_id,
        TransferStatus::WaitingConfirmations,
    );
    mine(true);
    rcv_party.wait_for_transfer_status_recipient(None, &recipient_id, TransferStatus::Settled);
    party.wait_for_transfer_status_sender(Some(&asset.asset_id), &txid, TransferStatus::Settled);
    assert_eq!(
        party.get_asset_balance(&asset.asset_id).spendable,
        AMOUNT - amount
    );
    assert_eq!(rcv_party.get_asset_balance(&asset.asset_id).settled, amount);

    reopen(&mut party, true).unwrap();
    assert!(party.wallet.completed_spends().is_empty());
}

// (b) CC-99 S3: a drain whose record is lost, as a copy of the wallet taken after drain_to_begin
// and restored after the drain went out leaves it (a seal, a VSS backup). go_online refuses it
// without the option (a drain is not an Initiated transfer) and completes it with the option.
#[test]
#[parallel]
fn era_lost_drain_completes_at_go_online() {
    initialize();

    let mut party = funded();
    let address = get_test_wallet(false, None).get_address().unwrap();
    let psbt = party
        .wallet
        .drain_to_begin(party.online, address, FEE_RATE, false)
        .unwrap();
    let signed = party.wallet.sign_psbt(psbt, None).unwrap();
    let txid = psbt_txid(&signed);
    let wallet_dir = party.wallet.get_wallet_dir();
    let copy = tempfile::tempdir().unwrap();
    copy_dir(&wallet_dir, copy.path());
    party.wallet.drain_to_end(party.online, signed).unwrap();
    wait_for_esplora(&party, &txid, true);

    let (wallet_data, keys) = (party.wallet.get_wallet_data(), party.wallet.get_keys());
    let online = party.online;
    drop(party);
    fs::remove_dir_all(&wallet_dir).unwrap();
    copy_dir(copy.path(), &wallet_dir);
    let mut party = SinglesigParty {
        wallet: Wallet::new(wallet_data, keys).unwrap(),
        online,
    };
    assert_eq!(party.wallet.list_pending_vanilla_txs().unwrap().len(), 1);

    let error = reopen(&mut party, false).unwrap_err();
    assert_matches!(error, Error::Inconsistency { .. });
    assert_eq!(error.inconsistency_reason(), None);

    reopen(&mut party, true).unwrap();
    assert_completed(&party, &txid, None, false);
    assert!(party.wallet.list_pending_vanilla_txs().unwrap().is_empty());
    assert!(
        party
            .wallet
            .list_unspents(None, false, true)
            .unwrap()
            .is_empty()
    );
}

// (c) D63, with the option: on protocol 1.0 a send_end that dies before its commit leaves the send
// Initiated with its signed TX on the sender proxy and its consignment posted. The recipient
// refreshes, ACKs, gets the TX released and broadcasts it. The sender's next go_online completes
// the send, and both settle.
#[test]
#[parallel]
fn era_interrupted_send_completes_at_go_online() {
    initialize();

    let amount: u64 = 66;
    let mut party = funded();
    let mut rcv_party = funded();
    let asset = party.issue_asset_nia(None);
    let (idx, signed, txid, recipient_id) =
        begin_send(&mut party, &mut rcv_party, &asset.asset_id, amount, false);
    crash_send_end(&mut party, &signed, idx);

    refresh_receive_until(
        &mut rcv_party,
        &recipient_id,
        TransferStatus::WaitingConfirmations,
    );
    wait_for_esplora(&party, &txid, true);

    reopen(&mut party, true).unwrap();
    assert_completed(&party, &txid, Some(idx), false);
    assert_eq!(
        batch_transfer_status(&party, idx),
        TransferStatus::WaitingConfirmations
    );

    mine(true);
    rcv_party.wait_for_transfer_status_recipient(None, &recipient_id, TransferStatus::Settled);
    party.wait_for_transfer_status_sender(Some(&asset.asset_id), &txid, TransferStatus::Settled);
    assert_eq!(
        party.get_asset_balance(&asset.asset_id).spendable,
        AMOUNT - amount
    );
    assert_eq!(rcv_party.get_asset_balance(&asset.asset_id).settled, amount);
}

// (c) D63, without the option: the check lets the spend of the interrupted send's own TX through,
// the send stays Initiated, and its send_end, called again with the signed PSBT, ends it; the
// refresh sees the recipient's ACK and both settle.
#[test]
#[parallel]
fn era_interrupted_send_goes_online_and_ends_again_without_the_option() {
    initialize();

    let amount: u64 = 66;
    let mut party = funded();
    let mut rcv_party = funded();
    let asset = party.issue_asset_nia(None);
    let (idx, signed, txid, recipient_id) =
        begin_send(&mut party, &mut rcv_party, &asset.asset_id, amount, false);
    crash_send_end(&mut party, &signed, idx);

    refresh_receive_until(
        &mut rcv_party,
        &recipient_id,
        TransferStatus::WaitingConfirmations,
    );
    wait_for_esplora(&party, &txid, true);

    reopen(&mut party, false).unwrap();
    assert!(party.wallet.completed_spends().is_empty());
    assert_eq!(
        batch_transfer_status(&party, idx),
        TransferStatus::Initiated
    );

    party.wallet.send_end(party.online, signed).unwrap();
    assert_eq!(
        batch_transfer_status(&party, idx),
        TransferStatus::WaitingCounterparty
    );
    mine(true);
    rcv_party.wait_for_transfer_status_recipient(None, &recipient_id, TransferStatus::Settled);
    party.wait_for_transfer_status_sender(Some(&asset.asset_id), &txid, TransferStatus::Settled);
    assert_eq!(
        party.get_asset_balance(&asset.asset_id).spendable,
        AMOUNT - amount
    );
}

// (d) Review N5, staged: beta.8's proxy_v1 migration fails every outgoing transfer still Initiated
// or WaitingCounterparty, as their 0.2 data no longer parses. One whose TX was already on chain is
// staged here with the migration's own write (the status set to Failed) on a donation whose
// send_end died after its broadcast. go_online refuses it, with nothing written: with the option as
// UnrecordedSpend { transfer-failed } naming the transfer, without it as upstream (a Failed transfer
// is not excused). ERA.md says how such a wallet is recovered.
#[test]
#[parallel]
fn era_a_send_the_migration_failed_with_its_tx_on_chain_is_refused() {
    initialize();

    let mut party = funded();
    let mut rcv_party = funded();
    let asset = party.issue_asset_nia(None);
    let (idx, signed, txid, _) = begin_send(&mut party, &mut rcv_party, &asset.asset_id, 66, true);
    crash_send_end(&mut party, &signed, idx);
    wait_for_esplora(&party, &txid, true);

    let txn = party.wallet.database().begin_transaction().unwrap();
    let batch_transfer = txn
        .iter_batch_transfers()
        .unwrap()
        .into_iter()
        .find(|b| b.idx == idx)
        .unwrap();
    let mut failed: DbBatchTransferActMod = batch_transfer.into();
    failed.status = ActiveValue::Set(TransferStatus::Failed);
    txn.update_batch_transfer(&mut failed).unwrap();
    txn.commit().unwrap();

    // the wallet's database (since beta.8 the RGB stock and BDK's changeset too), which a
    // refusal leaves as it was
    let wallet_dir = party.wallet.get_wallet_dir();
    let digest = || {
        crate::wallet::test::era_fixtures::dir_listing(
            wallet_dir.parent().unwrap(),
            &wallet_dir.file_name().unwrap().to_string_lossy(),
        )
        .lines()
        .filter(|l| l.ends_with("/rgb_lib_db"))
        .collect::<Vec<_>>()
        .join("\n")
    };
    let before = digest();
    for _ in 0..2 {
        let error = reopen(&mut party, true).unwrap_err();
        assert_eq!(
            error,
            Error::UnrecordedSpend {
                txid: txid.clone(),
                reason: s!("transfer-failed"),
                batch_transfer_idx: Some(idx),
            }
        );
        let error = reopen(&mut party, false).unwrap_err();
        assert_matches!(error, Error::Inconsistency { .. });
        assert_eq!(error.inconsistency_reason(), None);
        assert_eq!(batch_transfer_status(&party, idx), TransferStatus::Failed);
        assert_eq!(digest(), before);
    }
}

/// `bitcoin-cli -rpcwallet=miner <args>` on the main regtest node: its output.
fn miner_cli(args: &[&str]) -> String {
    let output = Command::new("docker")
        .stdin(Stdio::null())
        .arg("compose")
        .args(bitcoin_cli())
        .arg("-rpcwallet=miner")
        .args(args)
        .output()
        .expect("failed to call bitcoin-cli");
    assert!(
        output.status.success(),
        "bitcoin-cli {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

// (e) CC-115: a payment of envelopes from outside the wallet (bitcoind's miner wallet) to two
// colored addresses handed out, which the colored payment sync recorded while unconfirmed, is
// replaced by a fee bump paying the same addresses before it confirms. Without the option,
// go_online refuses (the payment's rows exist, BDK no longer holds it); with it, the rows of the
// replaced payment are marked as not existing and the replacement's outputs, which the full scan
// records, are the envelopes. Mining is held while the payment is unconfirmed.
#[test]
#[parallel]
fn era_a_replaced_payment_of_envelopes_lets_go_online_through() {
    initialize();

    let mut wallet = get_test_wallet(true, Some(1));
    let online = wallet.go_online(era_options(true)).unwrap();
    let mut party = SinglesigParty { wallet, online };
    let addresses: Vec<String> = (0..2)
        .map(|_| party.wallet.get_colored_address().unwrap())
        .collect();
    let envelopes = |party: &mut SinglesigParty, txid: &str| -> Vec<Unspent> {
        party
            .wallet
            .list_unspents(None, false, true)
            .unwrap()
            .into_iter()
            .filter(|u| u.utxo.colorable && u.utxo.exists && u.utxo.outpoint.txid == txid)
            .collect()
    };
    let rows = |party: &SinglesigParty, txid: &str| -> Vec<DbTxo> {
        let txn = party.wallet.database().begin_transaction().unwrap();
        txn.iter_txos()
            .unwrap()
            .into_iter()
            .filter(|t| t.txid == txid)
            .collect()
    };

    let guard = stop_mining();
    let amounts = format!(
        "amounts={{\"{}\":0.001,\"{}\":0.001}}",
        addresses[0], addresses[1]
    );
    let first = miner_cli(&["-named", "sendmany", "dummy=", &amounts, "replaceable=true"]);
    wait_for_esplora(&party, &first, true);
    party
        .wallet
        .sync_colored_payments(party.online, vec![])
        .unwrap();
    assert_eq!(envelopes(&mut party, &first).len(), 2);

    // BDK orders unconfirmed conflicts by the second it last saw each
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let bumped: Value = serde_json::from_str(&miner_cli(&["bumpfee", &first])).unwrap();
    let replacement = bumped["txid"].as_str().unwrap().to_string();
    wait_for_esplora(&party, &replacement, true);
    wait_for_esplora(&party, &first, false);

    let error = reopen(&mut party, false).unwrap_err();
    assert_matches!(error, Error::Inconsistency { .. });
    assert_eq!(error.inconsistency_reason(), None);
    assert!(rows(&party, &first).iter().all(|t| t.exists));

    reopen(&mut party, true).unwrap();
    assert!(party.wallet.completed_spends().is_empty());
    let first_rows = rows(&party, &first);
    assert_eq!(first_rows.len(), 2);
    assert!(first_rows.iter().all(|t| !t.exists && !t.spent));
    assert!(envelopes(&mut party, &first).is_empty());
    assert_eq!(envelopes(&mut party, &replacement).len(), 2);
    drop(guard);

    // once mined, the replacement's envelopes stay and the next go_online passes
    mine(true);
    reopen(&mut party, true).unwrap();
    assert_eq!(envelopes(&mut party, &replacement).len(), 2);
    assert!(rows(&party, &first).iter().all(|t| !t.exists));
}

/// An EVM endpoint the test takes down and brings back (S2): a loopback HTTP server that answers
/// every request with a 503 while down and hands it to the regtest anvil node while up, one
/// request per connection.
struct EvmRelay {
    addr: std::net::SocketAddr,
    up: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl EvmRelay {
    fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let up = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let relay_up = up.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let up = relay_up.load(std::sync::atomic::Ordering::SeqCst);
                let _ = Self::serve(stream, up);
            }
        });
        Self { addr, up }
    }

    fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    fn bring_up(&self) {
        self.up.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn serve(stream: std::net::TcpStream, up: bool) -> std::io::Result<()> {
        use std::io::{BufRead, Read, Write};
        let mut reader = std::io::BufReader::new(stream.try_clone()?);
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        let mut stream = stream;
        if !up {
            return stream.write_all(
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
        let node = EVM_RPC_URL.trim_start_matches("http://");
        let mut upstream = std::net::TcpStream::connect(node)?;
        write!(
            upstream,
            "POST / HTTP/1.1\r\nHost: {node}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        upstream.write_all(&body)?;
        let mut answer = vec![];
        upstream.read_to_end(&mut answer)?;
        stream.write_all(&answer)
    }
}

/// S2 (`OnlineOptions::defer_unavailable_evm_chains`): a wallet whose EVM endpoint is down goes
/// online without the chain; a transfer of a minted BFA asset it receives meanwhile waits, neither
/// refused nor failed, its sender waiting too; once the endpoint is back, the next refresh uses
/// the chain, validates the consignment and ACKs it, and the transfer settles.
#[test]
#[parallel]
#[serial(evm)]
fn era_a_bfa_transfer_waits_for_an_evm_chain_down() {
    initialize();

    // a BFA asset minted on anvil's bridge, settled at `holder`
    let token = deploy_test_erc20("Bridged Token", "BRG", 18, 1_000_000);
    let bridge = deploy_bridge(&token.address);
    let mut minter = get_funded_party!();
    let mut holder = get_funded_party!();
    let asset = minter.issue_asset_bfa(2, bridge.address.clone(), None);
    let mint_receive = holder.blind_receive();
    let res = minter.mint_begin(
        minter.online,
        &asset.asset_id,
        Recipient {
            assignment: Assignment::Fungible(AMOUNT),
            recipient_id: mint_receive.recipient_id.clone(),
            witness_data: None,
            transport_endpoints: TRANSPORT_ENDPOINTS.clone(),
            public_key: Some(mint_receive.public_key.clone()),
            expiration_timestamp: None,
            encryption: Encryption::Optional,
        },
    );
    erc20_approve(&token.address, &bridge.address, AMOUNT);
    bridge_funds_in(&bridge.address, AMOUNT, &mint_lock_id(&res));
    holder.refresh_all();
    evm_mine(EVM_FINALITY_DEPTH);
    holder.wait_for_refresh(None);
    let signed = minter.wallet.sign_psbt(res.psbt, None).unwrap();
    minter.mint_end(minter.online, signed);
    mine(false);
    holder.wait_for_transfer_status_recipient(
        None,
        &mint_receive.recipient_id,
        TransferStatus::Settled,
    );
    assert_eq!(holder.get_asset_balance(&asset.asset_id).settled, AMOUNT);

    // the recipient goes online on Esplora, with its EVM endpoint down
    let relay = EvmRelay::start();
    let mut recipient = funded();
    let options = || OnlineOptions {
        evm_chains: test_evm_chains(HashMap::from([(ANVIL_CHAIN_ID, relay.url())])),
        defer_unavailable_evm_chains: true,
        ..era_options(true)
    };
    recipient.wallet.go_offline();
    assert_matches!(
        recipient.wallet.go_online(OnlineOptions {
            defer_unavailable_evm_chains: false,
            ..options()
        }),
        Err(Error::InvalidEvmRpcUrl { .. })
    );
    recipient.online = recipient.wallet.go_online(options()).unwrap();
    let left_out = |party: &SinglesigParty| -> Vec<u64> {
        party
            .wallet
            .unavailable_evm_chains()
            .into_iter()
            .map(|c| c.chain_id)
            .collect()
    };
    assert_eq!(left_out(&recipient), vec![ANVIL_CHAIN_ID]);

    // a transfer of the minted asset (the mint in its history) waits for the chain: no failure, no
    // NACK, at either end
    let receive = recipient.blind_receive();
    let txid = holder.send_retry(&blind_recipient_map(
        &asset.asset_id,
        &receive,
        Assignment::Fungible(AMOUNT / 2),
    ));
    for _ in 0..3 {
        let result = recipient.refresh_result(None, &[]).unwrap();
        let refreshed = &result[&receive.batch_transfer_idx];
        assert!(refreshed.failure.is_none(), "{refreshed:?}");
        assert_eq!(refreshed.updated_status, None);
        holder.refresh_result(None, &[]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    assert_eq!(
        batch_transfer_status(&recipient, receive.batch_transfer_idx),
        TransferStatus::WaitingCounterparty
    );
    assert!(holder.check_test_transfer_status_sender(&txid, TransferStatus::WaitingCounterparty));
    assert_eq!(left_out(&recipient), vec![ANVIL_CHAIN_ID]);

    // the chain is back: the next refresh uses it and the transfer goes through
    relay.bring_up();
    recipient.wait_for_refresh(None);
    assert!(recipient.wallet.unavailable_evm_chains().is_empty());
    holder.wait_for_refresh(None);
    mine(false);
    recipient.wait_for_transfer_status_recipient(
        None,
        &receive.recipient_id,
        TransferStatus::Settled,
    );
    holder.wait_for_transfer_status_sender(None, &txid, TransferStatus::Settled);
    assert_eq!(
        recipient.get_asset_balance(&asset.asset_id).settled,
        AMOUNT / 2
    );
}
