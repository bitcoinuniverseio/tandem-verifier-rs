//! Deterministic block dispatch, validation, state reduction, and rollback.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::marker::{MarkerCandidate, ParsedMarker, find_marker_candidate};
use crate::roots::{block_root, event_root, object_state_root};
use crate::{
    Binding, BlockView, CARRIER_VALUE, CHANGE_FLOOR, ChainState, Event, EventType, FOUNDING_WINDOW,
    Hash32, HeightRoots, INIT_LEAD, Key33, KeyPair, ObjectState, ObjectStatus, Opcode, OutPointRef,
    ProtocolStatus, RBF_SEQUENCE, REFUND_DELAY, Reason, TxView, VERSION, ValidityClass, object_key,
};

/// Reversible result of one canonical block application.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlockDelta {
    /// Complete state before the block, used as an exact inverse journal.
    pub before: ChainState,
    /// Canonical block hash.
    pub block_hash: Hash32,
    /// Canonical parent block hash.
    pub previous_hash: Hash32,
    /// Canonical block height.
    pub height: u64,
    /// Ordered canonical events.
    pub events: Vec<Event>,
    /// Post-block root tuple.
    pub roots: HeightRoots,
}

/// Reducer boundary failure.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum ReducerError {
    /// The first supplied block did not contain the configured INIT.
    #[error("first processed block does not contain configured INIT")]
    ConfiguredInitAbsent,
    /// Block height is not consecutive.
    #[error("nonconsecutive height: expected {expected}, got {actual}")]
    NonconsecutiveHeight {
        /// Required next height.
        expected: u64,
        /// Supplied height.
        actual: u64,
    },
    /// Parent hash does not match the current tip.
    #[error("block parent does not match canonical tip")]
    ParentMismatch,
    /// A disconnect did not target the current tip.
    #[error("disconnect target is not the canonical tip")]
    DisconnectMismatch,
    /// Transaction index exceeds the event encoding.
    #[error("block contains more transactions than event indexing supports")]
    TransactionIndexOverflow,
}

#[derive(Clone, Copy, Debug)]
struct InitParams {
    h_open: u32,
    h_close: u32,
}

#[derive(Clone, Debug)]
enum ValidatedOperation {
    Create {
        keys: KeyPair,
    },
    Mark {
        object: ObjectState,
        sequence: u32,
        commitment: Hash32,
    },
    Rotate {
        object: ObjectState,
        sequence: u32,
        keys: KeyPair,
    },
    Close {
        object: ObjectState,
        sequence: u32,
        commitment: Hash32,
    },
}

/// Apply one canonical block atomically in memory.
///
/// # Errors
///
/// Returns a linkage, INIT location, or transaction-index error without
/// committing a partial block result.
pub fn apply_block(state: &mut ChainState, block: &BlockView) -> Result<BlockDelta, ReducerError> {
    validate_linkage(state, block)?;
    if block.transactions.len() > u32::MAX as usize {
        return Err(ReducerError::TransactionIndexOverflow);
    }
    let before = state.clone();
    let namespace = state.binding.namespace();
    let mut events = Vec::new();

    match state.protocol_status.clone() {
        ProtocolStatus::AwaitingInit => {
            let Some((init_index, init_tx)) = block
                .transactions
                .iter()
                .enumerate()
                .find(|(_, tx)| tx.txid == state.binding.init_txid)
            else {
                return Err(ReducerError::ConfiguredInitAbsent);
            };
            let init_index =
                u32::try_from(init_index).map_err(|_| ReducerError::TransactionIndexOverflow)?;
            match validate_configured_init(&state.binding, block.height, init_tx) {
                Ok(params) => {
                    state.protocol_status = ProtocolStatus::Active {
                        init_height: block.height,
                        h_open: params.h_open,
                        h_close: params.h_close,
                    };
                    for (index, tx) in block.transactions.iter().enumerate() {
                        let tx_index = u32::try_from(index)
                            .map_err(|_| ReducerError::TransactionIndexOverflow)?;
                        if tx.txid == state.binding.init_txid {
                            events.push(valid_init_event(state, block, tx, tx_index));
                        } else {
                            events.extend(process_transaction(state, block, tx, tx_index));
                        }
                    }
                }
                Err(reason) => {
                    state.protocol_status = ProtocolStatus::FailedInit {
                        reason,
                        init_height: block.height,
                    };
                    events.push(invalid_event(
                        &state.binding,
                        block,
                        init_tx,
                        init_index,
                        reason,
                        true,
                    ));
                }
            }
        }
        ProtocolStatus::Active { .. } => {
            for (index, tx) in block.transactions.iter().enumerate() {
                let tx_index =
                    u32::try_from(index).map_err(|_| ReducerError::TransactionIndexOverflow)?;
                events.extend(process_transaction(state, block, tx, tx_index));
            }
        }
        ProtocolStatus::FailedInit { .. } => {}
    }

    let event_root_value = event_root(namespace, &events);
    let object_root_value = object_state_root(namespace, state.objects.values());
    let counters = state.counters();
    let chained_root = block_root(
        namespace,
        state.chained_root,
        block.hash,
        block.height,
        event_root_value,
        object_root_value,
        counters,
    );
    state.tip_height = Some(block.height);
    state.tip_hash = Some(block.hash);
    state.chained_root = chained_root;
    let roots = HeightRoots {
        height: block.height,
        block_hash: block.hash,
        event_root: event_root_value,
        object_state_root: object_root_value,
        chained_root,
        counters,
    };
    Ok(BlockDelta {
        before,
        block_hash: block.hash,
        previous_hash: block.previous_hash,
        height: block.height,
        events,
        roots,
    })
}

/// Disconnect the exact current tip by restoring its inverse journal.
///
/// # Errors
///
/// Returns [`ReducerError::DisconnectMismatch`] when the journal does not
/// identify the current canonical tip.
pub fn disconnect_block(state: &mut ChainState, delta: &BlockDelta) -> Result<(), ReducerError> {
    if state.tip_hash != Some(delta.block_hash) || state.tip_height != Some(delta.height) {
        return Err(ReducerError::DisconnectMismatch);
    }
    *state = delta.before.clone();
    Ok(())
}

fn validate_linkage(state: &ChainState, block: &BlockView) -> Result<(), ReducerError> {
    if let Some(height) = state.tip_height {
        let expected = height
            .checked_add(1)
            .ok_or(ReducerError::NonconsecutiveHeight {
                expected: u64::MAX,
                actual: block.height,
            })?;
        if block.height != expected {
            return Err(ReducerError::NonconsecutiveHeight {
                expected,
                actual: block.height,
            });
        }
        if state.tip_hash != Some(block.previous_hash) {
            return Err(ReducerError::ParentMismatch);
        }
    }
    Ok(())
}

