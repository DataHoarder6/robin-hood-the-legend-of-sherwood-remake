# Replay: a modal dismissed outside the journal (#45)

Evidence for `docs/MULTIPLAYER.md`, "What each seat records": a recorded modal
dismissal must be consumed in its own host frame on playback.

Last verified: 2026-11-18 against `fork/main` `d6d7752e1` and the recording in
`~/robin-parity/rec1` (built from `5e1d2b17e5f5`, schema 76).

## Failure

Sherwood HQ queues its entry popup from `PostInitialize`, which the headless
driver runs *after* the frame's own modal drain. The next frame's pre-tick
`dismiss_pending_modals` dropped that popup without recording it. Playback skips
that drop (it replays the journal), reopens the popup and waits for an
acknowledgement that was never written, so the *next* recorded dismissal — the
crouch tutorial popup `PopupText { text_id: 34 }` at frame 151 — is orphaned:

```text
replay desync: 1 recorded modal dismissal(s) were unused in their host frame: [ModalDismiss { kind: PopupText { text_id: 34 }, result: Completed }]
```

State hashes 0..150 are identical; only the presentation acknowledgement is
missing from the recording. The recording's own `replay.debug.log` shows the
drop:

```text
DisplayPopupText(0): queued for UI system
robin_rs::game_session::tick: non-interactive driver: dismissing 1 pending modal(s) (dialogues=0, popups=1, …)
```

## Reproducer

`--mission Sherwood` runs the HQ mission; the spec's `CrouchDown` at frame 120
opens the second popup. `--record` writes the chunk, `--replay` reads it back
with the same binary:

```sh
ROBINHOOD_DATA_DIR=/path/to/robin-hood/game \
  robin --headless --no-sound --mission Sherwood --spec <fixture> --record DIR
ROBINHOOD_DATA_DIR=/path/to/robin-hood/game \
  robin --headless --no-sound --replay DIR
```

After the fix frame 1 carries `ModalDismiss { PopupText { text_id: 0 } }` and
frame 151 the tutorial's dismissal; playback verifies the same 24 cadence hashes
and exits 0.

## Where the fact belongs

The dismissal is host-side state: it lives in `ReplayHostControl::ModalDismiss`
on the frame whose driver consumed it (`crates/robin_engine/src/replay.rs`,
`crates/robin_rs/src/game_session/runtime.rs::finish_recording`). The modal's
open frame is not a separate record — playback reopens it from the replayed
`HostEffects` of the same frame. That is only sound while the recording driver
and the playback driver agree on which frame admits a queued modal; the fix
makes the journaled drain the headless driver's only dismissal sink.

`discard_obsolete_upgrade_popups`
(`crates/robin_rs/src/game_session/session_policy.rs`) is unchanged: it absorbs
a *capture-side* omission from an older engine, not a missing dismissal here.
