# Blender plugin for Relay

Turn this plugin on for a project and every agent Relay launches in it can build, check and ship
game art in Blender. It is off by default; switch it on per project from the **Plugins** page or
the plugin key on the project's row in the sidebar. Relay suggests it for projects with a `.blend`
at the root. It works on its own, and together with the Unreal Engine plugin for the
Blender-to-Unreal handoff.

The tools run Blender in **background mode** (`blender -b`) on the `.blend` files in the agent's
checkout. You do not need Blender open, and nothing appears on your screen.

## What an agent gets

| Piece | Where it lands | When |
| --- | --- | --- |
| 6 Blender skills | `.claude/skills/blender-*/`, `.agents/skills/blender-*/` | As soon as the plugin is on |
| Working rules (`instructions.md`) | The agent's session brief | Every start, wake and resume |
| `blender` MCP server (7 tools) | `.relay/relay.mcp.json` (Claude), `--config mcp_servers.blender.*` (Codex) | Every start, wake and resume |

## Skills

| Skill | Covers |
| --- | --- |
| `blender-fundamentals` | The data model and bpy in background mode, transforms and units, modifiers, the tool workflow |
| `blender-modeling` | Game-ready modeling: budgets, topology, hard surface, modular kits, UVs, LODs, collision |
| `blender-rigging` | Armatures for Unreal, Rigify, weights, props and sockets, fixing skinning |
| `blender-animation` | Actions, loops, root motion, timing, two-character moves, props, baking, export |
| `blender-materials-baking` | PBR for Unreal, texture packing, baking maps with Cycles |
| `blender-to-unreal` | The conventions both sides share, exports, import checks, troubleshooting |

## Tools

| Tool | What it does |
| --- | --- |
| `blender_info` | Lists the art files, or reports what a `.blend` holds |
| `blender_python` | Runs bpy code on a file; can save or save as |
| `blender_render` | Images from named views at chosen frames, returned to the agent |
| `blender_rig_check` | Rig and skin problems before export |
| `blender_anim_inspect` | Sides, grips, clearance, contacts and feet across an action |
| `blender_export` | FBX with Unreal's conventions |
| `blender_to_unreal` | Export, import into the running Unreal editor, and measure what arrived |

See [setup](docs/setup.md) and the [tool reference](docs/mcp-tools.md).
