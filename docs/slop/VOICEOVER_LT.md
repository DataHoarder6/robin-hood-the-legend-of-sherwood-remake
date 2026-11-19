# Lithuanian voice-over

Feasibility of a Lithuanian (`lt-LT`) dub, scoped to the first two missions:
`H01_Lin_VL` ("Robin's Godfather", level code `HA`) and `H02` (code `HB`).

**Verdict: plausible, and the two hard problems are already solved in this
engine.** Speech playback, per-locale voice packs, and language-independent
speech timing all exist. The work is a locale entry, a font switch, one
narration playback hook, a voice pack, and a text pipeline. The cost is
dominated by writing and checking Lithuanian text, not by code.

## What already exists

| Capability | Where | State |
| --- | --- | --- |
| Bark playback (exclamations) | `crates/robin_rs/src/sound/exclamations.rs`, `crates/robin_engine/src/sound.rs` | Wired, 1,362 clips |
| Cutscene dialogue: portrait + subtitle + voice | `crates/robin_rs/src/ingame_menu/dialogue.rs`, `crates/robin_rs/src/game_session/modal_state.rs`, `crates/robin_rs/src/sound.rs` | Wired, 166 clips |
| Language packs, Options → Language, `has_voice`, English voice fallback | `crates/robin_rs/src/localization.rs`, `crates/robin_rs/src/ingame_menu/language.rs` | 18 locales, **no `lt-LT`** |
| Speech timing independent of installed audio | `crates/robin_engine/src/audio_durations.rs`, `docs/NEW_FEATURES.md` | Done — see below |
| Narrator / spoken briefing / spoken debrief | — | **Does not exist** |

The timing question is settled by design: `audio_durations.rs` is engine-owned
English timing and rejects any other locale; `docs/NEW_FEATURES.md` states that
"localized recordings play to their natural end while simulation completes
speech on canonical frames". A Lithuanian clip may be any length. Nothing is
time-stretched, and recorded replays stay valid under any voice pack.

## What the two missions contain

`H01_Lin_VL` is the campaign's first mission: no prerequisites, Robin only,
Lincoln (`DATA/Levels/H01_Lin_VL.rhm`). Its level code is `HA`, derived from the
profile id `0x4148` byte-by-byte (`crates/robin_assets/src/res_descr.rs`).

- **`HA` has no dialogue at all.** No `DLG_HA_*` clips exist in any locale, and
  `RHLevelHA.red` records `dialogue_count = 0`. Mission 1 is narration and
  tutorial text, not conversation.
- Mission 1 text: 34 entries in `Level.res` — 23 popup paragraphs (storybook
  narration dissolving into tutorial voice), description, win/lose debrief, six
  short briefings.
- Mission 1 barks: 13 voice banks, 620 clips, 15.75 min (English, see below).
- **`HB` has 20 dialogue clips, 138.15 s**, which exercises the existing
  dialogue path with no engine work.

Whole game for scale: 166 dialogue clips / 17.2 min / 15,392 chars, plus 1,362
barks / 37.1 min. `Level.res` holds 1,501 strings / 146,579 chars in total.

## Scope decisions

- **Dialogue and narration are voiced; barks stay English.** `sounds/exclamations/*`
  is an optional English-fallback key, so a Lithuanian pack may omit it. Barks
  are vocalisations with no script — they would have to be re-performed, not
  translated.
- **A real `lt-LT` locale pack, not a `mods/` overlay.** An overlay shadows files
  and never appears in the language menu; it would hide the thing being tested.
- **A two-mission pack is shippable without translating the whole game.**
  `text/*` is a required-locale key with no English fallback
  (`crates/robin_util/src/asset_fs.rs`), so every referenced text asset must
  exist in the pack. Discovery is laxer than that: `validate_loose_pack`
  (`crates/robin_rs/src/localization.rs`) only requires a parseable
  `Data/Text/Level.res` with 32 or more strings in one menu table. The pack is
  therefore built *structurally complete, partially translated*: `HA` and `HB`
  entries translated, every other id and dialogue clip carried over verbatim in
  English. Nothing fails; missions outside the two read English.
- **Port-owned UI strings stay English for now.** `crates/robin_rs/src/localization/data/`
  has no `lt` catalog. Lithuanian text over English menus is a known gap, not a
  surprise.
- **Lithuanian name forms** (Robinas Hudas, Šervudas, Notingamas), with
  `pronunciation.toml` holding each fix once.

## Voice supply

`lt-LT` on Google Chirp 3 HD: **30 voices, 16 male / 14 female**, all shared
Chirp 3 roster names (`Charon`, `Puck`, `Fenrir`, `Orus`, `Schedar`, …). Free
allowance **1M characters/month**, which is roughly 40× the whole game's text.
A billing account with a card is required and overage bills automatically at
$30/1M, so the pipeline counts characters and refuses to submit past a set
budget.

