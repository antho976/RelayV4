# ue_anim_inspect: numbers, not impressions. Poses a skeleton at sample times straight from the
# animation data (no level, no ticking), carries attached items and an optional partner along,
# and reports sides, contacts, clearances and ground contact in the character's own frame.

mesh = load(ARGS["mesh"], "skeletal mesh")
anim = load(ARGS["animation"], "animation") if ARGS.get("animation") else None
skel = Skeleton(mesh)
frame = body_frame(skel)
times = sample_times(anim)
radius = float(ARGS.get("body_radius", 8.0))
touch = float(ARGS.get("touch_distance", 5.0))
problems = []
minimum = {}


def keep_min(key, clearance, near, t):
    if key not in minimum or clearance < minimum[key][0]:
        minimum[key] = (clearance, near, t)


def problem(t, kind, text):
    if len(problems) < 80:
        problems.append({"time": t, "kind": kind, "detail": text})


# ---- attached items: a transform chain from a character bone or socket

class Item(object):
    def __init__(self, spec):
        self.name = spec["name"]
        self.asset = load(spec["mesh"], "item mesh")
        self.attach = spec["socket"]
        self.offset = offset_transform(spec)
        self.grips = spec.get("grips", [])
        bounds = self.asset.get_bounds()
        origin, extent = vec(bounds.origin), vec(bounds.box_extent)
        axis = max(range(3), key=lambda i: extent[i])
        unit = tuple(1.0 if i == axis else 0.0 for i in range(3))
        self.ends = {"end_a": add(origin, mul(unit, extent[axis])), "end_b": sub(origin, mul(unit, extent[axis])),
                     "origin": (0.0, 0.0, 0.0), "center": origin}
        self.long_axis = "XYZ"[axis]
        self.length = 2.0 * extent[axis]

    def transform(self, pose):
        return compose(skel.point(pose, self.attach), self.offset)

    def point(self, pose, which):
        base = self.transform(pose)
        if which in self.ends:
            return compose(base, (self.ends[which], (0, 0, 0, 1), (1, 1, 1)))[0]
        s = self.asset.find_socket(which)
        if s is None:
            raise RuntimeError("item %r has no socket %r (it has end_a, end_b, origin, center)" % (self.name, which))
        rel = (vec(s.get_editor_property("relative_location")), quat_of_rotator(s.get_editor_property("relative_rotation")), (1, 1, 1))
        return compose(base, rel)[0]


items = dict((spec["name"], Item(spec)) for spec in ARGS.get("attachments", []))

# ---- a second character, placed in this character's mesh space

partner = None
if ARGS.get("partner"):
    spec = ARGS["partner"]
    p_mesh = load(spec["mesh"], "partner skeletal mesh")
    partner = {
        "skel": Skeleton(p_mesh),
        "anim": load(spec["animation"], "partner animation") if spec.get("animation") else None,
        "place": (tuple(float(c) for c in spec.get("location", [0, 150, 0])), yaw_quat(float(spec.get("yaw", 180))), (1.0, 1.0, 1.0)),
        "time_offset": float(spec.get("time_offset", 0.0)),
    }
    partner["segments"] = body_segments(partner["skel"])


def resolve(ref, pose, p_pose):
    """`bone_or_socket`, `partner:bone_or_socket` or `item:<name>:<socket|end_a|end_b|origin|center>`."""
    if ref.startswith("item:"):
        _, name, which = ref.split(":", 2)
        if name not in items:
            raise RuntimeError("no attachment named %r" % name)
        return items[name].point(pose, which)
    if ref.startswith("partner:"):
        if p_pose is None:
            raise RuntimeError("%r needs a partner" % ref)
        return compose(partner["place"], partner["skel"].point(p_pose, ref.split(":", 1)[1]))[0]
    return skel.point(pose, ref)[0]


segments = body_segments(skel)
feet = [n for n in skel.names if "foot" in n.lower() and skel.parent.get(n)]
ref_pose = skel.component_pose(skel.ref_local)
ground = frame["center"][2]

# Which character bones an item may legitimately touch: the chain it hangs from and the hands
# that grip it.
def allowed_near(item):
    allowed = set()
    bone = skel.socket(item.attach)[0] if item.attach not in skel.index and skel.socket(item.attach) else item.attach
    for _ in range(3):
        if bone is None:
            break
        allowed.add(bone)
        bone = skel.parent.get(bone)
    for grip in item.grips:
        b = grip.get("bone")
        for _ in range(3):
            if b is None:
                break
            allowed.add(b)
            b = skel.parent.get(b)
    return allowed


