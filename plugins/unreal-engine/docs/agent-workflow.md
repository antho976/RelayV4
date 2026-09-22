# How an agent works on an Unreal game

This is the loop the plugin's instructions and skills teach. It is written for agents; it is also
what a human reviewer should expect to see.

## 1. Orient

1. `ue_project_info` — engine version, modules, targets, maps, whether the project is Blueprint-only.
2. Load `unreal-fundamentals`, then the domain skill(s) for the task.
3. Read the relevant `Source/`, `Config/` and design documents in the repository.
4. `ue_editor_status` if the task touches assets or levels.

## 2. Decide where the change lives

| The change is… | Do it in |
| --- | --- |
| Rules, systems, data structures, performance-critical code | C++ (`unreal-cpp`) |
| Tunable numbers, content lists, dialogue, loot | Data Assets / Data Tables / Curve Tables, authored as CSV or JSON where possible |
| Wiring, one-off level logic, designer-facing behaviour | Blueprints, created and configured through `ue_python`; graph wiring handed to the human with exact steps (`unreal-blueprints`) |
| Asset creation, import, placement, bulk edits | `ue_python` (`unreal-editor-automation`) |
| Engine, rendering, input, project settings | `Config/Default*.ini` |

## 3. Change in small, verifiable steps

- C++: edit → `ue_build` (editor closed) or Live Coding → `ue_log` with `Error|Warning`.
- Editor: one `ue_python` script per logical change, inside `unreal.ScopedEditorTransaction`,
  ending with an explicit save and a printed summary of what changed.
- Keep the game playable after every step; prefer a greybox that works to a polished piece that
  does not.

## 4. Verify

- Build succeeds; the log has no new errors.
- Automation tests pass where the project has them (`unreal-testing-debugging`).
- For gameplay: describe exactly how to test it in PIE, and what the player should see.
- For performance-sensitive work: numbers from `stat unit` / Unreal Insights, not impressions.

## 5. Hand over

List the text files changed, the binary assets created or modified (they need Git LFS and cannot
be merged), any editor steps left for the human, and what to playtest.
