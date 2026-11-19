#!/usr/bin/env python3
"""Bug and crash bundle receiver: authenticate, keep evidence, file one issue each.

A game client POSTs one JSON document per crash to ``/crash`` and one per
voluntary report to ``/bug-report``. This service authenticates both, stores the
bundle under its state directory, and files one GitHub issue per fingerprint
through the REST API; a repeat fingerprint adds a comment instead of a second
issue, and a bug report says so in the comment.

Why a service at all: the GitHub credential, the dedupe state and the rate
limit live here, where they can be revoked, and the shipped binary only needs a
URL. ``docs/CRASHES.md`` documents the crash contract, ``docs/BUG_REPORTS.md``
the bug-report one, and both name the environment this service reads.

Standard library only: this runs beside the leaderboard service on the estate's
server and must not need a dependency it did not already have.
"""

from __future__ import annotations

import argparse
import dataclasses
import hmac
import json
import os
import re
import sys
import threading
import urllib.error
import urllib.request
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

CRASH_PATH = "/crash"
BUG_REPORT_PATH = "/bug-report"
BUG_REPORT_DOWNLOAD_PREFIX = "/bug-reports/"
HEALTH_PATH = "/healthz"
# Bigger than any bundle the client writes (bounded tails, one /state document)
# and small enough that a hostile body cannot exhaust this process.
DEFAULT_MAX_BODY_BYTES = 4 * 1024 * 1024
DEFAULT_GITHUB_API = "https://api.github.com"
FINGERPRINT = re.compile(r"^[0-9a-f]{16}$")
BUNDLE_ID = re.compile(r"^[0-9a-f]{16}$")
SESSION_SLUG = re.compile(r"[^0-9A-Za-z_-]+")
GITHUB_TIMEOUT_SECONDS = 20

# Every bug report is labelled; the second label separates voluntary reports
# from crashes in one issue list.
BUG_LABELS = ("bug", "bug-report")

# The fields a bundle must carry to be worth filing. Everything else is quoted
# when present and reported as "not captured" when absent.
REQUIRED_FIELDS = ("fingerprint", "version", "platform", "message")


class Refused(Exception):
    """A request the receiver will not serve, with the status to answer."""

    def __init__(self, status: int, reason: str):
        super().__init__(reason)
        self.status = status
        self.reason = reason


class GitHubError(Exception):
    """The GitHub API refused a request; the bundle stays on disk."""


def utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")


def session_slug(session: object) -> str:
    """A path-safe name for one session id; anything unusable is 'unlabelled'."""
    if not isinstance(session, str):
        return "unlabelled"
    slug = SESSION_SLUG.sub("", session)[:64]
    return slug or "unlabelled"


@dataclasses.dataclass(frozen=True)
class Config:
    crash_token: str
    github_token: str
    github_repo: str
    state_dir: Path
    github_api: str = DEFAULT_GITHUB_API
    max_body_bytes: int = DEFAULT_MAX_BODY_BYTES

    @classmethod
    def from_environment(cls) -> "Config":
        missing = [
            name
            for name in ("CRASH_TOKEN", "GITHUB_TOKEN", "GITHUB_REPO")
            if not os.environ.get(name)
        ]
        if missing:
            raise SystemExit(
                f"crash-receiver: set {', '.join(missing)} in the environment"
            )
        return cls(
            crash_token=os.environ["CRASH_TOKEN"],
            github_token=os.environ["GITHUB_TOKEN"],
            github_repo=os.environ["GITHUB_REPO"],
            state_dir=Path(os.environ.get("CRASH_STATE_DIR", "crash-state")),
            github_api=os.environ.get("GITHUB_API_URL", DEFAULT_GITHUB_API),
            max_body_bytes=int(
                os.environ.get("CRASH_MAX_BODY_BYTES", DEFAULT_MAX_BODY_BYTES)
            ),
        )


