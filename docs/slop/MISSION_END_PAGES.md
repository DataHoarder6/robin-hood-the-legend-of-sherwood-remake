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
`Tac18_FoA_EC`, `Emb01_FoA_EC`, `Str01_Lin_EC`.) The same
scripted retreat uses index `1` outside the terminal path (`H04_Lei_VL`
`CheckVictoryCondition`). The index selects a line in the level's win/lose
table named by `RHLevel*.red`. In the lose tables index 0 narrates a death or
defeat and index 1 usually a withdrawal — e.g. `1000356` sub 1 "Robin should
have been more restrained", `1000414` sub 1 "What a blow! But the worst has
been avoided…". A few missions use index 1 for the loss of the last PC with a
required skill instead (`1000342`, `1000352`).

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

## The leave / abandon confirmation

Before any of those pages, the rail's ✗ (in-mission `MissionAbandon`, tooltip
id 292) opens the mission-state confirmation. Its question is not fixed: it is
the retail ask for the state the button was pressed in, chosen from the same
`MissionEndKind` that titles the page (`MissionEndKind::leave_prompt_id`).

| state at the press | question id | text |
| --- | --- | --- |
| not won | 238 | "Do you really want to abort the mission?" |
| won (`MissionEndKind::Won`) | 235 | "You've won the mission. Do you want to leave now?" |

The engine's first-time "you may leave now" banner raises the same signal after
a won mission, so it asks 235 with the won title; the ✗ pressed while still
losing asks 238 and titles the popup as the abandon it confirms. Confirming
either one dispatches `QuitMissionRequested` and nothing else; cancelling
closes the popup and the mission keeps running.

What would reopen this: a retail mission in which an aborted outcome shows the
*won* table, or a leak of the original's mission-state popup or debriefing
resource ids showing only two title ids (24–25). Neither exists in the shipped
data above.
