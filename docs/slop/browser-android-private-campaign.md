# Private browser campaign on the estate origin — evidence (2026-11-22)

Stream: browser/WASM Android client. Companion spec:
`../superpowers/specs/2026-11-22-browser-android-design.md`; plan:
`../superpowers/plans/2026-11-22-browser-android-client.md`.

## What is live

- Private link: `https://robin.jautis.lt/play/eaab90981616b10b19c333fba8308133/`
- Password: held out-of-band and in `/tmp/opencode/robin-private-auth.txt` (mode
  0600). The sops secret stores only an argon2id hash (base64) rendered into
  `/run/secrets/robin-campaign-auth.caddy` at activation; no plaintext hash in
  git. Caddy `basic_auth` is enforced inside a `route @robinPrivate` so the
  capability-path rewrite cannot run before authentication.
- Caddy landed as `ed26a36`, `fdf93de`, `61168fe` on `nixos-config` main and
  deployed (`just deploy-wait 61168fe`).
- Public root, `/manifest.webmanifest`, `/sw.js` and `/icons/*` return 200
  without a prompt; `/play/<token>/*` and `/datadirs/full/*` return 401 without
  credentials and 200 with them.

## Corpus

- Source: the owner's paid GOG install at `/home/matt/Games/robin-hood-data`,
  with its neutral 2047 overlay (`2047/data`) merged into `DATA/` before
  conversion.
- Built with the canonical shipping recipe on `minipc`: AVIF 1.4.2/libaom
  3.15.0, Opus tools 0.2 on libopus 1.6.1 (verified by the converter), full
  edition, AVIF q60, Opus, zstd window 30. No lossless remaster drop was
  available, so `--lossless-music-dir` was omitted and the converter encoded
  music from the game's own WAVs (the documented fallback).
- The converter ran without Git metadata, so its manifest's `engine_version`
  was `unknown`; that field was patched byte-for-byte to the deployed wasm
  commit `95ab7b9dbdf25725789086ea83fd6a78a4479e1b` before publication.
- Published generation:
  `datadirs/full/3fe317b9984e85f23a7f77f97c39a076866f8053f88823bb39626b62f614f381/`,
  manifest digest `e7363aea…73c`; `datadir.bin` 118,779,443 bytes,
  SHA-256 `c5519a11…d8`, chunked into five `.partN` files as the shell expects.
  Binding: `/play/<token>/campaign.json`.
- The service worker rejects every `/datadirs/full/*` URL, so the private
  corpus is never cached.

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
