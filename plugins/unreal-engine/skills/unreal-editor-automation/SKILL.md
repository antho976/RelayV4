---
name: unreal-editor-automation
description: Drive the running Unreal Editor through the `unreal` MCP server - setup (Remote Control API, Python Editor Script Plugin, Editor Scripting Utilities, web server on port 30010, remote Python execution), every ue_* tool with example arguments, and the `unreal` Python API (EditorAssetLibrary, EditorActorSubsystem, LevelEditorSubsystem, AssetTools, factories, Asset Registry, transactions, slow tasks, saving). Use whenever you must create, import, rename, move, edit or save .uasset/.umap content, spawn or modify level actors, create materials instances, Data Tables or Blueprints, or query assets, since binary assets cannot be edited as text.
---

# Unreal Editor Automation

Binary assets (`.uasset`, `.umap`) can only be changed by the editor. This skill is how you do it:
Python executed inside the editor through the `unreal` MCP server, or, when that is not
possible, exact click-paths for the human. Read `unreal-fundamentals` first if you are unsure
about packages, object paths or the project layout.

Reference files:
- `reference/python-recipes.md` - read before writing any non-trivial asset script
  (queries, bulk rename/move with redirectors, imports, material instances, Data Tables,
  Blueprints, deleting, saving).
- `reference/python-level-recipes.md` - read before scripting the open level (spawning
  and arranging actors, editing selected actors, loading levels, viewport, console).
- `reference/remote-control-api.md` - read when using `ue_call` / `ue_property`, when an
  object path is rejected, or when debugging the HTTP layer the MCP server sits on.

## 1. Setup (do this once per project)

1. Run `ue_setup_check`. It reports whether `RemoteControl`, `PythonScriptPlugin` and
   `EditorScriptingUtilities` are enabled in the `.uproject` and whether the editor answers.
2. If plugins are missing, run `ue_setup_check {"fix": true}`. This edits the `.uproject`
   (a text file - show the diff to the human). The editor must be restarted to load them.
3. The Remote Control web server must be running on port 30010:
   - One-off: in the editor console (backtick key, or Window > Output Log > Cmd box) run
     `WebControl.StartServer`. `WebControl.StopServer` stops it.
   - Permanent: Edit > Project Settings > Plugins > Remote Control > enable
     **Auto Start Web Server** (the HTTP port setting there defaults to 30010).
   - A packaged or `-game` instance only starts it with the `-RCWebControlEnable` command-line flag.
4. Recent versions (5.3+ in practice) refuse remote Python unless it is allowed: Project
   Settings > Plugins > Remote Control > enable the remote Python execution option (labelled
   along the lines of "Enable Remote Python Execution"; type "Python" in the settings search
   box). Some versions have a similar switch for remote console commands. If the setting is
   saved to a `Config/*.ini`, commit that ini.
5. Run `ue_editor_status`. When it answers, you are live.

If a call is refused (HTTP 4xx, "not allowed", "remote python execution is disabled", or
connection refused), do not retry in a loop. Tell the human exactly which of steps 2-4 is
missing and what to click, then continue with offline work (C++, config) meanwhile.

## 2. The MCP tools

Offline (editor not needed):

| Tool | Use it for | Example |
|---|---|---|
| `ue_project_info` | First call in any project: name, engine version, modules, plugins, targets | `{}` |
| `ue_setup_check` | Is the bridge usable; `fix` adds missing plugins | `{"fix": true}` |
| `ue_build` | Compile C++ (editor closed, or use Live Coding instead) | `{"target": "MyGameEditor", "configuration": "Development"}` |
| `ue_log` | Read `Saved/Logs/<Project>.log` after any script or build | `{"lines": 200, "filter": "Error\|Warning\|LogPython"}` |

Live (editor open, web server running):

