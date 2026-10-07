# rig_frame: the character frame both engines measure in (D161), written once - left/right bone
# naming, which bones are body volume, and the frame itself from rest-pose joint positions. Plain
# Python on 3-sequences (tuples or mathutils Vectors), bundled after each engine's common.py in
# every Blender and Unreal script; the engine's body_frame only gathers the rest-pose points.

# ---- left/right from names: .L/.R, _l/_r, -L/-R suffixes, Left/Right as a word, 3ds Max Biped
# and CAT's " L "/" R " ("Bip01 L Hand"), l_/r_ prefixes
import re

PAIRS = [(".L", ".R"), (".l", ".r"), ("_L", "_R"), ("_l", "_r"), ("-L", "-R")]
PREFIXES = [("l_", "r_"), ("L_", "R_")]
# LeftHand, hand_left, Hand Left - but not cleft_chin or bright_eye.
SIDE_WORD = re.compile(r"Left|Right|(?<![A-Za-z])(?:left|right)| [LR] ")
SIDE_SWAP = {"Left": "Right", "Right": "Left", "left": "right", "right": "left", " L ": " R ", " R ": " L "}


def other_side(name):
    """("left" or "right", the other side's name) for a side-named bone, or None. Suffixes win
    over words, words over prefixes."""
    for left, right in PAIRS:
        if name.endswith(left):
            return "left", name[: -len(left)] + right
        if name.endswith(right):
            return "right", name[: -len(right)] + left
    m = SIDE_WORD.search(name)
    if m:
        word = m.group()
        return ("left" if word.strip() in ("Left", "left", "L") else "right"), name[: m.start()] + SIDE_SWAP[word] + name[m.end():]
    for left, right in PREFIXES:
        if name.startswith(left):
            return "left", right + name[len(left):]
        if name.startswith(right):
            return "right", left + name[len(right):]
    return None


def twin(name):
    """The right-side twin of a left-side bone name, or None."""
    s = other_side(name)
    return s[1] if s and s[0] == "left" else None


def left_right_pairs(names):
    names = set(names)
    return [(n, twin(n)) for n in sorted(names) if twin(n) in names and twin(n) != n]


# ---- which bones are body volume, by name

# Not body volume: fingers, twist and helper bones, props, face, control rigs.
NOT_BODY = ("finger", "thumb", "index", "middle", "ring", "pinky", "metacarpal", "twist", "weapon", "prop", "attach",
            "socket", "root", "camera", "correct", "eye", "jaw", "tongue", "pole", "ctrl", "mch", "org")
NOT_BODY_TOKENS = ("ik", "end", "tip")
PROBES = ("hand", "foot", "head", "forearm", "lowerarm", "shin", "calf")


def _tokens(name):
    out, word = [], ""
    for ch in name.lower():
        if ch.isalnum():
            word += ch
        elif word:
            out.append(word)
            word = ""
    return out + [word] if word else out


def body_bone(name):
    low = name.lower()
    return not any(k in low for k in NOT_BODY) and not any(t in NOT_BODY_TOKENS or t.startswith("ik") for t in _tokens(name))


def probe_bone(name):
    return body_bone(name) and any(k in name.lower() for k in PROBES)


def foot_bone(name):
    return body_bone(name) and "foot" in name.lower()


# ---- the frame

def _cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def _unit(a):
    n = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]) ** 0.5
    return (0.0, 0.0, 0.0) if n < 1e-9 else (a[0] / n, a[1] / n, a[2] / n)


def character_frame(names, parent, joints, ends=None, default_right=(0.0, 1.0, 0.0), right_handed=False, scale=1.0):
    """Character axes from the rig at rest: `right` points from left-side joints to their twins,
    `up` is +Z, `forward` completes the frame (up x right in Blender's right-handed space, right x
    up in Unreal's left-handed one). The centre is the mean of the pairs' midpoints (the centroid
    of all points without pairs) at ground level, the lowest point of the feet (each foot bone and
    the bones below it), or of the whole rig when nothing is called a foot. `joints` maps each
    bone to its joint (a Blender head), `ends` optionally to its far end (a Blender tail); `scale`
    is centimetres per unit, applied by to_body."""
    ends = ends or {}
    pairs = left_right_pairs(names)
    lateral, center = (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)
    for l, r in pairs:
        a, b = joints[l], joints[r]
        lateral = tuple(lateral[i] + b[i] - a[i] for i in range(3))
        center = tuple(center[i] + (a[i] + b[i]) / 2.0 for i in range(3))
    up = (0.0, 0.0, 1.0)
    flat = (lateral[0], lateral[1], 0.0)
    right = _unit(flat) if (flat[0] ** 2 + flat[1] ** 2) ** 0.5 > 1e-6 else tuple(float(c) for c in default_right)
    forward = _unit(_cross(up, right) if right_handed else _cross(right, up))
    points = [tuple(joints[n]) for n in names] + [tuple(ends[n]) for n in names if n in ends]
    children = {}
    for n in names:
        children.setdefault(parent.get(n), []).append(n)
    feet, todo = [], [n for n in names if foot_bone(n)]
    while todo:
        n = todo.pop()
        feet.append(n)
        todo.extend(children.get(n, []))
    low = [joints[n][2] for n in feet] + [ends[n][2] for n in feet if n in ends]
    ground = min(low) if low else min(p[2] for p in points)
    if pairs:
        center = tuple(c / len(pairs) for c in center)
    else:
        center = tuple(sum(p[i] for p in points) / len(points) for i in range(3))
    return {"right": right, "forward": forward, "up": up, "center": (center[0], center[1], ground),
            "pairs": pairs, "found_pairs": len(pairs), "scale": scale}


def to_body(frame, p):
    """[forward, right, up] in cm from the character's centre at ground level."""
    d = [p[i] - frame["center"][i] for i in range(3)]
    k = frame.get("scale", 1.0)
    return tuple(k * sum(d[i] * frame[axis][i] for i in range(3)) for axis in ("forward", "right", "up"))


def side(frame, p, tolerance=3.0):
    r = to_body(frame, p)[1]
    return "right" if r > tolerance else ("left" if r < -tolerance else "center")
