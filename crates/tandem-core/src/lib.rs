//! Independent Tandem v1 consensus parser, reducer, and root calculator.
//!
//! This crate has no database, HTTP, or TypeScript dependency. Callers supply
//! resolved Bitcoin transaction inputs, including exact prevout data and the
//! result of independent signature verification.

pub mod marker;
pub mod reducer;
pub mod roots;
pub mod types;

pub use marker::{MarkerCandidate, MarkerError, ParsedMarker, find_marker_candidate};
pub use reducer::{BlockDelta, ReducerError, apply_block, disconnect_block};
pub use roots::{block_root, event_leaf, event_root, object_state_root};
pub use types::*;