| Tool | Use it for | Example |
|---|---|---|
| `ue_editor_status` | Reachability, engine version | `{}` |
| `ue_python` | The workhorse: any asset/level/Blueprint/material edit or complex query | `{"code": "import unreal\nprint(unreal.SystemLibrary.get_engine_version())"}` |
| `ue_search_assets` | Find assets fast without writing Python | `{"query": "Chair", "class_names": ["StaticMesh"], "package_paths": ["/Game/Props"], "limit": 50}` |
| `ue_level_actors` | List actors in the open level (label, class, path, location) | `{"class_filter": "PointLight", "limit": 100}` |
| `ue_property` | Read or write one property on one object by path | `{"object_path": "/Game/Maps/Main.Main:PersistentLevel.PointLight_0.LightComponent0", "property": "Intensity", "value": 5000}` |
| `ue_call` | Call one UFUNCTION by path (static library functions via their CDO) | `{"object_path": "/Script/EditorScriptingUtilities.Default__EditorAssetLibrary", "function": "DoesAssetExist", "parameters": {"AssetPath": "/Game/Maps/Main"}}` |
| `ue_console` | Console commands: `stat unit`, `r.` cvars, `t.MaxFPS 60`, `WebControl.StartServer` | `{"command": "stat unit"}` |

Decision rules:
- Reading one property or calling one function: `ue_property` / `ue_call`. Anything with a
  loop, a condition, asset creation or saving: `ue_python`.
- Get object paths from `ue_level_actors` or `ue_search_assets`; never guess actor names -
  the actor *name* (`StaticMeshActor_12`) differs from the *label* shown in the Outliner.
- After every `ue_python` that mutates, check its returned log lines (or `ue_log` with
  `filter: "LogPython|Error"`) and print a summary from the script itself.

## 3. Python essentials

Everything is in `import unreal`. Python runs on the editor's game thread: while a script
runs, the editor is frozen. Names are snake_case versions of the C++ names (`bCastShadow`
becomes `cast_shadow`, `KismetSystemLibrary` becomes `unreal.SystemLibrary`).

Libraries and subsystems (5.x):
- `unreal.EditorAssetLibrary` - asset CRUD by path: `does_asset_exist`, `load_asset`,
  `list_assets(dir, recursive=True, include_folder=False)`, `duplicate_asset`, `rename_asset`,
  `delete_asset`, `make_directory`, `save_asset`, `save_loaded_asset`, `save_directory`,
  `find_package_referencers_for_asset`, `load_blueprint_class`.
- `unreal.get_editor_subsystem(unreal.EditorActorSubsystem)` - level actors:
  `get_all_level_actors`, `get_selected_level_actors`, `set_selected_level_actors`,
  `spawn_actor_from_class`, `spawn_actor_from_object`, `destroy_actor`, `duplicate_actor`.
- `unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)` - `load_level`, `new_level`,
  `save_current_level`, `save_all_dirty_levels`, `is_in_play_in_editor`, `editor_request_end_play`.
- `unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem)` - `get_editor_world`,
  `get_level_viewport_camera_info`, `set_level_viewport_camera_info`.
- `unreal.AssetToolsHelpers.get_asset_tools()` - `create_asset(name, package_path, class, factory)`,
  `import_asset_tasks`, `rename_assets`, `duplicate_asset`.
- Factories: `MaterialFactoryNew`, `MaterialInstanceConstantFactoryNew`, `BlueprintFactory`
  (`parent_class`), `DataTableFactory` (`struct`), `CSVImportFactory`, `FbxImportUI` (options, not a factory).
- `unreal.AssetRegistryHelpers.get_asset_registry()` - `get_assets(ARFilter)`,
  `get_assets_by_path`, `get_referencers` / `get_dependencies`. Use `class_paths=[unreal.TopLevelAssetPath("/Script/Engine", "StaticMesh")]`
  in `ARFilter` (5.1+; `class_names` is deprecated).
- `unreal.EditorUtilityLibrary.get_selected_assets()` - Content Browser selection.
- `unreal.MaterialEditingLibrary` - material graphs and material instance parameters.
- `unreal.EditorLevelLibrary` is **deprecated since 5.0**. Do not use it in new scripts; its
  functions moved to the three subsystems above.

Loading and objects:
```python
mesh = unreal.load_asset("/Game/Props/SM_Chair")            # package path or object path
cls  = unreal.load_class(None, "/Script/MyGame.MyCharacter") # native class
cdo  = unreal.get_default_object(cls)
obj  = unreal.load_object(None, "/Game/Data/DA_Sword.DA_Sword")
print(mesh.get_path_name(), mesh.get_class().get_name())
```

