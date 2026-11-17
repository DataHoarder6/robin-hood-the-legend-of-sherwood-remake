# Read-only preflight: exits non-zero on the states that cost a session.
doctor:
    scripts/doctor.sh

# Archive this checkout's drifted main tip, then move main to the integration ref.
reconcile-main:
    scripts/reconcile-main.sh

# Named gates from scripts/check-quality.sh; `tooling` is the fixture-free one.
check suite="tooling":
    bash scripts/check-quality.sh {{suite}}

# Build this worktree's dev game on the build host and copy it to the laptop.
ship-dev:
    scripts/ship-dev.sh
