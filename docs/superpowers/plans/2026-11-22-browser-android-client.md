# Browser/WASM Android Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the existing WASM build a comfortable, installable Android browser client with touch controls, a private password-gated full campaign for two paid-copy owners, and proved cross-play with a native host — no APK.

**Architecture:** The shell in `wasm-www/` is the only place that changes for input, layout and installability; the engine's `touch_input.rs → GameEvent` path and its `rh_rpc("command")` entry point are reused, not forked. Full content is a digest-pinned datadir generation served from a private capability path gated by Caddy `basic_auth`; the service worker caches only the shell and the Demo generation, never full content.

**Tech Stack:** TypeScript, Vite, Node built-in test runner (`node:test`), jsdom, Caddy headers, Cloudflare `_headers` staging.

**Spec:** `docs/superpowers/specs/2026-11-22-browser-android-design.md`

## Global Constraints

- No APK, WebView wrapper or second game client. One WASM codebase.
- No full Rust builds, no `scripts/land.sh`, no `push-gate.sh`, no `remote-check.sh` suites. The only compile gate is `bash scripts/check-quality.sh wasm` on the build host, run by a human.
- No v0.4.0 release work.
- Commit by pathspec (`git commit -- <files>`); never `git stash`, `git add -A`, a bare `git commit`, or force-push.
- Work in `.worktrees/browser-android-design` on `agent/browser-android-design`.
- Tests: from `wasm-www/`, `node --experimental-strip-types --test src/<file>.test.ts` (Node 24, `corepack pnpm install` if `node_modules` is missing). No Rust build is needed for any task here.
- Full corpus and the password never enter this repo. Password hash is a `nixos-config` secret.
- Preserve COOP/COEP/CORP on every response; a cached response that loses cross-origin isolation breaks the threaded runtime.

## Review Focus

1. **Touch pointer capture stealing multi-touch** — a second finger must still reach winit; expected: `captureDrag` ignores `pointerType === "touch"`. Test: Task 1 cursor-mode test.
2. **Notch/cutout covering controls** — expected: safe-area insets applied to the canvas box and control cluster. Test: Task 1 responsive-layout test.
3. **Stale/partial cache booting a different artifact generation** — expected: service worker falls back to network, never serves a mismatched generation. Test: Task 4 `sw.js` pure-function tests.
4. **Full content leaking to the public web root** — expected: no cache, no index, no public path; only the capability path with Basic Auth. Test: Task 4 `isCacheable` rejects `/datadirs/full/`; Task 3 header assertions; Task 6 Caddy config review.
5. **HUD button that does not reach the simulation** — expected: each button emits the same `PlayerCommand` JSON as its keyboard/mouse equivalent through `rh_rpc("command", …)`. Test: Task 2 rpc-payload assertions.

---

### Task 1: Touch-safe canvas and viewport layout

**Files:**
- Modify: `wasm-www/index.html`
- Modify: `wasm-www/src/cursor-mode.ts`
- Modify: `wasm-www/src/canvas-lifecycle.ts`
- Test: `wasm-www/src/responsive-layout.test.ts` (new)
- Modify test: `wasm-www/src/cursor-mode.test.ts`

**Interfaces:**
- Consumes: existing `installCanvasBackingStore(canvas, environment?)` from `canvas-lifecycle.ts`.
- Produces: no new exports. CSS contract only: `#canvas { touch-action: none }`, `viewport-fit=cover`, `env(safe-area-inset-*)` used in layout.

- [ ] **Step 1: Write the failing responsive-layout test**

`wasm-www/src/responsive-layout.test.ts`: read `index.html` as text and assert (a) the viewport meta contains `viewport-fit=cover`, (b) a `#canvas` rule contains `touch-action: none`, (c) at least one `env(safe-area-inset-top)` and one `env(safe-area-inset-bottom)` reference, (d) CSS uses `100dvh`, (e) a `.touch-controls` rule exists. Follow the existing `src/csp-test-utils.ts` pattern of parsing HTML with `JSDOM.fragment`.

