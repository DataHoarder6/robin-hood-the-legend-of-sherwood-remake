//! Plain run-identity data types shared by the deterministic engine and the
//! leaderboard wire protocol.
//!
//! `robin_engine` uses the board simulation policy and replay seat transcript
//! types, while `robin_run_protocol` builds the signed submission,
//! verification and query contracts on top of them and re-exports every item
//! unchanged. This crate deliberately has no signing, networking, service or
//! engine dependencies. Names, field order and serde attributes are wire
//! contracts: moving a type here must never change its encoding.

pub mod artifact;
pub mod canonical;
pub mod content;
pub mod digest;
pub mod ruleset;
pub mod session;
pub mod validation;

/// Simulation frame timing target: 40ms = 25fps. Shared by the engine and the
/// leaderboard service, which reports the exact tick duration to clients.
pub const FRAME_TIME_MS: u32 = 40;

/// Wire schema of a replay file: the header version, the record encoding and
/// the input layout. The engine writes it; the leaderboard verifier accepts
/// exactly this value and rejects anything else, so it is defined once here
/// and re-exported by both sides.
///
/// Version 41 removes deferred AI continuation
/// state and uses synchronous engine-owned decision execution.
/// Version 40 removes duplicate royalist and
/// lacklandist presence flags from AI snapshots and hashes, retaining the
/// existing soldier-camp set as the initial authority.
/// Version 39 removes the unused computed-location
/// dummy, object repulsive-point snapshot copy, and engine-camera scratch fields.
/// Hash-skipped values still emitted markers; removing those fields changes the
/// state-hash stream too. Version 38 stores the optional recording
/// session directly, without the recorder wrapper and its write-only
/// sequence ID, in snapshots and state hashes. Version 37 removes the unused imported
/// sound blob and unconsumed same-frame AI target claims from snapshots
/// and state hashes. Version 36 retains one game-owned
/// PostInitialize latch and removes unused imported Messenger copies from
/// snapshots. The separate Original-game trace format is unchanged.
/// Version 35 uses one canonical script-global
/// vector in authoritative snapshots and state hashes instead of parallel
/// imported-vector and live ID/value-map storage. Version 34 makes the ordered pending AI
/// detectable-mutation FIFO authoritative snapshot and state-hash data.
/// Version 33 embeds serde JSON saves in load
/// records when a pinned timeline marker cannot reproduce the loaded state.
/// Version 31 adds the opt-in reversible
/// background-patch configuration and remembered activation targets to
/// deterministic state. Version 30 makes vector fog part of the
/// deterministic state-hash contract; the replay JSONL record shape itself is
/// unchanged. The current schema combines the exact bounded
/// canonical Spellforge package and its deterministic journal/digest state with
/// every current-main simulation field documented below.
///
/// Version 15 combined the completed native
/// bitcode campaign/compact encoding with the strict full-frame boundary:
/// every admitted [`SimulationFrameInput`] and host control is explicit,
/// serialized command chains are non-recursive, stored movement actions carry
/// exact routes, and point-Seek state carries explicit route provenance.
/// Version 17 requires full-fidelity campaign history. Version 18 adds
/// achievement tracker state, and version 19 combines that with expanded item
/// rules, cached ale eligibility, and ground-stone state. Version 20 combines
/// that state with authoritative Sherwood trading configuration, commands,
/// and receipts. Version 21 carries resolved Legendary/Custom difficulty
/// rules alongside both feature families. Version 22 adds typed nullable AI
/// entity handles and exact spatial provenance to that complete state: arena
/// slot zero is a live entity and absence is encoded by `Option`, so the
/// preceding raw-zero layout cannot be decoded safely. Version 23 adds
/// deterministic authored-mission timers and runtime ambience scheduling to
/// the simulation config and recorded engine state. Version 24 adds mission
/// diplomacy state, player-coalition policy, faction statistics, and runtime
/// relationship commands. Version 25 adds the authoritative composite-gesture
/// and quality-damage rules plus resolved technique and quality state across
/// commands, quick actions, active sequences, and active sweeps. Version 26
/// adds per-seat planned shield prompts and deterministic tactical queue
/// formations to the completed planned quick-action payloads. Version 27 adds
/// deterministic shared-vision fog state and its host-owned toggle command to
/// the complete engine snapshots and input stream. Version 28 adds the
/// mandatory, bounded cold-start mission-asset descriptor. Version 29 adds the
/// canonical ranked input-provenance contract to that complete mission-assets
/// and Spellforge-capable replay. It remains one replay artifact: participant
/// identity and signatures live in the submission envelope rather than a
/// second replay representation. There is deliberately no Rust-schema
/// compatibility adapter; earlier incompatible layouts are rejected at the
/// header.
/// Version 44 removes deferred gameplay and message queues from engine state.
/// Version 45 removes retained instruction handoff fields from actor state.
/// Version 46 removes duplicated movement, shot, and ability execution trackers.
/// Version 47 removes the unused entity-mutation counter marker from state hashes.
/// Version 48 stores canonical sequence links and ordinary door-route orders.
/// Version 49 removes the retired fog-sprite bypass from simulation configuration.
/// Version 50 consolidates quick-action storage and actor execution state.
/// Version 51 stores ordered pathfinder state and canonical action payloads.
/// Version 52 stores canonical order ownership, sequence links, and host effects.
/// Version 53 stores unified host output and canonical human opponent records.
/// Version 54 removes inferred corpse-posture and special-strike tracking state.
/// Version 55 stores co-op rules, assignments, and hero-copy identities.
/// Version 56 stores direct movement ownership and per-frame direction commands.
/// Version 57 stores exact cooperative mission teams.
/// Version 58 adds the combat gate for script-reported mission victory.
/// Version 59 records cooperative campaign rules and shared Robin inventory.
/// Version 60 retains physical stair identities on movement orders.
/// Version 61 retains precise physical landing boundaries in loaded motion data.
/// Version 63 retains compiled world-space passage endpoints.
/// Version 64 retains connected physical stair floor patches.
/// Version 65 persists the mission profile catalog in the replay header.
/// Version 66 distinguishes exact network state adoption from saved-game loads.
/// Version 67 retains ordinary physical floor identities on movement orders.
/// Version 68 removes physical solver orders and runtime door endpoints.
/// Version 69 stores the prepared passage availability bit on doors.
/// Version 70 stores disconnected fragments of a shared sound emitter.
/// Version 71 retains completed mission outcomes through speculative rollback.
/// Version 72 adds the per-PC "fight through, don't stop" flag.
/// Version 73 adds the per-PC auto-loot flag.
/// Version 74 adds the purchased-perk flags to campaign PC status.
/// Version 75 adds the per-PC "no friendly-fire damage" flag.
pub const REPLAY_SCHEMA_VERSION: u32 = 75;

