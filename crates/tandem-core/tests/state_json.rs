//! Regression: chain state holding an active carrier must round-trip through JSON, which is how
//! the verifier persists it in `PostgreSQL` `JSONB`. A struct-keyed map previously failed with
//! "key must be a string" on the first block that created an object.

use tandem_core::{Binding, ChainState, Hash32, Network, OutPointRef};

#[test]
fn chain_state_with_active_carrier_round_trips_through_json() {
    let mut state = ChainState::new(Binding {
        network: Network::Signet,
        init_txid: Hash32([0x11; 32]),
        spec_hash: Hash32([0x22; 32]),
    });
    state.active.insert(
        OutPointRef {
            txid: Hash32([0x33; 32]),
            vout: 1,
        },
        Hash32([0x44; 32]),
    );
    let value = serde_json::to_value(&state).expect("chain state serializes to JSON");
    assert!(value["active"].is_array());
    let restored: ChainState = serde_json::from_value(value).expect("chain state deserializes");
    assert_eq!(restored, state);
}

#[test]
fn duplicate_active_outpoints_are_rejected() {
    let state = ChainState::new(Binding {
        network: Network::Signet,
        init_txid: Hash32([0x11; 32]),
        spec_hash: Hash32([0x22; 32]),
    });
    let mut value = serde_json::to_value(&state).expect("serializes");
    let pair = serde_json::json!([{ "txid": "33".repeat(32), "vout": 1 }, "44".repeat(32)]);
    value["active"] = serde_json::json!([pair.clone(), pair]);
    assert!(serde_json::from_value::<ChainState>(value).is_err());
}
