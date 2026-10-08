//! ERA fork (CC-99): completing, at `go_online`, an own colored spend whose record was lost.
//!
//! A colored spend reaches the network while the database does not record it when the operation
//! that broadcast it does not commit: its answer and the lookup after it are lost (S1), the
//! process dies between the broadcast and the commit (S2), or the wallet is restored from a
//! backup taken between the operation's `*_begin` and its commit (S3). The spent inputs are then
//! `exists && !spent` in the database and spent in BDK, and upstream's consistency check refuses
//! every `go_online` with [`Error::Inconsistency`]. Three operations can do it: a donation's
//! `send_end`, the `refresh` that broadcasts an ACKed send, and `drain_to_end`.
//!
//! With `OnlineOptions::complete_unrecorded_spends`, the check hands that verdict here instead.
//! The completion runs only when every divergent coin is spent, in BDK's canonical view after the
//! check's scan, by a transaction that exactly one record of this wallet names (an outgoing batch
//! transfer or a pending drain) and that the indexer knows, and when the record and the files of
//! the transfer match that transaction. It then writes what the operation would have committed,
//! with the operation's own writers ([`WalletOnline::record_broadcast`], `consume_fascia`, the
//! status update, the reservation release), inside the check's database transaction, and checks
//! that no divergence is left. It never broadcasts, never contacts an RGB proxy and never marks a
//! coin spent that such a transaction does not spend. Anything it cannot prove is refused, all or
//! nothing, with nothing written, save one case: rgb-ops commits each fascia on its own, so when
//! the stash refuses a spend (`stash-refused`), the spends of the same plan consumed before it
//! stay in the stash, the database rolled back. That is S2's state (the stash ahead of the
//! database), which the next `go_online` plans again, skipping those bundles. See the fork's
//! `ERA.md`, section 6.
//!
//! [`plan`] only reads (BDK's view, the database, the transfer files and the indexer's answers),
//! so the table of verdicts is tested without a chain; [`apply`] writes.
//!
//! CC-115, the one divergence that is not a spend: an envelope paid from outside the wallet whose
//! payment was replaced before it confirmed (an RBF, or another wallet on the same seed spending
//! the same coins). Its row is `exists && !spent`, BDK no longer lists it, and nothing spends it.
//! When BDK has seen the payment and no longer holds it as canonical (it learnt a conflicting TX,
//! or the payment descends from one: BDK records no eviction, since rgb-lib's requests carry no
//! expected TXIDs), no transfer of this wallet still in play names that TX, and the wallet's
//! records leave the coin empty, the check marks the row as not existing (`exists` false, as a TXO
//! of a TX that has not reached the network is) instead of refusing: out of every selection and of
//! the check, and back as it was once a sync sees the payment canonical again. Any other coin
//! without a spender is refused as before. See the fork's `ERA.md`, section 9.

use super::*;
use crate::wallet::online::SIGNED_PSBT_FILE;

/// ERA fork (CC-99): a spend of this wallet completed by `go_online`
/// (`OnlineOptions::complete_unrecorded_spends`), see
/// [`Wallet::completed_spends`](crate::wallet::Wallet::completed_spends).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "camel_case", serde(rename_all = "camelCase"))]
pub struct CompletedSpend {
    /// ID of the spending transaction
    pub txid: String,
    /// The batch transfer that recorded the spend, `None` for a drain
    pub batch_transfer_idx: Option<i32>,
    /// The status of that batch transfer before the completion, `None` for a drain (the transfer
    /// is now [`TransferStatus::WaitingConfirmations`])
    pub previous_status: Option<TransferStatus>,
    /// Whether the transfer is a donation
    pub donation: bool,
    /// The confirmations the indexer reported for the transaction (0: in its mempool)
    pub confirmations: u64,
}

