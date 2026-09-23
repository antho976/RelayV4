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


def coverage(rt):
    """Share of sampled pixels that differ from the corner (background) pixel: 0 means the
    subject did not render (an invisible mesh draws only its shadow, or nothing)."""
    try:
        bg = unreal.RenderingLibrary.read_render_target_pixel(world, rt, 1, 1)
        hits, total = 0, 0
        for gx in range(1, 24):
            for gy in range(1, 24):
                c = unreal.RenderingLibrary.read_render_target_pixel(world, rt, int(width * gx / 24), int(height * gy / 24))
                total += 1
                if abs(c.r - bg.r) + abs(c.g - bg.g) + abs(c.b - bg.b) > 24:
                    hits += 1
        return round(hits / float(total), 3)
    except Exception:
        return None


def capture(shots, hidden):
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
        # Isolation hides the actors around the subject. The show-only primitive list was the
        # first approach and crashed the editor in real use; hidden actors are plain data.
        if hidden:
            comp.set_editor_property("hidden_actors", hidden)
        for name, eye, rot in shots:
            cam.set_actor_location_and_rotation(unreal.Vector(*eye), rot, False, True)
            comp.capture_scene()
            file_name = "%s_%s.png" % (ARGS.get("prefix", "shot"), name)
            unreal.RenderingLibrary.export_render_target(world, rt, out_dir, file_name)
            entry = {"view": name, "file": os.path.join(out_dir, file_name)}
            if ARGS.get("coverage"):
                entry["coverage"] = coverage(rt)
            files.append(entry)
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


def neighbours(targets, center, radius):
    """Actors near the subject that are not part of it, for isolation (bounded)."""
    keep = set(a.get_path_name() for a in targets)
    out = []
    for a in actor_subsystem().get_all_level_actors():
        if a.get_path_name() in keep:
            continue
        try:
            origin, extent = a.get_actor_bounds(False)
        except Exception:
            continue
        if length(sub(vec(origin), center)) - length(vec(extent)) < radius * 6.0:
            out.append(a)
            if len(out) >= 2000:
                break
    return out


shots = []
hidden = []
if ARGS.get("center") is not None:
    # Framing given by the caller (animation previews frame the posed skeleton, which actor
    # bounds do not follow).
    center = tuple(float(c) for c in ARGS["center"])
    radius = float(ARGS.get("radius", 100.0))
    forward = normalize(tuple(ARGS.get("forward") or (1.0, 0.0, 0.0)))
    right = normalize(cross((0.0, 0.0, 1.0), forward))
    shots = views_around(center, radius, forward, right, ARGS.get("views") or ["front", "right"])
    if ARGS.get("isolate") and ARGS.get("actors"):
        hidden = neighbours([find_actor(a) for a in ARGS["actors"]], center, radius)
elif ARGS.get("camera"):
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
            hidden = neighbours(actors, center, radius)

emit({"files": capture(shots, hidden), "width": width, "height": height, "hidden_for_isolation": len(hidden)})