- [ ] **Step 2: Run the test to verify it fails**

Run from `wasm-www/`: `node --experimental-strip-types --test src/responsive-layout.test.ts`
Expected: FAIL on the first assertion.

- [ ] **Step 3: Make the layout touch-safe**

In `index.html`: add `viewport-fit=cover` to the viewport meta; add `touch-action: none;` and `touch-callout: none;` to `#canvas`; use `env(safe-area-inset-*)` in the canvas and toolbar spacing; add a `.touch-controls` CSS block (hidden by default) that will host Task 2's buttons. In `canvas-lifecycle.ts`: subtract the four safe-area insets from `availableWidth`/`availableHeight` in the non-fullscreen branch, reading them from the environment's viewport via `getComputedStyle(canvas.ownerDocument.documentElement)` with a numeric fallback of 0.

- [ ] **Step 4: Stop touch pointer capture from stealing multi-touch**

In `cursor-mode.ts` `captureDrag`, return early when `event.pointerType === "touch"`. Add a case to `cursor-mode.test.ts`: dispatch a `pointerdown` with `pointerType: "touch"`, `button: 0` and assert `setPointerCapture` was not called; assert a mouse `pointerdown` still calls it.

- [ ] **Step 5: Run both tests and commit**

Run: `node --experimental-strip-types --test src/responsive-layout.test.ts src/cursor-mode.test.ts`
Expected: PASS.
Commit: `git commit -- wasm-www/index.html wasm-www/src/canvas-lifecycle.ts wasm-www/src/cursor-mode.ts wasm-www/src/cursor-mode.test.ts wasm-www/src/responsive-layout.test.ts -m "feat(web): make canvas touch-safe and safe-area aware"`

### Task 2: On-screen touch controls bound to game commands

**Files:**
- Create: `wasm-www/src/touch-controls.ts`, `wasm-www/src/touch-controls.test.ts`
- Modify: `wasm-www/index.html`, `wasm-www/src/main.ts`

**Interfaces:**
- Consumes: `RobinRpc = <T>(method: string, params?: unknown) => Promise<T>` from `replay.ts`.
- Produces: `installTouchControls(root: Pick<Document, "querySelector" | "querySelectorAll">, rpc: RobinRpc, touchEnabled: boolean): () => void`.

- [ ] **Step 1: Write the failing test**

`touch-controls.test.ts` builds a jsdom fragment containing `<div id="touch-controls" hidden>` with three buttons whose `data-robin-command` values are `{"ScriptKeyPressed":{"virtual_key":27}}`, `{"ScriptKeyPressed":{"virtual_key":32}}` and `{"ClearAlliedSelection":null}` (serde rename of `ClearTacticalSelection` in `crates/robin_engine/src/player_command.rs`). With `touchEnabled = true`, assert the root is unhidden; click each button and assert `rpc` was called with `("command", <parsed object>)`, not a JSON string. With `touchEnabled = false`, assert it stays hidden and clicks do nothing. Assert the returned dispose function removes listeners by clicking after disposal and checking no further calls.

- [ ] **Step 2: Run to verify failure**

Run: `node --experimental-strip-types --test src/touch-controls.test.ts`
Expected: FAIL on import.

- [ ] **Step 3: Implement `installTouchControls`**

Parse each button's `data-robin-command` once at install; on `click`, `void rpc("command", parsed).catch(...)` using the repo's log helper for errors. Guard against double-install by marking the root with a data attribute. Never mutate game rules: buttons only emit `PlayerCommand` JSON accepted by `crates/robin_rs/src/http_server/request_decode.rs` (`HttpPayload::Command`).

- [ ] **Step 4: Wire into the shell**

In `main.ts`, keep a module-level `disposeTouchControls: (() => void) | undefined`; inside `installRpcClient` (line ~630), after `globalThis.robinRpc = rpc`, dispose any previous instance and set `disposeTouchControls = installTouchControls(document, rpc, navigator.maxTouchPoints > 0 || pageParams.get("touch") === "1")`.

