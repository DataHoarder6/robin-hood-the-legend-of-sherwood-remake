//! F7 teleport cheat — dispatch a teleport movement element for every
//! selected PC.
//!
//! Triggered by the `MSG_TELEPORT` input event.  The first selected PC
//! is teleported to the mouse map point; every subsequent PC keeps its
//! offset relative to the first.
//!
//! When there is no selection and the free-shadow-polygon dev cheat is
//! active, the teleport instead repositions the free-floating shadow-
//! polygon viewer.  That branch writes to non-deterministic dev state
//! (`DevState::cheat_free_shadow_polygon_pos`) so it is handled host-
//! side in `game_session.rs` rather than going through the
//! PlayerCommand / replay pipeline.

use super::EngineInner;
use crate::coordinates::MapPoint;
use crate::element::{Command, EntityId};
use crate::engine::TickCtx;
use crate::order::OrderType;
use crate::sequence::{MoveFlags, SequenceElement, SequenceElementData};

impl EngineInner {
    /// Teleport every selected PC.  The first selected PC lands on
    /// `dest`; subsequent PCs keep their offset from the first (the
    /// displacement is `dest - first_pc.position`).
    ///
    /// Each teleport is dispatched as a one-element sequence carrying
    /// a [`Command::Teleport`] movement element — the actual position
    /// snap + star-burst effects are handled by the existing
    /// `Command::Teleport` branch in `engine::tick`.
    pub(crate) fn manage_input_process_teleport(
        &mut self,
        tcx: TickCtx<'_>,
        dest: MapPoint,
        layer: u16,
        sector: Option<crate::position_interface::SectorHandle>,
        target_frame: Option<u32>,
    ) {
        let selected = self.players.seats[0].selection.clone();
        if selected.is_empty() {
            return;
        }
        // Anchor the teleport's fade countdown and star titbits to the
        // command's target frame, which every seat receives identically, so
        // those fields agree when the element executes after admission (D12 /
        // #83).
        let target_frame = target_frame.unwrap_or(self.control.frame_counter);
        // Commands run before the hourglass increments `frame_counter`, so the
        // element's authoritative execution frame is `frame_counter + 1`. A
        // command whose target frame is already past must undo the fallback
        // allocations made at that frame before launching (D12 / #83).
        let execution_frame = self.control.frame_counter.saturating_add(1);
        let late = execution_frame > target_frame;
        for pc in &selected {
            if let Some(entity) = self.world.entities.get_mut(*pc)
                && let Some(pc_data) = entity.pc_data_mut()
            {
                pc_data.teleport_target_frame = Some(target_frame);
            }
        }

        // Snapshot the first PC's current position to compute the
        // displacement that the rest of the group preserves.
        let first_pos = match self
            .get_entity(selected[0])
            .map(|e| e.element_data().position_map())
        {
            Some(p) => p,
            None => return,
        };

        let mut destinations = Vec::with_capacity(selected.len());
        for (idx, pc_id) in selected.iter().enumerate() {
            let pc_dest = if idx == 0 {
                dest
            } else {
                let pos = match self
                    .get_entity(*pc_id)
                    .map(|e| e.element_data().position_map())
                {
                    Some(p) => p,
                    None => continue,
                };
                MapPoint {
                    x: pos.x + (dest.x - first_pos.x),
                    y: pos.y + (dest.y - first_pos.y),
                }
            };
            destinations.push((*pc_id, pc_dest));
        }

        let displaced = if late {
            // Rewind before launching anything: a launched teleport installs a
            // new fallback for its owner and would hide a sibling's target-
            // frame slot. The target frame's actor update creates a Wait per
            // idle actor, so the tail can hold other actors' fallbacks as well
            // as the selected PCs' (#94). The manager reclaims the whole run
            // bounded by the selected actors' Waits, and every reclaimed slot
            // is re-homed after the teleport claims its ids.
            // TODO: a PC busy with a real order has no Wait; a lag beyond one
            // frame creates more fallbacks — generalise the rewind to both.
            let owners: Vec<_> = destinations.iter().map(|(pc_id, _)| *pc_id).collect();
            self.orders
                .sequence_manager
                .rewind_latest_wait_slots(&owners)
        } else {
            Vec::new()
        };

        // Both seats now launch through the same ordinary path. On the late
        // seat that path re-allocates precisely the slots rewound above.
        for (pc_id, pc_dest) in destinations {
            let mut elem = SequenceElement::new_movement(
                1,
                Command::Teleport,
                Some(pc_id),
                OrderType::RunningUpright,
            );
            elem.data = SequenceElementData::Movement {
                destination: crate::coordinates::MapPoint {
                    x: pc_dest.x,
                    y: pc_dest.y,
                },
                layer,
                sector,
                gate_id: None,
                line_id: None,
                element: None,
                flags: MoveFlags::empty(),
                tolerance: 0.0,
                direction: 0,
                action: OrderType::RunningUpright,
                speed_factor: 1.0,
                linked_seek: None,
                post_seek_sequence: None,
            };
            self.launch_element(tcx, elem);
        }
        if late {
            self.orders.sequence_manager.rehome_sequences(displaced);
        }
    }

