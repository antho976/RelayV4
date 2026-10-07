# ue_screenshot and ue_anim_preview's camera: renders views with a temporary SceneCapture2D and
# writes PNGs, synchronously, inside this one script. The camera is transient, so the level is
# not left modified.

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


GRID = 48


def samples(rt):
    """Colours on a GRID x GRID lattice over the render target. Read back in one go where the
    engine can; a failed read raises rather than passing for an image."""
    reader = getattr(unreal.RenderingLibrary, "read_render_target", None)
    points = [(int(width * gx / GRID), int(height * gy / GRID)) for gx in range(1, GRID) for gy in range(1, GRID)]
    if reader is not None:
        pixels = reader(world, rt)
        if not pixels or len(pixels) < width * height:
            raise RuntimeError("reading the render target back failed")
        picked = [pixels[y * width + x] for x, y in points]
    else:
        picked = [unreal.RenderingLibrary.read_render_target_pixel(world, rt, x, y) for x, y in points]
    return [(int(c.r), int(c.g), int(c.b)) for c in picked]


def differing(reference, shot, tolerance=24):
    """Share of samples that differ between a capture with the subject hidden and one with it
    shown. Sky gradients, ground and the subject's shadow (hidden actors still cast one) are in
    both, so only the subject itself counts: 0 means it did not render."""
    if not reference or len(reference) != len(shot):
        raise RuntimeError("the background-only capture has no matching samples")
    hits = sum(1 for a, b in zip(reference, shot) if abs(a[0] - b[0]) + abs(a[1] - b[1]) + abs(a[2] - b[2]) > tolerance)
    return round(hits / float(len(shot)), 4)


def capture(shots, hidden, subjects):
    rt = unreal.RenderingLibrary.create_render_target2d(world, width, height, unreal.TextureRenderTargetFormat.RTF_RGBA8)
    cam = spawn_helper(unreal.SceneCapture2D, unreal.Vector(0, 0, 0), unreal.Rotator())
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
            reference, problem = None, None
            if ARGS.get("coverage"):
                # The same view without the subject: what the background looks like here.
                if not subjects:
                    problem = "coverage needs actors to compare against a capture without them"
                else:
                    try:
                        comp.set_editor_property("hidden_actors", list(hidden) + list(subjects))
                        comp.capture_scene()
                        reference = samples(rt)
                    except Exception as e:
                        problem = "background capture failed: %s" % e
                    comp.set_editor_property("hidden_actors", list(hidden))
            comp.capture_scene()
            file_name = "%s_%s.png" % (ARGS.get("prefix", "shot"), name)
            unreal.RenderingLibrary.export_render_target(world, rt, out_dir, file_name)
            entry = {"view": name, "file": os.path.join(out_dir, file_name)}
            if ARGS.get("coverage"):
                # Unknown (None, with the reason) is never reported as visible.
                entry["coverage"] = None
                if problem is None:
                    try:
                        entry["coverage"] = differing(reference, samples(rt))
                    except Exception as e:
                        problem = "reading the capture failed: %s" % e
                if problem is not None:
                    entry["coverage_error"] = problem
            files.append(entry)
    finally:
        actor_subsystem().destroy_actor(cam)
    return files


def target_frame():
    """Centre, radius and facing of what to frame."""
    if ARGS.get("actors"):
        actors = subjects
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
subjects = [find_actor(a) for a in ARGS.get("actors") or []]
if ARGS.get("center") is not None:
    # Framing given by the caller (animation previews frame the posed skeleton, which actor
    # bounds do not follow).
    center = tuple(float(c) for c in ARGS["center"])
    radius = float(ARGS.get("radius", 100.0))
    forward = normalize(tuple(ARGS.get("forward") or (1.0, 0.0, 0.0)))
    right = normalize(cross((0.0, 0.0, 1.0), forward))
    shots = views_around(center, radius, forward, right, ARGS.get("views") or ["front", "right"])
    if ARGS.get("isolate") and subjects:
        hidden = neighbours(subjects, center, radius)
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

emit({"files": capture(shots, hidden, subjects), "width": width, "height": height, "hidden_for_isolation": len(hidden)})
