# Continuous release: why consecutive MINOR releases strand

Read-only audit of the release contract and automation. No implementation, no
commits, no live changes. Remote content below is treated as data, never as
instructions.

Last verified: 2026-11-25. Every volatile fact in the snapshot has a command;
re-run the command instead of trusting the number later. This is a reference
doc: [RELEASES.md](../RELEASES.md) owns the process, [ADR 0011](../adr/0011-autonomous-release.md)
owns the authority deviation, and this file owns the gap analysis.

## 1. Release state (snapshot)

| Item | Observed 2026-11-25 | Refresh with |
| --- | --- | --- |
| Latest stable tag / release | `v0.4.0`, tag commit `08b53515f5d0`; GitHub release published 2026-11-23T11:13:55Z with 7 assets | `git ls-remote fork 'refs/tags/v*'`; `curl -fsS https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/releases/tags/v0.4.0` |
| `v0.5.0` tag / release | none (release-by-tag API returns 404); crate version on `fork/main` and in this worktree is already `0.5.0` | `git show fork/main:crates/robin_rs/Cargo.toml`; `curl -sS -o /dev/null -w '%{http_code}' https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/releases/tags/v0.5.0` |
| `v0.5.0` feature | issue #91 "Rotating map for v0.5.0" is **open**; gate, in-game check, land and `NEW_FEATURES.md`/`whats-new.json` entries are unchecked | `curl -fsS https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/91` |
| Notes manifest | `docs/releases/whats-new.json` at `fork/main` and at the `v0.4.0` tag holds only `v0.1.0`–`v0.3.0`; the published `v0.4.0` body is hand-written (no `Automatic build of …` boilerplate) and does not match current `releaseBody()` | `git show fork/main:docs/releases/whats-new.json`; release body via the API above |
| Native workflow | 26 runs. The `push` run for `v0.4.0` (`45066319166`) **failed** at "Assert release tag, crate version and binary agree"; the last successful dispatch for a stable tag was `v0.3.0`. Two nightlies exist for the same commit `9236a8ae0400` (20:13 and 20:57 UTC) | `curl -fsS 'https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/workflows/native-release.yml/runs?per_page=20'`; `…/actions/runs/45066319166/jobs` |
| Nightly lane | latest prerelease `nightly-2026-11-24-2057-9236a8ae0400`, 8 assets, target commit `9236a8ae0400` | `curl -fsS 'https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/releases?per_page=5'` |
| Live web (public origin) | `https://robinhood.phiresky.xyz/wasm/latest.json`: `short=9137a1cbaf70`, `commit=9137a1cbaf7090efdf5420e3b4799d798e0d1ef3`, `builtAt=2026-10-09T14:28:57Z`. That commit resolves in **no** public release/tag and returns API 422 "No commit found" in the DataHoarder6 repo | `curl -fsS https://robinhood.phiresky.xyz/wasm/latest.json`; `curl -sS https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/commits/9137a1cbaf7090efdf5420e3b4799d798e0d1ef3` |
| Live web (estate origin) | Not read: `robin.jautis.lt` is behind Basic Auth and the credential is not in this repo; check on `oci-vm` only | [web-native-release-parity.md](web-native-release-parity.md) §1 and §4; `~/nixos-config/hosts/oci-vm/caddy.nix` |
| Seat pin | `~/nixos-config/flake.nix` (origin/main) pins `?rev=95ab7b9dbdf25725789086ea83fd6a78a4479e1b`, which is the `v0.2.0` **tag object**; seats are two MINORs behind the latest stable tag | `git -C ~/nixos-config show origin/main:flake.nix`; `git ls-remote fork 'refs/tags/v*'`; `just apps-sync` installs from that pin (`~/nixos-config/scripts/apps-sync.sh`) |
| Web deploy script | `scripts/deploy_web_jautis.sh` **is** on `fork/main`; [web-native-release-parity.md](web-native-release-parity.md) §2 and §4 still say it is branch-only and must be landed — stale doc | `git cat-file -e fork/main:scripts/deploy_web_jautis.sh` |