rows = []
track = ARGS.get("track") or []
for t in times:
    local = skel.local_pose(anim, t)
    pose = skel.component_pose(local)
    p_pose = None
    if partner:
        pt = t + partner["time_offset"]
        p_pose = partner["skel"].component_pose(partner["skel"].local_pose(partner["anim"], pt))
    row = {"t": t, "points": {}, "checks": []}

    for ref in track:
        p = resolve(ref, pose, p_pose)
        b = to_body(frame, p)
        row["points"][ref] = {"fwd_right_up": rnd(b), "side": side(frame, p)}

    for item in items.values():
        near = allowed_near(item)
        for end in ("end_a", "end_b", "center"):
            p = item.point(pose, end)
            best = (1e9, None)
            for a, b in segments:
                if a in near or b in near:
                    continue
                d = segment_distance(p, pose[a][0], pose[b][0])
                if d < best[0]:
                    best = (d, b)
            key = "item:%s:%s clearance to own body" % (item.name, end)
            keep_min(key, round(best[0] - radius, 1), best[1], t)
            if best[0] < radius:
                problem(t, "clipping", "%s of %r is %.1f cm inside the body near %s" % (end, item.name, radius - best[0], best[1]))
        for grip in item.grips:
            g = item.point(pose, grip["socket"])
            h = skel.point(pose, grip["bone"])[0]
            d = length(sub(g, h))
            row["checks"].append({"grip": "%s.%s -> %s" % (item.name, grip["socket"], grip["bone"]), "distance": round(d, 1)})
            if d > float(grip.get("tolerance", touch)):
                problem(t, "grip", "%s is %.1f cm from %s's grip %r (tolerance %.1f)" % (grip["bone"], d, item.name, grip["socket"], float(grip.get("tolerance", touch))))

    for c in ARGS.get("contacts", []):
        d = length(sub(resolve(c["a"], pose, p_pose), resolve(c["b"], pose, p_pose)))
        expect = c.get("expect", "touch")
        limit = float(c.get("distance", touch if expect == "touch" else 10.0))
        row["checks"].append({"contact": "%s ~ %s" % (c["a"], c["b"]), "distance": round(d, 1), "expect": expect})
        in_window = not c.get("window") or (c["window"][0] <= t <= c["window"][1])
        if in_window and expect == "touch" and d > limit:
            problem(t, "contact", "%s and %s should touch but are %.1f cm apart" % (c["a"], c["b"], d))
        if in_window and expect == "apart" and d < limit:
            problem(t, "contact", "%s and %s should stay %.0f cm apart but are %.1f cm" % (c["a"], c["b"], limit, d))

    if partner:
        probes = [n for n in skel.names if any(k in n.lower() for k in ("hand", "foot", "head", "lowerarm", "calf"))
                  and not any(k in n.lower() for k in ("twist", "ik_", "finger"))]
        probe_points = [(n, pose[n][0]) for n in probes]
        for item in items.values():
            probe_points += [("item:%s:%s" % (item.name, e), item.point(pose, e)) for e in ("end_a", "end_b", "center")]
        for name, p in probe_points:
            best = (1e9, None)
            for a, b in partner["segments"]:
                pa = compose(partner["place"], p_pose[a])[0]
                pb = compose(partner["place"], p_pose[b])[0]
                d = segment_distance(p, pa, pb)
                if d < best[0]:
                    best = (d, b)
            key = "%s clearance to partner" % name
            keep_min(key, round(best[0] - radius, 1), best[1], t)
            allowed = any(c.get("expect", "touch") == "touch" and name in (c["a"], c["b"]) for c in ARGS.get("contacts", []))
            if best[0] < radius and not allowed:
                problem(t, "partner_clipping", "%s is %.1f cm inside the partner near %s" % (name, radius - best[0], best[1]))

    if feet:
        heights = dict((f, round(pose[f][0][2] - ground, 1)) for f in feet)
        row["feet_height"] = heights
        low = min(heights.values())
        if low < -2.0:
            problem(t, "ground", "a foot is %.1f cm below the reference ground" % -low)
    rows.append(row)

# Sides at the reference pose: the quickest way to catch an item on the wrong hand.
attach_sides = {}
for item in items.values():
    p = item.transform(ref_pose)[0]
    attach_sides[item.name] = {"attach": item.attach, "side": side(frame, p), "fwd_right_up": rnd(to_body(frame, p)),
                               "long_axis": item.long_axis, "length_cm": round(item.length, 1)}
    first = skel.component_pose(skel.local_pose(anim, times[0]))
    a, b = item.point(first, "end_a"), item.point(first, "end_b")
    attach_sides[item.name]["end_a_at_start"] = rnd(to_body(frame, a))
    attach_sides[item.name]["end_b_at_start"] = rnd(to_body(frame, b))

emit({
    "mesh": mesh.get_path_name(),
    "animation": anim.get_path_name() if anim else None,
    "length_s": anim_length(anim) if anim else 0.0,
    "frame": {
        "note": "Positions are [forward, right, up] in cm from the character's centre at ground level, derived from the skeleton's left/right bone pairs. right > 0 is the character's right hand side.",
        "right_axis_in_mesh_space": rnd(frame["right"], 3), "forward_axis_in_mesh_space": rnd(frame["forward"], 3),
        "left_right_pairs_found": frame["found_pairs"], "sample_pairs": frame["pairs"],
    },
    "attachments": attach_sides,
    "problems": problems,
    "passed": not problems,
    "closest_approach": dict((k, {"clearance_cm": v[0], "near": v[1], "time": v[2]}) for k, v in sorted(minimum.items())),
    "samples": rows,
})
