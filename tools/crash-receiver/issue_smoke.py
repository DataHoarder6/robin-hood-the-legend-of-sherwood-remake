#!/usr/bin/env python3
"""Opt-in live proof that a voluntary bug report becomes a GitHub issue.

This is the third layer of #51. It posts one client-written ``bundle.json`` to
a bug receiver, reads the created issue back with the GitHub CLI, asserts the
title, the ``bug``/``bug-report`` labels, the reproduction facts, every
"unavailable" reason and the bundle download link, then closes the issue it
created. A normal gate never calls this module: the flag below is the only
gate, and the gate suites have neither the token nor a tracker write path.

The live run is a human gate. The owner provisions ``ROBIN_BUG_ENDPOINT``,
``ROBIN_CRASH_TOKEN`` and ``GITHUB_TOKEN`` for the real receiver, files a
report from the game and runs this harness on the resulting bundle with
``ROBIN_ISSUE_SMOKE=1``. A deterministic dry run points the endpoint at a
local receiver with a fake GitHub API and ``ROBIN_ISSUE_SMOKE_CLI`` at a stub
``gh`` executable; ``test_receiver.py`` does exactly that, so the gate proves
the readback and close calls without touching the tracker.

Standard library only, like the receiver beside it.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

import receiver

OPT_IN_ENV = "ROBIN_ISSUE_SMOKE"
ENDPOINT_ENV = "ROBIN_BUG_ENDPOINT"
TOKEN_ENV = "ROBIN_CRASH_TOKEN"
CLI_ENV = "ROBIN_ISSUE_SMOKE_CLI"


class SmokeError(Exception):
    """A refused run or a failed proof; nothing is assumed to have passed."""


def require_opt_in(environ: dict[str, str]) -> None:
    """The only gate. Called before any file, socket or CLI access."""
    if environ.get(OPT_IN_ENV) != "1":
        raise SmokeError(
            f"refusing to run: set {OPT_IN_ENV}=1 deliberately. "
            "A live smoke writes to the issue tracker and is never a gate step."
        )


def load_bundle(path: Path) -> dict:
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise SmokeError(f"cannot read bundle {path}: {error}") from error
    try:
        bundle = json.loads(raw)
    except json.JSONDecodeError as error:
        raise SmokeError(f"bundle {path} is not JSON: {error}") from error
    try:
        return receiver.validate_bug(bundle)
    except receiver.Refused as error:
        raise SmokeError(f"bundle {path} is not a bug report: {error.reason}") from error


def post_bundle(endpoint: str, bundle: dict, token: str) -> dict:
    """POST the exact bundle the client wrote; return the receiver's receipt."""
    body = json.dumps(bundle).encode()
    request = urllib.request.Request(endpoint, data=body, method="POST")
    request.add_header("Content-Type", "application/json")
    request.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(request, timeout=20) as reply:
            answer = json.loads(reply.read() or b"{}")
    except urllib.error.HTTPError as error:
        detail = error.read().decode(errors="replace")[:500]
        raise SmokeError(f"receiver answered HTTP {error.code}: {detail}") from error
    except (urllib.error.URLError, json.JSONDecodeError, TimeoutError) as error:
        raise SmokeError(f"receiver is unreachable or answered invalid JSON: {error}") from error
    for field in ("issue", "bundle", "fingerprint"):
        if field not in answer:
            raise SmokeError(f"receiver receipt has no {field!r}: {answer!r}")
    return answer


