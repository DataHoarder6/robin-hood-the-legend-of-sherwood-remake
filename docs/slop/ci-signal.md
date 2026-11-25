# CI signal: which red means what

Reference for reading `.github/workflows/rust-quality.yml`. The workflow matrix
and the landing gate's path selectors are owned by the workflow and
[`push-gate.sh`](../../scripts/push-gate.sh); this page does not repeat them.
Last verified: 2026-11-25.

## Authority

| Signal | Authority | Command that decides it |
| --- | --- | --- |
| Landing gate | `scripts/remote-check.sh` on the build host inside the pinned devShell | `bash scripts/remote-check.sh <suite>` |
| GitHub Actions `Rust quality` | Evidence only when its environment matches the gate's | the job's failing step in the Actions log |
| Missing external tool | Prerequisite failure, never a red assertion | `bash scripts/quality-preflight.sh <suite>` |

## Tool sources

| Tool | Source for the pinned version |
| --- | --- |
| `cargo-nextest` | `nix eval --raw nixpkgs#cargo-nextest.version`; CI installs that version, the gate gets it from `flake.nix` |
| wabt / binaryen | `scripts/install_pinned_wasm_tools.sh` |
| pnpm / Node | `pnpm/setup` step in the workflow, version 12.3.4 |
| everything else | `flake.nix` devShell |

## Quarantine

| Fact | Owner |
| --- | --- |
| D09 (`H14`/`H28`/`H33` load flakes) is `deferred` | [`DEFECTS.md`](../DEFECTS.md) and issue #75 |
| D11 (H38 discriminator) is `deferred` | [`DEFECTS.md`](../DEFECTS.md) and issue #77 |
| A row stays quarantined until its linked issue closes and its check is green | [`UNATTENDED.md`](../UNATTENDED.md), known-flake policy |

A red gate is not made green by a rerun. Retries classify a flake; only a
`deferred` defect row suppresses it, with the issuing row naming the owner.
