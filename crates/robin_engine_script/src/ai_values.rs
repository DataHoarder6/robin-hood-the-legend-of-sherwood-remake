//! AI values that script requests and native commands name: the alert level,
//! noise types and the sector-qualified waypoint position. `robin_engine::ai`
//! re-exports them.

use serde::{Deserialize, Serialize};

use crate::position_interface::SectorHandle;

/// Noise type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum NoiseType {
    Plouf = 0,
    Bonk,
    Zonk,
    TapTapTap,
    ArfArf,
    Tirili,
    PutPut,
    Aaargh,
    Heeelp,
    Pling,
    Pfiiit,
    Logs,
    Drawbridge,
    ZingZing,
    Off,
    /// An intentionally thrown object impact. Appended after every Original
    /// ordinal so legacy enum values remain stable.
    Distraction,
}

/// Global alert level.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum AlertLevel {
    #[default]
    Green = 0,
    Yellow,
    Red,
}

/// 2-D game position with sector and level info.
///
/// Sector is currently an opaque handle; once the sector system is fully
/// integrated this will reference it properly.
#[derive(
    Default,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    /// Sector handle (nominal newtype from `position_interface`).
    /// `None` indicates a null sector / unassigned waypoint.
    pub sector: Option<SectorHandle>,
    pub level: u16,
}

impl Position {
    /// The projected map point `(x, y)`; sector and level are dropped.
    /// `a.map_point() - b.map_point()` is the `MapVec` `(a.x - b.x, a.y - b.y)`.
    #[inline]
    pub fn map_point(&self) -> crate::coordinates::MapPoint {
        crate::coordinates::MapPoint::new(self.x, self.y)
    }
}
