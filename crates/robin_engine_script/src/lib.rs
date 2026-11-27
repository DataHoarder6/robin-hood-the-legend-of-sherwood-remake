//! The script layer below the simulation: the bytecode VM (`interp`), script
//! programs and instances (`script_manager`), sprite scripts (`sprite_script`),
//! the engine operations a native call yields (`native_commands`), and the
//! resource-derived values asset preparation hands the engine (hit masks,
//! titbit sprite rows, ground-mark sprite data).
//!
//! It never reaches back into `robin_engine`, so asset preparation
//! (`robin_assets`) and the offline tools compile beside the engine instead of
//! after it, and an engine change recompiles neither
//! (`docs/slop/build-time-root-causes.md`). `robin_engine` re-exports every
//! module and type at its old path.

// Catch the "method-lvalue footgun" the engine root denies as well.
#![deny(unused_must_use)]

// The shared crates under the names the engine root gives them.
pub(crate) use robin_data_io::{legacy_io, sbfile};
pub(crate) use robin_engine_geometry::{order, patch, position_interface};
pub(crate) use robin_engine_types::{coordinates, entity_id};
pub(crate) use robin_script_types::{scb, spellforge, vm};

pub mod ai_values;
pub mod ground_mark;
pub mod hit_mask;
pub mod interp;
pub mod jingle;
pub mod native_commands;
pub mod script_manager;
pub mod sequence_id;
pub mod sprite_script;
pub mod titbit_sprite_row;
