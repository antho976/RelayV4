# Shared helpers for Relay's Blender tools. Blender runs in background mode:
#   blender -b [file.blend] --factory-startup --python-exit-code 1 --python <script> -- <args.json>
import bpy, json, math, sys
from mathutils import Vector, Matrix, Quaternion, Euler

ARGS = json.load(open(sys.argv[sys.argv.index("--") + 1]))
scene = bpy.context.scene
# Scene units: metres per Blender unit. Every length the tools report is in centimetres, like Unreal.
TO_CM = 100.0 * (scene.unit_settings.scale_length or 1.0)


def emit(value):
    # The real stdout: blender_python captures sys.stdout while the agent's code runs.
    print("RELAY_JSON:" + json.dumps(value), file=sys.__stdout__, flush=True)


def own_rotation(o):
    """The object's own rotation in whichever mode it uses (glTF imports are QUATERNION), delta
    rotation included."""
    mode = o.rotation_mode
    if mode == "QUATERNION":
        return o.delta_rotation_quaternion.normalized() @ o.rotation_quaternion.normalized()
    if mode == "AXIS_ANGLE":
        angle, *axis = o.rotation_axis_angle
        return Quaternion(Vector(axis), angle) if Vector(axis).length > 1e-9 else Quaternion()
    return Euler([a + d for a, d in zip(o.rotation_euler, o.delta_rotation_euler)], mode).to_quaternion()


def unapplied_rotation(o, tolerance=1e-4):
    """The object's rotation as XYZ Euler degrees when it is not applied, else None."""
    q = own_rotation(o)
    if min(q.angle, 2 * math.pi - q.angle) <= tolerance:
        return None
    return rnd([math.degrees(a) for a in q.to_euler()])


def skinned_meshes(arm):
    """Meshes the armature deforms, as the FBX exporter sees them: an Armature modifier on it, or
    the legacy armature parent. Props parented to a bone are not skin."""
    return [o for o in scene.objects if o.type == "MESH" and (
        any(m.type == "ARMATURE" and m.object == arm for m in o.modifiers) or (o.parent == arm and o.parent_type == "ARMATURE"))]


class rest_pose:
    """`with rest_pose(armatures):` evaluates their meshes in bind pose, which is what the FBX
    exporter writes for skinned meshes and what Unreal measures after import."""
    def __init__(self, armatures):
        self.arms = [a for a in armatures if a is not None and a.type == "ARMATURE"]

    def __enter__(self):
        self.saved = [(a.data, a.data.pose_position) for a in self.arms]
        for data, _ in self.saved:
            data.pose_position = "REST"
        bpy.context.view_layer.update()

    def __exit__(self, *exc):
        for data, position in self.saved:
            data.pose_position = position
        bpy.context.view_layer.update()


def obj(name, kind=None):
    o = bpy.data.objects.get(name)
    if o is None:
        raise RuntimeError("no object %r in %s; objects: %s" % (name, bpy.data.filepath or "the scene", ", ".join(sorted(bpy.data.objects.keys())[:40])))
    if kind and o.type != kind:
        raise RuntimeError("%r is a %s, not a %s" % (name, o.type, kind))
    return o


def first(kind):
    for o in scene.objects:
        if o.type == kind:
            return o
    return None


def cm(v):
    return [round(c * TO_CM, 1) for c in v]


def rnd(v, digits=1):
    return [round(c, digits) for c in v]


# ---- left/right from names (Blender: .L/.R, _L/_R, Left/Right, l_/r_)

PAIRS = [(".L", ".R"), (".l", ".r"), ("_L", "_R"), ("_l", "_r"), ("-L", "-R"), ("Left", "Right"), ("left", "right")]


def twin(name):
    for left, right in PAIRS:
        if name.endswith(left):
            return name[: -len(left)] + right
        if left in ("Left", "left") and left in name:
            return name.replace(left, right, 1)
    for left, right in (("l_", "r_"), ("L_", "R_")):
        if name.startswith(left):
            return right + name[len(left):]
    return None


def left_right_pairs(names):
    names = set(names)
    return [(n, twin(n)) for n in sorted(names) if twin(n) in names and twin(n) != n]


