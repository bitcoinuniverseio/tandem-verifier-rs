//! Property coverage for malformed marker bytes and hash domain separation.

use proptest::prelude::*;
use tandem_core::{Hash32, Opcode, find_marker_candidate};

proptest! {
    #[test]
    fn marker_detection_and_parse_never_panics(script in proptest::collection::vec(any::<u8>(), 0..512)) {
        if let Some(candidate) = find_marker_candidate(&script, 0)
            && let Ok(marker) = candidate.parse() {
            prop_assert!((7..=80).contains(&marker.payload.len()));
            if let Some(opcode) = marker.opcode {
                prop_assert_eq!(marker.payload.len(), opcode.payload_len());
            }
        }
    }

    #[test]
    fn object_keys_are_domain_separated(
        namespace in any::<[u8; 32]>(),
        first in any::<[u8; 32]>(),
        second in any::<[u8; 32]>(),
    ) {
        prop_assume!(first != second);
        let first_key = tandem_core::object_key(Hash32(namespace), Hash32(first));
        let second_key = tandem_core::object_key(Hash32(namespace), Hash32(second));
        prop_assert_ne!(first_key, second_key);
    }
}

#[test]
fn exact_payload_lengths_match_the_specification() {
    assert_eq!(Opcode::Init.payload_len(), 59);
    assert_eq!(Opcode::Create.payload_len(), 40);
    assert_eq!(Opcode::Mark.payload_len(), 78);
    assert_eq!(Opcode::Rotate.payload_len(), 44);
    assert_eq!(Opcode::Close.payload_len(), 80);
}
