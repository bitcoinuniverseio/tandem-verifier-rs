//! Consensus data types with fixed-width encodings.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use bitcoin::hashes::{Hash as BitcoinHash, hash160};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

/// Fixed Tandem wire magic.
pub const MAGIC: [u8; 4] = *b"TNDM";
/// Fixed protocol version.
pub const VERSION: u8 = 1;
/// Carrier amount in satoshis.
pub const CARRIER_VALUE: u64 = 20_000;
/// Refund relative sequence in blocks.
pub const REFUND_DELAY: u32 = 52_560;
/// Founding window in blocks.
pub const FOUNDING_WINDOW: u32 = 4_320;
/// Required INIT lead in blocks.
pub const INIT_LEAD: u32 = 1_008;
/// Required change floor in satoshis.
pub const CHANGE_FLOOR: u64 = 1_000;
/// Replaceable sequence used by marked operations.
pub const RBF_SEQUENCE: u32 = 0xffff_fffd;

/// A fixed 32-byte digest serialized as lowercase hexadecimal.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Hash32(pub [u8; 32]);

impl Hash32 {
    /// All-zero digest.
    pub const ZERO: Self = Self([0; 32]);

    /// Hash arbitrary bytes with SHA256.
    pub fn sha256(bytes: impl AsRef<[u8]>) -> Self {
        Self(Sha256::digest(bytes.as_ref()).into())
    }

    /// Parse lowercase or uppercase hexadecimal.
    ///
    /// # Errors
    ///
    /// Returns a hex decoder error for invalid text or a length other than 32 bytes.
    pub fn from_hex(value: &str) -> Result<Self, hex::FromHexError> {
        let mut bytes = [0_u8; 32];
        hex::decode_to_slice(value, &mut bytes)?;
        Ok(Self(bytes))
    }

    /// Return lowercase hexadecimal.
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    /// Return true for the absent digest.
    pub fn is_zero(self) -> bool {
        self.0 == [0; 32]
    }
}

impl fmt::Debug for Hash32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl fmt::Display for Hash32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl FromStr for Hash32 {
    type Err = hex::FromHexError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_hex(value)
    }
}

impl Serialize for Hash32 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash32 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_hex(&value).map_err(serde::de::Error::custom)
    }
}

/// A wire-order Bitcoin outpoint.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct OutPointRef {
    /// Transaction hash in Bitcoin wire order.
    pub txid: Hash32,
    /// Output index.
    pub vout: u32,
}

impl OutPointRef {
    /// Absent outpoint.
    pub const ZERO: Self = Self {
        txid: Hash32::ZERO,
        vout: 0,
    };

    /// Serialize to the fixed 36-byte leaf encoding.
    pub fn wire_bytes(self) -> [u8; 36] {
        let mut bytes = [0_u8; 36];
        bytes[..32].copy_from_slice(&self.txid.0);
        bytes[32..].copy_from_slice(&self.vout.to_le_bytes());
        bytes
    }
}

/// Compressed secp256k1 public key bytes.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Key33(pub [u8; 33]);

impl Key33 {
    /// Absent key bytes.
    pub const ZERO: Self = Self([0; 33]);

    /// Parse hexadecimal bytes.
    ///
    /// # Errors
    ///
    /// Returns a hex decoder error for invalid text or a length other than 33 bytes.
    pub fn from_hex(value: &str) -> Result<Self, hex::FromHexError> {
        let mut bytes = [0_u8; 33];
        hex::decode_to_slice(value, &mut bytes)?;
        Ok(Self(bytes))
    }

    /// Return lowercase hexadecimal.
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    /// Return the exact native P2WPKH output script.
    pub fn p2wpkh_script(self) -> Vec<u8> {
        let program = hash160::Hash::hash(&self.0).to_byte_array();
        let mut script = Vec::with_capacity(22);
        script.extend_from_slice(&[0x00, 0x14]);
        script.extend_from_slice(&program);
        script
    }

    /// Return true when this is a valid compressed secp256k1 point.
    pub fn is_valid(self) -> bool {
        matches!(self.0[0], 0x02 | 0x03) && secp256k1::PublicKey::from_slice(&self.0).is_ok()
    }
}

impl Default for Key33 {
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Debug for Key33 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl Serialize for Key33 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Key33 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_hex(&value).map_err(serde::de::Error::custom)
    }
}

/// Sorted carrier participant keys.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeyPair {
    /// Lexicographically smaller key.
    pub key0: Key33,
    /// Lexicographically greater key.
    pub key1: Key33,
}

