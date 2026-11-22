# Playable, comfortable browser (WASM) client on Android — design

Status: **draft — awaiting owner approval. No code may be written until approved.**
Date: 2026-11-22
Stream owner: browser + Android WASM. This stream does **not** touch the v0.4.0
release, its gate, tag or artifacts.

All claims below were verified read-only against `fork/main` `7d4d08b00` (the
integration ref), plus the estate's `nixos-config` `origin/main` for hosting.
No builds were run.

## 1. Intent and why

The owner's words: *"play browser version on android comfortably. also the
browser version should be able to connect to windows/linux native builds or to
play separately single player games, but it should be possible to play full
campaign from the browser."*

So one client must serve three outcomes:

1. Open on a phone in Chrome, feel like a real game (touch, readable HUD,
   home-screen icon) — no APK.
2. Join a friend's native Linux/Windows host over the existing relay.
3. Play the full single-player campaign from the browser, start to finish,
   with progress surviving a reload.

The owner dislikes APKs; approach A (ship the existing WASM build as the Android
client, plus touch/responsive HUD and PWA install) was agreed before this doc.
No alternative is reopened here.

## 2. Verified current state

### 2.1 Touch input today

- The engine has a real gesture classifier: `crates/robin_rs/src/touch_input.rs`
  emits `TouchOutput::{PointerCancel, MotionStop, TouchTransformStart/
  TouchTransform/Motion Stop}`; `crates/robin_rs/src/window.rs:958-988` maps them
  to `GameEvent` variants declared in `crates/robin_rs/src/gfx_types.rs:338-363`
  (`PointerCancel`, `TouchMotionStop`, `TouchTransformStart`,
  `TouchTransform`, `TouchTransformEnd`). `crates/robin_rs/src/frontend_input.rs`
  routes touch-plan events, and `crates/robin_engine/src/engine/input_state.rs:279`
  handles "touch takeover must not dispatch the abandoned pointer's release".
- `docs/NEW_FEATURES.md:2137-2146` records taps, drags, double-taps, two-finger
  transforms, anchored pinch zoom, bounded pan/inertia, UI/minimap exclusion
  and a separate native-refresh presentation toggle.
- **In the web shell itself** there is almost nothing touch-specific:
  `wasm-www/src/cursor-mode.ts:35` (one `pointerdown` for cursor capture) and
  `wasm-www/src/timeline.ts:53` (one-finger replay scrub). `wasm-www/index.html:5`
  has `width=device-width, initial-scale=1`; the canvas is fixed
  `1024x768` at line 161. There is no safe-area/cutout CSS, no phone-sized HUD,
  no on-screen action buttons, and no page-level gesture arbitration.
- `docs/NEW_FEATURES.md:1554` says Android uses a cutout-safe immersive content
  area — that is the **native Android build**, not the browser page.
- **Unproved:** that the wasm/winit web backend actually delivers the multi-touch
  pointer stream that feeds `touch_input.rs`. The classifier exists in the
  compiled game; the end-to-end phone→gesture path has no evidence in the repo.

### 2.2 Browser ↔ native cross-play

- `crates/robin_rs/src/multiplayer/wasm.rs:1-12` states the design: browsers
  cannot use iroh's UDP discovery, so the endpoint dials one host-signed HTTPS
  relay route and carries the **unchanged** game ALPN, stream, framing,
  admission, rollback and snapshot protocol; the session state machine is
  shared with native in `client_session`. Browser hosting and DHT discovery are
  deliberately unsupported.
- `crates/robin_rs/src/multiplayer/join_ticket.rs:20` defines
  `IROH_RELAY_TRANSPORT = "iroh-relay-websocket"`; validation at lines 342-358
  rejects a mismatched transport, ticket schema or network protocol.
  `wasm.rs:110-125` requires the relay to be online within 15 s, otherwise the
  browser refuses to join.
