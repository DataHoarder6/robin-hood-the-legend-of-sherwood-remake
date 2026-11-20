//! Presentation-only store for teammates' cursors.
//!
//! Samples arrive from the host relay with a receive timestamp; rendering reads
//! a buffered point 120 ms in the past and interpolates between real samples.
//! Nothing here enters simulation, replay, rollback, or state hashes.

use std::collections::{BTreeMap, VecDeque};

use super::engine_coordinates::{MapPoint, ScreenPoint};
use robin_assets::resource_manager::ResourceManager;
use robin_engine::multiplayer::CursorWireState;
use robin_engine::player_command::PlayerId;

use crate::cursor::CursorRenderer;
use crate::host::Host;
use crate::host::ViewportState;
use crate::renderer::Renderer;

/// Colours for the five possible co-op seats, shared with the local
/// split-screen cursor markers so a seat keeps one identity everywhere.
pub(crate) const SEAT_COLORS: [[u8; 3]; 5] = [
    [100, 210, 255],
    [255, 180, 80],
    [130, 240, 130],
    [240, 130, 235],
    [255, 235, 100],
];

pub(crate) const CURSOR_SEND_INTERVAL_MS: u64 = 40;
pub(crate) const CURSOR_INTERPOLATION_DELAY_MS: u64 = 120;
pub(crate) const CURSOR_STALE_MS: u64 = 300;
pub(crate) const REMOTE_CURSOR_SCALE: f32 = 0.7;

const MAX_SAMPLES_PER_SEAT: usize = 8;

#[derive(Default)]
pub(crate) struct RemoteCursorStore {
    samples: BTreeMap<u8, VecDeque<(u64, CursorWireState)>>,
    renderers: BTreeMap<u8, CursorRenderer>,
    loaded_sprite: BTreeMap<u8, i32>,
}

#[derive(Default)]
pub(crate) struct LocalCursorTx {
    pub last_sent_ms: u64,
}

impl RemoteCursorStore {
    pub(crate) fn ingest(&mut self, now_ms: u64, seat: PlayerId, state: CursorWireState) {
        let samples = self.samples.entry(seat.0).or_default();
        while samples.len() >= MAX_SAMPLES_PER_SEAT {
            samples.pop_front();
        }
        samples.push_back((now_ms, state));
    }

    pub(crate) fn forget(&mut self, seat: PlayerId) {
        self.samples.remove(&seat.0);
        self.renderers.remove(&seat.0);
        self.loaded_sprite.remove(&seat.0);
    }

    /// Interpolated cursor states for every non-stale seat at `now - 120 ms`.
    pub(crate) fn render_states(&self, now_ms: u64) -> Vec<(PlayerId, CursorWireState)> {
        let target = now_ms.saturating_sub(CURSOR_INTERPOLATION_DELAY_MS);
        let mut out = Vec::new();
        for (&seat, samples) in &self.samples {
            let Some(&(newest_at, newest)) = samples.back() else {
                continue;
            };
            if now_ms.saturating_sub(newest_at) > CURSOR_STALE_MS {
                continue;
            }
            let older = samples
                .iter()
                .take_while(|(at, _)| *at <= target)
                .last()
                .map(|(_, state)| *state);
            let newer = samples
                .iter()
                .find(|(at, _)| *at >= target)
                .map(|(_, state)| *state);
            let Some(older) = older else {
                // No sample at or before the buffered point yet: show the most
                // recent real sample, never a predicted one.
                out.push((PlayerId(seat), newest));
                continue;
            };
            let Some(newer) = newer else {
                out.push((PlayerId(seat), older));
                continue;
            };
            if older.map_x == newer.map_x
                && older.map_y == newer.map_y
                && older.sprite_id == newer.sprite_id
            {
                out.push((PlayerId(seat), older));
                continue;
            }
            let older_at = samples
                .iter()
                .take_while(|(at, _)| *at <= target)
                .map(|(at, _)| *at)
                .last()
                .unwrap_or(target);
            let newer_at = samples
                .iter()
                .find(|(at, _)| *at >= target)
                .map(|(at, _)| *at)
                .unwrap_or(target);
            let span = newer_at.saturating_sub(older_at);
            let alpha = if span == 0 {
                0.0
            } else {
                (target.saturating_sub(older_at)) as f32 / span as f32
            };
            out.push((
                PlayerId(seat),
                CursorWireState {
                    map_x: older.map_x + (newer.map_x - older.map_x) * alpha,
                    map_y: older.map_y + (newer.map_y - older.map_y) * alpha,
                    sprite_id: newer.sprite_id,
                },
            ));
        }
        out
    }

    pub(crate) fn sync_sprites(
        &mut self,
        resources: &mut ResourceManager,
        renderer: &Renderer,
        states: &[(PlayerId, CursorWireState)],
    ) {
        for (seat, state) in states {
            if self.loaded_sprite.get(&seat.0) == Some(&state.sprite_id) {
                continue;
            }
            let cursor = self.renderers.entry(seat.0).or_default();
            cursor.load_cursor(state.sprite_id, resources, renderer);
            self.loaded_sprite.insert(seat.0, state.sprite_id);
        }
    }

