#![no_main]

use libfuzzer_sys::fuzz_target;
use tandem_core::{Binding, BlockView, ChainState, Hash32, Network, apply_block};

fuzz_target!(|data: &[u8]| {
    let Ok(blocks) = serde_json::from_slice::<Vec<BlockView>>(data) else {
        return;
    };
    let binding = Binding {
        network: Network::Regtest,
        init_txid: Hash32([1; 32]),
        spec_hash: Hash32([2; 32]),
    };
    let mut state = ChainState::new(binding);
    for block in blocks.iter().take(32) {
        if apply_block(&mut state, block).is_err() {
            break;
        }
    }
});

