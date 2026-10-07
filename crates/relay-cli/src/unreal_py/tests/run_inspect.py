# Runs anim_inspect.py against the fake `unreal` module and checks what it concludes.
# Usage: python3 run_inspect.py   (exit status 0 when every check holds)
import io, json, os, sys, contextlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
SRC = os.path.dirname(HERE)
import unreal


def inspect(args):
    # The bundle unreal.rs sends: arguments, common.py with the shared rig_frame.py, the shared
    # anim_rules.py, the script.
    shared = lambda name: open(os.path.join(os.path.dirname(SRC), name)).read()
    code = "ARGS_JSON = %s\n%s\n%s\n%s\n%s" % (json.dumps(json.dumps(args)),
                                               open(os.path.join(SRC, "common.py")).read(), shared("rig_frame.py"),
                                               shared("anim_rules.py"),
                                           open(os.path.join(SRC, "anim_inspect.py")).read())
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        exec(compile(code, "anim_inspect.py", "exec"), {})
    line = [l for l in out.getvalue().splitlines() if l.startswith("RELAY_JSON:")][-1]
    return json.loads(line[len("RELAY_JSON:"):])


def check(cond, what):
    if not cond:
        print("FAIL:", what)
        sys.exit(1)


sword = {"name": "sword", "mesh": "/Game/Sword", "socket": "weapon_r", "grips": [{"socket": "Grip", "bone": "hand_r"}]}

# The frame comes from the skeleton: this mesh faces +Y, so the character's right is -X.
base = inspect({"mesh": "/Game/Manny", "track": ["hand_r", "hand_l", "foot_l"]})
check(base["frame"]["right_axis_in_mesh_space"] == [-1.0, 0.0, 0.0], "right axis from bone pairs")
check(base["frame"]["forward_axis_in_mesh_space"][1] == 1.0, "forward axis is +Y")
pts = base["samples"][0]["points"]
check(pts["hand_r"]["side"] == "right" and pts["hand_l"]["side"] == "left", "hands on their own sides")
check(abs(pts["foot_l"]["fwd_right_up"][2]) < 0.01, "feet define the ground")
check(base["passed"], "the reference pose has no problems")

# The item hangs from the right hand, but its long axis (+X in item space) points to the
# character's left, straight through the torso: that is the classic mis-rotated attachment.
wrong = inspect({"mesh": "/Game/Manny", "attachments": [sword]})
check(wrong["attachments"]["sword"]["side"] == "right", "attached on the right")
check(any(p["kind"] == "clipping" for p in wrong["problems"]), "blade through the body is reported")
check(not any(p["kind"] == "grip" for p in wrong["problems"]), "grip at the hand passes")

# Turned 180 degrees about yaw, the blade points away from the body and nothing clips.
right = inspect({"mesh": "/Game/Manny", "attachments": [dict(sword, rotation=[0, 180, 0])]})
check(not any(p["kind"] == "clipping" for p in right["problems"]), "a correctly rotated item passes: %s" % right["problems"])
check(right["attachments"]["sword"]["end_a_at_start"][1] > right["attachments"]["sword"]["fwd_right_up"][1] + 50, "item extends outward")
check(wrong["attachments"]["sword"]["end_a_at_start"][1] < wrong["attachments"]["sword"]["fwd_right_up"][1] - 50, "the wrong way round points inward")

# Moving the grip away from the hand fails the grip check.
off = inspect({"mesh": "/Game/Manny", "attachments": [dict(sword, location=[0, 0, 12])]})
check(any(p["kind"] == "grip" for p in off["problems"]), "a grip 12 cm off the hand is reported")

# Contacts across an animation: the right hand swings across; hands must stay apart.
swing_args = {"mesh": "/Game/Manny", "animation": "/Game/Swing", "samples": 5,
              "contacts": [{"a": "hand_r", "b": "hand_l", "expect": "apart", "distance": 10}],
              "track": ["hand_r"]}
swing = inspect(swing_args)
sides = [r["points"]["hand_r"]["side"] for r in swing["samples"]]
check(sides[0] == "right" and sides[-1] == "left", "the swing crosses the body: %s" % sides)
check(swing["passed"], "a fully posed animation has no sampling problem: %s" % swing["problems"])

# The same numbers on an engine without AnimPoseExtensions (one call per bone).
pose_ext = unreal.AnimPoseExtensions
del unreal.AnimPoseExtensions
per_bone = inspect(swing_args)
unreal.AnimPoseExtensions = pose_ext
check(per_bone["samples"] == swing["samples"], "the per-bone fallback poses the same: %s" % per_bone["samples"])

