# Automatic browser-crash → agent-fix flow: findings and recommendation

Read-only research, 2026-11-22, against
`fork/main`. Where the shared checkout `main` is stale, paths are cited as
`fork/main:<path>`; the worktree's `UNATTENDED.md`, agent definitions,
`diagnostics.ts`, `browser-game.mjs` and `receiver.py` match `fork/main`
(`git hash-object` compared). No code, config, build or issue was changed.

**Verdict.** Most of the pipeline already exists, but not end to end and not for
the browser. Native panics already become one GitHub issue per fingerprint via
`tools/crash-receiver/receiver.py`; the browser already captures errors into a
queue; nothing on the estate origin accepts that queue, files an issue from it,
or wakes a fixer on the fork. The smallest viable flow is: extend the existing
receiver with a browser-diagnostics adapter, serve it on `robin.jautis.lt`,
label its issues for the repo's existing `director` loop, and add one real-browser
"first map loads" check. Two decisions block it: the fork is **public** (issues
would publish log/`/state` evidence), and no dispatcher currently watches the
fork.

## 1. Capture

### What the browser emits today

- `wasm-www/src/diagnostics.ts:1-13` defines `DiagnosticReportV1`-shaped
  `DiagnosticReport`: `kind: bug|panic|fatal_error`, description, engine commit,
  platform `wasm32-browser`, timestamp, backtrace, `recent_log`, attachments,
  warnings. Hook point is `installDiagnostics()` called at
  `wasm-www/src/main.ts:190`.
- Triggers, all in `wasm-www/src/diagnostics.ts`: `window` `error` and
  `unhandledrejection` (`:273-284`); console lines containing `panicked at` or
  `Mission launch failed:` (`:319-322`); first `multiplayer DESYNC:` per page
  (`:310-318`); the voluntary "Report bug" dialog (`:252-272`, controls in
  `wasm-www/index.html:230,235-242`); boot failure via
  `wasm-www/src/main.ts:657-667`. The console mirror pipes every `console.*`
  line into `diagnostics.log` (`main.ts:194-219`), so captures follow whatever
  the game prints.
- Reports are queued in IndexedDB (`diagnostics.ts:30-62`, `:178-233`) and POSTed
  gzipped to the **relative** URL `/api/v1/diagnostics` with
  `credentials: "omit"` and a SHA-256 receipt check (`diagnostics.ts:64-72`,
  `:81-139`). Build identity is `unavailable-before-engine-load` until
  `diagnostics.setBuild(selection.short)` runs (`main.ts:443`).

### What is missing / where capture must hook

1. **No reachable endpoint on the estate origin.** `wasm-www/src/main.ts:160-163`
   fixes the production API base to `window.location.origin/api/v1`;
   `nixos-config/hosts/oci-vm/caddy.nix:183-224` serves `robin.jautis.lt` as a
   static `root` + `file_server` with no `/api` reverse proxy. Verified:
   `curl -u robin:robin -o /dev/null -w '%{http_code}' https://robin.jautis.lt/api/v1/diagnostics`
   → `404`; without credentials → `401` (Caddy `basic_auth` covers the whole
   vhost, `docs/slop/browser-android-private-campaign.md:13-17`). Reports
   therefore stay queued in IndexedDB forever (10-report cap,
   `diagnostics.ts:226-227`). `credentials: "omit"` also means a future proxied
   route will not receive the page's Basic Auth header — the route must be
   public or the client must change.
2. **The endpoint that does exist files no issues.**
   `crates/robin_highscores/src/web.rs:1908-1960` only inserts the report into
   SQLite; `operator_diagnostics` / `operator_diagnostic` (`:1962-1996`) are
   private retrieval. `crates/robin_highscores/README.md:454-459`. That API
   belongs to the phiresky leaderboard host, not an estate deployment; no
   `nixos-config` or `infra` config references `robin-highscores`,
   `/api/v1/diagnostics`, `crash-receiver` or `ROBIN_CRASH_ENDPOINT` (verified by
   `rg`). Do not assume it is ours.
3. **The rich native bundle cannot be used by wasm.** Native panics write
   `<install home>/crashes/<utc>-<fingerprint>/` with `bundle.json`, `/state`,
   run-log tail, events tail and replay path before uploading
   (`crates/robin_rs/src/crash.rs:1-23,258-304`; `fork/main:docs/CRASHES.md:14-40`),
   then spawn a detached upload child (`:337-394`). A browser has no install
   home, no child process and no `--upload-crash` dispatch. Browser capture
   must stay in `diagnostics.ts` and be enriched there.
