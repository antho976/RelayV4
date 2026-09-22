# Python level and actor recipes

Snippets for `ue_python` that work on the open level (UE 5.3-5.6). Asset-side recipes
(import, rename, materials, Data Tables, Blueprints) are in `python-recipes.md`. Do not run
these during Play In Editor, and save the level explicitly afterwards.

Shorter snippets below assume:
```python
import unreal
ACTORS = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
LEVELS = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
EDITOR = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem)
if LEVELS.is_in_play_in_editor():
    raise RuntimeError("Stop Play In Editor before scripting the level")
```

## Spawn and arrange actors

```python
import unreal
ACTORS = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
mesh = unreal.EditorAssetLibrary.load_asset("/Game/Props/SM_Crate")
spawned = []
with unreal.ScopedEditorTransaction("Relay: crate grid"):
    for i in range(5):
        for j in range(5):
            loc = unreal.Vector(i * 200.0, j * 200.0, 0.0)
            a = ACTORS.spawn_actor_from_object(mesh, loc, unreal.Rotator(pitch=0, yaw=90 * ((i + j) % 4), roll=0))
            a.set_actor_label(f"Crate_{i}_{j}")
            a.set_folder_path("Props/Crates")
            spawned.append(a)
    light = ACTORS.spawn_actor_from_class(unreal.PointLight, unreal.Vector(400, 400, 300))
    light.point_light_component.set_editor_property("intensity", 8000.0)
unreal.get_editor_subsystem(unreal.LevelEditorSubsystem).save_current_level()
print(len(spawned), "crates spawned")
```
Spawning a Blueprint: `cls = unreal.EditorAssetLibrary.load_blueprint_class("/Game/BP/BP_Door")`
then `ACTORS.spawn_actor_from_class(cls, loc, rot)`.

Find and filter actors:
```python
lights = [a for a in ACTORS.get_all_level_actors() if isinstance(a, unreal.PointLight)]
by_label = [a for a in ACTORS.get_all_level_actors() if a.get_actor_label().startswith("Crate_")]
ACTORS.set_selected_level_actors(by_label)   # show the human what you mean
```

## Set properties on selected actors

```python
import unreal
ACTORS = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
sel = ACTORS.get_selected_level_actors()
if not sel:
    raise RuntimeError("Nothing selected - ask the human to select actors in the viewport")
with unreal.ScopedEditorTransaction("Relay: selected props no shadows"):
    for a in sel:
        for c in a.get_components_by_class(unreal.StaticMeshComponent):
            c.set_editor_property("cast_shadow", False)
        tags = list(a.get_editor_property("tags"))
        if unreal.Name("NoShadow") not in tags:
            tags.append(unreal.Name("NoShadow"))
            a.set_editor_property("tags", tags)
print("updated", [a.get_actor_label() for a in sel])
```
Replace a mesh: `a.static_mesh_component.set_static_mesh(new_mesh)` (on `StaticMeshActor`s).

## Levels, viewport, console

```python
LEVELS = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
LEVELS.load_level("/Game/Maps/Main")          # prompts if the current level is dirty: save first
LEVELS.new_level("/Game/Maps/Test_Sandbox")
world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
print(world.get_path_name())
unreal.SystemLibrary.execute_console_command(world, "stat unit")
EDITOR = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem)
EDITOR.set_level_viewport_camera_info(unreal.Vector(0, -800, 400), unreal.Rotator(pitch=-20, yaw=90, roll=0))
```
Screenshot of the viewport: `ue_console {"command": "HighResShot 1920x1080"}` (written under
`Saved/Screenshots/`).

