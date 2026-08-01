//! Bitcoin Core JSON-RPC block and mempool source with independent witness verification.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, ensure};
use async_trait::async_trait;
use bitcoin::consensus::{deserialize, serialize};
use bitcoin::ecdsa;
use bitcoin::hashes::Hash as _;
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::{Amount, Block, ScriptBuf, Transaction, Txid};
use reqwest::Client;
use secp256k1::{Message, PublicKey, Secp256k1};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tandem_core::{BlockView, Hash32, InputView, Network, OutPointRef, OutputView, TxView};

use crate::config::Config;

/// Bitcoin Core readiness facts used by `/readyz`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CoreStatus {
    /// Core chain label.
    pub chain: String,
    /// Best validated height.
    pub blocks: u64,
    /// Best header height.
    pub headers: u64,
    /// Initial block download flag.
    pub initial_block_download: bool,
    /// Transaction index is present and synced.
    pub txindex_synced: bool,
}

impl CoreStatus {
    /// Return true when Core is suitable for canonical ingestion.
    pub fn ready_for(&self, network: Network) -> bool {
        let expected = match network {
            Network::Mainnet => "main",
            Network::Signet => "signet",
            Network::Testnet4 => "testnet4",
            Network::Regtest => "regtest",
        };
        self.chain == expected
            && self.blocks == self.headers
            && !self.initial_block_download
            && self.txindex_synced
    }
}

/// Source boundary consumed by the ingestion worker.
#[async_trait]
pub trait BlockSource: Send + Sync {
    /// Return Core readiness facts.
    async fn status(&self) -> Result<CoreStatus>;
    /// Return the canonical best height.
    async fn best_height(&self) -> Result<u64>;
    /// Return canonical block hash at a height, in wire order.
    async fn block_hash(&self, height: u64) -> Result<Hash32>;
    /// Resolve the canonical block and every input prevout.
    async fn block(&self, height: u64) -> Result<BlockView>;
    /// Locate the configured INIT confirmation height.
    async fn transaction_height(&self, txid_wire: Hash32) -> Result<Option<u64>>;
    /// Resolve current mempool transactions without mutating canonical state.
    async fn mempool(&self) -> Result<Vec<TxView>>;
}

/// Minimal authenticated Bitcoin Core JSON-RPC client.
pub struct BitcoinCoreRpc {
    client: Client,
    url: url::Url,
    user: String,
    password: String,
    request_id: AtomicU64,
}