/// ERA fork (L4b): what [`Wallet::forget_dropped_payment`](crate::wallet::Wallet::forget_dropped_payment)
/// did with a payment of envelopes the host proved will never confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DroppedPayment {
    /// BDK never saw the TX: nothing of it to forget
    NotSeen,
    /// BDK holds the TX as mined — an anchor, a reorged block's too — or keeps it canonical
    /// whatever its eviction says: it was not dropped; nothing changed
    Confirmed,
    /// A TX BDK holds as canonical spends an output of it: forgetting the payment would take that
    /// TX out of BDK's view as well; nothing changed
    Spent,
    /// A transfer of this wallet still in play names the TX, the wallet recorded a spend of one of
    /// its colored outputs, or one of them holds something ([`holds_nothing`]); nothing changed
    Held,
    /// Setting it aside, with the transactions competing for its coins, would let another
    /// transaction BDK holds as not canonical come back; nothing changed
    Conflicted,
    /// BDK holds the TX — and every TX competing for its coins — as not canonical until one of
    /// them is mined, and `envelopes` of its colored outputs, which existed, are marked as not
    /// existing
    Forgotten {
        /// The rows marked
        envelopes: u32,
    },
}

/// How the indexer answers for a transaction: `Some(confirmations)` when it knows it (0 in its
/// mempool), `None` when it does not.
pub(crate) trait TxStatusLookup {
    fn confirmations(&self, txid: &str) -> Result<Option<u64>, Error>;
}

impl TxStatusLookup for Indexer {
    fn confirmations(&self, txid: &str) -> Result<Option<u64>, Error> {
        self.get_tx_confirmations(txid)
    }
}

/// The colored coins the database holds as unspent (`exists && !spent`) that BDK does not list
/// as unspent: the set upstream's singlesig check refuses on.
pub(crate) fn colored_divergence(
    bdk_wallet: &PersistedWallet<Store<ChangeSet>>,
    txn: &DbTxn,
) -> Result<BTreeSet<BdkOutPoint>, Error> {
    let bdk_unspent: HashSet<BdkOutPoint> = bdk_wallet.list_unspent().map(|u| u.outpoint).collect();
    Ok(txn
        .iter_txos()?
        .into_iter()
        .filter(|t| t.exists && !t.spent)
        .map(BdkOutPoint::from)
        .filter(|o| !bdk_unspent.contains(o))
        .collect())
}

/// What BDK's canonical view says about the divergent coins.
#[derive(Clone, Debug)]
pub(crate) struct SpendView {
    /// The canonical transaction spending each divergent coin that has one (P1: the canonical set
    /// is conflict-free, so there is at most one)
    pub(crate) spenders: BTreeMap<BdkOutPoint, String>,
    /// Those transactions, by ID
    pub(crate) txs: BTreeMap<String, BdkTransaction>,
    /// The wallet's colored outputs of each of them, and whether BDK sees each spent
    pub(crate) colored_outputs: BTreeMap<String, Vec<(BdkOutPoint, bool)>>,
    /// ERA fork (CC-115): of the transactions that created divergent coins, those BDK has seen
    /// (its graph holds them) and no longer holds as canonical: in conflict with a transaction it
    /// learnt, or descending from one (BDK records no eviction for this wallet: its requests carry
    /// no expected TXIDs). One BDK never saw is not among them.
    pub(crate) non_canonical_creators: BTreeSet<String>,
}