def _cli(gh: str, arguments: list[str]) -> dict:
    try:
        completed = subprocess.run(
            [gh, *arguments],
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise SmokeError(f"GitHub CLI failed ({' '.join(arguments)}): {error}") from error
    try:
        return json.loads(completed.stdout or "{}")
    except json.JSONDecodeError as error:
        raise SmokeError(f"GitHub CLI returned invalid JSON: {error}") from error


def read_issue(number: int, repo: str, gh: str) -> dict:
    """Read one issue back; this is the proof, not a log line."""
    issue = _cli(
        gh,
        [
            "issue",
            "view",
            str(number),
            "--repo",
            repo,
            "--json",
            "number,title,labels,body,state",
        ],
    )
    if issue.get("number") != number:
        raise SmokeError(f"read back issue {number!r}, got {issue.get('number')!r}")
    return issue


def close_issue(number: int, repo: str, gh: str) -> None:
    _cli(gh, ["issue", "close", str(number), "--repo", repo])


def _label_names(issue: dict) -> set[str]:
    labels = issue.get("labels") or []
    return {
        label.get("name", "") if isinstance(label, dict) else str(label)
        for label in labels
    }


def verify_issue(issue: dict, bundle: dict, receipt: dict) -> dict:
    """Assert every fact the issue owes the reader; return a recordable proof."""
    expected_title = receiver.bug_title(bundle)
    if issue.get("title") != expected_title:
        raise SmokeError(f"title was {issue.get('title')!r}, expected {expected_title!r}")
    labels = _label_names(issue)
    missing = set(receiver.BUG_LABELS) - labels
    if missing:
        raise SmokeError(f"issue is missing labels {sorted(missing)}, has {sorted(labels)}")

    body = issue.get("body")
    if not isinstance(body, str):
        raise SmokeError(f"issue has no body: {issue!r}")
    required = [
        "### Reproduction facts",
        *receiver.bug_facts(bundle),
        f"`{receipt['bundle']}`",
        f"GET {receiver.BUG_REPORT_DOWNLOAD_PREFIX}{receipt['bundle']}",
    ]
    for fact in required:
        if fact not in body:
            raise SmokeError(f"issue body is missing {fact!r}")
    unavailable = receiver.unavailable_evidence(bundle)
    for path, reason in unavailable:
        for expected in (f"`{path}`", reason):
            if expected not in body:
                raise SmokeError(f"issue body is missing unavailable evidence {expected!r}")
    if "### Description" not in body:
        raise SmokeError("issue body has no Description section")
    if body.index("### Reproduction facts") > body.index("### Description"):
        raise SmokeError("reproduction facts must lead the issue body")

    return {
        "issue": receipt["issue"],
        "title": expected_title,
        "labels": sorted(labels),
        "bundle": receipt["bundle"],
        "fingerprint": receipt["fingerprint"],
        "facts_checked": len(required),
        "unavailable_reasons_checked": len(unavailable),
        "state_at_read": issue.get("state"),
    }


def run(
    bundle_path: Path,
    *,
    endpoint: str,
    repo: str,
    token: str,
    gh: str,
    environ: dict[str, str] | None = None,
) -> dict:
    """File, read back, verify and close. The opt-in check comes first."""
    require_opt_in(environ if environ is not None else os.environ)
    bundle = load_bundle(bundle_path)
    receipt = post_bundle(endpoint, bundle, token)
    number = receipt["issue"]
    try:
        issue = read_issue(number, repo, gh)
        proof = verify_issue(issue, bundle, receipt)
    finally:
        # An issue opened by this harness is this harness's responsibility,
        # including when an assertion fails. The receipt names it even when
        # the readback did not succeed.
        close_issue(number, repo, gh)
    return proof


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bundle",
        required=True,
        type=Path,
        help="bundle.json written by the game, a console report or POST /bug-report",
    )
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPO", ""))
    args = parser.parse_args(argv)

    gh = os.environ.get(CLI_ENV, "gh")
    try:
        require_opt_in(os.environ)
        if not args.repo:
            raise SmokeError("set GITHUB_REPO or pass --repo")
        endpoint = os.environ.get(ENDPOINT_ENV, "")
        if not endpoint:
            raise SmokeError(f"set {ENDPOINT_ENV} to the receiver's POST /bug-report URL")
        token = os.environ.get(TOKEN_ENV, "")
        if not token:
            raise SmokeError(f"set {TOKEN_ENV} to the receiver's CRASH_TOKEN")
        proof = run(
            args.bundle,
            endpoint=endpoint,
            repo=args.repo,
            token=token,
            gh=gh,
        )
    except SmokeError as error:
        sys.stderr.write(f"issue-smoke: {error}\n")
        return 2
    print(json.dumps(proof, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