    pub(crate) fn draw(
        &self,
        renderer: &mut Renderer,
        viewport: &ViewportState,
        states: &[(PlayerId, CursorWireState)],
    ) {
        for (seat, state) in states {
            let Some(cursor) = self.renderers.get(&seat.0) else {
                continue;
            };
            let Some(screen) = cursor_screen_point(viewport, state) else {
                continue;
            };
            cursor.render_tinted_scaled(
                renderer,
                screen.x,
                screen.y,
                REMOTE_CURSOR_SCALE,
                tint_for_seat(*seat),
            );
        }
    }
}

/// Convert a teammate's world-space cursor into this client's screen space.
pub(crate) fn cursor_screen_point(
    viewport: &ViewportState,
    state: &CursorWireState,
) -> Option<ScreenPoint> {
    viewport.map_to_screen(MapPoint::new(state.map_x, state.map_y))
}

pub(crate) fn tint_for_seat(seat: PlayerId) -> [f32; 4] {
    let [r, g, b] = SEAT_COLORS[usize::from(seat.0) % SEAT_COLORS.len()];
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

pub(crate) fn remote_cursors_visible(
    pref: bool,
    pause: bool,
    ui_task: bool,
    user_locked: bool,
    console: bool,
) -> bool {
    pref && !pause && !ui_task && !user_locked && !console
}

/// Broadcast this seat's cursor at most every `CURSOR_SEND_INTERVAL_MS`.
pub(crate) fn publish_local_cursor(
    host: &mut Host,
    now_ms: u64,
    map_point: MapPoint,
    sprite_id: i32,
) {
    if !host.frontend.preferences().show_teammate_cursors() {
        return;
    }
    let elapsed = now_ms.saturating_sub(host.frontend.local_cursor_tx.last_sent_ms);
    if elapsed < CURSOR_SEND_INTERVAL_MS {
        return;
    }
    let Some(net) = host.transport.net() else {
        return;
    };
    let state = CursorWireState {
        map_x: map_point.x,
        map_y: map_point.y,
        sprite_id,
    };
    let message = crate::multiplayer::NetOutbound::Cursor {
        seat: host.transport.local_seat(),
        state,
    };
    if let Err(error) = net.outgoing.send(message) {
        tracing::warn!(%error, "teammate cursor broadcast failed");
    }
    host.frontend.local_cursor_tx.last_sent_ms = now_ms;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(x: f32) -> CursorWireState {
        CursorWireState {
            map_x: x,
            map_y: 0.0,
            sprite_id: 1,
        }
    }

    #[test]
    fn interpolation_uses_buffered_samples_without_extrapolation() {
        let mut store = RemoteCursorStore::default();
        let seat = PlayerId(2);
        store.ingest(1000, seat, sample(0.0));
        store.ingest(1040, seat, sample(40.0));
        store.ingest(1080, seat, sample(80.0));
        let states = store.render_states(1160);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].0, seat);
        assert_eq!(states[0].1.map_x, 40.0);
        let states = store.render_states(1300);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].1.map_x, 80.0);
        assert!(states[0].1.map_x <= 80.0);
    }

    #[test]
    fn stale_seat_is_hidden() {
        let mut store = RemoteCursorStore::default();
        let seat = PlayerId(1);
        store.ingest(1000, seat, sample(10.0));
        assert!(store.render_states(1400).is_empty());
    }

    #[test]
    fn forget_removes_seat() {
        let mut store = RemoteCursorStore::default();
        let seat = PlayerId(1);
        store.ingest(1100, seat, sample(10.0));
        store.forget(seat);
        assert!(store.render_states(1150).is_empty());
    }

    #[test]
    fn seat_colors_are_distinct_and_cover_all_seats() {
        assert_eq!(SEAT_COLORS.len(), 5);
        let unique: std::collections::BTreeSet<_> = SEAT_COLORS.iter().collect();
        assert_eq!(unique.len(), 5);
    }

    #[test]
    fn screen_roundtrip_preserves_world_point_at_two_viewport_sizes() {
        for (width, height) in [(1024.0, 768.0), (1600.0, 900.0)] {
            let mut viewport = ViewportState::new(width, height);
            viewport.set_level_size(4000.0, 4000.0);
            let center = viewport
                .screen_to_map(ScreenPoint::new(width / 2.0, height / 2.0))
                .expect("viewport centre maps into the world");
            let state = CursorWireState {
                map_x: center.x,
                map_y: center.y,
                sprite_id: 1,
            };
            let screen = cursor_screen_point(&viewport, &state).expect("on-screen point");
            let map = viewport.screen_to_map(screen).expect("mapped back");
            assert!((map.x - state.map_x).abs() < 0.01);
            assert!((map.y - state.map_y).abs() < 0.01);
        }
    }

    #[test]
    fn tint_for_seat_uses_the_seat_color() {
        let [r, g, b] = SEAT_COLORS[0];
        assert_eq!(
            tint_for_seat(PlayerId(0)),
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
        );
    }

    #[test]
    fn visibility_gating_hides_on_pause_ui_lock_and_console() {
        assert!(remote_cursors_visible(true, false, false, false, false));
        for flags in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            assert!(!remote_cursors_visible(
                true, flags.0, flags.1, flags.2, flags.3
            ));
        }
        assert!(!remote_cursors_visible(false, false, false, false, false));
    }
}
