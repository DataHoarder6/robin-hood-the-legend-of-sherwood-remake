#!/usr/bin/env python3
"""Receiver regressions: authentication, size refusal, dedupe and persistence.

The GitHub API is a stand-in HTTP server in this process, so every test proves
what the receiver sent without a token or the network. Run with

    python3 -m unittest discover -s tools/crash-receiver -p test_receiver.py
"""

from __future__ import annotations

import json
import socket
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import receiver

CRASH_TOKEN = "test-token"


class FakeGitHub:
    """The two issue endpoints the receiver uses, recording every call."""

    def __init__(self) -> None:
        self.requests: list[dict] = []
        self.issues: list[dict] = []
        self.labels: list[dict] = []
        self._lock = threading.Lock()
        github = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def do_POST(self) -> None:  # noqa: N802 - http.server's spelling
                length = int(self.headers.get("Content-Length", "0"))
                body = json.loads(self.rfile.read(length) or b"{}")
                with github._lock:
                    github.requests.append({"path": self.path, "payload": body})
                    if self.path.endswith("/labels"):
                        github.labels.append(body)
                        answer = {"name": body.get("name")}
                        status = 201
                    elif self.path.endswith("/issues"):
                        github.issues.append(body)
                        answer = {"number": len(github.issues)}
                        status = 201
                    else:
                        answer = {"id": len(github.requests)}
                        status = 201
                payload = json.dumps(answer).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def log_message(self, *_args) -> None:
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()

    def comments(self) -> list[dict]:
        return [item for item in self.requests if item["path"].endswith("/comments")]


def bug_bundle(fingerprint: str = "fedcba9876543210") -> dict:
    """One report of the shape the motivating defect produced."""
    return {
        "schema_version": 1,
        "fingerprint": fingerprint,
        "origin": "ingame-menu",
        "description": "mission is completed, standing on the Sherwood spinning wooden sign, no return message",
        "note": "every character is there",
        "captured_at": "20261119T101112Z",
        "build": {
            "version": "0.1.0+0123456789ab",
            "git_rev": "0123456789abcdef0123456789abcdef01234567",
            "target": "x86_64-unknown-linux-gnu",
            "profile": "release",
            "release": True,
            "replay_schema_version": 52,
            "runtime_contract_hash": "deadbeefdeadbeef",
        },
        "session": {
            "kind": "host",
            "session_id": "aabbccdd",
            "transport": {
                "local_seat": 0,
                "expected_players": 2,
                "reconnecting": False,
            },
            "mission": {
                "profile_id": 12,
                "title": "The Ambush",
                "filename": "Emb01_FoA_EC",
                "map": "Sherwood",
            },
            "tick": {"timeline_frame": 4321, "simulation_tick": 4320},
            "players": [
                {
                    "seat": 0,
                    "connected": True,
                    "nickname": "host",
                    "build_rev": {"available": False, "reason": "no wire field"},
                },
                {"seat": 1, "connected": True, "nickname": "guest"},
            ],
            "desync": {
                "current_state_hash": "0123456789abcdef",
                "last_compared_hash_frame": 4300,
                "desync_count": 0,
            },
        },
        "game": {
            "operation": "level_in_progress",
            "script": {
                "script_name": "Emb01_FoA_EC",
                "instance_counts": {"actors": 41, "zones": 3},
                "global": {
                    "class": "StartUp",
                    "members": {"mission_won": 0, "objectives_done": 2},
                },
            },
            "terminal": {"active": False},
            "presentation": {"pause_menu": False, "console_visible": False},
        },
        "world": {
            "player_characters": [{"entity": 7, "position_map": [1.0, 2.0]}],
            "alive": {"count": 4, "predicate": "player characters with life_points > 0"},
        },
        "files": {
            "saves": [{"file": "Savegame_002.json", "bytes": 2048}],
            "replay": [{"file": "run.rhrec.jsonl", "bytes": 4096}],
        },
        "notes": [],
    }