impl SpendView {
    pub(crate) fn from_bdk(
        bdk_wallet: &PersistedWallet<Store<ChangeSet>>,
        divergence: &BTreeSet<BdkOutPoint>,
    ) -> Self {
        let mut spenders = BTreeMap::new();
        let mut txs = BTreeMap::new();
        let mut canonical = HashSet::new();
        for wallet_tx in bdk_wallet.transactions() {
            canonical.insert(wallet_tx.tx_node.txid);
            let tx = wallet_tx.tx_node.tx.as_ref();
            for input in &tx.input {
                if divergence.contains(&input.previous_output) {
                    let txid = tx.compute_txid().to_string();
                    spenders.insert(input.previous_output, txid.clone());
                    txs.entry(txid).or_insert_with(|| tx.clone());
                }
            }
        }
        let mut colored_outputs: BTreeMap<String, Vec<(BdkOutPoint, bool)>> =
            txs.keys().map(|t| (t.clone(), vec![])).collect();
        for output in bdk_wallet
            .list_output()
            .filter(|o| o.keychain == KeychainKind::External)
        {
            if let Some(outputs) = colored_outputs.get_mut(&output.outpoint.txid.to_string()) {
                outputs.push((output.outpoint, output.is_spent));
            }
        }
        let non_canonical_creators = divergence
            .iter()
            .map(|o| o.txid)
            .filter(|txid| {
                !canonical.contains(txid) && bdk_wallet.tx_graph().get_tx(*txid).is_some()
            })
            .map(|txid| txid.to_string())
            .collect();
        Self {
            spenders,
            txs,
            colored_outputs,
            non_canonical_creators,
        }
    }
}

/// The record of this wallet that names a spend, as the completion will write it.
#[derive(Debug)]
pub(crate) enum PlannedRecord {
    Send {
        batch_transfer: DbBatchTransfer,
        donation: bool,
        /// The transfer's fascia, with the witness the stash gets
        fascia: Box<Fascia>,
    },
    Drain {
        reservations: Vec<DbReservedTxo>,
    },
}

#[derive(Debug)]
pub(crate) struct PlannedSpend {
    pub(crate) txid: String,
    pub(crate) tx: BdkTransaction,
    pub(crate) record: PlannedRecord,
    pub(crate) confirmations: u64,
}

/// The completions, in TXID order.
#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) spends: Vec<PlannedSpend>,
    /// ERA fork (CC-115): divergent coins that are envelopes of a payment BDK no longer holds,
    /// to be marked as not existing
    pub(crate) gone_envelopes: Vec<DbTxo>,
}

fn refusal(txid: &str, reason: UnrecordedSpendReason, batch_transfer_idx: Option<i32>) -> Error {
    Error::UnrecordedSpend {
        txid: txid.to_string(),
        reason: reason.code().to_string(),
        batch_transfer_idx,
    }
}

struct Records {
    batch_transfers: Vec<DbBatchTransfer>,
    wallet_transaction: Option<(DbWalletTransaction, Vec<DbReservedTxo>)>,
}

impl Records {
    fn count(&self) -> usize {
        self.batch_transfers.len() + usize::from(self.wallet_transaction.is_some())
    }

    // the batch transfer to name in an error, when exactly one record is a batch transfer
    fn batch_transfer_idx(&self) -> Option<i32> {
        match (self.batch_transfers.as_slice(), &self.wallet_transaction) {
            ([batch_transfer], None) => Some(batch_transfer.idx),
            _ => None,
        }
    }
}

/// A file of the transfer that is missing or does not parse is `transfer-data-missing`; any other
/// I/O error is reported as such, since it may not be the wallet's state that is at fault.
fn read_transfer_file(
    path: &Path,
    txid: &str,
    batch_transfer_idx: Option<i32>,
) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => refusal(
            txid,
            UnrecordedSpendReason::TransferDataMissing,
            batch_transfer_idx,
        ),
        _ => Error::IO {
            details: e.to_string(),
        },
    })
}

