//! Row indices of the titbit sprite sheet, shared by asset preparation and the
//! titbit system. `robin_engine::titbit` re-exports the type.

use serde::{Deserialize, Serialize};

/// Sprite row indices for the titbit sprite sheet.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u16)]
pub enum SpriteRow {
    Impact = 0,
    OneStar,
    TwoStars,
    ThreeStars,
    FourStars,
    FiveStars,
    QuickActionTitbits,
    Smoke,
    Water,
    Lock,
    EmoticonGrowingQMark,
    EmoticonQMark,
    EmoticonXMark,
    EmoticonZzz,
    EmoticonThunderstorm,
    EmoticonCloud,
    EmoticonDrunken,
    EmoticonSun,
    EmoticonKo,
    Plouf,
    Ghost,
    AppleSmell,
    Speak,
    DangerPoint,
    Hidden,
    WorkIconArrows,
    WorkIconPurses,
    WorkIconStones,
    WorkIconApples,
    WorkIconBeer,
    WorkIconLegs,
    WorkIconPlants,
    WorkIconNets,
    WorkIconWasps,
    WorkIconBowTraining,
    WorkIconSwordTraining,
    WorkIconRegeneration,
    NumberOfRows,
}