Properties: use `get_editor_property("name")` / `set_editor_property("name", value)` (or
`set_editor_properties({...})`). They respect edit permissions and fire PreEditChange/
PostEditChange, which marks the object dirty and records it in the active transaction.
Struct values are copies: read, modify, write back.

```python
comp = actor.static_mesh_component
comp.set_editor_property("cast_shadow", False)
loc = actor.get_actor_location(); loc.z += 100.0; actor.set_actor_location(loc, False, False)
```

Transactions, progress and logging:
```python
with unreal.ScopedEditorTransaction("Relay: align props") as trans:
    ...  # every edit in here is one Ctrl+Z for the human
with unreal.ScopedSlowTask(len(items), "Processing") as task:
    task.make_dialog(True)
    for it in items:
        if task.should_cancel():
            break
        task.enter_progress_frame(1, f"Processing {it}")
unreal.log("info"); unreal.log_warning("warn"); unreal.log_error("error")
```
`unreal.Rotator` positional order is `(roll, pitch, yaw)`. Always use keywords:
`unreal.Rotator(pitch=0.0, yaw=90.0, roll=0.0)`.

Saving (nothing is on disk until you save):
- `unreal.EditorAssetLibrary.save_asset("/Game/X/Y", only_if_is_dirty=False)`
- `unreal.EditorAssetLibrary.save_loaded_assets([a, b])` / `save_directory("/Game/X")`
- Levels: `LevelEditorSubsystem.save_current_level()` or `save_all_dirty_levels()`
- Everything dirty: `unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)`
  (maps, content). Prefer targeted saves so you know exactly which files changed.

## 4. Discovering APIs

Never guess a function name. Check it in the live editor:
```python
import unreal
help(unreal.EditorActorSubsystem)                     # full signatures and docs
print([n for n in dir(unreal.MaterialEditingLibrary) if "instance" in n])
print(unreal.StaticMeshActor.__doc__)
print(hasattr(unreal, "BlueprintEditorLibrary"))
```
For offline lookup, the human can enable **Developer Mode** (Editor Preferences > Plugins >
Python; some versions expose it in Project Settings > Plugins > Python) and restart. The
editor then writes a stub, `<Project>/Intermediate/PythonStub/unreal.py`, covering every
exposed class, including the project's own C++ types. Grep that file instead of guessing:
`grep -n "def spawn_actor_from_class" Intermediate/PythonStub/unreal.py`. It is generated and
large; never commit it. Only `UFUNCTION(BlueprintCallable)`/`BlueprintPure` functions and
`UPROPERTY` members with Blueprint or edit visibility are reachable from Python and Remote Control.

## 5. Workflow for any editor change

1. `ue_editor_status`; if down, see section 1 or fall back to human click-paths.
2. Query first (read-only script or `ue_search_assets`/`ue_level_actors`); confirm the
   targets exist and print what will change. For destructive or bulk operations, show the
   human the list before running the mutating script.
3. Mutate inside `ScopedEditorTransaction`, with `ScopedSlowTask` for loops over more than
   a few dozen items. Keep one script per logical step so failures are easy to locate.
4. Save explicitly and print the saved package paths.
5. Check the log (`ue_log {"filter": "LogPython|Error|Warning"}`).
6. Tell the human which `.uasset`/`.umap` files changed (see `unreal-source-control` for
   locking and committing binary files).

A script template that does all of this:
```python
import unreal
eas = unreal.EditorAssetLibrary
changed = []
with unreal.ScopedEditorTransaction("Relay: UI textures use the UI texture group"):
    paths = eas.list_assets("/Game/UI", recursive=True, include_folder=False)
    with unreal.ScopedSlowTask(len(paths), "Updating textures") as task:
        task.make_dialog(True)
        for p in paths:
            if task.should_cancel():
                break
            task.enter_progress_frame(1, p)
            a = eas.load_asset(p)
            if isinstance(a, unreal.Texture2D):
                a.set_editor_property("lod_group", unreal.TextureGroup.TEXTUREGROUP_UI)
                changed.append(a)
eas.save_loaded_assets(changed)
print(f"changed {len(changed)} assets:"); [print(" ", a.get_path_name()) for a in changed]
```

