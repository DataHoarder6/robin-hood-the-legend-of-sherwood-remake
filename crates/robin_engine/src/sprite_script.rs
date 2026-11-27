//! Sprite scripts live in `robin_engine_script`; this module adds the one rule
//! that reads a mission's level assets.

pub use robin_engine_script::sprite_script::*;

/// An explicitly prepared replacement package owns script execution without
/// requiring an original SCB. Augment modes retain the original-program contract.
pub fn original_mission_program_required(
    assets: &crate::engine::LevelAssets,
    script_enabled: bool,
) -> bool {
    original_program_requirement(
        script_enabled,
        assets
            .attachments
            .spellforge_runtime
            .as_ref()
            .map(|runtime| runtime.package().script_mode),
    )
}