# A bone the animation cannot pose stays at the reference pose, and the report says so.
real_pose = unreal.AnimationLibrary.get_bone_pose_for_time
def failing(anim, bone, t, rm):
    if bone == "hand_l":
        raise TypeError("cannot pose")
    return real_pose(anim, bone, t, rm)
unreal.AnimationLibrary.get_bone_pose_for_time = staticmethod(failing)
unposed = inspect({"mesh": "/Game/Manny", "animation": "/Game/Swing", "samples": 3})
unreal.AnimationLibrary.get_bone_pose_for_time = staticmethod(real_pose)
check(any(p["kind"] == "sampling" and "hand_l" in p["detail"] for p in unposed["problems"]), "unposed bones are reported: %s" % unposed["problems"])

# Only an animation sequence or montage can be sampled; a Blend Space is refused, not measured at rest.
class _Class(object):
    def get_name(s): return "BlendSpace"
class BlendSpace(object):
    def get_class(s): return _Class()
unreal.ASSETS["/Game/Blend"] = BlendSpace()
try:
    inspect({"mesh": "/Game/Manny", "animation": "/Game/Blend"})
    check(False, "a Blend Space is refused")
except RuntimeError as error:
    check("BlendSpace" in str(error), "the refusal names the class: %s" % error)
try:
    inspect({"mesh": "/Game/Manny", "animation": "/Game/Swing", "times": [0.0] * 61})
    check(False, "more than 60 times are refused")
except RuntimeError as error:
    check("at most 60" in str(error), "the refusal names the cap: %s" % error)

# A partner standing where the right hand ends up is hit by it.
partner = inspect({"mesh": "/Game/Manny", "animation": "/Game/Swing", "samples": 5,
                   "partner": {"mesh": "/Game/Manny", "location": [45, 0, 0], "yaw": 0}})
check(any(p["kind"] == "partner_clipping" for p in partner["problems"]), "hand inside the partner is reported")

# An off-hand grip names a socket (a palm socket on hand_l): the hand it sits on, and its arm,
# are exempt from the item's clearance like the holding hand's chain.
real_socket = unreal.SkeletalMesh.find_socket
unreal.SkeletalMesh.find_socket = lambda s, n: unreal._Sock("hand_l", (0, 0, 0)) if n == "palm_l" else real_socket(s, n)
pole = dict(sword, location=[130, 0, 0], grips=[{"socket": "Grip", "bone": "palm_l", "tolerance": 20}])
held = inspect({"mesh": "/Game/Manny", "attachments": [pole]})
check(not any(p["kind"] in ("clipping", "grip") for p in held["problems"]), "the off-hand's grip exempts its arm: %s" % held["problems"])
loose = inspect({"mesh": "/Game/Manny", "attachments": [dict(pole, grips=[])]})
check(any(p["kind"] == "clipping" and ("lowerarm_l" in p["detail"] or "hand_l" in p["detail"]) for p in loose["problems"]), "without the grip it clips the left arm: %s" % loose["problems"])
unreal.SkeletalMesh.find_socket = real_socket

# A touch with the partner excuses clipping only inside its window and only against the bone it
# names. The partner faces the character 55 cm ahead; at t=0.5 the right hand, swung forward,
# is 5 cm from the partner's upperarm_l joint.
facing = {"mesh": "/Game/Manny", "location": [0, 55, 0], "yaw": 180}
def touching(window, bone="upperarm_l", distance=10):
    return inspect({"mesh": "/Game/Manny", "animation": "/Game/Swing", "times": [0, 0.5, 1], "partner": facing,
                    "contacts": [{"a": "hand_r", "b": "partner:" + bone, "expect": "touch", "distance": distance, "window": window}]})
def clipped(result):
    return [(p["time"], p["detail"]) for p in result["problems"] if p["kind"] == "partner_clipping"]
inside = touching([0.4, 0.6])
check(inside["passed"], "a touch inside its window passes: %s" % inside["problems"])
outside = touching([0.9, 1.0])
check([t for t, _ in clipped(outside)] == [0.5], "clipping outside the window is reported: %s" % clipped(outside))
elsewhere = touching([0.4, 0.6], bone="head", distance=100)
check([t for t, _ in clipped(elsewhere)] == [0.5] and "upperarm_l" in clipped(elsewhere)[0][1],
      "a touch on the head does not excuse the arm: %s" % clipped(elsewhere))

