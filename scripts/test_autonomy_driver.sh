#!/usr/bin/env bash
# Fixture tests for scripts/autonomy-driver.sh: the run-level orchestrator.
# The batch controller is scripted by a stub so gate/land/bisect outcomes are
# deterministic; run-bounds, autobacklog, contract and quota are real/local.
# No gate, no build host, no network, no timer unit.
set -euo pipefail

source_root=$(cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/robin-autonomy-driver.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

SCRIPTS=$work/scripts
STATE=$work/state
CONTRACT=$work/contract.md
BACKLOG=$work/backlog.md
HEADS=$STATE/heads
BC_SCRIPT=$work/bc.script
BC_LOG=$work/bc.log
RB_LOG=$work/rb.log
RELEASE_LOG=$work/release.log
mkdir -p "$SCRIPTS" "$HEADS" "$work/runs"

pass_count=0
failures=0
pass() { printf 'ok   %s\n' "$1"; pass_count=$((pass_count + 1)); }
fail() {
    printf 'FAIL %s\n' "$1"
    failures=$((failures + 1))
}
expect_status() {
    if [ "$status" -eq "$1" ]; then pass "$2"; else fail "$2 (exit $status, want $1): $output"; fi
}
expect_contains() {
    case $output in
        *"$2"*) pass "$1" ;;
        *) fail "$1 (missing '$2'): $output" ;;
    esac
}
expect_absent() {
    case $output in
        *"$2"*) fail "$1 (unexpected '$2'): $output" ;;
        *) pass "$1" ;;
    esac
}
field() { # RUN FIELD
    sed -n "s/^$2=//p" "$STATE/runs/$1/status" | head -1
}

# A scripted batch controller. Each line is "<stdout>::<exit>"; one line is
# consumed per invocation, so the driver's branch decisions are exact.
cat >"$SCRIPTS/batch-controller.sh" <<'BC'
#!/usr/bin/env bash
if [ "${1:-}" = ready ]; then printf '%s %s\n' ready "$2" >>"$FAKE_BC_LOG"; printf 'ready ok\n'; exit 0; fi
line=$(sed -n '1p' "$FAKE_BC_SCRIPT" || true)
if [ -z "$line" ]; then
    printf 'batch-controller: scenario exhausted for %s\n' "$*" >&2
    exit 99