fn validate_configured_init(
    binding: &Binding,
    block_height: u64,
    tx: &TxView,
) -> Result<InitParams, Reason> {
    let candidates = marker_candidates(tx, binding);
    if candidates.len() != 1 {
        return Err(if candidates.len() > 1 {
            Reason::MultipleMarkers
        } else {
            Reason::BadMarkerEncodingOrLength
        });
    }
    let marker = candidates[0].parse().map_err(|error| error.reason)?;
    validate_marker_header(&marker, binding, Opcode::Init)?;
    let h_open = marker.u32_at(7);
    let h_close = marker.u32_at(11);
    if marker.u64_at(15) != CARRIER_VALUE
        || marker.u32_at(23) != REFUND_DELAY
        || marker.hash_at(27) != binding.spec_hash
    {
        return Err(Reason::UnsupportedOrReservedField);
    }
    if tx.version != 2 || tx.lock_time != 0 {
        return Err(Reason::BadTxVersionOrLocktime);
    }
    if tx.inputs.len() != 1 || tx.inputs[0].sequence != u32::MAX {
        return Err(Reason::BadInputCountOrOrder);
    }
    if tx.outputs.len() != 2 || marker.vout != 0 || has_other_op_return(tx, 0) {
        return Err(Reason::BadOutputCountOrOrder);
    }
    let input = &tx.inputs[0];
    if !input.has_earlier_confirmation(block_height) {
        return Err(Reason::UnconfirmedOrSameBlockPrevout);
    }
    if input.p2wpkh_revealed_key().is_none() {
        return Err(Reason::BadInputScript);
    }
    let Some(key) = input.p2wpkh_key() else {
        return Err(Reason::BadKeyOrderOrBinding);
    };
    if !input.signatures_valid {
        return Err(Reason::BadSignatureOrSighash);
    }
    if tx.outputs[0].value != 0 || tx.outputs[1].script_pubkey != key.p2wpkh_script() {
        return Err(Reason::BadOutputScriptOrValue);
    }
    let fee = input
        .prevout_value
        .checked_sub(tx.outputs[1].value)
        .filter(|fee| *fee > 0)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    let _ = fee;
    if tx.outputs[1].value < CHANGE_FLOOR {
        return Err(Reason::BadFeeSplitOrChange);
    }
    let expected_close = h_open
        .checked_add(FOUNDING_WINDOW)
        .ok_or(Reason::BadHeightOrPhase)?;
    let init_height = u32::try_from(block_height).map_err(|_| Reason::BadHeightOrPhase)?;
    let earliest_open = init_height
        .checked_add(INIT_LEAD)
        .ok_or(Reason::BadHeightOrPhase)?;
    if expected_close != h_close || earliest_open > h_open {
        return Err(Reason::BadHeightOrPhase);
    }
    Ok(InitParams { h_open, h_close })
}

#[allow(clippy::too_many_lines)]
fn process_transaction(
    state: &mut ChainState,
    block: &BlockView,
    tx: &TxView,
    tx_index: u32,
) -> Vec<Event> {
    let candidates = marker_candidates(tx, &state.binding);
    let carriers = carrier_spends(state, tx);

    if candidates.len() > 1 {
        let event_index = candidates
            .iter()
            .map(|candidate| candidate.vout)
            .min()
            .unwrap_or(u32::MAX);
        if carriers.is_empty() {
            return vec![bare_invalid_event(
                block,
                tx,
                tx_index,
                event_index,
                EventType::Invalid,
                Reason::MultipleMarkers,
                Hash32::ZERO,
            )];
        }
        return terminate_carriers(
            state,
            block,
            tx,
            tx_index,
            event_index,
            Reason::MultipleMarkers,
            Hash32::ZERO,
            carriers,
        );
    }

    if carriers.len() > 1 {
        let event_index = candidates
            .first()
            .map_or(u32::MAX, |candidate| candidate.vout);
        let namespace = candidates
            .first()
            .and_then(population_namespace)
            .unwrap_or_else(|| {
                if candidates.is_empty() {
                    state.binding.namespace()
                } else {
                    Hash32::ZERO
                }
            });
        return terminate_carriers(
            state,
            block,
            tx,
            tx_index,
            event_index,
            Reason::MultipleCarriers,
            namespace,
            carriers,
        );
    }

    if candidates.is_empty() {
        let Some((_, object)) = carriers.first() else {
            return Vec::new();
        };
        if tx.inputs.len() == 1 && tx.outputs.len() == 2 {
            if validate_refund(block.height, tx, object).is_ok() {
                let event = valid_refund_event(state, block, tx, tx_index, object);
                terminate_object(state, object, ObjectStatus::Refunded, tx.txid);
                return vec![event];
            }
            return terminate_carriers(
                state,
                block,
                tx,
                tx_index,
                u32::MAX,
                Reason::BadRefundShapeOrMaturity,
                state.binding.namespace(),
                carriers,
            );
        }
        return terminate_carriers(
            state,
            block,
            tx,
            tx_index,
            u32::MAX,
            Reason::UnmarkedCarrierSpend,
            state.binding.namespace(),
            carriers,
        );
    }

    let candidate = &candidates[0];
    let parsed = candidate.parse();
    let attempted_type = attempted_event_type(candidate);
    let namespace = population_namespace(candidate).unwrap_or(Hash32::ZERO);
    let carrier = carriers.first().cloned();
    let validation = parsed
        .map_err(|error| error.reason)
        .and_then(|marker| validate_single(state, block.height, tx, &marker, carrier.as_ref()));
    match validation {
        Ok(operation) => vec![apply_valid_operation(
            state,
            block,
            tx,
            tx_index,
            candidate.vout,
            operation,
        )],
        Err(reason) => {
            if carriers.is_empty() {
                vec![bare_invalid_event(
                    block,
                    tx,
                    tx_index,
                    candidate.vout,
                    attempted_type,
                    reason,
                    namespace,
                )]
            } else {
                terminate_carriers(
                    state,
                    block,
                    tx,
                    tx_index,
                    candidate.vout,
                    reason,
                    namespace,
                    carriers,
                )
            }
        }
    }
}

fn marker_candidates(tx: &TxView, binding: &Binding) -> Vec<MarkerCandidate> {
    tx.outputs
        .iter()
        .enumerate()
        .filter_map(|(index, output)| {
            let vout = u32::try_from(index).ok()?;
            let candidate = find_marker_candidate(&output.script_pubkey, vout)?;
            if candidate.is_init_candidate() && tx.txid != binding.init_txid {
                None
            } else {
                Some(candidate)
            }
        })
        .collect()
}

