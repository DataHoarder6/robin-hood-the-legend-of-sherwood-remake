//! Surface materials that select footstep and impact sounds.

use serde::{Deserialize, Serialize};

#[repr(u32)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Material {
    Ground = 0,
    Wood = 1,
    Stone = 2,
    Grass = 3,
    Leaves = 4,
    Water = 5,
    Bush = 6,
    Ice = 7,
    Hole = 8,
}

impl Material {
    pub const NUM_MATERIALS: usize = 9;
}
