# Bug reports

A stuck mission, a wrong objective, a prop that spins and never returns the
player: nothing crashed, so there is no panic and no crash bundle. A voluntary
report captures the same kind of evidence on demand and files it as an issue
labelled `bug` and `bug-report`.

Implementation: `crates/robin_rs/src/bug_bundle.rs` (reserve, write, upload),
`crates/robin_rs/src/game_session/report.rs` (the game state),
`crates/robin_rs/src/ingame_menu/bug_report.rs` (the screen), and
`tools/crash-receiver/receiver.py` (the receiver). Crashes are a separate
contract: [CRASHES.md](CRASHES.md).

## Three entrances, one path

| Entrance | What it captures |
| --- | --- |
| Pause menu → **Report a bug** | a text field; Enter files, Escape cancels. The pause menu stays up behind it, so neither screen pauses or resumes the mission |
| `/BUG <description>` in the in-game console | the same bundle, filed from the console scrollback |
| `POST /bug-report` (`docs/JSON_SERVER.md`) | the same bundle, for an automation client |

All three reserve a bundle directory, queue a request, and return. The queue is
drained once per mission frame by `game_session::report::drain_requested`, which
is the only place the whole live mission is borrowable: the engine that just
ticked, the host's modal stack, the timeline and the transport. Writing happens
on a worker thread, never on the frame.

`bug_bundle::reserve` creates `<install home>/bug-reports/<utc>-<origin>-<n>/`
immediately, so the answer to any entrance can name where the evidence goes and
two reports cannot collide. The origin is `ingame-menu`, `console` or `http`.

The pause-menu screen keeps a failure on screen: an empty description, a failed
reserve/queue, a missing `ROBIN_BUG_ENDPOINT`, and a browser build with no bundle
all leave the modal open with the reason in its status area. Only a queued
bundle dismisses it. The browser build says reporting is unsupported, not that
a report was filed. The description field wraps in a taller box and scrolls
vertically, following the caret while typing and allowing the wheel to move the
view; typed text uses the `EditField` font, not the popup label font.

## What the bundle carries

`bundle.json` is one pretty-printed JSON object. Every section is present or
says why it is not — a silently missing section reads as "there was nothing
there".

| Key | Contents |
| --- | --- |
| `description`, `note`, `origin`, `captured_at` | the player's words, the entrance, and when the report was written |
| `fingerprint` | `sha256(mission filename \0 script diagnostic state \0 version)[..16]`; the tick is excluded, so the same stuck state on two seats or two ticks is one issue |
| `build` | `crash::version()` (version plus stamped commit), `git_rev`, `target`, `profile`, `release`, `replay_schema_version`, `runtime_contract` and its `runtime_contract_hash` |
| `machine` | OS, arch, CPU count, and the process/driver attachment from `diagnostic_context` — only as far as it explains a symptom |
| `session` | `kind` (`single` / `host` / `client`), `session_id` (all seats of one session share it), `transport` (local seat, expected players, reconnecting, mission id), `mission` (profile id, title, filename, map, campaign slot), `tick` (timeline frame, simulation tick, engine frame counter), `players`, `desync` (current state hash, last agreeing frame, mismatch count), `replay_playback` |
| `session.players` | one entry per seat including empty ones: `seat`, `connected`, `nickname`, `assigned_character`, `selection`, `build_rev` |
| `game` | `operation` and its previous value, message text, engine mission document, `script`, `presentation`, `terminal`, `staged` (modal dismissals, post commands, pending modal kinds, pending external actions) |
| `game.script` | `MissionScript::diagnostic_state()`: script name, instance counts, and the global instance's class and members — the objective flags a stuck mission is diagnosed from |
| `game.terminal` | the mission-end page the player is looking at: `active`, `presentation`, `decision_order`, `awaiting_snapshot`, and whether a campaign update is pending |
| `world` | every player character's position and state, horses and companions, and an alive count with the predicate that produced it |
| `files.saves` | every file of the profile's save directory, copied under `saves/` with name and size, within a 32 MiB budget; anything left out is named in `notes` |
| `files.replay` | the active recording's files under `replay/`, within an 8 MiB budget, each entry recording the truncation the copy applied |
| `state` | the last `/state` document (`docs/JSON_SERVER.md`) |
| `run_log_tail`, `events_tail` | the last 200 lines of the run log and of the session event file `/state` names |
| `notes` | every section that could not be captured, and why |

