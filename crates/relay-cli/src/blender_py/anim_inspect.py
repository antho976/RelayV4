# blender_anim_inspect: the same measurements as Unreal's ue_anim_inspect, on a Blender action,
# before anything is exported. Items are objects already attached in the scene (bone parent or
# Child Of constraint); sockets are empties. The checks are anim_rules.py's; this script only
# poses the rigs and hands over points in centimetres.

arm = obj(ARGS["armature"], "ARMATURE") if ARGS.get("armature") else first("ARMATURE")
if arm is None:
    raise RuntimeError("no armature; pass armature")
frame = body_frame(arm)  # rest pose, before any action is applied
set_action(arm, ARGS.get("action"))
frames = sample_frames(arm)

partner = None
if ARGS.get("partner"):
    p = ARGS["partner"]
    partner = obj(p["armature"], "ARMATURE")
    set_action(partner, p.get("action"))


def segments(a):
    return [b.name for b in a.data.bones if b.use_deform and b.parent is not None and body_bone(b.name)]


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


def bone_ref(a, ref):
    """`bone` is the bone's head, `bone:tail` its tail."""
    name = ref[:-5] if ref.endswith(":tail") else ref
    if name not in a.pose.bones:
        raise RuntimeError("no bone %r in %s" % (name, a.name))
    return tail(a, name) if ref.endswith(":tail") else head(a, name)


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
        if partner is None:
            raise RuntimeError("%r needs a partner" % ref)
        return bone_ref(partner, ref.split(":", 1)[1])
    return bone_ref(arm, ref)


def cm3(v):
    return (v.x * TO_CM, v.y * TO_CM, v.z * TO_CM)


def foot_chain(f):
    """A foot and the deform bones below it (toes)."""
    out, todo = [], [arm.data.bones[f]]
    while todo:
        b = todo.pop()
        out.append(b)
        todo.extend(c for c in b.children if c.use_deform)
    return out


class Rig(object):
    def __init__(self):
        self.frame = {"center": cm3(frame["center"]), "forward": tuple(frame["forward"]), "right": tuple(frame["right"]), "up": tuple(frame["up"])}
        self.probes = [b.name for b in arm.data.bones if b.use_deform and probe_bone(b.name)]
        self.items = [{"name": n, "holder": it["holder"], "grips": [dict(g, label=g["point"]) for g in it["grips"]]} for n, it in items.items()]
        feet = [b.name for b in arm.data.bones if b.use_deform and foot_bone(b.name)]
        self.feet = dict((f, [r for c in foot_chain(f) for r in (c.name, c.name + ":tail")]) for f in feet)
        self.rest_feet = dict((f, [cm3(arm.matrix_world @ p) for c in foot_chain(f) for p in (c.head_local, c.tail_local)]) for f in feet)

    def pose(self, f):
        scene.frame_set(f)

    def point(self, ref):
        return cm3(resolve(ref))

    def owner(self, ref):
        if ref.startswith(("item:", "obj:", "partner:")):
            return None
        name = ref[:-5] if ref.endswith(":tail") else ref
        return name if name in arm.data.bones else None

    def parent(self, bone):
        b = arm.data.bones.get(bone)
        return b.parent.name if b is not None and b.parent is not None else None

    def segments(self):
        return [(s, (s,), cm3(head(arm, s)), cm3(tail(arm, s))) for s in arm_segments]

    def partner_segments(self):
        return [(s, (s,), cm3(head(partner, s)), cm3(tail(partner, s))) for s in p_segments]

    def partner_bones(self, ref):
        # A bone's head is where its parent ends; its tail is where its children start.
        name = ref.split(":", 1)[1]
        at_tail = name.endswith(":tail")
        b = partner.data.bones.get(name[:-5] if at_tail else name) if partner else None
        if b is None:
            return None
        return [x.name for x in [b] + (list(b.children) if at_tail else [b.parent] if b.parent else [])]


result = inspect_animation(Rig(), frames, ARGS, key="frame")
problems = result["problems"]

scene.frame_set(frames[0])
attach = {}
for name, it in items.items():
    o = it["object"]
    attach[name] = {"object": o.name, "held_by": it["holder"], "side": side(frame, o.matrix_world.translation),
                    "fwd_right_up": rnd(to_body(frame, o.matrix_world.translation)), "long_axis": it["points"]["_axis"],
                    "length_cm": round((o.matrix_world @ it["points"]["end_a"] - o.matrix_world @ it["points"]["end_b"]).length * TO_CM, 1),
                    "end_a_at_start": rnd(to_body(frame, o.matrix_world @ it["points"]["end_a"])),
                    "end_b_at_start": rnd(to_body(frame, o.matrix_world @ it["points"]["end_b"]))}
    if it["holder"] is None:
        problems.append({"frame": frames[0], "kind": "attachment", "detail": "%r is not parented to a bone of %s (or held by a Child Of constraint); it will not follow the hand" % (name, arm.name)})

emit({"armature": arm.name, "action": arm.animation_data.action.name if arm.animation_data and arm.animation_data.action else None,
      "frames": frames,
      "frame": {"note": "Positions are [forward, right, up] in cm from the character's centre at ground level, from the rig's own .L/.R bone pairs; right > 0 is the character's right.",
                "forward_world": rnd(frame["forward"], 3), "right_world": rnd(frame["right"], 3), "left_right_pairs_found": len(frame["pairs"])},
      "attachments": attach, "problems": problems, "passed": not problems,
      "closest_approach": result["closest_approach"],
      "samples": result["samples"]})
