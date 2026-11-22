# Automatic browser-crash triage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A browser crash on `robin.jautis.lt` files one redacted GitHub issue, and the estate's `director` agent picks it up, fixes it, verifies it in a real browser and lands the fix — while the owner just plays.

**Architecture:** Reuse `tools/crash-receiver/receiver.py` (store-first dedupe + GitHub client) by adding a `POST /api/v1/diagnostics` adapter for `DiagnosticReportV1`; deploy it on `oci-vm` behind Caddy on the private origin; label issues `crash`/`ready-for-agent` and wake the existing `director` loop through one webhook. Verification uses `scripts/validation/browser-game.mjs` with a real-browser first-map assertion instead of relying on the Rust-only gates.

**Tech Stack:** Python stdlib `http.server`, gzip, Node `node:test`, NixOS + sops-nix + Caddy, GitHub CLI/API, estate `runbg`/`land`.

**Spec:** `docs/slop/automatic-crash-triage.md`

## Global Constraints

- Fork is public: issue bodies must contain facts + a token-gated bundle link only. Never quote raw logs, `/state`, credentials, `?connect=` tickets or the site's Basic Auth password.
- Browser client sends `credentials: "omit"` and expects **202** with receipt `{schema_version:1, report_id: sha256(compressed bytes)}`; a different receipt leaves the report queued forever.
- Receiver cap is 4 MiB (`DEFAULT_MAX_BODY_BYTES`); browser will send up to 100 MiB compressed — reject early with 413 and let the browser keep the report, or split; do not silently drop.
- One receiver per fingerprint: dedupe first report → issue, repeats → comment. Fingerprint must include the engine commit (one issue per build).
- One gate and one landing at a time; all long work under `runbg`. No PRs, no force-push, commit by pathspec.
- No full Rust builds in this stream; the corpus rebuild is a separately authorised exception.

## Review Focus

1. **Private data in a public issue** — expected redacted facts + token link; test: adapter body assertions.
2. **Receipt mismatch silently dropping reports** — expected exact sha256-of-compressed receipt; test: adapter 202 body assertion.
3. **Oversize/never-decompressed body** — expected 413 or successful gunzip within cap; test: adapter size and gzip tests.
4. **Duplicate dispatch** — expected exactly one director per labelled issue; test: webhook/poller wiring review + trigger script guard (no test for GitHub side).
5. **First map "loads" while the mission already lost** — expected non-null state, no terminal debrief, empty diagnostics queue; test: `browser-game.mjs` firstMapLoads assertion.

---

### Task 1: Receiver `/api/v1/diagnostics` adapter

**Files:** Modify `tools/crash-receiver/receiver.py`, `tools/crash-receiver/test_receiver.py`.

**Interfaces:** Consumes `Service.report(raw)` and `Store.save`; produces `Service.report_diagnostic(raw) -> dict` with `fingerprint`, `issue`, `occurrences`, and `diagnostic_bundle(report, compressed) -> dict`.

