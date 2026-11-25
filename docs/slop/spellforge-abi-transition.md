# Spellforge ABI transition to `87c3b138a0f4`

- **Last verified:** 2026-11-25
- **Decision:** keep both new natives in the shipping `NATIVE_REGISTRY` and move
  the golden; do not hide the harness native behind a test-only feature.

`spellforge_vm_abi()` hashes every `NATIVE_REGISTRY` definition, so adding a
native moves the executable identity. Two commits moved it: `98795d198` added
`SummonReinforcements` and `0949a2966` added `HarnessMarkMissionWon` for the H38
scripted-victory proof. The pinned golden still named the pre-change digest.

The harness native is dispatched by shipping code
(`crates/robin_engine/src/natives/dispatch/script_core.rs`) and asserted by the
process-layer corpus (`scripts/validation/scenarios/H38.json`); moving it behind
a feature would break the H38 proof and change replay identity. The golden in
`crates/robin_spellforge/tests/abi.rs` was therefore updated to the digest the
build actually computes, `spellforge-v1-sha256:87c3b138a0f4…0cb5`.

Refresh the value only from a failing `contracts` check, never by guessing:

```sh
bash scripts/remote-check.sh contracts
```

The test's left-hand value is the computed digest. A transition must land in the
same commit as the registry change; `push-gate.sh` selects `contracts` for ABI
inputs so a stale golden cannot land.
