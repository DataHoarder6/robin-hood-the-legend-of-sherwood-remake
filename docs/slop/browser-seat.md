# A browser seat: can it join today, and what proves it

Written 2026-11-19 against `fork/main` `ac74b98c0a90`, re-read twice the same
day: section 5 and items 1-2 of section 6 after the reconnect-attempt and
content-write holes were closed, then the artifact section after the build plan
was fixed. Re-read again at `81264ba8929b`, when the join driver first passed
and section 10 replaced its own earlier reading of the modal barrier. The
production changes this file caused are the build plan, two wasm compile fixes,
the compatibility export's single source, the budget bounds and the join driver
itself; the devShell's wasm C compiler belongs to
[the wasm build evidence](wasm-build-host.md). The owner's question is
whether a browser (wasm) seat can join a multiplayer session today, and what
exactly proves it. Adjacent material this file links rather than restates: [the
harness contract](../multiplayer/HARNESS.md) (what a row may name),
[multiplayer waits without a deadline](multiplayer-unbounded-waits.md) (the wasm
rows), [the terminal matrix](multiplayer-terminal-matrix.md) (H21–H26, the rows
a browser row would sit beside), [the multiplayer architecture](../MULTIPLAYER.md).

## Verdict

| Direction | State (2026-11-19, after the join run) | What decides it |
| --- | --- | --- |
| native host + browser client, the repo's own artifact | **join proved; mission play unproved** | §10: a browser seat joins a native host through the public relay, is welcomed as seat 1, adopts the frame-0 snapshot, and matches the host's canonical state hash at every 25-frame comparison up to frame 350 with zero desyncs. What stays unproved is *play*: the Demo mission's own script reports defeat on its first tick, with one seat and no browser at all (§10.5), so the run replays a lost mission rather than a played one |
| native host + browser client, before this change | **impossible** | the artifact every repo script built had no multiplayer transport, so the join died before it reached a relay |
| browser host + native client | **impossible by construction** | browser hosting is not implemented and its server handle type has no values (`crates/robin_rs/src/multiplayer/wasm/native_only.rs:25-26`, `crates/robin_rs/src/game_session/multiplayer/session_setup.rs:35-38`) |
| browser reconnect, no hang | **bounded 2026-11-19** | a reconnect attempt carries a deadline of its own, so the 60 s give-up budget is reachable while an attempt is in flight (`crates/robin_rs/src/multiplayer/wasm.rs:197`, `crates/robin_rs/src/multiplayer/client_session.rs:617-637`, `:655-673`); section 6 still lists three wasm ways to hang that no attempt bound fixes |

Short names below; every path is repo-relative.

| Short name | Path |
| --- | --- |
| `wasm.rs`, `client_session.rs`, `native/client.rs` | `crates/robin_rs/src/multiplayer/` |
| `enabled.rs`, `native_only.rs` | `crates/robin_rs/src/multiplayer/{enabled.rs,wasm/native_only.rs}` |
| `session_setup.rs` | `crates/robin_rs/src/game_session/multiplayer/session_setup.rs` |
| `cli.rs`, `robin.rs` (bin) | `crates/robin_rs/src/main_entry/cli.rs`, `crates/robin_rs/src/bin/robin.rs` |
| `server.rs` | `crates/robin_rs/src/multiplayer/native/server.rs` |
| `boot-lifecycle.ts`, `main.ts`, `join_ticket.ts`, `multiplayer_identity.ts`, `multiplayer_content.ts` | `wasm-www/src/` |
| `browser-game.mjs`, `build-runtime.mjs` | `wasm-www/scripts/` (the first: `scripts/validation/browser-game.mjs`) |

## 1. What a browser seat is, in five facts the code enforces

1. **Ticket or nothing.** The browser client dials a host endpoint decoded from a
   signed `rhmp3` invitation; there is no address book, no rendezvous, no
   matchmaking. `connect_client` decodes the ticket before it can bind
   (`wasm.rs:61-63`), and the invitation must be `https`, canonical and 30
   minutes old at most (`join_ticket.ts:9-10`, `:183-193`, `:160-162`). A browser
   `--connect <endpoint id>` cannot work: the same field carries the ticket.
2. **Relay only.** A browser endpoint cannot use UDP, so every byte goes through
   the one HTTPS relay named in the ticket (`wasm.rs:1-14`, `:95-109`), and the
   client refuses to proceed unless `endpoint.online()` completes in 15 s
   (`wasm.rs:111-125`). The host side of the same requirement: with browser links
   on, the host refuses to start if its relay is not reachable in 15 s
   (`server.rs:659-677`).
3. **The artifact is pinned to the host's commit.** The build number comes from
   the ticket's `engine_version`, not from `/wasm/latest.json`
   (`main.ts:247-254`), and the shell rejects an artifact whose
   `engineCommit`/`artifactShort`/`netProtocol`/`ticketSchema` differ
   (`boot-lifecycle.ts:128-152`). A browser seat therefore needs a published
   `/wasm/<host commit>/` build of exactly the host's source tree.
4. **Content equality is explicit.** Either the Demo closure's
   `nativeContentSha256` equals the ticket's content identity
   (`multiplayer_content.ts:381-389`), or a Full package manifest hash must
   match; missing, extra or stale bytes stop before boot. The host can only mint
   that identity when its content is an unmodified source tree or a verified web
   package (`content_identity.rs:131-177`).
5. **The identity signer is a separate origin.** Seat proof is an Ed25519
   signature produced by an iframe on `https://identity.robinhood.phiresky.xyz/`
   (`multiplayer_identity.ts:7`, `:38-59`, `:92-129`), and the signer must not be
   the page's own origin (`:55-57`).