def bundle(fingerprint: str = "0123456789abcdef") -> dict:
    return {
        "schema_version": 1,
        "fingerprint": fingerprint,
        "message": "index out of bounds: the len is 3 but the index is 7",
        "location": "crates/robin_rs/src/game_session/runtime.rs:42:9",
        "backtrace": "   0: robin_rs::game_session::runtime::tick\n   1: robin\n",
        "version": "0.1.0+0123456789ab",
        "engine_commit": "0123456789abcdef0123456789abcdef01234567",
        "platform": "linux",
        "arch": "x86_64",
        "install_home": "/home/player/.local/share/robin_hood",
        "replay_path": None,
        "captured_at": "20261119T101112Z",
        "state": {"frame": 133, "mission": "Dem_Lei_MP"},
        "run_log_tail": "INFO run log → /home/player/.local/share/robin_hood/logs/robin-1.log\n",
        "events_tail": '{"seq":1,"kind":"mission_started"}\n',
        "notes": [],
    }


class ReceiverFixture(unittest.TestCase):
    """A live receiver over a fake GitHub; the tests are in its two subclasses."""

    def setUp(self) -> None:
        self.github = FakeGitHub()
        self.addCleanup(self.github.close)
        self.state = tempfile.TemporaryDirectory()
        self.addCleanup(self.state.cleanup)
        self.config = receiver.Config(
            crash_token=CRASH_TOKEN,
            github_token="github-token",
            github_repo="DataHoarder6/robin-hood-the-legend-of-sherwood-remake",
            state_dir=Path(self.state.name),
            github_api=self.github.url,
            max_body_bytes=4096,
        )
        self.server = receiver.make_server(self.config, "127.0.0.1", 0)
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}/crash"
        self.bug_url = f"http://127.0.0.1:{self.server.server_address[1]}/bug-report"

    def post(self, body: dict | bytes, token: str | None = CRASH_TOKEN) -> tuple[int, dict]:
        return self.post_to(self.url, body, token)

    def post_bug(self, body: dict | bytes, token: str | None = CRASH_TOKEN) -> tuple[int, dict]:
        return self.post_to(self.bug_url, body, token)

    def post_to(
        self, url: str, body: dict | bytes, token: str | None = CRASH_TOKEN
    ) -> tuple[int, dict]:
        raw = body if isinstance(body, bytes) else json.dumps(body).encode()
        request = urllib.request.Request(url, data=raw, method="POST")
        request.add_header("Content-Type", "application/json")
        if token is not None:
            request.add_header("Authorization", f"Bearer {token}")
        try:
            with urllib.request.urlopen(request, timeout=10) as reply:
                return reply.status, json.loads(reply.read() or b"{}")
        except urllib.error.HTTPError as error:
            try:
                return error.code, {"body": error.read().decode()}
            finally:
                error.close()

    def stored_bundles(self, fingerprint: str = "0123456789abcdef") -> list[Path]:
        directory = Path(self.state.name) / "bundles" / fingerprint
        return sorted(directory.glob("*.json")) if directory.exists() else []

    def stored_bug_bundles(self) -> list[Path]:
        return sorted((Path(self.state.name) / "bug-bundles").glob("*.json"))

    def download(self, bundle_id: str, token: str | None = CRASH_TOKEN) -> tuple[int, bytes]:
        request = urllib.request.Request(f"{self.url.rsplit('/', 1)[0]}/bug-reports/{bundle_id}")
        if token is not None:
            request.add_header("Authorization", f"Bearer {token}")
        try:
            with urllib.request.urlopen(request, timeout=10) as reply:
                return reply.status, reply.read()
        except urllib.error.HTTPError as error:
            try:
                return error.code, error.read()
            finally:
                error.close()

