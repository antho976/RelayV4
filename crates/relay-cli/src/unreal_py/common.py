# Shared helpers for Relay's `unreal` MCP tools. The Rust side prepends `ARGS_JSON = "..."`.
import json, math
import unreal

ARGS = json.loads(ARGS_JSON)


def emit(value):
    print("RELAY_JSON:" + json.dumps(value))


def editor_world():
    return unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()


def actor_subsystem():
    return unreal.get_editor_subsystem(unreal.EditorActorSubsystem)


def load(path, what="asset"):
    asset = unreal.load_asset(path) if path else None
    if asset is None:
        raise RuntimeError("cannot load %s %r - check the path with ue_search_assets" % (what, path))
    return asset


def find_actor(key):
    for actor in actor_subsystem().get_all_level_actors():
        if actor.get_actor_label() == key or actor.get_path_name() == key or actor.get_name() == key:
            return actor
    raise RuntimeError("no actor labelled or named %r in the open level - list them with ue_level_actors" % key)


# ---- vector and quaternion math on plain tuples; quaternions are (x, y, z, w) like FQuat.

def add(a, b): return (a[0] + b[0], a[1] + b[1], a[2] + b[2])
def sub(a, b): return (a[0] - b[0], a[1] - b[1], a[2] - b[2])
def mul(a, k): return (a[0] * k, a[1] * k, a[2] * k)
def hadamard(a, b): return (a[0] * b[0], a[1] * b[1], a[2] * b[2])
def dot(a, b): return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
def cross(a, b): return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])
def length(a): return math.sqrt(dot(a, a))


def normalize(a):
    n = length(a)
    return (0.0, 0.0, 0.0) if n < 1e-9 else mul(a, 1.0 / n)


def qmul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return (aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw,
            aw * bw - ax * bx - ay * by - az * bz)


def qrot(q, p):
    u = (q[0], q[1], q[2])
    t = mul(cross(u, p), 2.0)
    return add(add(p, mul(t, q[3])), cross(u, t))


IDENTITY = ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0, 1.0), (1.0, 1.0, 1.0))


def compose(parent, local):
    """`local` expressed in `parent`'s space, returned in parent's parent space."""
    ppos, pq, ps = parent
    lpos, lq, ls = local
    return (add(ppos, qrot(pq, hadamard(ps, lpos))), qmul(pq, lq), hadamard(ps, ls))


def vec(v): return (float(v.x), float(v.y), float(v.z))


def quat_of_rotator(r):
    return tuple(float(c) for c in (lambda q: (q.x, q.y, q.z, q.w))(r.quaternion()))


def from_ue(t):
    q = t.rotation
    return (vec(t.translation), (float(q.x), float(q.y), float(q.z), float(q.w)), vec(t.scale3d))


def yaw_quat(degrees):
    h = math.radians(degrees) / 2.0
    return (0.0, 0.0, math.sin(h), math.cos(h))


def offset_transform(spec):
    """{location:[x,y,z], rotation:[pitch,yaw,roll]} -> transform tuple."""
    if not spec:
        return IDENTITY
    loc = tuple(float(c) for c in spec.get("location", [0, 0, 0]))
    rot = spec.get("rotation", [0, 0, 0])
    q = quat_of_rotator(unreal.Rotator(roll=float(rot[2]), pitch=float(rot[0]), yaw=float(rot[1])))
    return (loc, q, (1.0, 1.0, 1.0))


def rnd(p, digits=1):
    return [round(c, digits) for c in p]


# ---- skeletons

LEFT_RIGHT = [("_l", "_r"), ("_L", "_R"), (".l", ".r"), (".L", ".R"), ("Left", "Right"), ("left", "right"), ("l_", "r_"), ("L_", "R_")]


def mirror_name(name):
    """The right-side twin of a left-side bone name, or None."""
    for left, right in LEFT_RIGHT:
        if name.endswith(left):
            return name[: -len(left)] + right
        if name.startswith(left) and left.endswith("_"):
            return right + name[len(left):]
        if left in ("Left", "left") and left in name:
            return name.replace(left, right, 1)
    return None


