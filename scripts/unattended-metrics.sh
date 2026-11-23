#!/usr/bin/env bash
# Print the unattended-loop pilot metrics. Each value comes from its source at
# run time; nothing here is a stored report. The journal is on oci-vm, so that is
# the only host this runs on.
#
#   scripts/unattended-metrics.sh [SINCE]   # SINCE defaults to the pilot start
set -euo pipefail

since=${1:-2026-11-22}
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

printf 'metric\tvalue\tsource\n'

journalctl --user -u director-loop.service --since "$since" --no-pager -o cat 2>/dev/null |
    python3 -c '
import json, re, sys

counts = {"LANDED": 0, "FILED": 0, "BLOCKED": 0}
dispatches = {"builder": 0, "reviewer": 0, "verifier": 0}
quota = 0
raw_lines = 0
for line in sys.stdin.buffer.read().decode("utf-8", "replace").splitlines():
    raw_lines += 1
    if "Free usage limit reached" in line or "provider.quota" in line:
        quota += 1
    try:
        record = json.loads(line)
    except (ValueError, TypeError):
        continue
    part = record.get("part")
    if not isinstance(part, dict):
        continue
    kind = record.get("type")
    if kind == "text":
        text = part.get("text", "")
        for state in counts:
            if re.search(r"\*\*" + state + r"\*\*|Terminal state:?\s*\*{0,2}" + state, text):
                counts[state] += 1
    elif kind == "tool_use" and part.get("tool") == "subagent":
        agent = part.get("state", {}).get("input", {}).get("agent")
        if agent in dispatches:
            dispatches[agent] += 1

for state in ("LANDED", "FILED", "BLOCKED"):
    print(f"terminal_{state.lower()}\t{counts[state]}\tdirector journal text markers")
for agent in ("builder", "reviewer", "verifier"):
    print(f"dispatches_{agent}\t{dispatches[agent]}\tdirector journal subagent tool calls")
print("review_iterations\t{}\treviewer dispatches (one round per dispatch)".format(dispatches["reviewer"]))
print(f"quota_failures\t{quota}\tdirector journal quota lines")
print(f"journal_lines\t{raw_lines}\tdirector journal since SINCE")
'

reverts=$(git -C "$repository" log fork/main --since="$since" --grep=revert -i --oneline | wc -l)
printf 'reverts\t%s\tgit log fork/main --grep=revert -i\n' "$reverts"

# Time-to-land, blocked dwell and cost need paired timestamps or per-session
# accounting that the journal does not carry in a machine-readable form. Do not
# invent them: the source must be extended before they can be reported.
printf 'time_to_land\tnot available\tdirector journal has no machine-readable iteration start\n'
printf 'blocked_dwell\tnot available\tdirector journal has no machine-readable requeue event\n'
printf 'cost_per_landed\tnot available\topencode stats is not exposed as a command here\n'
