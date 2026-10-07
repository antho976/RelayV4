# anim_rules: the checks behind blender_anim_inspect and ue_anim_inspect, written once (D161: a
# problem caught before export is measured the same way after import). Each engine's script
# poses its rig at a sample and hands over points through a `rig` adapter; everything here is
# plain Python on (x, y, z) tuples in centimetres, Z up, so a rule fixed here is fixed in both.
# Bundled after the engine's common.py (with rig_frame.py: names, body filters, to_body) and
# before its anim_inspect.py.
#
# The adapter:
#   rig.pose(key)           pose the rig (and partner) at a sample: a Blender frame or an Unreal time
#   rig.point(ref)          any reference the engine understands, at the current pose
#   rig.owner(ref)          the own bone a reference sits on (`bone:tail` -> bone, socket -> its bone), or None
#   rig.parent(bone)        its parent bone, or None
#   rig.segments()          the own body as [(id, bones, a, b)]; an item's holder and grips exempt by `bones`
#   rig.partner_segments()  the partner's body the same way, [] without a partner
#   rig.partner_bones(ref)  for `partner:...`, the partner segment bones that point touches
#   rig.probes              own bones whose clearance to the partner is measured
#   rig.items               [{name, holder, grips: [{label, point, bone, tolerance?}]}]
#   rig.feet, rig.rest_feet {foot: [refs of the foot and the bones below it]}, {foot: [their rest points]}
#   rig.frame               {center, forward, right, up}: the character's frame, same space and units

ITEM_ENDS = ("end_a", "end_b", "center")


def _sub(a, b): return (a[0] - b[0], a[1] - b[1], a[2] - b[2])
def _dot(a, b): return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
def _len(a): return _dot(a, a) ** 0.5


def _seg(p, a, b):
    ab = _sub(b, a)
    denom = _dot(ab, ab)
    t = 0.0 if denom < 1e-9 else max(0.0, min(1.0, _dot(_sub(p, a), ab) / denom))
    return _len(_sub(p, (a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t)))


def _nearest(p, segments, skip=()):
    best = (1e9, None)
    for sid, bones, a, b in segments:
        if skip and not skip.isdisjoint(bones):
            continue
        d = _seg(p, a, b)
        if d < best[0]:
            best = (d, sid)
    return best


def in_window(contact, key):
    w = contact.get("window")
    return not w or w[0] <= key <= w[1]


def chain(rig, bone, depth=3):
    """A bone and its parents: what an item hanging from or gripped by it may touch."""
    out = set()
    while bone is not None and len(out) < depth:
        out.add(bone)
        bone = rig.parent(bone)
    return out


def _thing(rig, ref):
    """What a contact end or a partner probe is, to match one with the other: `item:<name>` for
    any point of an item, otherwise the own bone it sits on (so `hand.R:tail` is `hand.R`)."""
    if ref.startswith("item:"):
        return "item:" + ref.split(":")[1]
    if ref.startswith("partner:") or ref.startswith("obj:"):
        return None
    return rig.owner(ref)


