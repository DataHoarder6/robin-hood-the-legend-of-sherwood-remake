# Map rotation: rebase conflict with the director-camera pan

Read-only investigation, 2026-11-21. This branch (`agent/map-rotation`, HEAD
`2ede03a14`) conflicts with `fork/main` in exactly one file:
`crates/robin_rs/src/host/viewport.rs`.

## Reproduce

`fork/main` moves continuously while 0.3.0 lands, so re-run this before acting:

```sh
git fetch fork
git merge-tree --write-tree fork/main HEAD
```

At `fork/main = dd0035a76` the merge returned rc 1 with one `CONFLICT
(content)` for `viewport.rs`. The overlapping upstream commit is `109b019ec`
("camera: pan from the player view and restore it on cutscene release"), which
rewrote `advance_director_camera` and added `DirectorSlide`,
`director_slide_progress` and `cutscene_view`. The rotation branch adds
`saved_camera_angle` save/restore to the same function and struct.

## Resolution (three hunks)

1. **Struct fields** — keep both sets: upstream's `director_slide`,
   `cutscene_view`, the `DirectorSlide` type and `director_slide_progress`, plus
   rotation's `camera_angle`, `display_angle`, `saved_camera_angle`,
   `last_rotation_ms`. The `std::f32::consts` import is already present on both
   sides.
2. **`ViewportState::new`** — initialise both sets (`None`/`None` and
   `0.0`/`0.0`/`None`/`None`).
3. **`advance_director_camera`** — keep upstream's pan/jump/restore logic
   verbatim; add rotation's `owns_view` transition block alongside upstream's
   `cutscene_view` capture:
   - `after.owns_view`: `saved_camera_angle.get_or_insert(camera_angle)` and
     zero `camera_angle`/`display_angle` (idempotent, unlike upstream's
     transition-only capture).
   - `before.owns_view && !after.owns_view`: `saved_camera_angle.take()` and
     restore both angles through `apply_rotation_constraints()`.

`adopt_director_camera` auto-merges: upstream's body is unchanged there and
rotation's prelude zeroing lands cleanly. The trailing
`apply_rotation_constraints()` call in `advance_director_camera` also
auto-merges.

No other file conflicts. A content conflict like this is resolvable during the
landing rebase; it does not require dropping upstream commits. Do not perform
the rebase until 0.3.0 has shipped and the owner asks for the landing.
