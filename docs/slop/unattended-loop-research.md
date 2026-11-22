# Unattended loop hardening: research evidence

Primary-source record behind the rules in [UNATTENDED.md](../UNATTENDED.md).
Each row is one claim, its source, whether that source is primary or secondary,
and the mitigation adopted here. This doc owns the external claims; the policy
pages own the rules. Reference doc, not a decision record.

Last verified: 2026-11-22.

## Claims and mitigations

| # | Claim | Source | Kind | Mitigation adopted |
| --- | --- | --- | --- | --- |
| 1 | MAST: 14 multi-agent failure modes — 44% system design, 32% inter-agent misalignment, 23.5% verification; step repetition 15.7%, unaware termination 12.4% | [arxiv.org/abs/2503.13657](https://arxiv.org/abs/2503.13657) | Primary (study) | Explicit termination contracts (the three terminal states) plus multi-level verification (suite verdict and origin read-back, not narration); repeat detection in the circuit breaker |
| 2 | METR: 38% algorithmic success but 0 of 15 PRs mergeable, about 42 minutes of human repair each | [metr.org research update](https://metr.org/blog/2025-08-12-research-update-towards-reconciling-slowdown-with-time-horizons) | Primary (study) | The gate and the origin read-back are the only merge authority; a green push exit status is never LANDED |
| 3 | AppWorld: 75.8% of failures were self-reported as success; automated judges reached AUROC at most 0.65 | [arxiv.org/html/2606.09863](https://arxiv.org/html/2606.09863) | Primary (study) | Never trust agent narration: only gate evidence and `git ls-remote` read-back decide a state |
| 4 | Lost in the Middle: long contexts degrade use of information in the middle; context rot accumulates over long sessions | [direct.mit.edu TACL article](https://direct.mit.edu/tacl/article/doi/10.1162/tacl_a_00638/119630/Lost-in-the-Middle-How-Language-Models-Use-Long) | Primary (study) | Fresh-context iterations; one small handoff artifact per iteration instead of a growing shared transcript |
| 5 | OWASP AI Agent Security Cheat Sheet: bound autonomy, least privilege, human oversight for high-impact actions | [OWASP cheat sheet](https://cheatsheetseries.owasp.org/cheatsheets/AI_Agent_Security_Cheat_Sheet.html) | Primary (authoritative guidance) | Hard ceilings (two-attempt breaker), kill switch, least-privilege `edit` denies on authority files |
| 6 | AWS Agentic AI Lens: iterative plan-execute-verify loops and multi-agent coordination create unpredictable cost; cost must be a first-class constraint | [AWS Well-Architected Agentic AI Lens, cost optimization](https://docs.aws.amazon.com/wellarchitected/latest/agentic-ai-lens/cost-optimization.html) | Primary (vendor guidance) | Hard ceiling on attempts plus a cost-per-landed-task pilot metric |
| 7 | OpenHands caps agent steps with `max_iterations` — an iteration ceiling is a normal, not exceptional, control | [OpenHands issue #2121](https://github.com/OpenHands/OpenHands/issues/2121), [SDK iterative refinement](https://docs.openhands.dev/sdk/guides/iterative-refinement) | Primary (project docs/issue) | Two fix attempts per item, then FILED — no third |
| 8 | GitHub Copilot coding agent: an agent cannot approve or merge its own PR; workflows are gated; least privilege applies | [GitHub docs: risks and mitigations](https://docs.github.com/en/copilot/concepts/agents/coding-agent/risks-and-mitigations) | Primary (vendor docs) | The loop never lands its own work without the gate and read-back; risk routing keeps CI/deploy definitions human |
| 9 | Runway software factory: re-check the gate at the exact merged commit, route migrations/secrets/CI/deploy to humans, keep one-call rollback | [runway.com engineering](https://runway.com/news/engineering/building-a-software-factory) | Primary (practitioner report) | Origin read-back at the pushed commit; risk routing to BLOCKED; one-call kill switch on `oci-vm` |
| 10 | mabl across 75 repos: classify flaky tests, then trip a circuit breaker after two consecutive failed auto-fixes | [mabl blog](https://mabl.com/blog/how-we-built-a-system-for-ai-agents-to-ship-real-code-across-75-repos) | Primary (practitioner report) | Two-attempt breaker and the known flake policy (`deferred` row, no indefinite retry) |
| 11 | Pilot metrics worth tracking: tasks attempted/LANDED/FILED/BLOCKED, first-pass gate success, revert rate, review iterations, time-to-land p50/p90, blocked dwell, cost per landed task, false-quarantine count | [Intercom on safe AI approvals](https://www.intercom.com/blog/ai-is-approving-our-pull-requests-heres-how-we-made-it-safe), [Runway](https://runway.com/news/engineering/building-a-software-factory) | Secondary (practitioner reports; the Runway source is primary for its own share) | One-week pilot metrics table with a named command/source for each metric |

## What this record does not do

It does not restate the policy. The rules live in [UNATTENDED.md](../UNATTENDED.md);
the retry budget and quarantine rule live in
[PARALLEL_AGENTS.md](../PARALLEL_AGENTS.md); the known flakes live in
[DEFECTS.md](../DEFECTS.md). External claims are never copied into AGENTS.md or
README.md.