def body_frame(arm):
    """Character axes in world space from the armature's own bone pairs at rest. Blender is
    right-handed Z-up: forward = up x right. Without pairs, Blender's front convention (-Y) is used."""
    pairs = left_right_pairs([b.name for b in arm.data.bones])
    mw = arm.matrix_world
    lateral = Vector((0, 0, 0))
    center = Vector((0, 0, 0))
    for l, r in pairs:
        hl, hr = mw @ arm.data.bones[l].head_local, mw @ arm.data.bones[r].head_local
        lateral += hr - hl
        center += (hl + hr) / 2
    up = Vector((0, 0, 1))
    right = lateral - up * lateral.dot(up)
    if right.length < 1e-6:
        right = Vector((-1, 0, 0))
    right.normalize()
    forward = up.cross(right).normalized()
    heads = [mw @ b.head_local for b in arm.data.bones] + [mw @ b.tail_local for b in arm.data.bones]
    ground = min(h.z for h in heads)
    if pairs:
        center /= len(pairs)
    else:
        center = sum(heads, Vector((0, 0, 0))) / len(heads)
    center.z = ground
    return {"right": right, "forward": forward, "up": up, "center": center, "pairs": pairs}


def to_body(frame, p):
    d = p - frame["center"]
    return Vector((d.dot(frame["forward"]), d.dot(frame["right"]), d.dot(frame["up"]))) * TO_CM


def side(frame, p, tolerance_cm=3.0):
    r = to_body(frame, p).y
    return "right" if r > tolerance_cm else ("left" if r < -tolerance_cm else "center")


def world_bbox(objects):
    lo, hi = None, None
    deps = bpy.context.evaluated_depsgraph_get()
    for o in objects:
        e = o.evaluated_get(deps)
        for c in e.bound_box:
            p = e.matrix_world @ Vector(c)
            lo = p.copy() if lo is None else Vector(map(min, lo, p))
            hi = p.copy() if hi is None else Vector(map(max, hi, p))
    return lo, hi


def segment_distance(p, a, b):
    ab = b - a
    denom = ab.dot(ab)
    t = 0.0 if denom < 1e-12 else max(0.0, min(1.0, (p - a).dot(ab) / denom))
    return (p - (a + ab * t)).length


def set_action(arm, action_name):
    if not action_name:
        return None
    action = bpy.data.actions.get(action_name)
    if action is None:
        raise RuntimeError("no action %r; actions: %s" % (action_name, ", ".join(bpy.data.actions.keys())))
    if arm.animation_data is None:
        arm.animation_data_create()
    ad = arm.animation_data
    ad.action = action

    def resolving(curves):
        n = 0
        for fc in curves:
            try:
                arm.path_resolve(fc.data_path, False)
                n += 1
            except ValueError:
                pass
        return n

    # Blender 4.4+ animates through a slot. Assignment picks one only when the action has a slot
    # this object used before (or one never used); another rig's action, an appended or an
    # FBX-imported clip is left without one and plays nothing. Pick the slot whose curves drive
    # this rig, as the FBX exporter does, else the only object slot.
    slots = list(getattr(action, "slots", ()))
    if hasattr(ad, "action_slot") and ad.action_slot is None and slots:
        scores = [(resolving(action_fcurves(action, s)), s) for s in slots]
        best = max(n for n, _ in scores)
        fit = [s for n, s in scores if n == best] if best else [s for s in slots if s.target_id_type in ("OBJECT", "UNSPECIFIED")]
        if len(fit) != 1:
            raise RuntimeError("action %r has %s for %s; slots: %s" % (
                action.name, "several slots that fit" if fit else "no slot", arm.name, ", ".join(s.identifier for s in slots)))
        ad.action_slot = fit[0]
    curves = action_fcurves(action, getattr(ad, "action_slot", None))
    if curves and not resolving(curves):
        keyed = sorted(set(fc.data_path.split('"')[1] for fc in curves if fc.data_path.startswith("pose.bones[")))
        raise RuntimeError("action %r animates nothing on %s: none of its %d curves match it (keyed bones: %s; the rig's bones: %s)" % (
            action.name, arm.name, len(curves), ", ".join(keyed[:8]) or "none", ", ".join(b.name for b in arm.data.bones[:8]) if arm.type == "ARMATURE" else "-"))
    return action


def action_used(o):
    """The action and slot an object is animated by, for a tool's result."""
    ad = o.animation_data if o is not None else None
    if ad is None or ad.action is None:
        return {"action": None, "slot": None}
    slot = getattr(ad, "action_slot", None)
    return {"action": ad.action.name, "slot": slot.identifier if slot is not None else None}


