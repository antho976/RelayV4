# Python Level Scripting (editor, via `ue_python`)

Every snippet runs in the editor with the `ue_python` MCP tool. Rules:
- Wrap mutations in `with unreal.ScopedEditorTransaction("…"):` so the human can Ctrl+Z them.
- Use the editor subsystems. `unreal.EditorLevelLibrary` is deprecated in UE5, so use
  `EditorActorSubsystem`, `LevelEditorSubsystem` and `UnrealEditorSubsystem`.
- `unreal.Rotator` positional order is **(roll, pitch, yaw)**. Always pass keywords.
- Python property names are the snake_case form of the C++ UPROPERTY names, with the `b` prefix dropped
  (`bRealTimeCapture` becomes `real_time_capture`). If unsure, use `help(unreal.<Class>)` or
  `obj.get_editor_property("…")` in a separate call.
- Print what you did (labels, counts, paths), because `ue_python` returns the printed output. Run a read-only
  query first, then the mutation.
- Under World Partition, only *loaded* actors are visible to these calls.

## 1. Handles and queries

```python
import unreal
eas   = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
les   = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()

actors   = eas.get_all_level_actors()
selected = eas.get_selected_level_actors()
by_label = [a for a in actors if a.get_actor_label().startswith("BO_")]
by_class = [a for a in actors if isinstance(a, unreal.StaticMeshActor)]
by_tag   = [a for a in actors if a.actor_has_tag("Destructible")]
in_dir   = [a for a in actors if str(a.get_folder_path()).startswith("Blockout")]
print(world.get_name(), len(actors), "actors;", len(by_label), "blockout")
```

`ue_level_actors` (the MCP tool) gives a quick listing without Python. Use Python when you need filtering beyond class and name.

## 2. Spawning

```python
cube = unreal.EditorAssetLibrary.load_asset("/Engine/BasicShapes/Cube")   # 100 cm cube
with unreal.ScopedEditorTransaction("Spawn grid"):
    for x in range(5):
        for y in range(5):
            a = eas.spawn_actor_from_object(cube, unreal.Vector(x * 400.0, y * 400.0, 50.0),
                                            unreal.Rotator(roll=0.0, pitch=0.0, yaw=0.0))
            a.set_actor_label(f"BO_Block_{x}_{y}")
            a.set_folder_path("Blockout/Grid")

# From a class (lights, volumes, Blueprints):
bp_class = unreal.EditorAssetLibrary.load_blueprint_class("/Game/Gameplay/BP_Checkpoint")
cp = eas.spawn_actor_from_class(bp_class, unreal.Vector(0, 0, 0), unreal.Rotator(roll=0, pitch=0, yaw=90))
```

`spawn_actor_from_object` with a Static Mesh creates a `StaticMeshActor`. With a Blueprint asset it
spawns that Blueprint. Volumes such as `NavMeshBoundsVolume` spawned from Python get a default brush.
Scale the actor to size, and check the result in the viewport (press P for the navmesh).

## 3. Snap to ground (line trace)

```python
def trace_down(x, y, ignore=()):
    """Returns the FHitResult under (x, y), or None. Python returns None when the trace hits nothing."""
    start, end = unreal.Vector(x, y, 100000.0), unreal.Vector(x, y, -100000.0)
    return unreal.SystemLibrary.line_trace_single(world, start, end,
            unreal.TraceTypeQuery.TRACE_TYPE_QUERY1,   # Visibility channel by default
            False, list(ignore), unreal.DrawDebugTrace.NONE, True)

hit = trace_down(0.0, 0.0)
print(hit.to_tuple() if hit else "no hit")   # run this once to find which tuple index is the impact point
```

The field order of `FHitResult.to_tuple()` differs between engine versions. Print it once and choose the
index before you rely on it. The alternative is the editor's End key (snap to floor) on selected actors,
done by the human.

## 4. Placing along a spline

Put a Blueprint or actor with a `SplineComponent` in the level (for example `BP_PathSpline`), and draw the path in the editor.