/// CC-115: the divergent coins without a spender, if every one is an envelope of a payment BDK
/// no longer holds: its creating TX is one of `view.non_canonical_creators`, no batch transfer of
/// this wallet but a `Failed` one names that TX (a payment from outside never is; a send's change
/// is, and holds nothing when the send moved a whole allocation, while its transfer waits on the
/// TX), and the wallet's records leave the coin empty ([`holds_nothing`]). `None` when any one is
/// not: it is refused as before.
fn gone_envelopes(
    view: &SpendView,
    coins: &[BdkOutPoint],
    txn: &DbTxn,
) -> Result<Option<Vec<DbTxo>>, Error> {
    if coins.is_empty() {
        return Ok(Some(vec![]));
    }
    if coins
        .iter()
        .any(|c| !view.non_canonical_creators.contains(&c.txid.to_string()))
    {
        return Ok(None);
    }
    let creators: BTreeSet<String> = coins.iter().map(|c| c.txid.to_string()).collect();
    for txid in &creators {
        if txn
            .get_batch_transfers_by_txid(txid)?
            .iter()
            .any(|b| !b.status.failed())
        {
            return Ok(None);
        }
    }
    let mut txos = vec![];
    for coin in coins {
        txos.push(
            txn.get_txo(&Outpoint::from(*coin))?
                .expect("a divergent coin is a row of the database"),
        );
    }
    let reserved: HashSet<BdkOutPoint> = txn
        .iter_reserved_txos()?
        .into_iter()
        .map(BdkOutPoint::from)
        .collect();
    let unspents = txn.get_rgb_allocations(txos, None, None, None, None)?;
    if !unspents.iter().all(|u| holds_nothing(u, &reserved)) {
        return Ok(None);
    }
    Ok(Some(unspents.into_iter().map(|u| u.utxo).collect()))
}

/// ERA fork (L4b): the colored outputs of the payment `txid` to mark as not existing once the host
/// proved it dropped, or `None` when anything of the wallet stands on them — CC-115's (c) and (a):
/// a batch transfer still in play names the TX, the wallet recorded a spend of one of them, or one
/// holds something ([`holds_nothing`]). Outputs already not existing are left as they are.
pub(crate) fn dropped_payment_envelopes(
    txn: &DbTxn,
    txid: &str,
) -> Result<Option<Vec<DbTxo>>, Error> {
    if txn
        .get_batch_transfers_by_txid(txid)?
        .iter()
        .any(|b| !b.status.failed())
    {
        return Ok(None);
    }
    let rows: Vec<DbTxo> = txn
        .iter_txos()?
        .into_iter()
        .filter(|t| t.txid == txid)
        .collect();
    if rows.iter().any(|t| t.spent) {
        return Ok(None);
    }
    let existing: Vec<DbTxo> = rows.into_iter().filter(|t| t.exists).collect();
    if existing.is_empty() {
        return Ok(Some(vec![]));
    }
    let reserved: HashSet<BdkOutPoint> = txn
        .iter_reserved_txos()?
        .into_iter()
        .map(BdkOutPoint::from)
        .collect();
    let unspents = txn.get_rgb_allocations(existing, None, None, None, None)?;
    if !unspents.iter().all(|u| holds_nothing(u, &reserved)) {
        return Ok(None);
    }
    Ok(Some(unspents.into_iter().map(|u| u.utxo).collect()))
}

/// CC-115: whether the wallet's records leave a coin empty, by its own accounting of a UTXO's
/// slots (`get_available_allocations`): no allocation but those of failed transfers (so no
/// issuance, no receive, no input or change of a transfer in flight or done), no blind receive
/// waiting on it (`pending_blinded`), no witness receive landing on it, and no reservation (a
/// drain or another vanilla TX begun on it).
fn holds_nothing(unspent: &LocalUnspent, reserved: &HashSet<BdkOutPoint>) -> bool {
    unspent.rgb_allocations.iter().all(|a| a.status.failed())
        && unspent.pending_blinded == 0
        && !unspent.utxo.pending_witness
        && !reserved.contains(&BdkOutPoint::from(unspent.utxo.clone()))
}

