//! Opt-in adaptive pressure director for staged off-map garrison bodies.
//!
//! The decision is a pure function of already-hashed simulation state: the
//! mission's global alert level, the frame counter and the authored off-map
//! budget. No new simulation field and no RNG draw are introduced, so sessions
//! that do not opt in keep their existing state hashes and replay identity.
//!
//! The opt-in is the existing `extra_guards` `SimConfig` axis: it is already
//! ranked-pinned, already forced off for Original-RNG parity, and it is the
//! only gate under which staged bodies and a nonzero budget exist.

use super::EngineInner;
use super::reinforcement::{SummonEntry, SummonRequest};
use crate::ai::{AlertLevel, OffmapReinforcementBudget};
use crate::element::Camp;

/// Cadence and limits of the director. These are presentation-independent
/// gameplay constants rather than a new `SimConfig` field: adding a field
/// would change the hash of every frame, including opt-out sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AdaptivePressureSettings {
    pub decision_cadence: u32,
    pub start_after_frames: u32,
    pub max_extra_summon_count: u16,
    pub max_min_tier: u8,
}

impl Default for AdaptivePressureSettings {
    fn default() -> Self {
        Self {
            decision_cadence: 25,
            start_after_frames: 25 * 5,
            max_extra_summon_count: 4,
            max_min_tier: 3,
        }
    }
}

/// One request emitted by the director, in the shape the reinforcement seam
/// already accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AdaptiveSummon {
    pub faction: Camp,
    pub min_tier: u8,
}

/// Decide whether this frame's director phase should call a staged body in.
pub(super) fn decide_adaptive_summon(
    settings: AdaptivePressureSettings,
    alert: AlertLevel,
    frame: u32,
    budget: &OffmapReinforcementBudget,
    faction: Camp,
) -> Option<AdaptiveSummon> {
    // Green is the stealthy encounter: leave the authored mission untouched.
    if alert == AlertLevel::Green
        || frame < settings.start_after_frames
        || !frame.is_multiple_of(settings.decision_cadence)
        || budget.remaining == 0
        || frame < budget.next_allowed_frame
    {
        return None;
    }
    // `commit` never refunds a lifetime slot, so the already-spent count is
    // derived from state that is already hashed. No director field, no RNG.
    let prior_arrivals = budget.total.saturating_sub(budget.remaining);
    if prior_arrivals >= settings.max_extra_summon_count {
        return None;
    }
    let floor = u8::from(alert == AlertLevel::Red);
    let min_tier = floor
        .saturating_add(prior_arrivals.min(2) as u8)
        .min(settings.max_min_tier);
    Some(AdaptiveSummon { faction, min_tier })
}

impl EngineInner {
    /// Director phase, called between `MissionAndMessages` and `Paths`.
    /// No-op unless the opt-in axis is enabled and an alert is active.
    pub(super) fn hourglass_phase_adaptive_pressure(&mut self, tcx: crate::engine::TickCtx<'_>) {
        let config = self.control.sim_config;
        if !config.extra_guards || !config.enable_offmap_reinforcements {
            return;
        }
        let alert = self.ai.global.overall_villain_alert();
        let frame = self.control.frame_counter;
        let budget = &self.ai.global.offmap_budget;
        let Some(faction) = self.staged_garrison_faction() else {
            return;
        };
        let Some(decision) = decide_adaptive_summon(
            AdaptivePressureSettings::default(),
            alert,
            frame,
            budget,
            faction,
        ) else {
            return;
        };
        let summoned = self.summon_garrison(
            tcx,
            SummonRequest {
                faction: decision.faction,
                entry: SummonEntry::Any,
                min_tier: decision.min_tier,
            },
        );
        if summoned.is_some() {
            tracing::debug!(
                frame,
                ?alert,
                ?faction,
                min_tier = decision.min_tier,
                "adaptive pressure called in a staged garrison body"
            );
        }
    }

