# Runs anim_inspect.py against the fake `unreal` module and checks what it concludes.
# Usage: python3 run_inspect.py   (exit status 0 when every check holds)
import io, json, os, sys, contextlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
SRC = os.path.dirname(HERE)


def inspect(args):
    code = "ARGS_JSON = %s\n%s\n%s" % (json.dumps(json.dumps(args)),
                                       open(os.path.join(SRC, "common.py")).read(),
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
check(right["attachments"]["sword"]["end_b_at_start"][1] > right["attachments"]["sword"]["fwd_right_up"][1] - 1, "item extends outward")

# Moving the grip away from the hand fails the grip check.
off = inspect({"mesh": "/Game/Manny", "attachments": [dict(sword, location=[0, 0, 12])]})
check(any(p["kind"] == "grip" for p in off["problems"]), "a grip 12 cm off the hand is reported")

# Contacts across an animation: the right hand swings across; hands must stay apart.
swing = inspect({"mesh": "/Game/Manny", "animation": "/Game/Swing", "samples": 5,
                 "contacts": [{"a": "hand_r", "b": "hand_l", "expect": "apart", "distance": 10}],
                 "track": ["hand_r"]})
sides = [r["points"]["hand_r"]["side"] for r in swing["samples"]]
check(sides[0] == "right" and sides[-1] == "left", "the swing crosses the body: %s" % sides)

# A partner standing where the right hand ends up is hit by it.
partner = inspect({"mesh": "/Game/Manny", "animation": "/Game/Swing", "samples": 5,
                   "partner": {"mesh": "/Game/Manny", "location": [45, 0, 0], "yaw": 0}})
check(any(p["kind"] == "partner_clipping" for p in partner["problems"]), "hand inside the partner is reported")
# A socket from a Blender empty: its 100x scale is divided back once, and only once.
import unreal
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
