#!/usr/bin/env python3
"""Reproduce the fail -> repeat transition hang with two real processes.

Drives a host and one peer through a scripted mission failure and a host
"restart" decision, then reports whether both peers re-admitted to the
replacement mission or the session froze.

The mission is failed through the debug console (`LOOSE`, the original game's
spelling), not by playing. The restart is performed through the same terminal
debriefing path a click uses: `POST /step-forward` with a typed dismissal whose
result is `Restart`. Nothing here needs a window.

Outcome:
  0  both peers re-admitted to the replacement mission (no hang)
  2  hang reproduced: the host never released the replacement barrier
  1  driver or environment failure (not a game verdict)

Run isolated, as the operator would:

  unshare --user --map-root-user --net python3 scripts/validation/fail_restart_repro.py \
      --binary target/debug/robin --data "$ROBINHOOD_DATA_DIR" \
      --evidence /tmp/fail-restart-repro --snapshot "$(git rev-parse HEAD)"
"""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time
import traceback
import urllib.error
import urllib.request

ANSI = re.compile(r"\x1b\[[0-9;]*m")
BEGIN_SIM = "begin-sim barrier released"
BLOCKED = "blocked by modal "


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--snapshot", required=True,
                        help="source revision the supplied executable was built from")
    parser.add_argument("--allow-shared-net", action="store_true",
                        help="skip the loopback-isolation requirement (debugging only)")
    parser.add_argument("--mission", default="S01_Not_VL",
                        help="mission launched directly; headless cannot navigate menus")
    parser.add_argument("--graphical", action="store_true",
                        help="run a real window under Xvfb; required for terminal debriefing, "
                             "which the headless driver does not present")
    parser.add_argument("--admit-seconds", type=float, default=90.0)
    parser.add_argument("--restart-seconds", type=float, default=90.0)
    args = parser.parse_args()

    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    transcript = (evidence / "events.jsonl").open("w", buffering=1)
    children = []
    started = time.monotonic()
    summary = {
        "snapshot": args.snapshot,
        "binary": str(args.binary),
        "data": str(args.data),
        "observed_modals": [],
        "restart_requested": False,
        "replacement_begin_sim": {"host": False, "peer": False},
        "verdict": "driver_error",
    }

    def event(kind, **fields):
        record = {"elapsed_s": round(time.monotonic() - started, 3), "event": kind, **fields}
        transcript.write(json.dumps(record) + "\n")
        print(json.dumps(record), flush=True)

    def request(port, path, body=None, timeout=15):
        payload = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}{path}", data=payload,
            headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            body = error.read()
            try:
                return json.loads(body)
            except json.JSONDecodeError:
                raise RuntimeError(f"HTTP {error.code}: {body[:400]!r}") from error

    def wait_for(description, predicate, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                value = predicate()
                if value:
                    event("observed", description=description, value=value)
                    return value
            except (OSError, ValueError, TimeoutError):
                pass
            time.sleep(0.1)
        raise RuntimeError(f"timed out waiting for {description}")

    def log(path):
        return ANSI.sub("", path.read_text(errors="replace"))

    def launch(name, port, connect=None):
        root = evidence / ("host" if connect is None else "peer")
        root.mkdir(exist_ok=True)
        for directory in ("save", "config", "cache", "data", "runtime", "cwd"):
            (root / directory).mkdir(exist_ok=True, mode=0o700)
        shutil.copytree(Path(__file__).resolve().parents[2] / "assets/core-datadir",
                        root / "cwd/assets/core-datadir", dirs_exist_ok=True)
        env = os.environ.copy()
        for key in ("ROBINHOOD_OVERLAY_DATA_DIRS", "ROBIN_WAIT_FOR_COMMAND", "WAYLAND_DISPLAY"):
            env.pop(key, None)
        env.update(ROBINHOOD_DATA_DIR=str(args.data),
                   ROBINHOOD_SAVE_DIR=str(root / "save"),
                   XDG_CONFIG_HOME=str(root / "config"),
                   XDG_CACHE_HOME=str(root / "cache"),
                   XDG_DATA_HOME=str(root / "data"),
                   XDG_RUNTIME_DIR=str(root / "runtime"),
                   RUST_LOG="info,robin_rs::game_session=debug,robin_rs::multiplayer=debug")
        if display:
            env["DISPLAY"] = display
            env["WGPU_BACKEND"] = "gl"
        else:
            argv.insert(1, "--headless")
        argv = [str(binary), "--no-sound", "--start-paused", "--mission", args.mission,
                "--mp-nickname", name, "--http-server", str(port),
                "--mp-browser-join-links", "false"]
        if connect is None:
            argv += ["--server", "--mp-expected-players", "2", "--record",
                     str(evidence / "host-recording")]
        else:
            argv += ["--connect", json.dumps(connect)]
        logfile = (evidence / f"{name}.log").open("w")
        process = subprocess.Popen(argv, start_new_session=True, cwd=root / "cwd", env=env,
                                   stdout=logfile, stderr=subprocess.STDOUT)
        children.append(process)
        event("launch", peer=name, pid=process.pid, argv=argv)
        return process, evidence / f"{name}.log"

    def stop_children():
        for process in children:
            if process.poll() is None:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        deadline = time.monotonic() + 10
        for process in children:
            while process.poll() is None and time.monotonic() < deadline:
                time.sleep(0.1)
            if process.poll() is None:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def endpoint_id(path):
        match = re.search(r"hosting on iroh endpoint ([0-9a-f]{64})", log(path))
        return match.group(1) if match else None

    def begin_sim_count(path):
        return log(path).count(BEGIN_SIM)

    def step(port, dismissals=None):
        body = {"n": 1, "synchronized_multiplayer": True}
        if dismissals:
            body["dismissals"] = dismissals
        return request(port, "/step-forward", body)

    def modal_kind_from(message):
        if BLOCKED not in message:
            return None
        tail = message.split(BLOCKED, 1)[1]
        end = tail.find("; retry")
        if end < 0:
            return None
        try:
            return json.loads(tail[:end])
        except json.JSONDecodeError:
            return None

    def result_for(kind):
        if "FinalDebriefing" in json.dumps(kind):
            return "Restart"
        return "Completed"

    def state(port):
        return request(port, "/state", timeout=10)

    try:
        binary = args.binary.resolve(strict=True)
        data = args.data.resolve(strict=True)
        display = None
        if args.graphical:
            read_fd, write_fd = os.pipe()
            xvfb_log = (evidence / "xvfb.log").open("w")
            xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0",
                                     "1280x1024x24", "-nolisten", "tcp", "-ac"],
                                    start_new_session=True, pass_fds=(write_fd,),
                                    stdout=xvfb_log, stderr=subprocess.STDOUT)
            children.append(xvfb)
            os.close(write_fd)
            with os.fdopen(read_fd) as handle:
                display = ":" + handle.readline().strip()
            event("display", value=display)

        if not args.allow_shared_net:
            interfaces = json.loads(subprocess.check_output(["ip", "-j", "link", "show"], timeout=10))
            names = sorted(interface["ifname"] for interface in interfaces)
            if names != ["lo"]:
                raise RuntimeError(f"refusing non-isolated network namespace: {names}")
            subprocess.run(["ip", "link", "set", "lo", "up"], check=True, timeout=10)

        host, host_log = launch("host", 7780)
        host_id = wait_for("host endpoint bound", lambda: endpoint_id(host_log), 120)

        inodes = set()
        for descriptor in Path(f"/proc/{host.pid}/fd").iterdir():
            try:
                match = re.fullmatch(r"socket:\[(\d+)\]", os.readlink(descriptor))
            except FileNotFoundError:
                continue
            if match:
                inodes.add(match.group(1))
        ports = set()
        for line in Path(f"/proc/{host.pid}/net/udp").read_text().splitlines()[1:]:
            fields = line.split()
            if fields[9] in inodes:
                ports.add(int(fields[1].split(":")[1], 16))
        if not ports:
            raise RuntimeError("host owns no IPv4 UDP socket")
        connect = {"id": host_id, "addrs": [{"Ip": f"127.0.0.1:{port}"} for port in sorted(ports)]}
        (evidence / "connect.json").write_text(json.dumps(connect, indent=2))
        event("connect", value=connect)

        peer, peer_log = launch("peer", 7781, connect)
        for name, path in (("host", host_log), ("peer", peer_log)):
            wait_for(f"{name} begin-sim", lambda path=path: BEGIN_SIM in log(path), args.admit_seconds)
        summary["admitted"] = True

        for port in (7780, 7781):
            wait_for(f"{port} answers state", lambda port=port: state(port).get("map"), args.admit_seconds)
        event("bootstrap", host=state(7780), peer=state(7781))

        console = request(7780, "/console", {"command": "LOOSE"})
        event("console", response=console)
        if isinstance(console, dict) and "error" in console:
            raise RuntimeError(f"LOOSE rejected: {console}")

        restart_sent = False
        host_before = begin_sim_count(host_log)
        peer_before = begin_sim_count(peer_log)
        deadline = time.monotonic() + args.restart_seconds
        steps = 0
        seen_messages = set()
        while time.monotonic() < deadline and not restart_sent:
            steps += 1
            response = step(7780, None)
            if "error" not in response:
                if steps % 25 == 0:
                    event("progress", steps=steps, host=state(7780), peer=state(7781))
                continue
            message = response["error"]
            kind = modal_kind_from(message)
            if kind is None:
                if message not in seen_messages:
                    seen_messages.add(message)
                    event("step_error", message=message)
                time.sleep(0.25)
                continue
            summary["observed_modals"].append(kind)
            result = result_for(kind)
            event("terminal_modal", kind=kind, result=result, step=steps)
            response = step(7780, [{"kind": kind, "result": result}])
            event("dismissal", response=response)
            if kind and "FinalDebriefing" in json.dumps(kind) and result == "Restart":
                restart_sent = True
                summary["restart_requested"] = True
                # The dismissal is applied by the outer frame, then the host
                # publishes the prepared transition to every connected peer.
                break

        if not restart_sent:
            raise RuntimeError("never reached a final debriefing that could request Restart")

        replacement = {"host": False, "peer": False}
        deadline = time.monotonic() + args.restart_seconds
        while time.monotonic() < deadline:
            if begin_sim_count(host_log) > host_before:
                replacement["host"] = True
            if begin_sim_count(peer_log) > peer_before:
                replacement["peer"] = True
            if all(replacement.values()):
                break
            time.sleep(0.1)
        summary["replacement_begin_sim"] = replacement
        event("replacement", value=replacement)
        summary["host_tail"] = log(host_log)[-4000:]
        summary["peer_tail"] = log(peer_log)[-4000:]

        if all(replacement.values()):
            summary["verdict"] = "recovered"
            return 0
        summary["verdict"] = "hang"
        return 2
    except Exception as error:
        summary["error"] = str(error)
        summary["traceback"] = traceback.format_exc()
        event("failure", error=str(error))
        return 1
    finally:
        stop_children()
        summary["elapsed_s"] = round(time.monotonic() - started, 3)
        (evidence / "summary.json").write_text(json.dumps(summary, indent=2))
        event("summary", summary=summary)
        transcript.close()


if __name__ == "__main__":
    raise SystemExit(main())