```python
spline_actor = next(a for a in eas.get_all_level_actors() if a.get_actor_label() == "FencePath")
spline = spline_actor.get_component_by_class(unreal.SplineComponent)
post = unreal.EditorAssetLibrary.load_asset("/Game/Environment/Props/SM_FencePost")
space = unreal.SplineCoordinateSpace.WORLD
length, step = spline.get_spline_length(), 250.0

with unreal.ScopedEditorTransaction("Fence posts along spline"):
    d, i = 0.0, 0
    while d <= length:
        loc = spline.get_location_at_distance_along_spline(d, space)
        rot = spline.get_rotation_at_distance_along_spline(d, space)
        a = eas.spawn_actor_from_object(post, loc, unreal.Rotator(roll=0.0, pitch=0.0, yaw=rot.yaw))
        a.set_actor_label(f"FencePost_{i:03d}")
        a.set_folder_path("Environment/Fence")
        d += step; i += 1
print("placed", i, "posts over", round(length), "cm")
```

For hundreds of pieces, prefer one actor with an Instanced Static Mesh or a PCG spline graph
(the `SKILL.md` PCG section). Thousands of individual actors hurt the editor and World Partition.

## 5. Bulk mobility, collision profile and tags

```python
with unreal.ScopedEditorTransaction("Environment to Static"):
    changed = 0
    for a in eas.get_all_level_actors():
        if not str(a.get_folder_path()).startswith("Environment") or a.actor_has_tag("Movable"):
            continue
        for c in a.get_components_by_class(unreal.StaticMeshComponent):
            c.set_mobility(unreal.ComponentMobility.STATIC)
            changed += 1
print("set static:", changed)

# Small clutter: no collision and no navmesh influence
with unreal.ScopedEditorTransaction("Clutter collision"):
    for a in eas.get_all_level_actors():
        if str(a.get_folder_path()).startswith("Environment/Clutter"):
            for c in a.get_components_by_class(unreal.StaticMeshComponent):
                c.set_collision_profile_name("NoCollision")
                c.set_can_ever_affect_navigation(False)

# Tags (actor.tags is an array of Names)
for a in eas.get_selected_level_actors():
    tags = list(a.tags)
    if "Destructible" not in [str(t) for t in tags]:
        tags.append("Destructible")
        a.set_editor_property("tags", tags)
```

Mobility rules: *Static* for anything that never moves (cheapest, with the best cached shadows), *Movable* for
anything animated, spawned or simulated. With Lumen there is no lightmap baking, but Static still helps VSM caching and rendering.

## 6. Outliner organization

```python
FOLDERS = {                         # prefix of asset name -> outliner folder
    "SM_Rock": "Environment/Rocks",
    "SM_Tree": "Environment/Foliage",
    "BP_Enemy": "Gameplay/Enemies",
    "BP_Pickup": "Gameplay/Pickups",
}
with unreal.ScopedEditorTransaction("Organize outliner"):
    for a in eas.get_all_level_actors():
        name = ""
        if isinstance(a, unreal.StaticMeshActor):
            m = a.static_mesh_component.get_editor_property("static_mesh")
            name = m.get_name() if m else ""
        else:
            name = a.get_class().get_name()
        for prefix, folder in FOLDERS.items():
            if name.startswith(prefix):
                a.set_folder_path(folder)
                break
```

Also enforce labels (`set_actor_label`), so that `ue_level_actors` name filters stay useful. Lighting
actors go in `Lighting/`, volumes in `Volumes/`, and blockout in `Blockout/`.

## 7. Swap blockout for art

```python
new_mesh = unreal.EditorAssetLibrary.load_asset("/Game/Environment/Bunker/SM_Bunker_Wall_400x300")
with unreal.ScopedEditorTransaction("Replace blockout walls"):
    n = 0
    for a in eas.get_all_level_actors():
        if isinstance(a, unreal.StaticMeshActor) and a.get_actor_label().startswith("BO_Wall_"):
            a.static_mesh_component.set_static_mesh(new_mesh)
            a.set_actor_scale3d(unreal.Vector(1, 1, 1))   # kit pieces are authored to size
            a.set_folder_path("Environment/Bunker")
            n += 1
print("replaced", n)
```