- [ ] **Step 1: Write failing tests** in `tools/crash-receiver/test_receiver.py`: POST gzipped `DiagnosticReportV1` to `/api/v1/diagnostics` with bearer token; assert 202, issue body contains description + engine commit, does **not** contain raw `recent_log`, receipt `report_id == sha256(compressed_bytes)` and `schema_version == 1`; a second identical report comments; oversize (config cap 4096) returns 413; a body with a `?connect=` URL and `Authorization` header in the log is redacted in the issue body.
- [ ] **Step 2:** Run `bash scripts/check-quality.sh tooling` (or the file's unittest command) and watch them fail.
- [ ] **Step 3:** Implement: route constant `DIAGNOSTIC_PATH`, gzip decompress bounded by cap, validate `schema_version == 1`, fingerprint `sha256(normalised description + engine_commit)[..16]`, version from `engine_commit`, platform `wasm32-browser`, map backtrace/recent_log into bundle fields, redact secrets before filing, reuse `Service.report` via a bundle builder, and answer with the receipt instead of the normal answer. Labels `crash`,`ready-for-agent`.
- [ ] **Step 4:** Run the suite; expect PASS. Commit by pathspec.
- [ ] **Step 5:** Add a row to `docs/DEFECTS.md` only if a real defect was reproduced; otherwise skip.

### Task 2: Deploy receiver on `oci-vm`

**Files (nixos-config worktree):** new `hosts/oci-vm/crash-receiver.nix`; modify `hosts/oci-vm/default.nix` (import), `hosts/oci-vm/secrets.nix`, `secrets/oci-vm-secrets.yaml`, `hosts/oci-vm/caddy.nix`.

**Interfaces:** Consumes Task 1's receiver and its `CRASH_TOKEN`, `GITHUB_TOKEN`, `GITHUB_REPO` env contract.

- [ ] **Step 1:** Write a NixOS module with a `systemd.services.robin-crash-receiver` unit (loopback, `DynamicUser` or `matt`, state dir under `/var/lib/robin-crash-receiver`, `WorkingDirectory` pointing at the tracked `tools/crash-receiver` checkout, `ExecStart` the receiver with sops `EnvironmentFile`).
- [ ] **Step 2:** Add sops secrets to the oci-vm bundle via `EDITOR` + host age key; declare them in `sops.secrets`; wire the env file.
- [ ] **Step 3:** Add a Caddy `handle /api/v1/diagnostics` route that reverse-proxies to loopback **without Basic Auth** (browser uses `credentials: "omit"`) while the rest of the vhost stays gated; keep `handle /healthz` optional and isolate headers.
- [ ] **Step 4:** `just check-eval`, commit by pathspec, `land finish --wait`, read back `origin/main`, `just deploy-wait <rev>`.
- [ ] **Step 5:** Live smoke with curl: POST a fake gzipped report with the token → 202 + matching receipt; confirm one GitHub issue exists and its body is redacted. Tear down the test issue after recording the fingerprint.

### Task 3: Labels and the single dispatcher

**Files:** GitHub repo labels + webhook (external), `docs/UNATTENDED.md`, `.opencode/agents/director.md` maybe.

**Interfaces:** Consumes Task 2's issue labels `crash`, `ready-for-agent`.

- [ ] **Step 1:** Create the two labels on the fork with `gh label create` (idempotent), verify with `gh label list`.
- [ ] **Step 2:** Add exactly one trigger to the existing `hook.jautis.lt/webhooks/github-issues` route for this repo, scoped to labelled issues; if the webhook adapter cannot be scoped without a second route, use an OCI timer polling `gh issue list --label crash --label ready-for-agent` and starting one `runbg run crash-<n> -- opencode run --agent director …` with a lock. Never enable both.
- [ ] **Step 3:** Update `docs/UNATTENDED.md` so crash issues are an authoritative source and the director owns terminal states (LANDED/FILED/BLOCKED) and comments/closes the issue with the shipped commit.
- [ ] **Step 4:** Document the one-time credential/privacy decisions and the runbook in `docs/slop/automatic-crash-triage.md` (update, do not duplicate).

### Task 4: Real-browser first-map assertion

**Files:** `scripts/validation/browser-game.mjs`, `docs/TESTING.md` if a suite name is added.

**Interfaces:** Consumes Task 1's diagnostics queue name and the corpus binding; produces check `firstMapLoads`.

- [ ] **Step 1:** Strengthen the existing `firstMapLoads` check: assert non-null `robinRpc('state')`, no `FinalDebriefing`/defeat terminal, `crossOriginIsolated === true`, no `Runtime.exceptionThrown`, and zero reports in IndexedDB store `robin-diagnostics` after boot; keep the `mission started:`/`decodeFailure` assertions.
- [ ] **Step 2:** Run `node --check scripts/validation/browser-game.mjs`; run the harness once against the live private origin with `--mission H01_Lin_VL` once the rebuilt corpus is published; expect PASS only after the corpus matches the wasm commit.
- [ ] **Step 3:** Record the result in `docs/slop/browser-android-private-campaign.md`; do not claim it is a landing gate until wired into `scripts/push-gate.sh`.

### Task 5: Close the loop docs and rerun the crash

**Files:** `docs/slop/browser-android-private-campaign.md`, `docs/DEFECTS.md`.

- [ ] **Step 1:** Reproduce the original `bitcode error` with the old corpus (already done) and add a `DEFECTS.md` row with class/reproduction/check.
- [ ] **Step 2:** After the corpus is rebuilt from `95ab7b9dbdf2` and published, rerun the browser check and confirm the crash is gone; record the artifact digest.
- [ ] **Step 3:** Verify one synthetic browser report reaches GitHub as a redacted issue and that the director can be dispatched on it without double-firing.
