# Map render pipeline — authoring twenty maps without a GUI

Research date: 2026-11-19. Read-only investigation; no code changed.
Citations are `[Rn]` and resolve to a URL plus retrieval date in
[References](#references). Measurements marked **measured** were taken on the
estate runtime on 2026-11-19 and are reproducible with the command given.

## Verdict

Keep the browser bake and drive it headlessly — it is the only path that
produces the image the game actually ships. `level-editor/app/src/map-bake-render.ts`
already renders through `THREE.WebGLRenderer` into 1024-pixel tiles, reads them
back with `readRenderTargetPixels`, and the repo already has a hand-rolled CDP
harness (`app/tests/run-lifecycle.mjs`) that launches Chromium with
`--enable-unsafe-swiftshader --use-angle=swiftshader`, requires the fixture to
print `PASS`, and otherwise exits non-zero with the captured console and
exception text. **Measured**: WebGL 2 works on this GPU-less
host with that stack (`ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (LLVM
10.0.0)), SwiftShader driver)`, `MAX_TEXTURE_SIZE` 8192). What is missing is not
a renderer but a *driver*: one command that loops twenty scene names through
`map-bake-library.html?map=<name>`, writes each ZIP, and gates the result on a
blank-image check and a cross-run pixel diff. Replace the hand-rolled CDP layer
with a pinned Playwright Chromium for the loop (version pinning, download
capture, console/exception surfacing come free); keep the existing harness for
the fixture tests. Reject Blender (re-shades the scene; EEVEE needs a GL
context), `headless-gl` (WebGL 2 untested, needs Xvfb+Mesa, open `texImage3D`
failures with three), and a Rust/wgpu path (the game's renderer is built around
a window surface; it would be a second renderer to keep in sync with the
shipping one).

## What already exists — read this before proposing anything

Four render paths exist in this repo. Only the first produces the shipped
background.

| Path | What it renders | GPU/display needed | Entry point |
|---|---|---|---|
| Editor bake (WebGL) | the shipping `map.png`, `min.png`, `occlusion-depth.png` | WebGL 2; SwiftShader is enough | `level-editor/app/src/map-bake-render.ts`, page `app/tests/map-bake-library.html` |
| Node software rasterizer | terrain volumes + textures, no glTF assets, no shadows | none | `level-editor/pipeline/src/render.ts`, `pipeline/src/bake.ts` |
| Game renderer, hidden window | a live mission frame, `/screenshot?full_map=1` over HTTP | wgpu + a window (hidden) | `crates/robin_rs/examples/render_mission_map.rs`, `crates/robin_rs/src/http_server.rs` |
| Browser fixture bake | synthetic crop/tile/depth contract | WebGL 2 | `app/tests/map-bake.ts` (`map-bake.html`) |

What the export writes, and the depth encoding, is
`level-editor/docs/map-compilation.md` — do not restate it here.

Two facts from the bake that constrain any replacement: the tile size is
`min(1024, renderer.capabilities.maxTextureSize)` and the depth pass patches
materials with `onBeforeCompile`. Any renderer that does not run *this* code
will not produce the same bytes.

## 1. Driving the editor headlessly

### The one supported GPU-less stack

Chromium renders WebGL on the CPU through SwANGLE (ANGLE over SwiftShader
Vulkan). The Chromium GPU doc lists exactly three switches `[R1]`:

| Intent | Switches | Note |
|---|---|---|
| SwANGLE as the OpenGL ES driver | `--use-gl=angle --use-angle=swiftshader` | "behaves as if running on a regular GPU" — exercises hardware-only code paths |
| SwANGLE as the **unsafe WebGL fallback** | `--use-gl=angle --use-angle=swiftshader-webgl --enable-unsafe-swiftshader` | needed only when Chromium would otherwise refuse |
| SwiftShader as the Vulkan driver | `--use-vulkan=swiftshader` | requires the `enable_swiftshader_vulkan` build feature |

The same doc states automatic SwiftShader WebGL fallback "has been deprecated
and WebGL context creation will soon fail instead of falling back", and that
Chromium "do[es] not guarantee WebGL availability" — so treat context creation
as a checkable precondition, not an assumption `[R1]`. The deprecation is tied
to M139 in the enterprise policy `EnableUnsafeSwiftShader`, which exists to
re-enable it `[R10]`; M139 reached stable 2025-07-30/2025-08-19 `[R9]`.

SwiftShader is a conformant implementation — Vulkan 1.3 and OpenGL ES 3.1
conformance submissions are cited in its README `[R11]` — so this is not a
"roughly right" rasterizer. It is slow, and it is JIT-compiled code inside the
GPU process, which is why Chromium gates it behind a flag `[R1]`.

### Measured on this host (2026-11-19)

Host: estate runtime, arm64, 2 cores, `/dev/dri` present but no usable Vulkan
device. Binary: Playwright's bundled Chrome for Testing
`156.0.8078.4` (`/nix/store/…-playwright-chromium/bin/chromium`).
Probe: `chrome --headless --no-sandbox --dump-dom 'data:text/html,<script>…getContext("webgl2")…'`.

| Flags | Result |
|---|---|
| none | WebGL 2 OK — renderer `ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (LLVM 10.0.0)), SwiftShader driver)` |
| `--enable-unsafe-swiftshader` | WebGL 2 OK, same renderer |
| `--disable-gpu --enable-unsafe-swiftshader` | WebGL 2 OK, same renderer |
| `--use-gl=angle --use-angle=swiftshader --enable-unsafe-swiftshader` | WebGL 2 OK, same renderer |
| `--use-angle=vulkan` | **no WebGL 2** — no hardware Vulkan device here |

Context limits measured in the same probe: `MAX_TEXTURE_SIZE` 8192,
`MAX_SAMPLES` 4, `EXT_color_buffer_float` present. `WEBGL_depth_texture` and
`ANGLE_instanced_arrays` report absent *because they are core in WebGL 2* —
do not read that as a missing feature.

Two consequences. The bake's 1024 tile is safe here (8192 ≥ 1024). And the
automatic fallback still worked in Chrome 156 without the flag, which
contradicts the M139 statement above — see [Open questions](#open-questions).

Pass the flags anyway. They cost nothing and they are what makes the behaviour
independent of a policy default that has already been announced for removal.

### Playwright vs Puppeteer vs raw CDP

| | Raw CDP (today's harness) | Puppeteer | Playwright |
|---|---|---|---|
| Browser pinning | you download and pin yourself | `npx @puppeteer/browsers install chrome@<ver>` `[R7]`, `PUPPETEER_CACHE_DIR` `[R3]` | browser build is fixed by the Playwright version; `npx playwright install --list` prints it `[R5]` |
| Headless variants | `--headless` (old mode removed) | `headless: true` = new headless since v22; `headless: 'shell'` = `chrome-headless-shell` `[R4]` | headless uses `chromium-headless-shell`; `channel: 'chromium'` opts into new headless; `--only-shell` / `--no-shell` pick what is downloaded `[R5]` |
| Download capture | hand-rolled: read `window.__bakeZip`, chunk it over `Runtime.evaluate` | `Browser.setDownloadBehavior` + CDP events | `page.on('download')`, `downloadsPath` on launch `[R6b]` |
| Error surfacing | you wire `Runtime.exceptionThrown` / `consoleAPICalled` (already done in `run-lifecycle.mjs`) | `page.on('pageerror')`, `console` | same, plus a trace |
| Verdict | works, no dependency, 123 lines to maintain | fine, but its API drifts with Chrome | **use for the twenty-map driver** |

`chrome-headless-shell` is the old headless implementation shipped as its own
binary from Chrome 120 through Chrome for Testing `[R7]`; it "does not require
X11/Wayland, D-Bus" `[R7]`, which is exactly right for a build host. Its
documented limitation is that it "requires `--enable-gpu` to enable GPU
acceleration in headless mode" `[R3]` — irrelevant for SwiftShader (CPU), and a
trap if someone later expects hardware GL from the shell. New headless
(`channel: 'chromium'`) is "the real Chrome browser" and is the better choice
when fidelity matters more than startup cost `[R7]`.

Playwright now runs on Chrome for Testing builds rather than Chromium
(1.57); 1.63 bundles Chromium 153 and 1.65 is current `[R6]`. Chrome for Testing publishes
`known-good-versions.json` and per-version asset URLs, so an exact browser
version is a pinnable string `[R8]`.

### Determinism and quality caveats

- Screenshot determinism is a browser-level concern; the bake avoids it
  entirely by reading pixels out of a render target, so `--hide-scrollbars`
  (documented as "Useful for taking consistent screenshots" `[R2]`) and
  colour-profile flags do not apply. What does apply: fixed camera, fixed
  light camera, `shadowMap.autoUpdate = false`, `stableOpaqueSort`, and
  integer tile rectangles — all already in `map-bake-render.ts`.
- Determinism is *within a pin*, not across upgrades. SwiftShader's shader
  compiler and ANGLE's translation are versioned; a browser bump is a
  renderer bump. Diffing must therefore compare against a baseline recorded
  with the same pin, and a pin bump must be an explicit, reviewed re-baseline.
- Colour: the bake sets `outputColorSpace = SRGBColorSpace` and encodes depth
  as two bytes in a `NoColorSpace` target. SwiftShader's blending is
  spec-exact, so no dithering or driver-specific rounding enters the image.

## 2. Rendering Three.js off-browser

| Option | Status 2026-11 | WebGL 2 features the editor uses | Verdict |
|---|---|---|---|
| `headless-gl` (`gl`) | alive but pre-release: `9.0.0-rc.10` (2026-04-10), npm `latest` still `8.1.6` (2024-10-29) `[R17]`. WebGL 2 landed 2025-07-22 (`#310`) `[R18]` and the README says "Every WebGL2 method is implemented, but WebGL2 hasn't been tested against the WebGL 2.0 conformance suite yet" `[R16]` | instancing yes; float colour buffers via `EXT_color_buffer_float` yes; depth textures yes. But an open issue since 2025-01-04 reports `gl.texImage3D is not a function` with three.js `[R19]` | **no.** Also needs Xvfb + Mesa on Linux — its own CI FAQ says so `[R16]` |
| three "software renderer" | removed in 2019 (`#17809`) `[R14]` | n/a | does not exist |
| `@napi-rs/canvas` + three | Skia 2D binding, no GL `[R21]` | none | not a renderer for meshes |
| three `WebGPURenderer` on a Node backend | WebGPU is exposed by Deno (2.9+, desktop windows) `[R20]`, not by Node; and the renderer explicitly does not support "`ShaderMaterial`, `RawShaderMaterial` and modifications of built-in materials via `onBeforeCompile()`" `[R13]` | the editor's depth pass *is* an `onBeforeCompile` patch | **no** without rewriting the depth encode |
| `wgpu` bindings / `headless-three-renderer` | hobby project, 3 stars, no licence, created 2025-09 `[R36]` | n/a | not a dependency to take |

The decisive fact is in three's own history: r163 removed WebGL 1 from
`WebGLRenderer` `[R12]`, so any off-browser path must provide WebGL 2 — and the
only mature software WebGL 2 is SwiftShader, which is what Chromium already
gives you.

## 3. Blender as the offline renderer

Blender 5.3 is current; `blender-5.3.0-linux-x64.tar.xz` is dated 2026-11-10 in
the release directory `[R26]`.

Documented entry points `[R22]`:

```sh
blender -b scene.blend --python-expr "…"        # -b / --background: "Run in background (often used for UI-less rendering)"
blender -b scene.blend -o //out -f 1            # --render-output, --render-frame
```

`--python-expr` "may be a complete multi-line script; you are limited only by
the platform's maximum argument length" `[R22]` — so a real pipeline writes a
`.py` file and uses `--python`, not a giant `-expr`.

Engine choice:

- **EEVEE** is rasterization and needs a GL context. Headless rendering under
  Linux has been supported since 3.4 `[R23]`; it still needs a working driver,
  and Blender 5.0 raised the minimum GPU requirements `[R24]`. On a GPU-less
  host that means Mesa llvmpipe `[R28]` — supported in the sense that it runs,
  slow in the sense that it is a software GL implementation.
- **Cycles CPU** always runs; `scene.cycles.seed` is documented as "Seed value
  for integrator to get different noise patterns", and "Use Animated Seed"
  changes it per frame `[R25]` — so pin the seed and leave animated seed off.
  Cross-version determinism is not documented; treat a Blender upgrade as a
  re-baseline.

Why it is still the wrong answer here: the bake is not a generic render of a
glTF scene. It has an oblique-orthographic camera at 1 px per map unit, a sun
model owned by `app/src/sun-lighting.ts`, per-tile shadow-map reuse, patch
appearance states, spline deformation, and a 16-bit ground-depth pass whose
encoding is defined in `map-compilation.md`. Reproducing that in Blender means
re-implementing the editor and then proving pixel parity forever. Time-of-day
would be a sun lamp rotation in Blender and a `SunLighting.sync()` call in the
editor — two sources for one fact.

Render time for 1792×1280 with shadows: **not measured** — no Blender on this
host. Do not quote a number until someone runs it on the gate host.

## 4. Rust with the game's own renderer

`wgpu` 30.0.2 is current (2026-11-12) `[R27]`, and offscreen rendering needs no
surface: create a texture, render, copy to buffer. The blocker is this repo's
renderer, not wgpu. `crates/robin_rs/examples/render_mission_map.rs` documents
its `--headless` flag as "Keep the required **GPU-backed window** hidden.
Rendering still occurs for the final offscreen screenshot", and the entry point
is `window::run_with_game_visibility(…, window_visible, …)`. `winit` needs a
display server even for a hidden window, so "no GUI" becomes "Xvfb plus a
window".

GPU-less Vulkan is practical in principle: lavapipe is Mesa's LLVM-based
software Vulkan driver, and Mesa's llvmpipe (the GL equivalent) is "the fastest
software rasterizer for Mesa", multithreaded up to 32 cores `[R28]`. Selecting
it is an ICD environment variable, and `wgpu`'s `Instance::request_adapter`
returns an error when no adapter satisfies the request `[R27]` — a clean
failure, which is what you want.

Maintainability is the deciding factor: a Rust path is a *second* map renderer.
The editor bake is what ships, so the Rust path must either reproduce it
bit-for-bit (impossible to keep) or become the new source of truth (a rewrite
of the export, its depth encoding, and the ZIP contract, plus the Rust loader
contract test that reads it). The one thing worth reusing later is the game's
`/screenshot?full_map=1` endpoint, which already renders a full map offscreen
and returns PNG `[R-http]` — but it renders *sprites and terrain*, not the
authored map background, so it is a preview, not the export.

## 5. Prior art

| Project | Mechanism | GPU-less host | Determinism | What it teaches |
|---|---|---|---|---|
| Tiled `tmxrasterizer` | ships with Tiled, "able to render any supported map format to an image" `[R29]` | yes — CPU rasterizer, no GL | not documented | the pattern to copy: a headless CLI that renders the *same* map format the editor edits |
| Godot `--headless` | `--display-driver headless --audio-driver Dummy`; docs say it is "required on platforms that do not have GPU access (such as continuous integration)" `[R30]` | yes for export; whether a captured viewport works under the headless display driver is not stated in the docs, so treat capture as needing an explicit rendering driver | n/a | headless is documented for *exporting*, not for pixels |
| Unity `-batchmode` | `-nographics` "doesn't initialize the graphics device … machines that don't have a GPU" `[R31]` | same split: batchmode yes, pixels no | n/a | confirms the industry split: batch automation and rendering are different modes |
| LDtk | — | — | — | **not verified**: `ldtk.io/docs/cli/` returns 404; no documented image export found, so nothing is claimed |

Nobody in this list renders authored 3D scenes GPU-lessly without a software GL
stack. The established practice is either "render with a CPU rasterizer that is
part of the tool" (Tiled) or "run the real renderer with a software driver".
This repo already has both halves; the missing piece is orchestration.

## 6. Reliability mechanics

**Pin**

| Thing | How | Why |
|---|---|---|
| three | `level-editor/pnpm-lock.yaml` already pins `three@0.186.0` | shader and material behaviour change between minors |
| browser | exact Playwright version in `package.json` (no caret) + `npx playwright install --only-shell`; print `npx playwright install --list` and `chrome --version` into the run manifest | SwiftShader/ANGLE ship inside the browser |
| browser, alternative | `npx @puppeteer/browsers install chrome-headless-shell@<exact>` `[R7]`, version chosen from `known-good-versions.json` `[R8]` | when you want the browser decoupled from the driver |
| flags | one exported constant, used by both the fixture harness and the map driver | a flag edited in one place and not the other is a silent behaviour split |
| inputs | the map document already pins `model_sha256` / `descriptor_sha256` per asset (`library/scenes/*.rhlos-map.json`) | the render is a function of these hashes |
| node | add `.node-version` — none exists today | `pnpm-lock.yaml` does not pin the runtime |

**Fail loudly** — the existing harness already collects the first two:

| Failure | Detection |
|---|---|
| page error / console error | `Runtime.exceptionThrown` + `Runtime.consoleAPICalled(type=error)`; `run-lifecycle.mjs` records both and prints them when the fixture does not print `PASS`. Make the driver treat any recorded error as a failure even on `PASS` |
| no WebGL 2 | probe `canvas.getContext('webgl2')` before navigating to the map; abort with the renderer string |
| context lost mid-bake | `renderer.getContext().isContextLost()` → the bake already throws |
| shader compile error | `renderer.debug.onShaderError` → the bake already throws |
| blank / black render | decode the PNG; fail if ≥ 99 % of pixels equal the modal colour, or if channel standard deviation is ~0 |
| empty depth | decode `occlusion-depth.png`; fail if the fraction of non-zero pixels is 0, or below the coverage the previous baseline had |
| truncated export | ZIP must contain every file listed in `map-compilation.md`; missing member → fail |
| wrong size | `map.png` dimensions must equal the export frame from the descriptor; depth PNG must match |

**Compare across runs** — exact equality is the goal inside a pin:

| Tool | Use | Status |
|---|---|---|
| SHA-256 per PNG, recorded in a manifest | primary gate: any hash change fails | stdlib |
| `pixelmatch` | localise a diff; `threshold: 0` for "must be identical" | v8.0.0, 2026-10-06 `[R32]` |
| `odiff` | same, much faster on large images (SIMD) | v4.5.0, 2026-07-23 `[R33]` |
| `dssim` | judge a *deliberate* change ("is this still the same map?") | 3.5.1, 2026-08-29 `[R34]` |
| ImageMagick `compare` | one-liner in a shell gate: `magick compare -metric AE a.png b.png diff.png` | documented `[R35]` |

Depth is 16-bit and must be compared exactly, never perceptually.

## Comparison

Scored 1–5, higher is better. "Quality" means fidelity to the image the game
ships.

| Approach | Reliability | Ease of access | Quality | Maintenance | Total |
|---|---|---|---|---|---|
| Editor bake, Playwright driver (**recommended**) | 5 | 5 | 5 | 4 | **19** |
| Editor bake, today's raw-CDP harness | 4 | 3 | 5 | 3 | 15 |
| `pipeline` Node software rasterizer | 4 | 4 | 1 | 3 | 12 |
| Blender offline | 3 | 2 | 2 | 2 | 9 |
| Rust/wgpu offscreen | 3 | 1 | 3 | 2 | 9 |
| `headless-gl` + three in Node | 2 | 2 | 3 | 1 | 8 |

Reliability scores: the browser path loses a point only on cross-version
drift, which pinning plus a re-baseline answers. `headless-gl` scores 2 because
WebGL 2 is explicitly untested against the conformance suite and three.js
interop has an open bug `[R19]`.

## Recommended architecture

One command, one browser, twenty maps, no GUI.

```
level-editor/pipeline/src/bake-maps.ts        new driver (Playwright)
level-editor/app/tests/map-bake-library.html  unchanged render page
level-editor/app/src/map-bake-render.ts       unchanged renderer
level-editor/pipeline/src/bake-verify.ts      new: ZIP contract + blank/depth/hash gates
```

Shape of the driver:

1. Spawn `vite` (dev server — `vite.lifecycle.config.ts` does **not** mount the
   library middleware, so `preview` of the lifecycle build cannot serve
   `/library/`; the dev server can, via `serveLibrary()` in `app/vite.config.ts`).
2. Launch one pinned browser with the SwiftShader flags and a fresh profile.
3. Probe WebGL 2 and record the renderer string in the manifest.
4. For each scene name from `/library/scenes/index.json`: navigate to
   `/tests/map-bake-library.html?map=<name>`, wait for `#result` to start with
   `PASS` or `FAIL` (never a fixed sleep), fail the map on `FAIL` or on any
   captured console/page error.
5. Read the archive (`window.__bakeZip`, or `page.on('download')` if the button
   is clicked), write it, then run the verify gates.
6. Emit `bake-manifest.json`: per map, the browser version, the renderer
   string, the flag set, the input asset hashes, and a SHA-256 per output file.
7. Render every map, then exit non-zero listing every failure — one broken map
   must not hide the other nineteen.

Commands. The first block works today, per map, with no new code:

```sh
cd level-editor
pnpm install --frozen-lockfile
pnpm --filter app dev --host 127.0.0.1 --port 5182 --strictPort
```

```sh
# second terminal; CHROME points at a pinned browser build. On NixOS the
# Playwright bundle in the store is the one that runs — the ~/.cache/ms-playwright
# binaries fail with "Could not start dynamically linked executable" (stub-ld).
CHROME=$(ls -d /nix/store/*-playwright-chromium/bin/chromium | tail -1)
"$CHROME" --version   # record this in the manifest
TEST_PAGE=map-bake-library.html \
TEST_QUERY='?map=york' \
TEST_BAKE_ZIP=work/needs-review/map-bake/york.zip \
  node app/tests/run-lifecycle.mjs http://127.0.0.1:5182
```

That is the whole render. Twenty maps is a loop over `TEST_QUERY`, which is why
it must become a driver and not a shell loop: one browser, twenty navigations,
one manifest.

With the driver in place:

```sh
cd level-editor
pnpm --filter app exec playwright install --with-deps --only-shell
pnpm --filter pipeline exec node src/bake-maps.ts \
  --library ../library \
  --out ../work/needs-review/map-bake \
  --maps all
```

On this estate the browsers come from the nix store
(`~/nixos-config/overlays/default.nix` symlinks Playwright's bundle as
`playwright-chromium`), so `playwright install` is unnecessary here and the
`CHROME` path above is what to use; a generic CI host needs the install step.

Cost control: SwiftShader is CPU-bound and scales with cores, not with a GPU.
Render maps sequentially in one browser; parallelism is more browser processes,
which on a 2-core host is slower, not faster. Measure before adding any.

## Open questions

1. **The M139 statement does not match the measurement.** The policy says WebGL
   context creation fails without `--enable-unsafe-swiftshader` from M139
   `[R10]`, but Chrome for Testing 156 created a SwiftShader WebGL 2 context
   with no flags on 2026-11-19. Either the enforcement is conditional (policy
   default, or CfT builds differ) or it was reverted. Passing the flag makes the
   question moot; someone should still find the current statement before
   relying on the fallback.
2. **No measured bake time.** Nothing here says how long twenty maps take on
   the gate host. Measure with `time` around the driver; the tile count is
   `ceil(w/1024) × ceil(h/1024) × 2` per map.
3. **Cross-machine byte equality is unverified.** Two hosts with different CPUs
   run the same SwiftShader build; rasterization should be exact, shader
   compilation is the risk. Record a baseline on the host that will run the
   gate, not on a laptop.
4. **Blender numbers are unverified** — no Blender on this host, so no
   EEVEE/Cycles time for 1792×1280.
5. **`headless-gl` + three r186 was not tried.** The README's WebGL 2 caveat and
   the open `texImage3D` issue are enough to rule it out, but nobody has proven
   it fails on *this* scene.
6. **Whether `/screenshot?full_map=1` matches the editor bake's framing** is
   unchecked. If it did, it would be a cheap preview; it is not the export.
7. **Twenty maps do not exist yet.** `library/scenes/` holds ten documents; a
   map document is plain JSON with pinned asset hashes, so authoring is text
   work, but the asset library must already contain every model a new map
   places.
8. **`three` r187 exists (2026-10-30)** `[R15]`; the editor pins 0.186.0. A bump
   is a renderer bump and needs a re-baseline, not a silent upgrade. Note also
   that `PCFSoftShadowMap`, which the bake uses, is deprecated since r182 in
   favour of a soft `PCFShadowMap` `[R12]`.

## References

All retrieved 2026-11-19.

| Ref | Source | URL |
|---|---|---|
| R1 | Chromium, "Using Chromium with SwiftShader" | https://chromium.googlesource.com/chromium/src/+/main/docs/gpu/swiftshader.md |
| R2 | Chromium, `content/public/common/content_switches.cc` (`--hide-scrollbars`) | https://chromium.googlesource.com/chromium/src/+/main/content/public/common/content_switches.cc |
| R3 | Puppeteer, Troubleshooting (`chrome-headless-shell` needs `--enable-gpu`; crbug 1416283) | https://pptr.dev/troubleshooting |
| R4 | Puppeteer, Headless mode (`headless: 'shell'`, v22 default) | https://pptr.dev/guides/headless-modes |
| R5 | Playwright, Browsers (headless shell, `channel: 'chromium'`, `--only-shell`, `--no-shell`) | https://playwright.dev/docs/browsers |
| R6 | Playwright, Release notes (Chrome for Testing since 1.57; 1.63 bundles Chromium 153; 1.65 current) | https://playwright.dev/docs/release-notes |
| R6b | Playwright, Downloads (`page.on('download')`, `downloadsPath`) | https://playwright.dev/docs/downloads |
| R7 | Chrome for Developers, "Chrome headless shell" (old headless as a separate binary from Chrome 120; `@puppeteer/browsers`) | https://developer.chrome.com/blog/chrome-headless-shell |
| R8 | GoogleChromeLabs, Chrome for Testing availability (JSON endpoints, supported binaries) | https://github.com/GoogleChromeLabs/chrome-for-testing |
| R9 | Chromium Dash, milestone schedule for M139 and M153 | https://chromiumdash.appspot.com/fetch_milestone_schedule?mstone=139 |
| R10 | Chromium policy `EnableUnsafeSwiftShader` ("Starting in M139, WebGL context creation will fail…") | https://raw.githubusercontent.com/chromium/chromium/main/components/policy/resources/templates/policy_definitions/Miscellaneous/EnableUnsafeSwiftShader.yaml |
| R11 | SwiftShader README (Vulkan 1.3 / GLES 3.1 conformance footnotes) | https://github.com/google/swiftshader |
| R12 | three.js Migration Guide (WebGL 1 removed in r163; `PCFSoftShadowMap` deprecated) | https://github.com/mrdoob/three.js/wiki/Migration-Guide |
| R13 | three.js manual, WebGPURenderer (WebGL 2 fallback; `onBeforeCompile` unsupported) | https://threejs.org/manual/pages/webgpurenderer |
| R14 | three.js PR #17809, "Removed SoftwareRenderer" | https://github.com/mrdoob/three.js/pull/17809 |
| R15 | three.js releases (r186 2026-09-24, r187 2026-10-30) | https://github.com/mrdoob/three.js/releases |
| R16 | headless-gl README (WebGL 2 caveat; Linux needs Xvfb + Mesa) | https://github.com/stackgl/headless-gl |
| R17 | headless-gl releases (`9.0.0-rc.10`, 2026-04-10) and npm `gl` (`8.1.6`, 2024-10-29) | https://github.com/stackgl/headless-gl/releases |
| R18 | headless-gl PR #310, "Initial support for creating WebGL 2 contexts" | https://github.com/stackgl/headless-gl/pull/310 |
| R19 | headless-gl issue #304, `gl.texImage3D is not a function` with three.js | https://github.com/stackgl/headless-gl/issues/304 |
| R20 | Deno docs, WebGPU rendering (available in Deno 2.9, desktop windows) | https://docs.deno.com/runtime/desktop/webgpu |
| R21 | `@napi-rs/canvas` README (Skia binding, no GL) | https://github.com/Brooooooklyn/canvas |
| R22 | Blender manual, Command Line Arguments (`-b/--background`, `--python-expr`, `-f`, `-o`) | https://docs.blender.org/manual/en/latest/advanced/command_line/arguments.html |
| R23 | Blender 3.4 release notes, EEVEE ("Headless rendering is now supported under Linux") | https://developer.blender.org/docs/release_notes/3.4/eevee/ |
| R24 | Blender 5.0 release notes ("Increase Minimum GPU requirements for Blender") | https://developer.blender.org/docs/release_notes/5.0/ |
| R25 | Blender manual, Cycles sampling (Seed, Use Animated Seed) | https://docs.blender.org/manual/en/latest/render/cycles/render_settings/sampling.html |
| R26 | Blender release downloads (5.3.0, files dated 2026-11-10) | https://download.blender.org/release/Blender5.3/ |
| R27 | wgpu 30.0.2 (crates.io 2026-11-12) and `Instance::request_adapter` docs | https://docs.rs/wgpu/30.0.2/wgpu/struct.Instance.html |
| R28 | Mesa, LLVMpipe (software rasterizer, multithreaded to 32 cores) | https://docs.mesa3d.org/drivers/llvmpipe.html |
| R29 | Tiled docs, Export as Image (`tmxrasterizer`) | https://github.com/mapeditor/tiled/blob/master/docs/manual/export-image.rst |
| R30 | Godot docs, Command line tutorial (`--headless`, GPU-less CI) | https://docs.godotengine.org/en/stable/tutorials/editor/command_line_tutorial.html |
| R31 | Unity manual, Editor command line arguments (`-batchmode`, `-nographics`) | https://docs.unity3d.com/Manual/EditorCommandLineArguments.html |
| R32 | pixelmatch v8.0.0 (2026-10-06) | https://github.com/mapbox/pixelmatch |
| R33 | odiff v4.5.0 (2026-07-23) | https://github.com/dmtrKovalenko/odiff |
| R34 | dssim 3.5.1 (2026-08-29) | https://github.com/kornelski/dssim |
| R35 | ImageMagick, `compare` | https://imagemagick.org/compare/ |
| R36 | `portwatcher/headless-three-renderer` (three.js on wgpu, 3 stars, created 2025-09-22) | https://github.com/portwatcher/headless-three-renderer |
| R-http | `crates/robin_rs/src/http_server.rs` (`/screenshot?full_map=1`) | repo file |
