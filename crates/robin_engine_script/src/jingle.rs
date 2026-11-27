//! Jingle identifiers a native command can play. `robin_engine::sound`
//! re-exports them.

/// Jingle type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Jingle {
    NewPeasantCalled = 0,
    MissionWon = 1,
    MissionLost = 2,
    CashWon = 3,
    QuickActionSucceeded = 4,
    QuickActionFailed = 5,
    TrapTriggered = 6,
    PcInComa = 7,
}
