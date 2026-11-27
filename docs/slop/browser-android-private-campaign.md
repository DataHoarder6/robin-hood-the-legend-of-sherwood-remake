# Private browser campaign on the estate origin — evidence (2026-11-22)

Stream: browser/WASM Android client. Companion spec:
`../superpowers/specs/2026-11-22-browser-android-design.md`; plan:
`../superpowers/plans/2026-11-22-browser-android-client.md`.

## What is live

- Private link: `https://robin.jautis.lt/` — the entire origin is private. The
  root is the only campaign entry point; there are no capability tokens or
  legacy links. The root boots the full campaign by default (`?edition=demo`
  selects the Demo generation).
- Username and password: **`robin` / `robin`** (owner's choice). The sops secret
  stores only an argon2id hash (base64) rendered into
  `/run/secrets/robin-campaign-auth.caddy` at activation; no plaintext hash in
  git. Caddy `basic_auth` applies to the whole vhost, so every path needs the
  credentials; the identity signer is a separate origin and is unaffected.
- Caddy landed as `ed26a36`, `fdf93de`, `61168fe` on `nixos-config` main and
  deployed (`just deploy-wait 61168fe`).
- Verified after `d7e06ac` deployed: `https://robin.jautis.lt/` returns 401
  without credentials, 200 with `robin`/`robin`, and 401 with the old password.
  `robin-id.jautis.lt` is a separate origin and is unchanged.

## Corpus

- Source: the owner's paid GOG install at `/home/matt/Games/robin-hood-data`,
  with its neutral 2047 overlay (`2047/data`) merged into `DATA/` before
  conversion.
- Built with the canonical shipping recipe on `minipc`: AVIF 1.4.2/libaom
  3.15.0, Opus tools 0.2 on libopus 1.6.1 (verified by the converter), full
  edition, AVIF q60, Opus, zstd window 30. No lossless remaster drop was
  available, so `--lossless-music-dir` was omitted and the converter encoded
  music from the game's own WAVs (the documented fallback).
- The first conversion used a converter built from a different commit and
  produced `native bitcode mission decode: Error("bitcode error")` on the first
  map. It was replaced: the converter was rebuilt from the deployed wasm commit
  `95ab7b9dbdf25725789086ea83fd6a78a4479e1b` with
  `ROBIN_PACKAGE_COMMIT` set, so the manifest carries that exact engine version.
- Published generation:
  `datadirs/full/ada1a6ff9d36a620f7a2094d67ad5a1886091639bce451a485ff3daffe3d3607/`,
  `datadir.bin` 118,781,741 bytes, SHA-256 `4fb3cdff…d5`, chunked into five
  `.partN` files as the shell expects. Binding: `/campaign.json`. The old
  generation and the legacy capability path were deleted.
- Verified in Chrome with `?mission=H01_Lin_VL`: `mission started`, no bitcode
  error, no game-loop failure; `robinRpc('state')` answered with `frame = 2`,
  `presentation = null` after the level finished loading. Screenshots:
  `/tmp/opencode/first-map-fixed.png`, `/tmp/opencode/first-map-loaded.png`.
- The service worker rejects every `/datadirs/full/*` URL, so the private
  corpus is never cached.

## Operator campaign binding (D20, #127)

The private root fetches `/campaign.json?build=<short>`; the build returns 404
when deployment has not published that binding. Deployment now stages and gates
it before the atomic swap.

- Contract: one JSON object with `url`, `sha256` and `byteLength`, exactly the
  shape `parseHostedReplayContent` accepts (`wasm-www/src/run-replay.ts`).
  `url` is a same-origin path `/datadirs/full/<64-hex>/datadir.bin`; `sha256`
  is 64 lowercase hex digits; `byteLength` is a positive integer at most
  256 MiB. Part names are derived from `byteLength` by the client, not stored
  in the binding.
- Fixture location: `/var/www/robin/datadirs/campaign.json` on `oci-vm`, beside
  the licensed corpus. Not yet created there as of 2026-11-27: the operator
  writes it, and it never enters git or the built site. Supply it explicitly
  as `CAMPAIGN_BINDING_PATH=/var/www/robin/datadirs/campaign.json` to
  `scripts/deploy_web_jautis.sh <v-tag>`; an absent path aborts before staging.
- Staging contract: the deploy copies the binding to
  `<staged root>/campaign.json` with mode `0644` and the owner/group of the
  staged `index.html`. The operator file's own mode is not preserved: a source
  that is `0600` would pass a loopback fetch but return 403 from Caddy after
  the swap. No reference file aborts before staging.
- Gate: the deploy fetches `/campaign.json?build=<short>` over loopback. It
  requires HTTP 200, served bytes equal to the staged bytes, and the exact
  `parseHostedReplayContent` shape validated in `jq` before shell extraction
  (a trailing newline in `url` or `sha256` is rejected). It also checks every
  derived part's total length and SHA-256. A missing, malformed or mismatched
  binding exits before either root is swapped; the rollback trees are
  untouched.
- Hermetic evidence: `wasm-www/scripts/static-deployment-workflow.test.mjs`
  executes the gate's shell function against generated fixtures. No live
  origin, licensed corpus or secret is used.
- Last verified: 2026-11-27 (workflow tests, deployment suite, wasm-www format
  and lint).

## Browser proof

Playwright (owner's visible Chrome, emulated 390x844 phone, current site):

- With `httpCredentials`, the private URL boots: wasm instantiated, manifest
  and assets preloaded, `118779443` byte full datadir verified, handoff to
  winit, in-game menu resources loaded (343 menu-text entries), cross-origin
  isolated with 12 decode threads. Screenshot: `/tmp/opencode/private-campaign-phone.png`.
- The phone-sized HUD shows the three touch buttons; after the coarse-pointer
  fix, the desktop cursor-capture control is hidden.
- On-screen buttons are wired to `rh_rpc("command", <PlayerCommand JSON>)` and
  unit-tested; before a mission is active the engine correctly reports
  `engine not ready — no active mission RPC session`, which is also what a
  tap showed in the browser.

## Not yet proved

- Starting a mission from the menu by touch and observing a player command
  change the simulation. Menu taps moved the in-game cursor but did not open a
  mission in the automated browser run; finishing this needs a human-driven
  session on the estate phone (POCO X7 Pro) or further input-bridge work.
- Repeatable add-to-home-screen and reload/continue proof on that phone.
- Native-host cross-play with a browser guest (design §6.4) — the relay path is
  unchanged and previously proved at the transport level only.

## Caveats

The corpus is for two licensed owners behind a password, not a public release.
No game data, converter binary or password hash is in any repo.
