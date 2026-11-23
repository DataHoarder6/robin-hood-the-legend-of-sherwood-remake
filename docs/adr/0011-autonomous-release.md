# 0011 — Autonomous MINOR releases

- **Status:** Accepted
- **Date:** 2026-11-23
- **Related:** [`0002`](0002-finished-work-lands-in-session.md), [`0009`](0009-batched-agent-builds.md), [`0010`](0010-linux-first-unattended-goal.md)
- **Design:** [`docs/superpowers/specs/2026-11-23-autonomous-loop-design.md`](../superpowers/specs/2026-11-23-autonomous-loop-design.md)

## Context

[UNATTENDED.md](../UNATTENDED.md) makes a stable tag human work. The approved
autonomous-loop design gives the bounded loop authority to gate, land, tag and
publish, because work otherwise strands in feature branches and releases wait
indefinitely on a human. The two statements conflict; this ADR is the explicit,
contract-backed deviation the design calls for.

## Decision

The loop may cut a stable release through
[`scripts/auto-release.sh`](../../scripts/auto-release.sh) only when the
committed [run contract](../AUTONOMOUS_RUN_CONTRACT.md) grants release authority.
The script refuses a tag that already exists, a bump that is not MINOR
(`0.x.0`), a breaking commit since the previous tag, and a release whose
[`release-gate.sh`](../../scripts/release-gate.sh) summary does not report
`.verdict == "observed"` and `.verdicts.H23 == "observed"`. On those conditions
it runs `git tag -a`, `git push fork <tag>` and
`gh workflow run native-release.yml --ref <tag>`. The bump and the releasable
decision come from Conventional Commits in the batch, not agent judgment.

The script never edits the machine repo's `?rev=` pin, never runs the seat sync,
never installs and never launches the game. Those steps stay with the owner;
[RELEASES.md](../RELEASES.md) keeps describing them. MAJOR bumps, moving an
existing tag and any ambiguity about the contract are `NEEDS-DECISION`, not
autonomous actions. Two prior known-good AppImages are retained by
[`appimage-retain.sh`](../../scripts/appimage-retain.sh); `robin-pin` and
`previous-good` are owner-owned.

## Consequences

- The loop can publish a low-risk batch without a mid-run question. The human
  still owns MAJOR versions, the machine pin and every seat.
- Release evidence is the same release gate the human path uses; the loop adds
  no weaker test and no second release mechanism.
- The deviation is narrow and testable: every refusal is a named reason in
  [`scripts/test_auto_release.sh`](../../scripts/test_auto_release.sh).
