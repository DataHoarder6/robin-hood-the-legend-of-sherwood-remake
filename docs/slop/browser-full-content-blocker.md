# Full web corpus for the private browser campaign — blocker evidence (2026-11-22)

Read-only findings for the browser/Android stream; no builds were run.

## What exists

- Paid GOG install with the full game data is on this host at
  `/home/matt/Games/robin-hood-data` (`DATA/` is 779 MiB, with `Levels/`,
  `Characters/`, `Musics/`, `Interface/`, `Text/`). This is the conversion
  source the design's Q1 answer refers to. It is **not** in any repo.
- The canonical converter recipe is `scripts/build_web_shipping_datadir.sh`
  (README.md:266-283) and `wasm-www/scripts/add-full-replay-content.mjs`
  publishes the resulting immutable generation under `datadirs/full/<digest>/`.
- The browser shell already validates exactly that path shape
  (`wasm-www/src/run-replay.ts:201-231`), so no new content protocol is needed.

## What is missing

- `scripts/build_web_shipping_datadir.sh` starts with
  `cargo build --locked --release -p robin_rs --bin convert_datadir --features tools`.
  That is a full release Rust build (~1 h on the build host) and this stream's
  rules forbid full Rust builds. There is no prebuilt `convert_datadir` on this
  host, `homelab` or `minipc`.
- The converter's pinned toolchain is absent:
  `$HOME/.local/share/robin_hood/deployment-toolchain` does not exist; no
  `avifenc` 1.4.2 / `avifdec` 1.4.2, no `opusenc` reporting libopus 1.6.1.
- The lossless music WAV directory (`datadirs/music-rhmods-lossless`) is absent
  from every checkout and host searched. The converter requires it rather than
  falling back to the game's own audio.
- No already-converted Full corpus exists to mirror: the only
  `robinhood-web-content.json` files on the estate are Demo generations
  (`/var/www/robin/datadirs/demo-local`, `~/robin-combat-lab/datadirs/demo-leicester/v18`
  and `v23`). `https://robinhood.phiresky.xyz/datadirs/` is 404 and no published
  full-content URL is recorded in `docs/`.

## Consequence

The private capability path, Basic Auth gate and browser resolver are ready in
code, but there is no Full generation to serve until the converter is run. The
public Demo site is unaffected.

## Required before publication

1. Explicit permission to run the release converter build on the build host
   (lifting the stream's no-full-build rule for this one publish step), or a
   prebuilt `convert_datadir` supplied from the release pipeline.
2. The pinned AVIF/Opus toolchain (`scripts/install_pinned_avif_tools.sh` plus
   the libopus 1.6.1 `opusenc` build) on that host.
3. The lossless music WAV drop at `ROBIN_LOSSLESS_MUSIC_DIR`.

Then: run the converter with `ROBIN_WEB_CONTENT_EDITION=full`, publish the
generation with `wasm-www/scripts/add-full-replay-content.mjs`, add the Caddy
`basic_auth` capability route, and verify from a clean browser.

Credentials already generated for the private path (not committed anywhere):
token `eaab90981616b10b19c333fba8308133`, password
`sQ6YksFw3KRztKZl1L-hSLg9`; held locally at
`/tmp/opencode/robin-private-auth.txt` with mode 0600.