- `docs/MULTIPLAYER.md:28-31` confirms native uses QUIC + relay fallback while
  browsers use relay-over-WebSocket with only the relay from the signed
  invitation; `docs/adr/0003-browser-is-a-guest.md` fixes the browser as guest.
- `docs/slop/browser-seat.md:751-853` records a **real public join** against
  `https://robin.jautis.lt/`: ticket schema 3, protocol 72, browser welcomed as
  seat 1, begin-sim barrier released, hash OK at frame 125, cross-origin
  isolated, no console exceptions.
- **Limits:** `docs/slop/browser-seat.md:476-489` says playable mission play,
  player input reaching the guest's simulation, and a repeatable fresh-page
  rejoin are still unproved; the Demo mission reports defeat on its first tick.
  The transport is proved; the *game* is not.

### 2.3 Full campaign from the browser

- The campaign engine is compiled into the wasm client:
  `crates/robin_rs/src/main_entry/callbacks.rs` handles campaign saves and
  continuation (`request_save`, campaign snapshot validation, restart checkpoints);
  `crates/robin_rs/src/campaign_map.rs` is in the same crate.
- Browser saves already persist: `crates/robin_rs/src/player_profile_store/browser.rs`
  stores profiles/key configs/saves in a versioned localStorage envelope under
  `robin-hood-player-profiles-v1`; `crates/robin_rs/src/player_profile_store.rs:494-506`
  sets `BROWSER_SAVE_DIRECTORY = "browser-save"` and a **4 MiB byte limit**
  (`BROWSER_PROFILE_BYTE_LIMIT`), wired in by
  `crates/robin_rs/src/main_entry/platform/wasm.rs:26-36`.
- But the shipped shell is Demo-only: `wasm-www/src/boot-lifecycle.ts:43-46`
  documents `loadDefaultContent` as "the Demo datadir generation pinned by the
  selected build"; `docs/adr/0003-browser-is-a-guest.md` says the first web page
  offers "play the Demo mission alone, or join a friend's game"; `README.md:263`
  says "Full-edition runs are download-only for now". Full content is otherwise
  selectable only via a local folder picker for multiplayer
  (`wasm-www/src/content-picker.ts`), which is not a phone-friendly path.
- Leaderboard UI already defines campaign achievements, including "Complete the
  full campaign" (`wasm-www/src/leaderboards/achievements.ts:48`) and campaign
  score fields — the destination is defined, the browser play path is not.
- **Correction to the handoff:** the premise "the browser already has campaign
  continuation signing" is not supported by the code. `sign_campaign_continuation`
  appears only in `wasm-www/src/identity-signer/protocol.test.ts:170-172` in a
  list of operations that must be **rejected**, and as an unsupported operation
  in `crates/robin_rs/js/browser_identity_modules.test.mjs:57`. There is no
  supported campaign-continuation signing operation in the browser identity
  protocol. Campaign continuation must therefore ride on the engine's own save
  and campaign snapshot (`main_entry/callbacks.rs`), never on that signer. The
  one-shot continuation in `docs/NEW_FEATURES.md:2050-2060` is for native-hosted
  replacement missions, not browser single player.

### 2.4 Deployment to `robin.jautis.lt`

- The estate origin is served by Caddy on `oci-vm`:
  `nixos-config origin/main hosts/oci-vm/caddy.nix:191-220` sets
  `root * /var/www/robin` for `robin.jautis.lt` and
  `root * /var/www/robin-id` for the separate signer origin
  `robin-id.jautis.lt`; `hosts/oci-vm/cloudflare-dns.nix:35-36` proxies both.
  `modules/system/robin-hood.nix:11-14` makes `https://robin.jautis.lt/` the
  default invitation origin for estate hosts.
- The exact self-hosted staging/deploy recipe, artifact hashes and the CSP
  pitfalls (baked meta CSP, origin-bound signer bridge, COOP/COEP/CORP) are in
  `docs/slop/browser-seat.md:751-853`. The two vhosts are a **separate repo**
  (`~/nixos-config`); this stream may propose changes there but must not mix
  them into the v0.4.0 release work.
