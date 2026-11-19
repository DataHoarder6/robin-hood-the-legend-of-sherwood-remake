# Lithuanian voice-over (missions H01, H02) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a selectable `lt-LT` language pack that narrates mission `H01_Lin_VL` in Lithuanian and dubs mission `H02`'s cutscene dialogue, plus the tooling that produces it.

**Architecture:** The engine already plays speech, switches language packs, and times speech independently of installed audio. This plan adds the missing pieces: a locale entry, a TrueType preference for Lithuanian diacritics, a `TEXT`-only splice writer so `Level.res` can be retranslated without re-encoding its pictures, a narration playback lane for popup text, and a synthesis pipeline that never exceeds the free character allowance.

**Tech Stack:** Rust (workspace crates `robin_rs`, `robin_assets`, `robin_engine`, `robin_modding_tools`), Google Cloud Text-to-Speech (Chirp 3 HD, `lt-LT`), Python 3 for the content pipeline, `ffmpeg` for loudness and container.

**Spec:** `docs/slop/VOICEOVER_LT.md`

## Global Constraints

- Test entry point is `scripts/check-quality.sh SUITE` only; this workspace does not build everywhere, so gate runs go through `scripts/remote-check.sh SUITE` on the build host. Bare `cargo test` is not a gate.
- Commit per task, by pathspec: `git commit -m "…" -- <files>`. Never `git add -A`.
- Never redirect Cargo's target directory; never `git stash`.
- `LANGUAGE_DEFINITIONS[0]` is `en-US` and is the English fallback root — append new entries, never prepend.
- Lithuanian text is UTF-8 in source and TOML; `Level.res` strings are UTF-16LE on disk (the writer encodes).
- Synthesis budget: 1,000,000 Chirp 3 HD characters per calendar month. The pipeline refuses to submit past `--budget` (default 900,000) and records characters spent.
- Every voiced line carries two strings: `display` (correct orthography, shown) and `spoken` (respelled, sent to synthesis). `display` is the only one written to `Level.res`.
- No new fallback paths: a missing narration or dialogue asset is silence, never a substituted file from another locale.

## Review Focus

- A pack directory present but missing `Data/Text/Level.res` — must be ignored at discovery, not crash startup.
- A `Level.res` whose picture nodes must survive the retranslation byte-for-byte — a re-encode would silently alter mission art.
- A text id present in the translation table but absent from the retail table — must fail the build, not invent an id.
- A narration clip that outlives the popup — dismiss must halt it; a second popup must not stack a second clip.
- A synthesis run interrupted midway — must leave a resumable manifest, never a half-written pack that loads.

---

### Task 1: `lt-LT` locale entry and Lithuanian font policy

**Files:**
- Modify: `crates/robin_rs/src/localization.rs:165-262` (`LANGUAGE_DEFINITIONS`), `:692-701` (`locale_prefers_truetype`)
- Modify: `crates/robin_rs/src/localization.rs:1500-1520`, `:1765-1780` (locale-set tests)
- Test: `crates/robin_rs/src/localization.rs` (`mod tests`)

