# ue_screenshot and ue_anim_preview's camera: renders views with a temporary SceneCapture2D and
# writes PNGs, synchronously, inside this one script.

import os

world = editor_world()
out_dir = ARGS["out_dir"]
width = int(ARGS.get("width", 640))
height = int(ARGS.get("height", 480))
fov = float(ARGS.get("fov", 50.0))


def look_rotation(eye, target):
    d = sub(target, eye)
    yaw = math.degrees(math.atan2(d[1], d[0]))
    pitch = math.degrees(math.atan2(d[2], math.sqrt(d[0] * d[0] + d[1] * d[1])))
    return unreal.Rotator(roll=0.0, pitch=pitch, yaw=yaw)


def views_around(center, radius, forward, right, names):
    up = (0.0, 0.0, 1.0)
    dist = max(radius, 20.0) / math.tan(math.radians(fov) / 2.0) * 1.15
    dirs = {
        "front": forward, "back": mul(forward, -1), "right": right, "left": mul(right, -1),
        "top": normalize(add(up, mul(forward, 0.001))),
        "three_quarter": normalize(add(add(forward, right), mul(up, 0.5))),
        "three_quarter_left": normalize(add(add(forward, mul(right, -1)), mul(up, 0.5))),
    }
    out = []
    for name in names:
        if name not in dirs:
            raise RuntimeError("unknown view %r; use %s" % (name, ", ".join(sorted(dirs))))
        eye = add(center, mul(dirs[name], dist))
        out.append((name, eye, look_rotation(eye, center)))
    return out


def capture(shots, show_only):
    rt = unreal.RenderingLibrary.create_render_target2d(world, width, height, unreal.TextureRenderTargetFormat.RTF_RGBA8)
    cam = actor_subsystem().spawn_actor_from_class(unreal.SceneCapture2D, unreal.Vector(0, 0, 0), unreal.Rotator())
    files = []
    try:
        comp = cam.get_editor_property("capture_component2d")
        comp.set_editor_property("texture_target", rt)
        comp.set_editor_property("capture_source", unreal.SceneCaptureSource.SCS_FINAL_COLOR_LDR)
        comp.set_editor_property("capture_every_frame", False)
        comp.set_editor_property("capture_on_movement", False)
        comp.set_editor_property("fov_angle", fov)
        if show_only:
            comp.set_editor_property("primitive_render_mode", unreal.SceneCapturePrimitiveRenderMode.PRM_USE_SHOW_ONLY_LIST)
            comp.set_editor_property("show_only_actors", show_only)
        for name, eye, rot in shots:
            cam.set_actor_location_and_rotation(unreal.Vector(*eye), rot, False, True)
            comp.capture_scene()
            file_name = "%s_%s.png" % (ARGS.get("prefix", "shot"), name)
            unreal.RenderingLibrary.export_render_target(world, rt, out_dir, file_name)
            files.append({"view": name, "file": os.path.join(out_dir, file_name)})
    finally:
        actor_subsystem().destroy_actor(cam)
    return files


def target_frame():
    """Centre, radius and facing of what to frame."""
    if ARGS.get("actors"):
        actors = [find_actor(a) for a in ARGS["actors"]]
        lo, hi = None, None
        for a in actors:
            origin, extent = a.get_actor_bounds(False)
            o, e = vec(origin), vec(extent)
            amin, amax = sub(o, e), add(o, e)
            lo = amin if lo is None else tuple(min(x, y) for x, y in zip(lo, amin))
            hi = amax if hi is None else tuple(max(x, y) for x, y in zip(hi, amax))
        center = mul(add(lo, hi), 0.5)
        radius = length(sub(hi, lo)) * 0.5
        if ARGS.get("forward"):
            forward = normalize(tuple(ARGS["forward"]))
        else:
            forward = vec(actors[0].get_actor_forward_vector())
        right = normalize(cross((0.0, 0.0, 1.0), forward))
        return center, radius, forward, right, actors
    return None


shots = []
show_only = []
if ARGS.get("camera"):
    c = ARGS["camera"]
    r = c.get("rotation", [0, 0, 0])
    shots.append(("camera", tuple(float(x) for x in c["location"]), unreal.Rotator(roll=float(r[2]), pitch=float(r[0]), yaw=float(r[1]))))
else:
    framed = target_frame()
    if framed is None:
        loc, rot = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_level_viewport_camera_info()
        shots.append(("viewport", vec(loc), rot))
    else:
        center, radius, forward, right, actors = framed
        shots = views_around(center, radius, forward, right, ARGS.get("views") or ["front", "right", "three_quarter"])
        if ARGS.get("isolate"):
            show_only = actors

emit({"files": capture(shots, show_only), "width": width, "height": height})