- **PWA blocker found:** both the page's meta CSP (`wasm-www/index.html:8`) and
  the Caddy header (`hosts/oci-vm/caddy.nix:197`) omit `manifest-src`, which
  therefore falls back to `default-src 'none'` — a `<link rel="manifest">`
  would be blocked. `worker-src 'self' blob:` is already present, so a service
  worker is permitted; COOP/COEP/CORP must be preserved on every cached response.

## 3. Goals, non-goals, anti-goals

**Goals**

- G1 — A phone in Chrome can start the game from `robin.jautis.lt`, install it
  to the home screen, and control it with touch without a keyboard or mouse.
- G2 — HUD, menus and dialogue are legible and reachable on a phone viewport,
  including notches/cutouts and both orientations.
- G3 — A browser guest can join a live native Linux/Windows host over the
  existing signed join ticket and relay, and that run can be shown to actually
  play (commands reach the simulation).
- G4 — A browser player can start and finish the full single-player campaign,
  and continue it after a reload, from the same origin.
- G5 — The existing one-WASM-codebase architecture and the guest-only protocol
  stay intact; no second game client is forked.

**Non-goals**

- No APK, WebView wrapper, store listing, or native Android UI work.
- No browser-side hosting, matchmaking or authority (ADR 0003).
- No new multiplayer transport, protocol version, or signer operation.
- No offline *full campaign*: full content is large and licensed; first load
  and cross-play stay online.
- No changes to the v0.4.0 release, gate, tag or artifacts in this stream.

**Anti-goals (what would make this a failure even if it "works")**

- A page that boots but cannot be driven past the first menu by touch.
- A "cross-play" claim backed only by hash parity with a mission that already
  lost, as in `browser-seat.md:476-480`.
- A campaign that is "available" only if the player supplies local licensed
  files through the desktop folder picker.
- A PWA that installs but whose manifest/service worker is blocked by CSP, or
  whose cache strips COOP/COEP.

## 4. Constraints

- **No APK** (owner preference); one WASM codebase, one published artifact per
  commit (`docs/slop/wasm-build-host.md`, `docs/TESTING.md:204-232`).
- The relay transport, tickets and guest/browser identity already exist and are
  the only cross-play path; the browser must remain guest-only.
- **No local builds in this stream** — each Rust build is ~1 h on the build
  host. The only compile gate is `bash scripts/check-quality.sh wasm` run there
  (or via `scripts/remote-check.sh wasm`), never from the runtime.
- The device is not always on; all phone work must be staged so the owner's
  turn is one action (open URL / scan / tap), and device proof must be captured
  as evidence, not assumed.
- Browser profile storage has a hard 4 MiB envelope; campaign history retains
  every attempt (`docs/NEW_FEATURES.md:2158-2164`), so save growth must be
  measured, not guessed.
- Full web content is licensed data and cannot live in the repo (`README.md:266-268`);
  a new corpus generation is a publication act on the estate origin.
- The signer is a separate origin with tight CSP; widening it is a
  `nixos-config` change with its own gate.

## 5. Approaches considered

**A — Existing WASM build + touch/responsive shell + PWA (chosen).**
Reuse the compiled game, add a phone-facing input/HUD layer in `wasm-www/`,
publish the same artifact to `robin.jautis.lt`, make it installable. Trades off
zero new native code and one codebase against needing real browser/device work
for input arbitration, safe areas, CSP and content hosting. This is the agreed
approach and the only one that satisfies "no APK" plus cross-play for free.

**B — Native WebView APK wrapper (rejected).** A thin Android app hosting the
same URL. It adds an APK, a build/signing/release pipeline and store-style
distribution the owner explicitly dislikes, while delivering no capability the
browser page cannot deliver; it would also fork device proof across two
clients. Nothing in the relay path needs it.