fn carrier_spends(state: &ChainState, tx: &TxView) -> Vec<(usize, ObjectState)> {
    tx.inputs
        .iter()
        .enumerate()
        .filter_map(|(index, input)| {
            state
                .active
                .get(&input.prevout)
                .and_then(|key| state.objects.get(key))
                .cloned()
                .map(|object| (index, object))
        })
        .collect()
}

fn validate_marker_header(
    marker: &ParsedMarker,
    binding: &Binding,
    required_opcode: Opcode,
) -> Result<(), Reason> {
    if marker.version != VERSION {
        return Err(Reason::UnknownVersion);
    }
    if marker.network != binding.network.code() {
        return Err(Reason::WrongNetwork);
    }
    let opcode = marker.opcode.ok_or(Reason::UnknownOpcode)?;
    if opcode != required_opcode {
        return Err(Reason::UnsupportedOrReservedField);
    }
    if opcode != Opcode::Init && marker.hash_at(8) != binding.namespace() {
        return Err(Reason::WrongNamespace);
    }
    Ok(())
}

fn validate_single(
    state: &ChainState,
    block_height: u64,
    tx: &TxView,
    marker: &ParsedMarker,
    carrier: Option<&(usize, ObjectState)>,
) -> Result<ValidatedOperation, Reason> {
    if marker.version != VERSION {
        return Err(Reason::UnknownVersion);
    }
    if marker.network != state.binding.network.code() {
        return Err(Reason::WrongNetwork);
    }
    let opcode = marker.opcode.ok_or(Reason::UnknownOpcode)?;
    if opcode != Opcode::Init && marker.hash_at(8) != state.binding.namespace() {
        return Err(Reason::WrongNamespace);
    }
    match opcode {
        Opcode::Init => Err(Reason::UnsupportedOrReservedField),
        Opcode::Create => validate_create(state, block_height, tx, marker),
        Opcode::Mark => validate_mark(state, block_height, tx, marker, carrier),
        Opcode::Rotate => validate_rotate(state, block_height, tx, marker, carrier),
        Opcode::Close => validate_close(state, block_height, tx, marker, carrier),
    }
}

#[allow(clippy::similar_names)]
fn validate_create(
    state: &ChainState,
    block_height: u64,
    tx: &TxView,
    marker: &ParsedMarker,
) -> Result<ValidatedOperation, Reason> {
    if marker.payload[7] != 1 {
        return Err(Reason::UnsupportedOrReservedField);
    }
    check_tx_header(tx)?;
    if tx.inputs.len() != 2 || tx.inputs.iter().any(|input| input.sequence != RBF_SEQUENCE) {
        return Err(Reason::BadInputCountOrOrder);
    }
    check_output_shape(tx, marker, 4)?;
    check_provenance(tx, block_height)?;
    if tx
        .inputs
        .iter()
        .any(|input| input.p2wpkh_revealed_key().is_none())
    {
        return Err(Reason::BadInputScript);
    }
    let key0 = tx.inputs[0]
        .p2wpkh_key()
        .ok_or(Reason::BadKeyOrderOrBinding)?;
    let key1 = tx.inputs[1]
        .p2wpkh_key()
        .ok_or(Reason::BadKeyOrderOrBinding)?;
    let keys = KeyPair::checked(key0, key1).ok_or(Reason::BadKeyOrderOrBinding)?;
    check_signatures(tx)?;
    if tx.outputs[0].value != 0
        || tx.outputs[1].value != CARRIER_VALUE
        || tx.outputs[1].script_pubkey != keys.carrier_script()
        || tx.outputs[2].script_pubkey != key0.p2wpkh_script()
        || tx.outputs[3].script_pubkey != key1.p2wpkh_script()
    {
        return Err(Reason::BadOutputScriptOrValue);
    }
    let debit0 = tx.inputs[0]
        .prevout_value
        .checked_sub(tx.outputs[2].value)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    let debit1 = tx.inputs[1]
        .prevout_value
        .checked_sub(tx.outputs[3].value)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    let total = debit0
        .checked_add(debit1)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    let fee = total
        .checked_sub(CARRIER_VALUE)
        .filter(|fee| *fee > 0)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    let ceil_half = fee / 2 + fee % 2;
    if debit0 != 10_000 + ceil_half
        || debit1 != 10_000 + fee / 2
        || tx.outputs[2].value < CHANGE_FLOOR
        || tx.outputs[3].value < CHANGE_FLOOR
    {
        return Err(Reason::BadFeeSplitOrChange);
    }
    let ProtocolStatus::Active { h_open, .. } = state.protocol_status else {
        return Err(Reason::BadHeightOrPhase);
    };
    if block_height < u64::from(h_open) {
        return Err(Reason::BadHeightOrPhase);
    }
    Ok(ValidatedOperation::Create { keys })
}

fn validate_mark(
    _state: &ChainState,
    block_height: u64,
    tx: &TxView,
    marker: &ParsedMarker,
    carrier: Option<&(usize, ObjectState)>,
) -> Result<ValidatedOperation, Reason> {
    if marker.payload[7] != 1 || marker.payload[44] > 5 || marker.payload[45] != 0 {
        return Err(Reason::UnsupportedOrReservedField);
    }
    check_tx_header(tx)?;
    if tx.inputs.len() != 2
        || tx.inputs.iter().any(|input| input.sequence != RBF_SEQUENCE)
        || carrier.is_some_and(|(index, _)| *index != 0)
    {
        return Err(Reason::BadInputCountOrOrder);
    }
    check_output_shape(tx, marker, 3)?;
    check_provenance(tx, block_height)?;
    check_transition_input_scripts(tx, &[0], &[1])?;
    let carrier_keys = tx.inputs[0]
        .carrier_keys()
        .ok_or(Reason::BadKeyOrderOrBinding)?;
    let sponsor = tx.inputs[1]
        .p2wpkh_key()
        .ok_or(Reason::BadKeyOrderOrBinding)?;
    let object = carrier.map(|(_, object)| object.clone());
    if object
        .as_ref()
        .is_some_and(|value| value.keys != carrier_keys || !value.keys.contains(sponsor))
        || object.is_none() && !carrier_keys.contains(sponsor)
    {
        return Err(Reason::BadKeyOrderOrBinding);
    }
    check_signatures(tx)?;
    if tx.outputs[0].value != 0
        || tx.outputs[1].value != CARRIER_VALUE
        || tx.outputs[2].script_pubkey != sponsor.p2wpkh_script()
    {
        return Err(Reason::BadOutputScriptOrValue);
    }
    let fee = tx.inputs[1]
        .prevout_value
        .checked_sub(tx.outputs[2].value)
        .filter(|fee| *fee > 0)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    let _ = fee;
    if tx.outputs[2].value < CHANGE_FLOOR {
        return Err(Reason::BadFeeSplitOrChange);
    }
    let object = object.ok_or(Reason::PredecessorNotActive)?;
    if tx.inputs[0].prevout != object.current_outpoint {
        return Err(Reason::PredecessorNotActive);
    }
    let sequence = marker.u32_at(40);
    if object.state_seq.checked_add(1) != Some(sequence) {
        return Err(Reason::BadStateSequence);
    }
    if tx.outputs[1].script_pubkey != object.keys.carrier_script() {
        return Err(Reason::BadSuccessor);
    }
    let commitment = marker.hash_at(46);
    if commitment.is_zero() {
        return Err(Reason::BadCommitment);
    }
    Ok(ValidatedOperation::Mark {
        object,
        sequence,
        commitment,
    })
}