pub use artifact::ArtifactRefV1;
pub use canonical::{
    CanonicalDocument, CanonicalDocumentError, CanonicalError, CanonicalValue, DomainSignedClaim,
    canonical_json_bytes,
};
pub use content::OfficialContentEditionV1;
pub use digest::{
    Digest32, HexError, OpaqueId, PublicKey32, Signature64, SimulationSeed64, SimulationSeedError,
};
pub use ruleset::{
    BoardSimulationPolicyV1, RANKED_SIMULATION_POLICY_VERSION_V1, RankedSimulationDifficultyV1,
    RankedSimulationPolicyV1, RankedSimulationPresetV1,
};
pub use session::{
    MAX_PARTICIPANT_INSTANCES_V1, MAX_REPLAY_SEATS_V1, ReplaySeatLifecycleEventV1,
    ReplaySeatLifecycleKindV1, ReplaySessionTranscriptV1,
};
pub use validation::{Validate, ValidationError};

/// Every explicitly named `V1` document carries this value on the wire.
pub const SCHEMA_VERSION_V1: u32 = 1;
/// Every explicitly named `V2` document carries this value on the wire.
pub const SCHEMA_VERSION_V2: u32 = 2;
/// Every explicitly named `V3` document carries this value on the wire.
pub const SCHEMA_VERSION_V3: u32 = 3;
