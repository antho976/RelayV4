# ue_anim_preview: a temporary posed copy of a character (plus attached items and an optional
# partner) in the open level, so the camera can see what the numbers say. Actions:
#   setup   - spawn "RelayPreview" actors (removing any left from before)
#   pose    - move every preview animation to ARGS["time"]; returns where a probe bone should be
#   check   - where that probe bone actually is (the editor applies a pose on its next tick)
#   cleanup - destroy every "RelayPreview" actor

PREFIX = "RelayPreview"
subsystem = actor_subsystem()


def previews():
    return [a for a in subsystem.get_all_level_actors() if a.get_actor_label().startswith(PREFIX)]


def call_first(obj, names, *args):
    for n in names:
        fn = getattr(obj, n, None)
        if fn is not None:
            return fn(*args)
    raise RuntimeError("%s has none of %s" % (obj.get_class().get_name(), ", ".join(names)))


def skeletal_actor(label, mesh, anim, location, yaw):
    actor = subsystem.spawn_actor_from_class(unreal.SkeletalMeshActor, unreal.Vector(*location), unreal.Rotator(roll=0.0, pitch=0.0, yaw=yaw))
    actor.set_actor_label(label)
    actor.set_folder_path(PREFIX)
    comp = actor.get_editor_property("skeletal_mesh_component")
    call_first(comp, ("set_skeletal_mesh_asset", "set_skeletal_mesh"), mesh)
    if anim is not None:
        comp.set_animation_mode(unreal.AnimationMode.ANIMATION_SINGLE_NODE)
        comp.set_animation(anim)
        comp.set_update_animation_in_editor(True)
        comp.set_play_rate(0.0)
    return actor


def skeletal_component(actor):
    return actor.get_editor_property("skeletal_mesh_component")


action = ARGS["action"]
if action == "cleanup":
    gone = previews()
    for a in gone:
        subsystem.destroy_actor(a)
    emit({"removed": len(gone)})

elif action == "setup":
    for a in previews():
        subsystem.destroy_actor(a)
    mesh = load(ARGS["mesh"], "skeletal mesh")
    anim = load(ARGS["animation"], "animation") if ARGS.get("animation") else None
    if ARGS.get("location"):
        base = tuple(float(c) for c in ARGS["location"])
    else:
        # High above where the editor camera is: lit by the level's sun and sky, and clear of
        # walls and props that would block the view.
        loc, rot = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_level_viewport_camera_info()
        base = add(vec(loc), (0.0, 0.0, float(ARGS.get("altitude", 50000.0))))
    labels = []
    main = skeletal_actor(PREFIX + " Character", mesh, anim, base, 0.0)
    labels.append(main.get_actor_label())
    for spec in ARGS.get("attachments", []):
        asset = load(spec["mesh"], "item mesh")
        if isinstance(asset, unreal.SkeletalMesh):
            item = skeletal_actor(PREFIX + " " + spec["name"], asset, None, base, 0.0)
        else:
            item = subsystem.spawn_actor_from_class(unreal.StaticMeshActor, unreal.Vector(*base), unreal.Rotator())
            item.set_actor_label(PREFIX + " " + spec["name"])
            item.set_folder_path(PREFIX)
            item.get_editor_property("static_mesh_component").set_static_mesh(asset)
        rule = unreal.AttachmentRule.SNAP_TO_TARGET
        item.attach_to_actor(main, spec["socket"], rule, rule, unreal.AttachmentRule.KEEP_WORLD, False)
        off = offset_transform(spec)
        root = item.get_editor_property("root_component")
        call_first(root, ("set_relative_location", "k2_set_relative_location"), unreal.Vector(*off[0]), False, True)
        rot = spec.get("rotation", [0, 0, 0])
        call_first(root, ("set_relative_rotation", "k2_set_relative_rotation"),
                   unreal.Rotator(roll=float(rot[2]), pitch=float(rot[0]), yaw=float(rot[1])), False, True)
        labels.append(item.get_actor_label())
    if ARGS.get("partner"):
        spec = ARGS["partner"]
        p_mesh = load(spec["mesh"], "partner skeletal mesh")
        p_anim = load(spec["animation"], "partner animation") if spec.get("animation") else None
        rel = tuple(float(c) for c in spec.get("location", [0, 150, 0]))
        partner = skeletal_actor(PREFIX + " Partner", p_mesh, p_anim, add(base, rel), float(spec.get("yaw", 180)))
        labels.append(partner.get_actor_label())
    frame = body_frame(Skeleton(mesh))
    emit({"actors": labels, "location": rnd(base), "forward": rnd(frame["forward"], 4),
          "times": sample_times(anim)})

elif action in ("pose", "check"):
    t = float(ARGS.get("time", 0.0))
    found = previews()
    main = [a for a in found if a.get_actor_label() == PREFIX + " Character"]
    if not main:
        raise RuntimeError("no preview character; run setup first")
    main = main[0]
    mesh = load(ARGS["mesh"], "skeletal mesh")
    anim = load(ARGS["animation"], "animation") if ARGS.get("animation") else None
    skel = Skeleton(mesh)
    probe = ARGS.get("probe") or next((n for n in skel.names if "hand" in n.lower()), skel.names[-1])
    comp = skeletal_component(main)
    if action == "pose":
        # Only the character and the partner play animations; attached items follow sockets.
        for a in found:
            label = a.get_actor_label()
            if label not in (PREFIX + " Character", PREFIX + " Partner"):
                continue
            offset = float((ARGS.get("partner") or {}).get("time_offset", 0.0)) if label.endswith("Partner") else 0.0
            skeletal_component(a).set_position(t + offset, False)
    pose = skel.component_pose(skel.local_pose(anim, t))
    expected = skel.point(pose, probe)[0]
    base = vec(main.get_actor_location())
    expected_world = add(base, expected)
    actual = vec(comp.get_socket_location(probe))
    # Frame the posed skeleton itself (plus a margin for held items), not the actor bounds.
    pts = [add(base, p[0]) for p in pose.values()]
    lo = tuple(min(p[i] for p in pts) for i in range(3))
    hi = tuple(max(p[i] for p in pts) for i in range(3))
    center = mul(add(lo, hi), 0.5)
    radius = length(sub(hi, lo)) * 0.5 + float(ARGS.get("margin", 40.0))
    emit({"time": t, "probe": probe, "expected": rnd(expected_world), "actual": rnd(actual),
          "off_by_cm": round(length(sub(expected_world, actual)), 2),
          "center": rnd(center), "radius": round(radius, 1)})
else:
    raise RuntimeError("unknown action %r" % action)