**Interfaces:**
- Consumes: `LanguageDefinition { locale, lcid, native_name, aliases }`, `find_loose_root`, `validate_loose_pack`, `locale_prefers_truetype`.
- Produces: `lt-LT` discoverable from a loose root named `1063`; `locale_prefers_truetype` true for `lt`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn lithuanian_pack_is_discovered_from_its_lcid_root() {
    // tempdir with 1063/Data/Text/Level.res holding MINIMUM_CORE_MENU_STRINGS
    // strings in MENU_TEXT_TABLES[0], per the fixture at
    // loose_pack_validates_level_text_independently_of_loading_picture.
    let files = /* primary path set to the tempdir */;
    let definition = definition_for_locale("lt-LT").expect("lt-LT must be defined");
    assert_eq!(definition.lcid, "1063");
    assert!(find_loose_root(definition, &files).is_some());
    assert!(locale_prefers_truetype(&files));
    // a pack directory without Data/Text/Level.res is not a pack
    std::fs::remove_file(root.join("Text/Level.res")).unwrap();
    assert!(find_loose_root(definition, &files).is_none());
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p robin_rs --lib localization::tests::lithuanian_pack_is_discovered_from_its_lcid_root`
Expected: FAIL — index out of bounds / `locale_prefers_truetype` false.

- [ ] **Step 3: Append the entry and add `lt` to the font list**

Append after the `pl-PL` entry:

```rust
    LanguageDefinition {
        locale: "lt-LT",
        lcid: "1063",
        native_name: "Lietuvių",
        aliases: &["lt-LT", "lt_LT", "lithuanian", "lt"],
    },
```

Add `"lt"` to the `matches!` list in `locale_prefers_truetype`.

- [ ] **Step 4: Update the tests that pin the locale set**

`every_shipped_language_has_a_complete_feature40_catalogue` must NOT gain `lt-LT` (no catalogue is written). `unshipped_locales_fall_back_to_english_feature40_text` gains `"lt-LT"`. `font_policy_uses_each_readers_selected_language_not_its_root_spelling` gains an `lt-LT` → truetype case.

- [ ] **Step 5: Run the suite**

Run: `cargo test -p robin_rs --lib localization::tests`
Expected: PASS, including the four updated tests.

- [ ] **Step 6: Commit**

```bash
git commit -m "feat(i18n): add lt-LT locale and Lithuanian TrueType font policy" -- crates/robin_rs/src/localization.rs
```

---

### Task 2: String-table setter on `ResourceManager`

**Files:**
- Modify: `crates/robin_assets/src/resource_manager.rs` (beside `get_strings` at `:1045`, `strings_raw` at `:1204`)
- Test: `crates/robin_assets/src/resource_manager.rs` (`mod tests`)

**Interfaces:**
- Consumes: private `data.strings: HashMap<ResourceId, Vec<String>>`, `has_string_resource`.
- Produces:
  - `pub fn set_strings(&mut self, id: ResourceId, strings: Vec<String>) -> Result<()>` — errors when `!has_string_resource(id)`; replaces the vector under the same id, so the node set and id are unchanged.
  - `pub fn set_string(&mut self, id: ResourceId, sub_id: usize, value: String) -> Result<()>` — errors when the sub-id is out of range.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn set_string_replaces_in_place_without_changing_table_shape() {
    // Write a synthetic SRES with one TEXT table (3 strings) to a tempfile and
    // load it with ResourceManager::legacy_tool().attach_resource_file(path),
    // the constructor every existing resource_manager test uses.
    let mut mgr = /* as above */;
    mgr.set_string(1_000_507, 1, "Lietuvių".to_owned()).unwrap();
    assert_eq!(mgr.get_string(1_000_507, 1).unwrap(), "Lietuvių");
    assert_eq!(mgr.get_string_count(1_000_507).unwrap(), 3);
    assert!(mgr.set_string(1_000_507, 3, "x".to_owned()).is_err());
    assert!(mgr.set_strings(999, vec![]).is_err());
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p robin_assets --lib resource_manager::tests::set_string_replaces_in_place_without_changing_table_shape`
Expected: FAIL — no such method.

- [ ] **Step 3: Implement both setters**

`set_strings` checks `has_string_resource(id)` and inserts into `self.data.strings`; `set_string` delegates to `get_strings` for the bound check then mutates the entry.

- [ ] **Step 4: Run the assets suite**

Run: `cargo test -p robin_assets --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(assets): add ResourceManager string-table setters" -- crates/robin_assets/src/resource_manager.rs
```

---

### Task 3: `TEXT`-only splice writer for `.res`

**Files:**
- Create: `crates/robin_assets/src/resource_manager/splice.rs`
- Modify: `crates/robin_assets/src/resource_manager.rs` (`mod splice;` + re-export)
- Test: `crates/robin_assets/src/resource_manager/splice.rs` (`mod tests`)

**Interfaces:**
- Consumes: `ResourceFileEntry { file_offset, .. }` (payload start, recorded by `original::load_file_resource_v100`), `ResourceId = i32`.
- Produces:

```rust
/// Rewrite the payloads of the named TEXT nodes in `bytes`, leaving every other
/// byte of the archive untouched. Node count, node order and all non-TEXT
/// payloads are preserved verbatim.
pub fn splice_text_tables(
    bytes: &[u8],
    tables: &BTreeMap<ResourceId, Vec<String>>,
) -> Result<Vec<u8>>;
```

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn splice_preserves_every_non_text_byte() {
    // synthetic SRES: TEXT 1, PIC 2 (2x2 Rgb16), TEXT 3
    let original = /* hand-built bytes */;
    let mut tables = BTreeMap::new();
    tables.insert(1, vec!["a".into(), "labas".into()]);
    let out = splice_text_tables(&original, &tables).unwrap();
    // PIC payload and both node headers are byte-identical
    assert_eq!(pic_payload(&out), pic_payload(&original));
    // the rewritten table parses back with the new strings
    let mut mgr = load_bytes(&out); // helper: write to a tempfile, attach_resource_file
    assert_eq!(mgr.get_string(1, 1).unwrap(), "labas");
    // table 3, untouched, is unchanged
    assert_eq!(mgr.get_string(3, 0).unwrap(), original_string_3);
}

#[test]
fn splice_rejects_a_table_id_absent_from_the_archive() {
    let mut tables = BTreeMap::new();
    tables.insert(42, vec!["x".into()]);
    assert!(splice_text_tables(&original, &tables).is_err());
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p robin_assets --lib resource_manager::splice`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement the splice**

Parse the archive once to collect `(id, payload_start)` for every node, sorted by offset; derive each node's end from the next node's start (last node ends at end-of-buffer). For each requested id, encode `[flags u32 = 0][count u16][len u16][utf16 code units]` exactly as `write_to_res_bytes` does for `TEXT`, and substitute the span. Copy everything else with `extend_from_slice`. Error when a requested id is not a `TEXT` node in the archive.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p robin_assets --lib resource_manager`
Expected: PASS.

- [ ] **Step 5: Add an ignored real-file round-trip test**

```rust
#[test]
#[ignore = "requires ROBINHOOD_DATA_DIR; see docs/TESTING.md"]
fn real_level_res_splices_to_a_parseable_archive_with_identical_pictures() {
    // load Data/Text/Level.res, splice an identity translation (every table
    // mapped to its current strings), re-parse, assert every picture resource
    // id and payload matches, and every string table is unchanged.
}
```

- [ ] **Step 6: Commit**

```bash
git commit -m "feat(assets): splice TEXT payloads into an existing .res byte stream" -- crates/robin_assets/src/resource_manager.rs crates/robin_assets/src/resource_manager/splice.rs
```

---

### Task 4: `translate_level_res` tool

**Files:**
- Create: `crates/robin_modding_tools/src/bin/translate_level_res.rs`
- Modify: `crates/robin_modding_tools/Cargo.toml` (`[[bin]]`, `required-features = ["cli"]`)
- Test: `crates/robin_modding_tools/tests/cli.rs` (`TOOLS` array at `:3-8`, plus a behaviour test)

**Interfaces:**
- Consumes: `splice_text_tables`, `ResourceManager::attach_resource_file`, `get_strings`.
- Produces: a bin that reads `--input <Level.res> --translations <table.toml> --output <dir>` and writes a structurally complete `Level.res` with translated ids replaced and every other id carried over verbatim.

- [ ] **Step 1: Write the failing CLI test**

```rust
#[test]
fn translate_level_res_replaces_listed_ids_and_carries_the_rest() {
    // tempdir: synthetic Level.res with TEXT 1 and TEXT 2
    // table.toml: [[line]] id = 1000001, index = 0, text = "Labas"
    // run the bin, re-parse the output:
    //   id 1000001 index 0 == "Labas"; id 1000002 unchanged
}

#[test]
fn translate_level_res_rejects_an_unknown_id() {
    // table names 1099999; assert exit failure and empty stdout
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p robin_modding_tools --test cli`
Expected: FAIL — `CARGO_BIN_EXE_translate_level_res` undefined.

- [ ] **Step 3: Implement the bin**

Copy `dump_res.rs`'s shape: `#![deny(clippy::print_stdout, clippy::print_stderr)]`, clap derive `Args`, `tracing_subscriber` to stderr, data on stdout, `ExitCode::FAILURE` on error. Read the translation table with `serde` + `toml`, group rows by table id, apply `set_string`, call `splice_text_tables` on the original bytes, write the file. Fail on any id absent from the archive.

- [ ] **Step 4: Run the tools suite**

Run: `cargo test -p robin_modding_tools`
Expected: PASS, including `--help` and unknown-flag exit-2 contracts for the new bin.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(tools): translate_level_res writes a partially translated Level.res" -- crates/robin_modding_tools/Cargo.toml crates/robin_modding_tools/src/bin/translate_level_res.rs crates/robin_modding_tools/tests/cli.rs
```

---

### Task 5: Narration playback lane

**Files:**
- Modify: `crates/robin_rs/src/sound.rs` (beside `play_dialog` at `:1147`), `crates/robin_rs/src/sound/channels.rs:171-179` (finished edge)
- Modify: `crates/robin_rs/src/ingame_menu/popup_scroll.rs` (`PopupScrollItem`, `tick`)
- Modify: `crates/robin_rs/src/game_session/modal_state.rs:832-900` (`start_active_popup_scroll_batch`)
- Modify: `crates/robin_assets/src/resource_manager.rs` (constant `NARRATION_WAVE_TABLE: ResourceId = 1_000_600`, verified unused in retail `Level.res`)
- Test: `crates/robin_rs/src/ingame_menu/popup_scroll.rs` (`mod tests`)

**Interfaces:**
- Consumes: `SoundManager::play_dialog`'s pattern, `AudioBackend::play_music`/`halt_music`, `ResourceManager::get_sample`, `has_resource`.
- Produces:
  - `SoundManager::play_narration(&mut self, file_path: &str, backend: &mut dyn AudioBackend)`, `close_narration(&mut self, backend)`, `is_narration_finished(&self) -> bool`.
  - `PopupScrollItem { narration_path: Option<String>, .. }` — resolved in the batch builder from `res.get_sample(NARRATION_WAVE_TABLE, text_id as usize)` when `res.has_resource(NARRATION_WAVE_TABLE)`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn popup_narration_starts_once_and_stops_on_dismiss() {
    let mut state = /* PopupScrollModalState with one item carrying narration_path */;
    let mut backend = ReplayAudioProbe::default();
    state.tick(/* ScreenAudio with backend */);
    assert_eq!(backend.played, ["narration.ogg"]);
    state.tick(/* second tick, same item */);
    assert_eq!(backend.played.len(), 1, "narration must not restart per tick");
    state.tick(/* dismissal key */);
    assert_eq!(backend.halted, 1);
}

#[test]
fn popup_without_a_narration_clip_stays_silent() {
    // narration_path None: backend.played is empty, dismissal still works
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p robin_rs --lib ingame_menu::popup_scroll::tests`
Expected: FAIL — no `narration_path` field.

- [ ] **Step 3: Implement the lane**

Mirror `play_dialog` with its own `has_narration`/`narration_finished` runtime flags so a popup never touches `has_dialog`. Start it once per item, gated by a `narration_started: bool` on the state, at the top of `tick` where `ScreenAudio` is already in hand; halt it on every dismissal path (`popup_scroll.rs:329-343`).

- [ ] **Step 4: Run the client suite**

Run: `cargo test -p robin_rs --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(sound): narrate popup text from the pack's narration table" -- crates/robin_rs/src/sound.rs crates/robin_rs/src/sound/channels.rs crates/robin_rs/src/ingame_menu/popup_scroll.rs crates/robin_rs/src/game_session/modal_state.rs crates/robin_assets/src/resource_manager.rs
```

---

### Task 6: Synthesis and pack-assembly pipeline

**Files:**
- Create: `scripts/voiceover/lines/h01.toml`, `scripts/voiceover/lines/h02.toml` (id, display, spoken, voice, provenance)
- Create: `scripts/voiceover/pronunciation.toml`, `scripts/voiceover/cast.toml`
- Create: `scripts/voiceover/synth.py`, `scripts/voiceover/pack.py`
- Modify: `justfile` (recipes `voiceover-synth`, `voiceover-pack`)

**Interfaces:**
- Consumes: `translate_level_res` (Task 4), Google Cloud TTS REST `v1beta1/text:synthesize` with `lt-LT-Chirp3-HD-*`.
- Produces: a pack directory `<out>/1063/Data/{Text/Level.res,Text/Narration/*.ogg,Text/Dialogues/*.ogg}` and `scripts/voiceover/spent.json`.

- [ ] **Step 1: Write the failing test**

`synth.py --dry-run` is the testable surface (no network):

```python
def test_dry_run_writes_a_manifest_and_no_audio(tmp_path): ...
def test_dry_run_refuses_a_budget_it_would_exceed(tmp_path): ...
def test_every_line_has_a_spoken_and_display_string(): ...
def test_resume_skips_lines_already_in_the_manifest(tmp_path): ...
```

Run under `pytest scripts/voiceover/tests` if `pytest` is available; otherwise a `unittest` module invoked by the recipe.

- [ ] **Step 2: Run it and watch it fail**

Run: `python3 -m pytest scripts/voiceover/tests -q`
Expected: FAIL — modules absent.

- [ ] **Step 3: Implement `synth.py`**

Read the line tables, apply `pronunciation.toml` to build `spoken` where it is absent, sum characters, refuse past `--budget` (default 900,000, persisted in `spent.json` per calendar month), call the API per line, write Ogg Vorbis 44.1 kHz mono via `ffmpeg`, loudness-normalise to −16 LUFS, and record `{line, chars, duration_ms, voice}` in the manifest. `--dry-run` stops before the first API call.

- [ ] **Step 4: Implement `pack.py`**

Assemble the pack: derive the `translate_level_res` table from `lines/*.toml` (`display` strings, grouped by table id), run it for `HA`/`HB`, copy the retail English dialogue clips for every other mission, write the `NARRATION_WAVE_TABLE` node mapping `text_id → Text/Narration/<id>.ogg`, and copy the Lithuanian clips for `HB` into `Text/Dialogues/`.

- [ ] **Step 5: Verify the pack loads**

Run the game with `--data-dir` pointing at the retail install and the pack mounted, open Options → Language, confirm `Lietuvių` is listed, select it, start `H01_Lin_VL`, confirm the popup narrates in Lithuanian and dismissing halts it.

- [ ] **Step 6: Commit**

```bash
git commit -m "feat(voiceover): synthesis and pack assembly for lt-LT" -- scripts/voiceover justfile
```

---

### Task 7: Documentation

**Files:**
- Modify: `docs/slop/VOICEOVER_LT.md` (results: per-line cost, durations, what broke)
- Modify: `docs/DATADIRS.md:12-30` (language codes), `docs/NEW_FEATURES.md:1664-1682` (language switching)

**Interfaces:** none — this task records measured numbers the pipeline printed, not claims.

- [ ] **Step 1: Record the measured report**

Characters spent, clips produced, wall-clock per stage, every line the ear pass rejected, and the arithmetic for the remaining 28 missions.

- [ ] **Step 2: Update the two reference docs**

Add `1063` to the language-code table; note that `lt-LT` is a port-authored pack, not a retail locale.

- [ ] **Step 3: Commit**

```bash
git commit -m "docs: measured lt-LT voice-over results and language-code entry" -- docs/slop/VOICEOVER_LT.md docs/DATADIRS.md docs/NEW_FEATURES.md
```

---

## Verification

Per task: its suite. Whole branch before landing: `scripts/remote-check.sh client`, `scripts/remote-check.sh assets`, `scripts/remote-check.sh tools`, then `scripts/check-quality.sh format`.