fi
sed -i '1d' "$FAKE_BC_SCRIPT"
out=${line%%::*}
rc=${line##*::}
printf '%s %s\n' "$1" "$out" >>"$FAKE_BC_LOG"
if [ "$1" = gate ] && [ -n "${FAKE_GATE_SLEEP:-}" ]; then sleep "$FAKE_GATE_SLEEP"; fi
printf '%s\n' "$out"
exit "$rc"
BC
cat >"$SCRIPTS/auto-release.sh" <<'REL'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_RELEASE_LOG"
printf 'RELEASED %s\n' v0.5.0
REL
cat >"$SCRIPTS/run-bounds.sh" <<'RB'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_RB_LOG"
exec bash "$REAL_RUN_BOUNDS" "$@"
RB
cp -- "$source_root/scripts/run-bounds.sh" "$SCRIPTS/run-bounds.real.sh"
cp -- "$source_root/scripts/autobacklog.sh" "$SCRIPTS/autobacklog.sh"
chmod +x "$SCRIPTS/"*.sh
export REAL_RUN_BOUNDS=$SCRIPTS/run-bounds.real.sh
export FAKE_BC_SCRIPT=$BC_SCRIPT FAKE_BC_LOG=$BC_LOG
export FAKE_RB_LOG=$RB_LOG FAKE_RELEASE_LOG=$RELEASE_LOG

# Record the timeout argument the driver actually passes, then run the real one.
mkdir -p "$work/bin"
export REAL_TIMEOUT=$(command -v timeout)
export FAKE_TIMEOUT_LOG=$work/timeout.log
cat >"$work/bin/timeout" <<'TO'
#!/usr/bin/env bash
printf '%s\n' "$1" >>"$FAKE_TIMEOUT_LOG"
exec "$REAL_TIMEOUT" "$@"
TO
chmod +x "$work/bin/timeout"
export PATH="$work/bin:$PATH"

export ROBIN_AUTONOMY_STATE=$STATE
export ROBIN_AUTONOMY_SCRIPTS=$SCRIPTS
export ROBIN_AUTONOMY_CONTRACT=$CONTRACT
export ROBIN_AUTONOMY_ALLOW_BARE=1
export ROBIN_BACKLOG_FILE=$BACKLOG
export ROBIN_ITEM_HEAD_DIR=$HEADS
export ROBIN_BATCH_STATE=$work/batch.state
export ROBIN_BATCH_REPO=$work/repo
export ROBIN_RELEASE_BINARY=$work/robin
export ROBIN_RELEASE_SUMMARY=$work/summary.json

fixture() { # NAME
    : >"$BC_SCRIPT"; : >"$BC_LOG"; : >"$RB_LOG"; : >"$RELEASE_LOG"; : >"$FAKE_TIMEOUT_LOG"
    rm -rf -- "$STATE" "$work/runs" "$BACKLOG"
    mkdir -p "$STATE" "$HEADS" "$work/runs" "$ROBIN_BATCH_STATE"
    printf 'ok\n' >"$STATE/quota"
    printf '{"verdict":"observed","verdicts":{"H23":"observed"}}\n' >"$ROBIN_RELEASE_SUMMARY"
    {
        printf '# Test contract\n\n```text\n'
        printf 'wall_clock_minutes: 240\nmax_batches: 3\nconsecutive_red_gates: 2\n'
        printf 'bisect_no_landing_halts: true\ngate_timeout_secs: 5400\n'
        printf 'cost_cap: none\ncontract_immutable: true\n```\n'
    } >"$CONTRACT"
    printf '# Test backlog\n\n```text\n' >"$BACKLOG"
    printf '```\n' >>"$BACKLOG"
}
item() { # ID SUBJECT
    printf '%s|ready|tests|small|%s|scripts/test_x.sh\n' "$1" "$2" \
        >>"$BACKLOG"
    printf '%s\n' "$(printf '%s' "$1" | sha256sum | cut -c1-40)" >"$HEADS/$1"
}
script_bc() { printf '%s\n' "$@" >"$BC_SCRIPT"; }
run() {
    status=0
    output=$(cd "$work" && bash "$source_root/scripts/autonomy-driver.sh" "$@" 2>&1) || status=$?
}
run_id() { cat "$STATE/run-id" 2>/dev/null || true; }

# 1. Happy path: LANDED, status file, spend meter, guard-edit, read-only attach.
fixture
item P001 'feat: add a fixture'
script_bc 'epoch e001 tree t1 members P001::0' 'gate green t1::0' \
    'land landed::0' 'none::0'
run run
expect_status 0 'a clean run succeeds'
expect_contains 'the iteration reports LANDED' 'LANDED'
expect_contains 'the run completes' 'RUN-COMPLETE'
if grep -qx 5400 "$FAKE_TIMEOUT_LOG"; then
    pass 'the driver uses the contract gate timeout'
else
    fail "the driver did not use 5400: $(cat "$FAKE_TIMEOUT_LOG")"
fi
id=$(run_id)
[ -n "$id" ] && pass 'a run id was recorded' || fail 'no run id was recorded'
[ "$(field "$id" terminal_state)" = LANDED ] && pass 'status records LANDED' || fail 'status lost LANDED'
[ "$(field "$id" run_end)" = RUN-COMPLETE ] && pass 'status records RUN-COMPLETE' || fail 'status lost run end'
[ "$(field "$id" batches)" = 1 ] && pass 'status counts one batch' || fail 'batch count is wrong'
[ "$(field "$id" consecutive_reds)" = 0 ] && pass 'status resets the breaker' || fail 'breaker was not reset'
if [ -s "$STATE/runs/$id/spend.log" ] && grep -q 'step=gate' "$STATE/runs/$id/spend.log"; then
    pass 'the spend meter recorded the gate'
else
    fail 'the spend meter is empty'
fi
if grep -q 'guard-edit' "$RB_LOG"; then pass 'guard-edit is called on a runtime write'; else fail 'guard-edit is dead code'; fi
before=$(stat -c %Y "$STATE/runs/$id/status")
output=''
output=$(cd "$work" && bash "$source_root/scripts/autonomy-driver.sh" attach 2>&1) || status=$?
expect_contains 'attach shows the run id' "$id"
after=$(stat -c %Y "$STATE/runs/$id/status")
[ "$before" = "$after" ] && pass 'attach is read-only' || fail 'attach wrote the status file'

# 2. Circuit breaker: two consecutive red gates halt before a third batch.
fixture
item P001 'feat: one'
item P002 'feat: two'
item P003 'feat: three'
script_bc 'epoch e001 tree t1 members P001::0' 'gate red t1::1' 'bisect quarantined P001::0' \
    'epoch e002 tree t2 members P002::0' 'gate red t2::1'
run run
expect_contains 'the breaker is named' 'circuit-breaker'
id=$(run_id)
[ "$(field "$id" consecutive_reds)" = 2 ] && pass 'status records two reds' || fail 'red count is wrong'
[ "$(field "$id" batches)" = 2 ] && pass 'the run stopped after two batches' || fail 'the run ran a third batch'
if grep -q '^gate ' "$BC_LOG" && [ "$(grep -c '^gate ' "$BC_LOG")" -eq 2 ]; then
    pass 'only two gates were attempted'
else
    fail "gate attempts: $(grep -c '^gate ' "$BC_LOG")"
fi

# 3. bisect-no-landing halts the run.
fixture
item P001 'feat: bad'
script_bc 'epoch e001 tree t1 members P001::0' 'gate red t1::1' \
    'bisect bisect-no-landing::1'
run run
expect_contains 'bisect failure halts the run' 'bisect-no-landing'
id=$(run_id)
[ "$(field "$id" run_end)" = HALTED ] && pass 'status records a halted run' || fail 'status lost the halt'

# 4. Quota exhaustion fails admission closed and never names a provider switch.
fixture
printf 'exhausted\n' >"$STATE/quota"
script_bc 'epoch e001 tree t1 members P001::0'
run run
expect_contains 'quota exhaustion is named' 'quota-exhausted'
expect_contains 'the terminal state is BLOCKED' 'BLOCKED'
expect_absent 'no provider switch is announced' 'provider'
if [ ! -s "$BC_LOG" ]; then pass 'no batch work follows a quota refusal'; else fail 'admission was fail-open'; fi
rm -f -- "$STATE/quota"
run run
expect_contains 'a missing quota signal fails closed' 'quota-exhausted'

# 5. Per-gate timeout fails the batch closed.
fixture
item P001 'feat: slow'
sed -i 's/^gate_timeout_secs: 5400$/gate_timeout_secs: 1/' "$CONTRACT"
script_bc 'epoch e001 tree t1 members P001::0' 'gate green t1::0'
FAKE_GATE_SLEEP=3 run run
expect_contains 'the timeout is named' 'gate-timeout'
expect_contains 'the run halts' 'HALTED'
if grep -qx 1 "$FAKE_TIMEOUT_LOG"; then pass 'the shortened contract timeout was used'; else fail "timeout arg was $(cat "$FAKE_TIMEOUT_LOG")"; fi
id=$(run_id)
[ "$(field "$id" terminal_state)" = FILED ] && pass 'the timed-out batch is FILED' || fail 'timeout did not fail closed'

# 6. Spend is metered without a cap.
fixture
item P001 'feat: metered'
script_bc 'epoch e001 tree t1 members P001::0' 'gate green t1::0' 'land landed::0' 'none::0'
ROBIN_RUN_TOKENS=5000 ROBIN_RUN_COST=9.99 run run
expect_status 0 'a costly run is not capped'
id=$(run_id)
if grep -q 'tokens=5000 cost=9.99' "$STATE/runs/$id/spend.log"; then
    pass 'the meter records tokens and cost'
else
    fail "the meter is wrong: $(cat "$STATE/runs/$id/spend.log")"
fi

# 7. Stop flag ends the run at a batch boundary.
fixture
item P001 'feat: one'
item P002 'feat: two'
script_bc 'epoch e001 tree t1 members P001::0' 'gate green t1::0' 'land landed::0' 'none::0'
# The land stub touches the stop flag after the first landing.
cat >"$SCRIPTS/batch-controller.sh" <<'BC'
#!/usr/bin/env bash
if [ "${1:-}" = ready ]; then printf '%s %s\n' ready "$2" >>"$FAKE_BC_LOG"; printf 'ready ok\n'; exit 0; fi
line=$(sed -n '1p' "$FAKE_BC_SCRIPT" || true)
sed -i '1d' "$FAKE_BC_SCRIPT"
out=${line%%::*}; rc=${line##*::}
printf '%s %s\n' "$1" "$out" >>"$FAKE_BC_LOG"
if [ "$1" = land ]; then touch "$ROBIN_AUTONOMY_STATE/runs/$(cat "$ROBIN_AUTONOMY_STATE/run-id")/stop"; fi
printf '%s\n' "$out"; exit "$rc"
BC
chmod +x "$SCRIPTS/batch-controller.sh"
run run
expect_contains 'the stop is reported' 'stopped'
[ "$(grep -c '^land ' "$BC_LOG")" -eq 1 ] && pass 'stop ended the run after one batch' || fail 'a second batch was landed'

# 8. Resumability from a recorded run id.
fixture
item P001 'feat: one'
script_bc 'epoch e001 tree t1 members P001::0' 'gate green t1::0' 'land landed::0' 'none::0'
cat >"$SCRIPTS/batch-controller.sh" <<'BC'
#!/usr/bin/env bash
if [ "${1:-}" = ready ]; then printf '%s %s\n' ready "$2" >>"$FAKE_BC_LOG"; printf 'ready ok\n'; exit 0; fi
line=$(sed -n '1p' "$FAKE_BC_SCRIPT" || true)
sed -i '1d' "$FAKE_BC_SCRIPT"
out=${line%%::*}; rc=${line##*::}
printf '%s %s\n' "$1" "$out" >>"$FAKE_BC_LOG"
if [ "$1" = land ]; then touch "$ROBIN_AUTONOMY_STATE/runs/$(cat "$ROBIN_AUTONOMY_STATE/run-id")/stop"; fi
printf '%s\n' "$out"; exit "$rc"
BC
chmod +x "$SCRIPTS/batch-controller.sh"
run run
id=$(run_id)
item P002 'feat: two'
: >"$BC_LOG"
script_bc 'epoch e002 tree t2 members P002::0' 'gate green t2::0' 'land landed::0' 'none::0'
# Restore the generic stub and clear the stop flag so the resumed run can go on.
cat >"$SCRIPTS/batch-controller.sh" <<'BC'
#!/usr/bin/env bash
if [ "${1:-}" = ready ]; then printf '%s %s\n' ready "$2" >>"$FAKE_BC_LOG"; printf 'ready ok\n'; exit 0; fi
line=$(sed -n '1p' "$FAKE_BC_SCRIPT" || true)
sed -i '1d' "$FAKE_BC_SCRIPT"
out=${line%%::*}; rc=${line##*::}
printf '%s %s\n' "$1" "$out" >>"$FAKE_BC_LOG"
printf '%s\n' "$out"; exit "$rc"
BC
chmod +x "$SCRIPTS/batch-controller.sh"
rm -f -- "$STATE/runs/$id/stop"
run resume "$id"
expect_contains 'the resume names the run id' "resumed $id"
expect_contains 'the resumed run lands the next item' 'LANDED'
[ "$(run_id)" = "$id" ] && pass 'the run id is unchanged on resume' || fail 'resume created a new run id'

# 9. Release outcomes and NEEDS-DECISION on a major bump.
fixture
item P001 'feat: shippable'
script_bc 'epoch e001 tree t1 members P001::0' 'gate green t1::0' 'land landed::0' 'minor::0'
run run
expect_contains 'a release is reported' 'RELEASED'
id=$(run_id)
[ "$(field "$id" release_outcome)" = RELEASED ] && pass 'status records RELEASED' || fail 'status lost RELEASED'
if [ -s "$RELEASE_LOG" ]; then pass 'auto-release was invoked'; else fail 'auto-release was not invoked'; fi

fixture
item P001 'feat!: breaking'
script_bc 'epoch e001 tree t1 members P001::0' 'gate green t1::0' 'land landed::0' \
    'major-not-allowed::1'
run run
expect_contains 'a major bump needs a decision' 'NEEDS-DECISION'
expect_contains 'the reason is named' 'major-not-allowed'

# 9b. A refused release records the ROLLED-BACK outcome and needs a decision.
fixture
item P001 'feat: refused'
cat >"$SCRIPTS/auto-release.sh" <<'REL'
#!/usr/bin/env bash
printf 'auto-release: tag-exists: v0.5.0 is never moved\n' >&2
exit 1
REL
cat >"$SCRIPTS/appimage-retain.sh" <<'RET'
#!/usr/bin/env bash
printf 'rollback-target robin-0.4.0.AppImage\n'
RET
chmod +x "$SCRIPTS/auto-release.sh" "$SCRIPTS/appimage-retain.sh"
export ROBIN_APPIMAGE_DIR=$work/appimages
script_bc 'epoch e001 tree t1 members P001::0' 'gate green t1::0' 'land landed::0' 'minor::0'
run run
expect_contains 'the rolled-back outcome is emitted' 'ROLLED-BACK'
expect_contains 'a refused release needs a decision' 'NEEDS-DECISION'
id=$(run_id)
[ "$(field "$id" release_outcome)" = ROLLED-BACK ] && pass 'status records ROLLED-BACK' || fail 'status lost ROLLED-BACK'
# Restore the succeeding release stub for subsequent cases.
cat >"$SCRIPTS/auto-release.sh" <<'REL'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_RELEASE_LOG"
printf 'RELEASED %s\n' v0.5.0
REL
chmod +x "$SCRIPTS/auto-release.sh"

# 10. Missing capability and invalid bounds fail closed.
fixture
printf 'P001|ready|tests|small|No head|scripts/test_x.sh\n' >>"$BACKLOG"
script_bc
run run
expect_contains 'a missing item head is BLOCKED' 'missing-item-head'

fixture
printf 'max_batches: 9\n' >"$CONTRACT"
script_bc
run run
expect_contains 'invalid bounds are refused' 'bounds-invalid'
expect_contains 'the bound reason is named' 'max-batches-not-3'

# 11. A bare run is refused with the runbg remedy.
fixture
output=''
status=0
output=$(cd "$work" && env -u ROBIN_AUTONOMY_ALLOW_BARE -u RUNBG_STATE \
    bash "$source_root/scripts/autonomy-driver.sh" run 2>&1) || status=$?
expect_status 2 'a bare driver run is refused'
expect_contains 'the remedy is printed' 'runbg run autonomy -- bash scripts/autonomy-driver.sh'

printf '\n%s checks passed, %s failed\n' "$pass_count" "$failures"
(( failures == 0 ))
