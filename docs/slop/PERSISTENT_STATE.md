# Adding persistent state

The gates a new persistent field (per-PC progression, campaign unlock, world
setting) has to pass before it can ship. Each row names where the gate is
enforced; read that code, not this page, when the two disagree.

Last verified: 2026-11-15.

## Save files (JSON, human-readable)

- `SAVE_FORMAT_VERSION` (`crates/robin_rs/src/save_file.rs`) is an early-access
  compatibility baseline. Its own comment: bump it only when absolutely
  necessary and after explicit human confirmation; adding fields is not
  justification. `is_supported_save_version` accepts a range around it, so an
  additive field ships without a bump.
- An additive field therefore needs `#[serde(default)]` and, on a type with a
  hand-written `Default`, a default entry there too.
- `SaveHeader` and the replay headers carry `#[serde(deny_unknown_fields)]`:
  a new *header* field is a real format change, a new engine field is not.
- Renames and removals are proven by the save-file test that loads an older
  version's JSON and asserts the defaults (see `save_file.rs`).

## Determinism (state hash, replay, net)

- Simulation state is hashed; anything that changes the hash stream is a
  replay and protocol break. `#[state_hash(skip)]` excludes a field, and every
  use needs a comment stating why the field cannot affect the simulation.
- A change to a hashed, bitcode-snapshotted struct bumps three constants
  together, and each carries a version note:
  `REPLAY_SCHEMA_VERSION` (`crates/robin_engine/src/replay.rs`),
  `NET_PROTOCOL_VERSION` (`crates/robin_engine/src/multiplayer.rs`),
  `NATIVE_SNAPSHOT_VERSION` (`crates/robin_engine/src/engine/snapshot.rs`).
- Host-side state — wall clock, file system, environment, UI — never enters
  engine state. `crates/robin_util/src/diagnostic_only.rs` marks the values
  that are allowed to differ between a replay and the live run.

## Multiplayer

- Campaign-level mutation goes through host-authority commands:
  `PlayerCommand::requires_host_authority` (`crates/robin_engine/src/player_command.rs`).
  A command appended to that list is refused from a non-host seat, and the
  authoritative handler revalidates rather than trusting the sender.
- Peers must reach the same state from the same inputs, so progression data
  rides the campaign snapshot, not a per-seat side channel.

## Mods

- Spellforge package state is engine state: the runtime tapers and replays its
  own VM journal, so a mod that branches on host time or host files diverges.
  Package bytes are part of the snapshot; see `docs/slop/SPELLFORGE.md`.

See also: [`SAVE_FORMAT.md`](SAVE_FORMAT.md),
[`CAMPAIGN_PURCHASES.md`](CAMPAIGN_PURCHASES.md), and
[`../MULTIPLAYER.md`](../MULTIPLAYER.md).
