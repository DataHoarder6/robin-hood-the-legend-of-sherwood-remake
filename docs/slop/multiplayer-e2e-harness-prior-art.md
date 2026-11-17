# Multiplayer E2E harness prior art (2026-11-17)

Research date: 2026-11-17. Target: a two-to-three peer rig that runs the real
`robin` process on Linux CI and on a Windows player machine, isolates peers by
save directory (and by network namespace on Linux), and drives them through the
loopback HTTP API and `--spec`.

Nothing here was executed against those external tools. Capabilities are read
from vendor documentation, man pages and source, and each claim links the page
that owns it.

## What this repo already provides

- **Frame/command runner.** `--headless --mission <M> --spec <PATH>` stages
  externally tagged `PlayerCommand` JSON at fixed frames and exits at the frame
  budget; assertions are deliberately absent and are pinned by the state-hash
  surface instead. Format: `crates/robin_rs/src/scenario_spec.rs`, docs:
  `docs/TESTING.md` "Scenario specs (L2)". The CLI currently rejects `--spec`
  together with `--server`, `--connect`, `--join`, `--replay` and
  `--start-paused` (`crates/robin_rs/src/main_entry/cli.rs`), so a multiplayer
  spec cannot reuse that flag as-is.
- **Control surface.** `docs/JSON_SERVER.md` documents the loopback server:
  `/step-forward`, `/go-to-frame`, `/set-paused`, `/command`, `/native`,
  `/batch`, `/state`, `/engine-dump`, `/get-replay`, `/load-replay`,
  `/screenshot`. The discovery document at `GET /` is the route list of record.
- **A written manual procedure.** `docs/MULTIPLAYER.md` already specifies the
  checks a native two-peer run must satisfy: no `multiplayer DESYNC`, matching
  hashes at 0/25/50/…, identical server order for both seats' commands, a late
  input rolling back to the same hash, and reconnect reusing the seat. It also
  warns that stepping empty frames proves snapshot agreement but not the
  command surface.
- **A browser peer driver.** `wasm-www/scripts/live-relay-e2e.mjs` drives a real
  Chrome over CDP against a native host and asserts relay welcome, input
  delivery, forced transport loss, replacement snapshot, post-reconnect input
  and replay ownership. It is the shape to copy: one driver, external
  assertions, artifacts on failure.
- **Replay corpus tooling.** `scripts/capture_fresh_mission_replays.py` and the
  `replay_*` scripts already run many fresh processes with private working
  directories and publish only complete traces. The harness should feed this
  corpus rather than invent a second one.

## (a) Declarative scenario formats and how they assert

| System | Declarative unit | Assertion mechanism | What transfers |
| --- | --- | --- | --- |
| Unreal **Gauntlet** | A *session*: a list of roles (`Server`, `Client` × N) with per-role platform, build, command line and controller. | Game-side `UGauntletTestController` calls `EndTest(ExitCode)`; UAT promotes it to a test result. Log/crash parsing is built in. | Split "what processes exist" from "what each process does". The driver never needs to understand gameplay. |
| Unreal **Automation Spec** | `Describe`/`It` with `BeforeEach`/`AfterEach`; `LatentIt` for multi-frame work. | `TestTrue`/`TestEqual` inside `It`; each `It` is independently runnable. | Latent steps for anything that spans frames; one expectation per named sentence. |
| Unity **Multiplayer Play Mode** | A *Play Mode Scenario*: named instances (editor, local builds, remote), each with role, tag, initial scene and launch arguments. | In-game test code; the scenario only defines topology and lifecycle. | Scenario as data, not code: name, instances, role, args. |
| **Jepsen** | `generator` (operation schedule) + `nemesis` (fault schedule) + `checker`. | A recorded `history` is checked against a model; `:ok`/`:fail`/`:info` distinguish definite effects from unknown outcomes. | Separate stimuli from faults from oracles; treat "no answer" as `:info`, never as failure. |
| **DST** (FoundationDB, TigerBeetle VOPR, Antithesis) | `(build, seed)` selects workload, fault classes and timings. | Invariants and state checkers, not golden outputs. | Replayability is `(build, seed)`; every run must be reproducible from the report. |
| This repo (`--spec`) | Frame budget + frame-indexed commands. | None in-layer; hash surface. | Keep the command JSON; add the missing peers, faults and checkpoints. |

