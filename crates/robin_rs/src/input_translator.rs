//! Game input translator.
//!
//! Translates raw keyboard state and mouse position/wheel into high-level
//! [`GameAction`]s, returning the actions for the caller to dispatch.

use robin_engine::coordinates::ScreenPoint;
use std::collections::BTreeSet;

use bitflags::bitflags;
use enum_map::{Enum, EnumMap};
use geo::Rect;
use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::key_config::{KeyConfig, REAL_KEY_COUNT};

// ---------------------------------------------------------------------------
// GameKey — bindable action slots
// ---------------------------------------------------------------------------

/// Bindable game key slots.  Each slot maps to a physical key via the key
/// configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Enum)]
#[repr(u16)]
pub enum GameKey {
    ZoomIn = 0,
    ZoomOut,

    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,

    DisplayMap,

    SelectCharacter1,
    SelectCharacter2,
    SelectCharacter3,
    SelectCharacter4,
    SelectCharacter5,
    SelectAll,
    SelectNone,

    CrouchDown,
    StandUp,

    ShowDoors,
    SwitchHiddenDisplay,

    Action1,
    Action2,
    Action3,

    MoveDuringAction,

    RecordQa,
    StartQa,
    DeleteQa,

    ShowViewCone,

    QuickSave1,
    QuickLoad1,
    PlanQuickActions,
    ToggleCloak,

    // --- Non-rebindable / debug ---
    StartMission,
    /// Player-triggered reveal of hostiles/NPCs hidden in buildings.
    /// CapsLock is taken by `SwitchHiddenDisplay` (`draw_hidden`, a
    /// different mechanism), so this uses its own reserved key.
    RevealHiddenCharacters,
    DisplayMenu,

    RecordMovie,

    PrintScreen,
    DisplayConsole,

    SlowMotion,
    RequestInfo,
    Teleport,
    AiInfo,
    /// Built-in Sherwood-only shortcut for the post-port trading panel.
    SherwoodTrading,
    /// Non-rebindable cooperative pause toggle (P). Host-authoritative in
    /// multiplayer; Esc/DisplayMenu stays the local modal binding.
    CoopPause,
}

// The persisted key-config rows occupy the prefix before reserved/debug keys.
// Fail at compile time if either side gains a slot without updating the other.
const _: () = assert!(GameKey::StartMission as u16 == REAL_KEY_COUNT);

// ---------------------------------------------------------------------------
// GameAction — output actions produced by translation
// ---------------------------------------------------------------------------

/// High-level game action produced by input translation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GameAction {
    // Camera
    ScrollLeft,
    ScrollRight,
    ScrollUp,
    ScrollDown,
    ZoomIn,
    ZoomOut,

    // Character selection
    SelectCharacter {
        portrait_index: u8,
    },
    SelectAll,
    UnselectAll,

    // Action slots
    SelectAction {
        index: u8,
    },

    // Modifier key press/release (ShowDoors, ShowViewCone, MoveDuringAction)
    KeyShift,
    KeyReleaseShift,
    KeyAlt,
    KeyReleaseAlt,
    KeyControl,
    KeyReleaseControl,

    // UI / Display
    DisplayMenu,
    DisplayConsole,
    DisplayInfo,
    DisplayAiInfo,
    /// Ask the host presentation to open the Sherwood trading panel.
    OpenSherwoodTrading,
    SwitchMaskedDisplay,
    /// Toggle constant-alpha outlines for hidden hostiles/NPCs.
    RevealHiddenCharacters,
    PrintScreen,

    // Game control
    SlowMotion,
    Teleport,
    RecordMovie,
    QuickSave,
    QuickLoad,

    // Macros (quick-action recording)
    StartMacro,
    DeleteAllMacros,
    /// RECORD_QA keybind (default F5).  The accelerator that the
    /// clock widget binds; the consumer in `game_session.rs` replays
    /// the corner-clock left-click path — record / cycle the
    /// currently selected PC's macro slot.
    RecordQa,

    // Stance
    CrouchDown,
    StandUp,
    ToggleCloak,

    // System
    SwitchTask,
    /// Toggle the host-authoritative cooperative pause (P).
    CoopPause,
}

// ---------------------------------------------------------------------------
// TranslationFlags — controls which input categories are active
// ---------------------------------------------------------------------------

bitflags! {
    /// Controls which input categories are translated.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct TranslationFlags: u32 {
        const QUICK_LOAD = 0x01;
        const QUICK_SAVE = 0x02;
        const INGAME_MENU = 0x04;
        const MISSION = 0x08;

        /// All categories enabled (default).
        const ALL = 0x0F;
    }
}

// ---------------------------------------------------------------------------
// Key state edge-detection helpers
// ---------------------------------------------------------------------------

fn key_hit(cur: &BTreeSet<KeyCode>, prev: &BTreeSet<KeyCode>, key: Option<KeyCode>) -> bool {
    key.is_some_and(|key| cur.contains(&key) && !prev.contains(&key))
}

fn key_released(cur: &BTreeSet<KeyCode>, prev: &BTreeSet<KeyCode>, key: Option<KeyCode>) -> bool {
    key.is_some_and(|key| !cur.contains(&key) && prev.contains(&key))
}

fn key_held(cur: &BTreeSet<KeyCode>, key: Option<KeyCode>) -> bool {
    key.is_some_and(|key| cur.contains(&key))
}

// ---------------------------------------------------------------------------
// Dead zone helpers
// ---------------------------------------------------------------------------

/// Check if a point falls inside any dead zone rectangle (boundary-inclusive).
///
/// We use `Intersects` rather than `Contains` because `geo::Contains` for
/// `Rect` uses strict inequality and excludes boundary points — but dead zones
/// need to cover the exact screen edges where scrolling triggers.
fn is_in_dead_zone(dead_zones: &[Rect<f32>], point: ScreenPoint) -> bool {
    use geo::Intersects;
    let p = geo::Point::new(point.x, point.y);
    dead_zones.iter().any(|dz| dz.intersects(&p))
}

// ---------------------------------------------------------------------------
// InputTranslator
// ---------------------------------------------------------------------------

/// Translates raw input events into [`GameAction`]s.
///
/// Maintains previous keyboard state for edge-detection and dead-zone
/// rectangles for mouse edge-scroll suppression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputTranslator {
    /// Physical key bound to each [`GameKey`] slot.
    bindings: EnumMap<GameKey, Option<KeyCode>>,
    /// Previous frame's keyboard state for edge detection.
    prev_keys: BTreeSet<KeyCode>,
    /// Rectangular screen regions where mouse edge-scrolling is suppressed
    /// (e.g. UI panels along screen borders).
    dead_zones: Vec<Rect<f32>>,
    pub screen_width: f32,
    pub screen_height: f32,
    /// Whether the user is "locked" (UI modal, cutscene, etc.).
    user_locked: bool,
    edge_scrolling: bool,
}