`lt-LT` supports rate but **not pitch, SSML phonemes, or custom
pronunciations**. A mispronounced name can only be fixed by respelling the
input, which is why every line carries two strings: the correct orthography for
the subtitle and the respelled text sent to the synthesiser.

Fallbacks, in order: Piper `lt_LT-reginute1` (free, offline, CC-BY-4.0, female
only), then Azure F0 (`lt-LT-LeonasNeural`, `lt-LT-OnaNeural`; 500k
chars/month, hard-capped so it cannot bill). No free Lithuanian male voice
exists outside a hosted service.

## Engine work

1. `LANGUAGE_DEFINITIONS` (`crates/robin_rs/src/localization.rs`) gains `lt-LT`.
   The table is hard-coded; a pack is not discovered without an entry.
2. `locale_prefers_truetype` gains `lt`, so ą č ę ė į š ų ū ž render from a
   TrueType face. The Latin bitmap fonts are untested for these glyphs; the
   mechanism already serves ja/zh/ko/th/ru/pl/cs.
3. **Narration playback** — the only new feature. Mission 1's popup, briefing
   and debrief text has no voice today. The hook is a text-id → audio path
   resolved beside the existing popup display, mirroring `play_dialog`.

## Pipeline

One tool, `tools/voiceover-lt` (not yet created), driven by a checked-in table:

```
line id | mission | display text (lt) | spoken text (lt, respelled) | voice | provenance
```

It translates (machine draft), synthesises, trims and normalises, writes the
audio tree, and rewrites `Data/Text/Level.res` through
`ResourceManager::write_to_res_bytes` (`crates/robin_assets/src/resource_manager.rs`),
carrying every untranslated id over from the retail table. It reports per-line
character counts and durations. `pronunciation.toml` feeds the spoken column. The
cast table maps character → Chirp voice; pitch offsets exist only for the Piper
fallback, since 30 voices make voice-per-character the default.

## Cost

Mission 1 and 2 are a few thousand characters: **zero cost** inside the free
allowance. The whole game's 21k spoken characters plus a complete 146k-character
text translation also fits one month's allowance. What costs is the ear pass on
pronunciation and the translation quality, both human time.

## Risks

- **Chirp 3 has no pronunciation control on `lt-LT`.** Respelling is the only
  lever, and Lithuanian stress is free — a wrong stress on a name is audible and
  unfixable at the API.
- **Machine-translated dialogue reads like translated dialogue.** Accepted for
  these two missions, recorded per line so a human pass can find the weak ones.
- **No narrator exists**, so mission 1's voice is a new feature and its quality
  is judged against nothing in the retail game.

## Implementation result (2026-11-20)

Engine work landed: `lt-LT` in `LANGUAGE_DEFINITIONS`, `lt` in
`locale_prefers_truetype`, a narration lane that never sets `has_dialog`, and a
one-shot narrator hook on the popup modal. Content tooling landed:
`splice_text_tables` (TEXT-only `.res` rewrite, pictures untouched),
`translate_level_res`, and `scripts/voiceover/` (extract, synth, pack).

**Measured scope.** `H01` has 32 text entries / 7,212 characters; `H02` has 55
entries / 6,751 characters, of which 20 are the cutscene sentences. Total
87 lines / 13,963 characters — 1.4% of Chirp 3 HD's 1,000,000-character monthly
allowance ($0.42 at list price). Both line tables are checked in under
`scripts/voiceover/lines/`.

**Ruling — narration naming.** An earlier version keyed narration by a
pack-level WAVE table. Popup text ids restart in every mission, so two missions
would collide on the same index. Narration is now a path convention
`Data/Text/Narration/<text table>_<text id>.ogg`, probed with
`ResourceManager::try_exists`; no archive node is added. Cost if wrong: a pack
built for the old convention is silent, and the probe is the only thing that
finds that out.

**Not done — the content itself.** The line tables still carry the English
text in `display`; they are extraction skeletons, not a Lithuanian script.
No synthesis has run: it needs `GOOGLE_TTS_API_KEY`, which is the one input the
pipeline cannot supply. Consequently no pack was assembled and no in-game
verification was performed. To finish: translate `display` for 87 lines
(keeping the correct orthography; `pronunciation.toml` respells names for the
synthesiser), then

```sh
# To re-extract a line table, pass the descriptor's level code (H01→HA, H02→HB):
# python3 scripts/voiceover/extract.py --dump level.json \
#   --red <datadir>/DATA/Text/RHLevelHA.red --mission H01 --level-code HA \
#   --output scripts/voiceover/lines/h01.toml
GOOGLE_TTS_API_KEY=... just voiceover-synth
just voiceover-pack --retail <datadir> --dump <dump_res JSON> \
    --tool target/debug/translate_level_res --out build/pack
```

and install `build/pack/` as the `1063` locale root. `just voiceover-synth
--dry-run` is the zero-cost way to re-measure after translation.

**Known scope limits, unchanged:** barks stay English (the engine's English
fallback), port-owned UI strings stay English, and the debrief has no audio
lane yet — its text is translated but not narrated.
