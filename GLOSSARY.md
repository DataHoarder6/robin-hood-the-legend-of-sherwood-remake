# Robin Hood port

The vocabulary of the Rust port: the game it reconstructs, the content it
loads, the co-op session it runs, and the loop that lands a change.

## The game

**Campaign**:
A run through the mission sequence with one gang, one purse and one set of
campaign values. Its progress outlives any single mission.
_Avoid_: playthrough, save game.

**Campaign slot**:
One saved campaign, named by the player: its gang, its mission sequence and its
campaign values. A joining co-op seat never writes one.
_Avoid_: save file (the file holds the slot), profile.

**Mission**:
One playable level with its script, objectives and level data. The unit of
play, of a mission bundle, and of a co-op session.
_Avoid_: level (that is the map geometry), map.

**Gang**:
The campaign's persistent set of playable characters. A co-op session uses the
campaign's gang, never an ad-hoc roster from the lobby.
_Avoid_: party, roster, team.

**Reservist**:
A gang member left out of the current mission's team. Returning one to the gang
pays the reintegration coefficients.
_Avoid_: reserve, bench.

**PC** (**hero**):
A playable character. The engine's selection is a list of hero entity ids; a
soldier is not one.
_Avoid_: character, unit, avatar.

**Actor**:
Anything the simulation moves and draws — a PC, a soldier, a civilian, an
animal. Not the storage id that names it.
_Avoid_: entity, sprite, object.

**Skill**:
A PC's trained combat stat, bow or hand-to-hand, with experience and capacity.
_Avoid_: ability, perk.

**Ability**:
A hero's special action in a mission — carry, tie-up, heal, whistle, listen,
place a trap.
_Avoid_: skill, perk, spell.

**Perk**:
A permanent upgrade a PC buys in Sherwood with clovers.
_Avoid_: skill, ability, upgrade.

**Clover**:
The campaign's death-save token and perk currency: one saves a VIP from a
lethal blow, so a purchase never spends the last. The UI calls the same counter
"Amulets".
_Avoid_: amulet, coin, gold.

**ARES**:
The original engine's per-mission progression state. The port keeps its
numbering, including `-1` for "no change".
_Avoid_: progress, mission status.

**Debriefing**:
The mission-end page. The only terminal screen, and where the result, the stats
and the leaderboard capture are fixed.
_Avoid_: results screen, summary.

## Content and data

**Datadir**:
An extracted game-data directory from one known release; `ROBINHOOD_DATA_DIR`
points at it. The port ships no assets.
_Avoid_: install, assets, game files.

**Content edition**:
The package family a guest must load to match the host: Demo or Full.
_Avoid_: datadir, install.

**Mission bundle**:
A content-addressed set of exactly the files one mission needs, plus the shared
core files. It is the proposed unit of browser download; not yet implemented.
_Avoid_: datadir (the whole closure), package (the whole edition).

**Content identity**:
The digest in the invitation that binds the guest's bytes to the host's.
_Avoid_: version, commit — those identify the engine, not the data.

## Co-op session

**Session**:
One hosted game: a mission, its seats and the transport between them. Not an
agent session.
_Avoid_: match, game, server.

**Seat**:
A player's place in a session, identified by `PlayerId`; seat 0 is the host. It
survives a reconnect and is never a machine's address.
_Avoid_: player slot, client, peer (a peer is a connection).

**Native host**:
The player process that owns the session and simulates the authoritative world.
_Avoid_: server, dedicated server — it is a game process, not a service.

**Browser guest**:
A browser that joins a hosted session; it cannot create or host one.
_Avoid_: browser host, web player that hosts.

**Invitation**:
A host-signed `rhmp3` ticket naming the host, relay, content edition and native
content identity. It may be time-limited or non-expiring.
_Avoid_: link (the URL merely carries it), endpoint ID (native-only join).

**Endpoint id**:
The public half of an install's persistent key, and so a peer's address on
iroh: known before it connects.
_Avoid_: address, IP, peer id.

**Relay**:
The HTTPS path a browser guest's traffic takes, and a native peer's fallback.
The only browser transport.
_Avoid_: proxy, server.

**Lobby**:
The published pre-mission room: co-op rules, seats and mission choice, opened
before a session exists.
_Avoid_: match, room.

## Determinism

**Tick**:
One step of the deterministic simulation; the unit frames, replays and state
hashes count in.
_Avoid_: update, step.

**State hash**:
The per-frame digest of deterministic state that peers compare. A mismatch is
a desync.
_Avoid_: checksum, digest.

**Rollback**:
Re-simulating frames already run when a late input arrives, so every peer
reaches the same state.
_Avoid_: rewind, resync.

**Desync**:
Two simulations of one session disagreeing at a frame, proved by a state-hash
mismatch.
_Avoid_: divergence (that is a deliberate difference from the original).

**Replay**:
A recorded session (`*.rhrec.jsonl`) that re-runs the same inputs. The default
way to reproduce a gameplay defect.
_Avoid_: recording, demo, log.

**Parity**:
Agreement between two runs that must be identical: the port against the
original game's traces, or Linux against Windows replay hashes.
_Avoid_: equivalence, match.

## Proving and landing

**Suite**:
A named gate target of `scripts/check-quality.sh`; `docs/TESTING.md` maps every
crate to the suite that covers it.
_Avoid_: test set, job.

**Gate**:
The suites a landing owes, run on the build host before the push. One gate is
in flight across all agents.
_Avoid_: CI (that is the remote workflow), check.

**Build host**:
The machine that compiles and runs suites (`minipc` by default); the runtime
and the seats do not build this tree.
_Avoid_: CI, server, build machine.

**Copy**:
A worktree's tree on the build host under a per-copy lock; `gate-shared` is the
one copy a gate may use.
_Avoid_: clone, cache.

**Harness row**:
One `Hnn` scenario in `docs/multiplayer/HARNESS.md`: its setup, fault and
asserts, and the coverage-matrix cell it proves.
_Avoid_: test case, scenario (the scenario file is the row's binding).

**Defect row**:
One diagnosed defect in `docs/DEFECTS.md`: class, reproduction, the check that
proves it, and status. The row, not another rerun, is the unit of work.
_Avoid_: bug, issue, ticket.

**Defect class**:
The shape of a defect — `wait`, `authority`, `desync`, `generation`,
`identity`, `presentation`, `release`, `data` — which decides the fix's
mechanism.
_Avoid_: category, type, severity.

**Terminal state**:
How an unattended iteration ends: `LANDED`, `FILED` or `BLOCKED`. There is no
fourth state and no question to the owner.
_Avoid_: outcome, status, result.

**Integration ref**:
`fork/main`, the branch a landing lands on and the only source for scripts and
docs while working.
_Avoid_: main, upstream, origin.

**Read mirror**:
The shared checkout's `main`: it follows the upstream project and is never
committed to.
_Avoid_: checkout (the directory), mirror branch.

**Worktree**:
`.worktrees/<name>`, one isolated checkout per task, cut from the integration
ref.
_Avoid_: checkout, clone, branch.

**Landing**:
`scripts/land.sh`: fetch, rebase onto the integration ref, gate the tree, push,
and read the remote back. A push's exit status is not a landing.
_Avoid_: merge, push, deploy.

**Slop doc**:
A subsystem evidence document under `docs/slop/`: what was measured, against
which tree, and what remains unproved. Evidence, not a guide.
_Avoid_: notes, research doc.