- [ ] **Step 5: Add the markup**

In `index.html`, add `<div id="touch-controls" hidden>` immediately after `#canvas` with three buttons: menu (`{"ScriptKeyPressed":{"virtual_key":27}}`), quick action (`{"ScriptKeyPressed":{"virtual_key":32}}`) and clear selection (`{"ClearAlliedSelection":null}`).

- [ ] **Step 6: Run tests and commit**

Run: `node --experimental-strip-types --test src/touch-controls.test.ts`
Expected: PASS.
Commit: `git commit -- wasm-www/index.html wasm-www/src/main.ts wasm-www/src/touch-controls.ts wasm-www/src/touch-controls.test.ts -m "feat(web): add on-screen touch controls via player commands"`

### Task 3: PWA manifest, icons and CSP

**Files:**
- Create: `wasm-www/public/manifest.webmanifest`, `wasm-www/public/icons/icon-192.png`, `wasm-www/public/icons/icon-512.png`
- Create: `wasm-www/scripts/generate-pwa-icons.mjs` (Node `zlib`, no new dependency)
- Create test: `wasm-www/src/pwa-manifest.test.ts`
- Modify: `wasm-www/index.html`, `wasm-www/deploy/public-headers.txt`

**Interfaces:**
- Consumes: `cspDirectives(html)` from `csp-test-utils.ts`.
- Produces: `/manifest.webmanifest` and two icons as static assets; `manifest-src 'self'` in both CSPs.

- [ ] **Step 1: Write the failing test**

`pwa-manifest.test.ts`: read `index.html` and assert `cspDirectives(html).get("manifest-src")` contains `'self'`, and that a `<link rel="manifest" href="/manifest.webmanifest">` exists. Read `public/manifest.webmanifest`, parse JSON, assert `name`, `short_name`, `display === "standalone"`, `start_url === "/"`, `background_color`, `theme_color`, and two icons with `sizes` `"192x192"` and `"512x512"` whose files exist under `public/icons/`. Read `deploy/public-headers.txt` and assert its CSP contains `manifest-src 'self'` while still containing `worker-src 'self' blob:` and all three COOP/COEP/CORP headers.

- [ ] **Step 2: Run to verify failure**

Run: `node --experimental-strip-types --test src/pwa-manifest.test.ts`
Expected: FAIL.

- [ ] **Step 3: Generate icons and write the manifest**

`scripts/generate-pwa-icons.mjs` writes two solid-colour PNGs (deep red `#8b1a1a`, transparent background) with a simple lighter mark, using Node's `zlib.deflateSync` and hand-built PNG chunks; run it once and commit the PNGs. Write `manifest.webmanifest` per the test. Add the manifest link and `manifest-src 'self'` to the meta CSP in `index.html`, and `manifest-src 'self'` to the `/*` CSP block in `deploy/public-headers.txt`.

- [ ] **Step 4: Run the test and commit**

Run: `node --experimental-strip-types --test src/pwa-manifest.test.ts`
Expected: PASS.
Commit: `git commit -- wasm-www/index.html wasm-www/deploy/public-headers.txt wasm-www/public wasm-www/scripts/generate-pwa-icons.mjs wasm-www/src/pwa-manifest.test.ts -m "feat(web): add PWA manifest, icons and CSP allowance"`

### Task 4: Service worker — shell + Demo only, fail closed

**Files:**
- Create: `wasm-www/public/sw.js`, `wasm-www/src/service-worker.test.ts`
- Modify: `wasm-www/src/main.ts`, `wasm-www/deploy/public-headers.txt`