## 6. Recipes (full versions in `reference/python-recipes.md` and `reference/python-level-recipes.md`)

- **Bulk rename / move**: `AssetTools.rename_assets([unreal.AssetRenameData(asset, new_path, new_name)])`,
  then fix redirectors (recipe in reference), then save. Never move files on disk or with git.
- **Material instance**: `create_asset(name, path, unreal.MaterialInstanceConstant,
  unreal.MaterialInstanceConstantFactoryNew())`, then `MaterialEditingLibrary.set_material_instance_parent`
  and `set_material_instance_scalar/vector/texture_parameter_value`, then `update_material_instance`.
- **Import FBX / textures**: `unreal.AssetImportTask` (`filename`, `destination_path`,
  `automated=True`, `replace_existing`, `save`, `options=unreal.FbxImportUI()`) passed to
  `import_asset_tasks`; read back `imported_object_paths`. In 5.5+ FBX goes through Interchange by
  default and some `FbxImportUI` options may be ignored - verify the result.
- **Spawn / arrange actors**: `spawn_actor_from_object(mesh, location, rotation)` creates a
  `StaticMeshActor`; set label with `set_actor_label`, Outliner folder with `set_folder_path`.
- **Selected actors**: `get_selected_level_actors()` then `set_editor_property` in a transaction.
- **Data Table from CSV**: `DataTableFactory` with `struct` set, then
  `unreal.DataTableFunctionLibrary.fill_data_table_from_csv_file(dt, csv_path)`.
- **Blueprint from a C++ parent**: `BlueprintFactory` with `parent_class`, `create_asset(..., unreal.Blueprint, factory)`.
  Keep logic in C++ where possible (`unreal-cpp`, `unreal-blueprints`).

## 7. Safety rules

- Never hand-edit `.uasset`/`.umap`. Never rename/move/delete them with the shell or git;
  use the editor so references and redirectors are handled.
- Wrap every mutation in `unreal.ScopedEditorTransaction`. Undo does not cover file
  operations (delete, rename on disk, save), so confirm those first.
- Save explicitly; unsaved changes are lost when the editor closes, and the human may be
  asked about "unsaved changes" they did not make.
- Long loops need `ScopedSlowTask` with `should_cancel()`. A script that runs for minutes
  freezes the editor and may time out the HTTP call while it keeps running.
- Do not script during Play In Editor. Check
  `unreal.get_editor_subsystem(unreal.LevelEditorSubsystem).is_in_play_in_editor()` first;
  objects under a `UEDPIE_0_` path belong to the PIE world and vanish when play stops.
- Do not `time.sleep` or wait for async work inside one call; split into several calls.
- Do not delete assets that have referencers without showing
  `find_package_referencers_for_asset` output to the human.
- Before editing a map or asset, make sure nobody else (human or agent) has it open or
  locked (`unreal-source-control`). One agent per map.
- The editor must be closed (or Live Coding used) when `ue_build` compiles editor modules;
  do not run Python while a build is replacing DLLs.

## 8. When Python cannot do it

Some operations are not exposed (for example editing arbitrary Blueprint graph nodes or
some editor-mode tools). Then: prefer moving the logic to C++; otherwise give the human
precise steps ("Content Browser > right-click `/Game/Props` > Fix Up Redirectors").
For HTTP details see `reference/remote-control-api.md`.

## Verify your work

- [ ] `ue_editor_status` answered and scripts returned without Python tracebacks.
- [ ] Every mutation was inside a `ScopedEditorTransaction`; loops had `ScopedSlowTask`.
- [ ] Changed assets and levels were saved; the script printed their paths.
- [ ] `ue_log` shows no new `Error` lines from `LogPython`, `LogAssetTools`, `LogBlueprint`.
- [ ] A re-query (`ue_search_assets`, `ue_level_actors`, or a read-only script) confirms the result.
- [ ] The human was told which binary files changed and which config/.uproject files were edited.
