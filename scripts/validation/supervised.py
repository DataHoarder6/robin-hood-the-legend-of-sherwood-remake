"""Supervise harness children so a cancelled driver cannot orphan them.

A process launched with `start_new_session` survives the death of the agent
that started it: its parent becomes init, and nothing remembers which run owned
it. That is how a headless browser host kept burning six cores after its agent
was cancelled.

The only reliable owner is the OS. Each child therefore runs inside a transient
systemd user unit with:

* `RuntimeMaxSec` — a hard upper bound, even if the driver is SIGKILLed;
* `KillMode=control-group` — stopping the unit stops every fork and exec it
  made, including Chrome's renderers and Xvfb's children;
* `--collect` — the unit disappears when it is done, so evidence is not
  polluted with stale units.

The driver still stops each unit explicitly in `finally`: the TTL is the
backstop for a dead driver, not the normal way a scenario ends.
"""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess


class SupervisionUnavailable(RuntimeError):
    """The host cannot give a child an OS-level owner; do not run unsupervised."""


def require_supervisor():
    """Fail before any process is started if the OS cannot supervise it."""
    missing = [name for name in ("systemd-run", "systemctl") if shutil.which(name) is None]
    if missing:
        raise SupervisionUnavailable(
            f"process supervision requires {', '.join(missing)} on PATH; "
            "an unsupervised harness can orphan a game host"
        )


def unit_name(scope: str) -> str:
    """A unique, systemd-safe unit name for one child of one scenario run.

    `scope` must identify the run, the peer and the launch. The digest keeps two
    runs with the same readable label from colliding; the readable part makes a
    `systemctl --user status rh-e2e-...` useful during an investigation.
    """
    readable = re.sub(r"[^A-Za-z0-9_.-]+", "-", scope).strip("-.")[:42] or "child"
    digest = hashlib.sha256(scope.encode("utf-8")).hexdigest()[:12]
    return f"rh-e2e-{readable}-{digest}"


def command(argv, *, unit: str, budget_s: int, env: dict[str, str] | None = None) -> list[str]:
    """Wrap `argv` in a transient user unit with a hard runtime budget."""
    if budget_s <= 0:
        raise ValueError("budget_s must be positive")
    # A scope starts in the caller's namespace and blocks until the child
    # exits, so the driver can track its PID and reach its loopback HTTP
    # server. A service starts in the user manager's namespace: a harness
    # inside `unshare --net` then cannot poll the child's /state at all.
    # `--pipe` and `--wait` are exclusive with `--scope`; stdin/stdout are
    # inherited through Popen, and systemd-run already waits for the scope.
    wrapped = [
        "systemd-run", "--user", "--scope",
        "--unit", unit,
        f"--property=RuntimeMaxSec={int(budget_s)}s",
        "--property=KillMode=control-group",
        "--property=TimeoutStopSec=10s",
        "--collect", "--same-dir",
    ]
    # systemd does not inherit the caller's environment. Pass exactly what the
    # child was going to get: no more, no less. Values are passed as one argv
    # element each, so spaces and newlines do not need shell quoting.
    for key, value in sorted((env or os.environ).items()):
        wrapped.append(f"--setenv={key}={value}")
    return wrapped + ["--", *argv]


def stop(unit: str, *, quiet: bool = True) -> list[str]:
    """Stop a unit and every process in its cgroup; return cleanup failures.

    Bounded, and never raises: one leaked child must not stop the other children
    from being cleaned up. A named failure is attached to the evidence instead.
    """
    failures = []
    for operation in ("stop", "kill"):
        argv = ["systemctl", "--user"]
        if operation == "kill":
            argv += ["kill", "-s", "SIGKILL", unit]
        else:
            argv += ["stop", unit]
        try:
            result = subprocess.run(argv, capture_output=True, text=True, timeout=10, check=False)
            if result.returncode == 0:
                return failures
            # `stop` of an already-finished unit is success in practice; a
            # missing unit means the child exited on its own.
            if result.stderr.strip():
                failures.append(f"{' '.join(argv)}: {result.stderr.strip()}")
        except (OSError, subprocess.TimeoutExpired) as error:
            failures.append(f"{' '.join(argv)}: {error}")
    return failures


def cleanup(children, summary, *, event=None):
    """Stop the units owned by `children`, then reap their wrapper processes.

    `children` is the list already passed to `runtime_evidence.finish_children`;
    each entry must carry a `unit` attribute set at launch. Keeping the two
    operations in one place makes the order explicit: the service first, so
    nothing it forked can reappear, then the wrapper's process group.
    """
    from runtime_evidence import finish_children  # same directory, existing helper

    failures = []
    for child in reversed(children):
        unit = getattr(child, "unit", None)
        if unit is None:
            continue
        failures += [f"{getattr(child, 'name', child.pid)}: {f}" for f in stop(unit)]
        if event is not None:
            event("supervised_stop", unit=unit, failures=failures)
    finish_children(children, summary)
    if failures:
        summary.setdefault("cleanup_errors", []).extend(failures)
