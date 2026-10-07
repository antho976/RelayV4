# ue_anim_inspect: numbers, not impressions. Poses a skeleton at sample times straight from the
# animation data (no level, no ticking), carries attached items and an optional partner along,
# and reports sides, contacts, clearances and ground contact in the character's own frame. The
# checks are anim_rules.py's, shared with blender_anim_inspect; this script only poses.

mesh = load(ARGS["mesh"], "skeletal mesh")
anim = load_animation(ARGS["animation"]) if ARGS.get("animation") else None
skel = Skeleton(mesh)
frame = body_frame(skel)
times = sample_times(anim)
ref_pose = skel.component_pose(skel.ref_local)


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
        "anim": load_animation(spec["animation"], "partner animation") if spec.get("animation") else None,
        "place": (tuple(float(c) for c in spec.get("location", [0, 150, 0])), yaw_quat(float(spec.get("yaw", 180))), (1.0, 1.0, 1.0)),
        "time_offset": float(spec.get("time_offset", 0.0)),
    }


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


def body_segments(skel):
    """(parent, child) joint pairs that carry body volume; a bone hanging from the root is not."""
    return [(n, (skel.parent[n], n)) for n in skel.names
            if skel.parent[n] is not None and skel.parent.get(skel.parent[n]) is not None and body_bone(n)]


def owner(sk, ref):
    """The bone a bone or socket name sits on, or None."""
    if ref in sk.index:
        return ref
    s = sk.socket(ref)
    return s[0] if s and s[0] in sk.index else None


def below(sk, bone):
    """A bone and every bone under it."""
    out, todo = [], [bone]
    while todo:
        b = todo.pop()
        out.append(b)
        todo.extend(n for n in sk.names if sk.parent[n] == b)
    return out


if partner:
    partner["segments"] = body_segments(partner["skel"])


class Rig(object):
    """What anim_rules needs, in mesh space: a segment is a (parent, child) joint pair and exempts
    by either joint; partner points are carried into this character's space by its placement."""

    def __init__(self):
        self.frame = frame
        self.body = body_segments(skel)
        self.probes = [n for n in skel.names if probe_bone(n)]
        self.items = [{"name": it.name, "holder": owner(skel, it.attach),
                       "grips": [dict(g, label="%s.%s" % (it.name, g["socket"]), point="item:%s:%s" % (it.name, g["socket"])) for g in it.grips]}
                      for it in items.values()]
        feet = [n for n in skel.names if foot_bone(n) and skel.parent.get(n)]
        self.feet = dict((f, below(skel, f)) for f in feet)
        self.rest_feet = dict((f, [ref_pose[n][0] for n in chain]) for f, chain in self.feet.items())
        self.pose_at, self.p_pose = None, None

    def pose(self, t):
        self.pose_at = skel.component_pose(skel.local_pose(anim, t))
        if partner:
            p = partner["skel"]
            self.p_pose = p.component_pose(p.local_pose(partner["anim"], t + partner["time_offset"]))

    def point(self, ref):
        return resolve(ref, self.pose_at, self.p_pose)

    def owner(self, ref):
        return None if ref.startswith(("item:", "partner:")) else owner(skel, ref)

    def parent(self, bone):
        return skel.parent.get(bone)

    def segments(self):
        return [(n, ab, self.pose_at[ab[0]][0], self.pose_at[ab[1]][0]) for n, ab in self.body]

    def partner_segments(self):
        if not partner:
            return []
        place = partner["place"]
        return [(n, ab, compose(place, self.p_pose[ab[0]])[0], compose(place, self.p_pose[ab[1]])[0]) for n, ab in partner["segments"]]

    def partner_bones(self, ref):
        bone = owner(partner["skel"], ref.split(":", 1)[1]) if partner else None
        return [bone] if bone else None


result = inspect_animation(Rig(), times, ARGS, key="time", row_key="t")
# A bone the animation could not pose was measured at the reference pose: say so, or the report
# passes for an animation that never played.
for who, sk in [("", skel)] + ([("partner ", partner["skel"])] if partner else []):
    if sk.fallbacks:
        result["problems"].append({"time": None, "kind": "sampling", "detail": "%d %sbones could not be posed from the animation and were measured at the reference pose: %s" % (
            len(sk.fallbacks), who, ", ".join(sorted(sk.fallbacks)[:8]))})

# Sides at the reference pose: the quickest way to catch an item on the wrong hand.
attach_sides = {}
first = skel.component_pose(skel.local_pose(anim, times[0])) if items else None
for item in items.values():
    p = item.transform(ref_pose)[0]
    attach_sides[item.name] = {"attach": item.attach, "side": side(frame, p), "fwd_right_up": rnd(to_body(frame, p)),
                               "long_axis": item.long_axis, "length_cm": round(item.length, 1)}
    a, b = item.point(first, "end_a"), item.point(first, "end_b")
    attach_sides[item.name]["end_a_at_start"] = rnd(to_body(frame, a))
    attach_sides[item.name]["end_b_at_start"] = rnd(to_body(frame, b))

emit({
    "mesh": mesh.get_path_name(),
    "animation": anim.get_path_name() if anim else None,
    "length_s": anim_length(anim) if anim else 0.0,
    "frame": {
        "note": "Positions are [forward, right, up] in cm from the character's centre at ground level, derived from the skeleton's left/right bone pairs. right > 0 is the character's right hand side."
                + ("" if frame["found_pairs"] else " No left/right pairs were found, so the axes are assumed: forward +Y, right -X in mesh space."),
        "right_axis_in_mesh_space": rnd(frame["right"], 3), "forward_axis_in_mesh_space": rnd(frame["forward"], 3),
        "left_right_pairs_found": frame["found_pairs"], "sample_pairs": frame["pairs"][:6],
    },
    "attachments": attach_sides,
    "problems": result["problems"],
    "passed": not result["problems"],
    "closest_approach": result["closest_approach"],
    "samples": result["samples"],
})
