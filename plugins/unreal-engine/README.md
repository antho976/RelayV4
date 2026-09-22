# Unreal Engine plugin for Relay

Turn this plugin on for a project and every agent Relay launches in it works as an Unreal Engine 5
developer. It is off by default; switch it on per project from the **Plugins** page or from the
plugin key on the project's row in the sidebar. Relay suggests it for any project whose root holds a
`.uproject`.

## What an agent gets

| Piece | Where it lands | When |
| --- | --- | --- |
| 19 Unreal skills | `.claude/skills/unreal-*/` and `.agents/skills/unreal-*/` in the project root and in every agent worktree (git-excluded), plus the provider homes for Codex | The moment you switch the plugin on, including for agents already running |
| Working rules (`instructions.md`) | The agent's session brief, injected into its system prompt | Every start, wake and resume |
| `unreal` MCP server (11 tools) | `.relay/relay.mcp.json` for Claude Code, `--config mcp_servers.unreal.*` for Codex | Every start, wake and resume |

Switching the plugin off removes its skill folders from every checkout and leaves it out of the next
launch. Folders you wrote yourself, or skills your repository checks in, are never touched.

## Skills

| Skill | Covers |
| --- | --- |
| `unreal-fundamentals` | Project layout, modules and targets, UObject/reflection, GC, actors and components, assets and references, config, naming |
| `unreal-cpp` | Specifiers, containers, strings, delegates, timers, logging, subsystems, Live Coding |
| `unreal-blueprints` | C++/Blueprint split, Blueprint kinds, data assets and tables, scripting Blueprints from Python |
| `unreal-gameplay-framework` | GameMode, PlayerController, Pawn/Character, Enhanced Input, cameras, spawning, SaveGame, travel |
| `unreal-game-design` | Pillars, loops, game feel, progression and balancing, GDDs, scoping, playtesting, accessibility |
| `unreal-level-environment` | Blockout, metrics, landscape, foliage, PCG, World Partition, lighting, Nanite, Python level scripting |
| `unreal-narrative` | Story structure, branching dialogue, quests, barks, localization, Sequencer, story state |
| `unreal-ui-umg` | UMG, CommonUI, MVVM, input modes, focus and gamepad navigation, HUDs, menus, UI performance |
| `unreal-animation` | Animation Blueprints, state machines, blend spaces, montages, IK Rig/Retargeter, Control Rig, Motion Matching |
| `unreal-ai` | AIController, Behavior Trees, StateTree, EQS, perception, navigation, Smart Objects |
| `unreal-gas` | Gameplay Ability System: ASC, attributes, effects, abilities, tags, cues |
| `unreal-multiplayer` | Replication, RPCs, ownership, relevancy, prediction, sessions, PIE network testing |
| `unreal-materials-vfx` | Materials, instances, textures, Niagara, material scripting |
| `unreal-audio` | MetaSounds, attenuation, concurrency, mixes, music, dialogue |
| `unreal-editor-automation` | The `unreal` MCP tools and the editor Python API |
| `unreal-performance` | Frame budgets, stat commands, Unreal Insights, CPU/GPU/memory optimization |
| `unreal-build-packaging` | UBT, targets, BuildCookRun, packaging, CI, build errors |
| `unreal-testing-debugging` | Automation and functional tests, logging, visual logger, gameplay debugger |
| `unreal-source-control` | Git LFS, `.gitignore`/`.gitattributes`, locking, One File Per Actor, redirectors |

## Documentation

- [Setup](docs/setup.md) — what to enable in Unreal so the live tools work.
- [MCP tools](docs/mcp-tools.md) — every `unreal` tool, its arguments and what it returns.
- [Agent workflow](docs/agent-workflow.md) — how an agent should approach a game task end to end.