impl Default for InputTranslator {
    fn default() -> Self {
        Self {
            bindings: EnumMap::default(),
            prev_keys: BTreeSet::new(),
            dead_zones: Vec::new(),
            screen_width: 1024.0,
            screen_height: 768.0,
            user_locked: false,
            edge_scrolling: true,
        }
    }
}

impl InputTranslator {
    pub fn new(screen_width: f32, screen_height: f32, key_config: &KeyConfig) -> Self {
        let mut t = Self {
            screen_width,
            screen_height,
            ..Self::default()
        };
        t.load_bindings_from_keyconfig(key_config);
        t.set_reserved_bindings();
        t
    }

    /// Set the non-rebindable console, menu, and debug bindings at construction.
    fn set_reserved_bindings(&mut self) {
        use KeyCode::*;
        self.bindings[GameKey::DisplayConsole] = Some(Backquote);
        self.bindings[GameKey::PrintScreen] = Some(PrintScreen);
        self.bindings[GameKey::DisplayMenu] = Some(Escape);
        self.bindings[GameKey::SherwoodTrading] = Some(KeyT);
        self.bindings[GameKey::CoopPause] = Some(KeyP);
        // CapsLock already drives SwitchMaskedDisplay (masked sprite
        // edges), so the reveal key is a distinct, unused binding.
        self.bindings[GameKey::RevealHiddenCharacters] = Some(KeyO);

        // Debug keys (only in non-shipping builds)
        self.bindings[GameKey::SlowMotion] = Some(Pause);
        self.bindings[GameKey::Teleport] = Some(F7);
        self.bindings[GameKey::RecordMovie] = Some(NumpadEnter);
        self.bindings[GameKey::RequestInfo] = Some(Home);
    }

    // --- Binding management ---

    pub fn set_binding(&mut self, key: GameKey, physical_key: Option<KeyCode>) {
        self.bindings[key] = physical_key;
    }

    /// Apply the shipping-build deity easter egg rebind. The original
    /// Triggered from `EngineInner::run_console_command` via
    /// `ConsoleResponse::DeityInvoked`, drained by the host game loop.
    pub fn deity_call(&mut self) {
        use KeyCode::*;
        self.bindings[GameKey::SlowMotion] = Some(ScrollLock);
        self.bindings[GameKey::Teleport] = Some(Home);
        self.bindings[GameKey::RecordMovie] = Some(NumpadEnter);
        self.bindings[GameKey::RequestInfo] = Some(End);
        // The reference duplicates the SLOW_MOTION assignment;
        // preserved here as a redundant write for literal parity.
        self.bindings[GameKey::SlowMotion] = Some(ScrollLock);
    }

    pub fn get_binding(&self, key: GameKey) -> Option<KeyCode> {
        self.bindings[key]
    }

    pub fn is_binding_held(&self, key: GameKey, current: &BTreeSet<KeyCode>) -> bool {
        key_held(current, self.key(key))
    }

    /// Load rebindable keys from a [`KeyConfig`].
    ///
    /// Uses index-based loading — a raw copy from the key config's
    /// flat array. Reserved/debug bindings (including deity overrides) and
    /// input edge state belong to the session and are preserved.
    pub fn load_bindings_from_keyconfig(&mut self, cfg: &KeyConfig) {
        for (game_key, binding) in self.bindings.iter_mut().take(REAL_KEY_COUNT as usize) {
            *binding = cfg.get_key_by_index(game_key as u16);
        }
    }

    /// Edge detection helper for physical keys that aren't routed through the
    /// standard [`GameAction`] translation path. Returns `true` on the
    /// frame the key transitions from up -> down. Must be called
    /// before [`Self::translate_keyboard`] advances `prev_keys`.
    pub fn was_key_pressed(&self, key: KeyCode, current: &BTreeSet<KeyCode>) -> bool {
        key_hit(current, &self.prev_keys, Some(key))
    }

    /// Edge detection helper for physical keys that aren't routed through the
    /// standard [`GameAction`] translation path — e.g. the minimap
    /// accelerator, which is stored on the widget rather than bound to
    /// a [`GameAction`] variant.  Returns `true` on the frame the
    /// key transitions from down to up.  Must be called before
    /// [`Self::translate_keyboard`] (which advances `prev_keys`).
    pub fn was_key_released(&self, key: KeyCode, current: &BTreeSet<KeyCode>) -> bool {
        key_released(current, &self.prev_keys, Some(key))
    }

    /// Look up which [`GameKey`] a physical key is bound to.
    fn translate_key(&self, key: KeyCode) -> Option<GameKey> {
        self.bindings
            .iter()
            .find_map(|(game_key, binding)| (*binding == Some(key)).then_some(game_key))
    }

    // --- Dead zones ---

    fn clear_dead_zones(&mut self) {
        self.dead_zones.clear();
    }

    /// Add a rectangular dead zone defined by two corner points.
    fn add_dead_zone(&mut self, a: ScreenPoint, b: ScreenPoint) {
        let min_x = a.x.min(b.x);
        let max_x = a.x.max(b.x);
        let min_y = a.y.min(b.y);
        let max_y = a.y.max(b.y);
        self.dead_zones.push(Rect::new(
            geo::coord! { x: min_x, y: min_y },
            geo::coord! { x: max_x, y: max_y },
        ));
    }

    /// Install the four HUD-adjacent edge-scroll dead-zone strips that
    /// keep the mouse from scrolling the viewport when the cursor is
    /// parked on or beside the bottom HUD panels. Called from
    /// post-initialize and on resolution change. `PANNEL_DEADZONE = 60`.
    pub fn install_hud_dead_zones(&mut self) {
        const PANNEL_DEADZONE: f32 = 60.0;
        let w = self.screen_width;
        let h = self.screen_height;

        self.clear_dead_zones();

        // Bottom-left vertical strip:
        //   ptA=(0, h-PANNEL_DEADZONE) .. ptB=(0, h-3)
        self.add_dead_zone(
            ScreenPoint::new(0.0, h - PANNEL_DEADZONE),
            ScreenPoint::new(0.0, h - 3.0),
        );
        // Bottom-left horizontal strip:
        //   ptA=(2, h-1) .. ptB=(PANNEL_DEADZONE, h-1)
        self.add_dead_zone(
            ScreenPoint::new(2.0, h - 1.0),
            ScreenPoint::new(PANNEL_DEADZONE, h - 1.0),
        );
        // Bottom-right horizontal strip:
        //   ptA=(w-PANNEL_DEADZONE, h-1) .. ptB=(w-3, h-1)
        self.add_dead_zone(
            ScreenPoint::new(w - PANNEL_DEADZONE, h - 1.0),
            ScreenPoint::new(w - 3.0, h - 1.0),
        );
        // Bottom-right vertical strip:
        //   ptA=(w-1, h-3) .. ptB=(w-1, h-PANNEL_DEADZONE)
        self.add_dead_zone(
            ScreenPoint::new(w - 1.0, h - 3.0),
            ScreenPoint::new(w - 1.0, h - PANNEL_DEADZONE),
        );
    }

    // --- User lock ---

    pub fn set_user_locked(&mut self, locked: bool) {
        self.user_locked = locked;
    }