def inspect_animation(rig, keys, args, key="frame", row_key=None):
    """The checks at every sample key. Returns problems, samples and closest_approach; `key` names
    the sample in problems and closest approaches, `row_key` (default `key`) in samples."""
    row_key = row_key or key
    radius = float(args.get("body_radius", 8.0))
    touch = float(args.get("touch_distance", 5.0))
    contacts = args.get("contacts", [])
    problems, minimum, rows = [], {}, []

    def problem(k, kind, text):
        if len(problems) < 80:
            problems.append({key: k, "kind": kind, "detail": text})

    def keep_min(name, clearance, near, k):
        if name not in minimum or clearance < minimum[name][0]:
            minimum[name] = (clearance, near, k)

    # An item may touch the chain it hangs from and the chains of the hands that grip it.
    allowed = {}
    for it in rig.items:
        allowed[it["name"]] = set(chain(rig, it["holder"]))
        for g in it["grips"]:
            allowed[it["name"]] |= chain(rig, rig.owner(g["bone"]))
    # An expected touch with the partner excuses clipping only inside its window, and only
    # against the partner bone it names: (own thing, partner bones, contact).
    excused = []
    for c in contacts:
        if c.get("expect", "touch") != "touch":
            continue
        for mine, theirs in ((c["a"], c["b"]), (c["b"], c["a"])):
            if theirs.startswith("partner:") and not mine.startswith("partner:"):
                thing, bones = _thing(rig, mine), rig.partner_bones(theirs)
                if thing and bones:
                    excused.append((thing, set(bones), c))
    # The floor is the lowest point of the feet at rest; each foot is measured by its lowest point.
    ground = min(p[2] for pts in rig.rest_feet.values() for p in pts) if rig.feet else 0.0

    for k in keys:
        rig.pose(k)
        row = {row_key: k, "points": {}, "checks": []}
        for ref in args.get("track") or []:
            p = rig.point(ref)
            row["points"][ref] = {"fwd_right_up": rnd(to_body(rig.frame, p)), "side": side(rig.frame, p)}

        body = rig.segments()
        for it in rig.items:
            name = it["name"]
            for end in ITEM_ENDS:
                d, near = _nearest(rig.point("item:%s:%s" % (name, end)), body, allowed[name])
                keep_min("item:%s:%s clearance to own body" % (name, end), round(d - radius, 1), near, k)
                if d < radius:
                    problem(k, "clipping", "%s of %r is %.1f cm inside the body near %s" % (end, name, radius - d, near))
            for g in it["grips"]:
                d = _len(_sub(rig.point(g["point"]), rig.point(g["bone"])))
                row["checks"].append({"grip": "%s -> %s" % (g["label"], g["bone"]), "distance": round(d, 1)})
                tol = float(g.get("tolerance", touch))
                if d > tol:
                    problem(k, "grip", "%s is %.1f cm from %s (tolerance %.1f)" % (g["bone"], d, g["label"], tol))

        for c in contacts:
            d = _len(_sub(rig.point(c["a"]), rig.point(c["b"])))
            expect = c.get("expect", "touch")
            limit = float(c.get("distance", touch if expect == "touch" else 10.0))
            row["checks"].append({"contact": "%s ~ %s" % (c["a"], c["b"]), "distance": round(d, 1), "expect": expect})
            if in_window(c, k) and expect == "touch" and d > limit:
                problem(k, "contact", "%s and %s should touch but are %.1f cm apart" % (c["a"], c["b"], d))
            if in_window(c, k) and expect == "apart" and d < limit:
                problem(k, "contact", "%s and %s should stay %.0f cm apart but are %.1f cm" % (c["a"], c["b"], limit, d))

        other = rig.partner_segments()
        if other:
            probes = [(n, n, rig.point(n)) for n in rig.probes]
            for it in rig.items:
                probes += [("item:%s:%s" % (it["name"], e), "item:" + it["name"], rig.point("item:%s:%s" % (it["name"], e))) for e in ITEM_ENDS]
            for name, thing, p in probes:
                d, near = _nearest(p, other)
                keep_min("%s clearance to partner" % name, round(d - radius, 1), near, k)
                skip = set()
                for t, bones, c in excused:
                    if t == thing and in_window(c, k):
                        skip |= bones
                if skip and d < radius:
                    d, near = _nearest(p, other, skip)
                if d < radius:
                    problem(k, "partner_clipping", "%s is %.1f cm inside the partner near %s" % (name, radius - d, near))

        if rig.feet:
            heights = dict((f, round(min(rig.point(r)[2] for r in refs) - ground, 1) + 0.0) for f, refs in rig.feet.items())  # no -0.0
            row["feet_height"] = heights
            low = min(sorted(heights), key=heights.get)
            if heights[low] < -2.0:
                problem(k, "ground", "%s is %.1f cm below the rest-pose ground" % (low, -heights[low]))
        rows.append(row)

    return {"problems": problems, "samples": rows,
            "closest_approach": dict((n, {"clearance_cm": v[0], "near": v[1], key: v[2]}) for n, v in sorted(minimum.items()))}
