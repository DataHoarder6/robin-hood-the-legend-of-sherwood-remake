//! The simulation's geometry and pathing layer: positions, sectors, gates,
//! sight obstacles, the fast-find grid, patches and material zones, plus the
//! element kinds and orders they are expressed in.
//!
//! It sits below `robin_engine` and never reaches back into it, so an engine
//! change does not recompile it and the engine's own compilation unit is
//! smaller (`docs/slop/build-time-root-causes.md`). `robin_engine` re-exports
//! every module at its old path.

#![deny(unsafe_op_in_unsafe_fn)]

// The shared value-type crates under the names the engine root gives them.
pub(crate) use robin_engine_types::{coordinates, entity_id, geo2d};
pub(crate) use robin_level_data::{level_data, profiles};

pub mod bitcode_adapters;
pub mod diagnostics;
pub mod element_kinds;
pub mod fast_find_grid;
pub mod gate;
pub mod jump_line;
pub mod le_bytes;
pub mod mask;
pub mod material_sectors;
pub mod order;
pub mod patch;
pub mod position_interface;
pub mod repulsive;
pub mod rhline;
pub mod sector;
pub mod sector_production;
pub mod shadow_polygon;
pub mod sight_obstacle;
pub mod sound_material;
pub mod water_zones;