**Interfaces:**
- Consumes: `/wasm/latest.json` (build manifest, provides the pinned Demo generation).
- Produces from `public/sw.js` (plain ESM, top-level SW registration guarded by `typeof self !== "undefined" && typeof self.skipWaiting === "function"` so Node can import it):
  - `isCacheable(url: string, pinnedGeneration: string | undefined): boolean`
  - `cacheNameFor(generation: string): string`
  - `isStale(cachedGeneration: string | undefined, latestGeneration: string): boolean`

**Rules pinned by the spec:** cache shell assets, manifest, icons and the Demo generation whose directory name equals `pinnedGeneration`; never cache `/datadirs/full/`, any URL reached after an auth challenge, any response without COOP/COEP/CORP, or any opaque/error response; a stale or partial cache falls back to the network.

- [ ] **Step 1: Write the failing test**

`service-worker.test.ts` imports `../public/sw.js` and asserts:
- `isCacheable("/", "v18") === true`, `isCacheable("/manifest.webmanifest", "v18") === true`, `isCacheable("/datadirs/demo/v18/v18-web-opus-q80.rhdata.zst", "v18") === true`.
- `isCacheable("/datadirs/demo/v17/...", "v18") === false`, `isCacheable("/datadirs/full/abc/datadir.bin", "v18") === false`, `isCacheable("https://other.example/x", "v18") === false`.
- `isStale("v17", "v18") === true`, `isStale(undefined, "v18") === true`, `isStale("v18", "v18") === false`.
- `cacheNameFor("v18")` is a fixed, versioned string.

- [ ] **Step 2: Run to verify failure**

Run: `node --experimental-strip-types --test src/service-worker.test.ts`
Expected: FAIL on import.

- [ ] **Step 3: Implement the worker**

`public/sw.js` exports the three pure functions, implements `install`/`activate` (delete caches whose name is not the current generation), and `fetch`: only call `isCacheable` after `fetch` resolves and the response has all three isolation headers; on any miss or stale key, return the network response without writing the cache. Add an explicit message handler that clears the cache when told the generation changed.

- [ ] **Step 4: Register in the shell and pin headers**

In `main.ts`, after `main()` succeeds in production (`import.meta.env.PROD`), `navigator.serviceWorker?.register("/sw.js")`, wrapped in try/catch that logs via the repo log helper. In `deploy/public-headers.txt`, add `Cache-Control: no-cache` and `Content-Type: application/javascript` for `/sw.js` and an explicit block preserving COOP/COEP/CORP on it.

- [ ] **Step 5: Run tests, verify headers, commit**

Run: `node --experimental-strip-types --test src/service-worker.test.ts` and `node --test scripts/verify-cloudflare-deployment.test.mjs`
Expected: PASS.
Commit: `git commit -- wasm-www/public/sw.js wasm-www/src/service-worker.test.ts wasm-www/src/main.ts wasm-www/deploy/public-headers.txt -m "feat(web): add fail-closed Demo-only service worker"`

### Task 5: Private full-campaign boot path (game repo side)

**Files:**
- Create: `wasm-www/src/private_campaign.ts`, `wasm-www/src/private_campaign.test.ts`
- Modify: `wasm-www/src/main.ts`, `wasm-www/src/mission-launch.ts`

**Interfaces:**
- Consumes: `parseHostedReplayContent(value, origin)` from `run-replay.ts` (already validates `/datadirs/full/<64-hex>/datadir.bin`), `RobinWasmModule`, `BuildSelection`.
- Produces: `resolvePrivateCampaignContent(pageUrl: URL, latestBuild: string, fetchJson: <T>(url: string, signal: AbortSignal) => Promise<T>, signal: AbortSignal): Promise<{ url: string; sha256: string; byteLength: number; parts: readonly string[] } | undefined>`; returns `undefined` when the page was not opened through the private capability path.

- [ ] **Step 1: Write the failing test**

