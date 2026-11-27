//! Where the skill tree's gameplay values meet the simulation.
//!
//! The campaign bitmask is the only stored fact; `SkillEffects` is derived on
//! demand and never serialized, so peers and replays with the same skill set
//! derive the same numbers on every tick.  Values that already have a
//! serialized home (`PcData::life_points`, `has_lockpick`, `has_climb`) are
//! stamped once when the PC spawns, or on a live purchase.  The remaining
//! values are read at the exact seam that consumes them — movement speed,
//! walking noise and the Listen radius — so they need no new state field.

use crate::campaign::PcDescription;
use crate::campaign::skill_tree::{SkillEffect, SkillEffects, SkillNode};
use crate::element::PcData;
use crate::entity_id::EntityId;

use super::EngineInner;

impl EngineInner {
    /// Derive a live PC's effects from the campaign record that owns it.
    ///
    /// Identity is the same `campaign_description_index` the perk paths use;
    /// character profiles are not identities because many peasants share one.
    pub(crate) fn pc_skill_effects(&self, id: EntityId) -> SkillEffects {
        let Some(index) = self
            .world
            .entities
            .get(id)
            .and_then(|entity| entity.pc_data())
            .and_then(|pc| self.pc_description_index_for_pc_data(pc))
        else {
            tracing::warn!(
                ?id,
                "PC has no campaign identity; using neutral skill effects"
            );
            return SkillEffects::neutral();
        };
        let description = &self.mission_domain.campaign.characters[index];
        SkillEffects::from_owner(|node| description.has_skill(node))
    }
}

/// Stamp all serialized effects of a character's skill set onto a newly
/// spawned PC.  This runs after the profile-derived fields are written, so
/// Burglars Kit adds an authorization the profile alone did not grant.
pub(crate) fn apply_spawn_effects(pc: &mut PcData, description: &PcDescription) {
    let effects = SkillEffects::from_owner(|node| description.has_skill(node));
    pc.life_points = pc.life_points.saturating_add(effects.bonus_life_points);
    pc.has_lockpick |= effects.lockpick_and_climb;
    pc.has_climb |= effects.lockpick_and_climb;
}

/// Apply a node bought during a live mission to the PCs that already exist.
/// Spawn has stamped the nodes owned at mission start, so only the newly
/// purchased node's serialized effect is added here; a replayed duplicate
/// purchase is rejected by the campaign authority before this runs.
pub(crate) fn apply_purchased_node(pc: &mut PcData, node: SkillNode) {
    match node.effect() {
        SkillEffect::BonusLifePoints(bonus) => {
            if pc.life_points > 0 {
                pc.life_points = pc.life_points.saturating_add(bonus);
            }
        }
        SkillEffect::LockpickAndClimb => {
            pc.has_lockpick = true;
            pc.has_climb = true;
        }
        // Speed, noise and Listen are read from the campaign on every tick;
        // combat nodes keep their existing serialized perk flags.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::campaign::{PcDescription, SkillSet};

    #[test]
    fn spawn_stamps_health_and_movement_authorization_into_existing_fields() {
        let mut description = PcDescription::default();
        description.skills = SkillSet::empty();
        description.grant_skill(SkillNode::Hardy);
        description.grant_skill(SkillNode::BurglarsKit);

        let mut pc = PcData::default();
        assert_eq!(pc.life_points, crate::pc_status::LIFEPOINTS_PC);
        assert!(!pc.has_lockpick);
        assert!(!pc.has_climb);

        apply_spawn_effects(&mut pc, &description);
        assert_eq!(
            pc.life_points,
            crate::pc_status::LIFEPOINTS_PC + crate::campaign::skill_tree::HARDY_BONUS_LIFE_POINTS
        );
        assert!(pc.has_lockpick);
        assert!(pc.has_climb);
    }

    #[test]
    fn spawn_without_skills_leaves_the_serialized_fields_untouched() {
        let mut pc = PcData::default();
        pc.life_points = 63;
        apply_spawn_effects(&mut pc, &PcDescription::default());
        assert_eq!(pc.life_points, 63);
        assert!(!pc.has_lockpick);
        assert!(!pc.has_climb);
    }
}