    /// Apply a pre-rolled Sherwood placement jitter and facing.
    ///
    /// The position jitter is only committed when the candidate bbox is
    /// collision-free (`is_position_authorized`) AND a straight-line
    /// path from the current position to the candidate is clear
    /// (`is_reachable_thin`).  The facing is ALWAYS reseeded — including
    /// when the position commit is rejected.
    ///
    /// Wired in at `engine::level_loading::spawn_sherwood_pcs` — every
    /// returning PC with a Sherwood beam-me index gets a randomised
    /// position around the spawn anchor.
    pub(super) fn apply_randomized_position(&mut self, eid: EntityId, roll: SherwoodPlacementRoll) {
        let (current_pos, layer, move_box) = {
            let Some(e) = self.get_entity(eid) else {
                tracing::warn!(?eid, "randomize_position: missing entity");
                return;
            };
            let ed = e.element_data();
            let pos = ed.position_map();
            let layer = ed.layer();
            let move_box = *ed.sprite.position_iface.get_move_box();
            (pos, layer, move_box)
        };

        let new_pos = crate::coordinates::MapPoint {
            x: current_pos.x + roll.dx,
            y: current_pos.y + roll.dy,
        };
        let bbox_at_new = move_box.translated(new_pos);

        let authorized = self
            .world
            .fast_grid
            .is_position_authorized(&bbox_at_new, layer)
            && self
                .world
                .fast_grid
                .is_reachable_thin(current_pos, new_pos, layer);

        let Some(entity) = self.get_entity_mut(eid) else {
            return;
        };
        if authorized {
            entity.element_data_mut().sprite.position_iface.new_move();
            entity.element_data_mut().set_position_map(new_pos);
        }
        entity
            .element_data_mut()
            .set_direction_instantly(roll.direction);
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SherwoodPlacementRoll {
    dx: f32,
    dy: f32,
    direction: i16,
}

/// Draw the returning-PC placement before the beam-me shuffle, matching
/// The original game's mission-character creation order.
pub(super) fn roll_sherwood_placement(
    sim: &crate::sim_rng::SimulationContext,
) -> SherwoodPlacementRoll {
    const RANDOM_SHERWOOD_POSITION: f32 = 5.0;
    let axis = || {
        (crate::sim_rng::c_rand_unit_inclusive(
            sim,
            crate::sim_rng::RngSite::SherwoodReturningPcPlacement,
        ) * 2.0
            - 1.0)
            * RANDOM_SHERWOOD_POSITION
    };
    SherwoodPlacementRoll {
        dx: axis(),
        dy: axis(),
        direction: crate::sim_rng::u32(
            sim,
            crate::sim_rng::RngSite::SherwoodReturningPcPlacement,
            0..16,
        ) as i16,
    }
}