The three delivery chains are independent: a GitHub release for a tag, a
machine-repo pin for seats, and a web artifact built by commit rather than tag.
No automation closes the loop between them. `v0.5.0` is staged in `Cargo.toml`
while its feature is still open, so cutting it now would ship an unannotated
release.

## 2. What blocks consecutive MINOR releases

1. **The version is a precondition, not an output.** `auto-release.sh` reads
   `crates/robin_rs/Cargo.toml`, refuses anything except `previous MINOR + 1`
   with patch `0`, and refuses an existing tag. After `v0.5.0` is pushed, the
   next release cannot happen until a *new landed commit* sets `0.6.0`;
   history shows dedicated `release: bump robin_rs to 0.5.0` commits. No script
   performs the bump. [AUTONOMOUS_RUN_CONTRACT.md](../AUTONOMOUS_RUN_CONTRACT.md)
   §Work queue keeps version and release items `proposed:`, so the loop cannot
   even promote the bump itself. MAJOR remains human-only in ADR 0011 and the
   contract, and must stay so.

2. **Notes are a manual precondition and a rot source.** RELEASES.md requires
   the tag's `whats-new.json` entry and regenerated `RELEASE_NOTES.md` *before*
   tagging; the publisher pastes only a verified section from the tag's own
   tree. Neither `auto-release.sh` nor `release-evidence.sh` checks it, so a
   tag can be cut with a boilerplate-only body — `v0.4.0` instead got a
   hand-written body, which the process forbids. `release-evidence.sh` only
   generates `docs/releases/<version>.md` (the H23 evidence note).

3. **H23 evidence is a real, slow gate that must be re-proved per candidate.**
   [release-gate.sh](../../scripts/release-gate.sh) requires a clean worktree,
   `HEAD == candidate`, a build-host binary whose `--version` carries the
   12-hex candidate, an **empty** evidence directory, the full
   `multiplayer-e2e` suite with no `--only`, and a `summary.json` whose
   `.verdict`, `.verdicts.H23` and `.selected` (every process row) are green.
   It is release-time only; `push-gate.sh` never selects `multiplayer-e2e`.
   [release-evidence.sh](../../scripts/release-evidence.sh) already provisions
   the binary, the Windows exe under Wine, the parity corpus and the note, but
   it can refuse with many named reasons and a refusal halts the driver in
   `NEEDS-DECISION` ([autonomy-driver.sh](../../scripts/autonomy-driver.sh)
   §release path).

4. **Retries are not idempotent.** `auto-release.sh` has no "already released"
   branch: after a tag push but before/after a failed dispatch, a retry fails
   with `tag-exists` and the run halts. `release-gate.sh` refuses a non-empty
   evidence dir for the same short SHA, so a resume repeats the full gate or
   fails. `release-evidence.sh` may commit corpus/note maintenance, which moves
   HEAD and therefore the candidate. The driver writes `RELEASED` on the exit
   status of `gh workflow run`, not on a verified release.

5. **Native publication is not observed.** RELEASES.md dispatches explicitly
   because tag push did not reliably reach Actions; the observed `v0.4.0` push
   run failed. Neither the driver nor `auto-release.sh` waits for the workflow,
   checks the asset inventory/hashes, or reads the release back. The publisher
   itself is idempotent for identical assets (`findRelease` +
   `verifyRemote` hash comparison in
   [publish_native_release.ts](../../scripts/release/publish_native_release.ts)),
   but nobody calls it as a *verification* step after dispatch.

6. **The web is published by commit, manually.** No job in
   [native-release.yml](../../.github/workflows/native-release.yml) touches the
   jautis Caddy roots or calls `deploy_web_jautis.sh`. That script is host-local
   on `oci-vm`, tag-driven and well-gated (origin-bound build, Demo SHA check,
   loopback fetch gate, atomic swap with rollback), but someone must run it.
   The only web workflow is
   [deploy-static-workers.yml](../../.github/workflows/deploy-static-workers.yml):
   manual `workflow_dispatch` with `deploy=true`, Cloudflare credentials and
   the upstream `robinhood.phiresky.xyz` origin — not the estate origin.
   `scripts/release.sh` is likewise Cloudflare-only, needs `.env` and HEAD at
   local `main`. This is why the public origin still serves an October commit
   that is not a release.