The browser menu adds one more entry point — `?connect=<ticket>` as a direct
invite listing (`crates/robin_rs/src/main_menu/multiplayer_menu.rs:1795-1832`) —
but it decodes the same ticket, so it changes nothing about the five facts.

## 2. The build plan: what it did, what it does, and what that proves

Until 2026-11-19 the one build plan for a browser runtime passed `audio` and,
threaded, `audio,wasm-threads` — never `multiplayer`:

```
wasm-www/scripts/build-runtime.mjs:13-15 (before this change)
    else cargo.push('-p', 'robin_rs', '--bin', 'robin', '--no-default-features',
        '--features', threads ? 'audio,wasm-threads' : 'audio');
```

`multiplayer` is what mounts `wasm.rs` at all: `enabled.rs:42-46` selects the
relay client only under `#[cfg(target_arch = "wasm32")]` **inside** the
feature-gated module, and the no-feature build substitutes
`multiplayer/disabled.rs` plus `session_setup_disabled.rs`. Production staging
took that same plan (`wasm-www/scripts/stage-runtime-addition.mjs:13`, used by
`.github/workflows/build-static-runtime.yml:95`), so the deployed
`/wasm/<commit>/robin_bg.wasm` had no client either.

What a browser did with that artifact, in order:

1. `wasm_multiplayer_compatibility()` returned `ticketSchema: 0` because the
   feature was off, so `assertMultiplayerWasmCompatibility` threw `loaded
   browser artifact does not exactly match the host-signed invitation`
   (`boot-lifecycle.ts:146-151`). This happened before the ticket was
   installed.
2. Had the schema matched, `wasm_set_multiplayer_join_ticket` returned
   `browser multiplayer is unavailable in this build; rebuild with
   --features multiplayer` (`cli.rs:363-368`).
3. Had the mission started anyway, `establish` would answer with
   `multiplayer was requested but is unavailable in this build`
   (`session_setup_disabled.rs:16-19`) — and `resolve_join_ticket` refuses the
   route one layer earlier for the same reason (`cli.rs:436-446`).

### The plan now carries the client

`RUNTIME_FEATURES` in `wasm-www/scripts/build-runtime.mjs` is
`audio,multiplayer`; the threaded plan appends `wasm-threads`. Both the dev
artifact and the staged production addition inherit it, because staging calls
the same `buildRuntime`. The choices, from the code:

- **`multiplayer` with `wasm-threads` works.** They are independent: threads
  selects the atomics/shared-memory build (`scripts/wasm-threads.cargo-config.toml`)
  and `robin_assets/wasm-threads`; `multiplayer` adds iroh. The threaded
  `wasm-release` artifact builds and exports the same contract as the plain one.
- **No relay host is baked in.** The relay route is decoded from the signed
  ticket (`wasm.rs:61-63`) and `presets::N0` names iroh's preset
  (`wasm.rs:95-109`), not a host in this repository. The one fixed origin in
  the browser path is the identity signer
  (`wasm-www/src/multiplayer_identity.ts:7`, default
  `https://identity.robinhood.phiresky.xyz/identity-signer/`), which is a page
  build input (`VITE_IDENTITY_SIGNER_URL`) and pre-exists this feature change.
- **The contract gains a real value, not a new field.** The published
  manifest already carried `"ticketSchema": 3` from
  `wasm-www/runtime-contract.json`; only the artifact disagreed, because the
  export hard-coded `0` for a build without the feature. The manifest was
  right and the binary was the lie.
- **Size, measured on the same pipeline** (`wasm-release`, `-Oz`, stripped;
  commit `e4f3712bbf0a`): `audio` alone 19,094,019 B; `audio,multiplayer`
  21,323,868 B (+2,229,849 B, 11.7%); the production `audio,multiplayer,wasm-threads`
  plan 21,474,874 B, so the thread mode adds 151,006 B on top.

### 2.1 Proving it without a browser

`wasm-www/scripts/verify-runtime-multiplayer.mjs` instantiates an artifact's
own `robin.js` glue under Node, calls `wasm_multiplayer_compatibility()`, and
exits non-zero unless `ticketSchema` equals `--expect-ticket-schema`. Run
against the three builds of commit `e4f3712bbf0a`:

| Build plan | `robin_bg.wasm` sha256 | Export |
| --- | --- | --- |
| `--features audio` (the pre-fix artifact) | `f4edf0390e9c33673cfd08e8b09ad951272e485190cfe8e565ebed8f22f03693` | `"ticketSchema":0` |
| plan default (`audio,multiplayer`) | `7abad049da4106a8f122c6eaa4cd822b3b058dda052fdd924b4c70b68a644ac2` | `"ticketSchema":3` |
| plan `--threads` (production: `audio,multiplayer,wasm-threads`) | `3c3d47161f1f9824fd4fe96dcb86b3858ad571365af6f26dc818de6122d068ad` | `"ticketSchema":3` |

```
$ node wasm-www/scripts/verify-runtime-multiplayer.mjs --pkg target/wasm-proof/pkg-mp \
    --expect-ticket-schema 3
{"engineCommit":"e4f3712bbf0ae3080675dff2e8e1af61fdbb7538","artifactShort":"e4f3712bbf0a","netProtocol":72,"ticketSchema":3}
runtime multiplayer compatibility: ticketSchema 3 as required
```

The same command with `--expect-ticket-schema 0` against the `audio`-only build
prints `"ticketSchema":0` and exits 0; against either multiplayer build it
fails, so the difference is a command's exit status rather than a claim in a
document. Staging runs this check on the optimized bytes before it writes
`manifest.json` (`stage-runtime-addition.mjs`), so an artifact whose export
disagrees with the published contract cannot be published.

