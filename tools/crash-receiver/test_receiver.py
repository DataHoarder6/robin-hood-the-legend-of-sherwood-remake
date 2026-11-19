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
        self._lock = threading.Lock()
        github = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def do_POST(self) -> None:  # noqa: N802 - http.server's spelling
                length = int(self.headers.get("Content-Length", "0"))
                body = json.loads(self.rfile.read(length) or b"{}")
                with github._lock:
                    github.requests.append({"path": self.path, "payload": body})
                    if self.path.endswith("/issues"):
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


class ReceiverTests(unittest.TestCase):
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

    def post(self, body: dict | bytes, token: str | None = CRASH_TOKEN) -> tuple[int, dict]:
        raw = body if isinstance(body, bytes) else json.dumps(body).encode()
        request = urllib.request.Request(self.url, data=raw, method="POST")
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


if __name__ == "__main__":
    unittest.main()