impl KeyPair {
    /// Construct only a valid sorted, distinct pair.
    pub fn checked(key0: Key33, key1: Key33) -> Option<Self> {
        (key0.is_valid() && key1.is_valid() && key0 < key1).then_some(Self { key0, key1 })
    }

    /// Exact 71-byte 2-of-2 witness script.
    pub fn witness_script(self) -> Vec<u8> {
        let mut script = Vec::with_capacity(71);
        script.extend_from_slice(&[0x52, 0x21]);
        script.extend_from_slice(&self.key0.0);
        script.push(0x21);
        script.extend_from_slice(&self.key1.0);
        script.extend_from_slice(&[0x52, 0xae]);
        script
    }

    /// Exact native P2WSH carrier output script.
    pub fn carrier_script(self) -> Vec<u8> {
        let digest = Hash32::sha256(self.witness_script());
        let mut script = Vec::with_capacity(34);
        script.extend_from_slice(&[0x00, 0x20]);
        script.extend_from_slice(&digest.0);
        script
    }

    /// Return true when the key is one of the pair.
    pub fn contains(self, key: Key33) -> bool {
        self.key0.0 == key.0 || self.key1.0 == key.0
    }
}

/// Bitcoin network bound by the deployment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    /// Bitcoin mainnet.
    Mainnet,
    /// Bitcoin signet.
    Signet,
    /// Bitcoin testnet4.
    Testnet4,
    /// Local regtest.
    Regtest,
}

impl Network {
    /// On-chain network byte.
    pub const fn code(self) -> u8 {
        match self {
            Self::Mainnet => 0,
            Self::Signet => 1,
            Self::Testnet4 => 2,
            Self::Regtest => 3,
        }
    }

    /// Stable protocol identifier label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Signet => "signet",
            Self::Testnet4 => "testnet4",
            Self::Regtest => "regtest",
        }
    }
}

impl FromStr for Network {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "mainnet" => Ok(Self::Mainnet),
            "signet" => Ok(Self::Signet),
            "testnet4" => Ok(Self::Testnet4),
            "regtest" => Ok(Self::Regtest),
            _ => Err(format!("unsupported Bitcoin network: {value}")),
        }
    }
}

/// Immutable external protocol binding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Binding {
    /// Bound Bitcoin network.
    pub network: Network,
    /// Configured INIT transaction hash in wire order.
    pub init_txid: Hash32,
    /// SHA256 of the exact normative specification bytes.
    pub spec_hash: Hash32,
}

impl Binding {
    /// Stable protocol identifier using display-order INIT txid text supplied by the caller.
    pub fn protocol_id(&self) -> String {
        format!(
            "tndm:{}:{}",
            self.network.label(),
            display_hash(self.init_txid)
        )
    }

    /// Fixed namespace commitment.
    pub fn namespace(&self) -> Hash32 {
        let mut preimage = Vec::with_capacity(17 + 1 + 32 + 32);
        preimage.extend_from_slice(b"TANDEM/NAMESPACE\0");
        preimage.push(self.network.code());
        preimage.extend_from_slice(&self.init_txid.0);
        preimage.extend_from_slice(&self.spec_hash.0);
        Hash32::sha256(preimage)
    }
}

/// Reverse a wire-order Bitcoin hash for conventional display.
pub fn display_hash(hash: Hash32) -> String {
    let mut bytes = hash.0;
    bytes.reverse();
    hex::encode(bytes)
}

/// Convert conventional display hexadecimal to wire order.
///
/// # Errors
///
/// Returns a hex decoder error for invalid text or a length other than 32 bytes.
pub fn wire_hash(display: &str) -> Result<Hash32, hex::FromHexError> {
    let mut hash = Hash32::from_hex(display)?;
    hash.0.reverse();
    Ok(hash)
}

/// Derive the binary object key for a CREATE output at vout 1.
pub fn object_key(namespace: Hash32, create_txid: Hash32) -> Hash32 {
    let mut preimage = Vec::with_capacity(14 + 32 + 32 + 4);
    preimage.extend_from_slice(b"TANDEM/OBJECT\0");
    preimage.extend_from_slice(&namespace.0);
    preimage.extend_from_slice(&create_txid.0);
    preimage.extend_from_slice(&1_u32.to_le_bytes());
    Hash32::sha256(preimage)
}