Sources: Gauntlet overview and roles
(<https://dev.epicgames.com/documentation/unreal-engine/gauntlet-automation-framework-overview-in-unreal-engine>),
Gauntlet controllers
(<https://dev.epicgames.com/documentation/unreal-engine/gauntlet-controller-in-unreal-engine>),
networked-game testing with Gauntlet
(<https://dev.epicgames.com/documentation/unreal-engine/testing-and-debugging-networked-games-in-unreal-engine>),
Automation Spec
(<https://dev.epicgames.com/documentation/unreal-engine/automation-spec-in-unreal-engine>),
Play Mode scenario configuration
(<https://docs.unity3d.com/Packages/com.unity.multiplayer.playmode@2.0/manual/play-mode-scenario/play-mode-scenario-configurations.html>)
and window reference
(<https://mp-docs.dl.it.unity3d.com/mppm/current/mppm-reference/play-mode-scenario-window-reference/index.html>),
Jepsen README (<https://github.com/jepsen-io/jepsen>) and history semantics
(<https://github.com/jepsen-io/history>), FoundationDB simulation
(<https://apple.github.io/foundationdb/testing.html>), VOPR
(<https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vopr.md>),
Antithesis DST (<https://antithesis.com/docs/resources/deterministic_simulation_testing>).

**Adopt.** A session spec whose top level is `peers` (seat, role, data dir,
HTTP port, connect target), then per-seat `commands` keyed by frame, then a
`faults` schedule keyed by frame or wall-clock, then `checkpoints` naming the
oracle. Keep `PlayerCommand` JSON unchanged. Do not put assertions in the game:
assert from the driver against `/state`, `/engine-dump` and replays, matching
the current L2/L3 split.

## (b) Replay corpora as regression tests

- **OpenTTD** logs every client action to `commands-out.log`, autosaves at map
  construction (`dmp_cmds_*.sav`), and replays the log against the savegame;
  the replay compares the checksums embedded in the log and traps on mismatch.
  The docs treat "replay reproduces the original server state" as the test and
  use progressively later savegames to bisect a desync
  (<https://github.com/OpenTTD/OpenTTD/blob/master/docs/desync.md>). The
  debugging guide defines the levels and the artifacts a report must carry
  (<https://github.com/OpenTTD/OpenTTD/blob/master/docs/debugging_desyncs.md>).
- **0 A.D.** records `commands.txt` plus a metadata file, and its replayer
  exposes `serializationtest`, `rejointestturn`, `ooslog`, `testHashFull` and
  `testHashQuick` — i.e. the replay is simultaneously a determinism test, a
  rejoin test and a hash test
  (<http://docs.wildfiregames.com/pyrogenesis/Replay_8h_source.html>). The
  engine's `-replay=PATH` runs non-visually
  (<https://manpages.debian.org/testing/0ad/pyrogenesis.6.en.html>).
- **OpenRA** ships `Server.RecordReplays` and a `--replay-metadata` utility,
  but its own test gate is YAML lint plus a small unit suite (Makefile `test`
  vs `tests`) — a cautionary example: replay recording without a corpus gate
  buys little
  (<https://github.com/OpenRA/OpenRA/blob/bleed/launch-dedicated.sh>,
  <https://github.com/OpenRA/OpenRA/blob/bleed/Makefile>).
- **ggrs** treats replay as a local rollback check: `SyncTestSession` saves,
  advances, rewinds `check_distance` frames, re-simulates and compares
  checksums, panicking on mismatch
  (<https://docs.rs/ggrs/latest/ggrs/struct.SyncTestSession.html>). That is the
  same oracle this project already has as periodic state-hash verification.
- **DST** makes the corpus a seed: FoundationDB and TigerBeetle replay a
  failure from `(commit, seed)`; Jepsen stores the history and re-runs the
  checker later (<https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vopr.md>,
  <https://github.com/jepsen-io/jepsen>).

**Adopt.** One artifact set per scenario run: session spec, fault seed, each
peer's `.rhrec.jsonl`, per-peer hash manifest at checkpoints, and logs. The
regression gate is: same spec + same seed reproduces the same per-frame hash
sequence on every peer and every platform. That is stronger than screenshot
comparison and cheaper than re-deriving expectations.

## (c) Network fault injection, and what it means for QUIC

### Linux: `tc` + `netem`

`netem` is a queue discipline on a device, so it impairs packets without
understanding the transport above them — QUIC is opaque UDP to it. It offers
delay/jitter with correlation and a distribution, loss (Bernoulli, 4-state
Markov, Gilbert–Elliot) with optional ECN marking, corruption, duplication,
reordering (percentage or `gap`, and **reordering needs delay to be visible**),
rate with per-packet/cell overhead, slot-based burst approximation, a queue
`limit`, and a `seed` for reproducible loss/corruption events
(<https://man7.org/linux/man-pages/man8/netem.8.html>). Documented limits:
kernel timer granularity, reorder interactions, the TCP-small-queues note that
puts netem on the receiver's ingress for realistic TCP results, and qdisc/skb
interactions.

Use it with `ip netns` for peer isolation and a veth pair between namespaces
(<https://man7.org/linux/man-pages/man8/ip-netns.8.html>,
<https://man7.org/linux/man-pages/man4/veth.4.html>). Apply a qdisc on each
peer's veth to shape that peer's egress, or on both ends for independent
directions. A QUIC benchmarking thesis uses exactly this: `ip netns` plus
`tc qdisc netem` for delay, loss and reordering against MsQuic and Quinn
(<http://sppifo.ethz.ch/files/public/theses/2024-quic_benchmark_suite/thesis-1.pdf>).

What netem **can** do for this rig: loss (including bursty Gilbert–Elliot,
which matters far more to QUIC than uniform loss), delay/jitter, duplication,
reorder (with a delay floor), bandwidth/rate limits, ECN marking, reproducible
seeds, and clean bidirectional asymmetry.

What it **cannot** do, and must be driven through the game instead:

- Target a QUIC stream, frame type or packet-number space. Corrupted QUIC
  packets fail AEAD and are discarded — corruption is just expensive loss
  (<https://www.rfc-editor.org/rfc/rfc9001#section-5.5>).
- Emit `CONNECTION_CLOSE`, `STREAM_RESET`, a graceful drain, or an idle
  timeout at a chosen instant. Blackholing works but conflates timeout,
  path failure and congestion.
- Change paths (NAT rebinding, Wi-Fi-to-cellular migration). netem keeps the
  4-tuple; a real migration needs conntrack/NAT or address changes plus policy
  routing.
- Create a PMTUD black hole (ICMP suppressed with an undersized link MTU) or
  exercise QUIC's own loss detection deterministically
  (<https://www.rfc-editor.org/rfc/rfc9002>).
- Model a relay hop, a relay-only path, or a relay failure. Those are
  application scenarios, not link scenarios.

### `toxiproxy`: not usable here

Toxiproxy is "a TCP proxy written in Go"; its toxics are TCP-stream concepts
(latency, bandwidth, slow close, timeout, reset peer, slicer, limit data)
(<https://github.com/Shopify/toxiproxy>). UDP support was requested and closed
as out of scope, with the maintainers noting a layer-3 rewrite would be needed
(<https://github.com/Shopify/toxiproxy/issues/39>). QUIC over UDP cannot be
proxied by it. Do not adopt.

### `pumba`: Linux containers only

Pumba drives `netem`/`iptables` against container network namespaces and
publishes no Windows binary; its README says Windows is unsupported and not
planned (<https://github.com/alexei-led/pumba>). Useful only if the rig ever
runs peers as containers; it adds nothing over `ip netns exec … tc`.

### Windows: `clumsy` on WinDivert

Clumsy intercepts packets with WinDivert, applies lag, drop, throttle,
duplicate, out-of-order, tamper, bandwidth limit and TCP reset, then reinjects;
its README notes it works "even if you're offline (ie, connecting from
localhost to localhost)" (<https://github.com/jagt/clumsy>). WinDivert's filter
language exposes `udp`, addresses, ports and direction, so a QUIC/loopback
filter is expressible
(<https://github.com/basil00/WinDivert>,
<http://jagt.github.io/clumsy/manual.html>). It has documented command-line
flags for filter and per-effect chances, so a driver can start it without the
GUI (<https://github.com/jagt/clumsy/wiki/Command-Line-Arguments>,
<https://github.com/jagt/clumsy/issues/1>).

Limits that matter:

- It loads the WinDivert kernel driver, so it runs as Administrator; the 4.0
  line states this explicitly (<https://github.com/burning-calamity/clumsy-test>).
- Effects are probabilistic percentages with **no documented seed**, so a
  Windows fault run is not reproducible the way `netem seed` is.
- No ECN marking, no Gilbert–Elliot/burst model, no rate/slot model with
  packet overhead, and "reset" is a TCP RST — meaningless to QUIC.
- It is one GUI process; changing effects mid-run from the CLI is still an open
  request (<https://github.com/jagt/clumsy/issues/190>). The practical driver
  shape is: start clumsy with a fixed filter and schedule, run the scenario,
  stop it.

There is no built-in Windows equivalent of `netem`; `tc` inside WSL2 shapes the
WSL VM's interfaces, not a native Windows process's loopback, so it does not
substitute.

### Two complementary tools, explicitly not substitutes

- **iroh's in-memory `TestNetwork`/`TestTransport`** replaces the real network
  transport with in-memory channels for tests. It is gated on
  `test-utils` + `unstable-custom-transports`, and the docs describe
  connectivity, not latency/loss injection
  (<https://docs.rs/iroh/latest/iroh/test_utils/test_transport/struct.TestNetwork.html>).
  It can test protocol/state logic without sockets, but it is not a real-process
  rig and the game's `iroh` dependency does not currently enable those features
  (`crates/robin_rs/Cargo.toml`).
- **`turmoil`** runs multiple hosts in one thread with simulated time,
  partitions, one-way partitions, hold/release and a seeded RNG
  (<https://docs.rs/turmoil/latest/turmoil/>). Excellent for deterministic unit
  tests of session logic; cannot exercise the real iroh/QUIC stack or the HTTP
  control API.
- **A custom `AsyncUdpSocket`** (quinn) or iroh custom transport lets a test
  own packet I/O and time. A published QUIC experiment did this to make
  downloads instant and deterministic, and recorded a synthetic pcap
  (<https://ochagavia.nl/blog/towards-interplanetary-quic-traffic>,
  <https://docs.rs/quinn/latest/quinn/trait.AsyncUdpSocket.html>). This is the
  right long-term hook for QUIC-aware faults, but it is in-process code, not
  netem.

## (d) Headless client and bot orchestration

- **0 A.D.** is the closest analogue: `-autostart-host`,
  `-autostart-client`, `-autostart-players=N`, `-autostart-ip` and
  `-replay=PATH` are on the man page
  (<https://manpages.debian.org/testing/0ad/pyrogenesis.6.en.html>);
  `--autostart-nonvisual` and `--rl-interface` are used by its RL harness
  (<https://brollb.github.io/machine-learning/games/2021/02/02/0ad-rl>).
  A driver can therefore launch a real host and real clients with one command
  line each and no bot implementation. Its
  third-party dedicated server goes further: a sidecar headless engine runs AI
  and rebuilds snapshots for rejoin
  (<https://github.com/victorcrimea/veredus>).
- **HeadlessMc** runs the real Minecraft client with LWJGL patched to render
  nothing, one game directory per profile, and a version-specific mod that
  accepts console commands (`connect`, `key`, chat, `/command`). A multi-client
  manager gives each client its own profile and tracks instances, states, logs
  and screens
  (<https://github.com/headlesshq/headlessmc>,
  <https://headlesshq.github.io/headlessmc/specifics>,
  <https://github.com/headlesshq/headlessmc-mcp>). The transferable rules:
  one private game directory per client, a readiness predicate, and a typed
  command channel rather than synthetic input.
- **Unreal Gauntlet** uses per-role controllers to puppeteer a client and report
  a result; ShooterGame ships a reference automation script plus native test
  controllers
  (<https://dev.epicgames.com/documentation/unreal-engine/gauntlet-controller-in-unreal-engine>).
- **ggrs** ships a three-player example launched as two processes, each with
  `--local-port` and a `--players` list that encodes which seats are local to
  that process (<https://github.com/gschup/ggrs/blob/main/examples/README.md>).
  This is the seat-to-process mapping pattern, expressed on the command line.
- **OpenRA** runs a console dedicated server with bots and a
  `Server.EnableSingleplayer`/bot policy, so the "server" peer needs no
  renderer at all
  (<https://github.com/OpenRA/OpenRA/wiki/Dedicated-Server>).

**Adopt.** No new bot code. Every peer is the real game in `--headless` mode.
The host runs `--server --mp-expected-players N`; joiners run
`--connect <EndpointAddr JSON>`; the driver issues `PlayerCommand`s with
`POST /command` and advances with `/step-forward` / `/go-to-frame`. Bots are
just another command schedule in the session spec.

## (e) Prior art for one driver over many real processes

- **Gauntlet** is the canonical shape: `ITargetDevice`/`IAppInstance` wrap a
  process; `UnrealSessionRole`/`UnrealSession` describe "N clients and a
  server"; `TestExecutor` runs tests in parallel and the docs call out that
  parallel tests must use unique server/client ports. It also collects logs,
  crashes and saved data from each instance
  (<https://dev.epicgames.com/documentation/unreal-engine/gauntlet-automation-framework-overview-in-unreal-engine>).
- **Unity MPPM** turns the same idea into editor UI: scenario = instance list
  with role/tag/build profile/arguments, launched together, logs streamed to
  one console; `ParrelSync` is the pre-Unity-6 clone-based equivalent, sharing
  assets by symlink while keeping a private `Library` per clone
  (<https://docs.unity3d.com/Packages/com.unity.multiplayer.playmode@2.0/manual/play-mode-scenario/play-mode-scenario-configurations.html>,
  <https://github.com/VeriorPies/ParrelSync>). The known failure mode is
  shared mutable state: MPPM needs separate processes, ParrelSync forbids edits
  in clones.
- **Jepsen** separates a control node from the nodes under test, with a
  `nemesis` process injecting faults on a schedule while clients record a
  history (<https://github.com/jepsen-io/jepsen>). Faults are a first-class
  process, not a side effect of test code.
- **HeadlessMc** demonstrates per-instance profiles, a launch registry, log
  tailing, readiness waits (`wait_for`) and stop/cleanup in one controller
  (<https://github.com/headlesshq/headlessmc-mcp>).

Common shape, all five: **supervisor → per-instance private directory and
ports → readiness probe → stimulus/fault schedule → artifact collection →
oracle**. That is the whole rig. Nothing in the prior art drives peers by
shared memory or by one process pretending to be several.

## Recommended rig for this repo

1. **Driver.** One test binary (`crates/robin_rs/tests/…` or a `just` recipe)
   owns process supervision. It spawns the host, reads its startup log for the
   endpoint id and bound address, then spawns joiners with a JSON `EndpointAddr`
   so no DNS/DHT/relay lookup is needed; `parse_connect_addr` already accepts
   that form (`crates/robin_rs/src/multiplayer/identity.rs`).
2. **Isolation.** Per peer: `ROBINHOOD_SAVE_DIR=<tmp>/save` (saves, identity
   key, preferences — `crates/robin_rs/src/save_file.rs`,
   `crates/robin_rs/src/native_game_identity.rs`,
   `crates/robin_rs/src/localization.rs`) and, on Linux,
   `XDG_DATA_HOME=<tmp>/data` for the replay archive
   (`crates/robin_rs/src/mission_replays/native.rs`). Separate HTTP ports.
3. **Network.** Linux: one netns per peer, veth pair, qdisc per peer; fault
   schedule compiled to `tc` changes with a `seed`. Windows: clumsy with a
   `udp and (ip.DstPort == … or ip.SrcPort == …)` filter and a fixed effect
   set; accept it as a smoke path, not a reproducible gate.
4. **Stimulus.** Session spec JSON: peers, per-seat frame-indexed commands,
   fault schedule, checkpoint frames. Reuse `PlayerCommand` and the existing
   `--spec` validation rules; extend the runner rather than inventing a second
   command language.
5. **Oracles.** At each checkpoint: `/state` frame equality across peers, hash
   equality (the existing periodic hash surface), `/engine-dump` diff on
   mismatch, then replay equality after the run. Add reconnect and late-input
   cases from `docs/MULTIPLAYER.md` as named scenarios.
6. **Corpus.** Every run writes spec + seed + hashes + each peer's
   `.rhrec.jsonl`. Re-run the corpus as a regression gate; treat a hash change
   as a failure unless the spec records a new expectation.

Do not adopt: toxiproxy (TCP), pumba (containers only), in-process `turmoil` or
iroh `TestNetwork` as the *E2E* rig (they cannot prove the real transport,
process split or HTTP surface), or a hand-rolled packet proxy on Windows when
WinDivert already solves loopback interception.

## Open questions for wayfinder

1. **Spec vs multiplayer CLI.** `--spec` conflicts with `--server`/`--connect`.
   Extend `MissionSpec` into a session spec, or leave `--spec` single-player
   and make the driver own the command schedule over HTTP?
2. **Lockstep semantics.** `/step-forward` is per process. Does the host's
   scheduling delay guarantee it never outruns a joiner's input for a stepped
   frame, or is a per-seat "inputs pending / ready" endpoint needed before
   unattended lockstep is sound? `docs/MULTIPLAYER.md` only describes stepping
   "in matching increments".
3. **Per-install state on Windows.** Replays resolve through
   `dirs::data_dir()`, which on Windows is the Known Folder API, not `%APPDATA%`
   (<https://docs.rs/dirs/latest/dirs/fn.data_dir.html>; `dirs 7.0.0` in
   `Cargo.lock`). Two peers on one Windows account therefore share
   `robin_hood/replays`. Should all per-install state honour one override env
   var?
4. **CI privileges.** Does the CI runner allow `ip netns add` (CAP_SYS_ADMIN)
   or `unshare -Urn` with `tc` inside the user namespace? The fault tier must
   fail loudly on a runner that cannot do it, not silently pass.
5. **Hermetic session bootstrap.** With `EndpointAddr` JSON, can the rig avoid
   n0 DNS, Mainline DHT and the public relay entirely, including for
   matchmaking and `--join`? If not, which local relay/discovery substitute is
   supported?
6. **Cross-platform determinism.** OpenTTD found a desync that reproduced only
   with an MSVC client against a GCC server
   (<https://github.com/OpenTTD/OpenTTD/issues/14677>). Should the corpus gate
   include a Linux-replay × Windows-replay hash comparison, or is same-platform
   determinism sufficient?

## Sources

Primary docs and source used above, in one place:

- Unreal Gauntlet: <https://dev.epicgames.com/documentation/unreal-engine/gauntlet-automation-framework-overview-in-unreal-engine>,
  <https://dev.epicgames.com/documentation/unreal-engine/gauntlet-controller-in-unreal-engine>,
  <https://dev.epicgames.com/documentation/unreal-engine/testing-and-debugging-networked-games-in-unreal-engine>,
  <https://dev.epicgames.com/documentation/unreal-engine/automation-spec-in-unreal-engine>
- Unity: <https://docs.unity3d.com/Packages/com.unity.multiplayer.playmode@2.0/manual/play-mode-scenario/play-mode-scenario-configurations.html>,
  <https://mp-docs.dl.it.unity3d.com/mppm/current/mppm-reference/play-mode-scenario-window-reference/index.html>,
  <https://docs-multiplayer.unity3d.com/netcode/current/tutorials/testing/testing_locally/index.html>,
  <https://github.com/VeriorPies/ParrelSync>
- HeadlessMc: <https://github.com/headlesshq/headlessmc>,
  <https://headlesshq.github.io/headlessmc/specifics>,
  <https://github.com/headlesshq/headlessmc-mcp>
- Linux netem/netns/veth: <https://man7.org/linux/man-pages/man8/netem.8.html>,
  <https://man7.org/linux/man-pages/man8/ip-netns.8.html>,
  <https://man7.org/linux/man-pages/man4/veth.4.html>
- QUIC: <https://www.rfc-editor.org/rfc/rfc9001#section-5.5>,
  <https://www.rfc-editor.org/rfc/rfc9002>,
  <https://ochagavia.nl/blog/towards-interplanetary-quic-traffic>,
  <https://docs.rs/quinn/latest/quinn/trait.AsyncUdpSocket.html>,
  <http://sppifo.ethz.ch/files/public/theses/2024-quic_benchmark_suite/thesis-1.pdf>
- Windows fault injection: <https://github.com/jagt/clumsy>,
  <https://github.com/jagt/clumsy/wiki/Command-Line-Arguments>,
  <https://github.com/jagt/clumsy/issues/1>,
  <https://github.com/jagt/clumsy/issues/190>,
  <http://jagt.github.io/clumsy/manual.html>,
  <https://github.com/burning-calamity/clumsy-test>,
  <https://github.com/basil00/WinDivert>
- Proxies and containers: <https://github.com/Shopify/toxiproxy>,
  <https://github.com/Shopify/toxiproxy/issues/39>,
  <https://github.com/alexei-led/pumba>
- Replay corpora: <https://github.com/OpenTTD/OpenTTD/blob/master/docs/desync.md>,
  <https://github.com/OpenTTD/OpenTTD/blob/master/docs/debugging_desyncs.md>,
  <https://github.com/OpenTTD/OpenTTD/issues/14677>,
  <http://docs.wildfiregames.com/pyrogenesis/Replay_8h_source.html>,
  <https://manpages.debian.org/testing/0ad/pyrogenesis.6.en.html>,
  <https://brollb.github.io/machine-learning/games/2021/02/02/0ad-rl>,
  <https://github.com/victorcrimea/veredus>,
  <https://github.com/OpenRA/OpenRA/blob/bleed/launch-dedicated.sh>,
  <https://github.com/OpenRA/OpenRA/blob/bleed/Makefile>,
  <https://github.com/OpenRA/OpenRA/wiki/Dedicated-Server>
- Rollback/determinism: <https://docs.rs/ggrs/latest/ggrs/struct.SyncTestSession.html>,
  <https://github.com/gschup/ggrs/blob/main/examples/README.md>
- DST and history checking: <https://apple.github.io/foundationdb/testing.html>,
  <https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vopr.md>,
  <https://antithesis.com/docs/resources/deterministic_simulation_testing>,
  <https://github.com/jepsen-io/jepsen>,
  <https://github.com/jepsen-io/history>,
  <https://docs.rs/turmoil/latest/turmoil/>
- iroh: <https://docs.rs/iroh/latest/iroh/test_utils/test_transport/struct.TestNetwork.html>,
  <https://docs.rs/iroh/latest/iroh/test_utils/fn.run_relay_server.html>,
  <https://github.com/n0-computer/iroh/blob/main/iroh-relay/README.md>
- `dirs` platform paths: <https://docs.rs/dirs/latest/dirs/fn.data_dir.html>