#[allow(clippy::similar_names)]
fn validate_rotate(
    _state: &ChainState,
    block_height: u64,
    tx: &TxView,
    marker: &ParsedMarker,
    carrier: Option<&(usize, ObjectState)>,
) -> Result<ValidatedOperation, Reason> {
    if marker.payload[7] != 1 {
        return Err(Reason::UnsupportedOrReservedField);
    }
    check_tx_header(tx)?;
    if tx.inputs.len() != 3
        || tx.inputs.iter().any(|input| input.sequence != RBF_SEQUENCE)
        || carrier.is_some_and(|(index, _)| *index != 0)
    {
        return Err(Reason::BadInputCountOrOrder);
    }
    check_output_shape(tx, marker, 4)?;
    check_provenance(tx, block_height)?;
    check_transition_input_scripts(tx, &[0], &[1, 2])?;
    let old_keys = tx.inputs[0]
        .carrier_keys()
        .ok_or(Reason::BadKeyOrderOrBinding)?;
    let key0 = tx.inputs[1]
        .p2wpkh_key()
        .ok_or(Reason::BadKeyOrderOrBinding)?;
    let key1 = tx.inputs[2]
        .p2wpkh_key()
        .ok_or(Reason::BadKeyOrderOrBinding)?;
    let keys = KeyPair::checked(key0, key1).ok_or(Reason::BadKeyOrderOrBinding)?;
    let object = carrier.map(|(_, object)| object.clone());
    if object
        .as_ref()
        .is_some_and(|value| value.keys != old_keys || value.keys == keys)
        || object.is_none() && old_keys == keys
    {
        return Err(Reason::BadKeyOrderOrBinding);
    }
    check_signatures(tx)?;
    if tx.outputs[0].value != 0
        || tx.outputs[1].value != CARRIER_VALUE
        || tx.outputs[2].script_pubkey != key0.p2wpkh_script()
        || tx.outputs[3].script_pubkey != key1.p2wpkh_script()
    {
        return Err(Reason::BadOutputScriptOrValue);
    }
    let debit0 = tx.inputs[1]
        .prevout_value
        .checked_sub(tx.outputs[2].value)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    let debit1 = tx.inputs[2]
        .prevout_value
        .checked_sub(tx.outputs[3].value)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    let fee = debit0
        .checked_add(debit1)
        .filter(|fee| *fee > 0)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    if debit0 != fee / 2 + fee % 2
        || debit1 != fee / 2
        || tx.outputs[2].value < CHANGE_FLOOR
        || tx.outputs[3].value < CHANGE_FLOOR
    {
        return Err(Reason::BadFeeSplitOrChange);
    }
    let object = object.ok_or(Reason::PredecessorNotActive)?;
    if tx.inputs[0].prevout != object.current_outpoint {
        return Err(Reason::PredecessorNotActive);
    }
    let sequence = marker.u32_at(40);
    if object.state_seq.checked_add(1) != Some(sequence) {
        return Err(Reason::BadStateSequence);
    }
    if tx.outputs[1].script_pubkey != keys.carrier_script() {
        return Err(Reason::BadSuccessor);
    }
    Ok(ValidatedOperation::Rotate {
        object,
        sequence,
        keys,
    })
}

fn validate_close(
    _state: &ChainState,
    block_height: u64,
    tx: &TxView,
    marker: &ParsedMarker,
    carrier: Option<&(usize, ObjectState)>,
) -> Result<ValidatedOperation, Reason> {
    if marker.payload[7] != 0xff || marker.payload[44] > 3 || marker.payload[45..48] != [0, 0, 0] {
        return Err(Reason::UnsupportedOrReservedField);
    }
    check_tx_header(tx)?;
    if tx.inputs.len() != 1
        || tx.inputs[0].sequence != RBF_SEQUENCE
        || carrier.is_some_and(|(index, _)| *index != 0)
    {
        return Err(Reason::BadInputCountOrOrder);
    }
    check_output_shape(tx, marker, 3)?;
    check_provenance(tx, block_height)?;
    check_transition_input_scripts(tx, &[0], &[])?;
    let keys = tx.inputs[0]
        .carrier_keys()
        .ok_or(Reason::BadKeyOrderOrBinding)?;
    let object = carrier.map(|(_, object)| object.clone());
    if object.as_ref().is_some_and(|value| value.keys != keys) {
        return Err(Reason::BadKeyOrderOrBinding);
    }
    check_signatures(tx)?;
    if tx.outputs[0].value != 0
        || tx.outputs[1].script_pubkey != keys.key0.p2wpkh_script()
        || tx.outputs[2].script_pubkey != keys.key1.p2wpkh_script()
    {
        return Err(Reason::BadOutputScriptOrValue);
    }
    let outputs = tx.outputs[1]
        .value
        .checked_add(tx.outputs[2].value)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    let fee = CARRIER_VALUE
        .checked_sub(outputs)
        .filter(|fee| *fee > 0)
        .ok_or(Reason::NonpositiveOrInvalidFee)?;
    let _ = fee;
    if tx.outputs[1].value == 0 || tx.outputs[1].value != tx.outputs[2].value {
        return Err(Reason::BadFeeSplitOrChange);
    }
    let object = object.ok_or(Reason::PredecessorNotActive)?;
    if tx.inputs[0].prevout != object.current_outpoint {
        return Err(Reason::PredecessorNotActive);
    }
    let sequence = marker.u32_at(40);
    if object.state_seq.checked_add(1) != Some(sequence) {
        return Err(Reason::BadStateSequence);
    }
    Ok(ValidatedOperation::Close {
        object,
        sequence,
        commitment: marker.hash_at(48),
    })
}