impl BitcoinCoreRpc {
    /// Build from validated process configuration.
    pub fn new(config: &Config) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("cannot build Bitcoin Core HTTP client")?;
        Ok(Self {
            client,
            url: config.bitcoin_rpc_url.clone(),
            user: config.bitcoin_rpc_user.clone(),
            password: config.bitcoin_rpc_password.clone(),
            request_id: AtomicU64::new(1),
        })
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        let response = self
            .client
            .post(self.url.clone())
            .basic_auth(&self.user, Some(&self.password))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }))
            .send()
            .await
            .with_context(|| format!("Bitcoin Core RPC {method} request failed"))?;
        let status = response.status();
        let envelope: RpcEnvelope<T> = response
            .json()
            .await
            .with_context(|| format!("Bitcoin Core RPC {method} returned invalid JSON"))?;
        if let Some(error) = envelope.error {
            return Err(CoreRpcError {
                method: method.to_owned(),
                code: error.code,
                message: error.message,
            }
            .into());
        }
        ensure!(
            status.is_success(),
            "Bitcoin Core RPC {method} returned HTTP {status}"
        );
        envelope
            .result
            .ok_or_else(|| anyhow!("Bitcoin Core RPC {method} omitted result"))
    }

    async fn raw_transaction(&self, txid: Txid) -> Result<ResolvedTransaction> {
        let verbose: RawTransactionVerbose = self
            .call("getrawtransaction", json!([txid.to_string(), true]))
            .await?;
        let raw = hex::decode(&verbose.hex).context("Core returned non-hex raw transaction")?;
        let transaction: Transaction =
            deserialize(&raw).context("Core returned invalid raw transaction")?;
        ensure!(
            transaction.compute_txid() == txid,
            "Core returned a different transaction than requested"
        );
        let height = if verbose
            .confirmations
            .is_some_and(|confirmations| confirmations > 0)
            && let Some(block_hash) = verbose.blockhash
        {
            let header: HeaderVerbose = self
                .call("getblockheader", json!([block_hash, true]))
                .await?;
            Some(header.height)
        } else {
            None
        };
        Ok(ResolvedTransaction {
            transaction,
            height,
        })
    }

    async fn resolve_transaction(
        &self,
        transaction: &Transaction,
        local: &HashMap<Txid, ResolvedTransaction>,
        cache: &mut HashMap<Txid, ResolvedTransaction>,
    ) -> Result<TxView> {
        let mut inputs = Vec::with_capacity(transaction.input.len());
        for (index, input) in transaction.input.iter().enumerate() {
            if input.previous_output.is_null() {
                inputs.push(InputView {
                    prevout: OutPointRef::ZERO,
                    sequence: input.sequence.to_consensus_u32(),
                    script_sig: input.script_sig.as_bytes().to_vec(),
                    witness: input.witness.iter().map(<[u8]>::to_vec).collect(),
                    prevout_value: 0,
                    prevout_script: Vec::new(),
                    prevout_height: None,
                    signatures_valid: false,
                });
                continue;
            }
            let parent = if let Some(parent) = local.get(&input.previous_output.txid) {
                parent.clone()
            } else if let Some(parent) = cache.get(&input.previous_output.txid) {
                parent.clone()
            } else {
                let parent = self.raw_transaction(input.previous_output.txid).await?;
                cache.insert(input.previous_output.txid, parent.clone());
                parent
            };
            let prevout = parent
                .transaction
                .output
                .get(input.previous_output.vout as usize)
                .with_context(|| format!("prevout {} is out of range", input.previous_output))?;
            let signatures_valid = verify_input_signature(transaction, index, prevout);
            inputs.push(InputView {
                prevout: OutPointRef {
                    txid: txid_wire(input.previous_output.txid),
                    vout: input.previous_output.vout,
                },
                sequence: input.sequence.to_consensus_u32(),
                script_sig: input.script_sig.as_bytes().to_vec(),
                witness: input.witness.iter().map(<[u8]>::to_vec).collect(),
                prevout_value: prevout.value.to_sat(),
                prevout_script: prevout.script_pubkey.as_bytes().to_vec(),
                prevout_height: parent.height,
                signatures_valid,
            });
        }
        Ok(TxView {
            txid: txid_wire(transaction.compute_txid()),
            wtxid: wtxid_wire(transaction.compute_wtxid()),
            version: transaction.version.0,
            lock_time: transaction.lock_time.to_consensus_u32(),
            inputs,
            outputs: transaction
                .output
                .iter()
                .map(|output| OutputView {
                    value: output.value.to_sat(),
                    script_pubkey: output.script_pubkey.as_bytes().to_vec(),
                })
                .collect(),
        })
    }
}

#[async_trait]
impl BlockSource for BitcoinCoreRpc {
    async fn status(&self) -> Result<CoreStatus> {
        let chain: BlockchainInfo = self.call("getblockchaininfo", json!([])).await?;
        let indexes: HashMap<String, IndexInfo> =
            self.call("getindexinfo", json!(["txindex"])).await?;
        let txindex_synced = indexes.get("txindex").is_some_and(|index| index.synced);
        Ok(CoreStatus {
            chain: chain.chain,
            blocks: chain.blocks,
            headers: chain.headers,
            initial_block_download: chain.initialblockdownload,
            txindex_synced,
        })
    }

    async fn best_height(&self) -> Result<u64> {
        self.call::<u64>("getblockcount", json!([])).await
    }

    async fn block_hash(&self, height: u64) -> Result<Hash32> {
        let display: String = self.call("getblockhash", json!([height])).await?;
        tandem_core::wire_hash(&display).context("Core returned invalid block hash")
    }

