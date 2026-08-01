//! Canonical Tandem v1 event, object, and chained root calculations.

use crate::{Counters, Event, Hash32, ObjectState, ObjectStatus};

/// Compute the exact event leaf digest.
pub fn event_leaf(event: &Event) -> Hash32 {
    let mut preimage = Vec::with_capacity(374);
    preimage.extend_from_slice(b"TANDEM/EVENT/V1\0");
    preimage.extend_from_slice(&event.namespace.0);
    preimage.extend_from_slice(&event.block_hash.0);
    preimage.extend_from_slice(&event.height.to_le_bytes());
    preimage.extend_from_slice(&event.tx_index.to_le_bytes());
    preimage.extend_from_slice(&event.event_index.to_le_bytes());
    preimage.extend_from_slice(&event.sub_index.to_le_bytes());
    preimage.push(event.event_type as u8);
    preimage.push(event.validity_class as u8);
    preimage.extend_from_slice(&(event.reason as u16).to_le_bytes());
    preimage.extend_from_slice(&event.txid.0);
    preimage.extend_from_slice(&event.wtxid.0);
    preimage.extend_from_slice(&event.object_key.0);
    preimage.extend_from_slice(&event.state_seq.to_le_bytes());
    preimage.extend_from_slice(&event.predecessor.wire_bytes());
    preimage.extend_from_slice(&event.successor.wire_bytes());
    preimage.extend_from_slice(&event.key0.0);
    preimage.extend_from_slice(&event.key1.0);
    preimage.extend_from_slice(&event.commitment.0);
    debug_assert_eq!(preimage.len(), 374);
    Hash32::sha256(preimage)
}

/// Compute the ordered event Merkle root.
pub fn event_root(namespace: Hash32, events: &[Event]) -> Hash32 {
    if events.is_empty() {
        let mut preimage = Vec::with_capacity(54);
        preimage.extend_from_slice(b"TANDEM/EVENT-EMPTY/V1\0");
        preimage.extend_from_slice(&namespace.0);
        return Hash32::sha256(preimage);
    }
    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|event| (event.tx_index, event.event_index, event.sub_index));
    merkle(
        ordered.into_iter().map(event_leaf).collect(),
        b"TANDEM/EVENT-NODE/V1\0",
    )
}

/// Compute one canonical object-state snapshot leaf.
pub fn object_state_leaf(object: &ObjectState) -> Hash32 {
    let mut preimage = Vec::with_capacity(232);
    preimage.extend_from_slice(b"TANDEM/OBJECT-STATE/V1\0");
    preimage.extend_from_slice(&object.object_key.0);
    preimage.push(u8::from(object.founding));
    preimage.push(object.status as u8);
    preimage.extend_from_slice(&object.create_height.to_le_bytes());
    preimage.extend_from_slice(&object.state_seq.to_le_bytes());
    let active_outpoint = if object.status == ObjectStatus::Active {
        object.current_outpoint
    } else {
        crate::OutPointRef::ZERO
    };
    preimage.extend_from_slice(&active_outpoint.wire_bytes());
    preimage.extend_from_slice(&object.keys.key0.0);
    preimage.extend_from_slice(&object.keys.key1.0);
    let terminal_txid = if object.status == ObjectStatus::Active {
        Hash32::ZERO
    } else {
        object.terminal_txid
    };
    preimage.extend_from_slice(&terminal_txid.0);
    preimage.extend_from_slice(&object.chapter_count.to_le_bytes());
    Hash32::sha256(preimage)
}

/// Compute the object-state Merkle root from objects ordered by binary key.
pub fn object_state_root<'a>(
    namespace: Hash32,
    objects: impl IntoIterator<Item = &'a ObjectState>,
) -> Hash32 {
    let mut leaves = objects
        .into_iter()
        .map(|object| (object.object_key, object_state_leaf(object)))
        .collect::<Vec<_>>();
    if leaves.is_empty() {
        let mut preimage = Vec::with_capacity(55);
        preimage.extend_from_slice(b"TANDEM/OBJECT-EMPTY/V1\0");
        preimage.extend_from_slice(&namespace.0);
        return Hash32::sha256(preimage);
    }
    leaves.sort_by_key(|(key, _)| *key);
    merkle(
        leaves.into_iter().map(|(_, leaf)| leaf).collect(),
        b"TANDEM/OBJECT-NODE/V1\0",
    )
}

/// Compute the chained block root.
#[allow(clippy::too_many_arguments)]
pub fn block_root(
    namespace: Hash32,
    previous_root: Hash32,
    block_hash: Hash32,
    height: u64,
    event_root: Hash32,
    object_state_root: Hash32,
    counters: Counters,
) -> Hash32 {
    let mut preimage = Vec::with_capacity(228);
    preimage.extend_from_slice(b"TANDEM/BLOCKROOT/V1\0");
    preimage.extend_from_slice(&namespace.0);
    preimage.extend_from_slice(&previous_root.0);
    preimage.extend_from_slice(&block_hash.0);
    preimage.extend_from_slice(&height.to_le_bytes());
    preimage.extend_from_slice(&event_root.0);
    preimage.extend_from_slice(&object_state_root.0);
    preimage.extend_from_slice(&counters.founding_created.to_le_bytes());
    preimage.extend_from_slice(&counters.all_objects.to_le_bytes());
    preimage.extend_from_slice(&counters.active_objects.to_le_bytes());
    Hash32::sha256(preimage)
}

fn merkle(mut leaves: Vec<Hash32>, domain: &[u8]) -> Hash32 {
    debug_assert!(!leaves.is_empty());
    while leaves.len() > 1 {
        if leaves.len() % 2 == 1 {
            leaves.push(*leaves.last().expect("nonempty"));
        }
        leaves = leaves
            .chunks_exact(2)
            .map(|pair| {
                let mut preimage = Vec::with_capacity(domain.len() + 64);
                preimage.extend_from_slice(domain);
                preimage.extend_from_slice(&pair[0].0);
                preimage.extend_from_slice(&pair[1].0);
                Hash32::sha256(preimage)
            })
            .collect();
    }
    leaves[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_leaf_is_not_rehashed() {
        let leaf = Hash32([7; 32]);
        assert_eq!(merkle(vec![leaf], b"unused"), leaf);
    }

    #[test]
    fn odd_leaf_is_duplicated() {
        let leaves = vec![Hash32([1; 32]), Hash32([2; 32]), Hash32([3; 32])];
        assert_ne!(merkle(leaves, b"domain"), Hash32::ZERO);
    }
}