def action_fcurves(action, slot=None):
    """An action's F-curves, on any Blender. 4.4 made actions layered (one channelbag per slot)
    and 5.0 removed Action.fcurves; on 4.4/4.5 that legacy view shows only the first slot.
    With `slot`, only the curves animating that slot, so one action shared by several objects
    does not lend one object the bones of another."""
    if getattr(action, "layers", None):
        curves = []
        for layer in action.layers:
            for strip in layer.strips:
                for bag in getattr(strip, "channelbags", ()):
                    if slot is None or getattr(bag, "slot", slot) == slot:
                        curves.extend(bag.fcurves)
        return curves
    return list(getattr(action, "fcurves", ()))


def action_range(arm):
    ad = arm.animation_data
    if ad and ad.action:
        a, b = ad.action.frame_range
        return int(math.floor(a)), int(math.ceil(b))
    return scene.frame_start, scene.frame_end


def sample_frames(arm):
    if ARGS.get("frames"):
        return [int(f) for f in ARGS["frames"]]
    a, b = action_range(arm)
    n = max(2, min(int(ARGS.get("samples", 9)), 60))
    return sorted(set(int(round(a + (b - a) * i / (n - 1))) for i in range(n)))


def mesh_issues(o):
    """Problems in a mesh as it will be exported: modifiers applied (a bevel wider than a thin
    part only shows up here), skin in bind pose as the FBX exporter writes it. Returns the counts
    and (level, problem, fix) entries: `problem` breaks rendering in Unreal, `warning` is worth a
    look. The one implementation behind mesh_check, export and rig_check."""
    import bmesh
    with rest_pose([m.object for m in o.modifiers if m.type == "ARMATURE"] + [o.parent if o.parent_type == "ARMATURE" else None]):
        ev = o.evaluated_get(bpy.context.evaluated_depsgraph_get())
        me = ev.to_mesh()
        try:
            bm = bmesh.new()
            bm.from_mesh(me)
            counts = {"triangles": sum(len(p.vertices) - 2 for p in me.polygons),
                      "degenerate_faces": sum(1 for f in bm.faces if f.calc_area() < 1e-10),
                      "zero_length_edges": sum(1 for e in bm.edges if e.calc_length() < 1e-6),
                      "loose_vertices": sum(1 for v in bm.verts if not v.link_edges),
                      "non_manifold_edges": sum(1 for e in bm.edges if not e.is_manifold),
                      "ngons": sum(1 for f in bm.faces if len(f.verts) > 4)}
            closed = counts["non_manifold_edges"] == 0 and len(bm.faces) > 0
            volume = bm.calc_volume(signed=True) if closed else None
            bm.free()
            has_uv = bool(me.uv_layers)
        finally:
            ev.to_mesh_clear()
    issues = []
    if counts["degenerate_faces"]:
        issues.append(("problem", "%d zero-area faces (often a bevel or solidify wider than a thin part)" % counts["degenerate_faces"], "lower the modifier's width, or Mesh > Clean Up > Degenerate Dissolve after applying"))
    if counts["zero_length_edges"]:
        issues.append(("problem", "%d zero-length edges" % counts["zero_length_edges"], "Mesh > Clean Up > Merge by Distance"))
    if volume is not None and volume < 0:
        issues.append(("problem", "normals point inward (negative volume)", "Mesh > Normals > Recalculate Outside"))
    if counts["loose_vertices"]:
        issues.append(("warning", "%d loose vertices" % counts["loose_vertices"], "Mesh > Clean Up > Delete Loose"))
    if counts["ngons"]:
        issues.append(("warning", "%d n-gons: the FBX exporter skips tangents for this mesh" % counts["ngons"], "triangulate or quad them"))
    if not has_uv:
        issues.append(("warning", "no UV map: textures and lightmaps need one", "unwrap it (a second UV channel too if you use baked lightmaps)"))
    return counts, issues


def mesh_report(o):
    counts, issues = mesh_issues(o)
    report = {"object": o.name}
    report.update(counts)
    report["problems"] = ["%s - %s" % (text, fix) for level, text, fix in issues if level == "problem"]
    report["warnings"] = ["%s - %s" % (text, fix) for level, text, fix in issues if level == "warning"]
    return report
