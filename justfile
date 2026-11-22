# Read-only preflight: exits non-zero on the states that cost a session.
doctor:
    scripts/doctor.sh

# Archive this checkout's drifted main tip, then move main to the integration ref.
reconcile-main:
    scripts/reconcile-main.sh

# Retire the worktrees a merge already finished; `--apply` removes them.
prune-worktrees *args:
    scripts/prune-worktrees.sh {{args}}

# Named gates from scripts/check-quality.sh; `tooling` is the fixture-free one.
check suite="tooling":
    bash scripts/check-quality.sh {{suite}}

# Fixture regression: quality entry points refuse to run on the runtime.
test-no-local-build:
    bash scripts/tests/no-local-build.sh

# Fixture regression: a frozen session with a pending tool call fails the check.
test-backlog-pending:
    bash scripts/tests/backlog-pending-tool.sh

# Build this worktree's dev game on the build host and copy it to the laptop.
ship-dev:
    scripts/ship-dev.sh

# One-shot read-only report of build-host state and the local checkout.
build-status:
    scripts/build-status.sh

# Lithuanian voice-over: measure the batch (add --dry-run to skip the network).
voiceover-synth *args:
    python3 scripts/voiceover/synth.py {{args}}

# Lithuanian voice-over: assemble the 1063/ language pack.
voiceover-pack *args:
    python3 scripts/voiceover/pack.py {{args}}

# Lithuanian voice-over: offline pipeline tests.
voiceover-test:
    python3 -m unittest discover -s scripts/voiceover/tests