    async fn block(&self, height: u64) -> Result<BlockView> {
        let hash_display: String = self.call("getblockhash", json!([height])).await?;
        let block_hex: String = self
            .call("getblock", json!([hash_display.clone(), 0]))
            .await?;
        let bytes = hex::decode(block_hex).context("Core returned non-hex block")?;
        let block: Block = deserialize(&bytes).context("Core returned invalid block bytes")?;
        ensure!(
            block.block_hash().to_string() == hash_display,
            "Core returned a different block than requested"
        );
        let mut local = HashMap::new();
        for transaction in &block.txdata {
            local.insert(
                transaction.compute_txid(),
                ResolvedTransaction {
                    transaction: transaction.clone(),
                    height: Some(height),
                },
            );
        }
        let mut cache = HashMap::new();
        let mut transactions = Vec::with_capacity(block.txdata.len());
        for transaction in &block.txdata {
            transactions.push(
                self.resolve_transaction(transaction, &local, &mut cache)
                    .await?,
            );
        }
        Ok(BlockView {
            hash: block_hash_wire(block.block_hash()),
            previous_hash: block_hash_wire(block.header.prev_blockhash),
            height,
            transactions,
        })
    }

    async fn transaction_height(&self, txid_wire_hash: Hash32) -> Result<Option<u64>> {
        let txid = txid_from_wire(txid_wire_hash)?;
        match self.raw_transaction(txid).await {
            Ok(transaction) => Ok(transaction.height),
            Err(error)
                if error
                    .downcast_ref::<CoreRpcError>()
                    .is_some_and(|rpc_error| rpc_error.code == -5) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    async fn mempool(&self) -> Result<Vec<TxView>> {
        let txids: Vec<String> = self.call("getrawmempool", json!([false])).await?;
        let mut cache = HashMap::new();
        let local = HashMap::new();
        let mut views = Vec::with_capacity(txids.len());
        for txid in txids {
            let txid = txid
                .parse::<Txid>()
                .context("Core returned invalid mempool txid")?;
            let resolved = self.raw_transaction(txid).await?;
            views.push(
                self.resolve_transaction(&resolved.transaction, &local, &mut cache)
                    .await?,
            );
        }
        Ok(views)
    }
}

#[derive(Clone)]
struct ResolvedTransaction {
    transaction: Transaction,
    height: Option<u64>,
}

#[derive(Deserialize)]
struct RpcEnvelope<T> {
    result: Option<T>,
    error: Option<RpcFailure>,
}

#[derive(Deserialize)]
struct RpcFailure {
    code: i64,
    message: String,
}

#[derive(Debug)]
struct CoreRpcError {
    method: String,
    code: i64,
    message: String,
}

impl std::fmt::Display for CoreRpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Bitcoin Core RPC {} error {}: {}",
            self.method, self.code, self.message
        )
    }
}

impl std::error::Error for CoreRpcError {}

#[derive(Deserialize)]
struct BlockchainInfo {
    chain: String,
    blocks: u64,
    headers: u64,
    initialblockdownload: bool,
}

#[derive(Deserialize)]
struct IndexInfo {
    synced: bool,
}

#[derive(Deserialize)]
struct RawTransactionVerbose {
    hex: String,
    blockhash: Option<String>,
    confirmations: Option<i64>,
}

#[derive(Deserialize)]
struct HeaderVerbose {
    height: u64,
}

fn verify_input_signature(
    transaction: &Transaction,
    input_index: usize,
    prevout: &bitcoin::TxOut,
) -> bool {
    let Some(input) = transaction.input.get(input_index) else {
        return false;
    };
    if !input.script_sig.is_empty() {
        return false;
    }
    let witness = input.witness.iter().collect::<Vec<_>>();
    if prevout.script_pubkey.is_p2wpkh() && witness.len() == 2 {
        return verify_p2wpkh(transaction, input_index, prevout, witness[0], witness[1]);
    }
    if prevout.script_pubkey.is_p2wsh() && witness.len() == 4 && witness[0].is_empty() {
        return verify_p2wsh_2of2(
            transaction,
            input_index,
            prevout,
            witness[1],
            witness[2],
            witness[3],
        );
    }
    false
}