/// Decide what to complete, or why not. Nothing is written.
///
/// The verdicts, in this order of precedence (the first that applies wins):
/// 1. a divergent coin that no own spend explains: [`Error::Inconsistency`] with a reason
///    (P1, P2, P4), save an envelope of a payment BDK no longer holds (CC-115), which the plan
///    marks as not existing;
/// 2. the indexer's lookup failing: its error (P3);
/// 3. the indexer not knowing an own spend: [`Error::UnrecordedSpendUnseen`] (P3);
/// 4. an own spend whose record or files do not allow completing it: [`Error::UnrecordedSpend`]
///    (P5);
/// 5. otherwise the plan, which covers every divergent coin (P6).
pub(crate) fn plan(
    view: &SpendView,
    divergence: &BTreeSet<BdkOutPoint>,
    lookup: &dyn TxStatusLookup,
    txn: &DbTxn,
    transfers_dir: &Path,
    details: String,
) -> Result<Plan, Error> {
    // phase 1, local: every divergent coin is spent by a canonical TX (P1) that one of this
    // wallet's records names (P2), and nothing that TX created was spent by a TX the wallet has
    // no record of (P4); or it is not spent at all, but an envelope of a payment BDK no longer
    // holds (CC-115)
    let mut own_spends = BTreeSet::new();
    let mut without_spender = vec![];
    for outpoint in divergence {
        match view.spenders.get(outpoint) {
            Some(txid) => {
                own_spends.insert(txid.clone());
            }
            None => without_spender.push(*outpoint),
        }
    }
    let spenders: Vec<String> = own_spends.iter().cloned().collect();
    let Some(gone_envelopes) = gone_envelopes(view, &without_spender, txn)? else {
        return Err(Error::unrecorded_inconsistency(
            details,
            &spenders,
            InconsistencyReason::NoCanonicalSpender,
        ));
    };
    let mut records = BTreeMap::new();
    for txid in &own_spends {
        let found = Records {
            batch_transfers: txn
                .get_batch_transfers_by_txid(txid)?
                .into_iter()
                .filter(|b| !b.incoming)
                .collect(),
            wallet_transaction: txn.get_wallet_transaction_with_reserved_txos_by_txid(txid)?,
        };
        if found.count() == 0 {
            return Err(Error::unrecorded_inconsistency(
                details,
                &spenders,
                InconsistencyReason::SpenderNotRecorded,
            ));
        }
        records.insert(txid.clone(), found);
    }
    for txid in &own_spends {
        for (outpoint, spent) in view.colored_outputs.get(txid).into_iter().flatten() {
            if *spent
                && !txn
                    .get_txo(&Outpoint::from(*outpoint))?
                    .is_some_and(|txo| txo.spent)
            {
                return Err(Error::unrecorded_inconsistency(
                    details,
                    &spenders,
                    InconsistencyReason::LaterSpendUnrecorded,
                ));
            }
        }
    }

    // phase 2, network: the indexer knows every own spend (P3), the bar broadcast_tx uses to count
    // a broadcast. A failed lookup is reported before any unknown TX.
    let mut confirmations = BTreeMap::new();
    let mut unseen = None;
    for txid in &own_spends {
        match lookup.confirmations(txid)? {
            Some(n) => {
                confirmations.insert(txid.clone(), n);
            }
            None => {
                unseen.get_or_insert(txid.clone());
            }
        }
    }
    if let Some(txid) = unseen {
        let batch_transfer_idx = records[&txid].batch_transfer_idx();
        return Err(Error::UnrecordedSpendUnseen {
            txid,
            batch_transfer_idx,
        });
    }

    // phase 3, local: each record and its files describe exactly that TX (P5)
    let mut spends = vec![];
    for (txid, found) in records {
        let tx = view.txs[&txid].clone();
        let idx = found.batch_transfer_idx();
        if found.count() > 1 {
            return Err(refusal(&txid, UnrecordedSpendReason::RecordAmbiguous, None));
        }
        let record = match (
            found.batch_transfers.into_iter().next(),
            found.wallet_transaction,
        ) {
            (Some(batch_transfer), _) => plan_send(txn, transfers_dir, &tx, batch_transfer)?,
            (None, Some((wallet_transaction, reservations))) => {
                if wallet_transaction.r#type != WalletTransactionType::Drain {
                    return Err(refusal(
                        &txid,
                        UnrecordedSpendReason::RecordKindUnsupported,
                        idx,
                    ));
                }
                if reservations.is_empty() {
                    return Err(refusal(&txid, UnrecordedSpendReason::UnexpectedStatus, idx));
                }
                let reserved: BTreeSet<BdkOutPoint> = reservations
                    .iter()
                    .cloned()
                    .map(BdkOutPoint::from)
                    .collect();
                if reserved != inputs(&tx) {
                    return Err(refusal(&txid, UnrecordedSpendReason::RecordMismatch, idx));
                }
                PlannedRecord::Drain { reservations }
            }
            (None, None) => unreachable!("every spender has a record (phase 1)"),
        };
        spends.push(PlannedSpend {
            confirmations: confirmations[&txid],
            txid,
            tx,
            record,
        });
    }
    Ok(Plan {
        spends,
        gone_envelopes,
    })
}