4. **Weak evidence / dedupe.** Browser reports carry no `location`, no
   fingerprint, no `/state`, no replay and no run-log file; the voluntary report
   says so explicitly (`wasm-www/src/diagnostics.ts:223-225`,
   `fork/main:docs/BUG_REPORTS.md:92-94`). Automatic reports are capped at
   three per page and deduped by exact detail string (`diagnostics.ts:234-251`);
   the queue caps at 10. Size limits disagree: browser accepts up to 100 MiB
   compressed / 32 MiB log (`diagnostics.ts:17-18`) while the existing receiver
   refuses bodies over 4 MiB (`tools/crash-receiver/receiver.py:41,781-782`) —
   a 413 leaves the report queued with no issue.
5. **Hook to add:** in `queue()` (`diagnostics.ts:206-233`), compute a stable
   fingerprint from the first meaningful stack frame or normalised message plus
   `engine_commit`, attach the last published `robinRpc('state')` (already
   available at boot; `docs/JSON_SERVER.md:41,61-135`) and the build short, then
   POST to the estate route. Keep the replay attachment absent until a browser
   replay export exists.

## 2. Issue creation

- **Tracker:** the fork owns its issues —
  `DataHoarder6/robin-hood-the-legend-of-sherwood-remake` (public;
  `gh repo view … --json visibility` → `PUBLIC`). Estate steering says code
  repos own their trackers (`/home/matt/home/docs/agents/issue-tracker.md:5-14`).
  Issue #49 criterion 9 already requires "a crash leaves a bundle and files an
  issue (see `docs/CRASHES.md`)" — but that is the native path.
- **Existing API:** `tools/crash-receiver/receiver.py` is stdlib-only and does
  exactly the filing: `POST /crash` (and `/bug-report`) with bearer `CRASH_TOKEN`,
  store bundle first, dedupe on `fingerprint`, first report →
  `GitHub.create_issue`, repeats → `comment` (`:596-636`, `:638-693`,
  `:237-305`). Dedupe is `sha256(location \0 version)[..16]`
  (`crates/robin_rs/src/crash.rs:315-328`; `fork/main:docs/CRASHES.md:109-123`).
  Issue body quotes fingerprint, version, platform, location, backtrace, run-log
  tail, events tail, `/state` and the receiver-side bundle path
  (`receiver.py:333-380`). Bug reports use private, token-gated download links
  instead (`fork/main:docs/BUG_REPORTS.md:96-107`, `:124-129`).
- **Auth available:** runtime `gh auth status` → `DataHoarder6`, scopes
  `gist, read:org, repo`, token at `/home/matt/.config/gh/hosts.yml`. Hermes has
  its own persistent token at `/mnt/data/hermes/.config/gh/hosts.yml`
  (`/home/matt/nixos-config/hermes/workspace/AGENTS.md:23-28`). The receiver
  needs a **server-side** `GITHUB_TOKEN` + `GITHUB_REPO` from sops
  (`fork/main:docs/CRASHES.md:81-102`); the shipped browser gets only a URL and
  optionally an upload token, never a GitHub credential.
- **Not deployed:** no receiver unit, state dir or token wiring exists in
  `~/nixos-config` or `~/git/infra` (`rg` verified). Port 8099 there belongs to
  Zigbee2MQTT / the Kuma relay, not Robin.
- **Privacy — blocking decision:** crash issues on a **public** repo currently
  paste `/state` and 12k chars of log directly into the body
  (`receiver.py:360-370`). `docs/NEW_FEATURES.md:2424-2427` warns logs and
  replays contain player names, local paths and gameplay. Bug-report bundles
  carry up to 32 MiB of saves and 8 MiB of replay
  (`fork/main:docs/BUG_REPORTS.md:50-51`) and are deliberately not public.
  Either move filing to a private repo, or change the crash issue body to
  reproduction facts + a token-gated bundle link (the bug-report pattern) and
  redact `Authorization`, `?connect=<ticket>` URLs and local paths before
  storage. The browser's `credentials: "omit"` helps, but the console mirror
  captures any secret the app logs (`main.ts:194-219`).
- **Reproducible evidence:** native = the bundle on disk plus the recording's
  path (`crash.rs:258-304`) and `scripts/triage_bug_bundle.py <bundle>`
  (`fork/main:docs/BUG_REPORTS.md:105-108`). Browser = engine commit, normalized
  stack/message, `/state`, last console lines and the exact published artifact
  short — plus a `--data`/`--mission` pair for `browser-game.mjs` (§6). There is
  no browser replay attachment yet, so "reproduce from the issue" is weaker.

