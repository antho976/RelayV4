# Setting up Unreal for the Relay plugin

The plugin works in two tiers. Everything in the first tier needs nothing but the project checkout.

## Tier 1 — offline (no editor)

- **Skills and rules** reach every agent as soon as the plugin is on.
- **`ue_project_info`, `ue_log`** read files in the checkout.
- **`ue_build`** needs the engine installed. It finds it from the `.uproject`'s `EngineAssociation`:
  - `~/.config/Epic/UnrealEngine/Install.ini` (Linux) or
    `~/Library/Application Support/Epic/UnrealEngine/Install.ini` (macOS), where launcher and source
    builds register themselves;
  - a source tree the project sits inside;
  - common install folders (`~/UnrealEngine`, `~/UnrealEngine-<version>`, `/opt/UnrealEngine`,
    `/Users/Shared/Epic Games/UE_<version>`, `C:/Program Files/Epic Games/UE_<version>`).

  If none match, set **`UE_ROOT`** to the folder that contains `Engine/` in the environment Relay's
  engine is started from.

UnrealBuildTool refuses to build the editor target while the editor has Live Coding active. Agents
are told to close the editor first or to ask you to press **Ctrl+Alt+F11** (Live Coding) instead.

## Tier 2 — the live editor

The live tools (`ue_python`, `ue_call`, `ue_property`, `ue_search_assets`, `ue_level_actors`,
`ue_console`) talk to the editor over its Remote Control web server.

1. **Enable three editor plugins** — *Remote Control API*, *Python Editor Script Plugin* and
   *Editor Scripting Utilities* (Edit → Plugins). An agent can do this for you:
   `ue_setup_check` with `fix: true` adds them to the `.uproject`. Restart the editor afterwards.
2. **Start the web server.** Either run `WebControl.StartServer` in the editor console each session,
   or turn on the auto-start option under *Project Settings → Plugins → Remote Control*. The default
   endpoint is `http://127.0.0.1:30010`.
3. **Allow remote Python.** Recent engine versions gate Python execution over Remote Control behind
   a setting in *Project Settings → Plugins → Remote Control*. If `ue_setup_check` reports that the
   editor answers but remote Python fails, that setting is the usual cause.
4. **Keep the editor responsive in the background.** Turn off *Editor Preferences → General →
   Performance → Use Less CPU when in Background*. Otherwise the editor barely ticks while you are
   in another window, and animation previews capture poses late.
5. **Optional:** a different port or host goes in **`UE_REMOTE_CONTROL_URL`**, and a passphrase, if
   you configured one, in **`UE_REMOTE_CONTROL_PASSPHRASE`**.

Run `ue_setup_check` (or ask an agent to) at any time: it lists what is missing and what to do.

## Which checkout the editor opens

Open the project's **main checkout** in the editor. With the plugin on, new agents start there too,
and the live tools refuse to act when the editor has a different copy of the project open.

## Security

The Remote Control server can run arbitrary editor Python. Keep it bound to localhost (the
default) and do not expose port 30010 on a shared network.

## Environment variables

| Variable | Meaning |
| --- | --- |
| `UE_PROJECT` | Path of the `.uproject` to use instead of searching the checkout |
| `UE_ROOT` | Engine folder (the one holding `Engine/`) |
| `UE_REMOTE_CONTROL_URL` | Remote Control endpoint, default `http://127.0.0.1:30010` |
| `UE_REMOTE_CONTROL_PASSPHRASE` | Sent as the `Passphrase` header |

Agents inherit these from the Relay engine's environment.
