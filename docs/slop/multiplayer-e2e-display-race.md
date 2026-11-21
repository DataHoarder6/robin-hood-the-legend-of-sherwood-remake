# Multiplayer e2e RED: connect-string hang vs display race

Investigated 2026-11-22 against `fork/main` `dd0035a766f6`. Scope: the 0.3.0
blocker reported as "17 selected rows all hang: timed out after 120s waiting
for host publishes its connect string".

## The connect-string hang is already fixed; no regression

`docs/DEFECTS.md` D05 records the class: children moved to systemd **service**
units, which start in the user manager's network namespace, so the driver
(inside `unshare --user --map-root-user --net`) could not reach the host's
loopback `/state`. Evidence at `/tmp/nix-shell.GFL9PO/robin-multiplayer-e2e.t1uY40`
(snapshot `2229c2695`, the commit **before** the fix) shows every row hanging
with `samples/host.jsonl` empty and the host log repeating the first-mission
presentation; the host was a service (`Service runtime: 10min 30s`).

The fix is commit `e134a0113`, already on `fork/main`: `supervised.command()`
uses `systemd-run --user --scope`, so the child stays in the caller's namespace.
`supervised.py` and `multiplayer_e2e.py` are byte-identical between
`e134a0113` and current `fork/main`. A full 17-row run at `e134a0113`
(`/tmp/nix-shell.1oKcvr/robin-multiplayer-e2e.bRRWZx`) is `observed` on every
row. At current HEAD the host connects at ~12 s in every row (verified again in
`/home/admin/e2e-gate-ehang-current`). So the status doc's RED is a stale
pre-fix run, not a regression of D05.

## What still made the suite RED: an X display allocation race

A fresh full run at `dd0035a766f6` (`/home/admin/e2e-gate-ehang-current`) had
16/17 rows `observed`, including **H23** (the D06 intermittency check), but
**H14** hung. Its failure is a different one: `timed out after 210s waiting
for every fault's trigger`. The host sat at frame 1 in `running` with no window
dismissal, and `xvfb.log` said:

```
(EE) Server is already active for display 101
```

`start_display()` reserved a display under a flock but released the lock as
soon as it picked a number, before Xvfb had created `/tmp/.X11-unix/XN`. With
`--jobs 2`, a second scenario scanned an empty socket directory and chose the
same number; one Xvfb then refused to start, its game never got a window, the
story modal was never dismissed, and the frame never reached the fault's
frame-40 trigger. The first failed `briefing_x11.py` call also showed a
transient `BadWindow`, a symptom of the same broken display.

Fix: hold the allocator lock until Xvfb's socket appears (or startup fails).
This serialises a couple of seconds of Xvfb startup, not a whole scenario.
The regression test `scripts/validation/display_allocation_test.py` starts two
`start_display()` calls concurrently with a delayed fake socket; it failed
`[100, 100]` before the fix and passes with distinct numbers after. It runs in
the `tooling` push-gate suite via `scripts/check-quality.sh`.

## Evidence paths

- Pre-fix connect-string hang: `/tmp/nix-shell.GFL9PO/robin-multiplayer-e2e.t1uY40`
- Post-`--scope` green: `/tmp/nix-shell.1oKcvr/robin-multiplayer-e2e.bRRWZx`
- Current HEAD before display fix: `/home/admin/e2e-gate-ehang-current`
- Current HEAD after display fix: `/home/admin/e2e-gate-ehang-fixed`