/// Defined marker operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
pub enum Opcode {
    /// Deployment INIT.
    Init = 0,
    /// Create object.
    Create = 1,
    /// Append chapter.
    Mark = 2,
    /// Rotate participant keys.
    Rotate = 3,
    /// Cooperatively close.
    Close = 4,
}

impl Opcode {
    /// Decode a defined opcode.
    pub const fn from_byte(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Init),
            1 => Some(Self::Create),
            2 => Some(Self::Mark),
            3 => Some(Self::Rotate),
            4 => Some(Self::Close),
            _ => None,
        }
    }

    /// Exact payload length.
    pub const fn payload_len(self) -> usize {
        match self {
            Self::Init => 59,
            Self::Create => 40,
            Self::Mark => 78,
            Self::Rotate => 44,
            Self::Close => 80,
        }
    }
}

/// Stable validation reason.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u16)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Reason {
    /// Valid recognized operation.
    Valid = 0x0000,
    /// More than one marker candidate.
    MultipleMarkers = 0x0001,
    /// Marker encoding or length failure.
    BadMarkerEncodingOrLength = 0x0002,
    /// Unknown version.
    UnknownVersion = 0x0003,
    /// Wrong bound network.
    WrongNetwork = 0x0004,
    /// Unknown opcode.
    UnknownOpcode = 0x0005,
    /// Wrong namespace commitment.
    WrongNamespace = 0x0006,
    /// Fixed or reserved field mismatch.
    UnsupportedOrReservedField = 0x0007,
    /// Wrong transaction version or locktime.
    BadTxVersionOrLocktime = 0x0010,
    /// Wrong input count, order, role, or sequence.
    BadInputCountOrOrder = 0x0011,
    /// Wrong output count, order, or role.
    BadOutputCountOrOrder = 0x0012,
    /// Prevout lacks earlier-block confirmation.
    UnconfirmedOrSameBlockPrevout = 0x0013,
    /// Input script or witness shape failure.
    BadInputScript = 0x0014,
    /// Invalid key order or binding.
    BadKeyOrderOrBinding = 0x0015,
    /// Signature or sighash failure.
    BadSignatureOrSighash = 0x0016,
    /// Output script or amount failure.
    BadOutputScriptOrValue = 0x0017,
    /// Fee is nonpositive or invalid.
    NonpositiveOrInvalidFee = 0x0018,
    /// Fee split, payout, sponsor equation, or change floor failure.
    BadFeeSplitOrChange = 0x0019,
    /// Required active predecessor is absent.
    PredecessorNotActive = 0x001a,
    /// State sequence failure.
    BadStateSequence = 0x001b,
    /// Successor script failure.
    BadSuccessor = 0x001c,
    /// Required nonzero commitment is zero.
    BadCommitment = 0x001d,
    /// Height or phase failure.
    BadHeightOrPhase = 0x001e,
    /// Refund shape or maturity failure.
    BadRefundShapeOrMaturity = 0x001f,
    /// More than one active carrier consumed.
    MultipleCarriers = 0x0020,
    /// Markerless non-refund carrier spend.
    UnmarkedCarrierSpend = 0x0030,
}

/// Event type byte.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    /// Valid INIT.
    Init = 0,
    /// Valid or attempted CREATE.
    Create = 1,
    /// Valid or attempted MARK.
    Mark = 2,
    /// Valid or attempted ROTATE.
    Rotate = 3,
    /// Valid or attempted CLOSE.
    Close = 4,
    /// Valid REFUND.
    Refund = 5,
    /// Terminal invalid carrier spend.
    ExitedNoncanonical = 6,
    /// Unclassified invalid marker.
    Invalid = 7,
}

impl From<Opcode> for EventType {
    fn from(value: Opcode) -> Self {
        match value {
            Opcode::Init => Self::Init,
            Opcode::Create => Self::Create,
            Opcode::Mark => Self::Mark,
            Opcode::Rotate => Self::Rotate,
            Opcode::Close => Self::Close,
        }
    }
}

/// Event validity class.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidityClass {
    /// Invalid event without state change.
    InvalidNoState = 0,
    /// Valid operation.
    ValidOperation = 1,
    /// Carrier spend that terminates canonical state.
    TerminalNoncanonical = 2,
}

/// Canonical object status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ObjectStatus {
    /// Active carrier exists.
    Active = 0,
    /// Cooperatively closed.
    Closed = 1,
    /// Mature refund.
    Refunded = 2,
    /// Consumed by an invalid operation.
    ExitedNoncanonical = 3,
}

