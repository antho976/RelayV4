# Place an imported mesh where nothing else is (500 m above the editor camera) so the camera can
# check that it renders. "spawn" returns the actor label and framing; "cleanup" removes it.
PREFIX = "RelayPreview"
subsystem = actor_subsystem()
if ARGS["action"] == "cleanup":
    gone = [a for a in subsystem.get_all_level_actors() if a.get_actor_label().startswith(PREFIX + " Asset")]
    for a in gone:
        subsystem.destroy_actor(a)
    emit({"removed": len(gone)})
else:
    asset = load(ARGS["path"], "mesh")
    loc, rot = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_level_viewport_camera_info()
    base = add(vec(loc), (0.0, 0.0, 50000.0))
    if isinstance(asset, unreal.SkeletalMesh):
        actor = subsystem.spawn_actor_from_class(unreal.SkeletalMeshActor, unreal.Vector(*base), unreal.Rotator())
        comp = actor.get_editor_property("skeletal_mesh_component")
        (getattr(comp, "set_skeletal_mesh_asset", None) or comp.set_skeletal_mesh)(asset)
    else:
        actor = subsystem.spawn_actor_from_class(unreal.StaticMeshActor, unreal.Vector(*base), unreal.Rotator())
        actor.get_editor_property("static_mesh_component").set_static_mesh(asset)
    actor.set_actor_label(PREFIX + " Asset")
    actor.set_folder_path(PREFIX)
    b = asset.get_bounds()
    center = add(base, vec(b.origin))
    emit({"actor": actor.get_actor_label(), "center": rnd(center), "radius": round(max(length(vec(b.box_extent)), 5.0), 1)})