This works only if the kit pivots match the blockout convention (bottom corner) or you adjust the offsets.
Check a few in the viewport.

## 8. Data Layers (World Partition)

```python
dls = unreal.get_editor_subsystem(unreal.DataLayerEditorSubsystem)
layers = dls.get_all_data_layers()
for l in layers:
    print(l.get_name(), l.get_data_layer_short_name() if hasattr(l, "get_data_layer_short_name") else "")
```

```python
target = next(l for l in layers if "Blockout" in str(l.get_data_layer_short_name()))
actors = [a for a in eas.get_all_level_actors() if a.get_actor_label().startswith("BO_")]
with unreal.ScopedEditorTransaction("Blockout to data layer"):
    ok = dls.add_actors_to_data_layer(actors, target)
print("added:", ok, len(actors))
```

Creating Data Layer Assets and Instances is easiest for the human: Window > World Partition > Data
Layers Outliner > right-click > *Create Data Layer*, then assign or create the Data Layer Asset. If you
must script it, inspect `help(unreal.DataLayerEditorSubsystem)` for the creation call and its
parameter struct in the installed version first. Method names on data layer instances changed
across 5.0–5.3.

Actors that must always be loaded: `a.set_editor_property("is_spatially_loaded", False)` (World
Partition only). Check the property name with `help(unreal.Actor)` if it fails.

Loading regions from Python: 5.1+ has `unreal.WorldPartitionBlueprintLibrary` (actor descriptor
queries, load and unload by GUID). Check `help()` for the exact API, or ask the human to load the region
in the World Partition editor.

## 9. Assets: Nanite and simple collision in bulk

```python
sme = unreal.get_editor_subsystem(unreal.StaticMeshEditorSubsystem)
paths = unreal.EditorAssetLibrary.list_assets("/Game/Environment/Rocks", recursive=True, include_folder=False)
for p in paths:
    m = unreal.EditorAssetLibrary.load_asset(p)
    if not isinstance(m, unreal.StaticMesh):
        continue
    ns = m.get_editor_property("nanite_settings")
    if not ns.get_editor_property("enabled"):
        ns.set_editor_property("enabled", True)
        m.set_editor_property("nanite_settings", ns)       # triggers a rebuild of the mesh
    if sme.get_simple_collision_count(m) == 0:
        sme.add_simple_collisions(m, unreal.ScriptingCollisionShapeType.BOX)
    unreal.EditorAssetLibrary.save_loaded_asset(m)
    print("updated", p)
```

Rebuilding Nanite on many large meshes takes time, so run it in batches and tell the human. For concave
shapes, use `sme.set_convex_decomposition_collisions(m, hull_count, max_hull_verts, hull_precision)`,
or author UCX meshes in the DCC.

## 10. Saving and reporting

```python
# Current level only:
les.save_current_level()
# Everything dirty (maps, OFPA external actor packages, assets):
unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)
```

Before saving, list what is dirty so that you can report it. If these calls are missing in your version, check
`help(unreal.EditorLoadingAndSavingUtils)`:

```python
maps = unreal.EditorLoadingAndSavingUtils.get_dirty_map_packages()
content = unreal.EditorLoadingAndSavingUtils.get_dirty_content_packages()
print("maps:", [p.get_name() for p in maps])
print("content:", [p.get_name() for p in content])
```

With One File Per Actor, the changed files are under `Content/__ExternalActors__/<MapPath>/…`, not the
`.umap`. Tell the human to commit them (and `__ExternalObjects__` if present).

## 11. Useful console commands through `ue_console`

- `show Navigation`: toggle navmesh display.
- `show Collision`: toggle collision display.
- `stat unit`, `stat gpu`, `stat scenerendering`: performance numbers.
- `r.Nanite 0` or `1`, and `r.Shadow.Virtual.Enable 0` or `1`: quick A/B comparisons. These don't persist, so restore the value after testing.