    pub fn user_locked(&self) -> bool {
        self.user_locked
    }

    pub fn set_edge_scrolling(&mut self, enabled: bool) {
        self.edge_scrolling = enabled;
    }

    // --- State reset ---

    /// Reset stored keyboard state.  Called when re-entering gameplay.
    pub fn reset_state(&mut self) {
        self.prev_keys.clear();
    }

    // --- Mouse translation ---

    /// Translate mouse position and wheel into game actions.
    ///
    /// Edge-scrolling triggers when the cursor is within 1–2 pixels of
    /// a screen edge and not in a dead zone.
    pub fn translate_mouse(&self, x: f32, y: f32, wheel_delta: i16) -> Vec<GameAction> {
        let mut actions = Vec::new();

        if self.user_locked {
            return actions;
        }

        let point = ScreenPoint::new(x, y);

        if self.edge_scrolling && !is_in_dead_zone(&self.dead_zones, point) {
            if x <= 1.0 {
                tracing::trace!(x, y, "edge-scroll: Left");
                actions.push(GameAction::ScrollLeft);
            }
            if x >= self.screen_width - 2.0 {
                tracing::trace!(x, y, sw = self.screen_width, "edge-scroll: Right");
                actions.push(GameAction::ScrollRight);
            }
            if y <= 1.0 {
                tracing::trace!(x, y, "edge-scroll: Up");
                actions.push(GameAction::ScrollUp);
            }
            if y >= self.screen_height - 2.0 {
                tracing::trace!(x, y, sh = self.screen_height, "edge-scroll: Down");
                actions.push(GameAction::ScrollDown);
            }
        }

        if wheel_delta > 0 {
            actions.push(GameAction::ZoomIn);
        }
        if wheel_delta < 0 {
            actions.push(GameAction::ZoomOut);
        }

        actions
    }

    // --- Keyboard translation ---

    /// Shorthand to get the physical key for a game key slot.
    fn key(&self, gk: GameKey) -> Option<KeyCode> {
        self.bindings[gk]
    }

    /// Translate a full keyboard state array into game actions.
    ///
    /// Call once per frame with the current key state. Updates internal
    /// previous-state for next-frame edge detection.
    pub fn translate_keyboard(
        &mut self,
        keys: &BTreeSet<KeyCode>,
        flags: TranslationFlags,
    ) -> Vec<GameAction> {
        let mut actions = Vec::new();
        let prev = &self.prev_keys;

        // --- Always-active keys ---
        if key_released(keys, prev, self.key(GameKey::SlowMotion)) {
            actions.push(GameAction::SlowMotion);
        }
        if key_released(keys, prev, self.key(GameKey::PrintScreen)) {
            actions.push(GameAction::PrintScreen);
        }

        // --- Ingame menu ---
        if flags.contains(TranslationFlags::INGAME_MENU)
            && key_released(keys, prev, self.key(GameKey::DisplayMenu))
        {
            actions.push(GameAction::DisplayMenu);
        }

        // --- Quick load/save ---
        if flags.contains(TranslationFlags::QUICK_LOAD)
            && key_released(keys, prev, self.key(GameKey::QuickLoad1))
        {
            actions.push(GameAction::QuickLoad);
        }
        if flags.contains(TranslationFlags::QUICK_SAVE)
            && key_released(keys, prev, self.key(GameKey::QuickSave1))
        {
            actions.push(GameAction::QuickSave);
        }

        // --- Mission keys ---
        if flags.contains(TranslationFlags::MISSION) {
            if key_released(keys, prev, self.key(GameKey::DisplayConsole)) {
                actions.push(GameAction::DisplayConsole);
            }
            if key_released(keys, prev, self.key(GameKey::CoopPause)) {
                actions.push(GameAction::CoopPause);
            }

            // Alt+Tab or Ctrl+Esc -> SwitchTask (physical keys, not bindings).
            if key_held(keys, Some(KeyCode::Tab)) && key_held(keys, Some(KeyCode::AltLeft)) {
                actions.push(GameAction::SwitchTask);
            }
            if key_held(keys, Some(KeyCode::Escape))
                && (key_held(keys, Some(KeyCode::ControlLeft))
                    || key_held(keys, Some(KeyCode::ControlRight)))
            {
                actions.push(GameAction::SwitchTask);
            }

            if key_released(keys, prev, self.key(GameKey::RecordMovie)) {
                actions.push(GameAction::RecordMovie);
            }

            // --- User-unlocked mission keys ---
            if !self.user_locked {
                // Modifier key hit (down-edge) and release (up-edge) produce
                // separate actions. Order: all three hits first, then all
                // three releases (group-by-edge).
                let show_doors = self.key(GameKey::ShowDoors);
                let view_cone = self.key(GameKey::ShowViewCone);
                let move_action = self.key(GameKey::MoveDuringAction);

                if key_hit(keys, prev, show_doors) {
                    actions.push(GameAction::KeyShift);
                }
                if key_hit(keys, prev, view_cone) {
                    actions.push(GameAction::KeyAlt);
                }
                if key_hit(keys, prev, move_action) {
                    actions.push(GameAction::KeyControl);
                }

                if key_released(keys, prev, show_doors) {
                    actions.push(GameAction::KeyReleaseShift);
                }
                if key_released(keys, prev, view_cone) {
                    actions.push(GameAction::KeyReleaseAlt);
                }
                if key_released(keys, prev, move_action) {
                    actions.push(GameAction::KeyReleaseControl);
                }

                if key_released(keys, prev, self.key(GameKey::Teleport)) {
                    actions.push(GameAction::Teleport);
                }
                if key_released(keys, prev, self.key(GameKey::SwitchHiddenDisplay)) {
                    actions.push(GameAction::SwitchMaskedDisplay);
                }
                if key_released(keys, prev, self.key(GameKey::RevealHiddenCharacters)) {
                    actions.push(GameAction::RevealHiddenCharacters);
                }
                if key_released(keys, prev, self.key(GameKey::AiInfo)) {
                    actions.push(GameAction::DisplayAiInfo);
                }
                let trading_key = self.key(GameKey::SherwoodTrading);
                if key_released(keys, prev, trading_key)
                    && trading_key.is_some_and(|key| {
                        self.translate_key(key) == Some(GameKey::SherwoodTrading)
                    })
                {
                    actions.push(GameAction::OpenSherwoodTrading);
                }

                // Macro keys
                if key_released(keys, prev, self.key(GameKey::StartQa)) {
                    actions.push(GameAction::StartMacro);
                }
                if key_released(keys, prev, self.key(GameKey::DeleteQa)) {
                    actions.push(GameAction::DeleteAllMacros);
                }
                // RECORD_QA keybind — clock-widget accelerator.
                if key_released(keys, prev, self.key(GameKey::RecordQa)) {
                    actions.push(GameAction::RecordQa);
                }

                // Selection
                if key_released(keys, prev, self.key(GameKey::SelectNone)) {
                    actions.push(GameAction::UnselectAll);
                }
                if key_released(keys, prev, self.key(GameKey::SelectAll)) {
                    actions.push(GameAction::SelectAll);
                }

                // Stance
                if key_released(keys, prev, self.key(GameKey::CrouchDown)) {
                    actions.push(GameAction::CrouchDown);
                }
                if key_released(keys, prev, self.key(GameKey::StandUp)) {
                    actions.push(GameAction::StandUp);
                }
                if key_released(keys, prev, self.key(GameKey::ToggleCloak)) {
                    actions.push(GameAction::ToggleCloak);
                }

                if key_released(keys, prev, self.key(GameKey::RequestInfo)) {
                    actions.push(GameAction::DisplayInfo);
                }

                // Scroll keys use held (continuous while pressed)
                if key_held(keys, self.key(GameKey::ScrollLeft)) {
                    actions.push(GameAction::ScrollLeft);
                }
                if key_held(keys, self.key(GameKey::ScrollRight)) {
                    actions.push(GameAction::ScrollRight);
                }
                if key_held(keys, self.key(GameKey::ScrollUp)) {
                    actions.push(GameAction::ScrollUp);
                }
                if key_held(keys, self.key(GameKey::ScrollDown)) {
                    actions.push(GameAction::ScrollDown);
                }

                // Zoom uses released (single trigger)
                if key_released(keys, prev, self.key(GameKey::ZoomOut)) {
                    actions.push(GameAction::ZoomOut);
                }
                if key_released(keys, prev, self.key(GameKey::ZoomIn)) {
                    actions.push(GameAction::ZoomIn);
                }

                // Action slots precede portrait selection, including when bindings overlap.
                for (key, index) in [GameKey::Action1, GameKey::Action2, GameKey::Action3]
                    .into_iter()
                    .zip(0u8..)
                {
                    if key_released(keys, prev, self.key(key)) {
                        actions.push(GameAction::SelectAction { index });
                    }
                }

                // The caller resolves portrait indices to entities.
                for (key, portrait_index) in [
                    GameKey::SelectCharacter1,
                    GameKey::SelectCharacter2,
                    GameKey::SelectCharacter3,
                    GameKey::SelectCharacter4,
                    GameKey::SelectCharacter5,
                ]
                .into_iter()
                .zip(0u8..)
                {
                    if key_released(keys, prev, self.key(key)) {
                        actions.push(GameAction::SelectCharacter { portrait_index });
                    }
                }
            }
        }

        // Save current state as previous for next frame.
        self.prev_keys.clone_from(keys);

        actions
    }
}