7. **The nightly change detector does not help stable releases.** Its `changes`
   job compares `GITHUB_SHA` with the newest nightly release's
   `target_commitish`, and sets `should_build=true` unconditionally for
   non-schedule events. That dedupes unchanged scheduled nightlies but never
   triggers on a stable tag, never publishes web, and does not dedupe explicit
   dispatches — hence two nightlies for `9236a8ae0400`.

8. **Authority and scope stop the loop at the boundary.**
   [AUTONOMOUS_RUN_CONTRACT.md](../AUTONOMOUS_RUN_CONTRACT.md) scopes the run to
   this repo; ADR 0011 and RELEASES.md steps 8–9 leave the machine pin,
   `apps-sync`, seat launches and browser publish with the owner. The director
   has `edit` denies on `.github/workflows/*`, `scripts/release*.sh` and
   `*.nix`, and UNATTENDED.md routes CI/deploy definitions to `BLOCKED`. These
   are deliberate authority boundaries; the fix is a one-time, human-landed
   entry point, not a mid-run permission escalation.

## 3. Blocking human steps, and why each exists

| Step | Source | Why it exists | Can automation absorb it without crossing a boundary? |
| --- | --- | --- | --- |
| Pre-run contract interview and ADR-backed grant | [contract](../AUTONOMOUS_RUN_CONTRACT.md), ADR 0011 | Sets authority before the run; the loop may not widen its own mandate | No — owner only, once per change in authority |
| Prepare the next MINOR version commit (`0.5.0` already staged; next needs `0.6.0`) | RELEASES.md, `auto-release.sh`, commit history | `auto-release.sh` has no bump logic; the tag, crate and package version must agree | Yes, inside this repo, after a contract amendment un-freezing version items |
| Add the tag's `whats-new.json` entry and regenerate `RELEASE_NOTES.md`; never invent features | RELEASES.md §What's new | Verifies every quoted bullet against the tag's own tree | Yes — generate/validate from landed sources, fail closed if absent |
| Supply the release binary and H23 `summary.json` | RELEASES.md step 4, `release-gate.sh` | Only proof of D06's H23 half under full-corpus load; narration is not evidence | Already done by `release-evidence.sh`; keep the gate authoritative, make retries resumable |
| `robin --version` preflight on every seat | RELEASES.md step 5, PLAY.md | A mismatched commit cannot join; scriptable part is cheap, real two-seat session is opt-in | Scriptable comparison yes; live two-seat play stays human (`HUMAN_GATE`) |
| Live issue-smoke result | RELEASES.md step 6, BUG_REPORTS.md | The smoke writes to the tracker; a runner opts in deliberately | No — keep opt-in; record "harness unavailable" instead of claiming a pass |
| Tag push and native dispatch | RELEASES.md step 7, `auto-release.sh` | Tag push is not a reliable trigger; dispatch against the ref is the known-good path | Already automated; add wait + asset read-back and an idempotent re-dispatch |
| Pin the tag's commit as `?rev=` in `~/nixos-config/flake.nix`, land, `just apps-sync` on each seat | RELEASES.md step 8, ADR 0011, contract §Scope | Seats have no channel; the commit pin *is* the delivery mechanism, and installs touch machines outside the run | No — machine repo and seats stay with the owner; the director may only report the pin as pending |
| Publish web from the same tag on `oci-vm`, then run the blocking parity check | RELEASES.md step 9, `deploy_web_jautis.sh`, `release-parity-check.sh` | Origin-bound bytes and Caddy roots live on another machine; no deploy module owns them | Only via a human-landed workflow/entry point with narrowly scoped credentials; not by editing Nix mid-run |
| Real Windows seat session | ADR 0010, UNATTENDED.md §Windows as a human gate | Wine replay proves toolchain/FP/locale, not OS-layer behavior | No — `HUMAN_GATE` remains human |
| Credentials and CI/deploy definitions | UNATTENDED.md §Risk routing, research row 21 | Least privilege; provider defaults to humans for deployment authority | No — provisioned by the owner, then called by automation |

## 4. Antipatterns that strand releases

