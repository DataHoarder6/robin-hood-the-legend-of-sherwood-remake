# Issue #51 — live bug-report smoke: exact owner run

The deterministic layer is in the tree: `tools/crash-receiver/issue_smoke.py`
posts one client-written `bundle.json`, reads the issue back with `gh`, asserts
title, `bug`/`bug-report` labels, reproduction facts, every "unavailable" reason
and the bundle link, then closes the issue it created. Its dry run is
`python3 -m unittest discover -s tools/crash-receiver -p test_receiver.py`
(gated by `tooling`): a stub endpoint and a stub `gh` prove the readback and
close calls without touching a tracker. Last verified: 2026-11-26.

## The live run is owner-only

A normal gate never writes to the tracker: the harness refuses to start unless
`ROBIN_ISSUE_SMOKE=1` is set deliberately. No smoke was run for this work, and
no issue was filed, read back or closed here. Capability needed: a reachable
receiver with `CRASH_TOKEN` and `GITHUB_TOKEN`, a `gh` authenticated with
issue create/read/close rights on the target repo, and a bundle filed by the
real client.

1. File one report from the real client so a bundle exists. The client needs
   `ROBIN_BUG_ENDPOINT` and `ROBIN_CRASH_TOKEN` in its environment; entrances
   are in [BUG_REPORTS.md](../BUG_REPORTS.md#three-entrances-one-path).
2. Run the smoke on that bundle (the harness posts it again and owns the issue
   it creates):

```sh
ROBIN_ISSUE_SMOKE=1 \
ROBIN_BUG_ENDPOINT=https://<host>:8099/bug-report \
ROBIN_CRASH_TOKEN=<receiver CRASH_TOKEN> \
GITHUB_REPO=DataHoarder6/robin-hood-the-legend-of-sherwood-remake \
python3 tools/crash-receiver/issue_smoke.py \
  --bundle <install-home>/bug-reports/<utc>-<origin>-<n>/bundle.json
```

Exit `0` prints the proof JSON (`issue`, `title`, `labels`, `bundle`,
`fingerprint`, facts and unavailable-reason counts, state at read). Exit `2`
prints `issue-smoke: <reason>` on stderr; the harness still closes the issue it
opened, even when an assertion failed. Record the proof with the release, as
[RELEASES.md](../RELEASES.md) requires; never paste the bundle itself.

## What is not proved by the dry run

The stub proves the harness's calls, not that a GitHub issue exists. Until the
command above is run by the owner with real credentials, #51's third layer
stays unproved. This page is the pointer; the opt-in rule stays in
[BUG_REPORTS.md](../BUG_REPORTS.md#upload) and the release policy in
[RELEASES.md](../RELEASES.md).
