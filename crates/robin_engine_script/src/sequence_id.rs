//! Sequence identity, named by the VM when a native launches a sequence.
//! `robin_engine::sequence` re-exports it beside the sequence manager.

use serde::{Deserialize, Serialize};

/// Unique identifier for a sequence (`robin_engine::sequence::Sequence`).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SequenceId(pub u32);