class Store:
    """Bundles and per-fingerprint occurrence counts on disk.

    Layout: ``<state>/bundles/<fingerprint>/<received>.json`` keeps every
    upload, and ``<state>/occurrences/<fingerprint>.json`` records the count and
    the issue it belongs to.
    """

    def __init__(self, directory: Path):
        self.directory = directory

    def save(self, fingerprint: str, bundle: dict) -> Path:
        directory = self.directory / "bundles" / fingerprint
        directory.mkdir(parents=True, exist_ok=True)
        received = utc_now()
        path = directory / f"{received}.json"
        suffix = 0
        while path.exists():
            suffix += 1
            path = directory / f"{received}-{suffix}.json"
        _write_atomically(path, json.dumps(bundle).encode())
        return path

    def occurrence(self, fingerprint: str) -> dict | None:
        path = self._occurrence_path(fingerprint)
        try:
            return json.loads(path.read_text())
        except FileNotFoundError:
            return None

    def record_occurrence(self, fingerprint: str, issue: int | None) -> dict:
        existing = self.occurrence(fingerprint) or {
            "fingerprint": fingerprint,
            "count": 0,
            "issue": None,
            "first_seen": utc_now(),
        }
        existing["count"] += 1
        existing["last_seen"] = utc_now()
        if issue is not None:
            existing["issue"] = issue
        path = self._occurrence_path(fingerprint)
        path.parent.mkdir(parents=True, exist_ok=True)
        _write_atomically(path, json.dumps(existing, indent=2).encode())
        return existing

    def _occurrence_path(self, fingerprint: str) -> Path:
        return self.directory / "occurrences" / f"{fingerprint}.json"

    # ── Bug reports ─────────────────────────────────────────────────────
    #
    # A bug report is stored under a random id, because a report's identity is
    # its evidence, not its fingerprint: two seats of one stuck session file two
    # bundles that an issue must be able to link separately.

    def save_bug_bundle(self, bundle: dict) -> str:
        directory = self.directory / "bug-bundles"
        directory.mkdir(parents=True, exist_ok=True)
        while True:
            bundle_id = os.urandom(8).hex()
            path = directory / f"{bundle_id}.json"
            if not path.exists():
                break
        _write_atomically(path, json.dumps(bundle).encode())
        return bundle_id

    def bug_bundle(self, bundle_id: str) -> dict | None:
        path = self.directory / "bug-bundles" / f"{bundle_id}.json"
        try:
            return json.loads(path.read_text())
        except FileNotFoundError:
            return None

    def bug_occurrence(self, fingerprint: str) -> dict | None:
        path = self.directory / "bug-occurrences" / f"{fingerprint}.json"
        try:
            return json.loads(path.read_text())
        except FileNotFoundError:
            return None

    def record_bug_occurrence(
        self, fingerprint: str, bundle_id: str, issue: int | None, session: str
    ) -> dict:
        existing = self.bug_occurrence(fingerprint) or {
            "fingerprint": fingerprint,
            "count": 0,
            "issue": None,
            "sessions": [],
            "first_seen": utc_now(),
        }
        existing["count"] += 1
        existing["last_seen"] = utc_now()
        existing.setdefault("bundles", []).append(bundle_id)
        if session not in existing["sessions"]:
            existing["sessions"].append(session)
        if issue is not None:
            existing["issue"] = issue
        path = self.directory / "bug-occurrences" / f"{fingerprint}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        _write_atomically(path, json.dumps(existing, indent=2).encode())
        return existing

    def group_session(self, session: str, entry: dict) -> None:
        """Keep one session's reports together, whatever order they arrive in."""
        slug = session_slug(session)
        directory = self.directory / "bug-sessions"
        directory.mkdir(parents=True, exist_ok=True)
        path = directory / f"{slug}.json"
        try:
            grouped = json.loads(path.read_text())
        except FileNotFoundError:
            grouped = {"session": session, "reports": []}
        grouped["reports"].append(entry)
        _write_atomically(path, json.dumps(grouped, indent=2).encode())

    def session_group(self, session: str) -> dict | None:
        path = self.directory / "bug-sessions" / f"{session_slug(session)}.json"
        try:
            return json.loads(path.read_text())
        except FileNotFoundError:
            return None


