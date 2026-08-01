#![no_main]

use libfuzzer_sys::fuzz_target;
use tandem_core::find_marker_candidate;

fuzz_target!(|data: &[u8]| {
    if let Some(candidate) = find_marker_candidate(data, 0) {
        let _ = candidate.parse();
    }
});