### Captured as unavailable, on purpose

These are facts a reader might expect and will find marked instead:

- per-seat build revision: `NetMsg::Hello` carries only the protocol version and
  a nickname, so every seat except the local one reports
  `{"available": false, "reason": ...}`. Adding the field is a wire change.
- `TerminalDebriefingState::phase`: private to a module this work must not
  touch; `decision_order` and `presentation` carry the equivalent.

## Upload

The bundle is written first, then `spawn_upload_child("--upload-bug-report",
<dir>)` starts the same executable detached, with no stdin or output and its own
process group, so quitting the game cannot cancel it. That child POSTs
`bundle.json` to `ROBIN_BUG_ENDPOINT` with `ROBIN_CRASH_TOKEN` as a bearer token
and writes `upload-status.json` beside the bundle.

Nothing is uploaded unless `ROBIN_BUG_ENDPOINT` is set. A failed upload leaves
the bundle where it is, and the path is printed in the console and on the report
screen: the evidence is never the part that is lost.

## Endpoint contract

`POST /bug-report`, `Content-Type: application/json`, `Authorization: Bearer
<CRASH_TOKEN>`, at most `CRASH_MAX_BODY_BYTES` (default 4 MiB). The body is
`bundle.json` as written.

| Status | Meaning |
| --- | --- |
| 202 | accepted; body `{"fingerprint", "bundle", "issue", "occurrences"}` |
| 400 | not JSON, an empty `description`, a `fingerprint` that is not 16 lowercase hex characters, or a missing `build` / `session` |
| 401 | missing or wrong bearer token |
| 411 / 413 | no `Content-Length`, or a body larger than the limit |
| 502 | the bundle is stored but GitHub refused the issue |

`POST /bug-report` on the game answers immediately with the reserved bundle
directory; the bundle itself appears at the next frame boundary. A browser build
has no bundle directory and answers with an unavailable-capability error.

### Reading a bundle back

Both routes demand the same bearer token as the upload; they are never public,
because a bundle carries saves, a replay and the player's own words.

- `GET /bug-reports/<bundle id>` — the stored bundle;
- `GET /bug-reports/by-session/<session id>` — every report of one session, in
  arrival order.

A bundle on disk is turned into a row of [DEFECTS.md](DEFECTS.md) by
`scripts/triage_bug_bundle.py <bundle>`: it prints the fields the row needs and
the `--replay` command that reproduces the defect. It prints the class and the
check as `?`, because choosing them is the diagnosis.

## The receiver

Same process and the same environment as the crash side; `CRASH_TOKEN`,
`GITHUB_TOKEN`, `GITHUB_REPO`, `GITHUB_API_URL`, `CRASH_STATE_DIR`,
`CRASH_MAX_BODY_BYTES` and the bind variables are documented in
[CRASHES.md](CRASHES.md#the-receiver). Under `CRASH_STATE_DIR` bug reports add:

- `bug-bundles/<16 hex>.json` — one file per report, named at random because a
  report's identity is its evidence, not its fingerprint;
- `bug-occurrences/<fingerprint>.json` — count, issue, bundle ids and session
  ids for one stuck state;
- `bug-sessions/<session id>.json` — one session's reports grouped, whatever
  order they arrive in.

A first report creates one issue with the labels `bug` and `bug-report` (created
first if the repository lacks them), titled with the description and led by the
reproduction facts: session, mission, tick, build, operation, seats, desync and
alive count. The body summarises the bundle and links its download; it never
pastes the raw document. A repeat fingerprint adds a comment naming the
occurrence count and the new bundle's id.

## Multiplayer

Every seat of one session reports under the same `session_id`, and the receiver
groups those bundles. There is no wire message that asks another peer for its
bundle, so each player files their own: the report screen says so whenever the
session has a transport.

## Tests

`bash scripts/check-quality.sh client-release` runs the client regressions under
`crates/robin_rs/src/bug_bundle.rs` and `crates/robin_rs/src/game_session/report.rs`
(a written bundle carries every section, missing evidence is named, no endpoint
keeps the bundle, an empty description is refused). `bash
scripts/check-quality.sh tooling` runs `tools/crash-receiver/test_receiver.py`:
authentication, size, first report files one labelled issue, repeat comments,
downloads with and without the token, session grouping, and the bundle kept
through a GitHub outage.
