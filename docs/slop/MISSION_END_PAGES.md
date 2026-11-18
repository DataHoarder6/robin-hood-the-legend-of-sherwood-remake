# Mission-end pages: won, lost and aborted

The original game presents one mission-end page per terminal outcome, and an
abandoned mission is its own outcome, not a lost one. The port stages the same
page through the terminal debriefing flow (`game_session/terminal_debriefing.rs`)
and now selects the original's third title for an interrupted mission.

## Evidence

Retail menu text table, English (GB) `2047/data/Text/Level.res`, id `1000507`
(the table the port loads as `MenuText`; 24–26 are the three mission-end
titles):

| id | text |
| --- | --- |
| 24 | Mission won |
| 25 | Mission lost |
| 26 | Mission aborted |

Mission scripts: the engine calls the mission script's `Finalize(abandoned)`
with `1` when the player quits a live mission. Scripts that offer an abandoned
variant select the lose debriefing table's retreat entry:

```lua
Finalize(abandoned)
    if abandoned == 1 then
        ChooseVictoryDefeatText(1)
    end
end
```

(Decompiled from `DATA/Levels/*.scb` with `disasm_scb --decompile`; e.g.
`Tac18_FoA_EC`, `Emb01_FoA_EC`, `Str01_Lin_EC`, `H04_Lei_VL`.) The same
scripted retreat uses index `1` outside the terminal path (`H04_Lei_VL`
`CheckVictoryCondition`). The index selects a line in the level's win/lose
table named by `RHLevel*.red`; for the lose tables, index 0 narrates a defeat
and index 1 the withdrawal — e.g. `1000356` sub 1 "Robin should have been more
restrained", `1000414` sub 1 "What a blow! But the worst has been avoided…".

The port keeps the same data path: `ChooseVictoryDefeatText` writes
`mission.victory_defeat_id`, and the terminal page reads
`debriefing.lose_text_table_id` at that index. Missions whose scripts do not
select a retreat entry (e.g. `H01_Lin_VL`, empty `Finalize`, index 0) show
their ordinary lose text on abandon, exactly as the data specifies; no page or
line is invented.

## Port behaviour

`QuitMissionRequested` sets `quit_interrupted`; the tick calls
`finalize_mission_script(…, true)` and returns `LevelInterrupted`. The terminal
flow pages that outcome as `MissionEndKind::Aborted`: retail title id 26 on the
mission-state popup and on the debriefing page, lose table for the body. The
dismissal path is unchanged — `DialogResult::Completed` settles to
`SettledDebriefingOutcome::Ok`, the session keeps `LevelInterrupted`, and the
campaign map reopens with the attempt recorded as `Interrupted`.

What would reopen this: a retail mission in which an aborted outcome shows the
*won* table, or a leak of the original's mission-state popup or debriefing
resource ids showing only two title ids (24–25). Neither exists in the shipped
data above.