fn verify_p2wpkh(
    transaction: &Transaction,
    input_index: usize,
    prevout: &bitcoin::TxOut,
    signature_bytes: &[u8],
    public_key_bytes: &[u8],
) -> bool {
    let Ok(signature) = strict_signature(signature_bytes) else {
        return false;
    };
    let Ok(public_key) = PublicKey::from_slice(public_key_bytes) else {
        return false;
    };
    let Ok(sighash) = SighashCache::new(transaction).p2wpkh_signature_hash(
        input_index,
        &prevout.script_pubkey,
        prevout.value,
        EcdsaSighashType::All,
    ) else {
        return false;
    };
    let message = Message::from_digest(sighash.to_byte_array());
    Secp256k1::verification_only()
        .verify_ecdsa(&message, &signature.signature, &public_key)
        .is_ok()
}

fn verify_p2wsh_2of2(
    transaction: &Transaction,
    input_index: usize,
    prevout: &bitcoin::TxOut,
    signature0_bytes: &[u8],
    signature1_bytes: &[u8],
    witness_script_bytes: &[u8],
) -> bool {
    if witness_script_bytes.len() != 71
        || witness_script_bytes[0..2] != [0x52, 0x21]
        || witness_script_bytes[35] != 0x21
        || witness_script_bytes[69..71] != [0x52, 0xae]
    {
        return false;
    }
    let Ok(signature0) = strict_signature(signature0_bytes) else {
        return false;
    };
    let Ok(signature1) = strict_signature(signature1_bytes) else {
        return false;
    };
    let Ok(key0) = PublicKey::from_slice(&witness_script_bytes[2..35]) else {
        return false;
    };
    let Ok(key1) = PublicKey::from_slice(&witness_script_bytes[36..69]) else {
        return false;
    };
    let script = ScriptBuf::from_bytes(witness_script_bytes.to_vec());
    let Ok(sighash) = SighashCache::new(transaction).p2wsh_signature_hash(
        input_index,
        &script,
        Amount::from_sat(prevout.value.to_sat()),
        EcdsaSighashType::All,
    ) else {
        return false;
    };
    let message = Message::from_digest(sighash.to_byte_array());
    let secp = Secp256k1::verification_only();
    secp.verify_ecdsa(&message, &signature0.signature, &key0)
        .is_ok()
        && secp
            .verify_ecdsa(&message, &signature1.signature, &key1)
            .is_ok()
}

fn strict_signature(bytes: &[u8]) -> Result<ecdsa::Signature> {
    let signature =
        ecdsa::Signature::from_slice(bytes).context("invalid Bitcoin ECDSA signature")?;
    ensure!(
        signature.sighash_type == EcdsaSighashType::All,
        "sighash is not ALL"
    );
    let mut normalized = signature.signature;
    normalized.normalize_s();
    ensure!(normalized == signature.signature, "high-S signature");
    ensure!(signature.to_vec() == bytes, "noncanonical DER signature");
    Ok(signature)
}

fn txid_wire(txid: Txid) -> Hash32 {
    fixed_hash(serialize(&txid))
}

fn txid_from_wire(hash: Hash32) -> Result<Txid> {
    deserialize(&hash.0).context("invalid wire txid")
}

fn wtxid_wire(wtxid: bitcoin::Wtxid) -> Hash32 {
    fixed_hash(serialize(&wtxid))
}

fn block_hash_wire(hash: bitcoin::BlockHash) -> Hash32 {
    fixed_hash(serialize(&hash))
}

fn fixed_hash(bytes: Vec<u8>) -> Hash32 {
    Hash32(
        bytes
            .try_into()
            .expect("serialized Bitcoin hash has 32 bytes"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_wire_txid_round_trip() {
        let display = "0000000000000000000000000000000000000000000000000000000000000001";
        let wire = tandem_core::wire_hash(display).expect("wire");
        let txid = txid_from_wire(wire).expect("txid");
        assert_eq!(txid.to_string(), display);
        assert_eq!(txid_wire(txid), wire);
    }

    #[test]
    fn core_status_requires_synced_txindex() {
        let status = CoreStatus {
            chain: "regtest".to_owned(),
            blocks: 10,
            headers: 10,
            initial_block_download: false,
            txindex_synced: false,
        };
        assert!(!status.ready_for(Network::Regtest));
    }
}
