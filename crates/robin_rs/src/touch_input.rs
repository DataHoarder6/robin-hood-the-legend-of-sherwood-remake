//! Platform-independent touch gesture classification.
//!
//! Winit reports independent pointer changes, while the game expects either a
//! complete mouse interaction or a camera gesture.  This module owns that
//! arbitration before either kind of event reaches gameplay.  In particular,
//! a pending tap never emits a mouse-down until it is known to be a tap, so a
//! second finger can cancel it without accidentally committing a world/UI
//! action.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

const DEFAULT_TOUCH_SLOP_PX: f64 = 12.0;
const DOUBLE_TAP_INTERVAL_MS: u32 = 300;
const DEFAULT_DOUBLE_TAP_SLOP_PX: f64 = 32.0;
const LONG_PRESS_MS: u32 = 450;
const TWO_FINGER_TAP_MS: u32 = 250;
const VELOCITY_FILTER_NEW_SAMPLE: f64 = 0.35;
const MAX_FLING_SAMPLE_AGE_MS: u32 = 100;
const PRIMARY_BUTTON: u8 = 1;
const SECONDARY_BUTTON: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct TouchPoint {
    start_x: f64,
    start_y: f64,
    x: f64,
    y: f64,
    feedback_active: bool,
}

impl TouchPoint {
    fn new(x: f64, y: f64) -> Self {
        Self {
            start_x: x,
            start_y: y,
            x,
            y,
            feedback_active: true,
        }
    }

    fn current(self) -> (f64, f64) {
        (self.x, self.y)
    }

    fn origin(self) -> (f64, f64) {
        (self.start_x, self.start_y)
    }

