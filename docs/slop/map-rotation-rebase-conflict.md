# Map rotation and the director-camera pan

Landing `agent/map-rotation` onto an integration branch that carries the
director-camera pan work conflicts in `crates/robin_rs/src/host/viewport.rs`.
Both changes own the same struct fields, constructor and
`advance_director_camera`; neither can be dropped.

Last verified: 2026-11-21.

## Check before rebasing

The integration branch moves; always ask git, not this doc, what conflicts:

```sh
git fetch fork
git merge-tree --write-tree fork/main HEAD
```

## Merge rule

Keep both behaviours. The rotation side owns `camera_angle`, `display_angle`,
`saved_camera_angle` and `last_rotation_ms`; the director side owns
`director_slide`, `cutscene_view`, `DirectorSlide` and
`director_slide_progress`. Add both field sets and initialise both in
`ViewportState::new`.

In `advance_director_camera`, keep the director's pan/jump/restore logic and
add the rotation transition block beside its `cutscene_view` capture:

- On taking the camera: `saved_camera_angle.get_or_insert(camera_angle)`, then
  zero `camera_angle` and `display_angle`. `get_or_insert` makes this safe on
  every frame the director owns the view.
- On release: `saved_camera_angle.take()`, restore both angles and call
  `apply_rotation_constraints()`.

`adopt_director_camera` and the trailing `apply_rotation_constraints()` call
merge without a conflict. No other file conflicts.

The conflict is content-only; it never requires dropping upstream commits.
Do not rebase this branch until 0.3.0 has shipped and the owner asks.