/// Resolved transaction input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InputView {
    /// Input prevout.
    pub prevout: OutPointRef,
    /// Input sequence.
    pub sequence: u32,
    /// Exact scriptSig bytes.
    #[serde(with = "hex_bytes")]
    pub script_sig: Vec<u8>,
    /// Exact witness stack.
    #[serde(with = "hex_vec")]
    pub witness: Vec<Vec<u8>>,
    /// Spent output amount in satoshis.
    pub prevout_value: u64,
    /// Spent output scriptPubKey bytes.
    #[serde(with = "hex_bytes")]
    pub prevout_script: Vec<u8>,
    /// Canonical confirmation height of the spent output.
    pub prevout_height: Option<u64>,
    /// Result of independent signature and sighash validation.
    pub signatures_valid: bool,
}

impl InputView {
    /// Decode the 33-byte key from a native P2WPKH witness shape.
    pub fn p2wpkh_revealed_key(&self) -> Option<Key33> {
        if !self.script_sig.is_empty()
            || self.witness.len() != 2
            || self.prevout_script.len() != 22
            || self.prevout_script[..2] != [0x00, 0x14]
        {
            return None;
        }
        Some(Key33(self.witness[1].as_slice().try_into().ok()?))
    }

    /// Decode a valid native P2WPKH witness key bound to its prevout.
    pub fn p2wpkh_key(&self) -> Option<Key33> {
        let key = self.p2wpkh_revealed_key()?;
        if !key.is_valid() || key.p2wpkh_script() != self.prevout_script {
            return None;
        }
        Some(key)
    }

    /// Decode carrier keys from the exact multisig witness shape.
    pub fn carrier_revealed_keys(&self) -> Option<(Key33, Key33)> {
        if !self.script_sig.is_empty()
            || self.witness.len() != 4
            || !self.witness[0].is_empty()
            || self.prevout_script.len() != 34
            || self.prevout_script[..2] != [0x00, 0x20]
        {
            return None;
        }
        let script = &self.witness[3];
        if script.len() != 71
            || script[0..2] != [0x52, 0x21]
            || script[35] != 0x21
            || script[69..71] != [0x52, 0xae]
        {
            return None;
        }
        Some((
            Key33(script[2..35].try_into().ok()?),
            Key33(script[36..69].try_into().ok()?),
        ))
    }

    /// Decode a valid carrier witness shape and key pair bound to its prevout.
    pub fn carrier_keys(&self) -> Option<KeyPair> {
        let (key0, key1) = self.carrier_revealed_keys()?;
        let pair = KeyPair::checked(key0, key1)?;
        (pair.carrier_script() == self.prevout_script).then_some(pair)
    }

    /// Return true when the prevout was confirmed before the containing block.
    pub fn has_earlier_confirmation(&self, block_height: u64) -> bool {
        self.prevout_height
            .is_some_and(|height| height < block_height)
    }
}

/// Transaction output view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OutputView {
    /// Amount in satoshis.
    pub value: u64,
    /// Exact scriptPubKey bytes.
    #[serde(with = "hex_bytes")]
    pub script_pubkey: Vec<u8>,
}

/// Fully resolved transaction view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TxView {
    /// Transaction hash in wire order.
    pub txid: Hash32,
    /// Witness transaction hash in wire order.
    pub wtxid: Hash32,
    /// Consensus version.
    pub version: i32,
    /// Consensus locktime.
    pub lock_time: u32,
    /// Resolved inputs.
    pub inputs: Vec<InputView>,
    /// Outputs.
    pub outputs: Vec<OutputView>,
}

/// Canonical block view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlockView {
    /// Block hash in wire order.
    pub hash: Hash32,
    /// Parent block hash in wire order.
    pub previous_hash: Hash32,
    /// Canonical height.
    pub height: u64,
    /// Transactions in block order, including coinbase.
    pub transactions: Vec<TxView>,
}

/// Canonical event fields.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Event {
    /// Namespace field.
    pub namespace: Hash32,
    /// Block hash.
    pub block_hash: Hash32,
    /// Block height.
    pub height: u64,
    /// Transaction index.
    pub tx_index: u32,
    /// Marker output index, or `u32::MAX`.
    pub event_index: u32,
    /// Per-carrier sub-index.
    pub sub_index: u32,
    /// Event type.
    pub event_type: EventType,
    /// Validity class.
    pub validity_class: ValidityClass,
    /// Stable reason.
    pub reason: Reason,
    /// Transaction hash.
    pub txid: Hash32,
    /// Witness transaction hash.
    pub wtxid: Hash32,
    /// Object key or zero.
    pub object_key: Hash32,
    /// State sequence or `u32::MAX`.
    pub state_seq: u32,
    /// Consumed carrier or zero.
    pub predecessor: OutPointRef,
    /// Successor carrier or zero.
    pub successor: OutPointRef,
    /// Current or successor key0.
    pub key0: Key33,
    /// Current or successor key1.
    pub key1: Key33,
    /// Operation commitment or zero.
    pub commitment: Hash32,
}