fn inputs(tx: &BdkTransaction) -> BTreeSet<BdkOutPoint> {
    tx.input.iter().map(|i| i.previous_output).collect()
}

/// P5 for a send: `tx` is the canonical spender that `batch_transfer` names.
fn plan_send(
    txn: &DbTxn,
    transfers_dir: &Path,
    tx: &BdkTransaction,
    batch_transfer: DbBatchTransfer,
) -> Result<PlannedRecord, Error> {
    let txid = &tx.compute_txid().to_string();
    let inputs = inputs(tx);
    let idx = Some(batch_transfer.idx);
    let mismatch = || refusal(txid, UnrecordedSpendReason::RecordMismatch, idx);
    let missing = || refusal(txid, UnrecordedSpendReason::TransferDataMissing, idx);
    match batch_transfer.status {
        TransferStatus::Initiated | TransferStatus::WaitingCounterparty => {}
        TransferStatus::Failed => {
            return Err(refusal(txid, UnrecordedSpendReason::TransferFailed, idx));
        }
        _ => {
            return Err(refusal(txid, UnrecordedSpendReason::UnexpectedStatus, idx));
        }
    }

    // the files get_transfer_end_data reads
    let transfer_dir = transfers_dir.join(txid);
    let info: InfoBatchTransfer = serde_json::from_str(&read_transfer_file(
        &transfer_dir.join(TRANSFER_DATA_FILE),
        txid,
        idx,
    )?)
    .map_err(|_| missing())?;
    let mut fascia: Fascia = serde_json::from_str(&read_transfer_file(
        &transfer_dir.join(FASCIA_FILE),
        txid,
        idx,
    )?)
    .map_err(|_| missing())?;
    if info
        .transfers
        .values()
        .any(|t| t.main_transition != TypeOfTransition::Transfer)
    {
        return Err(refusal(
            txid,
            UnrecordedSpendReason::RecordKindUnsupported,
            idx,
        ));
    }

    // the fascia commits to this TX, and moves exactly the assets of this batch transfer (the
    // user-driven ones and the ones moved along, save_transfers)
    if fascia.witness_id().to_string() != *txid {
        return Err(mismatch());
    }
    let asset_transfers: Vec<DbAssetTransfer> = txn
        .iter_asset_transfers()?
        .into_iter()
        .filter(|a| a.batch_transfer_idx == batch_transfer.idx)
        .collect();
    let batch_assets: BTreeSet<String> = asset_transfers
        .iter()
        .filter_map(|a| a.asset_id.clone())
        .collect();
    let fascia_assets: BTreeSet<String> = fascia.bundles().keys().map(|c| c.to_string()).collect();
    if batch_assets != fascia_assets {
        return Err(mismatch());
    }

    // the coins this batch transfer spends are inputs of this TX
    let asset_transfer_idxs: HashSet<i32> = asset_transfers.iter().map(|a| a.idx).collect();
    let input_txo_idxs: HashSet<i32> = txn
        .iter_colorings()?
        .into_iter()
        .filter(|c| {
            c.r#type == ColoringType::Input && asset_transfer_idxs.contains(&c.asset_transfer_idx)
        })
        .map(|c| c.txo_idx)
        .collect();
    if txn
        .iter_txos()?
        .into_iter()
        .filter(|t| input_txo_idxs.contains(&t.idx))
        .any(|t| !inputs.contains(&BdkOutPoint::from(t)))
    {
        return Err(mismatch());
    }

    // the signed PSBT send_end saved, if it got that far, is of this TX; a donation's stash gets
    // the TX it signed, as send_end gives it. Without that file (S3) the stash gets the fascia as
    // stored, with the TX unsigned, as every non-donation send consumes it. The copy of the TX
    // that BDK got from the indexer is never used as a witness: the TXID does not commit to the
    // witnesses in it.
    let signed_psbt_path = transfer_dir.join(SIGNED_PSBT_FILE);
    if signed_psbt_path.exists() {
        let signed_psbt = Psbt::from_str(&read_transfer_file(&signed_psbt_path, txid, idx)?)
            .map_err(|_| missing())?;
        if signed_psbt.unsigned_tx.compute_txid().to_string() != *txid {
            return Err(mismatch());
        }
        if info.donation {
            // the TXID of the extracted TX is the unsigned TX's, checked above
            let signed_tx = signed_psbt.extract_tx().map_err(|_| mismatch())?;
            fascia.update_pub_witness(PubWitness::with(signed_tx));
        }
    }

    Ok(PlannedRecord::Send {
        batch_transfer,
        donation: info.donation,
        fascia: Box::new(fascia),
    })
}