class Skeleton(object):
    """The bone hierarchy of a skeletal mesh, read through an unregistered component."""

    def __init__(self, mesh):
        self.mesh = mesh
        comp = unreal.new_object(unreal.SkeletalMeshComponent)
        setter = getattr(comp, "set_skeletal_mesh_asset", None) or getattr(comp, "set_skeletal_mesh")
        setter(mesh)
        self.comp = comp
        count = comp.get_num_bones()
        if count == 0:
            raise RuntimeError("%s has no bones" % mesh.get_path_name())
        self.names = [str(comp.get_bone_name(i)) for i in range(count)]
        self.index = dict((n, i) for i, n in enumerate(self.names))
        self.parent = {}
        for n in self.names:
            p = str(comp.get_parent_bone(n))
            self.parent[n] = p if p and p != "None" and p in self.index else None
        self.ref_local = {}
        for i, n in enumerate(self.names):
            getter = getattr(comp, "get_ref_pose_transform", None)
            if getter is not None:
                self.ref_local[n] = from_ue(getter(i))
            else:
                self.ref_local[n] = (vec(comp.get_ref_pose_position(i)), (0.0, 0.0, 0.0, 1.0), (1.0, 1.0, 1.0))

    def local_pose(self, anim, time):
        if anim is None:
            return dict(self.ref_local)
        pose = {}
        for n in self.names:
            try:
                pose[n] = from_ue(unreal.AnimationLibrary.get_bone_pose_for_time(anim, n, time, False))
            except Exception:
                pose[n] = self.ref_local[n]
        return pose

    def component_pose(self, local):
        out = {}
        for n in self.names:  # parents always precede children in a reference skeleton
            p = self.parent[n]
            out[n] = local[n] if p is None else compose(out[p], local[n])
        return out

    def socket(self, name):
        """(parent bone, relative transform) of a mesh or skeleton socket, or None."""
        s = self.mesh.find_socket(name)
        if s is None:
            return None
        rel = (vec(s.get_editor_property("relative_location")),
               quat_of_rotator(s.get_editor_property("relative_rotation")),
               vec(s.get_editor_property("relative_scale")))
        return str(s.get_editor_property("bone_name")), rel

    def point(self, pose, name):
        """Component-space transform of a bone or socket."""
        if name in pose:
            return pose[name]
        s = self.socket(name)
        if s is None or s[0] not in pose:
            raise RuntimeError("%r is neither a bone nor a socket of %s" % (name, self.mesh.get_name()))
        return compose(pose[s[0]], s[1])


def anim_length(anim):
    for getter in (lambda: anim.get_play_length(), lambda: unreal.AnimationLibrary.get_sequence_length(anim)):
        try:
            return float(getter())
        except Exception:
            pass
    return 0.0


def sample_times(anim):
    if ARGS.get("times"):
        return [float(t) for t in ARGS["times"]]
    if anim is None:
        return [0.0]
    n = max(2, min(int(ARGS.get("samples", 9)), 60))
    total = anim_length(anim)
    return [round(total * i / (n - 1), 4) for i in range(n)]


def body_frame(skel):
    """Character axes in mesh space from the skeleton itself: `right` points from left-side
    bones to their right-side twins, `up` is +Z, `forward` completes the frame. This is why the
    checks work for any skeleton and any mesh orientation."""
    comp = skel.component_pose(skel.ref_local)
    pairs = []
    for n in skel.names:
        twin = mirror_name(n)
        if twin and twin in comp and twin != n:
            pairs.append((n, twin))
    lateral = (0.0, 0.0, 0.0)
    for left, right in pairs:
        lateral = add(lateral, sub(comp[right][0], comp[left][0]))
    up = (0.0, 0.0, 1.0)
    right = normalize(sub(lateral, mul(up, dot(lateral, up))))
    if length(right) < 0.5:
        right = (0.0, 1.0, 0.0)
    forward = normalize(cross(right, up))
    feet = [comp[n][0][2] for n in skel.names if "foot" in n.lower() or "ball" in n.lower() or "toe" in n.lower()]
    ground = min(feet) if feet else min(p[0][2] for p in comp.values())
    center = (0.0, 0.0, 0.0)
    if pairs:
        for left, right_name in pairs:
            center = add(center, mul(add(comp[left][0], comp[right_name][0]), 0.5))
        center = mul(center, 1.0 / len(pairs))
    return {"right": right, "forward": forward, "up": up, "center": (center[0], center[1], ground),
            "pairs": pairs[:6], "found_pairs": len(pairs)}


def to_body(frame, p):
    d = sub(p, frame["center"])
    return (dot(d, frame["forward"]), dot(d, frame["right"]), dot(d, frame["up"]))


def side(frame, p, tolerance=3.0):
    r = to_body(frame, p)[1]
    return "right" if r > tolerance else ("left" if r < -tolerance else "center")


SKIP_SEGMENT = ("finger", "thumb", "index", "middle", "ring", "pinky", "metacarpal", "twist", "ik_", "weapon",
                "prop", "attach", "socket", "root", "camera", "correct", "_end", "tip", "eye", "jaw", "tongue")


def body_segments(skel):
    segs = []
    for n in skel.names:
        p = skel.parent[n]
        low = n.lower()
        if p is None or skel.parent.get(p) is None or any(k in low for k in SKIP_SEGMENT):
            continue
        segs.append((p, n))
    return segs


def segment_distance(p, a, b):
    ab = sub(b, a)
    denom = dot(ab, ab)
    t = 0.0 if denom < 1e-9 else max(0.0, min(1.0, dot(sub(p, a), ab) / denom))
    return length(sub(p, add(a, mul(ab, t))))
