# Fighting mechanics backlog: what actually exists

Last verified: 2026-11-28 against `fork/main` `527df68d2`.

## Question

The frozen [deferred-work backlog](../BACKLOG.md) has a “Fighting mechanics” entry. Where is that alternative or additional fighting logic, and why has it never been implemented?

## Answer

There are two different things behind that question.

### 1. The fighting systems exist

The port already has a substantial combat stack:

- [`crates/robin_engine/src/combat.rs`](../../crates/robin_engine/src/combat.rs) owns engine-independent damage, concussion, knockout, parry, strike and relative-fighting-ability rules.
- [`crates/robin_engine/src/engine/melee/`](../../crates/robin_engine/src/engine/melee/) owns engine-level swordfight AI, strike selection, damage application and combat sequencing.
- [`crates/robin_engine/src/engine/archery.rs`](../../crates/robin_engine/src/engine/archery.rs) owns bow shots, arrows and thrown/fired projectiles.
- [`crates/robin_rs/src/mouse_way.rs`](../../crates/robin_rs/src/mouse_way.rs) recognises the Original A–I sword strokes and nine optional composite single-stroke techniques.
- [NEW_FEATURES.md, “Advanced combat gestures”](../NEW_FEATURES.md) records the shipped optional composites, deterministic quality tiers, guide/coach overlays, and authoritative save/replay/multiplayer state. [SAVE_FORMAT.md](../slop/SAVE_FORMAT.md) records that this state entered native save schema v67.
- Campaign combat options such as `keep_moving_in_combat` and `auto_loot` are in [CAMPAIGN_PURCHASES.md](../slop/CAMPAIGN_PURCHASES.md); per-PC no-friendly-fire flags landed in commits `3b1665dc0`, `25cec51be`, `0634fbc71` and `5df2870b1`.

So “fighting mechanics” is not a wholly missing feature. The original combat model and several extensions are already implemented and gated.

### 2. The backlog entry is an unscoped placeholder

The entry was created by commit `21988f991` on 2026-11-21. Its complete content is:

- Target release: `later patch`.
- Why deferred: postponed behind the 0.3.0 and 0.5.0 priorities.
- No live worktree.

It names no issue, spec, acceptance criteria, owner, test, branch or concrete behavior. A GitHub issue search for fighting/combat/melee/sword returned no dedicated fighting-mechanics issue; the only open match was #113, which is the alternate camera/map POC. No branch named for fighting, melee or combat exists in the checkout.

The entry is therefore not a plan that was repeatedly rejected. It is a frozen label with no definition. It cannot be dispatched until someone says what behavior it means and what failing check would prove it.

## Why it was not implemented

The recorded reason is priority ordering: 0.3.0 and 0.5.0 came first, and the entry was frozen with no live worktree. Since then, other combat work has landed, but none of it claims to satisfy this placeholder. The absence of a worktree is deliberate under the freeze rule, not an abandoned implementation.

## Ruling

Do not describe this as “never going to be implemented”. Describe it as **undefined deferred work**. Either:

1. Replace the backlog entry with one or more scoped issues, each with a concrete behavior and a failing test, or
2. Retire the entry if the shipped combat system and advanced gestures already cover what it meant.

Until one of those happens, there is no missing code to find.