/// Write a plan: the stash first, durable before anything else, then the database in the
/// check's transaction (committed by `go_online`), the envelopes gone (CC-115) last, then the
/// check that no divergence is left.
pub(crate) fn apply<W: WalletOnline + ?Sized>(
    wallet: &mut W,
    txn: &DbTxn,
    runtime: &mut RgbRuntime,
    plan: Plan,
) -> Result<Vec<CompletedSpend>, Error> {
    // phase 4, the stash. Each consume_fascia stores what it consumed when it commits (rgb-ops
    // commits index, state and stash together, and the stock autosaves): that is where the stash
    // becomes durable, fascia by fascia. A store the disk refuses there comes back as
    // StockNotStored (CC-101, stock_store), which is the retryable Error::IO. A consume that fails
    // half way stores nothing: the runtime's drop, which would store it, is turned off. A bundle
    // the stash already holds (S2, or a completion that did not commit) is not consumed again.
    // With no spend to complete (only envelopes gone, CC-115) the runtime is left as upstream's
    // check leaves it.
    let completing = !plan.spends.is_empty();
    if completing {
        runtime.require_explicit_persistence();
    }
    for spend in &plan.spends {
        let PlannedRecord::Send {
            batch_transfer,
            fascia,
            ..
        } = &spend.record
        else {
            continue;
        };
        let Some(fascia) = runtime.fascia_unknown_part(*fascia.clone()) else {
            debug!(
                wallet.logger(),
                "CC-99: the stash already holds the transitions of TX {}", spend.txid
            );
            continue;
        };
        runtime.consume_fascia(fascia, None).map_err(|e| match e {
            // the disk refused the stash (CC-101): an I/O error, not the stash's verdict
            InternalError::StockNotStored(details) => Error::IO { details },
            e => {
                warn!(
                    wallet.logger(),
                    "CC-99: the stash refused the transitions of TX {}: {e}", spend.txid
                );
                refusal(
                    &spend.txid,
                    UnrecordedSpendReason::StashRefused,
                    Some(batch_transfer.idx),
                )
            }
        })?;
    }
    // with everything stored by the commits above, this finds nothing to write on this base; it
    // stays as the durability point of a base whose stock does not store at each commit
    if completing {
        runtime.persist()?;
    }

    // phase 5, the database: what the operation's commit would have written
    let mut completed = vec![];
    for spend in plan.spends {
        wallet.record_broadcast(txn, &spend.tx)?;
        match spend.record {
            PlannedRecord::Send {
                batch_transfer,
                donation,
                ..
            } => {
                let previous_status = batch_transfer.status;
                let idx = batch_transfer.idx;
                let mut updated: DbBatchTransferActMod = batch_transfer.into();
                updated.status = ActiveValue::Set(TransferStatus::WaitingConfirmations);
                txn.update_batch_transfer(&mut updated)?;
                warn!(
                    wallet.logger(),
                    "completed batch transfer {idx}: its TX {} is known to the indexer but was \
                     not recorded (CC-99)",
                    spend.txid
                );
                completed.push(CompletedSpend {
                    txid: spend.txid,
                    batch_transfer_idx: Some(idx),
                    previous_status: Some(previous_status),
                    donation,
                    confirmations: spend.confirmations,
                });
            }
            PlannedRecord::Drain { reservations } => {
                txn.del_reserved_txos(&reservations)?;
                warn!(
                    wallet.logger(),
                    "completed a drain: its TX {} is known to the indexer but was not recorded \
                     (CC-99)",
                    spend.txid
                );
                completed.push(CompletedSpend {
                    txid: spend.txid,
                    batch_transfer_idx: None,
                    previous_status: None,
                    donation: false,
                    confirmations: spend.confirmations,
                });
            }
        }
    }
    // CC-115: an envelope of a payment BDK no longer holds is marked as a TXO of a TX that has not
    // reached the network is (`exists` false): out of every selection and of the check;
    // `list_unspents` still lists it, flagged, for the host to filter. `spent` stays false, so the
    // sync that sees the payment canonical again sets `exists` back (`set_txo` raises it and never
    // touches `spent`): the same row, as it was.
    // Not reported as a completion: it is derived from the chain, and a copy without it does the
    // same at its next go_online.
    for txo in plan.gone_envelopes {
        let outpoint = txo.outpoint();
        let mut gone: DbTxoActMod = txo.into();
        gone.exists = ActiveValue::Set(false);
        txn.update_txo(gone)?;
        warn!(
            wallet.logger(),
            "CC-115: TXO {outpoint} holds nothing and BDK no longer holds its TX as canonical \
             (replaced): marked as not existing until the TX is back"
        );
    }

    // phase 6 (P7): the writes left no divergence
    if !colored_divergence(wallet.bdk_wallet(), txn)?.is_empty() {
        return Err(Error::Internal {
            details: s!("CC-99 completion left a divergence"),
        });
    }
    Ok(completed)
}