class CrashTests(ReceiverFixture):
    def test_upload_without_a_token_is_refused_and_stores_nothing(self) -> None:
        status, _ = self.post(bundle(), token=None)
        self.assertEqual(status, 401)
        status, _ = self.post(bundle(), token="wrong-token")
        self.assertEqual(status, 401)
        self.assertEqual(self.stored_bundles(), [])
        self.assertEqual(self.github.requests, [])

    def test_an_oversized_body_is_refused_before_it_is_read(self) -> None:
        # A raw request, so the declared length is exactly what the receiver
        # sees; urllib would recompute Content-Length from the body it sends.
        host, port = self.server.server_address[:2]
        request = (
            f"POST /crash HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            f"Authorization: Bearer {CRASH_TOKEN}\r\n"
            f"Content-Type: application/json\r\n"
            f"Content-Length: {self.config.max_body_bytes + 1}\r\n"
            f"\r\n"
        ).encode()
        with socket.create_connection((host, port), timeout=10) as connection:
            connection.sendall(request + b"x")
            status_line = connection.makefile("rb").readline()
        self.assertIn(b" 413 ", status_line)
        self.assertEqual(self.stored_bundles(), [])
        self.assertEqual(self.github.requests, [])

    def test_a_first_report_stores_the_bundle_and_creates_one_issue(self) -> None:
        status, answer = self.post(bundle())
        self.assertEqual(status, 202)
        self.assertEqual(answer["issue"], 1)
        self.assertEqual(answer["occurrences"], 1)

        stored = self.stored_bundles()
        self.assertEqual(len(stored), 1, "the bundle is persisted")
        self.assertEqual(json.loads(stored[0].read_text()), bundle())

        self.assertEqual(len(self.github.issues), 1)
        issue = self.github.issues[0]
        self.assertIn("index out of bounds", issue["title"])
        for expected in (
            "0123456789abcdef",
            "0.1.0+0123456789ab",
            "linux",
            "runtime.rs:42:9",
            "game_session::runtime::tick",
            "robin-1.log",
            '"frame": 133',
            stored[0].as_posix(),
        ):
            self.assertIn(expected, issue["body"], f"{expected} is in the issue body")
        self.assertEqual(self.github.comments(), [])

    def test_a_repeat_fingerprint_comments_with_the_occurrence_count(self) -> None:
        self.assertEqual(self.post(bundle())[0], 202)
        status, answer = self.post(bundle())
        self.assertEqual(status, 202)
        self.assertEqual(answer["occurrences"], 2)
        self.assertEqual(answer["issue"], 1)

        self.assertEqual(len(self.github.issues), 1, "no second issue for one crash")
        comments = self.github.comments()
        self.assertEqual(len(comments), 1)
        self.assertTrue(comments[0]["path"].endswith("/issues/1/comments"))
        self.assertIn("Occurrence 2", comments[0]["payload"]["body"])
        self.assertEqual(len(self.stored_bundles()), 2, "both occurrences are kept")

    def test_a_body_without_a_fingerprint_is_rejected(self) -> None:
        broken = bundle()
        del broken["location"]
        del broken["fingerprint"]
        status, _ = self.post(broken)
        self.assertEqual(status, 400)
        self.assertEqual(self.stored_bundles(), [])

    def test_a_malformed_fingerprint_is_rejected(self) -> None:
        status, _ = self.post(bundle(fingerprint="../../etc/passwd"))
        self.assertEqual(status, 400)
        self.assertEqual(list(Path(self.state.name).glob("bundles/*")), [])

    def test_a_github_outage_keeps_the_bundle(self) -> None:
        self.github.close()
        status, _ = self.post(bundle())
        self.assertEqual(status, 502)
        self.assertEqual(len(self.stored_bundles()), 1)