// ---------------------------------------------------------------------------
// Touch input mapping
// ---------------------------------------------------------------------------
//
// The platform recogniser (winit -> touch_input.rs) arbitrates contacts, then
// hands a *classified* gesture to this layer.  This module deliberately has no
// engine or winit dependency: it is the typed vocabulary and mapping contract
// that the touch path consumes, and it is fully testable without a device.
// Adding a variant here does not touch replay or state-hash serialization —
// these actions are produced per poll and dispatched immediately.

/// A finger held this long without a slop breach is a secondary (right-button)
/// press: cancel/stop, right-drag deselect, macro commit, plan cancel.
pub const TOUCH_LONG_PRESS_MS: u32 = 450;
/// A second contact inside this window is a two-finger secondary tap.
pub const TOUCH_TWO_FINGER_TAP_MS: u32 = 250;
/// Travel beyond this distance cancels a pending tap (the recogniser's slop).
pub const TOUCH_GESTURE_SLOP_PX: f64 = 12.0;
pub const TOUCH_PRIMARY_BUTTON: u8 = 1;
pub const TOUCH_SECONDARY_BUTTON: u8 = 3;
pub const TOUCH_QUICK_GROUP_MIN: u8 = 1;
pub const TOUCH_QUICK_GROUP_MAX: u8 = 9;

/// Expand a widget's base rect about its centre to at least 48x48 logical
/// pixels. The old zoom fallback was 24x24 and the plan button 118x34; both are
/// thumb-hostile at the scaled logical canvas. Both touch hit-test gates must
/// call this one helper rather than duplicating a padded rectangle. Returns
/// `(min_x, min_y, max_x, max_y)`, boundary-inclusive, matching the dead-zone
/// convention used by translation.
pub fn touch_target_padding() -> f32 {
    const MIN_FINGER_TARGET: f32 = 48.0;
    MIN_FINGER_TARGET / 2.0
}

pub fn touch_target_rect(
    center_x: f32,
    center_y: f32,
    base_width: f32,
    base_height: f32,
) -> (f32, f32, f32, f32) {
    let half_w = (base_width / 2.0).max(touch_target_padding());
    let half_h = (base_height / 2.0).max(touch_target_padding());
    (
        center_x - half_w,
        center_y - half_h,
        center_x + half_w,
        center_y + half_h,
    )
}

/// Classification of a touch contact from caller-supplied monotonic timing and
/// motion.  Kept pure so the thresholds are provable without a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchClassification {
    /// Still inside the long-press window: releasing now is a primary tap.
    PrimaryTap,
    /// Long-press threshold crossed: releasing is a secondary click.
    SecondaryHold,
    /// A second contact arrived inside the two-finger window.
    TwoFingerTap,
    /// Slop breached, or the second contact came too late: no click.
    Cancelled,
}

/// Classify a contact by how long it was held, how far it travelled and when a
/// second contact arrived. A slop breach outranks every other classification
/// (it is the recogniser's cancel condition); a timely second contact is a
/// two-finger secondary tap; otherwise the long-press threshold decides.
pub fn classify_touch(
    held_ms: u32,
    travel_px: f64,
    second_contact_ms: Option<u32>,
) -> TouchClassification {
    if travel_px > TOUCH_GESTURE_SLOP_PX {
        return TouchClassification::Cancelled;
    }
    if let Some(ms) = second_contact_ms {
        return if ms <= TOUCH_TWO_FINGER_TAP_MS {
            TouchClassification::TwoFingerTap
        } else {
            TouchClassification::Cancelled
        };
    }
    if held_ms >= TOUCH_LONG_PRESS_MS {
        TouchClassification::SecondaryHold
    } else {
        TouchClassification::PrimaryTap
    }
}

/// A gesture after the recogniser has arbitrated the contacts. `PressDown` and
/// `PressUp` are presentation-only: they must never carry a command.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TouchGesture {
    PrimaryTap {
        x: f32,
        y: f32,
    },
    /// Long-press release — secondary click.
    LongPress {
        x: f32,
        y: f32,
    },
    /// Two-finger tap — secondary click.
    TwoFingerTap {
        x: f32,
        y: f32,
    },
    /// Long-press then movement — secondary drag has started; the recogniser
    /// emits the matching move/up events separately.
    LongDragStart {
        x: f32,
        y: f32,
    },
    /// HUD quick-group button: store the current selection in a slot (1-9).
    AssignQuickGroup {
        slot: u8,
    },
    /// HUD quick-group button: select the stored slot (1-9).
    RecallQuickGroup {
        slot: u8,
    },
    /// Finger down, for press feedback only.
    PressDown {
        contact: u64,
        x: f32,
        y: f32,
    },
    /// Finger up, for press feedback only.
    PressUp {
        contact: u64,
        x: f32,
        y: f32,
    },
}