fn validate_refund(block_height: u64, tx: &TxView, object: &ObjectState) -> Result<(), Reason> {
    if tx.version != 2
        || tx.lock_time != 0
        || tx.inputs.len() != 1
        || tx.outputs.len() != 2
        || tx
            .outputs
            .iter()
            .any(|output| is_op_return(&output.script_pubkey))
    {
        return Err(Reason::BadRefundShapeOrMaturity);
    }
    let input = &tx.inputs[0];
    if input.sequence != REFUND_DELAY
        || input.prevout != object.current_outpoint
        || !input.has_earlier_confirmation(block_height)
        || input.carrier_keys() != Some(object.keys)
        || !input.signatures_valid
    {
        return Err(Reason::BadRefundShapeOrMaturity);
    }
    let Some(confirmed_height) = input.prevout_height else {
        return Err(Reason::BadRefundShapeOrMaturity);
    };
    if confirmed_height.checked_add(u64::from(REFUND_DELAY)) > Some(block_height) {
        return Err(Reason::BadRefundShapeOrMaturity);
    }
    if tx.outputs[0].script_pubkey != object.keys.key0.p2wpkh_script()
        || tx.outputs[1].script_pubkey != object.keys.key1.p2wpkh_script()
        || tx.outputs[0].value == 0
        || tx.outputs[0].value != tx.outputs[1].value
    {
        return Err(Reason::BadRefundShapeOrMaturity);
    }
    let output_sum = tx.outputs[0]
        .value
        .checked_add(tx.outputs[1].value)
        .ok_or(Reason::BadRefundShapeOrMaturity)?;
    CARRIER_VALUE
        .checked_sub(output_sum)
        .filter(|fee| *fee > 0)
        .ok_or(Reason::BadRefundShapeOrMaturity)?;
    Ok(())
}

fn check_tx_header(tx: &TxView) -> Result<(), Reason> {
    if tx.version == 2 && tx.lock_time == 0 {
        Ok(())
    } else {
        Err(Reason::BadTxVersionOrLocktime)
    }
}

fn check_output_shape(tx: &TxView, marker: &ParsedMarker, count: usize) -> Result<(), Reason> {
    if tx.outputs.len() == count && marker.vout == 0 && !has_other_op_return(tx, 0) {
        Ok(())
    } else {
        Err(Reason::BadOutputCountOrOrder)
    }
}

fn check_provenance(tx: &TxView, block_height: u64) -> Result<(), Reason> {
    if tx
        .inputs
        .iter()
        .all(|input| input.has_earlier_confirmation(block_height))
    {
        Ok(())
    } else {
        Err(Reason::UnconfirmedOrSameBlockPrevout)
    }
}

fn check_transition_input_scripts(
    tx: &TxView,
    carrier_indexes: &[usize],
    p2wpkh_indexes: &[usize],
) -> Result<(), Reason> {
    if carrier_indexes
        .iter()
        .any(|index| tx.inputs[*index].carrier_revealed_keys().is_none())
        || p2wpkh_indexes
            .iter()
            .any(|index| tx.inputs[*index].p2wpkh_revealed_key().is_none())
    {
        Err(Reason::BadInputScript)
    } else {
        Ok(())
    }
}

fn check_signatures(tx: &TxView) -> Result<(), Reason> {
    if tx.inputs.iter().all(|input| input.signatures_valid) {
        Ok(())
    } else {
        Err(Reason::BadSignatureOrSighash)
    }
}