class BugReportTests(ReceiverFixture):
    """The voluntary report: its own store, its own labels, its own downloads."""

    def test_a_first_bug_report_files_one_labelled_issue(self) -> None:
        status, answer = self.post_bug(bug_bundle())
        self.assertEqual(status, 202)
        self.assertEqual(answer["issue"], 1)
        self.assertEqual(answer["occurrences"], 1)

        stored = self.stored_bug_bundles()
        self.assertEqual(len(stored), 1, "the bundle is persisted under a random id")
        self.assertEqual(stored[0].stem, answer["bundle"])
        self.assertEqual(json.loads(stored[0].read_text()), bug_bundle())

        self.assertEqual(len(self.github.issues), 1)
        issue = self.github.issues[0]
        self.assertEqual(issue["labels"], ["bug", "bug-report"])
        self.assertIn("standing on the Sherwood spinning wooden sign", issue["title"])
        for expected in (
            "### Reproduction facts",
            "`aabbccdd`",
            "host",
            "`Emb01_FoA_EC`",
            "`4321`",
            "`level_in_progress`",
            "seat 1 guest (connected)",
            "`Savegame_002.json`",
            "`run.rhrec.jsonl`",
            '"mission_won": 0',
            f"`{answer['bundle']}`",
        ):
            self.assertIn(expected, issue["body"], f"{expected} is in the issue body")
        self.assertEqual(self.github.comments(), [])

    def test_labels_are_created_once_before_the_first_issue(self) -> None:
        self.assertEqual(self.post_bug(bug_bundle())[0], 202)
        self.assertEqual([label["name"] for label in self.github.labels], ["bug", "bug-report"])
        self.assertEqual(self.post_bug(bug_bundle(fingerprint="0000000000000001"))[0], 202)
        self.assertEqual(len(self.github.labels), 2, "already-created labels are not retried")

    def test_a_repeat_fingerprint_comments_and_names_its_own_bundle(self) -> None:
        first = self.post_bug(bug_bundle())[1]
        status, second = self.post_bug(bug_bundle())
        self.assertEqual(status, 202)
        self.assertEqual(second["issue"], first["issue"])
        self.assertEqual(second["occurrences"], 2)
        self.assertNotEqual(second["bundle"], first["bundle"])

        self.assertEqual(len(self.github.issues), 1, "one issue per stuck state")
        comments = self.github.comments()
        self.assertEqual(len(comments), 1)
        self.assertTrue(comments[0]["path"].endswith(f"/issues/{first['issue']}/comments"))
        self.assertIn("occurrence 2", comments[0]["payload"]["body"])
        self.assertIn(f"`{second['bundle']}`", comments[0]["payload"]["body"])
        self.assertEqual(len(self.stored_bug_bundles()), 2, "both seats' bundles are kept")

    def test_a_bug_bundle_is_downloadable_only_with_the_token(self) -> None:
        answer = self.post_bug(bug_bundle())[1]
        status, body = self.download(answer["bundle"])
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(body), bug_bundle())
        self.assertEqual(self.download(answer["bundle"], token=None)[0], 401)
        self.assertEqual(self.download(answer["bundle"], token="wrong-token")[0], 401)
        self.assertEqual(self.download("0123456789abcdef")[0], 404)

    def test_both_seats_reports_are_grouped_by_session(self) -> None:
        self.assertEqual(self.post_bug(bug_bundle())[0], 202)
        self.assertEqual(self.post_bug(bug_bundle(fingerprint="0000000000000002"))[0], 202)
        host, port = self.server.server_address[:2]
        request = urllib.request.Request(
            f"{self.url.rsplit('/', 1)[0]}/bug-reports/by-session/aabbccdd"
        )
        request.add_header("Authorization", f"Bearer {CRASH_TOKEN}")
        with urllib.request.urlopen(request, timeout=10) as reply:
            grouped = json.loads(reply.read())
        self.assertEqual(grouped["session"], "aabbccdd")
        self.assertEqual(len(grouped["reports"]), 2)
        self.assertEqual([entry["seat"] for entry in grouped["reports"]], [0, 0])

    def test_a_bug_report_without_a_description_is_rejected(self) -> None:
        broken = bug_bundle()
        del broken["description"]
        status, _ = self.post_bug(broken)
        self.assertEqual(status, 400)
        status, _ = self.post_bug(bug_bundle() | {"description": "   "})
        self.assertEqual(status, 400)
        self.assertEqual(self.stored_bug_bundles(), [])
        self.assertEqual(self.github.requests, [])

    def test_a_bug_report_without_a_token_is_refused_and_stores_nothing(self) -> None:
        status, _ = self.post_bug(bug_bundle(), token=None)
        self.assertEqual(status, 401)
        self.assertEqual(self.stored_bug_bundles(), [])
        self.assertEqual(self.github.requests, [])

    def test_an_oversized_bug_report_is_refused_before_it_is_read(self) -> None:
        host, port = self.server.server_address[:2]
        request = (
            f"POST /bug-report HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            f"Authorization: Bearer {CRASH_TOKEN}\r\n"
            f"Content-Type: application/json\r\n"
            f"Content-Length: {self.config.max_body_bytes + 1}\r\n"
            f"\r\n"
        ).encode()
        with socket.create_connection((host, port), timeout=10) as connection:
            connection.sendall(request + b"x")
            status_line = connection.makefile("rb").readline()
        self.assertIn(b" 413 ", status_line)
        self.assertEqual(self.stored_bug_bundles(), [])
        self.assertEqual(self.github.requests, [])

    def test_a_github_outage_keeps_the_bug_bundle(self) -> None:
        self.github.close()
        status, _ = self.post_bug(bug_bundle())
        self.assertEqual(status, 502)
        self.assertEqual(len(self.stored_bug_bundles()), 1)


if __name__ == "__main__":
    unittest.main()