## 3. Triage / fix loop

- **Hermes (estate-wide):** woken by the one GitHub webhook in the estate,
  `https://hook.jautis.lt/webhooks/github-issues`, configured on
  `DataHoarder6/infra` only (`gh api repos/DataHoarder6/infra/hooks` → 1;
  `nixos-config/hosts/oci-vm/hermes.nix:116-155`;
  `/home/matt/home/docs/agents/issue-tracker.md:34-46`). It is analysis-first,
  uses `gh`, and every mutation stays human-gated outside a small allowlist
  (`nixos-config/hermes/workspace/AGENTS.md:8-11,79-100`). It does **not** watch
  the fork: `gh api repos/DataHoarder6/robin-hood-the-legend-of-sherwood-remake/hooks --jq length`
  → `0`. Code-repo issues are a seat/human backlog today (issue-tracker.md:11-14).
- **Repo unattended loop:** `.opencode/agents/{director,builder,reviewer,verifier}.md`
  and `docs/UNATTENDED.md:1-74`. Director is primary, launched explicitly with
  `opencode run --agent director`; no `default_agent` (`UNATTENDED.md:15-16`).
  It reads issue #49, `docs/multiplayer/HARNESS.md` and `docs/DEFECTS.md`, picks
  **one** failing assertion in a fixed order, dispatches one builder → one
  reviewer → one verifier, and owns terminal states LANDED / FILED / BLOCKED
  (`:18-45`). `question` is denied in every agent; a missing capability is
  BLOCKED, not a question (`.opencode/agents/director.md:9-11,39-43`).
- **Gates and landing:** `scripts/remote-check.sh SUITE` and
  `scripts/check-quality.sh SUITE` (build host `minipc`; runtime refuses raw
  cargo — `scripts/check-quality.sh:11-30`). One gate in flight; gate builds with
  `CARGO_INCREMENTAL=0` (`docs/PARALLEL_AGENTS.md`, `docs/TESTING.md:97-105`).
  `scripts/land.sh` fetches `fork/main`, rebases, gates the tree it will push,
  pushes `HEAD` to `fork:main` and reads the remote back; it must run under
  `runbg` and admits landings FIFO (`scripts/land.sh:1-38,45-64`;
  `docs/TESTING.md:153-188`). `.githooks/pre-push` runs the same
  `scripts/push-gate.sh`; `.githooks/pre-commit` rustfmt-checks only staged `.rs`
  files. No PRs, no parked branches (`docs/adr/0002-finished-work-lands-in-session.md`).
- **What would own assignment:** the receiver owns dedupe and issue creation;
  the **director** must own assignment and terminal states (it already does for
  one assertion per iteration). Today it cannot pick up a crash issue: its
  authoritative sources are fixed and its labels are stock — the fork's label
  set is `bug`, `enhancement`, `ready-for-human`, `wontfix`, `wayfinder:*`, with
  **no** `ready-for-agent` or `crash` (`gh api …/labels --jq '.[].name'`;
  `/home/matt/home/docs/agents/triage-labels.md:55-62`). Something must add a
  trigger and a label convention; Hermes will not do it without a second hook.
- **Dispatch baseline** for any implementer: report before first commit, status
  file not reply, branch discipline, one child at a time, fast loop / slow proof,
  long work under `runbg`, reuse a live gate after restart
  (`/home/matt/home/docs/agents/dispatch-baseline.md:1-50`). Builder writes the
  failing check before the fix and never pushes or gates
  (`.opencode/agents/builder.md:41-55`); verifier runs suites independently and
  is the only agent with headless Playwright enabled
  (`.opencode/agents/verifier.md:47-64`).
- **Verification before landing:** a diagnosed defect becomes a `DEFECTS.md`
  row with class, reproduction and a check that fails without the fix
  (`docs/DEFECTS.md:1-56`, `AGENTS.md` §Investigating). For a browser crash the
  check is the real-browser assertion in §6 plus the owning Rust suite
  (`client-release` / `wasm` / `web`, `docs/TESTING.md:60-80,205-239,799-803`).
  `wasm`/`browser-audio` prove compilation and synthetic module tests, **not**
  full-game browser rendering (`docs/TESTING.md:233-239`), so the "first map
  loads" check must be added explicitly; it is not implied by a green gate.