# Ground: each foot is measured by its lowest point (here a ball joint 5 cm under the ankle),
# against the lowest rest point of the feet - not the ankle against the floor.
unreal.BONES.extend([("ball_r", "foot_r", (0, 10, -5)), ("ball_l", "foot_l", (0, 10, -5))])
real_pose = unreal.AnimationLibrary.get_bone_pose_for_time
def sinking(anim, bone, t, rm):
    if bone == "pelvis":
        return unreal.Transform((0, 0, 95 - 5 * t))
    return real_pose(anim, bone, t, rm)
unreal.AnimationLibrary.get_bone_pose_for_time = staticmethod(sinking)
sunk = inspect({"mesh": "/Game/Manny", "animation": "/Game/Swing", "times": [0, 0.2, 1]})
unreal.AnimationLibrary.get_bone_pose_for_time = staticmethod(real_pose)
del unreal.BONES[-2:]
grounds = [p["time"] for p in sunk["problems"] if p["kind"] == "ground"]
check(grounds == [1.0], "a foot 5 cm into the floor is reported, 1 cm is not: %s" % sunk["problems"])
check(sunk["samples"][0]["feet_height"] == {"foot_l": 0.0, "foot_r": 0.0}, "planted feet read 0: %s" % sunk["samples"][0])

# The left/right naming is Blender's too: a rig named hand-L/hand-R gets its pairs and facing.
dashed = [(n.replace("_l", "-L").replace("_r", "-R"), p and p.replace("_l", "-L").replace("_r", "-R"), loc) for n, p, loc in unreal.BONES]
real_bones = unreal.BONES[:]
unreal.BONES[:] = dashed
named = inspect({"mesh": "/Game/Manny"})
unreal.BONES[:] = real_bones
check(named["frame"]["left_right_pairs_found"] == 6 and named["frame"]["right_axis_in_mesh_space"] == [-1.0, 0.0, 0.0], "-L/-R pairs: %s" % named["frame"])

# 3ds Max Biped names carry the side as a word: "Bip01 L Hand" / "Bip01 R Hand".
biped_name = lambda n: n and ("Bip01 %s %s" % (n[-1].upper(), n[:-2]) if n.endswith(("_l", "_r")) else n)
unreal.BONES[:] = [(biped_name(n), biped_name(p), loc) for n, p, loc in real_bones]
biped = inspect({"mesh": "/Game/Manny"})
unreal.BONES[:] = real_bones
check(biped["frame"]["left_right_pairs_found"] == 6 and biped["frame"]["right_axis_in_mesh_space"] == [-1.0, 0.0, 0.0], "Biped L/R pairs: %s" % biped["frame"])

# Without any left/right names the mesh is assumed to face +Y, as a Blender export arrives, and
# the report says the axes are assumed.
unsided = [(n.replace("_l", "_a").replace("_r", "_b"), p and p.replace("_l", "_a").replace("_r", "_b"), loc) for n, p, loc in real_bones]
unreal.BONES[:] = unsided
guessed = inspect({"mesh": "/Game/Manny"})
unreal.BONES[:] = real_bones
check(guessed["frame"]["left_right_pairs_found"] == 0 and guessed["frame"]["forward_axis_in_mesh_space"] == [0.0, 1.0, 0.0]
      and "assumed" in guessed["frame"]["note"], "no pairs: +Y forward, flagged: %s" % guessed["frame"])

# A socket from a Blender empty: its 100x scale is divided back once, and only once.
helpers = {"ARGS_JSON": "{}"}
exec(compile(open(os.path.join(SRC, "common.py")).read(), "common.py", "exec"), helpers)


class Socket(object):
    def __init__(s, scale):
        s.p = {"relative_scale": unreal.Vector(*scale)}
    def get_editor_property(s, k): return s.p[k]
    def set_editor_property(s, k, v): s.p[k] = v


grip = Socket((100, 100, 100))
change = helpers["undo_blender_socket_scale"](grip)
sc = grip.p["relative_scale"]
check(change and (sc.x, sc.y, sc.z) == (1, 1, 1), "scale back to 1: %s" % change)
check(helpers["undo_blender_socket_scale"](grip) is None, "a second pass changes nothing")
check(helpers["undo_blender_socket_scale"](Socket((2, 2, 2))) is None, "a socket scaled 2x in Blender is left alone")
print("ok")