/// The singlesig consistency check found `details`: complete what can be completed, or refuse.
pub(crate) fn complete<W: WalletOnline + ?Sized>(
    wallet: &mut W,
    txn: &DbTxn,
    runtime: &mut RgbRuntime,
    details: String,
) -> Result<Vec<CompletedSpend>, Error> {
    let divergence = colored_divergence(wallet.bdk_wallet(), txn)?;
    if divergence.is_empty() {
        // not a divergence of spent coins: the upstream verdict
        return Err(Error::Inconsistency { details });
    }
    let view = SpendView::from_bdk(wallet.bdk_wallet(), &divergence);
    let plan = plan(
        &view,
        &divergence,
        wallet.indexer(),
        txn,
        &wallet.get_transfers_dir(),
        details,
    )
    .inspect_err(|e| match e {
        // the error unsaid: an indexer's answer can name the URL it was asked, which carries the
        // forwarder's session secret in the app (as in fail_transfers_impl)
        Error::Indexer { .. } | Error::Network { .. } => warn!(
            wallet.logger(),
            "CC-99: not completing the spends of divergent coins {divergence:?}: the indexer's \
             lookup failed"
        ),
        e => warn!(
            wallet.logger(),
            "CC-99: not completing the spends of divergent coins {divergence:?}: {e}"
        ),
    })?;
    apply(wallet, txn, runtime, plan)
}