#[allow(clippy::too_many_lines)]
fn apply_valid_operation(
    state: &mut ChainState,
    block: &BlockView,
    tx: &TxView,
    tx_index: u32,
    event_index: u32,
    operation: ValidatedOperation,
) -> Event {
    let namespace = state.binding.namespace();
    match operation {
        ValidatedOperation::Create { keys } => {
            let key = object_key(namespace, tx.txid);
            let successor = OutPointRef {
                txid: tx.txid,
                vout: 1,
            };
            let ProtocolStatus::Active {
                h_open, h_close, ..
            } = state.protocol_status
            else {
                unreachable!("CREATE validation requires active protocol");
            };
            let founding = block.height >= u64::from(h_open) && block.height < u64::from(h_close);
            let object = ObjectState {
                object_key: key,
                genesis: successor,
                founding,
                status: ObjectStatus::Active,
                create_height: block.height,
                state_seq: 0,
                current_outpoint: successor,
                keys,
                terminal_txid: Hash32::ZERO,
                chapter_count: 0,
            };
            state.active.insert(successor, key);
            state.objects.insert(key, object);
            valid_event_base(
                state,
                block,
                tx,
                tx_index,
                event_index,
                EventType::Create,
                key,
                0,
                OutPointRef::ZERO,
                successor,
                keys,
                Hash32::ZERO,
            )
        }
        ValidatedOperation::Mark {
            mut object,
            sequence,
            commitment,
        } => {
            let predecessor = object.current_outpoint;
            let successor = OutPointRef {
                txid: tx.txid,
                vout: 1,
            };
            state.active.remove(&predecessor);
            object.current_outpoint = successor;
            object.state_seq = sequence;
            object.chapter_count = object
                .chapter_count
                .checked_add(1)
                .expect("chapter count bounded by state sequence");
            state.active.insert(successor, object.object_key);
            state.objects.insert(object.object_key, object.clone());
            valid_event_base(
                state,
                block,
                tx,
                tx_index,
                event_index,
                EventType::Mark,
                object.object_key,
                sequence,
                predecessor,
                successor,
                object.keys,
                commitment,
            )
        }
        ValidatedOperation::Rotate {
            mut object,
            sequence,
            keys,
        } => {
            let predecessor = object.current_outpoint;
            let successor = OutPointRef {
                txid: tx.txid,
                vout: 1,
            };
            state.active.remove(&predecessor);
            object.current_outpoint = successor;
            object.state_seq = sequence;
            object.keys = keys;
            state.active.insert(successor, object.object_key);
            state.objects.insert(object.object_key, object.clone());
            valid_event_base(
                state,
                block,
                tx,
                tx_index,
                event_index,
                EventType::Rotate,
                object.object_key,
                sequence,
                predecessor,
                successor,
                keys,
                Hash32::ZERO,
            )
        }
        ValidatedOperation::Close {
            object,
            sequence,
            commitment,
        } => {
            let predecessor = object.current_outpoint;
            terminate_object(state, &object, ObjectStatus::Closed, tx.txid);
            if let Some(stored) = state.objects.get_mut(&object.object_key) {
                stored.state_seq = sequence;
            }
            valid_event_base(
                state,
                block,
                tx,
                tx_index,
                event_index,
                EventType::Close,
                object.object_key,
                sequence,
                predecessor,
                OutPointRef::ZERO,
                object.keys,
                commitment,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn valid_event_base(
    state: &ChainState,
    block: &BlockView,
    tx: &TxView,
    tx_index: u32,
    event_index: u32,
    event_type: EventType,
    object_key: Hash32,
    state_seq: u32,
    predecessor: OutPointRef,
    successor: OutPointRef,
    keys: KeyPair,
    commitment: Hash32,
) -> Event {
    Event {
        namespace: state.binding.namespace(),
        block_hash: block.hash,
        height: block.height,
        tx_index,
        event_index,
        sub_index: 0,
        event_type,
        validity_class: ValidityClass::ValidOperation,
        reason: Reason::Valid,
        txid: tx.txid,
        wtxid: tx.wtxid,
        object_key,
        state_seq,
        predecessor,
        successor,
        key0: keys.key0,
        key1: keys.key1,
        commitment,
    }
}

fn valid_init_event(state: &ChainState, block: &BlockView, tx: &TxView, tx_index: u32) -> Event {
    let event_index = marker_candidates(tx, &state.binding)
        .first()
        .map_or(u32::MAX, |candidate| candidate.vout);
    Event {
        namespace: state.binding.namespace(),
        block_hash: block.hash,
        height: block.height,
        tx_index,
        event_index,
        sub_index: 0,
        event_type: EventType::Init,
        validity_class: ValidityClass::ValidOperation,
        reason: Reason::Valid,
        txid: tx.txid,
        wtxid: tx.wtxid,
        object_key: Hash32::ZERO,
        state_seq: u32::MAX,
        predecessor: OutPointRef::ZERO,
        successor: OutPointRef::ZERO,
        key0: Key33::ZERO,
        key1: Key33::ZERO,
        commitment: state.binding.spec_hash,
    }
}

fn valid_refund_event(
    state: &ChainState,
    block: &BlockView,
    tx: &TxView,
    tx_index: u32,
    object: &ObjectState,
) -> Event {
    valid_event_base(
        state,
        block,
        tx,
        tx_index,
        u32::MAX,
        EventType::Refund,
        object.object_key,
        object.state_seq,
        object.current_outpoint,
        OutPointRef::ZERO,
        object.keys,
        Hash32::ZERO,
    )
}

fn invalid_event(
    binding: &Binding,
    block: &BlockView,
    tx: &TxView,
    tx_index: u32,
    reason: Reason,
    configured_init: bool,
) -> Event {
    let candidates = marker_candidates(tx, binding);
    let event_index = match candidates.len().cmp(&1) {
        std::cmp::Ordering::Equal => candidates[0].vout,
        std::cmp::Ordering::Greater => candidates
            .iter()
            .map(|candidate| candidate.vout)
            .min()
            .unwrap_or(u32::MAX),
        std::cmp::Ordering::Less => u32::MAX,
    };
    let event_type = if candidates.len() == 1 {
        attempted_event_type(&candidates[0])
    } else {
        EventType::Invalid
    };
    let namespace = if configured_init {
        binding.namespace()
    } else if candidates.len() == 1 {
        population_namespace(&candidates[0]).unwrap_or(Hash32::ZERO)
    } else {
        Hash32::ZERO
    };
    bare_invalid_event(
        block,
        tx,
        tx_index,
        event_index,
        event_type,
        reason,
        namespace,
    )
}

#[allow(clippy::too_many_arguments)]
fn bare_invalid_event(
    block: &BlockView,
    tx: &TxView,
    tx_index: u32,
    event_index: u32,
    event_type: EventType,
    reason: Reason,
    namespace: Hash32,
) -> Event {
    Event {
        namespace,
        block_hash: block.hash,
        height: block.height,
        tx_index,
        event_index,
        sub_index: 0,
        event_type,
        validity_class: ValidityClass::InvalidNoState,
        reason,
        txid: tx.txid,
        wtxid: tx.wtxid,
        object_key: Hash32::ZERO,
        state_seq: u32::MAX,
        predecessor: OutPointRef::ZERO,
        successor: OutPointRef::ZERO,
        key0: Key33::ZERO,
        key1: Key33::ZERO,
        commitment: Hash32::ZERO,
    }
}

#[allow(clippy::too_many_arguments)]
fn terminate_carriers(
    state: &mut ChainState,
    block: &BlockView,
    tx: &TxView,
    tx_index: u32,
    event_index: u32,
    reason: Reason,
    namespace: Hash32,
    mut carriers: Vec<(usize, ObjectState)>,
) -> Vec<Event> {
    carriers.sort_by_key(|(_, object)| object.object_key);
    carriers
        .into_iter()
        .enumerate()
        .map(|(sub_index, (_, object))| {
            let event = Event {
                namespace,
                block_hash: block.hash,
                height: block.height,
                tx_index,
                event_index,
                sub_index: u32::try_from(sub_index)
                    .expect("input count is bounded by transaction size"),
                event_type: EventType::ExitedNoncanonical,
                validity_class: ValidityClass::TerminalNoncanonical,
                reason,
                txid: tx.txid,
                wtxid: tx.wtxid,
                object_key: object.object_key,
                state_seq: object.state_seq,
                predecessor: object.current_outpoint,
                successor: OutPointRef::ZERO,
                key0: object.keys.key0,
                key1: object.keys.key1,
                commitment: Hash32::ZERO,
            };
            terminate_object(state, &object, ObjectStatus::ExitedNoncanonical, tx.txid);
            event
        })
        .collect()
}

fn terminate_object(
    state: &mut ChainState,
    object: &ObjectState,
    status: ObjectStatus,
    terminal_txid: Hash32,
) {
    state.active.remove(&object.current_outpoint);
    if let Some(stored) = state.objects.get_mut(&object.object_key) {
        stored.status = status;
        stored.current_outpoint = OutPointRef::ZERO;
        stored.terminal_txid = terminal_txid;
    }
}

fn attempted_event_type(candidate: &MarkerCandidate) -> EventType {
    if candidate.payload.get(4) == Some(&VERSION) {
        candidate
            .opcode_byte()
            .and_then(Opcode::from_byte)
            .map_or(EventType::Invalid, EventType::from)
    } else {
        EventType::Invalid
    }
}

fn population_namespace(candidate: &MarkerCandidate) -> Option<Hash32> {
    if !candidate.payload_complete || candidate.payload.len() < 40 {
        return None;
    }
    let opcode = candidate.opcode_byte().and_then(Opcode::from_byte)?;
    if candidate.payload[4] != VERSION
        || opcode == Opcode::Init
        || candidate.payload.len() != opcode.payload_len()
    {
        return None;
    }
    Some(Hash32(candidate.payload[8..40].try_into().ok()?))
}

fn has_other_op_return(tx: &TxView, marker_vout: u32) -> bool {
    tx.outputs.iter().enumerate().any(|(index, output)| {
        u32::try_from(index) != Ok(marker_vout) && is_op_return(&output.script_pubkey)
    })
}

fn is_op_return(script: &[u8]) -> bool {
    script.first() == Some(&0x6a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InputView, MAGIC, Network, OutputView};

    fn binding() -> Binding {
        Binding {
            network: Network::Regtest,
            init_txid: Hash32([1; 32]),
            spec_hash: Hash32([2; 32]),
        }
    }

    fn key_pair() -> KeyPair {
        KeyPair::checked(
            Key33::from_hex("0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .expect("key0"),
            Key33::from_hex("02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5")
                .expect("key1"),
        )
        .expect("pair")
    }

    fn marker_script(payload: &[u8]) -> Vec<u8> {
        let mut script = vec![0x6a];
        if payload.len() <= 75 {
            script.push(u8::try_from(payload.len()).expect("direct push"));
        } else {
            script.extend_from_slice(&[0x4c, u8::try_from(payload.len()).expect("pushdata1")]);
        }
        script.extend_from_slice(payload);
        script
    }

    fn p2wpkh_input(
        key: Key33,
        txid_byte: u8,
        height: u64,
        value: u64,
        sequence: u32,
    ) -> InputView {
        InputView {
            prevout: OutPointRef {
                txid: Hash32([txid_byte; 32]),
                vout: 0,
            },
            sequence,
            script_sig: Vec::new(),
            witness: vec![vec![0x30, 0x01], key.0.to_vec()],
            prevout_value: value,
            prevout_script: key.p2wpkh_script(),
            prevout_height: Some(height),
            signatures_valid: true,
        }
    }

    fn carrier_input(keys: KeyPair, prevout: OutPointRef, height: u64, sequence: u32) -> InputView {
        InputView {
            prevout,
            sequence,
            script_sig: Vec::new(),
            witness: vec![
                Vec::new(),
                vec![0x30, 0x01],
                vec![0x30, 0x01],
                keys.witness_script(),
            ],
            prevout_value: CARRIER_VALUE,
            prevout_script: keys.carrier_script(),
            prevout_height: Some(height),
            signatures_valid: true,
        }
    }

    fn height_hash(height: u64) -> Hash32 {
        let mut bytes = [0_u8; 32];
        bytes[..8].copy_from_slice(&height.to_le_bytes());
        Hash32(bytes)
    }

    fn init_transaction(binding: &Binding, key: Key33) -> TxView {
        let mut payload = Vec::with_capacity(59);
        payload.extend_from_slice(&MAGIC);
        payload.extend_from_slice(&[VERSION, Network::Regtest.code(), Opcode::Init as u8]);
        payload.extend_from_slice(&1_108_u32.to_le_bytes());
        payload.extend_from_slice(&5_428_u32.to_le_bytes());
        payload.extend_from_slice(&CARRIER_VALUE.to_le_bytes());
        payload.extend_from_slice(&REFUND_DELAY.to_le_bytes());
        payload.extend_from_slice(&binding.spec_hash.0);
        TxView {
            txid: binding.init_txid,
            wtxid: Hash32([9; 32]),
            version: 2,
            lock_time: 0,
            inputs: vec![p2wpkh_input(key, 8, 99, 2_000, u32::MAX)],
            outputs: vec![
                OutputView {
                    value: 0,
                    script_pubkey: marker_script(&payload),
                },
                OutputView {
                    value: 1_000,
                    script_pubkey: key.p2wpkh_script(),
                },
            ],
        }
    }

    fn create_transaction(binding: &Binding, keys: KeyPair) -> TxView {
        let mut payload = Vec::with_capacity(40);
        payload.extend_from_slice(&MAGIC);
        payload.extend_from_slice(&[VERSION, Network::Regtest.code(), Opcode::Create as u8, 1]);
        payload.extend_from_slice(&binding.namespace().0);
        TxView {
            txid: Hash32([20; 32]),
            wtxid: Hash32([21; 32]),
            version: 2,
            lock_time: 0,
            inputs: vec![
                p2wpkh_input(keys.key0, 12, 1_000, 20_000, RBF_SEQUENCE),
                p2wpkh_input(keys.key1, 13, 1_000, 20_000, RBF_SEQUENCE),
            ],
            outputs: vec![
                OutputView {
                    value: 0,
                    script_pubkey: marker_script(&payload),
                },
                OutputView {
                    value: CARRIER_VALUE,
                    script_pubkey: keys.carrier_script(),
                },
                OutputView {
                    value: 8_485,
                    script_pubkey: keys.key0.p2wpkh_script(),
                },
                OutputView {
                    value: 8_485,
                    script_pubkey: keys.key1.p2wpkh_script(),
                },
            ],
        }
    }

    fn mark_transaction(
        binding: &Binding,
        keys: KeyPair,
        predecessor: OutPointRef,
        predecessor_height: u64,
        txid_byte: u8,
        sequence: u32,
        commitment: Hash32,
    ) -> TxView {
        let mut payload = Vec::with_capacity(78);
        payload.extend_from_slice(&MAGIC);
        payload.extend_from_slice(&[VERSION, Network::Regtest.code(), Opcode::Mark as u8, 1]);
        payload.extend_from_slice(&binding.namespace().0);
        payload.extend_from_slice(&sequence.to_le_bytes());
        payload.extend_from_slice(&[0, 0]);
        payload.extend_from_slice(&commitment.0);
        TxView {
            txid: Hash32([txid_byte; 32]),
            wtxid: Hash32([txid_byte.wrapping_add(1); 32]),
            version: 2,
            lock_time: 0,
            inputs: vec![
                carrier_input(keys, predecessor, predecessor_height, RBF_SEQUENCE),
                p2wpkh_input(
                    keys.key0,
                    txid_byte.wrapping_add(20),
                    1_000,
                    5_000,
                    RBF_SEQUENCE,
                ),
            ],
            outputs: vec![
                OutputView {
                    value: 0,
                    script_pubkey: marker_script(&payload),
                },
                OutputView {
                    value: CARRIER_VALUE,
                    script_pubkey: keys.carrier_script(),
                },
                OutputView {
                    value: 4_000,
                    script_pubkey: keys.key0.p2wpkh_script(),
                },
            ],
        }
    }

    fn close_transaction(binding: &Binding, keys: KeyPair, predecessor: OutPointRef) -> TxView {
        let mut payload = Vec::with_capacity(80);
        payload.extend_from_slice(&MAGIC);
        payload.extend_from_slice(&[VERSION, Network::Regtest.code(), Opcode::Close as u8, 0xff]);
        payload.extend_from_slice(&binding.namespace().0);
        payload.extend_from_slice(&2_u32.to_le_bytes());
        payload.extend_from_slice(&[0, 0, 0, 0]);
        payload.extend_from_slice(&[44; 32]);
        TxView {
            txid: Hash32([40; 32]),
            wtxid: Hash32([41; 32]),
            version: 2,
            lock_time: 0,
            inputs: vec![carrier_input(keys, predecessor, 1_109, RBF_SEQUENCE)],
            outputs: vec![
                OutputView {
                    value: 0,
                    script_pubkey: marker_script(&payload),
                },
                OutputView {
                    value: 9_500,
                    script_pubkey: keys.key0.p2wpkh_script(),
                },
                OutputView {
                    value: 9_500,
                    script_pubkey: keys.key1.p2wpkh_script(),
                },
            ],
        }
    }

    #[test]
    fn state_fails_closed_without_configured_init() {
        let mut state = ChainState::new(binding());
        let block = BlockView {
            hash: Hash32([3; 32]),
            previous_hash: Hash32::ZERO,
            height: 100,
            transactions: Vec::new(),
        };
        assert_eq!(
            apply_block(&mut state, &block).expect_err("must fail"),
            ReducerError::ConfiguredInitAbsent
        );
        assert_eq!(state.protocol_status, ProtocolStatus::AwaitingInit);
    }

    #[test]
    fn unknown_transactions_do_not_emit_events() {
        let binding = binding();
        let mut state = ChainState::new(binding.clone());
        state.protocol_status = ProtocolStatus::Active {
            init_height: 1,
            h_open: 1_009,
            h_close: 5_329,
        };
        state.tip_height = Some(1);
        state.tip_hash = Some(Hash32([4; 32]));
        let block = BlockView {
            hash: Hash32([5; 32]),
            previous_hash: Hash32([4; 32]),
            height: 2,
            transactions: vec![TxView {
                txid: Hash32([6; 32]),
                wtxid: Hash32([6; 32]),
                version: 2,
                lock_time: 0,
                inputs: Vec::new(),
                outputs: vec![OutputView {
                    value: 0,
                    script_pubkey: vec![0x6a, 0x01, 0x00],
                }],
            }],
        };
        let delta = apply_block(&mut state, &block).expect("apply");
        assert!(delta.events.is_empty());
        disconnect_block(&mut state, &delta).expect("disconnect");
        assert_eq!(state.tip_height, Some(1));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn full_lifecycle_reduces_terminates_and_reverses() {
        let binding = binding();
        let keys = key_pair();
        let mut state = ChainState::new(binding.clone());
        let init_block = BlockView {
            hash: height_hash(100),
            previous_hash: height_hash(99),
            height: 100,
            transactions: vec![init_transaction(&binding, keys.key0)],
        };
        let init_delta = apply_block(&mut state, &init_block).expect("INIT");
        assert_eq!(init_delta.events[0].event_type, EventType::Init);
        for height in 101..1_108 {
            let empty = BlockView {
                hash: height_hash(height),
                previous_hash: height_hash(height - 1),
                height,
                transactions: Vec::new(),
            };
            assert!(
                apply_block(&mut state, &empty)
                    .expect("empty block")
                    .events
                    .is_empty()
            );
        }

        let create_block = BlockView {
            hash: height_hash(1_108),
            previous_hash: height_hash(1_107),
            height: 1_108,
            transactions: vec![create_transaction(&binding, keys)],
        };
        let create_delta = apply_block(&mut state, &create_block).expect("CREATE");
        assert_eq!(create_delta.events[0].event_type, EventType::Create);
        assert_eq!(state.counters().founding_created, 1);
        let created = OutPointRef {
            txid: Hash32([20; 32]),
            vout: 1,
        };

        let mark_block = BlockView {
            hash: height_hash(1_109),
            previous_hash: height_hash(1_108),
            height: 1_109,
            transactions: vec![mark_transaction(
                &binding,
                keys,
                created,
                1_108,
                30,
                1,
                Hash32([42; 32]),
            )],
        };
        let mark_delta = apply_block(&mut state, &mark_block).expect("MARK");
        assert_eq!(mark_delta.events[0].event_type, EventType::Mark);
        let object = state.objects.values().next().expect("object");
        assert_eq!(object.state_seq, 1);
        assert_eq!(object.chapter_count, 1);
        let marked = object.current_outpoint;

        let refund = TxView {
            txid: Hash32([50; 32]),
            wtxid: Hash32([51; 32]),
            version: 2,
            lock_time: 0,
            inputs: vec![carrier_input(keys, marked, 1_109, REFUND_DELAY)],
            outputs: vec![
                OutputView {
                    value: 9_830,
                    script_pubkey: keys.key0.p2wpkh_script(),
                },
                OutputView {
                    value: 9_830,
                    script_pubkey: keys.key1.p2wpkh_script(),
                },
            ],
        };
        assert!(validate_refund(53_669, &refund, object).is_ok());
        assert_eq!(
            validate_refund(53_668, &refund, object),
            Err(Reason::BadRefundShapeOrMaturity)
        );

        let bad_mark_block = BlockView {
            hash: height_hash(1_110),
            previous_hash: height_hash(1_109),
            height: 1_110,
            transactions: vec![mark_transaction(
                &binding,
                keys,
                marked,
                1_109,
                32,
                2,
                Hash32::ZERO,
            )],
        };
        let bad_delta = apply_block(&mut state, &bad_mark_block).expect("invalid MARK spend");
        assert_eq!(bad_delta.events[0].reason, Reason::BadCommitment);
        assert_eq!(
            bad_delta.events[0].validity_class,
            ValidityClass::TerminalNoncanonical
        );
        disconnect_block(&mut state, &bad_delta).expect("disconnect invalid MARK");

        let close_block = BlockView {
            hash: height_hash(1_110),
            previous_hash: height_hash(1_109),
            height: 1_110,
            transactions: vec![close_transaction(&binding, keys, marked)],
        };
        let close_delta = apply_block(&mut state, &close_block).expect("CLOSE");
        assert_eq!(close_delta.events[0].event_type, EventType::Close);
        assert_eq!(state.counters().active_objects, 0);
        assert_eq!(
            state.objects.values().next().expect("object").status,
            ObjectStatus::Closed
        );

        disconnect_block(&mut state, &close_delta).expect("disconnect CLOSE");
        disconnect_block(&mut state, &mark_delta).expect("disconnect MARK");
        disconnect_block(&mut state, &create_delta).expect("disconnect CREATE");
        assert!(state.objects.is_empty());
        assert_eq!(state.tip_height, Some(1_107));
    }
}