class GitHub:
    """The REST calls this service needs, and nothing more."""

    def __init__(self, api: str, repo: str, token: str):
        self.api = api.rstrip("/")
        self.repo = repo
        self.token = token

    def ensure_labels(self, names: tuple[str, ...]) -> None:
        """Create each label once; 422 means it is already there."""
        for name in names:
            try:
                self._request(
                    "POST",
                    f"/repos/{self.repo}/labels",
                    {
                        "name": name,
                        "color": "d73a4a" if name == "bug" else "1d76db",
                        "description": (
                            "A defect the game reports itself"
                            if name == "bug"
                            else "A voluntary in-game bug report with a bundle"
                        ),
                    },
                )
            except GitHubError as error:
                if "HTTP 422" not in str(error):
                    raise

    def create_issue(
        self, title: str, body: str, labels: tuple[str, ...] = ()
    ) -> int:
        payload: dict = {"title": title, "body": body}
        if labels:
            payload["labels"] = list(labels)
        response = self._request("POST", f"/repos/{self.repo}/issues", payload)
        number = response.get("number")
        if not isinstance(number, int):
            raise GitHubError(f"issue response has no number: {response!r}")
        return number

    def comment(self, number: int, body: str) -> None:
        self._request("POST", f"/repos/{self.repo}/issues/{number}/comments", {
            "body": body,
        })

    def _request(self, method: str, path: str, payload: dict) -> dict:
        request = urllib.request.Request(
            f"{self.api}{path}",
            data=json.dumps(payload).encode(),
            method=method,
            headers={
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
                "User-Agent": "robin-crash-receiver",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=GITHUB_TIMEOUT_SECONDS) as reply:
                return json.loads(reply.read() or b"{}")
        except urllib.error.HTTPError as error:
            detail = error.read().decode(errors="replace")[:500]
            raise GitHubError(f"GitHub answered HTTP {error.code}: {detail}") from error
        except urllib.error.URLError as error:
            raise GitHubError(f"GitHub is unreachable: {error.reason}") from error
        except json.JSONDecodeError as error:
            raise GitHubError(f"GitHub answered invalid JSON: {error}") from error


def _write_atomically(path: Path, content: bytes) -> None:
    temporary = path.with_suffix(path.suffix + ".partial")
    temporary.write_bytes(content)
    os.replace(temporary, path)


def bounded(text: object, limit: int) -> str:
    """Text for an issue body, bounded and never raising on odd JSON types."""
    if text is None:
        return "not captured"
    if not isinstance(text, str):
        text = json.dumps(text, indent=2, sort_keys=True)
    if len(text) <= limit:
        return text
    return f"{text[:limit]}\n… truncated at {limit} of {len(text)} characters"


def fenced(text: str) -> str:
    """A code block whose fence cannot be closed by the text it carries."""
    fence = "```"
    while fence in text:
        fence += "`"
    return f"{fence}\n{text}\n{fence}"


def issue_title(bundle: dict) -> str:
    message = " ".join(str(bundle["message"]).split())
    return message[:120] or "crash without a message"


def issue_body(bundle: dict, bundle_path: Path) -> str:
    def field(name: str, limit: int = 400) -> str:
        return bounded(bundle.get(name), limit)

    sections = [
        "### Crash report",
        "",
        f"- Fingerprint: `{field('fingerprint')}`",
        f"- Version: `{field('version')}`",
        f"- Platform: `{field('platform')}` / `{field('arch')}`",
        f"- Panic location: `{field('location')}`",
        f"- Captured at: `{field('captured_at')}`",
        f"- Replay: `{field('replay_path')}`",
        "",
        "### Panic message",
        "",
        fenced(bounded(bundle.get("message"), 2000)),
        "",
        "### Backtrace",
        "",
        fenced(bounded(bundle.get("backtrace"), 12000)),
        "",
        "### Run log tail",
        "",
        fenced(bounded(bundle.get("run_log_tail"), 12000)),
        "",
        "### Session events tail",
        "",
        fenced(bounded(bundle.get("events_tail"), 6000)),
        "",
        "### /state at the crash",
        "",
        fenced(bounded(bundle.get("state"), 20000)),
        "",
        "### Bundle",
        "",
        f"The complete bundle is attached to this report and stored by the "
        f"receiver at `{bundle_path}`.",
    ]
    notes = bundle.get("notes")
    if notes:
        sections += ["", "### Receiver notes", "", fenced(bounded(notes, 4000))]
    return "\n".join(sections)


def comment_body(bundle: dict, occurrences: int) -> str:
    return "\n".join([
        f"Occurrence {occurrences} of this crash "
        f"(fingerprint `{bounded(bundle.get('fingerprint'), 32)}`).",
        "",
        f"- Version: `{bounded(bundle.get('version'), 200)}`",
        f"- Captured at: `{bounded(bundle.get('captured_at'), 200)}`",
        "",
        "The new bundle is attached to this report.",
    ])


def validate(bundle: object) -> dict:
    if not isinstance(bundle, dict):
        raise Refused(400, "body is not a JSON object")
    for field_name in REQUIRED_FIELDS:
        value = bundle.get(field_name)
        if not isinstance(value, str) or not value.strip():
            raise Refused(400, f"missing or empty field {field_name!r}")
    if not FINGERPRINT.match(bundle["fingerprint"]):
        raise Refused(400, "fingerprint is not 16 lowercase hex characters")
    return bundle


def validate_bug(bundle: object) -> dict:
    """A bug report exists to be reproduced: description, fingerprint, build, session."""
    if not isinstance(bundle, dict):
        raise Refused(400, "body is not a JSON object")
    description = bundle.get("description")
    if not isinstance(description, str) or not description.strip():
        raise Refused(400, "missing or empty field 'description'")
    fingerprint = bundle.get("fingerprint")
    if not isinstance(fingerprint, str) or not FINGERPRINT.match(fingerprint):
        raise Refused(400, "fingerprint is not 16 lowercase hex characters")
    for field_name in ("build", "session"):
        if not isinstance(bundle.get(field_name), dict):
            raise Refused(400, f"missing field {field_name!r}")
    return bundle


def _dig(document: object, *path: str, default: object = None) -> object:
    """One value from a nested document, never raising on a wrong shape."""
    value = document
    for key in path:
        if not isinstance(value, dict) or key not in value:
            return default
        value = value[key]
    return value


def bug_title(bundle: dict) -> str:
    return " ".join(str(bundle["description"]).split())[:120] or "bug report without a description"


def bug_facts(bundle: dict) -> list[str]:
    """The reproduction facts, first thing in the issue body."""
    session = bundle["session"]
    build = bundle["build"]
    game = bundle.get("game") if isinstance(bundle.get("game"), dict) else {}
    tick = session.get("tick") if isinstance(session.get("tick"), dict) else {}
    mission = session.get("mission") if isinstance(session.get("mission"), dict) else {}
    transport = session.get("transport") if isinstance(session.get("transport"), dict) else {}
    desync = session.get("desync") if isinstance(session.get("desync"), dict) else {}
    world = bundle.get("world") if isinstance(bundle.get("world"), dict) else {}
    alive = world.get("alive") if isinstance(world.get("alive"), dict) else {}
    seats = session.get("players") if isinstance(session.get("players"), list) else []
    roster = ", ".join(
        f"seat {seat.get('seat')}"
        f"{' ' + seat['nickname'] if seat.get('nickname') else ''}"
        f"{' (connected)' if seat.get('connected') else ' (empty)'}"
        for seat in seats
        if isinstance(seat, dict)
    )
    return [
        f"- Session: `{bounded(session.get('session_id'), 80)}` — "
        f"{bounded(session.get('kind'), 20)}, local seat "
        f"`{bounded(transport.get('local_seat'), 8)}` of "
        f"`{bounded(transport.get('expected_players'), 8)}`, "
        f"transport reconnecting: `{bounded(transport.get('reconnecting'), 8)}`",
        f"- Mission: `{bounded(mission.get('title'), 120)}` "
        f"(`{bounded(mission.get('filename'), 120)}`, profile "
        f"`{bounded(mission.get('profile_id'), 12)}`), map `{bounded(mission.get('map'), 60)}`",
        f"- Tick: timeline frame `{bounded(tick.get('timeline_frame'), 12)}`, "
        f"simulation tick `{bounded(tick.get('simulation_tick'), 12)}`",
        f"- Build: `{bounded(build.get('version'), 120)}` at `{bounded(build.get('git_rev'), 80)}`, "
        f"{bounded(build.get('target'), 60)} {bounded(build.get('profile'), 20)}, "
        f"release `{bounded(build.get('release'), 8)}`",
        f"- Operation: `{bounded(game.get('operation'), 40)}`",
        f"- Seats: {roster or 'not captured'}",
        f"- Desync: hash `{bounded(desync.get('current_state_hash'), 32)}`, "
        f"last agreeing frame `{bounded(desync.get('last_compared_hash_frame'), 12)}`, "
        f"mismatches `{bounded(desync.get('desync_count'), 12)}`",
        f"- Alive characters: `{bounded(alive.get('count'), 8)}` "
        f"({bounded(alive.get('predicate'), 80)})",
    ]


def bug_issue_body(bundle: dict, bundle_id: str, bundle_path: Path) -> str:
    """The issue body: reproduction facts first, the bundle summarised, not pasted."""
    script = _dig(bundle, "game", "script", default={}) or {}
    counts = script.get("instance_counts") if isinstance(script, dict) else None
    terminal = _dig(bundle, "game", "terminal", default={})
    presentation = _dig(bundle, "game", "presentation", default={})
    saves = _dig(bundle, "files", "saves", default=[])
    replay = _dig(bundle, "files", "replay", default=[])
    save_lines = (
        "\n".join(
            f"  - `{bounded(item.get('file'), 120)}`: {bounded(item.get('bytes'), 20)} bytes"
            for item in saves
            if isinstance(item, dict)
        )
        or "  - none copied"
    )
    replay_lines = (
        "\n".join(
            f"  - `{bounded(item.get('file'), 120)}`: {bounded(item.get('bytes'), 20)} bytes"
            + (
                f" ({bounded(item.get('truncation'), 80)})"
                if item.get("truncation")
                else ""
            )
            for item in replay
            if isinstance(item, dict)
        )
        or "  - no recording was active"
    )
    sections = [
        "### Reproduction facts",
        "",
        *bug_facts(bundle),
        "",
        "### Description",
        "",
        fenced(bounded(bundle.get("description"), 4000)),
        "",
        "### Note",
        "",
        fenced(bounded(bundle.get("note") or "not given", 4000)),
        "",
        "### Script state",
        "",
        f"Global script instance `{bounded(_dig(script, 'global', 'class'), 80)}`; "
        f"instances: {bounded(counts, 300)}.",
        "",
        fenced(bounded(_dig(script, "global", "members"), 8000)),
        "",
        "### Mission-end and presentation",
        "",
        fenced(
            bounded(
                {
                    "terminal": terminal,
                    "presentation": presentation,
                },
                4000,
            )
        ),
        "",
        "### Bundle",
        "",
        f"- Id: `{bundle_id}`",
        f"- Download: `GET {BUG_REPORT_DOWNLOAD_PREFIX}{bundle_id}` "
        f"(the same bearer token this service demands)",
        f"- On the receiver: `{bundle_path}`",
        "",
        "Saves copied into the bundle:",
        "",
        save_lines,
        "",
        "Replay copied into the bundle:",
        "",
        replay_lines,
    ]
    notes = bundle.get("notes")
    if notes:
        sections += ["", "### Capture notes", "", fenced(bounded(notes, 4000))]
    return "\n".join(sections)


def bug_comment_body(bundle: dict, bundle_id: str, occurrences: int) -> str:
    session = bundle["session"]
    tick = _dig(session, "tick", "timeline_frame")
    return "\n".join(
        [
            f"Another report of this state (fingerprint "
            f"`{bounded(bundle.get('fingerprint'), 32)}`, occurrence {occurrences}).",
            "",
            f"- Session: `{bounded(session.get('session_id'), 80)}` "
            f"({bounded(session.get('kind'), 20)})",
            f"- Tick: `{bounded(tick, 12)}`",
            f"- Build: `{bounded(_dig(bundle, 'build', 'version'), 120)}`",
            "",
            f"The new bundle is `{bundle_id}`: "
            f"`GET {BUG_REPORT_DOWNLOAD_PREFIX}{bundle_id}` with the receiver's bearer token.",
        ]
    )


class Service:
    """One authenticated report: store it, then dedupe into GitHub.

    The lock covers the read-decide-write of the occurrence record, so two
    simultaneous uploads of one fingerprint cannot both create an issue.
    """

    def __init__(self, config: Config):
        self.config = config
        self.store = Store(config.state_dir)
        self.github = GitHub(config.github_api, config.github_repo, config.github_token)
        self.lock = threading.Lock()
        self.labels_ready = False

    def report(self, raw: bytes) -> dict:
        try:
            bundle = json.loads(raw)
        except json.JSONDecodeError as error:
            raise Refused(400, f"body is not JSON: {error}") from error
        bundle = validate(bundle)
        fingerprint = bundle["fingerprint"]
        # Evidence first: a GitHub outage must not cost the bundle.
        path = self.store.save(fingerprint, bundle)
        with self.lock:
            occurrence = self.store.occurrence(fingerprint)
            if occurrence is None:
                number = self.github.create_issue(
                    issue_title(bundle), issue_body(bundle, path)
                )
                occurrence = self.store.record_occurrence(fingerprint, number)
            else:
                self.github.comment(
                    occurrence["issue"],
                    comment_body(bundle, occurrence["count"] + 1),
                )
                occurrence = self.store.record_occurrence(fingerprint, None)
        return {
            "fingerprint": fingerprint,
            "issue": occurrence["issue"],
            "occurrences": occurrence["count"],
        }

    def report_bug(self, raw: bytes) -> dict:
        """One voluntary report: store its bundle, then file or comment once.

        The lock covers the read-decide-write of the occurrence record, so two
        seats filing the same stuck state at the same moment cannot both create
        an issue; the second becomes a comment that names its own bundle.
        """
        try:
            bundle = json.loads(raw)
        except json.JSONDecodeError as error:
            raise Refused(400, f"body is not JSON: {error}") from error
        bundle = validate_bug(bundle)
        fingerprint = bundle["fingerprint"]
        bundle_id = self.store.save_bug_bundle(bundle)
        path = self.config.state_dir / "bug-bundles" / f"{bundle_id}.json"
        session = _dig(bundle, "session", "session_id")
        with self.lock:
            occurrence = self.store.bug_occurrence(fingerprint)
            if occurrence is None:
                if not self.labels_ready:
                    self.github.ensure_labels(BUG_LABELS)
                    self.labels_ready = True
                number = self.github.create_issue(
                    bug_title(bundle),
                    bug_issue_body(bundle, bundle_id, path),
                    labels=BUG_LABELS,
                )
                occurrence = self.store.record_bug_occurrence(
                    fingerprint, bundle_id, number, session if isinstance(session, str) else ""
                )
            else:
                self.github.comment(
                    occurrence["issue"],
                    bug_comment_body(bundle, bundle_id, occurrence["count"] + 1),
                )
                occurrence = self.store.record_bug_occurrence(
                    fingerprint, bundle_id, None, session if isinstance(session, str) else ""
                )
            self.store.group_session(
                session if isinstance(session, str) else "",
                {
                    "bundle": bundle_id,
                    "fingerprint": fingerprint,
                    "issue": occurrence["issue"],
                    "seat": _dig(bundle, "session", "transport", "local_seat"),
                    "kind": _dig(bundle, "session", "kind"),
                    "tick": _dig(bundle, "session", "tick", "timeline_frame"),
                    "received_at": utc_now(),
                },
            )
        return {
            "fingerprint": fingerprint,
            "bundle": bundle_id,
            "issue": occurrence["issue"],
            "occurrences": occurrence["count"],
        }


def make_handler(config: Config, service: Service):
    """A handler class for one configuration and its one service."""

    class CrashHandler(BaseHTTPRequestHandler):
        server_version = "robin-crash-receiver/1"
        protocol_version = "HTTP/1.1"

        def do_GET(self) -> None:  # noqa: N802 - http.server's spelling
            if self.path == HEALTH_PATH:
                self._reply(200, b"ok\n", "text/plain")
                return
            if self.path.startswith(BUG_REPORT_DOWNLOAD_PREFIX):
                self._download_bug_report()
                return
            self._reply(404, b"not found\n", "text/plain")

        def _download_bug_report(self) -> None:
            """The authenticated bundle download an issue links.

            Never public: the bundle carries saves, a replay and the player's
            own words.
            """
            try:
                self._authorize()
                target = self.path[len(BUG_REPORT_DOWNLOAD_PREFIX) :]
                if target.startswith("by-session/"):
                    session = target[len("by-session/") :]
                    grouped = service.store.session_group(session)
                    if grouped is None:
                        raise Refused(404, "no reports for that session")
                    self._reply(
                        200,
                        json.dumps(grouped).encode(),
                        "application/json",
                    )
                    return
                if not BUNDLE_ID.match(target):
                    raise Refused(400, "bundle id is not 16 lowercase hex characters")
                bundle = service.store.bug_bundle(target)
                if bundle is None:
                    raise Refused(404, "no such bundle")
                self._reply(200, json.dumps(bundle).encode(), "application/json")
            except Refused as refusal:
                self._reply(
                    refusal.status,
                    (refusal.reason + "\n").encode(),
                    "text/plain",
                )

        def do_POST(self) -> None:  # noqa: N802 - http.server's spelling
            try:
                if self.path == CRASH_PATH:
                    self._authorize()
                    answer = service.report(self._read_body())
                elif self.path == BUG_REPORT_PATH:
                    self._authorize()
                    answer = service.report_bug(self._read_body())
                else:
                    raise Refused(404, "unknown path")
            except Refused as refusal:
                self._reply(
                    refusal.status,
                    (refusal.reason + "\n").encode(),
                    "text/plain",
                )
                return
            except GitHubError as error:
                self.log_error("github: %s", error)
                self._reply(502, (f"{error}\n").encode(), "text/plain")
                return
            self._reply(202, json.dumps(answer).encode(), "application/json")

        def _authorize(self) -> None:
            header = self.headers.get("Authorization", "")
            if not hmac.compare_digest(header, f"Bearer {config.crash_token}"):
                raise Refused(401, "missing or wrong Authorization header")

        def _read_body(self) -> bytes:
            length = self.headers.get("Content-Length")
            if length is None:
                raise Refused(411, "Content-Length is required")
            try:
                size = int(length)
            except ValueError as error:
                raise Refused(400, "Content-Length is not a number") from error
            if size > config.max_body_bytes:
                raise Refused(413, f"body exceeds {config.max_body_bytes} bytes")
            body = self.rfile.read(size)
            if len(body) != size:
                raise Refused(400, "body ended before Content-Length")
            return body

        def _reply(self, status: int, body: bytes, content_type: str) -> None:
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            if status == 413:
                # Never keep reading a body we refused.
                self.close_connection = True
                self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *args) -> None:  # noqa: A002
            sys.stderr.write(f"{self.address_string()} - {format % args}\n")

    return CrashHandler


def make_server(config: Config, host: str, port: int) -> ThreadingHTTPServer:
    """A listening server carrying its own service, so a caller can inspect it."""
    service = Service(config)
    server = ThreadingHTTPServer((host, port), make_handler(config, service))
    server.service = service  # type: ignore[attr-defined]
    return server



def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Receive Robin Hood crash bundles.")
    parser.add_argument("--host", default=os.environ.get("CRASH_BIND_HOST", "127.0.0.1"))
    parser.add_argument("--port", type=int, default=int(os.environ.get("CRASH_BIND_PORT", "8099")))
    arguments = parser.parse_args(argv)
    config = Config.from_environment()
    server = make_server(config, arguments.host, arguments.port)
    host, port = server.server_address[:2]
    sys.stderr.write(f"crash-receiver listening on {host}:{port}, state in {config.state_dir}\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
