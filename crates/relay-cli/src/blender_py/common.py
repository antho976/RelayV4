# Shared helpers for Relay's Blender tools. Blender runs in background mode:
#   blender -b [file.blend] --factory-startup --python-exit-code 1 --python <script> -- <args.json>
import bpy, json, math, sys
from mathutils import Vector, Matrix

ARGS = json.load(open(sys.argv[sys.argv.index("--") + 1]))
scene = bpy.context.scene
# Scene units: metres per Blender unit. Every length the tools report is in centimetres, like Unreal.
TO_CM = 100.0 * (scene.unit_settings.scale_length or 1.0)


def emit(value):
    print("RELAY_JSON:" + json.dumps(value))


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
    arm.animation_data.action = action
    return action


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


def mesh_report(o):
    """Problems in a mesh as it will be exported: modifiers applied (a bevel wider than a thin
    part only shows up here). `problems` break rendering in Unreal; `warnings` are worth a look."""
    import bmesh
    deps = bpy.context.evaluated_depsgraph_get()
    ev = o.evaluated_get(deps)
    me = ev.to_mesh()
    try:
        bm = bmesh.new()
        bm.from_mesh(me)
        degenerate = sum(1 for f in bm.faces if f.calc_area() < 1e-10)
        zero_edges = sum(1 for e in bm.edges if e.calc_length() < 1e-6)
        loose = sum(1 for v in bm.verts if not v.link_edges)
        non_manifold = sum(1 for e in bm.edges if not e.is_manifold)
        ngons = sum(1 for f in bm.faces if len(f.verts) > 4)
        closed = non_manifold == 0 and len(bm.faces) > 0
        volume = bm.calc_volume(signed=True) if closed else None
        bm.free()
        problems, warnings = [], []
        if degenerate:
            problems.append("%d zero-area faces (often a bevel or solidify wider than a thin part) - lower the modifier's width, or Mesh > Clean Up > Degenerate Dissolve after applying" % degenerate)
        if zero_edges:
            problems.append("%d zero-length edges - Mesh > Clean Up > Merge by Distance" % zero_edges)
        if volume is not None and volume < 0:
            problems.append("normals point inward (negative volume) - Mesh > Normals > Recalculate Outside")
        if loose:
            warnings.append("%d loose vertices - Mesh > Clean Up > Delete Loose" % loose)
        if ngons:
            warnings.append("%d n-gons: the FBX exporter skips tangents for this mesh - triangulate or quad them" % ngons)
        if not me.uv_layers:
            warnings.append("no UV map: textures and lightmaps need one")
        return {"object": o.name, "triangles": sum(len(p.vertices) - 2 for p in me.polygons),
                "degenerate_faces": degenerate, "zero_length_edges": zero_edges, "loose_vertices": loose,
                "non_manifold_edges": non_manifold, "ngons": ngons, "problems": problems, "warnings": warnings}
    finally:
        ev.to_mesh_clear()