The Rust half is a lib test that runs in both gated feature sets:
`runtime_contract::tests::browser_compatibility_names_the_schema_this_build_can_redeem`
asserts `0` under `client` (default features) and the manifest's schema under
`client-release` (`release` includes `multiplayer`), plus
`browser_compatibility_serializes_in_the_canonical_shell_order` for the key
order the shell compares.

Two wasm compile failures sat between "the code type-checks" and "the bin
builds", and both are fixed:

- `main_entry/cli.rs` gave `--spec` both `#[arg(skip)]` on wasm and `arg(long)`,
  which clap rejects: **no** wasm32 build of `robin` compiled, feature or not.
- `multiplayer/wasm/native_only.rs` declared the browser `ServerHandle::connect_string`
  as `Option<String>` while the shared call site (`enabled.rs:101`) wraps the
  native `String` in `Some`. The `browser-audio` gate's
  `--features audio,multiplayer` check failed on this before any artifact existed.

A cold `wasm`/`browser-audio` build also failed in `ring`/`zstd` C compilation
because the devShell exported the host `CC=gcc`; the shell now sets
`CC_wasm32_unknown_unknown` to the unwrapped clang
([the wasm build evidence](wasm-build-host.md) owns that fix), so the wasm
target builds from a cold tree.

So the failure is a build-feature refusal, not a network or protocol failure:
there is no relay traffic to time out. The `wasm` and `browser-audio` gates prove
only that this code **compiles** — `cargo check --bin robin --tests` with
`audio,multiplayer`, then one linked test module run in Chrome
(`scripts/validation/lifecycle_gate.py:133-178`, `docs/TESTING.md:65-66`). The
browser tests in that module are the shared protocol and identity unit tests, not
a join (`lifecycle_gate.py:25-31`).

## 3. The browser harness, and what it actually asserts

| Harness | What it starts | What it dials | What it asserts | Verdict |
| --- | --- | --- | --- | --- |
| `scripts/validation/browser-game.mjs` | `google-chrome --headless=new` against a Node HTTPS server on `127.0.0.1:0`, self-signed cert with SANs for both game names (`:40-42`, `:63-97`, `:127-130`) | Single-player mode: every hostname to loopback. Join mode: only the game and signer hostnames to loopback; the ticket's relay resolves normally (`--host-resolver-rules` in the launch line) | Single-player Demo boot, first mission state, replay export through the real wasm admission worker, pause-menu replay restart, bfcache restore, no console "required mission text" errors, no desync lines (`:252-398`). Join mode additionally: the shell's manifest check, the browser's own `state`/`events` RPC, the host's `/state`/`/events`, canonical-hash parity, and the host's reaction to the page vanishing | join mode is the recorded run in §10; the single-player acceptance path is unchanged |

In single-player mode the `browser-game.mjs` isolation is deliberate and total:
`MAP * 127.0.0.1:${port}` sends *every* hostname to the fixture server, and the
handler serves only two names. A relay is a third name and would be answered
`local acceptance: denied` (`:67`). Join mode is the one mode that leaves the
loopback: it maps only the two page origins and lets the relay resolve, because
a browser seat's transport is the public relay and nothing in the ticket names a
local one (§10.1).

`wasm-www/scripts/live-relay-e2e.mjs` was the only browser-join driver, and it was
deleted on 2026-11-19. It failed at its first assertion: it waited for

```
browser welcomed through iroh WebSocket relay          (wait at 180 s)
browser multiplayer session ended; reconnecting…       (waits at 45 s)
```

and neither string exists in the game — the only occurrences in the tree were the
driver's own (`grep -rn "welcomed through iroh" --glob '!target' .` → two hits,
both in `live-relay-e2e.mjs`). The code logs `browser multiplayer client
welcomed` at debug level (`wasm.rs:263-268`) and `client session ended:
{reason}; reconnecting...` (`client_session.rs:795`). Three other waits did match
real lines — `begin-sim barrier released` (`game_session/multiplayer.rs:772`),
`transport reconnected; awaiting host snapshot` (`:671`), `adopting host's engine
snapshot` (`:883`) — and a fourth matched a line emitted only when local init
diverged (`frame-0 host snapshot`, `:965`), so the driver was stale rather than
hypothetical. Repairing the two dead waits was not enough to make it coverage: it
also pressed the host through `POST /step-forward`, which advances the host and
re-admits every peer from the new snapshot
(`docs/multiplayer/HARNESS.md:226-234`), so even a passing run would not have
proved input scheduling — and the run it needed is the §4 stack, which exists
now (§10).

The join mode added on 2026-11-19 is the driver §10 records. It waits on lines
the game prints (`browser multiplayer client welcomed`), reads the two live
surfaces (`/state`, `/events` on both seats) and asserts canonical-hash parity
rather than a screenshot; nothing in it presses `POST /step-forward`, so the
comparisons describe the running mission, not a re-admission from a fresh
snapshot.

## 4. What a driver that runs would need

The artifact blocker is gone: `wasm-www/scripts/build-runtime.mjs` builds the
relay client, and §2.1 checks the export. §10 is the run this recipe produced;
the steps below are still the way to stand the stack up, with the three
additions the run needed: an `Xvfb` display and a page warden for the host
(§10.4), and a `--join-content`/`--join-datadir` pair whose digests the driver
re-measures.

Native host, same commit as the wasm artifact, browser links on (default), with
an explicit install home so it cannot collide with another seat
(`docs/PLAY.md:73-84`):

```
cargo build --locked --release --features desktop,multiplayer -p robin_rs --bin robin
ROBINHOOD_INSTALL_HOME=/tmp/robin-host ROBINHOOD_DATA_DIR=~/Games/robin-hood-data \
  target/release/robin --server --mp-expected-players 2 --mp-nickname host --http-server 18641
```