**C — Standalone web-only single-player (rejected).** A separate browser-only
game with its own campaign and save model. It would satisfy G4 alone but break
G3 (no shared relay/ticket/identity) and G5 (second codebase), and it would
duplicate the engine's campaign logic that already compiles to wasm.

## 6. The decided how

### 6.1 Touch controls on the WASM canvas

Build on the engine's existing `touch_input.rs` → `GameEvent` path rather than
inventing a second gesture model. Work is in the shell and the small bridge
where the shell touches input:

- Set `touch-action: none` on the canvas and use pointer events so Android
  Chrome does not steal drags for scroll/zoom; keep the existing cursor-capture
  (`cursor-mode.ts`) and replay-scrub (`timeline.ts`) paths intact.
- Feed taps/double-taps/long-press into the game's existing `GameEvent` actions
  for selection, move/attack and quick actions, and keep pinch/pan with the
  engine's `TouchTransform*` events. HUD buttons emit the same typed commands
  as their keyboard/mouse equivalents — no gameplay rule may be phone-only.
- Add an on-screen control cluster (movement/selection/quick actions/pause)
  rendered as DOM over the canvas or drawn through the existing HUD widgets;
  it must be reachable with a thumb and must not overlap the minimap.
- Verify on a device that a tap becomes a `PlayerCommand` in the simulation —
  the gap `browser-seat.md:481-483` explicitly leaves open.

### 6.2 Responsive HUD scaled for phone

- Replace the fixed `1024x768` canvas CSS (`wasm-www/index.html:161`) with a
  viewport-sized box that preserves the engine's logical aspect and lets the
  existing widescreen/letterbox logic (`docs/NEW_FEATURES.md:1547-1556`) do the
  scaling; keep the physical canvas independent of the logical resolution.
- Use `env(safe-area-inset-*)` so notches/cutouts do not cover controls,
  mirror the native Android cutout-safe area rather than re-implementing it.
- Keep `width=device-width, initial-scale=1` and add portrait/landscape rules;
  a narrow portrait phone must still expose pause, map and action buttons.
- No HUD text below a legible minimum; test at real device pixel densities.

### 6.3 PWA: manifest, add-to-home-screen, offline cache

- Add `wasm-www/manifest.webmanifest` (name, icons, `display: standalone`,
  `start_url` at the origin) and link it from `wasm-www/index.html`.
- Widen **both** CSPs to allow it: the baked meta CSP in `wasm-www/index.html:8`
  and the Caddy header in `nixos-config hosts/oci-vm/caddy.nix:197` need
  `manifest-src 'self'`. The service worker needs the already-present
  `worker-src 'self' blob:`; COOP/COEP/CORP must be kept on cached responses or
  the threaded runtime loses cross-origin isolation.
- A service worker caches the shell, manifest, icons and the most recently used
  runtime/datadir generation with versioned keys and a cache-busting check of
  `/wasm/latest.json`. It must fail closed: a stale or partial cache falls back
  to the network instead of booting a mismatched artifact (the deployment rule
  at `README.md:305-307`). Full campaign content stays online-only.
- Installability is the goal; "works fully offline" is not (see §3).

### 6.4 Cross-play via the existing relay

- No protocol work. The host publishes a 30-minute `rhmp3` invitation whose
  `browser_url` points at `https://robin.jautis.lt/` (estate default in
  `modules/system/robin-hood.nix`); the phone opens the fragment URL and the
  shell redeems it through `wasm_set_multiplayer_join_ticket` as today
  (`boot-lifecycle.ts:130-145`, `wasm.rs:39-62`).
- Keep the guest-only rule and the relay-privacy notice in the invitation UI.
- **Acceptance is a played run**, not a hash-parity screenshot: a native host on
  Linux or Windows, a real Android device as guest, and evidence that a
  player command changed the shared simulation (the unproved case in
  `browser-seat.md:481-483`).

### 6.5 Full campaign from the browser