    /// Camp of the first staged, still-inactive garrison body. Returning
    /// `None` when there is no such body keeps the director from inventing a
    /// faction for a mission that has no staged content.
    fn staged_garrison_faction(&self) -> Option<Camp> {
        self.world
            .npc_registry_ids
            .iter()
            .copied()
            .filter_map(|id| self.get_entity(id))
            .find(|entity| {
                !entity.element_data().active
                    && entity.soldier_data().is_some_and(|soldier| soldier.staged)
            })
            .map(|entity| entity.camp())
            .filter(|camp| *camp != Camp::Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(total: u16, remaining: u16, cooldown: u32) -> OffmapReinforcementBudget {
        OffmapReinforcementBudget {
            total,
            concurrent: 2,
            cooldown_ticks: cooldown,
            remaining,
            live: 0,
            next_allowed_frame: 0,
        }
    }

    fn settings() -> AdaptivePressureSettings {
        AdaptivePressureSettings {
            start_after_frames: 0,
            ..AdaptivePressureSettings::default()
        }
    }

    #[test]
    fn stealthy_green_alert_never_calls_a_staged_body() {
        let decision = decide_adaptive_summon(
            settings(),
            AlertLevel::Green,
            25,
            &budget(4, 4, 10),
            Camp::Lacklandists,
        );
        assert_eq!(decision, None);
    }

    #[test]
    fn sustained_noise_on_the_cadence_calls_one_body() {
        let decision = decide_adaptive_summon(
            settings(),
            AlertLevel::Yellow,
            25,
            &budget(4, 4, 10),
            Camp::Lacklandists,
        );
        assert_eq!(
            decision,
            Some(AdaptiveSummon {
                faction: Camp::Lacklandists,
                min_tier: 0,
            })
        );
    }

    #[test]
    fn red_alert_and_earlier_arrivals_raise_the_minimum_tier() {
        let first = decide_adaptive_summon(
            settings(),
            AlertLevel::Red,
            25,
            &budget(4, 4, 0),
            Camp::Lacklandists,
        );
        assert_eq!(first.map(|d| d.min_tier), Some(1));
        let escalated = decide_adaptive_summon(
            settings(),
            AlertLevel::Red,
            50,
            &budget(4, 2, 0),
            Camp::Lacklandists,
        );
        assert_eq!(
            escalated.map(|d| d.min_tier),
            Some(3),
            "Red floor plus two prior arrivals reaches the tier ceiling"
        );
    }

    #[test]
    fn cooldown_and_cadence_suppress_extra_calls() {
        let mut spent = budget(4, 3, 30);
        spent.next_allowed_frame = 40;
        assert_eq!(
            decide_adaptive_summon(
                settings(),
                AlertLevel::Yellow,
                25,
                &spent,
                Camp::Lacklandists,
            ),
            None,
            "a frame inside the cooldown must not call another body"
        );
        assert_eq!(
            decide_adaptive_summon(
                settings(),
                AlertLevel::Yellow,
                24,
                &budget(4, 4, 0),
                Camp::Lacklandists,
            ),
            None,
            "off-cadence frames are not decision points"
        );
    }

    #[test]
    fn summon_cap_and_budget_are_respected_without_new_state() {
        let cap_settings = AdaptivePressureSettings {
            max_extra_summon_count: 2,
            ..settings()
        };
        assert_eq!(
            decide_adaptive_summon(
                cap_settings,
                AlertLevel::Red,
                25,
                &budget(4, 1, 0),
                Camp::Lacklandists,
            ),
            None,
            "prior arrivals counted from total-remaining reach the cap"
        );
        assert_eq!(
            decide_adaptive_summon(
                settings(),
                AlertLevel::Red,
                25,
                &budget(0, 0, 0),
                Camp::Lacklandists,
            ),
            None,
            "no authored budget means no staged body may be woken"
        );
    }

    #[test]
    fn decision_waits_for_the_start_window() {
        let delayed = AdaptivePressureSettings {
            start_after_frames: 50,
            ..settings()
        };
        assert_eq!(
            decide_adaptive_summon(
                delayed,
                AlertLevel::Yellow,
                25,
                &budget(4, 4, 0),
                Camp::Lacklandists,
            ),
            None
        );
        assert!(
            decide_adaptive_summon(
                delayed,
                AlertLevel::Yellow,
                50,
                &budget(4, 4, 0),
                Camp::Lacklandists,
            )
            .is_some()
        );
    }

    #[test]
    fn decision_is_deterministic_and_draws_no_rng() {
        let first = decide_adaptive_summon(
            settings(),
            AlertLevel::Yellow,
            25,
            &budget(4, 4, 0),
            Camp::Lacklandists,
        );
        for _ in 0..8 {
            assert_eq!(
                decide_adaptive_summon(
                    settings(),
                    AlertLevel::Yellow,
                    25,
                    &budget(4, 4, 0),
                    Camp::Lacklandists,
                ),
                first
            );
        }
    }
}