The host prints `Browser join code (expires after 30 minutes if unused): rhmp3-…`
and `Browser join link: <base>/#join=<ticket>` (`session_setup.rs:352-372`). The
link's base is `ROBINHOOD_BROWSER_URL`, default `https://robinhood.phiresky.xyz/`
(`join_ticket.rs:24`, `session_setup.rs:343-345`) — the env var is the only way to
point it at a local origin.

Wasm artifact from the same tree (profile and bindgen pin from
`rust-toolchain.toml`, `scripts/install_pinned_wasm_bindgen.sh`):

```
cargo build --locked -Zbuild-std=std,panic_abort --target wasm32-unknown-unknown \
  --profile wasm-dev -p robin_rs --bin robin --no-default-features --features audio,multiplayer
wasm-bindgen --target web --out-dir wasm-www/pkg --out-name robin \
  target/wasm32-unknown-unknown/wasm-dev/robin.wasm
VITE_IDENTITY_SIGNER_URL=https://identity.robinhood.phiresky.xyz/ pnpm --dir wasm-www build
```

`audio` alone keeps the runtime single-threaded: no shared memory, so no COOP/COEP
requirement (`README.md:149-156`). Add `wasm-threads` only if the threaded path is
under test, and then the page must send `Cross-Origin-Opener-Policy: same-origin`
and `Cross-Origin-Embedder-Policy: require-corp` — the exact values are in
`wasm-www/deploy/public-headers.txt`, and `--isolated` in `browser-game.mjs:54-58`
shows how the fixture server applies them.

Serving, and the three things a fixture server must add over `browser-game.mjs`:

1. `/wasm/<host commit 12 hex>/manifest.json` with
   `multiplayerContent.demo.{url,sha256,byteLength,nativeContentSha256}` where
   `nativeContentSha256` equals the host's ticket content identity
   (`multiplayer_content.ts:381-389`; manifest shape in
   `stage-runtime-addition.mjs:88-95`). The Demo `.rhdata.zst` it points at must
   be served over `https` and be the byte-exact generation whose *native* closure
   the host has (`content_identity.rs:152-176`). Today's published corpus carries
   one native closure for every generation (`wasm-www/scripts/verify-datadir-corpus.mjs:42`)
   and the local data tree is `~/Games/robin-hood-data/DATA/robinhood.bks`
   (1.1 GB); whether that closure equals the published one is **not verified here**
   — `cargo run --example content_identity -- <install>/Data` prints it.
2. The resolver rule must stop being `MAP * 127.0.0.1`. The relay host has to
   resolve normally (and get a real certificate), so it is
   `MAP robinhood.phiresky.xyz 127.0.0.1, MAP identity.robinhood.phiresky.xyz 127.0.0.1,
   EXCLUDE <relay host>`, with the CDP `Fetch` allowlist in
   `browser-game.mjs:158-161` widened to that one relay origin.
3. A signer origin. `pnpm build` pins the production signer origin for the
   leaderboard client (`wasm-www/package.json:12`) but the multiplayer signer URL
   defaults to production too (`multiplayer_identity.ts:7`), so either build with
   `VITE_IDENTITY_SIGNER_URL` pointing at a second local origin (must be a
   different origin, `:55-57`, served from `wasm-www/signer-dist` as
   `browser-game.mjs:84` already does) or let the browser reach the deployed
   signer.

Then a driver, with the artifact served at the ticket's own commit path. There is
none in the tree: the deleted one's `google-chrome --headless=new
--ignore-certificate-errors --host-resolver-rules='MAP robinhood.phiresky.xyz
127.0.0.1, …' --remote-debugging-port=9222 about:blank` launch was sound, but its
waits must name lines the game prints — `browser multiplayer client welcomed`
(debug, `wasm.rs:263-268`), `client session ended: …; reconnecting...`
(`client_session.rs:795`), `multiplayer: begin-sim barrier released at frame …`
(`game_session/multiplayer.rs:772`), `multiplayer: transport reconnected;
awaiting host snapshot` (`:671`), `multiplayer: adopting host's engine snapshot`
(`:883`) — and its assertions must not rest on `POST /step-forward`, which
re-admits every peer from a fresh snapshot.

`--wasm-base` is useless on a join: `resolveBuild` returns the ticket's build
before it looks at that parameter (`main.ts:252-254`), so the artifact must exist
under `/wasm/<commit>/` on the page origin.

**Prerequisites on the build host**, read (not built) today:

- `ssh build-host` → `google-chrome --version` → `Google Chrome 157.0.8103.50`;
  `chromedriver` **absent** (needed only by the `browser-audio` gate's
  wasm-bindgen-test runner, `lifecycle_gate.py:121-130`); `openssl 3.5.9`;
  `node v24.22.2`; `pnpm 12.9.0`. This runtime host has node 24 but no `openssl`
  on `PATH`, so `browser-game.mjs:41` would fail here.
- Licensed data on this host: `ls ~/Games/robin-hood-data/DATA` →
  `Animations Characters Configuration Interface Levels Musics robinhood.bks …`,
  `du -sh` → `1.1G`; a source tree, the shape that lets a host mint a ticket
  (`content_identity.rs:157-176`). A `find /home/you /srv -maxdepth 5` for
  `robinhood-web-content.json` and `*.rhdata.zst` on `build-host` returned nothing, so
  the browser-side Demo closure has to be built
  (`scripts/build_web_shipping_datadir.sh`) and staged.
- WAN: the relay is n0's, chosen by `presets::N0` (`multiplayer/identity.rs:85-95`);
  there is no flag or env that points hosting at a local relay, so a browser
  seat is a WAN test by construction, which is exactly what
  `docs/multiplayer/HARNESS.md:311-317` excludes from the process layer.

## 5. The wasm budgets: which are real, which are holes

`wasm.rs:194-205` is the whole wasm timing table; every value is consumed by
`client_session.rs`, so none is a comment:

| Wait | Wasm value | Consumed at | Native value |
| --- | --- | --- | --- |
| one whole initial attempt | 5 s | `wasm.rs:196` → `client_session.rs:401` | 15 s (`native/client.rs:232`) |
| one whole reconnect attempt | 15 s | `wasm.rs:42`, `:197` → `client_session.rs:617-637` | 15 s (`native/client.rs:233`) |
| each Hello write / Welcome read | **none** | `wasm.rs:195` → `:345-358` | 15 s (`native/client.rs:231`) |
| post-content Welcome | 30 s | `wasm.rs:199` → `:424-429` | 15 s |
| content chunk idle | 30 s | `wasm.rs:201` → `:503-507` | 30 s |
| content transfer | 15 min | `wasm.rs:202` → `:495-514` | 30 min |
| content decision | 30 min | `wasm.rs:203` → `:475-481` | **none** (`native/client.rs:239`) |
| content readiness | 5 min | `wasm.rs:204` → `:520-526` | **none** (`native/client.rs:240`) |
| content write | 30 s | `wasm.rs:200` → `:480`, `:528`, `:541`, `:551`, `:588`, `:598` | 30 s (`native/client.rs:236`) |
| reconnect give-up budget | 60 s | `wasm.rs:198` → `:807-816`, `:852-867` | same 60 s |

The two wasm-only holes this table used to show are closed: the reconnect attempt
was `None` (so a single attempt could outlive the 60 s budget that is read only
between attempts) and `content_write` was `None` (a `SendStream` write during
content admission could wait forever, where native bounds the same call at 30 s).
Both now carry the native numbers, and `reconnect_attempt` is a plain `Duration`
in `ClientTimings`, so an unbounded attempt is not representable. What the table
still says, and no attempt bound fixes:

- `handshake_frame: None` (`wasm.rs:195`) is never itself a deadline; it is only
  safe because every call site sits inside an attempt that has one — the 5 s
  initial attempt or the 15 s reconnect attempt. Nothing enforces that nesting.
- the content decision, transfer and readiness budgets are enforced
  (`wasm.rs:203-204`), and each is longer than any one attempt.

## 6. Wasm-specific ways to hang that are real today

Beyond what the native rows already cover. Each is a code fact; the ones marked
*unproved* need a run to confirm the trigger, not the mechanism.

1. **A reconnect attempt never expires — closed 2026-11-19.** The budget was
   read on the drop (`client_session.rs:807-816`) and after a *completed* attempt
   (`:855-867`), never while one was in flight, so with `reconnect_attempt: None`
   the connect + Hello + Welcome could wait forever with a frozen frame. The
   attempt is now 15 s on both platforms (`wasm.rs:42`, `native/client.rs:233`),
   covers the content prelude it opens (`attempt_reconnect`, `:617-637`), and is
   capped by what is left of the budget (`reconnect_step`, `:655-673`).
2. **A content-admission write never expires — closed 2026-11-19.**
   `content_write: None` (`wasm.rs:200`) left the decision, readiness and
   prepared writes (`client_session.rs:480`, `:528`, `:541`) unbounded, where
   `write_frame_within` treats `None` as "await, unbounded", and native bounds
   the same calls at 30 s. The browser now uses its own 30 s frame bound.
3. **Budgets are JS timers on the thread that runs the game.** The client is a
   `spawn_local` future (`wasm.rs:60`) and every timeout is
   `gloo_timers::future::sleep` (`wasm.rs:169-173`), i.e. a `setTimeout` that can
   only fire when the main thread yields. A long synchronous stretch inside
   admission, snapshot adoption or engine construction freezes the budget with
   the game. Native has the timers on a tokio runtime beside the game loop.
   Consequence, not yet observed: a browser can sit through a deadline it would
   have caught.
4. **bfcache freezes a live session.** `pagehide` only aborts boot work when the
   page is not persisted (`main.ts:367-370`); a persisted page keeps its wasm
   state and loses its relay socket for an unbounded wall-clock time. On restore
   it enters a reconnect whose attempts are bounded (item 1) but whose budget is
   a `setTimeout` on a thread that just resumed (item 3). `browser-game.mjs:366-382`
   exercises restore for a single-player session, so the shape is reachable.
5. **No process to kill.** H19's proof is "gives up and exits non-zero"
   (`docs/multiplayer/HARNESS.md:87`). A browser has no exit: the fatal becomes a
   menu return (`MULTIPLAYER.md:262-266`) whose path on wasm is unproved, so no
   browser row can carry an `exit` assertion — it needs a different observation.

Not wasm-specific: the `BeginSim` and `InitialSnapshot` waits are bounded by the
180 s admission budget ([multiplayer waits](multiplayer-unbounded-waits.md), the
admission rows of "Waits that do have deadlines"), which is read from process
uptime once per frame rather than by a timer, so a browser that never returns to
its game loop cannot observe it either (item 3 above).

## 7. What is missing for a browser row beside H21+

`docs/multiplayer/HARNESS.md:210-218` names three layers, and
`scripts/validation/scenario_corpus.py:65` encodes exactly those
(`LAYERS = ("core", "transport", "process")`). There is no browser layer and no
row today: the corpus stops at `H20`, and H21–H26 are a proposal
([terminal matrix](multiplayer-terminal-matrix.md)).

The smallest seam, in dependency order — each is a separate decision, and the
first is what blocked everything:

1. **An artifact.** Done: `wasm-www/scripts/build-runtime.mjs` builds
   `audio,multiplayer` (`+wasm-threads` threaded), so a dev artifact can carry
   the relay client and §2.1 checks its export. A scenario launcher still does
   not exist; that is harness work.
