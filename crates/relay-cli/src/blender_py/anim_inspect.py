# blender_anim_inspect: the same measurements as Unreal's ue_anim_inspect, on a Blender action,
# before anything is exported. Items are objects already attached in the scene (bone parent or
# Child Of constraint); sockets are empties.

arm = obj(ARGS["armature"], "ARMATURE") if ARGS.get("armature") else first("ARMATURE")
if arm is None:
    raise RuntimeError("no armature; pass armature")
frame = body_frame(arm)  # rest pose, before any action is applied
set_action(arm, ARGS.get("action"))
frames = sample_frames(arm)
radius = float(ARGS.get("body_radius", 8.0)) / TO_CM
touch = float(ARGS.get("touch_distance", 5.0))
problems, minimum, rows = [], {}, []

partner = None
if ARGS.get("partner"):
    p = ARGS["partner"]
    partner = obj(p["armature"], "ARMATURE")
    set_action(partner, p.get("action"))

SKIP = ("finger", "thumb", "index", "middle", "ring", "pinky", "metacarpal", "twist", "ik", "weapon", "prop", "root",
        "eye", "jaw", "tongue", "pole", "ctrl", "mch", "org")


def segments(a):
    return [b.name for b in a.data.bones if b.use_deform and b.parent is not None and not any(k in b.name.lower() for k in SKIP)]


arm_segments = segments(arm)
p_segments = segments(partner) if partner else []


def head(a, bone):
    return a.matrix_world @ a.pose.bones[bone].head


def tail(a, bone):
    return a.matrix_world @ a.pose.bones[bone].tail


def item_points(o):
    local = [Vector(c) for c in o.bound_box]
    lo = Vector((min(c.x for c in local), min(c.y for c in local), min(c.z for c in local)))
    hi = Vector((max(c.x for c in local), max(c.y for c in local), max(c.z for c in local)))
    ext = hi - lo
    axis = max(range(3), key=lambda i: ext[i])
    center = (lo + hi) / 2
    a, b = center.copy(), center.copy()
    a[axis], b[axis] = hi[axis], lo[axis]
    return {"end_a": a, "end_b": b, "center": center, "origin": Vector((0, 0, 0)), "_axis": "XYZ"[axis], "_length": ext[axis]}


items = {}
for spec in ARGS.get("attachments", []):
    o = obj(spec["object"])
    holder = o.parent_bone if o.parent == arm and o.parent_type == "BONE" else None
    for c in o.constraints:
        if c.type == "CHILD_OF" and getattr(c, "target", None) == arm and c.subtarget:
            holder = c.subtarget
    items[spec.get("name", o.name)] = {"object": o, "points": item_points(o), "holder": holder, "grips": spec.get("grips", [])}


def resolve(ref):
    if ref.startswith("item:"):
        _, name, which = ref.split(":", 2)
        it = items.get(name)
        if it is None:
            raise RuntimeError("no attachment named %r" % name)
        return it["object"].matrix_world @ it["points"][which]
    if ref.startswith("obj:"):
        return obj(ref[4:]).matrix_world.translation.copy()
    if ref.startswith("partner:"):
        return head(partner, ref.split(":", 1)[1])
    if ref.endswith(":tail"):
        return tail(arm, ref[:-5])
    if ref not in arm.pose.bones:
        raise RuntimeError("no bone %r in %s" % (ref, arm.name))
    return head(arm, ref)


def near_chain(bone):
    out, b = set(), arm.data.bones.get(bone) if bone else None
    for _ in range(3):
        if b is None:
            break
        out.add(b.name)
        b = b.parent
    return out


def keep_min(key, value, near, f):
    if key not in minimum or value < minimum[key][0]:
        minimum[key] = (value, near, f)


def problem(f, kind, text):
    if len(problems) < 80:
        problems.append({"frame": f, "kind": kind, "detail": text})