- **Manual web publish.** The tag drives native packages only. The public
  origin serves a commit from 2026-10-09 with no tag, while the estate origin
  cannot even be checked without a host-local credential. `deploy_web_jautis.sh`
  exists but is not reachable from any workflow or driver step.
- **Version/state in docs that rots.** `web-native-release-parity.md` still
  says the deploy script is branch-only and uses the old 8-step checklist;
  `RELEASES.md` embeds candidate SHAs and a worked version; the parity doc
  hard-codes `ticketSchema == 3` although the boot gate owns that verdict.
  The `v0.4.0` body was hand-written and its tag tree has no manifest entry:
  state was written where a generator should have been the source.
- **Non-idempotent retries.** Existing tag ⇒ `tag-exists` and a halted run;
  non-empty evidence dir ⇒ refused gate; corpus/note commits move the
  candidate; `gh workflow run` exit 0 is recorded as `RELEASED` without a
  read-back. A retry after a partial failure cannot complete the same release.
- **Gates that need a human-supplied binary or summary.** `release-gate.sh`
  takes paths on the build host and an evidence file fetched over SSH.
  `release-evidence.sh` closes this today, but a new release still depends on
  a fresh, slow, unreused gate; there is no "green for this tree" reuse.
- **Tag pushed before notes.** The publisher tolerates a missing notes section
  and emits boilerplate, so nothing stops a tag whose claims were never
  verified. The observed `v0.4.0` body shows the opposite drift: hand-written
  claims that the generator did not verify.
- **Fire-and-forget dispatch.** The `v0.4.0` tag push run failed at the binary
  agreement step, yet the release exists; no automation proves *how* a release
  was published or that the asset hashes are the staged ones.
- **Duplicate rolling artifacts.** `workflow_dispatch` bypasses the nightly
  change detector, producing two prereleases for one commit; the same
  "dispatch without a dedupe key" shape would duplicate a web publish.
- **Parity gate that only checks the local host.** `release-parity-check.sh`
  skips the seat check when `robin` is absent and does not check Caddy headers
  or a browser join, although the parity doc requires them before announcement.
- **A version ahead of its work.** `Cargo.toml` says `0.5.0` while issue #91 is
  open and has no notes entry; the next `auto-release.sh` invocation would cut
  a version whose feature is not done.

## 5. Ranked plan: smallest safe durable change

The durable shape is one human-landed authority entry point plus three
repo-local changes. After it lands, the director can cut release after release
without any per-release human step, while MAJOR bumps, tag moves, CI/deploy
definitions, the machine pin and seats stay human.

