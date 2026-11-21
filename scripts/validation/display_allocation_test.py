"""Concurrent scenarios must never allocate the same X display number.

The process layer runs scenarios in a pool. A display number is reserved under
a flock, but Xvfb creates its socket asynchronously: if the lock is released
as soon as the number is chosen, a second scenario can scan an empty socket
directory and pick the same number. Xvfb then refuses to start ("Server is
already active for display N"), the game on that seat never gets a window, its
story modal is never dismissed, and the scenario hangs waiting for a fault at
frame 40 while its log repeats the first-mission presentation.
"""

import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

import multiplayer_e2e


class DisplayAllocationTests(unittest.TestCase):
    def test_concurrent_starts_reserve_distinct_numbers(self):
        with tempfile.TemporaryDirectory() as tmp:
            sockets = Path(tmp) / "X11-unix"
            sockets.mkdir()
            lock = Path(tmp) / "display.lock"
            results = []

            def fake_popen(argv, **_kwargs):
                number = int(argv[1][1:])
                process = Mock()
                process.poll.return_value = None

                # A real Xvfb publishes its socket a moment after the command
                # starts. Both scenarios can reach this point before either
                # socket exists, which is the race under test.
                def publish():
                    time.sleep(0.1)
                    (sockets / f"X{number}").touch()

                threading.Thread(target=publish, daemon=True).start()
                return process

            def start(index):
                run = Path(tmp) / f"run{index}"
                run.mkdir()
                results.append(multiplayer_e2e.start_display(run, f"scenario-{index}")[1])

            with patch.object(multiplayer_e2e, "X_SOCKET_DIR", sockets), \
                    patch.object(multiplayer_e2e, "display_lock_path", return_value=lock), \
                    patch.object(multiplayer_e2e.subprocess, "Popen", side_effect=fake_popen), \
                    patch.object(multiplayer_e2e.supervised, "command",
                                 side_effect=lambda argv, **_kw: argv), \
                    patch.object(multiplayer_e2e.supervised, "stop", return_value=[]):
                threads = [threading.Thread(target=start, args=(n,)) for n in range(2)]
                for thread in threads:
                    thread.start()
                for thread in threads:
                    thread.join(timeout=20)
                self.assertFalse(any(thread.is_alive() for thread in threads),
                                 "concurrent start_display calls must terminate")

            numbers = [int(display[1:]) for display in results]
            self.assertEqual(len(numbers), 2)
            self.assertEqual(len(set(numbers)), 2,
                             f"two concurrent scenarios must get distinct displays, got {numbers}")


if __name__ == "__main__":
    unittest.main()