feet = [b.name for b in arm.data.bones if "foot" in b.name.lower() and b.use_deform]
ground = frame["center"].z
for f in frames:
    scene.frame_set(f)
    row = {"frame": f, "points": {}, "checks": []}
    for ref in ARGS.get("track") or []:
        p = resolve(ref)
        row["points"][ref] = {"fwd_right_up": rnd(to_body(frame, p)), "side": side(frame, p)}
    for name, it in items.items():
        allowed = near_chain(it["holder"])
        for g in it["grips"]:
            allowed |= near_chain(g.get("bone"))
        for end in ("end_a", "end_b", "center"):
            p = it["object"].matrix_world @ it["points"][end]
            best = (1e9, None)
            for s in arm_segments:
                if s in allowed:
                    continue
                d = segment_distance(p, head(arm, s), tail(arm, s))
                if d < best[0]:
                    best = (d, s)
            keep_min("item:%s:%s clearance to own body" % (name, end), round((best[0] - radius) * TO_CM, 1), best[1], f)
            if best[0] < radius:
                problem(f, "clipping", "%s of %r is %.1f cm inside the body near %s" % (end, name, (radius - best[0]) * TO_CM, best[1]))
        for g in it["grips"]:
            gp = resolve(g["point"])
            bp = resolve(g["bone"])
            d = (gp - bp).length * TO_CM
            row["checks"].append({"grip": "%s -> %s" % (g["point"], g["bone"]), "distance": round(d, 1)})
            tol = float(g.get("tolerance", touch))
            if d > tol:
                problem(f, "grip", "%s is %.1f cm from %s (tolerance %.1f)" % (g["bone"], d, g["point"], tol))
    for c in ARGS.get("contacts", []):
        d = (resolve(c["a"]) - resolve(c["b"])).length * TO_CM
        expect = c.get("expect", "touch")
        limit = float(c.get("distance", touch if expect == "touch" else 10.0))
        row["checks"].append({"contact": "%s ~ %s" % (c["a"], c["b"]), "distance": round(d, 1), "expect": expect})
        inside = not c.get("window") or c["window"][0] <= f <= c["window"][1]
        if inside and expect == "touch" and d > limit:
            problem(f, "contact", "%s and %s should touch but are %.1f cm apart" % (c["a"], c["b"], d))
        if inside and expect == "apart" and d < limit:
            problem(f, "contact", "%s and %s should stay %.0f cm apart but are %.1f cm" % (c["a"], c["b"], limit, d))
    if partner:
        probes = [b.name for b in arm.data.bones if b.use_deform and any(k in b.name.lower() for k in ("hand", "foot", "head", "forearm", "lowerarm", "shin", "calf"))]
        points = [(n, head(arm, n)) for n in probes] + [("item:%s:%s" % (n, e), it["object"].matrix_world @ it["points"][e]) for n, it in items.items() for e in ("end_a", "end_b", "center")]
        for name, p in points:
            best = (1e9, None)
            for s in p_segments:
                d = segment_distance(p, head(partner, s), tail(partner, s))
                if d < best[0]:
                    best = (d, s)
            keep_min("%s clearance to partner" % name, round((best[0] - radius) * TO_CM, 1), best[1], f)
            intended = any(c.get("expect", "touch") == "touch" and name in (c["a"], c["b"]) for c in ARGS.get("contacts", []))
            if best[0] < radius and not intended:
                problem(f, "partner_clipping", "%s is %.1f cm inside the partner near %s" % (name, (radius - best[0]) * TO_CM, best[1]))
    if feet:
        heights = dict((n, round((head(arm, n).z - ground) * TO_CM, 1)) for n in feet)
        row["feet_height"] = heights
        if min(heights.values()) < -2.0:
            problem(f, "ground", "a foot is %.1f cm below the rest-pose ground" % -min(heights.values()))
    rows.append(row)

scene.frame_set(frames[0])
attach = {}
for name, it in items.items():
    o = it["object"]
    attach[name] = {"object": o.name, "held_by": it["holder"], "side": side(frame, o.matrix_world.translation),
                    "fwd_right_up": rnd(to_body(frame, o.matrix_world.translation)), "long_axis": it["points"]["_axis"],
                    "length_cm": round(it["points"]["_length"] * TO_CM * max(o.scale), 1),
                    "end_a_at_start": rnd(to_body(frame, o.matrix_world @ it["points"]["end_a"])),
                    "end_b_at_start": rnd(to_body(frame, o.matrix_world @ it["points"]["end_b"]))}
    if it["holder"] is None:
        problems.append({"frame": frames[0], "kind": "attachment", "detail": "%r is not parented to a bone of %s (or held by a Child Of constraint); it will not follow the hand" % (name, arm.name)})

emit({"armature": arm.name, "action": arm.animation_data.action.name if arm.animation_data and arm.animation_data.action else None,
      "frames": frames,
      "frame": {"note": "Positions are [forward, right, up] in cm from the character's centre at ground level, from the rig's own .L/.R bone pairs; right > 0 is the character's right.",
                "forward_world": rnd(frame["forward"], 3), "right_world": rnd(frame["right"], 3), "left_right_pairs_found": len(frame["pairs"])},
      "attachments": attach, "problems": problems, "passed": not problems,
      "closest_approach": dict((k, {"clearance_cm": v[0], "near": v[1], "frame": v[2]}) for k, v in sorted(minimum.items())),
      "samples": rows})
