# blender_rig_check: the rig and skin problems that turn into wrong-handed, mis-scaled or
# broken characters after export. The mesh checks are mesh_check's (common.mesh_issues); the
# skin checks on top read the weights on the original mesh.
problems, warnings = [], []


def problem(where, text, fix):
    problems.append({"where": where, "problem": text, "fix": fix})


def warn(where, text, fix=""):
    warnings.append({"where": where, "problem": text, "fix": fix})


arm = obj(ARGS["armature"], "ARMATURE") if ARGS.get("armature") else first("ARMATURE")
if arm is None:
    raise RuntimeError("no armature in the file; pass armature, or check the object names with blender_info")
bones = arm.data.bones
names = [b.name for b in bones]

if any(abs(s - 1.0) > 1e-4 for s in arm.scale):
    problem(arm.name, "armature object scale is %s, not applied" % rnd(arm.scale, 3), "Object > Apply > Scale (with the meshes) before rigging further or exporting")
if unapplied_rotation(arm):
    problem(arm.name, "armature object rotation is not applied (%s deg)" % unapplied_rotation(arm), "Object > Apply > Rotation")

roots = [b.name for b in bones if b.parent is None]
if len(roots) > 1:
    problem(arm.name, "%d root bones (%s); Unreal needs one" % (len(roots), ", ".join(roots[:6])), "parent every root under a single root bone at the origin")
deform = [b for b in bones if b.use_deform]
if not deform:
    problem(arm.name, "no deform bones", "enable Deform on the bones that should move the mesh")
ends = [b.name for b in bones if not b.children and (b.name.lower().endswith("_end") or b.name.endswith("_End"))]
if ends:
    warn(arm.name, "%d leaf '_end' bones, usually left over from an FBX import with leaf bones" % len(ends), "delete them if nothing uses them; export with add_leaf_bones off")
spaced = [n for n in names if " " in n]
if spaced:
    warn(arm.name, "bone names with spaces: %s" % ", ".join(spaced[:6]), "use underscores; spaces become underscores or break lookups in engine code")

# Symmetry, in armature space: a left bone mirrors its right twin across X.
pairs = left_right_pairs(names)
asym = []
for l, r in pairs:
    hl, hr = bones[l].head_local, bones[r].head_local
    off = Vector((hl.x + hr.x, hl.y - hr.y, hl.z - hr.z)).length * TO_CM
    if off > 1.0:
        asym.append("%s/%s off by %.1f cm" % (l, r, off))
if asym:
    warn(arm.name, "%d bone pairs are not mirror images: %s" % (len(asym), "; ".join(asym[:6])), "Armature > Symmetrize from the correct side, unless the asymmetry is intended")
# Either side: a right-side bone without its left twin is as wrong as the reverse.
side_named = [n for n in names if other_side(n) and other_side(n)[1] not in names]
if side_named:
    warn(arm.name, "side-named bones without a twin: %s" % ", ".join(side_named[:8]), "check the naming (.L/.R) so mirroring and retargeting pair them")
frame = body_frame(arm)
facing = "-Y (Blender front)" if frame["forward"].y < -0.7 else ("+Y" if frame["forward"].y > 0.7 else ("%s" % rnd(frame["forward"], 2)))
if pairs and frame["forward"].y > -0.7:
    warn(arm.name, "the character faces %s, not -Y (Blender's front view)" % facing, "rotate the rig and meshes to face -Y and apply rotation, so exports land facing the way Unreal expects")

deform_names = set(b.name for b in deform)
skinned = [o for o in skinned_meshes(arm) if not ARGS.get("meshes") or o.name in ARGS["meshes"]]
if not skinned:
    warn(arm.name, "no meshes are skinned to this armature", "add an Armature modifier pointing at it (Ctrl+P > With Automatic Weights)")

mesh_reports = []
lo, hi = None, None
for o in skinned:
    me = o.data
    r = {"mesh": o.name, "vertices": len(me.vertices)}
    if any(abs(s - 1.0) > 1e-4 for s in o.scale) or unapplied_rotation(o):
        problem(o.name, "mesh transform not applied", "Object > Apply > All Transforms before skinning")
    groups = dict((g.index, g.name) for g in o.vertex_groups)
    orphan_groups = [n for n in groups.values() if n not in names]
    if orphan_groups:
        warn(o.name, "%d vertex groups match no bone: %s" % (len(orphan_groups), ", ".join(orphan_groups[:6])), "delete them or rename to the bone they should follow")
    unweighted, over4, over8 = 0, 0, 0
    non_deform_weight = {}
    for v in me.vertices:
        n = 0
        for g in v.groups:
            name = groups.get(g.group)
            if g.weight <= 0.001 or name is None:
                continue
            if name in deform_names:
                n += 1
            elif name in names:
                non_deform_weight[name] = non_deform_weight.get(name, 0) + 1
        if n == 0:
            unweighted += 1
        elif n > 8:
            over8 += 1
        elif n > 4:
            over4 += 1
    r.update({"unweighted_vertices": unweighted, "vertices_over_4_influences": over4, "vertices_over_8_influences": over8})
    if unweighted:
        problem(o.name, "%d vertices have no deform-bone weight; they will stay behind when the character moves" % unweighted, "select them (Select > Select All by Trait > Ungrouped Verts) and weight them")
    if over8:
        warn(o.name, "%d vertices use more than 8 bones" % over8, "Weights > Limit Total (8, or 4 for mobile) then Normalize All")
    for name, count in sorted(non_deform_weight.items())[:6]:
        problem(o.name, "%d vertices are weighted to non-deform bone %s, which is not exported" % (count, name), "move the weights to a deform bone, or enable Deform on %s" % name)
    counts, issues = mesh_issues(o)
    r.update(counts)
    for level, text, fix in issues:
        (problem if level == "problem" else warn)(o.name, text, fix)
    mesh_reports.append(r)
    with rest_pose([arm]):
        a, b = world_bbox([o])
    lo = a if lo is None else Vector(map(min, lo, a))
    hi = b if hi is None else Vector(map(max, hi, b))

height = round((hi.z - lo.z) * TO_CM, 1) if lo is not None else None
if height is not None and (height > 500 or height < 10):
    warn(arm.name, "the skinned meshes are %.1f cm tall" % height, "check the scene unit scale and object scale; a human should be about 150-200 cm")

emit({"armature": arm.name, "bones": len(bones), "deform_bones": len(deform), "roots": roots,
      "left_right_pairs": len(pairs), "faces": facing, "height_cm": height, "meshes": mesh_reports,
      "problems": problems, "warnings": warnings, "passed": not problems})
