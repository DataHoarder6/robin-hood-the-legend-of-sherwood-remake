# Presentation decisions

Choices about how the port looks that are not simulation facts. Each entry says
what was decided, what it cost, and what would reopen it.

## Zoom-out coverage

**Decided: accept the letterbox.** The zoom ladder stays exactly `[0.5, 1.0,
2.0]` (`BackgroundTransform::zoom_values`, `crates/robin_engine/src/engine/camera_state.rs`),
the level is drawn where the camera puts it, and a viewport larger than the
level at 0.5 shows the opaque background around it.

Why not the alternatives:

- **Centring the letterbox** moves the projection offset, and that offset also
  positions entities, anchors the HUD and maps screen coordinates back to the
  map. It is a presentation pass with a real regression surface, not a cleanup,
  so it does not belong behind a bug report about smeared texels.
- **A fit zoom** would make the level always cover the window, but
  `zoom_factor` is part of `BackgroundTransform`, which is state-hashed and
  replay-compared, and the `SetZoomLevel`/`RecordSetZoom` native contract
  validates exactly the three authored values
  (`crates/robin_engine/src/natives/dispatch/sequences.rs`). A fourth value
  changes the replay format to solve a display problem.

The original's three zooms were authored against 4:3 displays, where 0.5 always
covered. Black bands on a 16:10 window are the honest rendering of "zoomed out
past the level", and they are opaque rather than stretched edge texels.

**Reopens if** large-screen play becomes a goal worth versioning the replay
format for (fit zoom), or the presentation pass that repositions the projection
is undertaken deliberately (centring). Neither is smuggled in behind a bug fix.