/// Sticky HUD modifier state consumed alongside a touch click. This is the
/// touch counterpart of physical Shift; it is *not* a persisted input mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TouchModifierLatch {
    pub add_to_selection: bool,
}

/// Touch-only quick-group modifier. The HUD control cycles this mode on a
/// short tap; a latched portrait tap then assigns or recalls that slot. It is
/// host presentation state — never persisted, replayed, or sent to the engine
/// as a modifier, and deliberately independent of physical keyboard modifiers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TouchQuickGroupMode {
    #[default]
    Off,
    Assign,
    Recall,
}

impl TouchQuickGroupMode {
    /// One tap advances `off → assign → recall → off`.
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Assign,
            Self::Assign => Self::Recall,
            Self::Recall => Self::Off,
        }
    }

    pub const fn is_active(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// One typed output of touch mapping. Pointer actions carry an explicit button
/// number so the existing right-button handlers need no new code path.
#[derive(Debug, Clone, PartialEq)]
pub enum TouchMappedAction {
    PointerDown {
        button: u8,
        x: f32,
        y: f32,
    },
    PointerUp {
        button: u8,
        x: f32,
        y: f32,
    },
    /// Shift-equivalent latch edge, emitted around a click so click resolution
    /// sees add-to-selection while it runs.
    ShiftLatch(bool),
    QuickGroup {
        slot: u8,
        assign: bool,
    },
    /// Presentation-only pressed state for a HUD widget; no command.
    PressFeedback {
        contact: u64,
        x: f32,
        y: f32,
        pressed: bool,
    },
}

/// Map one classified touch gesture into typed actions for the existing input
/// handlers. A latched add-to-selection wraps a primary click in Shift edges;
/// secondary gestures deliberately ignore the latch (cancel is unconditional).
/// Out-of-range quick-group slots produce no action rather than a synthetic
/// command.
pub fn map_touch_gesture(
    gesture: TouchGesture,
    latch: &TouchModifierLatch,
) -> Vec<TouchMappedAction> {
    let pointer_click = |button: u8, x: f32, y: f32| {
        vec![
            TouchMappedAction::PointerDown { button, x, y },
            TouchMappedAction::PointerUp { button, x, y },
        ]
    };
    match gesture {
        TouchGesture::PrimaryTap { x, y } => {
            let mut actions = Vec::with_capacity(4);
            if latch.add_to_selection {
                actions.push(TouchMappedAction::ShiftLatch(true));
            }
            actions.extend(pointer_click(TOUCH_PRIMARY_BUTTON, x, y));
            if latch.add_to_selection {
                actions.push(TouchMappedAction::ShiftLatch(false));
            }
            actions
        }
        TouchGesture::LongPress { x, y } | TouchGesture::TwoFingerTap { x, y } => {
            pointer_click(TOUCH_SECONDARY_BUTTON, x, y)
        }
        TouchGesture::LongDragStart { x, y } => {
            vec![TouchMappedAction::PointerDown {
                button: TOUCH_SECONDARY_BUTTON,
                x,
                y,
            }]
        }
        TouchGesture::AssignQuickGroup { slot } | TouchGesture::RecallQuickGroup { slot }
            if (TOUCH_QUICK_GROUP_MIN..=TOUCH_QUICK_GROUP_MAX).contains(&slot) =>
        {
            vec![TouchMappedAction::QuickGroup {
                slot,
                assign: matches!(gesture, TouchGesture::AssignQuickGroup { .. }),
            }]
        }
        TouchGesture::AssignQuickGroup { .. } | TouchGesture::RecallQuickGroup { .. } => Vec::new(),
        TouchGesture::PressDown { contact, x, y } => {
            vec![TouchMappedAction::PressFeedback {
                contact,
                x,
                y,
                pressed: true,
            }]
        }
        TouchGesture::PressUp { contact, x, y } => {
            vec![TouchMappedAction::PressFeedback {
                contact,
                x,
                y,
                pressed: false,
            }]
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use winit::keyboard::KeyCode;

    fn make_translator() -> InputTranslator {
        InputTranslator::new(1024.0, 768.0, &KeyConfig::default_preset())
    }

    fn keys_down(keys: &[KeyCode]) -> BTreeSet<KeyCode> {
        keys.iter().copied().collect()
    }

    #[test]
    fn translate_key_returns_bound_game_key() {
        let t = make_translator();
        assert_eq!(t.translate_key(KeyCode::NumpadAdd), Some(GameKey::ZoomIn));
        assert_eq!(
            t.translate_key(KeyCode::NumpadSubtract),
            Some(GameKey::ZoomOut)
        );
        assert_eq!(
            t.translate_key(KeyCode::Digit1),
            Some(GameKey::SelectCharacter1)
        );
    }

    #[test]
    fn rotation_keys_never_enter_the_action_stream() {
        let mut t = make_translator();
        for key in [
            KeyCode::BracketLeft,
            KeyCode::BracketRight,
            KeyCode::Backslash,
        ] {
            assert_eq!(
                t.translate_key(key),
                None,
                "rotation keys are physical edges, not bindable GameKeys"
            );
            let pressed = keys_down(&[key]);
            assert!(!t.was_key_released(key, &pressed));
            assert!(
                t.translate_keyboard(&pressed, TranslationFlags::ALL)
                    .is_empty()
            );
            assert!(t.was_key_released(key, &BTreeSet::new()));
            assert!(
                t.translate_keyboard(&BTreeSet::new(), TranslationFlags::ALL)
                    .is_empty()
            );
        }
    }

    #[test]
    fn translate_key_returns_none_for_unbound() {
        let t = make_translator();
        assert_eq!(t.translate_key(KeyCode::F24), None);
    }

    #[rstest::rstest]
    #[case::left(0.0, 400.0, GameAction::ScrollLeft)]
    #[case::right(1023.0, 400.0, GameAction::ScrollRight)]
    #[case::up(500.0, 0.0, GameAction::ScrollUp)]
    #[case::down(500.0, 767.0, GameAction::ScrollDown)]
    fn mouse_edge_scroll(#[case] x: f32, #[case] y: f32, #[case] expected: GameAction) {
        let t = make_translator();
        assert!(t.translate_mouse(x, y, 0).contains(&expected));
    }

    #[test]
    fn free_cursor_disables_only_edge_scrolling() {
        let mut t = make_translator();
        t.set_edge_scrolling(false);
        assert_eq!(t.translate_mouse(0.0, 0.0, 1), vec![GameAction::ZoomIn]);
        assert_eq!(
            t.translate_mouse(1023.0, 767.0, -1),
            vec![GameAction::ZoomOut]
        );
        assert!(t.translate_mouse(0.0, 400.0, 0).is_empty());
        t.set_edge_scrolling(true);
        assert!(
            t.translate_mouse(0.0, 400.0, 0)
                .contains(&GameAction::ScrollLeft)
        );
    }

    #[test]
    fn mouse_center_no_scroll() {
        let t = make_translator();
        let actions = t.translate_mouse(500.0, 400.0, 0);
        assert!(actions.is_empty());
    }

    #[test]
    fn mouse_wheel_zoom() {
        let t = make_translator();
        assert!(
            t.translate_mouse(500.0, 400.0, 1)
                .contains(&GameAction::ZoomIn)
        );
        assert!(
            t.translate_mouse(500.0, 400.0, -1)
                .contains(&GameAction::ZoomOut)
        );
    }

    #[test]
    fn mouse_locked_suppresses_all() {
        let mut t = make_translator();
        t.set_user_locked(true);
        let actions = t.translate_mouse(0.0, 0.0, 5);
        assert!(actions.is_empty());
    }

    #[test]
    fn mouse_dead_zone_suppresses_scroll() {
        let mut t = make_translator();
        t.add_dead_zone(ScreenPoint::new(0.0, 350.0), ScreenPoint::new(50.0, 450.0));
        // Point (0, 400) is in the dead zone → no scroll
        let actions = t.translate_mouse(0.0, 400.0, 0);
        assert!(!actions.contains(&GameAction::ScrollLeft));
    }

    #[test]
    fn mouse_dead_zone_does_not_suppress_wheel() {
        let mut t = make_translator();
        t.add_dead_zone(ScreenPoint::new(0.0, 0.0), ScreenPoint::new(1024.0, 768.0));
        // Wheel still works even inside dead zone
        let actions = t.translate_mouse(500.0, 400.0, 3);
        assert!(actions.contains(&GameAction::ZoomIn));
    }

    #[test]
    fn keyboard_released_triggers_action() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::NumpadAdd]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);

        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::ZoomIn));
    }

    #[test]
    fn reveal_hidden_uses_a_distinct_key_from_the_masked_display_switch() {
        let mut t = make_translator();
        let reveal = t.get_binding(GameKey::RevealHiddenCharacters);
        let masked = t.get_binding(GameKey::SwitchHiddenDisplay);
        assert_eq!(reveal, Some(KeyCode::KeyO));
        assert_ne!(reveal, masked, "CapsLock must stay on draw_hidden");

        let pressed = keys_down(&[KeyCode::KeyO]);
        assert!(
            !t.translate_keyboard(&pressed, TranslationFlags::ALL)
                .contains(&GameAction::RevealHiddenCharacters)
        );
        assert!(
            t.translate_keyboard(&BTreeSet::new(), TranslationFlags::ALL)
                .contains(&GameAction::RevealHiddenCharacters)
        );
    }

    #[test]
    fn cloak_key_is_rebindable_and_release_triggered() {
        let mut t = make_translator();
        let pressed = keys_down(&[KeyCode::KeyV]);
        assert!(
            !t.translate_keyboard(&pressed, TranslationFlags::ALL)
                .contains(&GameAction::ToggleCloak)
        );
        assert!(
            t.translate_keyboard(&BTreeSet::new(), TranslationFlags::ALL)
                .contains(&GameAction::ToggleCloak)
        );
    }

    #[test]
    fn raw_key_pressed_is_edge_triggered() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::Period]);
        assert!(t.was_key_pressed(KeyCode::Period, &frame1));

        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(!t.was_key_pressed(KeyCode::Period, &frame1));
    }

    #[test]
    fn keyboard_held_scroll() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::ArrowLeft]);
        let actions = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::ScrollLeft));

        // Frame 2: still holding → still scrolling
        let actions = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::ScrollLeft));
    }

    #[test]
    fn keyboard_show_doors_hit_and_release() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::ShiftLeft]);
        let actions = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::KeyShift));
        assert!(!actions.contains(&GameAction::KeyReleaseShift));

        // Frame 2: released → KeyReleaseShift
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::KeyReleaseShift));
        assert!(!actions.contains(&GameAction::KeyShift));
    }

    #[test]
    fn keyboard_flags_filter_categories() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::F1]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);

        // Release with QUICK_SAVE disabled → no action
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::MISSION);
        assert!(!actions.contains(&GameAction::QuickSave));
    }

    #[test]
    fn keyboard_user_locked_blocks_mission_keys() {
        let mut t = make_translator();
        t.set_user_locked(true);
        let frame1 = keys_down(&[KeyCode::KeyQ]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(!actions.contains(&GameAction::SelectAll));
    }

    #[test]
    fn overlapping_slot_bindings_emit_every_index_in_action_then_portrait_order() {
        for flags in [TranslationFlags::ALL, TranslationFlags::empty()] {
            for locked in [false, true] {
                let mut translator = make_translator();
                for (_, binding) in translator.bindings.iter_mut() {
                    *binding = None;
                }
                for key in [
                    GameKey::Action1,
                    GameKey::Action2,
                    GameKey::Action3,
                    GameKey::SelectCharacter1,
                    GameKey::SelectCharacter2,
                    GameKey::SelectCharacter3,
                    GameKey::SelectCharacter4,
                    GameKey::SelectCharacter5,
                ] {
                    translator.set_binding(key, Some(KeyCode::KeyG));
                }
                translator.set_user_locked(locked);
                let held = keys_down(&[KeyCode::KeyG]);
                assert!(translator.translate_keyboard(&held, flags).is_empty());
                assert!(translator.translate_keyboard(&held, flags).is_empty());
                let expected = if !locked && flags.contains(TranslationFlags::MISSION) {
                    vec![
                        GameAction::SelectAction { index: 0 },
                        GameAction::SelectAction { index: 1 },
                        GameAction::SelectAction { index: 2 },
                        GameAction::SelectCharacter { portrait_index: 0 },
                        GameAction::SelectCharacter { portrait_index: 1 },
                        GameAction::SelectCharacter { portrait_index: 2 },
                        GameAction::SelectCharacter { portrait_index: 3 },
                        GameAction::SelectCharacter { portrait_index: 4 },
                    ]
                } else {
                    Vec::new()
                };
                assert_eq!(
                    translator.translate_keyboard(&BTreeSet::new(), flags),
                    expected
                );
                assert!(
                    translator
                        .translate_keyboard(&BTreeSet::new(), flags)
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn keyboard_select_character() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::Digit1]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::SelectCharacter { portrait_index: 0 }));
    }

    #[test]
    fn keyboard_select_action_index() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::KeyG]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::SelectAction { index: 0 }));
    }

    #[test]
    fn reset_state_clears_previous_keys() {
        let mut t = make_translator();
        let frame1 = keys_down(&[KeyCode::NumpadAdd]);
        let _ = t.translate_keyboard(&frame1, TranslationFlags::ALL);

        t.reset_state();

        // After reset, releasing key 10 should NOT trigger (prev is cleared)
        let frame2 = keys_down(&[]);
        let actions = t.translate_keyboard(&frame2, TranslationFlags::ALL);
        assert!(!actions.contains(&GameAction::ZoomIn));
    }

    #[test]
    fn clear_dead_zones() {
        let mut t = make_translator();
        t.add_dead_zone(ScreenPoint::new(0.0, 0.0), ScreenPoint::new(100.0, 100.0));
        assert!(!t.dead_zones.is_empty());
        t.clear_dead_zones();
        assert!(t.dead_zones.is_empty());
    }

    #[test]
    fn load_bindings_from_keyconfig() {
        let mut t = InputTranslator::new(1024.0, 768.0, &KeyConfig::default_preset());
        let mut cfg = KeyConfig::default();
        cfg.set_binding("ZoomIn", Some(KeyCode::PageUp), None);
        cfg.set_binding("ScrollUp", Some(KeyCode::ArrowUp), None);

        t.load_bindings_from_keyconfig(&cfg);

        assert_eq!(t.get_binding(GameKey::ZoomIn), Some(KeyCode::PageUp));
        assert_eq!(t.get_binding(GameKey::ScrollUp), Some(KeyCode::ArrowUp));
        // Reserved bindings survive
        assert_eq!(t.get_binding(GameKey::DisplayMenu), Some(KeyCode::Escape));
        assert_eq!(t.get_binding(GameKey::SherwoodTrading), Some(KeyCode::KeyT));
    }

    #[test]
    fn p_key_is_reserved_for_coop_pause_without_touching_esc_modal() {
        let mut t = InputTranslator::new(1024.0, 768.0, &KeyConfig::default_preset());
        assert_eq!(t.get_binding(GameKey::CoopPause), Some(KeyCode::KeyP));
        assert_eq!(t.get_binding(GameKey::DisplayMenu), Some(KeyCode::Escape));

        let pressed = keys_down(&[KeyCode::KeyP]);
        assert!(
            !t.translate_keyboard(&pressed, TranslationFlags::ALL)
                .contains(&GameAction::CoopPause)
        );
        let released = keys_down(&[]);
        let actions = t.translate_keyboard(&released, TranslationFlags::ALL);
        assert!(actions.contains(&GameAction::CoopPause));
        assert!(!actions.contains(&GameAction::DisplayMenu));
    }

    #[test]
    fn built_in_t_shortcut_emits_typed_trading_action_on_release() {
        let mut t = InputTranslator::new(1024.0, 768.0, &KeyConfig::default_preset());

        let pressed = keys_down(&[KeyCode::KeyT]);
        assert!(
            !t.translate_keyboard(&pressed, TranslationFlags::ALL)
                .contains(&GameAction::OpenSherwoodTrading)
        );

        let released = keys_down(&[]);
        assert!(
            t.translate_keyboard(&released, TranslationFlags::ALL)
                .contains(&GameAction::OpenSherwoodTrading)
        );
    }

    #[test]
    fn custom_binding_on_t_takes_precedence_over_trading_shortcut() {
        let mut t = InputTranslator::new(1024.0, 768.0, &KeyConfig::default_preset());
        t.set_binding(GameKey::Action1, Some(KeyCode::KeyT));

        let pressed = keys_down(&[KeyCode::KeyT]);
        let _ = t.translate_keyboard(&pressed, TranslationFlags::ALL);
        let actions = t.translate_keyboard(&keys_down(&[]), TranslationFlags::ALL);

        assert!(actions.contains(&GameAction::SelectAction { index: 0 }));
        assert!(!actions.contains(&GameAction::OpenSherwoodTrading));
    }

    #[test]
    fn enum_map_order_matches_persisted_slots_and_reserved_keys_are_not_loaded() {
        let mut custom = KeyConfig::default_preset();
        custom.set_binding("ZoomIn", None, None);
        custom.set_binding("ScrollUp", Some(KeyCode::KeyW), None);
        for config in [
            KeyConfig::default_preset(),
            KeyConfig::alternate_preset(),
            custom,
            KeyConfig::default(),
        ] {
            let mut translator = InputTranslator::new(1024.0, 768.0, &config);
            for (key, binding) in translator.bindings.iter().take(REAL_KEY_COUNT as usize) {
                assert_eq!(*binding, config.get_key_by_index(key as u16));
            }
            assert_eq!(
                translator.get_binding(GameKey::DisplayMenu),
                Some(KeyCode::Escape)
            );
            assert_eq!(
                translator.get_binding(GameKey::DisplayConsole),
                Some(KeyCode::Backquote)
            );
            assert_eq!(
                translator.get_binding(GameKey::SlowMotion),
                Some(KeyCode::Pause)
            );
            let reserved_before: Vec<_> = translator
                .bindings
                .iter()
                .skip(REAL_KEY_COUNT as usize)
                .map(|(key, binding)| (key, *binding))
                .collect();
            translator.load_bindings_from_keyconfig(&config);
            for (index, (key, binding)) in translator.bindings.iter().enumerate() {
                assert_eq!(key as usize, index);
                if index < REAL_KEY_COUNT as usize {
                    assert_eq!(*binding, config.get_key_by_index(index as u16));
                }
            }
            let reserved_after: Vec<_> = translator
                .bindings
                .iter()
                .skip(REAL_KEY_COUNT as usize)
                .map(|(key, binding)| (key, *binding))
                .collect();
            assert_eq!(reserved_after, reserved_before);
        }
    }

    #[test]
    fn reverse_lookup_keeps_enum_order_for_shared_bindings() {
        let mut translator = InputTranslator::default();
        translator.set_binding(GameKey::PlanQuickActions, Some(KeyCode::ShiftLeft));
        translator.set_binding(GameKey::ShowDoors, Some(KeyCode::ShiftLeft));
        assert_eq!(
            translator.translate_key(KeyCode::ShiftLeft),
            Some(GameKey::ShowDoors)
        );
        translator.set_binding(GameKey::ShowDoors, None);
        assert_eq!(
            translator.translate_key(KeyCode::ShiftLeft),
            Some(GameKey::PlanQuickActions)
        );
    }

    #[test]
    fn planning_binding_is_rebindable_and_held_independently() {
        let mut translator = InputTranslator::new(1024.0, 768.0, &KeyConfig::default_preset());
        translator.set_binding(GameKey::PlanQuickActions, Some(KeyCode::KeyP));
        assert!(
            translator.is_binding_held(GameKey::PlanQuickActions, &keys_down(&[KeyCode::KeyP]))
        );
        assert!(
            !translator
                .is_binding_held(GameKey::PlanQuickActions, &keys_down(&[KeyCode::ShiftLeft]))
        );
    }

    #[test]
    fn translation_flags_combine() {
        let flags = TranslationFlags::QUICK_LOAD | TranslationFlags::MISSION;
        assert!(flags.contains(TranslationFlags::QUICK_LOAD));
        assert!(flags.contains(TranslationFlags::MISSION));
        assert!(!flags.contains(TranslationFlags::QUICK_SAVE));
        assert!(!flags.contains(TranslationFlags::INGAME_MENU));
    }

    #[test]
    fn serde_round_trip_translator() {
        let mut t = make_translator();
        t.add_dead_zone(ScreenPoint::new(10.0, 20.0), ScreenPoint::new(30.0, 40.0));
        let json = serde_json::to_string(&t).unwrap();
        let back: InputTranslator = serde_json::from_str(&json).unwrap();
        assert_eq!(back.screen_width, 1024.0);
        assert_eq!(back.get_binding(GameKey::ZoomIn), Some(KeyCode::NumpadAdd));
        assert_eq!(back.dead_zones.len(), 1);
    }

    // --- Touch mapping fixtures (issues #57/#58) -----------------------

    #[rstest::rstest]
    #[case::short_tap(100, 0.0, None, TouchClassification::PrimaryTap)]
    #[case::long_press(TOUCH_LONG_PRESS_MS, 0.0, None, TouchClassification::SecondaryHold)]
    #[case::longer_press(900, 0.0, None, TouchClassification::SecondaryHold)]
    #[case::slop_breach(100, TOUCH_GESTURE_SLOP_PX + 0.5, None, TouchClassification::Cancelled)]
    #[case::two_finger_tap(
        100,
        0.0,
        Some(TOUCH_TWO_FINGER_TAP_MS),
        TouchClassification::TwoFingerTap
    )]
    #[case::late_second_contact(100, 0.0, Some(TOUCH_TWO_FINGER_TAP_MS + 1), TouchClassification::Cancelled)]
    fn touch_timing_classification(
        #[case] held_ms: u32,
        #[case] travel_px: f64,
        #[case] second_contact_ms: Option<u32>,
        #[case] expected: TouchClassification,
    ) {
        assert_eq!(
            classify_touch(held_ms, travel_px, second_contact_ms),
            expected
        );
    }

    #[test]
    fn long_press_maps_to_secondary_button_down_and_up() {
        let actions = map_touch_gesture(
            TouchGesture::LongPress { x: 12.0, y: 34.0 },
            &TouchModifierLatch::default(),
        );
        assert_eq!(
            actions,
            vec![
                TouchMappedAction::PointerDown {
                    button: TOUCH_SECONDARY_BUTTON,
                    x: 12.0,
                    y: 34.0,
                },
                TouchMappedAction::PointerUp {
                    button: TOUCH_SECONDARY_BUTTON,
                    x: 12.0,
                    y: 34.0,
                },
            ]
        );
    }

    #[test]
    fn two_finger_tap_maps_to_secondary_button() {
        let actions = map_touch_gesture(
            TouchGesture::TwoFingerTap { x: 5.0, y: 6.0 },
            &TouchModifierLatch::default(),
        );
        assert!(actions.iter().all(|a| match a {
            TouchMappedAction::PointerDown { button, .. }
            | TouchMappedAction::PointerUp { button, .. } => *button == TOUCH_SECONDARY_BUTTON,
            _ => false,
        }));
        assert_eq!(actions.len(), 2);
    }

    #[test]
    fn long_drag_starts_a_secondary_drag() {
        let actions = map_touch_gesture(
            TouchGesture::LongDragStart { x: 7.0, y: 8.0 },
            &TouchModifierLatch::default(),
        );
        assert_eq!(
            actions,
            vec![TouchMappedAction::PointerDown {
                button: TOUCH_SECONDARY_BUTTON,
                x: 7.0,
                y: 8.0,
            }]
        );
    }

    #[test]
    fn plain_tap_stays_on_the_primary_button() {
        let actions = map_touch_gesture(
            TouchGesture::PrimaryTap { x: 1.0, y: 2.0 },
            &TouchModifierLatch::default(),
        );
        assert_eq!(
            actions,
            vec![
                TouchMappedAction::PointerDown {
                    button: TOUCH_PRIMARY_BUTTON,
                    x: 1.0,
                    y: 2.0,
                },
                TouchMappedAction::PointerUp {
                    button: TOUCH_PRIMARY_BUTTON,
                    x: 1.0,
                    y: 2.0,
                },
            ]
        );
    }

    #[test]
    fn latched_add_to_selection_wraps_the_primary_click() {
        let latch = TouchModifierLatch {
            add_to_selection: true,
        };
        let actions = map_touch_gesture(TouchGesture::PrimaryTap { x: 3.0, y: 4.0 }, &latch);
        assert_eq!(
            actions,
            vec![
                TouchMappedAction::ShiftLatch(true),
                TouchMappedAction::PointerDown {
                    button: TOUCH_PRIMARY_BUTTON,
                    x: 3.0,
                    y: 4.0,
                },
                TouchMappedAction::PointerUp {
                    button: TOUCH_PRIMARY_BUTTON,
                    x: 3.0,
                    y: 4.0,
                },
                TouchMappedAction::ShiftLatch(false),
            ]
        );
    }

    #[test]
    fn quick_group_gestures_emit_typed_assign_and_recall_commands() {
        let latch = TouchModifierLatch::default();
        assert_eq!(
            map_touch_gesture(TouchGesture::AssignQuickGroup { slot: 3 }, &latch),
            vec![TouchMappedAction::QuickGroup {
                slot: 3,
                assign: true,
            }]
        );
        assert_eq!(
            map_touch_gesture(TouchGesture::RecallQuickGroup { slot: 7 }, &latch),
            vec![TouchMappedAction::QuickGroup {
                slot: 7,
                assign: false,
            }]
        );
    }

    #[rstest::rstest]
    #[case::zero(0)]
    #[case::too_high(TOUCH_QUICK_GROUP_MAX + 1)]
    fn quick_group_slot_out_of_range_is_ignored(#[case] slot: u8) {
        let actions =
            map_touch_gesture(TouchGesture::AssignQuickGroup { slot }, &Default::default());
        assert!(actions.is_empty());
    }

    #[test]
    fn touch_quick_group_mode_cycles_off_assign_recall_off() {
        use crate::input_translator::TouchQuickGroupMode;
        use TouchQuickGroupMode::*;

        let mut mode = Off;
        assert_eq!(mode, Off);
        mode = mode.next();
        assert_eq!(mode, Assign, "first tap arms assignment");
        mode = mode.next();
        assert_eq!(mode, Recall, "second tap arms recall");
        mode = mode.next();
        assert_eq!(mode, Off, "third tap returns to plain selection");
        assert!(!Off.is_active());
        assert!(Assign.is_active());
        assert!(Recall.is_active());
    }

    #[test]
    fn press_feedback_is_presentation_only() {
        let down = map_touch_gesture(
            TouchGesture::PressDown {
                contact: 9,
                x: 20.0,
                y: 21.0,
            },
            &TouchModifierLatch {
                add_to_selection: true,
            },
        );
        assert_eq!(
            down,
            vec![TouchMappedAction::PressFeedback {
                contact: 9,
                x: 20.0,
                y: 21.0,
                pressed: true,
            }]
        );
        let up = map_touch_gesture(
            TouchGesture::PressUp {
                contact: 9,
                x: 20.0,
                y: 21.0,
            },
            &TouchModifierLatch::default(),
        );
        assert_eq!(
            up,
            vec![TouchMappedAction::PressFeedback {
                contact: 9,
                x: 20.0,
                y: 21.0,
                pressed: false,
            }]
        );
    }
}
