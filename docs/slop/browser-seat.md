# A browser seat: can it join today, and what proves it

Written 2026-11-19 against `fork/main` `ac74b98c0a90`, re-read twice the same
day: section 5 and items 1-2 of section 6 after the reconnect-attempt and
content-write holes were closed, then the artifact section after the build plan
was fixed. The production changes this file caused are the build plan, two wasm
compile fixes, the compatibility export's single source and the budget bounds;
the devShell's wasm C compiler belongs to
[the wasm build evidence](wasm-build-host.md). The owner's question is
whether a browser (wasm) seat can join a multiplayer session today, and what
exactly proves it. Adjacent material this file links rather than restates: [the
harness contract](../multiplayer/HARNESS.md) (what a row may name),
[multiplayer waits without a deadline](multiplayer-unbounded-waits.md) (the wasm
rows), [the terminal matrix](multiplayer-terminal-matrix.md) (H21–H26, the rows
a browser row would sit beside), [the multiplayer architecture](../MULTIPLAYER.md).

## Verdict

| Direction | State (2026-11-19, after the build-plan fix) | What decides it |
| --- | --- | --- |
| native host + browser client, the repo's own artifact | **artifact ready, join unproved** | `wasm-www/scripts/build-runtime.mjs` now builds `audio,multiplayer` (threaded: `+wasm-threads`), so the artifact exports the real `ticketSchema` and the shell no longer refuses the invitation. No browser seat has joined a native host: no run is recorded, and the one driver was deleted on 2026-11-19 because it could not pass (§3) |
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
| `scripts/validation/browser-game.mjs` | `google-chrome --headless=new` against a Node HTTPS server on `127.0.0.1:0`, self-signed cert with SANs for both game names (`:40-42`, `:63-97`, `:127-130`) | Only `robinhood.phiresky.xyz` and `identity.robinhood.phiresky.xyz`, force-resolved to loopback (`:28-29`, `:96`, `:130`); every other host is refused with 403 (`:67`) and every non-GET, non-https or off-allowlist request is failed in CDP (`:155-163`) | single-player Demo boot, first mission state, replay export through the real wasm admission worker, pause-menu replay restart, bfcache restore, no console "required mission text" errors, no desync lines (`:252-398`) | no multiplayer anywhere: it navigates to `/?<query>` with no ticket and no fragment (`:265-268`) |

The `browser-game.mjs` isolation is deliberate and total: `MAP * 127.0.0.1:${port}`
sends *every* hostname to the fixture server, and the handler serves only two
names. A relay is a third name and would be answered `local acceptance: denied`
(`:67`). In other words the loopback harness is loopback by construction, and it
cannot host a browser multiplayer run without changing both the resolver rule and
the allowlist.

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
proved input scheduling — and the run it needed is the §4 stack, which has never
existed here.

Nothing in the tree records a browser join having happened: no evidence
directory, no gate invoked the driver (`grep -rn live-relay-e2e
scripts docs wasm-www/package.json` → this file and `MULTIPLAYER.md`, nothing
else), and
`scripts/README.md` lists `browser-game.mjs` but not a relay driver. What the
tree does now prove is narrower and exact: the artifact the repo builds exports
the manifest's `ticketSchema` (§2.1), so the documented capability no longer
dies at the shell's first check. No browser seat has joined a host.

## 4. What a driver that runs would need

The artifact blocker is gone: `wasm-www/scripts/build-runtime.mjs` builds the
relay client, and §2.1 checks the export. Nothing below has been run against a
native host, so this section is still a recipe, not a runbook.

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
5. **A driver that works.** None exists: the stale one was deleted on
   2026-11-19 (§3), so a row needs a driver written to the lines the game
   actually prints before it can name one.

Given 1–5, a browser row is a `process`-shaped row with a different runner and a
different terminal observation, not a fourth execution site of the same scenario:
the faults a browser can express are a subset (socket close, offline, tab
freeze), and `POST /native`-style divergence is unavailable because the browser
has no HTTP surface.

## 8. What is unproved

Proved now, and no longer listed here: the plan builds the relay client, the
`wasm-release` threaded binary links, and its
`wasm_multiplayer_compatibility()` reports the manifest's `ticketSchema` (§2.1).

- That the artifact **boots a multiplayer mission in a browser**. Instantiation
  and the compatibility export are proved; the boot path after
  `wasm_set_multiplayer_join_ticket` is not.
- Whether iroh's relay-over-WebSocket path works from a browser against a host
  built from this tree at all — no recorded run, and the one driver is stale.
- Whether `~/Games/robin-hood-data`'s closure equals the published Demo
  generation's `nativeContentSha256` (`verify-datadir-corpus.mjs:42`). If it does
  not, a locally hosted game cannot be joined by the published demo artifact and
  both sides must be built together.
- What the wasm client does with `NetEvent::Fatal` after
  `RECONNECT_BUDGET` — the native path records a diagnostic bundle and returns to
  the menu (`game_session/multiplayer.rs:719-728`); the browser path has no
  process to record into and no evidence was read.
- Whether the JS-timer starvation in §6.3 is reachable: it needs a profile of a
  real join, not a reading.

## 9. What to do next, in order

1. Stand up the §4 stack and write a driver that waits on the lines the game
   prints (`wasm.rs:263`, `client_session.rs:795`) — the stale driver is gone, so
   this is a new file, not a repair. Running it once against a staged artifact
   decides whether the browser path is real or aspirational, and produces the
   first evidence this file cannot.
2. ~~Bound the wasm reconnect attempt and the content write.~~ Done
   2026-11-19: the attempt is 15 s like native (`wasm.rs:42`, `:197`) and now
   covers the content prelude it opens, and the write is 30 s (`wasm.rs:200`).
   The remaining wasm hangs are §6.3 (timer starvation) and §6.4 (bfcache),
   neither of which an attempt bound reaches.
3. Publish the join code on `/state` (`diagnostics.rs:191`) so a runner never
   parses a user-facing log line, then add `proves.layer: "browser"`, the
   runner, and the peer platform field (`scenario_corpus.py:65`, `:121`, `:125`).
4. Only then write the browser row beside H21+, with `requires: ["relay"]`
   declared and section 6's hermetic bootstrap given an explicit exception.
5. Correct `MULTIPLAYER.md:404-411` and `NEW_FEATURES.md:2039-2051` to say what
   the artifact does, not what the code path was written to do — done: they now
   name the artifact proof and the unproved join.