    fn moved_beyond(self, slop_px: f64) -> bool {
        distance(self.origin(), self.current()) >= slop_px
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) enum TouchOutput {
    /// A new primary contact immediately stops any camera momentum, before
    /// the contact is release-classified as a tap or drag.
    MotionStop,
    PointerMove {
        x: f64,
        y: f64,
    },
    PointerDown {
        x: f64,
        y: f64,
        button: u8,
        clicks: u8,
    },
    PointerUp {
        x: f64,
        y: f64,
        button: u8,
    },
    /// Abort a mouse drag without running its release/click action.
    PointerCancel,
    /// Presentation-only contact state. It never becomes a gameplay command.
    PressFeedback {
        contact: u64,
        x: f64,
        y: f64,
        pressed: bool,
    },
    TransformStart {
        first_x: f64,
        first_y: f64,
        second_x: f64,
        second_y: f64,
    },
    TransformUpdate {
        centroid_x: f64,
        centroid_y: f64,
        pan_x: f64,
        pan_y: f64,
        scale: f64,
        velocity_x: f64,
        velocity_y: f64,
    },
    TransformEnd {
        velocity_x: f64,
        velocity_y: f64,
        cancelled: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
enum GestureState {
    Idle,
    PendingTap {
        id: u64,
        started_ms: u32,
    },
    /// Two stationary contacts may still become one secondary tap. Movement or
    /// expiry promotes them to the existing camera transform.
    PendingTransform {
        started_ms: u32,
        released: Option<(f64, f64)>,
    },
    PointerDrag {
        id: u64,
        button: u8,
    },
    Transform {
        last_centroid_x: f64,
        last_centroid_y: f64,
        last_distance: f64,
        velocity_sample_centroid_x: f64,
        velocity_sample_centroid_y: f64,
        velocity_sample_ms: u32,
        velocity_x: f64,
        velocity_y: f64,
    },
    /// One finger remains after a transform.  It must be lifted before a new
    /// one-finger interaction can begin; promoting it would create a click at
    /// an unrelated location.
    Suppressed,
}

/// Pure touch recognizer. Timestamps are caller-supplied monotonic
/// milliseconds, which keeps unit tests deterministic and avoids serializing
/// platform `Instant` values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TouchClassifier {
    touches: BTreeMap<u64, TouchPoint>,
    ignored_touches: BTreeSet<u64>,
    state: GestureState,
    last_tap: Option<(u32, f64, f64)>,
    touch_slop_px: f64,
    double_tap_slop_px: f64,
}

impl Default for TouchClassifier {
    fn default() -> Self {
        Self::new(DEFAULT_TOUCH_SLOP_PX)
    }
}

impl TouchClassifier {
    pub(crate) fn new(touch_slop_px: f64) -> Self {
        assert!(touch_slop_px.is_finite() && touch_slop_px > 0.0);
        Self {
            touches: BTreeMap::new(),
            ignored_touches: BTreeSet::new(),
            state: GestureState::Idle,
            last_tap: None,
            touch_slop_px,
            double_tap_slop_px: touch_slop_px * DEFAULT_DOUBLE_TAP_SLOP_PX / DEFAULT_TOUCH_SLOP_PX,
        }
    }

    /// Winit touch locations are physical pixels. Scale the logical gesture
    /// tolerances to match the current monitor density.
    pub(crate) fn set_scale_factor(&mut self, scale_factor: f64) {
        assert!(scale_factor.is_finite() && scale_factor > 0.0);
        self.touch_slop_px = DEFAULT_TOUCH_SLOP_PX * scale_factor;
        self.double_tap_slop_px = DEFAULT_DOUBLE_TAP_SLOP_PX * scale_factor;
    }

    pub(crate) fn started(&mut self, id: u64, x: f64, y: f64, now_ms: u32) -> Vec<TouchOutput> {
        if self.touches.contains_key(&id) || self.ignored_touches.contains(&id) {
            return Vec::new();
        }
        if self.touches.len() >= 2 {
            self.ignored_touches.insert(id);
            return Vec::new();
        }
        self.touches.insert(id, TouchPoint::new(x, y));

        match self.state {
            GestureState::Idle if self.touches.len() == 1 => {
                self.state = GestureState::PendingTap {
                    id,
                    started_ms: now_ms,
                };
                vec![
                    TouchOutput::MotionStop,
                    TouchOutput::PressFeedback {
                        contact: id,
                        x,
                        y,
                        pressed: true,
                    },
                ]
            }
            GestureState::PendingTap { started_ms, .. } if self.touches.len() == 2 => {
                self.last_tap = None;
                if now_ms.wrapping_sub(started_ms) > TWO_FINGER_TAP_MS {
                    self.begin_transform(now_ms)
                } else {
                    self.state = GestureState::PendingTransform {
                        started_ms,
                        released: None,
                    };
                    vec![TouchOutput::PressFeedback {
                        contact: id,
                        x,
                        y,
                        pressed: true,
                    }]
                }
            }
            GestureState::PointerDrag { .. } if self.touches.len() == 2 => {
                let mut output = vec![TouchOutput::PointerCancel];
                output.extend(self.begin_transform(now_ms));
                output
            }
            GestureState::PendingTransform { .. } if self.touches.len() == 2 => {
                self.last_tap = None;
                self.begin_transform(now_ms)
            }
            GestureState::Suppressed => vec![TouchOutput::PressFeedback {
                contact: id,
                x,
                y,
                pressed: true,
            }],
            state => {
                tracing::warn!(
                    ?state,
                    touch_count = self.touches.len(),
                    "unexpected touch start"
                );
                vec![TouchOutput::PressFeedback {
                    contact: id,
                    x,
                    y,
                    pressed: true,
                }]
            }
        }
    }

    pub(crate) fn moved(&mut self, id: u64, x: f64, y: f64, now_ms: u32) -> Vec<TouchOutput> {
        if self.ignored_touches.contains(&id) {
            return Vec::new();
        }
        let Some(point) = self.touches.get_mut(&id) else {
            return Vec::new();
        };
        point.x = x;
        point.y = y;
        match self.state {
            GestureState::PendingTap {
                id: primary,
                started_ms,
            } if primary == id => {
                let point = self
                    .touches
                    .get(&primary)
                    .expect("pending tap lost its contact");
                if !point.moved_beyond(self.touch_slop_px) {
                    return Vec::new();
                }
                let (start_x, start_y) = point.origin();
                let button = if now_ms.wrapping_sub(started_ms) >= LONG_PRESS_MS {
                    SECONDARY_BUTTON
                } else {
                    PRIMARY_BUTTON
                };
                self.state = GestureState::PointerDrag { id, button };
                self.last_tap = None;
                vec![
                    TouchOutput::PointerDown {
                        x: start_x,
                        y: start_y,
                        button,
                        clicks: 1,
                    },
                    TouchOutput::PointerMove { x, y },
                ]
            }
            GestureState::PointerDrag { id: primary, .. } if primary == id => {
                vec![TouchOutput::PointerMove { x, y }]
            }
            GestureState::PendingTransform { started_ms, .. } => {
                let movement_cancels_tap = self
                    .touches
                    .values()
                    .any(|point| point.moved_beyond(self.touch_slop_px));
                if self.touches.len() == 2
                    && (movement_cancels_tap || now_ms.wrapping_sub(started_ms) > TWO_FINGER_TAP_MS)
                {
                    let mut output = self.begin_transform(now_ms);
                    output.extend(self.moved(id, x, y, now_ms));
                    output
                } else if self.touches.len() == 1 && movement_cancels_tap {
                    self.state = GestureState::Suppressed;
                    self.last_tap = None;
                    self.clear_feedback()
                } else {
                    Vec::new()
                }
            }
            GestureState::Transform {
                last_centroid_x,
                last_centroid_y,
                last_distance,
                velocity_sample_centroid_x,
                velocity_sample_centroid_y,
                velocity_sample_ms,
                velocity_x,
                velocity_y,
            } if self.touches.len() == 2 => {
                let (first, second) = self.two_points();
                let (centroid_x, centroid_y, current_distance) = transform_geometry(first, second);
                let pan_x = centroid_x - last_centroid_x;
                let pan_y = centroid_y - last_centroid_y;
                let scale = if last_distance > f64::EPSILON {
                    current_distance / last_distance
                } else {
                    1.0
                };
                let elapsed_ms = now_ms.wrapping_sub(velocity_sample_ms);
                let (
                    velocity_sample_centroid_x,
                    velocity_sample_centroid_y,
                    velocity_sample_ms,
                    velocity_x,
                    velocity_y,
                ) = if elapsed_ms > 0 {
                    let elapsed_ms = f64::from(elapsed_ms);
                    let raw_velocity_x =
                        (centroid_x - velocity_sample_centroid_x) * 1000.0 / elapsed_ms;
                    let raw_velocity_y =
                        (centroid_y - velocity_sample_centroid_y) * 1000.0 / elapsed_ms;
                    (
                        centroid_x,
                        centroid_y,
                        now_ms,
                        velocity_x * (1.0 - VELOCITY_FILTER_NEW_SAMPLE)
                            + raw_velocity_x * VELOCITY_FILTER_NEW_SAMPLE,
                        velocity_y * (1.0 - VELOCITY_FILTER_NEW_SAMPLE)
                            + raw_velocity_y * VELOCITY_FILTER_NEW_SAMPLE,
                    )
                } else {
                    // Winit can batch both contacts' move events under the
                    // same millisecond timestamp. Accumulate their centroid
                    // displacement until time advances instead of treating
                    // the second event as an implausible 1 ms sample.
                    (
                        velocity_sample_centroid_x,
                        velocity_sample_centroid_y,
                        velocity_sample_ms,
                        velocity_x,
                        velocity_y,
                    )
                };
                self.state = GestureState::Transform {
                    last_centroid_x: centroid_x,
                    last_centroid_y: centroid_y,
                    last_distance: current_distance,
                    velocity_sample_centroid_x,
                    velocity_sample_centroid_y,
                    velocity_sample_ms,
                    velocity_x,
                    velocity_y,
                };
                vec![TouchOutput::TransformUpdate {
                    centroid_x,
                    centroid_y,
                    pan_x,
                    pan_y,
                    scale,
                    velocity_x,
                    velocity_y,
                }]
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn ended(
        &mut self,
        id: u64,
        x: f64,
        y: f64,
        now_ms: u32,
        cancelled: bool,
    ) -> Vec<TouchOutput> {
        if self.ignored_touches.remove(&id) {
            if self.touches.is_empty() && self.ignored_touches.is_empty() {
                self.state = GestureState::Idle;
            }
            return Vec::new();
        }
        let Some(previous) = self.touches.get(&id).copied() else {
            return Vec::new();
        };
        let final_position_changed = previous.x != x || previous.y != y;
        let point = self
            .touches
            .get_mut(&id)
            .expect("ending touch disappeared before classification");
        point.x = x;
        point.y = y;
        let point = *point;
        let state = self.state;
        let mut output = if final_position_changed
            && !cancelled
            && matches!(state, GestureState::Transform { .. })
        {
            self.moved(id, x, y, now_ms)
        } else {
            Vec::new()
        };
        output.extend(match state {
            GestureState::PendingTap {
                id: primary,
                started_ms,
            } if primary == id => {
                let mut output = self.release_feedback(id, x, y);
                if cancelled {
                    self.last_tap = None;
                } else if point.moved_beyond(self.touch_slop_px) {
                    // Some platforms coalesce a short drag into Start+End
                    // without an intermediate Move. Preserve release-time
                    // classification by materializing that complete drag now.
                    let (start_x, start_y) = point.origin();
                    let button = if now_ms.wrapping_sub(started_ms) >= LONG_PRESS_MS {
                        SECONDARY_BUTTON
                    } else {
                        PRIMARY_BUTTON
                    };
                    self.last_tap = None;
                    output.extend([
                        TouchOutput::PointerDown {
                            x: start_x,
                            y: start_y,
                            button,
                            clicks: 1,
                        },
                        TouchOutput::PointerMove { x, y },
                        TouchOutput::PointerUp { x, y, button },
                    ]);
                } else {
                    let (button, clicks) = if now_ms.wrapping_sub(started_ms) >= LONG_PRESS_MS {
                        self.last_tap = None;
                        (SECONDARY_BUTTON, 1)
                    } else {
                        (PRIMARY_BUTTON, self.tap_count(now_ms, x, y))
                    };
                    output.extend([
                        TouchOutput::PointerMove { x, y },
                        TouchOutput::PointerDown {
                            x,
                            y,
                            button,
                            clicks,
                        },
                        TouchOutput::PointerUp { x, y, button },
                    ]);
                }
                output
            }
            GestureState::PointerDrag {
                id: primary,
                button,
            } if primary == id => {
                let mut output = self.release_feedback(id, x, y);
                self.last_tap = None;
                if cancelled {
                    output.push(TouchOutput::PointerCancel);
                } else {
                    output.extend([
                        TouchOutput::PointerMove { x, y },
                        TouchOutput::PointerUp { x, y, button },
                    ]);
                }
                output
            }
            GestureState::PendingTransform {
                started_ms,
                released,
            } => {
                if cancelled {
                    self.state = GestureState::Suppressed;
                    self.last_tap = None;
                    self.clear_feedback()
                } else if self.touches.len() == 2 {
                    let movement_cancels_tap = self
                        .touches
                        .values()
                        .any(|point| point.moved_beyond(self.touch_slop_px));
                    if movement_cancels_tap || now_ms.wrapping_sub(started_ms) > TWO_FINGER_TAP_MS {
                        self.last_tap = None;
                        let mut output = self.begin_transform(now_ms);
                        if final_position_changed {
                            output.extend(self.moved(id, x, y, now_ms));
                        }
                        output.push(self.transform_end(now_ms, false));
                        output
                    } else {
                        self.state = GestureState::PendingTransform {
                            started_ms,
                            released: Some((x, y)),
                        };
                        self.release_feedback(id, x, y)
                    }
                } else {
                    let mut output = self.release_feedback(id, x, y);
                    if now_ms.wrapping_sub(started_ms) <= TWO_FINGER_TAP_MS
                        && !point.moved_beyond(self.touch_slop_px)
                    {
                        let (released_x, released_y) =
                            released.expect("two-finger tap lost its first release");
                        let centroid_x = (released_x + x) * 0.5;
                        let centroid_y = (released_y + y) * 0.5;
                        self.last_tap = None;
                        output.extend([
                            TouchOutput::PointerMove {
                                x: centroid_x,
                                y: centroid_y,
                            },
                            TouchOutput::PointerDown {
                                x: centroid_x,
                                y: centroid_y,
                                button: SECONDARY_BUTTON,
                                clicks: 1,
                            },
                            TouchOutput::PointerUp {
                                x: centroid_x,
                                y: centroid_y,
                                button: SECONDARY_BUTTON,
                            },
                        ]);
                    } else {
                        self.last_tap = None;
                    }
                    output
                }
            }
            GestureState::Transform { .. } => {
                self.last_tap = None;
                vec![self.transform_end(now_ms, cancelled)]
            }
            _ => self.release_feedback(id, x, y),
        });
        self.touches.remove(&id);
        self.state = if self.touches.is_empty() && self.ignored_touches.is_empty() {
            GestureState::Idle
        } else if !matches!(self.state, GestureState::PendingTransform { .. }) {
            GestureState::Suppressed
        } else {
            self.state
        };
        output
    }

    pub(crate) fn cancel_all(&mut self) -> Vec<TouchOutput> {
        let mut output = self.clear_feedback();
        match self.state {
            GestureState::PointerDrag { .. } => output.push(TouchOutput::PointerCancel),
            GestureState::Transform { .. } => {
                output.push(TouchOutput::TransformEnd {
                    velocity_x: 0.0,
                    velocity_y: 0.0,
                    cancelled: true,
                });
            }
            _ => {}
        }
        self.touches.clear();
        self.ignored_touches.clear();
        self.state = GestureState::Idle;
        self.last_tap = None;
        output
    }

    fn release_feedback(&mut self, id: u64, x: f64, y: f64) -> Vec<TouchOutput> {
        let Some(point) = self.touches.get_mut(&id) else {
            return Vec::new();
        };
        if !point.feedback_active {
            return Vec::new();
        }
        point.feedback_active = false;
        vec![TouchOutput::PressFeedback {
            contact: id,
            x,
            y,
            pressed: false,
        }]
    }

    fn clear_feedback(&mut self) -> Vec<TouchOutput> {
        self.touches
            .iter_mut()
            .filter_map(|(&contact, point)| {
                if !point.feedback_active {
                    return None;
                }
                point.feedback_active = false;
                Some(TouchOutput::PressFeedback {
                    contact,
                    x: point.x,
                    y: point.y,
                    pressed: false,
                })
            })
            .collect()
    }

    fn begin_transform(&mut self, now_ms: u32) -> Vec<TouchOutput> {
        let mut output = self.clear_feedback();
        let (first, second) = self.two_touches();
        let (first_x, first_y) = first.origin();
        let (second_x, second_y) = second.origin();
        let (centroid_x, centroid_y, distance) =
            transform_geometry(first.current(), second.current());
        self.state = GestureState::Transform {
            last_centroid_x: centroid_x,
            last_centroid_y: centroid_y,
            last_distance: distance,
            velocity_sample_centroid_x: centroid_x,
            velocity_sample_centroid_y: centroid_y,
            velocity_sample_ms: now_ms,
            velocity_x: 0.0,
            velocity_y: 0.0,
        };
        self.last_tap = None;
        output.push(TouchOutput::TransformStart {
            first_x,
            first_y,
            second_x,
            second_y,
        });
        output
    }

    fn transform_end(&self, now_ms: u32, cancelled: bool) -> TouchOutput {
        let GestureState::Transform {
            velocity_x,
            velocity_y,
            velocity_sample_ms,
            ..
        } = self.state
        else {
            panic!("transform end requested without an active transform");
        };
        let velocity_is_fresh = now_ms.wrapping_sub(velocity_sample_ms) <= MAX_FLING_SAMPLE_AGE_MS;
        TouchOutput::TransformEnd {
            velocity_x: if velocity_is_fresh { velocity_x } else { 0.0 },
            velocity_y: if velocity_is_fresh { velocity_y } else { 0.0 },
            cancelled,
        }
    }

    fn two_points(&self) -> ((f64, f64), (f64, f64)) {
        let (first, second) = self.two_touches();
        (first.current(), second.current())
    }

    fn two_touches(&self) -> (TouchPoint, TouchPoint) {
        let mut values = self.touches.values().copied();
        let first = values.next().expect("two-point gesture lost first touch");
        let second = values.next().expect("two-point gesture lost second touch");
        assert!(
            values.next().is_none(),
            "touch classifier tracked too many pointers"
        );
        (first, second)
    }

    fn tap_count(&mut self, now_ms: u32, x: f64, y: f64) -> u8 {
        let is_double = self.last_tap.is_some_and(|(last_ms, last_x, last_y)| {
            now_ms.wrapping_sub(last_ms) <= DOUBLE_TAP_INTERVAL_MS
                && distance((last_x, last_y), (x, y)) <= self.double_tap_slop_px
        });
        if is_double {
            self.last_tap = None;
            2
        } else {
            self.last_tap = Some((now_ms, x, y));
            1
        }
    }
}

fn transform_geometry(first: (f64, f64), second: (f64, f64)) -> (f64, f64, f64) {
    (
        (first.0 + second.0) * 0.5,
        (first.1 + second.1) * 0.5,
        distance(first, second).max(f64::EPSILON),
    )
}

fn distance(first: (f64, f64), second: (f64, f64)) -> f64 {
    (second.0 - first.0).hypot(second.1 - first.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_contact_updates_leave_each_gesture_unchanged() {
        let mut touch = TouchClassifier::default();
        for phase in 0..5 {
            match phase {
                1 => {
                    touch.started(1, 10.0, 20.0, 0);
                }
                2 => {
                    touch.moved(1, 40.0, 20.0, 10);
                }
                3 => {
                    touch.started(2, 60.0, 20.0, 20);
                }
                4 => {
                    touch.ended(1, 45.0, 20.0, 30, false);
                }
                _ => {}
            }
            let before = serde_json::to_value(&touch).unwrap();
            assert!(touch.moved(99, 100.0, 200.0, 40).is_empty());
            assert!(touch.ended(99, 100.0, 200.0, 50, false).is_empty());
            assert!(touch.ended(99, 100.0, 200.0, 60, true).is_empty());
            assert_eq!(serde_json::to_value(&touch).unwrap(), before);
        }
    }

    #[test]
    fn transform_uses_origins_for_start_and_current_points_for_geometry() {
        let mut touch = TouchClassifier::default();
        touch.started(20, 10.0, 20.0, 0);
        touch.moved(20, 14.0, 20.0, 10);
        assert_eq!(
            touch.started(10, 30.0, 20.0, 251),
            vec![
                TouchOutput::PressFeedback {
                    contact: 10,
                    x: 30.0,
                    y: 20.0,
                    pressed: false,
                },
                TouchOutput::PressFeedback {
                    contact: 20,
                    x: 14.0,
                    y: 20.0,
                    pressed: false,
                },
                TouchOutput::TransformStart {
                    first_x: 30.0,
                    first_y: 20.0,
                    second_x: 10.0,
                    second_y: 20.0,
                },
            ]
        );
        assert!(matches!(
            touch.state,
            GestureState::Transform {
                last_centroid_x: 22.0,
                last_centroid_y: 20.0,
                last_distance: 16.0,
                ..
            }
        ));
    }

    #[test]
    fn tap_is_emitted_as_atomic_release_classified_click() {
        let mut touch = TouchClassifier::default();
        assert_eq!(
            touch.started(7, 10.0, 20.0, 100),
            vec![
                TouchOutput::MotionStop,
                TouchOutput::PressFeedback {
                    contact: 7,
                    x: 10.0,
                    y: 20.0,
                    pressed: true,
                },
            ]
        );
        assert_eq!(
            touch.ended(7, 11.0, 21.0, 140, false),
            vec![
                TouchOutput::PressFeedback {
                    contact: 7,
                    x: 11.0,
                    y: 21.0,
                    pressed: false,
                },
                TouchOutput::PointerMove { x: 11.0, y: 21.0 },
                TouchOutput::PointerDown {
                    x: 11.0,
                    y: 21.0,
                    button: 1,
                    clicks: 1,
                },
                TouchOutput::PointerUp {
                    x: 11.0,
                    y: 21.0,
                    button: 1,
                },
            ]
        );
    }

    #[test]
    fn release_displacement_classifies_a_coalesced_drag_as_a_drag() {
        let mut touch = TouchClassifier::new(5.0);
        touch.started(7, 10.0, 20.0, 100);

        assert_eq!(
            touch.ended(7, 30.0, 20.0, 140, false),
            vec![
                TouchOutput::PressFeedback {
                    contact: 7,
                    x: 30.0,
                    y: 20.0,
                    pressed: false,
                },
                TouchOutput::PointerDown {
                    x: 10.0,
                    y: 20.0,
                    button: 1,
                    clicks: 1,
                },
                TouchOutput::PointerMove { x: 30.0, y: 20.0 },
                TouchOutput::PointerUp {
                    x: 30.0,
                    y: 20.0,
                    button: 1,
                },
            ]
        );
    }

    #[test]
    fn second_finger_cancels_pending_tap_without_mouse_release() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 10.0, 10.0, 0);
        assert_eq!(
            touch.started(2, 30.0, 10.0, 20),
            vec![TouchOutput::PressFeedback {
                contact: 2,
                x: 30.0,
                y: 10.0,
                pressed: true,
            }]
        );
        let output = touch.moved(2, 42.0, 10.0, 30);
        assert!(
            output
                .iter()
                .any(|event| { matches!(event, TouchOutput::TransformStart { .. }) })
        );
        assert!(!output.iter().any(|event| matches!(
            event,
            TouchOutput::PointerDown { .. } | TouchOutput::PointerUp { .. }
        )));
    }

    #[test]
    fn second_finger_explicitly_cancels_started_drag() {
        let mut touch = TouchClassifier::new(5.0);
        touch.started(1, 10.0, 10.0, 0);
        assert!(matches!(
            touch.moved(1, 20.0, 10.0, 10).as_slice(),
            [
                TouchOutput::PointerDown { .. },
                TouchOutput::PointerMove { .. }
            ]
        ));
        assert!(matches!(
            touch.started(2, 30.0, 10.0, 20).as_slice(),
            [
                TouchOutput::PointerCancel,
                TouchOutput::PressFeedback {
                    contact: 1,
                    pressed: false,
                    ..
                },
                TouchOutput::PressFeedback {
                    contact: 2,
                    pressed: false,
                    ..
                },
                TouchOutput::TransformStart { .. },
            ]
        ));
    }

    #[test]
    fn transform_reports_simultaneous_pan_scale_and_velocity() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 251);
        let output = touch.moved(2, 22.0, 0.0, 351);
        let [
            TouchOutput::TransformUpdate {
                centroid_x,
                pan_x,
                scale,
                velocity_x,
                ..
            },
        ] = output.as_slice()
        else {
            panic!("expected transform update: {output:?}");
        };
        assert_eq!(*centroid_x, 11.0);
        assert_eq!(*pan_x, 6.0);
        assert_eq!(*scale, 2.2);
        assert!(*velocity_x > 0.0);
    }

    #[test]
    fn transform_end_includes_a_coalesced_final_position() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 251);

        let output = touch.ended(2, 22.0, 0.0, 351, false);
        assert!(matches!(
            output.as_slice(),
            [
                TouchOutput::TransformUpdate {
                    centroid_x: 11.0,
                    pan_x: 6.0,
                    scale: 2.2,
                    ..
                },
                TouchOutput::TransformEnd {
                    cancelled: false,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn remaining_finger_after_transform_cannot_turn_into_tap() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 251);
        assert!(matches!(
            touch.ended(2, 10.0, 0.0, 301, false).as_slice(),
            [TouchOutput::TransformEnd { .. }]
        ));
        assert!(touch.ended(1, 0.0, 0.0, 310, false).is_empty());
    }

    #[test]
    fn nearby_taps_emit_double_click_but_distant_taps_do_not() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 10.0, 10.0, 0);
        touch.ended(1, 10.0, 10.0, 20, false);
        touch.started(2, 12.0, 11.0, 100);
        let second = touch.ended(2, 12.0, 11.0, 120, false);
        assert!(
            second
                .iter()
                .any(|event| matches!(event, TouchOutput::PointerDown { clicks: 2, .. }))
        );

        touch.started(3, 100.0, 100.0, 160);
        let distant = touch.ended(3, 100.0, 100.0, 170, false);
        assert!(
            distant
                .iter()
                .any(|event| matches!(event, TouchOutput::PointerDown { clicks: 1, .. }))
        );
    }

    #[test]
    fn third_pointer_is_ignored_without_disturbing_transform() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 251);
        assert!(touch.started(3, 20.0, 0.0, 251).is_empty());
        assert!(touch.moved(3, 30.0, 0.0, 261).is_empty());
        assert!(touch.ended(3, 30.0, 0.0, 271, false).is_empty());
        assert!(matches!(
            touch.moved(2, 15.0, 0.0, 281).as_slice(),
            [TouchOutput::TransformUpdate { .. }]
        ));
    }

    #[test]
    fn transform_start_reports_pointer_origins_for_world_eligibility() {
        let mut touch = TouchClassifier::new(5.0);
        touch.started(1, 10.0, 10.0, 0);
        touch.moved(1, 100.0, 100.0, 10);
        let output = touch.started(2, 120.0, 100.0, 20);

        let Some(TouchOutput::TransformStart {
            first_x,
            first_y,
            second_x,
            second_y,
        }) = output.last()
        else {
            panic!("expected transform start: {output:?}");
        };
        assert_eq!((*first_x, *first_y), (10.0, 10.0));
        assert_eq!((*second_x, *second_y), (120.0, 100.0));
    }

    #[test]
    fn ignored_third_pointer_cannot_leave_recognizer_suppressed() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 251);
        touch.started(3, 20.0, 0.0, 251);
        touch.ended(1, 0.0, 0.0, 261, false);
        touch.ended(2, 10.0, 0.0, 271, false);
        touch.ended(3, 20.0, 0.0, 281, false);

        assert_eq!(
            touch.started(4, 40.0, 40.0, 300),
            vec![
                TouchOutput::MotionStop,
                TouchOutput::PressFeedback {
                    contact: 4,
                    x: 40.0,
                    y: 40.0,
                    pressed: true,
                },
            ]
        );
        assert!(
            touch
                .ended(4, 40.0, 40.0, 310, false)
                .iter()
                .any(|event| { matches!(event, TouchOutput::PointerDown { clicks: 1, .. }) })
        );
    }

    #[test]
    fn density_scale_applies_to_drag_and_double_tap_slop() {
        let mut touch = TouchClassifier::default();
        touch.set_scale_factor(2.0);
        touch.started(1, 0.0, 0.0, 0);
        assert!(touch.moved(1, 15.0, 0.0, 10).is_empty());
        touch.ended(1, 15.0, 0.0, 20, false);

        touch.started(2, 55.0, 0.0, 100);
        let second = touch.ended(2, 55.0, 0.0, 120, false);
        assert!(
            second
                .iter()
                .any(|event| { matches!(event, TouchOutput::PointerDown { clicks: 2, .. }) })
        );
    }

    #[test]
    fn same_millisecond_moves_accumulate_without_velocity_spike() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 251);
        touch.moved(1, 2.0, 0.0, 261);
        let output = touch.moved(2, 12.0, 0.0, 261);
        let [TouchOutput::TransformUpdate { velocity_x, .. }] = output.as_slice() else {
            panic!("expected transform update: {output:?}");
        };
        assert!((*velocity_x - 35.0).abs() < f64::EPSILON);

        let output = touch.moved(2, 14.0, 0.0, 271);
        let [TouchOutput::TransformUpdate { velocity_x, .. }] = output.as_slice() else {
            panic!("expected transform update: {output:?}");
        };
        assert!(*velocity_x < 200.0);
    }

    #[test]
    fn holding_transform_still_before_release_suppresses_stale_fling() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 0.0, 0.0, 0);
        touch.started(2, 10.0, 0.0, 251);
        touch.moved(2, 20.0, 0.0, 271);

        assert!(matches!(
            touch.ended(2, 20.0, 0.0, 451, false).as_slice(),
            [TouchOutput::TransformEnd {
                velocity_x: 0.0,
                velocity_y: 0.0,
                cancelled: false,
            }]
        ));
    }

    #[test]
    fn intervening_drag_or_transform_invalidates_double_tap_history() {
        let mut touch = TouchClassifier::new(5.0);
        touch.started(1, 10.0, 10.0, 0);
        touch.ended(1, 10.0, 10.0, 10, false);
        touch.started(2, 10.0, 10.0, 20);
        touch.moved(2, 20.0, 10.0, 30);
        touch.ended(2, 20.0, 10.0, 40, false);
        touch.started(3, 10.0, 10.0, 50);
        let after_drag = touch.ended(3, 10.0, 10.0, 60, false);
        assert!(
            after_drag
                .iter()
                .any(|event| matches!(event, TouchOutput::PointerDown { clicks: 1, .. }))
        );

        touch.started(4, 10.0, 10.0, 70);
        touch.started(5, 20.0, 10.0, 321);
        touch.ended(5, 20.0, 10.0, 331, false);
        touch.ended(4, 10.0, 10.0, 341, false);
        touch.started(6, 10.0, 10.0, 350);
        let after_transform = touch.ended(6, 10.0, 10.0, 360, false);
        assert!(
            after_transform
                .iter()
                .any(|event| matches!(event, TouchOutput::PointerDown { clicks: 1, .. }))
        );
    }

    #[test]
    fn long_press_releases_as_a_secondary_click_at_450_ms() {
        let mut touch = TouchClassifier::default();
        assert_eq!(
            touch.started(7, 10.0, 20.0, 100),
            vec![
                TouchOutput::MotionStop,
                TouchOutput::PressFeedback {
                    contact: 7,
                    x: 10.0,
                    y: 20.0,
                    pressed: true,
                },
            ]
        );

        assert_eq!(
            touch.ended(7, 11.0, 21.0, 550, false),
            vec![
                TouchOutput::PressFeedback {
                    contact: 7,
                    x: 11.0,
                    y: 21.0,
                    pressed: false,
                },
                TouchOutput::PointerMove { x: 11.0, y: 21.0 },
                TouchOutput::PointerDown {
                    x: 11.0,
                    y: 21.0,
                    button: 3,
                    clicks: 1,
                },
                TouchOutput::PointerUp {
                    x: 11.0,
                    y: 21.0,
                    button: 3,
                },
            ]
        );
    }

    #[test]
    fn a_449_ms_hold_remains_a_primary_click() {
        let mut touch = TouchClassifier::default();
        touch.started(7, 10.0, 20.0, 100);
        let output = touch.ended(7, 10.0, 20.0, 549, false);
        assert!(output.iter().any(|event| {
            matches!(
                event,
                TouchOutput::PointerDown {
                    button: 1,
                    clicks: 1,
                    ..
                }
            )
        }));
        assert!(
            !output
                .iter()
                .any(|event| { matches!(event, TouchOutput::PointerDown { button: 3, .. }) })
        );
    }

    #[test]
    fn long_drag_uses_the_secondary_button_through_release() {
        let mut touch = TouchClassifier::default();
        touch.started(7, 10.0, 20.0, 100);
        assert_eq!(
            touch.moved(7, 30.0, 20.0, 550),
            vec![
                TouchOutput::PointerDown {
                    x: 10.0,
                    y: 20.0,
                    button: 3,
                    clicks: 1,
                },
                TouchOutput::PointerMove { x: 30.0, y: 20.0 },
            ]
        );
        assert_eq!(
            touch.ended(7, 40.0, 20.0, 600, false),
            vec![
                TouchOutput::PressFeedback {
                    contact: 7,
                    x: 40.0,
                    y: 20.0,
                    pressed: false,
                },
                TouchOutput::PointerMove { x: 40.0, y: 20.0 },
                TouchOutput::PointerUp {
                    x: 40.0,
                    y: 20.0,
                    button: 3,
                },
            ]
        );
    }

    #[test]
    fn two_finger_tap_emits_one_secondary_click_at_the_centroid() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 10.0, 20.0, 0);
        assert_eq!(
            touch.started(2, 30.0, 40.0, 100),
            vec![TouchOutput::PressFeedback {
                contact: 2,
                x: 30.0,
                y: 40.0,
                pressed: true,
            }]
        );
        assert_eq!(
            touch.ended(1, 10.0, 20.0, 200, false),
            vec![TouchOutput::PressFeedback {
                contact: 1,
                x: 10.0,
                y: 20.0,
                pressed: false,
            }]
        );
        assert_eq!(
            touch.ended(2, 30.0, 40.0, 250, false),
            vec![
                TouchOutput::PressFeedback {
                    contact: 2,
                    x: 30.0,
                    y: 40.0,
                    pressed: false,
                },
                TouchOutput::PointerMove { x: 20.0, y: 30.0 },
                TouchOutput::PointerDown {
                    x: 20.0,
                    y: 30.0,
                    button: 3,
                    clicks: 1,
                },
                TouchOutput::PointerUp {
                    x: 20.0,
                    y: 30.0,
                    button: 3,
                },
            ]
        );
    }

    #[test]
    fn two_finger_slop_takes_over_the_transform_and_clears_feedback() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 10.0, 20.0, 0);
        touch.started(2, 30.0, 20.0, 100);
        let output = touch.moved(2, 42.0, 20.0, 150);
        assert!(matches!(
            output.as_slice(),
            [
                TouchOutput::PressFeedback {
                    contact: 1,
                    pressed: false,
                    ..
                },
                TouchOutput::PressFeedback {
                    contact: 2,
                    pressed: false,
                    ..
                },
                TouchOutput::TransformStart { .. },
                TouchOutput::TransformUpdate { .. },
            ]
        ));
        assert!(
            !output
                .iter()
                .any(|event| { matches!(event, TouchOutput::PointerDown { button: 3, .. }) })
        );
    }

    #[test]
    fn a_late_second_contact_cannot_become_a_secondary_click() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 10.0, 20.0, 0);
        assert_eq!(
            touch.started(2, 30.0, 20.0, 251),
            vec![
                TouchOutput::PressFeedback {
                    contact: 1,
                    x: 10.0,
                    y: 20.0,
                    pressed: false,
                },
                TouchOutput::PressFeedback {
                    contact: 2,
                    x: 30.0,
                    y: 20.0,
                    pressed: false,
                },
                TouchOutput::TransformStart {
                    first_x: 10.0,
                    first_y: 20.0,
                    second_x: 30.0,
                    second_y: 20.0,
                },
            ]
        );
        let first = touch.ended(1, 10.0, 20.0, 260, false);
        let second = touch.ended(2, 30.0, 20.0, 270, false);
        assert!(
            !first
                .iter()
                .chain(&second)
                .any(|event| { matches!(event, TouchOutput::PointerDown { button: 3, .. }) })
        );
    }

    #[test]
    fn a_late_second_release_cannot_become_a_secondary_click() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 10.0, 20.0, 0);
        touch.started(2, 30.0, 20.0, 100);
        touch.ended(1, 10.0, 20.0, 200, false);
        let output = touch.ended(2, 30.0, 20.0, 251, false);
        assert_eq!(
            output,
            vec![TouchOutput::PressFeedback {
                contact: 2,
                x: 30.0,
                y: 20.0,
                pressed: false,
            }]
        );
    }

    #[test]
    fn cancel_all_releases_every_active_press_feedback_contact() {
        let mut touch = TouchClassifier::default();
        touch.started(1, 10.0, 20.0, 0);
        touch.started(2, 30.0, 20.0, 100);
        assert_eq!(
            touch.cancel_all(),
            vec![
                TouchOutput::PressFeedback {
                    contact: 1,
                    x: 10.0,
                    y: 20.0,
                    pressed: false,
                },
                TouchOutput::PressFeedback {
                    contact: 2,
                    x: 30.0,
                    y: 20.0,
                    pressed: false,
                },
            ]
        );
    }
}
