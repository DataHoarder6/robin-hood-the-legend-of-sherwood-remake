# Web / native release parity

One tag must name one commit and publish both the native packages and the
browser artifact. Today the tag drives only the native path; the estate web
site is published by a separate, host-local path. Keep this doc about the
mechanism. Read current tags, releases and the served manifest with commands;
never copy them into prose.

```
git ls-remote fork 'refs/tags/v0.[45]*'
gh api repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/releases/tags/<tag> --jq '.published_at, (.assets | length)'
curl -fsS https://robin.jautis.lt/wasm/<short>/manifest.json   # Basic Auth; ~/nixos-config/hosts/oci-vm/caddy.nix:206-210
```

## 1. Root cause of the split

- **Two delivery owners.** Native releases are a GitHub Actions product:
  tag/schedule/dispatch trigger, version from `GITHUB_REF_NAME#v`, release and
  Velopack assets published by the `release` job
  ([native-release.yml#L6-L18](../../.github/workflows/native-release.yml#L6-L18),
  [#L162-L186](../../.github/workflows/native-release.yml#L162-L186),
  [#L352-L403](../../.github/workflows/native-release.yml#L352-L403)). Seats
  install the tag's commit through the machine repo's flake pin, not a channel
  ([RELEASES.md#L3-L12](../RELEASES.md#L3-L12),
  `~/nixos-config/modules/system/robin-hood.nix:13-21`). The web site is a pair
  of Caddy `file_server` roots on `oci-vm`
  (`~/nixos-config/hosts/oci-vm/caddy.nix:183-243`, DNS at
  `~/nixos-config/hosts/oci-vm/cloudflare-dns.nix:34-36`). No deploy module in
  `~/nixos-config` writes those roots: only those two files and
  `modules/system/robin-hood.nix` mention them.
- **The release checklist stops at the pin.** Steps 1–8 end with the native
  release and `just apps-sync`; no step builds or publishes a browser artifact
  ([RELEASES.md#L43-L95](../RELEASES.md#L43-L95)). The repo's "one-command
  release" script is also Cloudflare-only: its `site` is
  `https://robinhood.phiresky.xyz`, its web half calls `deploy-cloudflare.sh`,
  and it requires HEAD at local `main`
  ([release.sh#L30-L38](../../scripts/release.sh#L30-L38),
  [#L74-L80](../../scripts/release.sh#L74-L80),
  [#L170-L276](../../scripts/release.sh#L170-L276)).
- **The only web workflow is the wrong web.** `deploy-static-workers.yml` is a
  manual, `deploy=true`-gated Cloudflare Worker deploy whose environment URL is
  upstream's site. It has no jautis input and never touches the Caddy roots
  ([deploy-static-workers.yml#L14-L28](../../.github/workflows/deploy-static-workers.yml#L14-L28),
  [#L78-L85](../../.github/workflows/deploy-static-workers.yml#L78-L85),
  [#L107-L115](../../.github/workflows/deploy-static-workers.yml#L107-L115)).
- **Origin-bound bytes force a second build recipe.** The shell's inline CSP
  bakes the estate signer
  ([index.html#L8](../../wasm-www/index.html#L8)); the bridge verifier and the
  bridge builder bake upstream's two origins
  ([verify-identity-signer-bridge.mjs#L11-L12](../../wasm-www/scripts/verify-identity-signer-bridge.mjs#L11-L12),
  [#L182-L187](../../wasm-www/scripts/verify-identity-signer-bridge.mjs#L182-L187),
  [build_identity_signer.sh#L25-L31](../../scripts/build_identity_signer.sh#L25-L31)).
  `ROBIN_SIGNER_ORIGIN` makes the verifier's signer origin an input
  ([verify-cloudflare-deployment.mjs#L5-L13](../../wasm-www/scripts/verify-cloudflare-deployment.mjs#L5-L13)),
  but CI does not set it
  ([deploy-static-workers.yml#L62-L76](../../.github/workflows/deploy-static-workers.yml#L62-L76)).
  [browser-seat.md §11.2](browser-seat.md) records the three
  constraints; a Cloudflare-built artifact cannot simply be copied to jautis.
- **The web was published by commit, never by tag.** The recorded self-hosted
  publish is keyed to a commit ([browser-seat.md §11](browser-seat.md)); no tag
  reaches the web path in the checklist or the workflows. A boot gate now derives `/wasm/<host-short>/manifest.json` from the host's
  `--version` and fails on a miss
  ([browser-boot.mjs#L1-L52](../../scripts/validation/browser-boot.mjs#L1-L52),
  [browser-game.mjs#L376-L394](../../scripts/validation/browser-game.mjs#L376-L394)),
  but the tooling suite runs only its unit test, not a tag-time live check
  ([check-quality.sh#L198-L215](../../scripts/check-quality.sh#L198-L215)).
- **Tag push is not a reliable trigger.** The publisher documents that a tag
  push did not reliably reach Actions and requires a dispatch against the tag
  ref ([publish_native_release.ts#L109-L120](../../scripts/release/publish_native_release.ts#L109-L120)).
  A fix for the release-job `chmod +x version-check/robin` exists on
  `fork/main`
  ([native-release.yml#L393-L397](../../.github/workflows/native-release.yml#L393-L397))
  but is absent from the `v0.4.0` tag's workflow; replaying that tag fails.

## 2. What the estate already has

- A tag-driven, host-local web deploy script: it resolves `<v-tag>` to a
  12-hex short, builds on the build host in a detached worktree, sets the
  estate origins, stages the shell/signer/runtime, checks the Demo datadir
  SHA-256, then swaps both roots atomically and retains each previous tree
  ([deploy_web_jautis.sh#L12-L21](../../scripts/deploy_web_jautis.sh#L12-L21),
  [#L23-L75](../../scripts/deploy_web_jautis.sh#L23-L75),
  [#L84-L104](../../scripts/deploy_web_jautis.sh#L84-L104)). It lives only on
  `agent/unattended-loop-hardening`, not `fork/main`:
  `git cat-file -e fork/main:scripts/deploy_web_jautis.sh` fails. Land it
  before using it.
- Origin inputs the script already sets: `VITE_IDENTITY_SIGNER_URL`,
  `VITE_ROBIN_IDENTITY_SIGNER_ORIGIN`, `ROBIN_SIGNER_ORIGIN`, `VITE_GAME_ORIGIN`,
  `ROBINHOOD_LEADERBOARD_WEB_ORIGIN`,
  `ROBINHOOD_IDENTITY_SIGNER_ORIGIN`
  ([deploy_web_jautis.sh#L43-L52](../../scripts/deploy_web_jautis.sh#L43-L52)),
  plus the pinned bindgen/Opus toolchain overrides. `ROBIN_SIGNER_ORIGIN`
  already drives the public-build verifier
  ([verify-cloudflare-deployment.mjs#L5-L13](../../wasm-www/scripts/verify-cloudflare-deployment.mjs#L5-L13)).
- Build and verify commands to reuse: `build:site`, `verify:public`,
  `build:signer-shell`, `verify:signer`, `verify:signer-bridge`
  ([package.json#L9-L11](../../wasm-www/package.json#L9-L11),
  [#L29-L34](../../wasm-www/package.json#L29-L34)).
- The gate seeds: native tag/crate/binary agreement
  ([verify_release_tag.sh#L22-L61](../../scripts/release/verify_release_tag.sh#L22-L61)),
  immutable, hash-verified release assets
  ([publish_native_release.ts#L63-L107](../../scripts/release/publish_native_release.ts#L63-L107),
  [#L145-L179](../../scripts/release/publish_native_release.ts#L145-L179)),
  the browser boot gate above, and the canonical Opus datadir recipe behind
  D08 ([DEFECTS.md#L45](../DEFECTS.md#L45),
  [build_web_shipping_datadir.sh](../../scripts/build_web_shipping_datadir.sh)).
  Release notes are generated per tag, not hand-written
  ([RELEASES.md#L19-L31](../RELEASES.md#L19-L31)).

## 3. Best practice: one tag, one gate

Use `v0.5.0` as the worked example. Do not move an existing tag; cut a new one.

1. Land the prerequisites on `fork/main` first: the web deploy script, the
   origin-input fixes in §4, the `0.5.0` crate version, and the `v0.5.0`
   entry in `docs/releases/whats-new.json` with regenerated
   `docs/RELEASE_NOTES.md`. Run the release checklist steps 1–6
   ([RELEASES.md#L43-L83](../RELEASES.md#L43-L83)).
2. Create one annotated tag on that exact commit and read it back:
   `git tag -a v0.5.0 -m v0.5.0 <fork/main-commit> && git push fork v0.5.0`.
   The tag is the only version source: native derives the package version from
   it, the web script derives the artifact short from it, and
   `verify_release_tag.sh` proves tag = crate = binary
   ([native-release.yml#L169-L186](../../.github/workflows/native-release.yml#L169-L186),
   [deploy_web_jautis.sh#L18-L21](../../scripts/deploy_web_jautis.sh#L18-L21),
   [verify_release_tag.sh#L37-L61](../../scripts/release/verify_release_tag.sh#L37-L61)).
3. Dispatch native publication against the tag:
   `gh workflow run native-release.yml --repo DataHoarder6/robin-hood-the-legend-of-sherwood-remake --ref v0.5.0`.
   A tag push alone is not enough. Verify the published asset inventory and
   hashes, not just the job status
   ([publish_native_release.ts#L210-L266](../../scripts/release/publish_native_release.ts#L210-L266)).
4. Publish web from the same tag on `oci-vm`:
   `scripts/deploy_web_jautis.sh v0.5.0`. The build stays on the build host;
   `oci-vm` only stages and swaps static files
   ([deploy_web_jautis.sh#L6-L9](../../scripts/deploy_web_jautis.sh#L6-L9)).
5. Pin the tag's commit in the machine repo's `robin-hood` flake input, land
   it, then run `just apps-sync` on every seat
   ([RELEASES.md#L88-L91](../RELEASES.md#L88-L91)).
6. **Blocking parity gate before announcement.** Put the check in a script
   that the checklist invokes; do not leave it as prose. Require all of:
   native release exists for `v0.5.0` with verified assets; every seat's
   `--version` short equals the tag short; `https://robin.jautis.lt/wasm/<short>/manifest.json`
   reports the same `short`/`commit` and `ticketSchema == 3`
   ([browser-boot.mjs#L30-L52](../../scripts/validation/browser-boot.mjs#L30-L52));
   the served Demo datadir SHA-256 equals the manifest
   ([deploy_web_jautis.sh#L84-L93](../../scripts/deploy_web_jautis.sh#L84-L93));
   Caddy still sends the pinned headers and `frame-src`
   (`~/nixos-config/hosts/oci-vm/caddy.nix:212-242`); and a real host plus
   browser join passes `browser-game.mjs` join mode
   ([browser-game.mjs#L376-L394](../../scripts/validation/browser-game.mjs#L376-L394)).
   Reuse the boot gate's named failure rather than inventing a second message.
7. **Rollback.** Web: restore the retained trees, signer then public
   ([deploy_web_jautis.sh#L95-L104](../../scripts/deploy_web_jautis.sh#L95-L104)).
   Native: never move or delete the tag or release assets; re-pin the previous
   tag's commit in the machine repo, land, and `just apps-sync`. A fix gets a
   new tag with a higher version — regressing package versions are rejected
   ([native-release.yml#L188-L199](../../.github/workflows/native-release.yml#L188-L199),
   [package_version.ts#L58-L102](../../scripts/release/package_version.ts#L58-L102)).
   The Cloudflare Workers path is untouched by either rollback.

## 4. Risks and owner decisions

- **Land the web script.** It is branch-only; §3 step 4 depends on a file not
  on `fork/main`.
- **Make the bridge origin an input.** The script calls `build:signer-shell`
  only, but `verify:signer` requires bridge files that
  `build_identity_signer.sh` generates — and that builder and its verifier
  hardcode upstream's origins
  ([deploy_web_jautis.sh#L57-L61](../../scripts/deploy_web_jautis.sh#L57-L61),
  [verify-static-build.mjs#L189-L197](../../wasm-www/scripts/verify-static-build.mjs#L189-L197),
  [build_identity_signer.sh#L25-L59](../../scripts/build_identity_signer.sh#L25-L59),
  [verify-identity-signer-bridge.mjs#L11-L12](../../wasm-www/scripts/verify-identity-signer-bridge.mjs#L11-L12)).
  Owner chooses: parameterise the bridge for both origins (best practice) or
  keep two signer recipes and prove each in the gate.
- **Remove the origin literal.** `index.html` bakes the estate signer while
  the default verifier expects upstream's, so a Cloudflare verify run is
  inconsistent with its own build
  ([index.html#L8](../../wasm-www/index.html#L8),
  [verify-static-build.mjs#L160-L179](../../wasm-www/scripts/verify-static-build.mjs#L160-L179),
  [package.json#L9](../../wasm-www/package.json#L9)).
  One origin input should drive build, verifier and Caddy headers.
- **Credential path for the gate.** `robin.jautis.lt` is behind Basic Auth
  (`~/nixos-config/hosts/oci-vm/caddy.nix:206-210`). Owner decides where the
  parity gate runs and how it authenticates; no credential is in this repo.
- **Datadir generation.** The script reuses the live corpus and refuses a
  manifest/SHA mismatch ([deploy_web_jautis.sh#L84-L93](../../scripts/deploy_web_jautis.sh#L84-L93)).
  If v0.5.0 changes the content closure, rebuild the canonical Opus datadir
  first (D08); owner decides whether that is in scope for v0.5.0.
- **Dispatch vs true automation.** Tag push is unreliable, so "single tag"
  still means two dispatches plus a gate. Automating the write to the `oci-vm`
  roots from CI is a risk-routing decision, not a script edit; the estate's
  research record keeps CI/deploy definitions human
  ([unattended-loop-research.md#L21](unattended-loop-research.md#L21)).
