//! `Entity::camp()` returns `Camp::Error` for non-actor entities by design.
//! The renderer and AI query player-alignment for every entity in draw order,
//! so an unresolved camp must answer "not aligned" instead of reaching the
//! diplomacy guard that (correctly) rejects invalid allegiances.

use super::*;
use crate::element::Camp;

#[test]
fn unresolved_camp_is_not_player_aligned() {
    let engine = EngineInner::new();
    assert!(!engine.is_player_aligned_camp(Camp::Error));
}
