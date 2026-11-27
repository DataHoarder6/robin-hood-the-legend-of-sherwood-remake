//! Production-sector kinds carried by geometry sectors.

use serde::{Deserialize, Serialize};

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
    num_enum::TryFromPrimitive,
)]
#[repr(i32)]
pub enum Type {
    MakeArrow,
    MakePurse,
    MakeStone,
    MakeApple,
    MakeAle,
    MakeLamblegg,
    MakePlant,
    MakeNet,
    MakeWaspNest,
    TrainBow,
    TrainHandToHand,
    Heal,
    Relic,
    #[default]
    Unknown,
}

impl Type {
    /// Map the production type to the raw `BonusType` ordinal used by
    /// mission-file bonus entries (and by `bonus_type_to_sprite_asset`).
    /// Returns `None` for non-MAKE_* types.
    pub fn bonus_raw_type(self) -> Option<u16> {
        match self {
            Type::MakeArrow => Some(0),
            Type::MakeStone => Some(1),
            Type::MakeApple => Some(2),
            Type::MakeAle => Some(3),
            Type::MakeLamblegg => Some(4),
            Type::MakePlant => Some(5),
            Type::MakeNet => Some(6),
            Type::MakeWaspNest => Some(7),
            Type::MakePurse => Some(8),
            _ => None,
        }
    }

    pub fn from_script_i32(value: i32) -> Option<Self> {
        Self::try_from(value)
            .ok()
            .filter(|kind| *kind != Self::Unknown)
    }
}