/// Canonical Tandem object state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ObjectState {
    /// Binary object key.
    pub object_key: Hash32,
    /// CREATE genesis outpoint.
    pub genesis: OutPointRef,
    /// Founding-window classification.
    pub founding: bool,
    /// Current status.
    pub status: ObjectStatus,
    /// Canonical CREATE height.
    pub create_height: u64,
    /// Current state sequence.
    pub state_seq: u32,
    /// Active outpoint or absent zero outpoint.
    pub current_outpoint: OutPointRef,
    /// Current key pair.
    pub keys: KeyPair,
    /// Terminal transaction hash or zero.
    pub terminal_txid: Hash32,
    /// Number of valid MARK operations.
    pub chapter_count: u32,
}

/// Configured protocol lifecycle state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProtocolStatus {
    /// INIT is not yet on the processed canonical chain.
    AwaitingInit,
    /// INIT is valid and supplies phase heights.
    Active {
        /// INIT canonical confirmation height.
        init_height: u64,
        /// CREATE opening height.
        h_open: u32,
        /// Founding close height.
        h_close: u32,
    },
    /// Configured INIT is canonical but invalid.
    FailedInit {
        /// Stable validation reason.
        reason: Reason,
        /// INIT canonical confirmation height.
        init_height: u64,
    },
}

/// All canonical state required to reduce the next block.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChainState {
    /// Immutable deployment binding.
    pub binding: Binding,
    /// Protocol lifecycle.
    pub protocol_status: ProtocolStatus,
    /// Last applied canonical height.
    pub tip_height: Option<u64>,
    /// Last applied canonical block hash.
    pub tip_hash: Option<Hash32>,
    /// Last chained root, or the pre-INIT empty root.
    pub chained_root: Hash32,
    /// Objects ordered by object key.
    pub objects: BTreeMap<Hash32, ObjectState>,
    /// Active carrier to object-key index.
    pub active: BTreeMap<OutPointRef, Hash32>,
}

impl ChainState {
    /// Construct a fail-closed state from an explicit binding.
    pub fn new(binding: Binding) -> Self {
        let namespace = binding.namespace();
        let mut preimage = Vec::with_capacity(19 + 32);
        preimage.extend_from_slice(b"TANDEM/STATE-EMPTY\0");
        preimage.extend_from_slice(&namespace.0);
        Self {
            binding,
            protocol_status: ProtocolStatus::AwaitingInit,
            tip_height: None,
            tip_hash: None,
            chained_root: Hash32::sha256(preimage),
            objects: BTreeMap::new(),
            active: BTreeMap::new(),
        }
    }

    /// Return post-block counters.
    pub fn counters(&self) -> Counters {
        Counters {
            founding_created: self
                .objects
                .values()
                .filter(|object| object.founding)
                .count() as u64,
            all_objects: self.objects.len() as u64,
            active_objects: self
                .objects
                .values()
                .filter(|object| object.status == ObjectStatus::Active)
                .count() as u64,
        }
    }
}

/// Post-block protocol counters.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Counters {
    /// Founding CREATE count.
    pub founding_created: u64,
    /// Total canonical object count.
    pub all_objects: u64,
    /// Active object count.
    pub active_objects: u64,
}

/// Per-height root tuple.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HeightRoots {
    /// Canonical height.
    pub height: u64,
    /// Canonical block hash.
    pub block_hash: Hash32,
    /// Event Merkle root.
    pub event_root: Hash32,
    /// Object snapshot Merkle root.
    pub object_state_root: Hash32,
    /// Chained block root.
    pub chained_root: Hash32,
    /// Post-block counters.
    pub counters: Counters,
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        hex::decode(value).map_err(serde::de::Error::custom)
    }
}

mod hex_vec {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(value: &[Vec<u8>], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value
            .iter()
            .map(hex::encode)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<String>::deserialize(deserializer)?
            .into_iter()
            .map(|value| hex::decode(value).map_err(serde::de::Error::custom))
            .collect()
    }
}