2. **A ticket the runner can read.** The only place a join code exists is the
   host's console log and the trace event (`session_setup.rs:352-368`);
   `/state` publishes `multiplayer.connect_string` and nothing else
   (`crates/robin_rs/src/http_server/diagnostics.rs:191`). A runner must either
   parse `Browser join code:` from the seat's log — brittle, and the log is a
   user-facing string — or `/state` gains a `browser_join` field. The second is
   the honest seam, with a decision about a bearer invitation on a loopback
   developer surface.
3. **A layer name and a runner.** `proves.layer: "browser"` added to
   `scenario_corpus.py:65`, a `BROWSER_RUNNER` beside `PROCESS_RUNNER`
   (`:121`), and section 1 of `HARNESS.md` gains a layer whose observation is
   browser console lines plus the host's `/state` — never an `exit`, because
   there is no process. `PEER_FIELDS`/`role` (`:125`, `:588-589`) need a way to
   say "seat 1 is a browser", either `platform` on the peer or a role.
4. **A relay story.** Section 5 refuses faults "needing a real WAN" and section 6
   bootstraps with "no relay" (`HARNESS.md:311-317`, `:319-335`). A browser row is
   a WAN row unless hosting learns a custom relay, so it needs a declared
   `requires` entry (today `{"game-data","graphical","netem"}`, `:112`) and an
   explicit exception in section 6 — or the relay requirement is removed from the
   game, which is a much larger change than a row.
5. **A driver that works.** Done for a single join: `browser-game.mjs` join
   mode (§10). A row still needs the runner seam (item 3) and a repeatable
   rejoin (§8) before it can name one.

Given 1–5, a browser row is a `process`-shaped row with a different runner and a
different terminal observation, not a fourth execution site of the same scenario:
the faults a browser can express are a subset (socket close, offline, tab
freeze), and `POST /native`-style divergence is unavailable because the browser
has no HTTP surface.

## 8. What is unproved

The join itself is answered by §10; what remains is play, and the defects the
run exposed on the way.

Proved now, and no longer listed here: the plan builds the relay client, the
`wasm-release` threaded binary links, and its
`wasm_multiplayer_compatibility()` reports the manifest's `ticketSchema` (§2.1);
the artifact boots a multiplayer mission in a browser; iroh's
relay-over-WebSocket path works from a browser against a host built from this
tree; and the browser seat replays the host's canonical state hashes without a
mismatch (§10.3).

- Whether a browser seat can **play**: the Demo mission's script returns "lost"
  on its first tick, with one seat and no browser (§10.5), so no run has put a
  browser-controlled character into a mission that is still in progress. This
  is the first thing to settle, and it is not a browser question — the same
  defeat reproduces in a single native seat.
- Whether player input reaches the guest's simulation. Parity proves the world
  agrees; it does not prove that a key pressed in the page becomes a
  `PlayerCommand` the host applies, because nothing pressed a gameplay key.
- Whether a **fresh page** can rejoin with the same invitation. One run rejoined
  after the page was closed and the seat evicted (browser `begin_sim_frame`
  852, first comparison after the rejoin at 875), and a later attempt booted to
  the main menu and never reached the relay. The rejoin path exists
  (`transport reconnected; awaiting host snapshot`) but is not yet a repeatable
  observation, so the driver does not assert it.
- Whether `~/Games/robin-hood-data`'s closure equals the published Demo
  generation's `nativeContentSha256` (`verify-datadir-corpus.mjs:42`). §10 shows
  the published generation does not match the Demo build converted here, so a
  locally hosted game and the published artifact are built from different Demo
  bytes.
- What the wasm client does with `NetEvent::Fatal` after
  `RECONNECT_BUDGET` — the native path records a diagnostic bundle and returns to
  the menu (`game_session/multiplayer.rs:719-728`); the browser path has no
  process to record into and no evidence was read.
- Whether the JS-timer starvation in §6.3 is reachable: it needs a profile of a
  real join, not a reading.

## 9. What to do next, in order

1. Find why the Demo mission's `CheckVictoryCondition` returns 2 at
   `game_seconds = 0` (§10.5). Until it returns 0, no browser seat can play a
   mission, and every parity run replays a mission that is already lost. The
   single-seat reproduction (`--mp-expected-players 1`, no browser) makes this a
   mission/script question, not a multiplayer one.
2. ~~Stand up the §4 stack and write a driver that waits on the lines the game
   prints.~~ Done: `browser-game.mjs` join mode drives a real host's signed
   invitation and asserts parity (§10).
3. Press a gameplay key in the page and read the host's applied commands, so
   "input reaches the guest" becomes an assertion rather than an assumption.
4. Make the rejoin repeatable — a closed page, a fresh page, the same
   invitation — and decide whether the invitation is single-use, since one
   attempt reached the relay and one did not.
5. Publish the join code on `/state` (`diagnostics.rs:191`) so a runner never
   parses a user-facing log line, then add `proves.layer: "browser"`, the
   runner, and the peer platform field (`scenario_corpus.py:65`, `:121`, `:125`).
6. Only then write the browser row beside H21+, with `requires: ["relay"]`
   declared and section 6's hermetic bootstrap given an explicit exception.
7. Correct `MULTIPLAYER.md:404-411` and `NEW_FEATURES.md:2039-2051` to say what
   the artifact does, not what the code path was written to do — done: they now
   name the artifact proof and the unproved join.

## 10. The join, run and recorded (2026-11-19, `81264ba8929b`)

One recorded run on `build-host`, host and artifact from the same commit. The
evidence is `/home/admin/join-evidence-20261119-230156`: `browser/result.json`
(every assertion named below), the CDP journal, the page's requests, the
screenshot, and the host's own log. It answers §8's join questions and corrects
this file's earlier reading of the modal barrier (§10.4).

### 10.1 What ran

Artifact (threaded, `audio,multiplayer,wasm-threads`), staged as an immutable
addition with its module closure:

```
node wasm-www/scripts/build-runtime.mjs --threads \
  --bindgen /home/admin/build/robin/.tools/wasm-bindgen-0.2.129/bin/wasm-bindgen \
  --out-dir target/static-runtime-addition/wasm/81264ba8929b
node wasm-www/scripts/stage-engine-preload-assets.mjs assets/core-datadir \
  target/static-runtime-addition/wasm/81264ba8929b
node wasm-www/scripts/runtime-javascript-modules.mjs \
  target/static-runtime-addition/wasm/81264ba8929b > target/static-runtime-addition/js-modules.json
node wasm-www/scripts/verify-runtime-multiplayer.mjs \
  --pkg target/static-runtime-addition/wasm/81264ba8929b --expect-ticket-schema 3
```

| file | bytes | sha256 |
| --- | --- | --- |
| `wasm/81264ba8929b/robin_bg.wasm` | 21,497,396 | `e072a3e4ff2e96f81a5cf20989f7096d1a6a0e19155b35c8963e1ed840e4da4a` |
| `wasm/81264ba8929b/robin.js` | 163,873 | `69500448f388a8f27156ea4512ab401b0c82f78dd7a2a9ec4da9246f6a1d1d3e` |
| `js-modules.json` | — | `e71b1e9a2bf305f2fc94711cbdc49101e710a352d023f4b5a108635ce1283eb9` |

`verify-runtime-multiplayer.mjs` prints
`{"engineCommit":"81264ba8929b4178a11fac09c77c087beaf83130","artifactShort":"81264ba8929b","netProtocol":72,"ticketSchema":3}`.

