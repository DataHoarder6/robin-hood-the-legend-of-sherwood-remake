# Crash reporting

Every panic leaves a bundle on disk, tries to upload it, and becomes one GitHub
issue per distinct crash — with the evidence needed to fix it and no human
copying logs.

Implementation: `crates/robin_rs/src/crash.rs` (client),
`tools/crash-receiver/receiver.py` (receiver). This page is the contract between
them; the code is the behaviour.

A player can also file a report when nothing crashed; that is a separate
contract with its own bundle, screen and endpoint: [BUG_REPORTS.md](BUG_REPORTS.md).

## What a crash leaves on disk

`<install home>/crashes/<utc>-<fingerprint>/` is written by the panic hook,
**before** any upload, so an offline crash still leaves evidence:

| File | Contents |
| --- | --- |
| `bundle.json` | panic message and location, `std::backtrace`, version, engine commit, platform, arch, install home, replay path, capture time, and a `notes` list naming any evidence that is missing and why |
| `state.json` | the last `/state` document (`docs/JSON_SERVER.md`) |
| `run-log-tail.txt` | the last 200 lines of the run log `/state` names |
| `events-tail.jsonl` | the last 50 lines of the session event file `/state` names |
| `replay-path.txt` | present only when the session was recording a replay |

The bundle directory is the unit of everything that follows: the uploader is
given exactly that path. If the hook cannot write the bundle at all it writes a
one-line `<utc>-marker.txt` beside the bundles instead of panicking again.

`<install home>` is the root `install_home` resolves
(`crates/robin_rs/src/install_home.rs`); `ROBINHOOD_INSTALL_HOME` overrides it.

The hook is installed once, at startup, beside the run log. It chains to the
hook it replaced: stderr output, the exit status and the leaderboard diagnostic
queue (`crates/robin_rs/src/bug_report.rs`) are unchanged.

Not captured: fatal signals (SIGSEGV/SIGABRT). Their handler runs when the
allocator may already be unusable and still writes only a fixed message; see
the TODO at the top of `crash.rs`.

## Upload

The dying process never touches the network. It starts the same executable
with the hidden `--upload-crash <bundle dir>` argument, detached, with no stdin
or output and its own process group, so a terminal signal that kills the game
cannot kill the upload. That child performs one request with a five-second
budget and exits non-zero on any failure, leaving the bundle for a later
attempt.

Nothing is uploaded unless `ROBIN_CRASH_ENDPOINT` is set, which also makes the
default local-only. `ROBIN_CRASH_TOKEN`, when set, is sent as
`Authorization: Bearer <token>`.

The upload body is the bundle as one JSON document: `bundle.json` with
`state`, `run_log_tail` and `events_tail` added.

`crash_report.endpoint` in the profile config is not read yet — the profile
domain carries no such setting. The environment is the only source; see the
TODO in `crash.rs`.

## Endpoint contract

`POST /crash`, `Content-Type: application/json`, `Authorization: Bearer
<CRASH_TOKEN>` (the receiver refuses a request without it), at most
`CRASH_MAX_BODY_BYTES` (default 4 MiB).

| Status | Meaning |
| --- | --- |
| 202 | accepted; body `{"fingerprint", "issue", "occurrences"}` |
| 400 | not JSON, a required field missing, or a fingerprint that is not 16 lowercase hex characters |
| 401 | missing or wrong bearer token |
| 411 / 413 | no `Content-Length`, or a body larger than the limit |
| 502 | the receiver stored the bundle but GitHub refused the issue |

`GET /healthz` answers `200 ok`.

A required field is one of `fingerprint`, `version`, `platform`, `message`;
everything else is quoted into the issue when present.

## The receiver

```
CRASH_TOKEN=<upload bearer token> \
GITHUB_TOKEN=<repo token> \
GITHUB_REPO=<owner>/<name> \
CRASH_STATE_DIR=/var/lib/robin-crash \
python3 tools/crash-receiver/receiver.py --host 127.0.0.1 --port 8099
```

| Variable | Meaning |
| --- | --- |
| `CRASH_TOKEN` | bearer token an upload must present (required) |
| `GITHUB_TOKEN` | token allowed to create issues and comments on the repository (required) |
| `GITHUB_REPO` | `owner/name` that receives the issues (required) |
| `GITHUB_API_URL` | API base, for GitHub Enterprise or a stand-in server |
| `CRASH_STATE_DIR` | where bundles and occurrence records are kept, default `crash-state` |
| `CRASH_MAX_BODY_BYTES` | largest accepted body, default 4 MiB |
| `CRASH_BIND_HOST`, `CRASH_BIND_PORT` | defaults for `--host` / `--port` |

The GitHub credential lives only on the server: the shipped binary needs a URL
and, when the receiver asks for one, an upload token. The same process and the
same environment serve voluntary reports; their extra endpoint and state paths
are owned by [BUG_REPORTS.md](BUG_REPORTS.md#provisioning-the-endpoint-operator).
Neither side has a default endpoint: without one, the bundle stays local and no
issue is filed.

Under `CRASH_STATE_DIR` the receiver keeps `bundles/<fingerprint>/<received>.json`
for every upload and `occurrences/<fingerprint>.json` for the count and the
issue it belongs to. The bundle is written before GitHub is contacted, so a
GitHub outage costs the issue, not the evidence.

## Dedupe rule

`fingerprint` is `sha256("<panic location>\\0<version>")` truncated to 16 hex
characters. The version is the `--version` line (package version plus the
commit stamped at build time), so the same line after a rebuild is a different
crash.

- first report of a fingerprint: one issue, title = the panic message, body =
  fingerprint, version, platform, panic location, backtrace, run-log tail,
  events tail, `/state`, and the bundle's path on the receiver;
- every later report of the same fingerprint: one comment with the occurrence
  count, and no second issue.

One fingerprint per issue is what keeps a crash loop from filing a thousand
issues; the comment is what proves the loop is still happening.

## Tests

`bash scripts/check-quality.sh client-release` runs the client regressions
(panic in a thread writes a bundle, no endpoint spawns no uploader, fingerprint
stability). `bash scripts/check-quality.sh tooling` runs
`tools/crash-receiver/test_receiver.py`: authentication refusal, oversized-body
refusal, first report files one issue, repeat comments instead of filing, and
the bundle persisted.