`private_campaign.test.ts`: with `pageUrl = new URL("https://robin.jautis.lt/play/<32-hex>/")`, fake `fetchJson` returning a valid full-content binding (`/datadirs/full/<64-hex>/datadir.bin`, matching sha256 and byteLength), assert the resolver returns the URL/identity/parts. With a root page URL, assert it returns `undefined` without fetching. With a malformed token (not 32 hex), assert the same, and with a binding that fails `parseHostedReplayContent`, assert a descriptive error is thrown — never a silent fallback.

- [ ] **Step 2: Run to verify failure**

Run: `node --experimental-strip-types --test src/private_campaign.test.ts`

- [ ] **Step 3: Implement the resolver and wire the launch path**

Match `^/play/([0-9a-f]{32})/$` on `pageUrl.pathname`; fetch the campaign content binding from `/play/<token>/campaign.json`, validate with `parseHostedReplayContent`. In `main.ts`'s `loadDefaultContent`, call this resolver before the Demo path; when it returns a value, use it as the datadir source and pass `build.source === "latest" ? "no-cache" : "force-cache"` exactly as today. In `mission-launch.ts`, treat an explicit `?edition=campaign` as a request for this path so a replay never silently triggers it.

- [ ] **Step 4: Check the save envelope**

Add a test that a campaign save write over 4 MiB (`BROWSER_PROFILE_BYTE_LIMIT` in `crates/robin_rs/src/player_profile_store.rs`) is reported as an explicit error, never silently reset. If a real full-campaign continuation exceeds it, stop before Task 6 and open a separate task to move the campaign archive to IndexedDB; do not trim save history.

- [ ] **Step 5: Run the tests and commit**

Run: `node --experimental-strip-types --test src/private_campaign.test.ts`
Expected: PASS.
Commit: `git commit -- wasm-www/src/private_campaign.ts wasm-www/src/private_campaign.test.ts wasm-www/src/main.ts wasm-www/src/mission-launch.ts -m "feat(web): resolve private full-campaign content generation"`

### Task 6: Origin access control and published corpus (separate repo, needs owner inputs)

**Files (in `~/nixos-config`, separate commit):**
- Modify: `hosts/oci-vm/caddy.nix` — capability-path route, `basic_auth`, password hash from a secret, `/play/<token>/*` and `/datadirs/full/*` gated; full path excluded from indexes.
- Modify: `hosts/oci-vm/caddy.nix` — add `manifest-src 'self'` to the game vhost CSP; keep COOP/COEP/CORP on the gated responses.

**Owner inputs required:** which licensed install is the conversion source; the password value; the 32-hex token chosen for the capability path.

- [ ] **Step 1: Build the corpus on a machine with the licensed install**

Run the canonical converter (`scripts/build_web_shipping_datadir.sh`) on the owner's paid install, publish the generation under `/datadirs/full/<sha256>/` with its `Data/` closure and digests. Never commit the bytes or the source data to any repo.

- [ ] **Step 2: Configure Caddy**

Add the `basic_auth` block and the capability-path route; confirm the public vhost still serves the Demo generation and shell without a prompt.

- [ ] **Step 3: Verify from a clean browser**

Open the root URL — no auth prompt, Demo boots. Open `/play/<token>/` without credentials — auth prompt; with the shared password — campaign boots and continues after reload. Confirm no full-content URL is cached by the service worker.

- [ ] **Step 4: Commit in `nixos-config` by pathspec and deploy through its own gate.** Record the artifact hash and origin used in the evidence.

### Task 7: Device and cross-play proof (manual, after Tasks 1–6)

No code. Required evidence from the estate phone (POCO X7 Pro, current stable Chrome) and a native Linux or Windows host:
- tap/drag/pinch produce observable `PlayerCommand`s in the simulation;
- add-to-home-screen launches standalone; orientation and cutout screenshots;
- campaign begun and a later mission continued after reload;
- a native host plus the phone guest, with logs showing a browser-issued command changing the shared world;
- artifact hash and origin recorded in every piece of evidence.

Extend `docs/slop/browser-seat.md` with the played run. Fixing the Demo first-tick defeat remains out of scope (design §6.4).