Demo generation: `convert_datadir --format shipping --web-content-manifest
--web-content-edition demo` over the Linux demo's `Data` + `1033/Data`;
`Data/datadir.bin` 11,211,392 bytes, `sha256
62abd8003e69bb83f6453a4998f236e8d27fc49626e517dd5932f4451f9d8dd5`, native
content identity `b69b4e8f…febe` (the ticket's `content_identity_sha256`).

Host, from a directory with no `mods/` (an overlay makes the host refuse
browser invitations — `content_identity.rs:137`). The devShell has no
`xvfb-run`, so the display is a bare `Xvfb :98 -screen 0 1280x900x24`:

```
ROBINHOOD_INSTALL_HOME=/tmp/robin-host-test \
ROBINHOOD_DATA_DIR=~/Games/robin-hood/demo-leicester/robinhood_demo \
ROBINHOOD_MODS_DIR=/tmp/robin-mods-empty \
ROBINHOOD_BROWSER_URL=https://robinhood.phiresky.xyz/ \
  DISPLAY=:98 robin --server --mp-browser-join-links true \
    --mp-expected-players 2 --mp-nickname host --http-server 18641 \
    --mission Dem_Lei_MP --no-sound
```

The invitation is published once at session start
(`session_setup.rs:352-368`); `--mp-browser-join-links true` is what turns it
on for this run instead of the saved profile preference
(`session_setup.rs:410-425`). With browser links on, the host also requires the
public relay to answer within 15 s — one host start in this session failed with
`iroh relay did not become reachable within 15 seconds`, so the run is retried
rather than diagnosed.

Driver: `scripts/validation/browser-game.mjs` in join mode, which serves the
staged shell and artifact from the production hostnames over a loopback HTTPS
server, maps only those two hostnames, and lets the relay resolve normally:

```
node scripts/validation/browser-game.mjs \
  --pkg target/static-runtime-addition \
  --join-ticket "$TICKET" \
  --join-content ~/demo-local-81264ba8929b/Data \
  --join-datadir ~/demo-local-81264ba8929b/Data/datadir.bin \
  --host-display :98 \
  --join-log-timeout 420000 --timeout 1800000 \
  --evidence ~/join-evidence
```

`--host-display` is not optional in join mode: it is the display the page
warden answers the host's shared story page on (§10.4). The 420 s log timeout
is the host's own `MODAL_ACK_BUDGET_MS` (300 s, `modal_session.rs`) plus
margin; a shorter wait reads that budget as a hang.

### 10.2 What the driver asserts

Every assertion reads a live surface, never a log line, and the log waits it
does use name lines the game actually prints (`browser multiplayer client
welcomed`, `wasm.rs:260`):

1. The ticket's `engine_version`, `net_protocol` and `ticket_schema` match the
   staged artifact, and every digest in the manifest it serves is re-measured
   from the bytes it hands out (`buildJoinManifest`).
2. The page is a secure, cross-origin-isolated origin — the threaded artifact
   imports shared memory.
3. The host's ring records a `seat_connected` for a peer after the join began.
4. Both seats report `admission: running`, the same `map`, and the same
   `begin_sim_frame`, on different seats.
5. The browser's own `last_compared_hash_frame` passes 300 with
   `desync_count: 0`, and no `hash comparison missed` line was logged — the
   host is the only seat that publishes canonical hashes
   (`send_state_hash` is server-side), so the guest's comparisons are the
   parity surface.
6. Closing the page makes the host record `seat_evicted` (or a transport loss)
   and keeps the session `running`.

### 10.3 What the run proves

- **The relay path works from a browser.** The page's iroh endpoint reached
  `home is now relay https://euc1-1.relay.n0.iroh.link./`, dialed
  `wss://…/relay`, connected to the host's endpoint, and the host answered with
  `received authoritative multiplayer Welcome your_seat = PlayerId(1)`.
- **The host admits the seat and ships the world.** The host logged `sending
  initial snapshot to peer seat=1 frame=0 bytes=192084`, `ready barrier
  complete frame=0`, `begin-sim barrier released at frame 0`, and the browser's
  `/state` moved `peer_waiting_for_snapshot → peer_waiting_for_begin → running`
  22 s after navigation.
- **Both seats agree on the world, frame by frame.** The browser logged
  `multiplayer hash OK frame = N` for 23 consecutive comparisons (frames 25 to
  575), and the driver's assertion — a compared frame past 300 with
  `desync_count: 0` — held at frame 350, the thirteenth comparison. A
  mismatch would have logged `multiplayer DESYNC` and incremented the counter
  (`pre_tick.rs:133-160`); neither happened.
- **The page disappearing does not take the host with it.** Closing the target
  made the host record `seat evicted: player` (`seat=1`, frame 990) and its
  `/state` still answered `admission: running`, `desync_count: 0`.
- **The artifact contract is enforced end to end.** The shell's manifest check
  (`wasm-www/src/multiplayer_content.ts`) accepted the served manifest only
  because `commit`, `short`, `netProtocol`, `ticketSchema` and the Demo's
  `nativeContentSha256` all matched the host-signed ticket.

One measurement is a trap worth naming: `/state.frame` is the Original's
hourglass tick (`engine/host_accessors.rs:881`), not simulation progress, and it
stays 0 for a mission this short on both seats. The driver therefore never uses
`frame` as a progress signal; `last_compared_hash_frame` is what moves.

### 10.4 The "modal blocker" was an unanswered host page

The earlier reading of the failed runs was that `modal_session.rs` strands a
late-joining browser seat. The code does not replay a page to a seat that joins
after its announcement (`announce_modal_instance` returns early when the
instance is already announced, `crates/robin_engine/src/multiplayer/modal_session.rs:853-865`),
but the run that failed was not a late joiner's problem: **nobody answered the
host's own page at all.** The host has no player at its window, so the
`PopupText { text_id: 0 }` page announced at frame 0
(`modal_announced`, `role: "host"`) held the begin-sim barrier until the
300 s budget expired (`MODAL_ACK_BUDGET_MS`, `modal_session.rs:26`), and the
browser sat at `peer_waiting_for_begin` for the whole of that wait.

The driver now runs a page warden: it reads the host's event ring from its first
event and, for every `modal_announced` with `role: "host"`, presses Return on
the host's display with the same helper the native harness uses
(`scripts/validation/briefing_x11.py`). The browser seat answers nothing: a
peer's copy of a shared page is dismissed by the host's decision
(`modal_acked`, "modal decided (host authority)"), and the browser's own ring
carries no `modal_announced` — the recorded run answered exactly one page, on
the host (`hostPageAnswered: {"page":"PopupText { text_id: 0 }","seq":3}`).

So the modal is a harness gap, not a game defect. A real host has a person at
the window; an unattended host needs a synthetic one.

### 10.5 What it does not prove: the mission is lost on its first tick

The session is healthy; the mission inside it is not. The host's own log, in
this run and in every earlier one, has

```
Engine tick returned: LevelFailed
```

one tick after `admission … → running`, and both seats' `/state` report
`presentation.kind.FinalDebriefing { text_id: { Lose: { index: 0 } } }` while
the simulation keeps ticking behind it (the hash comparisons above are of that
lost mission).

This is not a browser or multiplayer defect. With `--mp-expected-players 1`,
no browser, and the same warden answering the same page
(`/home/admin/solo-victory-20261119-221832`), the engine logs

```
DEBUG victory: CheckVictoryCondition game_seconds=0 result=Ok(2) mission_won=false
  blocked_by_combat=false pc_positions=[(Pc(PcId(198)), (209.0, 1881.0)),
  (Pc(PcId(81)), (184.0, 1039.0))]
```

The Demo mission's script says "lost" at game second 0, with a single native
seat and two live PCs. Nothing in the join path is involved. Until that returns
0, "a browser seat can play" is unproved, and this file says so rather than
upgrading "instantiated, admitted, in lockstep" into "played".

### 10.6 What a public deployment still needs

A link the owner can send, and nothing else:

1. **Two HTTPS origins, both already named in the code.** The shell and the
   `/wasm/<short>/` artifact on the game origin; the identity signer and its
   bridge on the signer origin (`wasm-www/deploy/`). The game origin must send
   `Cross-Origin-Opener-Policy: same-origin` and
   `Cross-Origin-Embedder-Policy: require-corp` (the threaded artifact imports
   shared memory), and the artifact and Demo files need
   `Cross-Origin-Resource-Policy: cross-origin` for the signer's iframe to
   embed. Both header sets are already files in `wasm-www/deploy/`.
2. **The published Demo generation must be the one hosts run.** The ticket
   carries `content_identity_sha256` and the manifest carries the generation's
   `nativeContentSha256`; the shell refuses a mismatch. So the site's datadir
   has to be built by `convert_datadir --web-content-manifest` from the same
   Demo build the host has installed — publishing bytes from a different build
   makes every invitation invalid (the published generation does not match the
   Demo build converted here).
3. **The relay URL comes from the ticket, not from the site.** The host signs
   `relay_url` (the public n0 relay today) and the page dials it; no
   deployment-side relay configuration and no self-hosted relay are needed. A
   custom relay is a host-side change, and browsers must reach its
   `wss://<relay>/relay`.
4. **No ticket service and no accounts.** The host signs the invitation with
   its install identity (`<install home>/multiplayer_identity.key`) and the page
   verifies it against the ticket's `host_public_key`; the static site holds no
   secret and needs no backend.
5. **What the guest does:** open the share URL, keep the tab in front (the
   runtime uses wasm threads and `Atomics.wait`), accept a ~32 MB download, and
   wait for the host's world. Nothing is installed and no port is forwarded.
6. **What the host needs:** the public relay reachable within 15 s of session
   start, browser join links enabled, an unmodified content closure, and
   somebody — or something — at the window to answer the mission's first page.