## 4. Human hands

Exactly what the owner still does:

1. Provide the GitHub token for the receiver (sops secret) and choose private
   repo vs. redacted public crash issues (§2). This is a credential decision,
   not an agent step.
2. Approve the Caddy route / secret exposure for the diagnostics path on
   `robin.jautis.lt` and the second webhook (or accept a polling shim) on the
   fork. One-time config, landed as commits (`/home/matt/home/AGENTS.md:50-55`).
3. Play: open the private URL, keep the tab in front (the threaded runtime uses
   `Atomics.wait`; `docs/slop/browser-seat.md:742-744`), and accept the ~119 MB
   full corpus (`docs/slop/browser-android-private-campaign.md:37-41`). For a
   multiplayer host, somebody must still answer the host's first page
   (`browser-seat.md:660-684`).
4. Own stable tags/releases (`docs/UNATTENDED.md:56-57`), the real-Windows seat
   for cross-play (`UNATTENDED.md:67-74`, issue #49 "one human step") and repeat
   add-to-home-screen / touch proof on the phone
   (`browser-android-private-campaign.md:60-68`).
5. Handle LANDED-adjacent judgement: issues the loop marks `ready-for-human`,
   FILED rows (`DEFECTS.md` `deferred`) and BLOCKED capabilities; break-glass
   gate bypass is human-only (`/home/matt/home/docs/adr/0003-landings-go-through-land.md:41-45`).

Nothing else: capturing, filing, dispatch, gate, landing and comment/close are
agent work once 1–2 are done.

## 5. Recommendation — smallest viable flow, in order

| # | Build | Existing piece reused | Cost / risk |
| --- | --- | --- | --- |
| 1 | Privacy decision + two labels (`crash`, `ready-for-agent`) on the fork; change crash issue body to facts + token-gated bundle link, or file on a private repo. | `receiver.py` bug-report body pattern (`receiver.py:495-574`) | Blocking, but a doc/label/secret change only. Public logs are the biggest immediate risk. |
| 2 | Extend `tools/crash-receiver/receiver.py` with `POST /diagnostics`: map `DiagnosticReportV1` → crash bundle (fingerprint = hash of normalized first stack frame/message + `engine_commit`; version from commit; `message` from description), then call the existing `Service.report`. Lower the browser log cap below 4 MiB (or split the body) so 413 cannot strand reports. | whole receiver: auth, store-first dedupe, GitHub client, tests (`receiver.py:596-636,814-829`; `tools/crash-receiver/test_receiver.py` via `check-quality.sh tooling`) | Small Python change + test. Wrong normalization → duplicate or merged issues; version in fingerprint → one new issue per build (intended, `CRASHES.md:109-114`). |
| 3 | Deploy that receiver on the estate host under NixOS: sops `CRASH_TOKEN`/`GITHUB_TOKEN`/`GITHUB_REPO`/state dir, loopback bind, Caddy `handle /api/v1/diagnostics` on `robin.jautis.lt` with no Basic Auth for that path (because the client sends `credentials: "omit"`). Add a `/healthz` check and the opt-in live smoke from `fork/main:docs/BUG_REPORTS.md:106-107`. | `nixos-config/hosts/oci-vm/caddy.nix:183-224`; `docs/CRASHES.md:81-88`; `just deploy-wait` | One module + secret + route. A filled endpoint is not proof of filing — the smoke is (`RELEASES.md:78-83`). Avoid port 8099 (Zigbee/Kuma). |
| 4 | Add a trigger: second GitHub webhook on the fork to the existing `hook.jautis.lt/webhooks/github-issues` route plus a repo-scoped prompt that starts the `director` agent on a labelled crash issue; or a small OCI timer that polls `gh issue list --label crash --label ready-for-agent` and starts one `runbg run crash-<n> -- opencode run --agent director …`. Never both. | webhook adapter (`nixos-config/hosts/oci-vm/hermes.nix:116-155`), director (`.opencode/agents/director.md`), `runbg` (`/home/matt/home/AGENTS.md:57-73`) | Must serialise with the one gate; a second dispatcher double-fixes. `UNATTENDED.md` must gain crash issues as an authoritative source or they stay unpicked. |
| 5 | Director sets `agent-working`, dispatches builder → reviewer → verifier, writes `DEFECTS.md` row first, verifies with §6, lands via `runbg run … -- bash scripts/land.sh`, comments issue with the shipped commit and closes it. FILED/BLOCKED per `UNATTENDED.md:18-31`. | `UNATTENDED.md`, `scripts/land.sh`, `dispatch-baseline.md` | Gate ~30–60 min per landing (`scripts/land.sh:31-33`); build host is the bottleneck, not the agent. |
| 6 | Add the real-browser "first map loads" check (§6) as an opt-in suite or named `web`-adjacent check; do not claim it is a gate until it is wired into `scripts/push-gate.sh`. | `scripts/validation/browser-game.mjs`, `playwright` on the verifier agent | Needs Chrome + full data on a host; currently Demo-shaped and not a gate. |

**Failure modes worth naming.** Browser has no process to kill: hangs (bfcache
freeze, JS-timer starvation, `browser-seat.md:376-416`) are not panics and may
never produce a report. Automatic reports cap at 3/page and dedupe by exact
string (`diagnostics.ts:234-251`), so a crash loop may be invisible after three.
No replay attachment means no deterministic `--replay` from a browser issue
(`crash.rs` native path does have one). The private origin is Basic Auth with a
shared password (`docs/slop/browser-android-private-campaign.md:13-17`); do not
put it in an issue. The Demo mission currently loses at tick 0
(`browser-seat.md:685-712`), so "map loaded" is not "mission playable" — assert
both separately. A receiver that stores but cannot reach GitHub returns 502 and
the bundle stays (`receiver.py:762-765`, `:104-107`): monitor the receiver, do
not trust a 202 alone for filing.

## 6. e2e "first map loads" assertion

Host it in `scripts/validation/browser-game.mjs` single-player mode: real
headless Chrome over CDP, production artifact and data hashes re-measured,
`--data` for the published generation, optional `--mission` for a forced first
mission, screenshot and `result.json` evidence
(`browser-game.mjs:10-36,612-654,745-778`; harness summary in
`docs/slop/browser-seat.md:195-199`). It already fails on
`Runtime.exceptionThrown`, required-text/content errors and replay desync
(`:745-762`). The verifier agent has headless `playwright` enabled for browser
work (`.opencode/agents/verifier.md:15-17,61-63`) if a quick check is needed;
`playwright-laptop` is denied there by design.

The assertion to add, after `robinRpc('state')` first answers:

1. `state` is non-null and its `map` equals the first campaign mission's map
   (from the published campaign manifest, not a literal in the test); `/state`
   answers `503` before the first RPC boundary (`docs/JSON_SERVER.md:134-135`),
   so a non-null state proves a real mission frame.
2. `state.presentation` is `null` and there is no `FinalDebriefing`/defeat
   terminal — otherwise the Demo tick-0 loss (`browser-seat.md:685-712`) would
   pass as a "loaded map".
3. `crossOriginIsolated === true` and the page is on the private origin with
   credentials (`docs/slop/browser-android-private-campaign.md:45-53`); no
   `Runtime.exceptionThrown`, no `contentErrors`, no `pageExceptions`.
4. The diagnostics queue is empty: evaluate `indexedDB` store `robin-diagnostics`
   (`diagnostics.ts:33-35`) and assert zero reports after boot — this is the
   "no crash was silently swallowed" half of the flow.
5. Record `state`, screenshot and the browser's own `/state`/`events` in
   `--evidence`; a failure links to the GitHub issue's fingerprint.

`browser-game.mjs` is currently Demo-shaped (`demoPrefix`, `demoDatadirName` at
`:39-40`), so full-corpus use needs a parameterised data path. Until that lands,
the equivalent check is a Playwright page evaluate of `robinRpc('state')` on
`https://robin.jautis.lt/` plus an IndexedDB read — record it as evidence, not
as a gate pass.

## Sources not reached / not verified

- No receiver is deployed: concluded from `rg` over `~/nixos-config` and
  `~/git/infra`; no live host was inspected for a stray unit. If one is later
  found, §2's "not deployed" must be re-verified.
- Hermes' own token contents were not read (only its documented path and scope);
  no `GITHUB_TOKEN` for a Robin receiver exists in any config read.
- Upstream ingestion services (Sentry, GlitchTip, GitHub Issues API docs) were
  not consulted: `research-before-building.md` says prefer the estate's existing
  mechanism, and the existing receiver already provides dedupe, auth, evidence
  storage and issue creation. If a hosted service is later considered, re-run
  the three-index check and record the outcome in the issue.
- The public phiresky leaderboard's operator endpoints were not queried (no
  credential); its behaviour here is code-derived from
  `crates/robin_highscores/src/web.rs:1908-1996`.