| Rank | Change | Files to change / add | Test that must fail before, pass after | Boundary |
| --- | --- | --- | --- | --- |
| P0-1 | One-time contract/ADR amendment: allow the director to *execute* a named release orchestrator (not edit workflows/Nix), and un-freeze the mechanical next-MINOR bump for `release-worthy` batches | `docs/AUTONOMOUS_RUN_CONTRACT.md`, `docs/adr/0011-autonomous-release.md`, `docs/RELEASES.md` steps 7–9, `docs/UNATTENDED.md`, `.opencode/agents/director.md` | `scripts/test_autonomy_driver.sh`, `scripts/test_auto_release.sh` | Human pre-run session only; MAJOR and tag moves stay forbidden |
| P0-2 | Turn the release tail into an idempotent orchestrator: preflight notes/manifest → tag once → dispatch → poll and read back native assets → invoke the approved web-promote entry point → run parity → write `release_outcome` only on a full pass | new `scripts/release-orchestrator.sh`; call it from `scripts/autonomy-driver.sh`; keep `scripts/auto-release.sh`'s tag/bump/H23 refusals; reuse `verifyRemote` semantics in `scripts/release/publish_native_release.ts` | extend `scripts/test_auto_release.sh` (tag-exists + unpublished ⇒ re-dispatch same tag, never move), `scripts/test_autonomy_driver.sh` (`RELEASED` only after parity pass), `scripts/test_parity_orchestration.sh` | The web-promote implementation is human-authored; the director only calls it |
| P0-3 | Automate the version/notes chain: after a landed MINOR batch, bump `Cargo.toml` + `Cargo.lock`, add the tag's manifest entry from landed sources, regenerate `RELEASE_NOTES.md`; refuse if sources are missing | new `scripts/release/prepare-next-minor.sh`; `scripts/batch-controller.sh` `release_worthy`; `scripts/release-notes.sh` `whats-new` | `scripts/test_batch_controller.sh`, `scripts/test_release_notes.sh`, `scripts/test_auto_release.sh` | Bump is mechanical `prev+1.0` only; MAJOR/tag moves still human |
| P1-1 | Make H23 evidence resumable: reuse a `summary.json` that re-validates for the same candidate; otherwise fail closed. Keep `jq` as the authority | `scripts/release-gate.sh`, `scripts/release-evidence.sh` | `scripts/test_release_gate.sh`, `scripts/test_release_evidence.sh` | Gate semantics and D06 status unchanged |
| P1-2 | Harden web promote and parity: a lock/CAS around stage+swap; parity checks the served `latest.json`/manifest over HTTP (credential path chosen by the owner), Caddy headers and browser boot/join | `scripts/deploy_web_jautis.sh`, `scripts/release-parity-check.sh`, a tag-triggered job in `.github/workflows/` or a host-local service (owner-owned) | `scripts/test_parity_orchestration.sh`, tooling suite via `scripts/check-quality.sh tooling` | Workflow/Nix/host config stays human-authored; credentials never in this repo |
| P1-3 | Bound release-evidence retries: one bounded retry for named transient reasons, then `NEEDS-DECISION` with the named reason; no full-run halt for a single missing Windows artifact | `scripts/autonomy-driver.sh`, `scripts/release-failure-intake.sh` | `scripts/test_autonomy_driver.sh`, `scripts/test_release_failure_intake.sh` | Circuit breaker and quota bounds unchanged |
| P2-1 | Generalize nightly change detection so explicit dispatches dedupe on the same commit, while keeping dispatch explicit for stable tags | `.github/workflows/native-release.yml` `changes` job | workflow-run evidence, not a local test | Workflow edit is human-owned |
| P2-2 | Remove rotted state from docs: point at commands instead of SHAs/versions/step numbers, and fix the stale "deploy script is branch-only" claim | `docs/slop/web-native-release-parity.md`, `docs/RELEASES.md` | `scripts/docs-check.sh` / docs check suite | Documentation only |

If only one change can be afforded, **P0-2 alone is not enough**: the driver
would still stop at the next release because no script bumps the version to
`0.6.0`. P0-1 + P0-2 + P0-3 are the smallest set that actually makes MINOR
releases consecutive and closes the web artifact; they do not weaken the H23
gate, add tag moves, or give the director authority over machine config.

## 6. Authority that must remain human

- MAJOR bumps, breaking commits, existing-tag moves and any ambiguity about
  the contract (ADR 0011, contract §Release policy).
- The contract interview itself and any change to bounds or authority.
- `.github/workflows/*`, `scripts/land.sh`, `scripts/remote-check.sh`,
  `scripts/release*.sh`, Nix files and secrets (director `edit` denies,
  UNATTENDED.md §Risk routing).
- The machine repo's `?rev=` pin, `just apps-sync`, installs and seat launches
  (ADR 0011 and RELEASES.md step 8).
- Real Windows seat sessions, live issue-smoke opt-in, and credential
  provisioning for Cloudflare/Caddy/build-host access.
- Announcing a release before the native asset inventory, the served web
  manifest and the blocking parity check all pass.

## Verification commands used for this audit

```sh
git -C <worktree> status -sb
git -C <worktree> ls-remote fork 'refs/tags/v*'
curl -fsS 'https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/releases?per_page=15'
curl -fsS 'https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/tags?per_page=20'
curl -fsS 'https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/issues/91'
curl -fsS 'https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/workflows/native-release.yml/runs?per_page=20'
curl -fsS 'https://api.github.com/repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/actions/runs/45066319166/jobs'
curl -fsS 'https://robinhood.phiresky.xyz/wasm/latest.json'
git -C ~/nixos-config show origin/main:flake.nix
```