- Use the campaign code already in the wasm binary and the existing browser save
  store; add a shell entry point (New campaign / Continue) that boots the
  campaign mission rather than only the Demo mission. Continuation comes from
  the engine's campaign snapshot and `request_save` path
  (`main_entry/callbacks.rs:553-560,660-699`), not from the removed signer
  operation (§2.3).
- Publish a **full web-content generation** on the estate origin, built by the
  canonical shipping converter (`README.md:266-283`), with its own manifest and
  digests, alongside the Demo generation. The shell picks full vs demo by
  explicit user choice, never by silent substitution; mismatched content is a
  launch error, as in the multiplayer path.
- Measure campaign save size against the 4 MiB browser envelope. If it exceeds,
  the builder moves the campaign archive to IndexedDB (already used by
  diagnostics and the identity signer) rather than trimming history. Save
  corruption is reported, never silently reset.

### 6.6 Where the build and deploy boundary sits

- Game/wasm changes land in this repo through this stream's worktree; the
  Caddy/CSP change is a separate `nixos-config` commit on the estate origin.
  Neither touches v0.4.0.
- Only the build host runs `bash scripts/check-quality.sh wasm`; no gate, tag or
  release is started from here.

## 7. What's left to the builder

**Specific files to change (starting points, not an exhaustive list)**

- `wasm-www/index.html` — responsive canvas/CSS, safe areas, `touch-action`,
  manifest link, meta CSP `manifest-src`.
- `wasm-www/src/main.ts`, `canvas-lifecycle.ts`, `cursor-mode.ts` — pointer
  arbitration, canvas sizing, on-screen control wiring.
- New `wasm-www/src/touch-controls.*` and `wasm-www/manifest.webmanifest`,
  `wasm-www/sw.js` (+ tests alongside, matching the existing `*.test.ts` style).
- `wasm-www/src/boot-lifecycle.ts`, `content-picker.ts`, `mission-launch.ts` —
  full/demo content selection and campaign boot.
- `crates/robin_rs/src/main_entry/callbacks.rs` / `player_profile_store/browser.rs`
  only if save measurement forces an IndexedDB store; otherwise no engine change.
- `nixos-config hosts/oci-vm/caddy.nix` + `wasm-www/deploy/public-headers.txt` —
  `manifest-src`, service-worker MIME/cache headers, preserving COOP/COEP/CORP.

**Test strategy**

- Static: `bash scripts/check-quality.sh wasm` on the build host (never the
  runtime), plus the repo's JS/TS checks (`pnpm test` in `wasm-www/`) and
  `pnpm test:deployment` for header/topology assertions.
- Extend browser synthetic tests to cover the new pointer/gesture mapping,
  responsive layout, manifest/CSP and service-worker cache invalidation.
- Extend `docs/slop/browser-seat.md` style evidence with a played cross-play run.

**Device proof (required before claiming done)**

- Real Android phone, Chrome: tap/drag/pinch produce observable gameplay
  commands; screenshot or screen recording of a menu → mission → campaign
  progression.
- Add-to-home-screen works and launches standalone; orientation and cutout
  screenshots; save/continue across a reload.
- Full campaign begun and a subsequent mission continued from the browser on
  the phone.
- A native Linux **or** Windows host plus the phone guest, with logs/screenshots
  showing a browser-issued command changing the shared world.
- Record the artifact hash and origin used in every piece of evidence.

## 8. Calls made in this design (designer's calls, not owner decisions)

- Offline scope = installable + cached shell/recent assets; full offline
  campaign is not a goal.
- Reuse the engine gesture classifier; no second gesture model.
- Keep the browser guest-only and the existing relay/ticket protocol untouched.
- Start with localStorage saves and add an explicit oversize error; migrate to
  IndexedDB only if measurement shows the 4 MiB limit is exceeded.
- Full content is published as a new, digest-pinned corpus generation on the
  estate origin; no local-folder path for phone players.
- The `nixos-config` CSP change is a separate commit from the game repo work.

## 9. Open questions for the owner (from the builder check)

_To be filled after the builder-check subagent returns; see the handoff report._
