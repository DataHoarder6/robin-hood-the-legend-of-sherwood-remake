# 0014 — Two-tier landing gate: seconds before the push, the rest right after

- **Status:** Accepted
- **Date:** 2026-11-27
- **Related:** [`0009`](0009-batched-agent-builds.md), [`0013`](0013-gate-incremental-without-sccache.md)
- **Evidence:** [`docs/slop/build-time-root-causes.md`](../slop/build-time-root-causes.md)

## Context

After the build-time work a warm crate landing still held the queue for about
six minutes: test builds of `robin_engine` and `robin_rs` (over a minute each),
the threaded wasm client, test runs. Every landing waited for all of it, and
every agent behind it waited too.

## Decision

- **Before the push:** `format`, `lint` (clippy over libraries and binaries
  with the shipping feature set) and `docs`. On a warm `gate-shared` copy these
  take well under a minute.
- **After the push:** `land.sh` starts `scripts/post-land-verify.sh` under
  runbg with every other suite the diff selected (`lint-tests`,
  `client-release`, `engine`, `wasm`, `tooling`, ...). It runs on its own warm
  copy, `post-land`, so it never queues behind the next landing's gate.
- **Stop the line:** a red verification is recorded by `scripts/main-red.sh`.
  While it is red, `push-gate` runs the full set plus the failed suites before
  any push; the landing that passes it clears the record.
- A plain `git push` through the pre-push hook has no post-land runner, so it
  gates the full set.

## Consequences

- `main` can be red for the minutes a verification takes. The next landing
  then pays the full gate, which keeps a red main from compounding.
- Nothing watches a red record except the next landing. `scripts/main-red.sh
  show` prints it; the verification's log is the runbg unit it names.
- CI still runs every suite on each push to `main`.
