# Cross-build replay corpus

Recordings the `parity-cross` suite replays on the native Linux build and on the
Windows release build under Wine. `scripts/check-quality.sh parity-cross` runs
every recording here (plus an extra one named by `PARITY_REPLAY`); each must
reach EOF with the two binaries' per-25-frame state-hash chains identical.

`scripts/validation/parity_check.py` owns the comparison. This directory only
chooses *what* is replayed: a case belongs here when it exercises a recorded
fact the hash chain alone would not stress often enough.

Each case is a mission recording directory: the recorder's `mission.json`
chronology and its chunks. The suite passes the first chunk's path, which the
client resolves through the chronology beside it (the same path a player's
`--replay` accepts).

## Cases

### `sherwood-hq-entry/`

- Mission: `Sherwood` (HQ), 240 simulation frames, replay schema 78.
- Covers: the entry popup `PostInitialize` queues after its own frame's modal
  drain (dismissed on frame 1) and the crouch tutorial popup opened at frame
  151. Playback must reopen both and consume their recorded dismissals in those
  frames; a driver that dismissed a boundary modal without journaling it left
  playback waiting on an acknowledgement that did not exist and orphaned the
  tutorial's (`docs/slop/replay-modal-desync.md`, #45).
- Made by the `robin` of the commit it was recorded on, which must be the
  schema the binaries replay (`REPLAY_SCHEMA_VERSION`); re-record it in the same
  change that moves the schema, or the gate cannot open the recording:

  ```sh
  ROBINHOOD_DATA_DIR=/path/to/robin-hood/game \
    robin --headless --no-sound --mission Sherwood \
      --spec crates/robin_app/tests/fixtures/scenario_spec_sherwood_hq.json \
      --record DIR
  # then move DIR (mission.json and 00000000.rhrec.jsonl) here, renamed
  ```

  `--spec` stops the run after its frame budget (`GameCode::LevelInterrupted`,
  exit 3). The recording is complete and replayable at EOF because the recorder
  flushes every record.

## Re-recording

A replay schema bump, or any change the recorder legitimately reinterprets,
invalidates these files: playback rejects an obsolete schema rather than
upgrading it. Re-record every case with the procedure above on the new build and
replace the file in the same change that bumps the schema — a stale corpus turns
`parity-cross` red for the wrong reason.
