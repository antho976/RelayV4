# Rigging recipes (verified in Blender 4.0 background mode)

Each block runs as `blender_python` `code`. The tool's scope has `bpy`, `Vector`, `Matrix`,
`math`, `ARGS`; import anything else (`Euler`, `Quaternion`, `bmesh`) yourself. Names below
(`Hero`, `HeroBody`, `hand.R`) are examples - read the real ones with `blender_info` first. Use
`save_as` for results unless you mean to overwrite the source.

## 1. Inventory the armature

```python
arm = bpy.data.objects["Hero"]
b = arm.data.bones
print("roots:", [x.name for x in b if x.parent is None])
print("deform:", sum(x.use_deform for x in b), "of", len(b))
print("object transform identity:", arm.matrix_world == Matrix.Identity(4))
for x in b:
    z = x.matrix_local.col[2].to_3d()          # armature-space Z axis (roll); x.z_axis is parent-relative
    print(f"{x.name:24s} deform={x.use_deform!s:5s} parent={(x.parent.name if x.parent else '-'):16s}"
          f" len={x.length*100:6.1f}cm z={tuple(round(c, 2) for c in z)}")
```

## 2. Apply scale and rotation to the armature and its meshes together

Applying on the armature alone leaves children and actions inconsistent. Location keys in
existing actions are **not** rescaled - fix scale before animating.

```python
arm = bpy.data.objects["Hero"]
objs = [arm] + [c for c in arm.children_recursive if c.type == "MESH"]
with bpy.context.temp_override(active_object=arm, object=arm,
                               selected_objects=objs, selected_editable_objects=objs):
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
print([(o.name, tuple(round(s, 3) for s in o.scale)) for o in objs])
```

## 3. Add bones, a twist bone, then symmetrize

```python
arm = bpy.data.objects["Hero"]
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="EDIT")
eb = arm.data.edit_bones
ua = eb["upper_arm.L"]
t = eb.new("upper_arm_twist.L")
t.head, t.tail, t.roll = ua.head.lerp(ua.tail, 0.5), ua.tail.copy(), ua.roll
t.parent, t.use_deform = ua, True
# Mirror every .L bone to .R (replaces existing .R twins); only selected bones are mirrored.
for b in eb:
    b.select = b.select_head = b.select_tail = b.name.endswith(".L")
bpy.ops.armature.symmetrize(direction="NEGATIVE_X")
# Recalculate roll on the legs: local Z toward -Y (forward)
for b in eb:
    b.select = b.name.startswith(("thigh", "shin"))
bpy.ops.armature.calculate_roll(type="GLOBAL_NEG_Y")
bpy.ops.object.mode_set(mode="OBJECT")
print(sorted(b.name for b in arm.data.bones))
```

Roll types: `bpy.ops.armature.calculate_roll.get_rna_type().properties["type"].enum_items.keys()`.

## 4. Mirror report: positions and rolls of .L/.R pairs

`blender_rig_check` compares head positions only. This also checks tails and roll.

```python
def mirror_report(arm, tol_cm=0.5, tol_dot=0.99):
    bones = arm.data.bones
    mx = lambda v: Vector((-v.x, v.y, v.z))
    for b in bones:
        twin_name = bpy.utils.flip_name(b.name)
        if twin_name == b.name or not b.name.endswith((".L", "_L", "_l")):
            continue
        r = bones.get(twin_name)
        if r is None:
            print("NO TWIN", b.name); continue
        he = (mx(b.head_local) - r.head_local).length * 100
        te = (mx(b.tail_local) - r.tail_local).length * 100
        zd = mx(b.matrix_local.col[2].to_3d()).dot(r.matrix_local.col[2].to_3d())  # +1 = roll mirrored
        bad = he > tol_cm or te > tol_cm or zd < tol_dot
        print(f"{b.name:24s} head {he:5.2f}cm tail {te:5.2f}cm roll-dot {zd:+.3f}{'  <-- CHECK' if bad else ''}")

mirror_report(bpy.data.objects["Hero"])
```

A roll-dot near -1 means the twin's roll is flipped 180 degrees: symmetrize again.

## 5. Automatic weights (bone heat)

```python
arm, body = bpy.data.objects["Hero"], bpy.data.objects["HeroBody"]
body.vertex_groups.clear()               # start clean: old groups, armature modifiers, parent
for m in [m for m in body.modifiers if m.type == "ARMATURE"]:
    body.modifiers.remove(m)
body.parent = None
sel = [body, arm]
with bpy.context.temp_override(active_object=arm, object=arm,
                               selected_objects=sel, selected_editable_objects=sel):
    print(bpy.ops.object.parent_set(type="ARMATURE_AUTO", keep_transform=True))
unweighted = [v.index for v in body.data.vertices if not any(g.weight > 0 for g in v.groups)]
print("groups", len(body.vertex_groups), "unweighted verts", len(unweighted),
      "max influences", max(len(v.groups) for v in body.data.vertices))
```

`ARMATURE_NAME` instead makes empty groups per bone. Non-zero `unweighted` after heat usually
means a shell heat could not solve (SKILL.md section 5).

## 6. Weight cleanup with operators (mirror -> clean -> limit -> normalize)

```python
body = bpy.data.objects["HeroBody"]
# Clean and Limit Total act on the objects SELECTED IN THE VIEW LAYER and ignore the override's
# selected_objects (verified): select the mesh for real, and only the mesh.
for o in bpy.context.view_layer.objects:
    o.select_set(o == body)
bpy.context.view_layer.objects.active = body
with bpy.context.temp_override(active_object=body, object=body,
                               selected_objects=[body], selected_editable_objects=[body]):
    # mirror all groups across X, renaming .L<->.R (only for a symmetric mesh)
    bpy.ops.object.vertex_group_mirror(mirror_weights=True, flip_group_names=True,
                                       all_groups=True, use_topology=False)
    bpy.ops.object.vertex_group_clean(group_select_mode="ALL", limit=0.01, keep_single=True)
    bpy.ops.object.vertex_group_limit_total(group_select_mode="ALL", limit=4)
    bpy.ops.object.vertex_group_normalize_all(group_select_mode="ALL", lock_active=False)
s = [sum(g.weight for g in v.groups) for v in body.data.vertices]
print("max influences", max(len(v.groups) for v in body.data.vertices), "sum range", min(s), max(s))
```

`use_topology=False` mirrors by position: the mesh must be symmetric about X=0.

## 7. Weight fixes with the data API

```python
body = bpy.data.objects["HeroBody"]
arm = body.parent if body.parent and body.parent.type == "ARMATURE" else bpy.data.objects["Hero"]

def strip_side(obj, group, keep_positive_x, margin=0.01):
    """Remove a left-side group's weights from right-side verts (or the reverse)."""
    vg = obj.vertex_groups.get(group)
    if vg is None:
        return 0
    bad = [v.index for v in obj.data.vertices
           if (v.co.x < -margin if keep_positive_x else v.co.x > margin)
           and any(g.group == vg.index for g in v.groups)]
    vg.remove(bad)
    return len(bad)

def drop_non_deform_groups(obj, arm):
    deform = {b.name for b in arm.data.bones if b.use_deform}
    gone = [g.name for g in obj.vertex_groups if g.name not in deform]
    for n in gone:
        obj.vertex_groups.remove(obj.vertex_groups[n])
    return gone

def clean_limit_normalize(obj, max_influences=4, min_weight=0.01):
    for v in obj.data.vertices:
        ws = sorted(((g.group, g.weight) for g in v.groups), key=lambda t: -t[1])
        keep = [(i, w) for i, w in ws[:max_influences] if w >= min_weight] or ws[:1]
        total = sum(w for _, w in keep) or 1.0
        keep = {i: w / total for i, w in keep}
        for gi in [g.group for g in v.groups]:        # copy first: removing edits v.groups
            vg = obj.vertex_groups[gi]
            if gi in keep:
                vg.add([v.index], keep[gi], "REPLACE")
            else:
                vg.remove([v.index])

for g in ("thigh", "shin", "foot"):
    print(g, strip_side(body, g + ".L", True), strip_side(body, g + ".R", False))
print("dropped", drop_non_deform_groups(body, arm))
clean_limit_normalize(body, 4, 0.01)
print("unweighted", sum(1 for v in body.data.vertices if not v.groups))
```

Stripping can leave a vertex with no weights; the last line catches it - re-weight those
(assign to the nearest correct bone) before export.

## 8. Held props: bone parent, and Child Of pick-up/drop

Bone parent (always held). Children of a bone are placed relative to the bone's tail; set
`matrix_world`, not `location`:

```python
arm, sword = bpy.data.objects["Hero"], bpy.data.objects["Sword"]
grip_world = arm.matrix_world @ arm.pose.bones["hand.R"].tail      # or a palm point you measured
sword.parent, sword.parent_type, sword.parent_bone = arm, "BONE", "hand.R"
sword.matrix_world = Matrix.Translation(grip_world) @ Matrix.Rotation(math.pi, 4, "Z")
```

Pick up at frame 10, drop at frame 30, with no jump at either switch:

```python
scene = bpy.context.scene
arm, cup = bpy.data.objects["Hero"], bpy.data.objects["Cup"]
PICK, DROP, HAND = 10, 30, "hand.R"
con = cup.constraints.new("CHILD_OF"); con.name = "Hold"
con.target, con.subtarget = arm, HAND
scene.frame_set(PICK)
con.inverse_matrix = (arm.matrix_world @ arm.pose.bones[HAND].matrix).inverted()   # "Set Inverse" at PICK
con.influence = 0.0; con.keyframe_insert("influence", frame=PICK - 1)
con.influence = 1.0; con.keyframe_insert("influence", frame=PICK)
scene.frame_set(DROP)
dropped = cup.matrix_world.copy()                    # visual transform while held
cup.keyframe_insert("location", frame=DROP - 1); cup.keyframe_insert("rotation_euler", frame=DROP - 1)
con.influence = 1.0; con.keyframe_insert("influence", frame=DROP)
con.influence = 0.0; con.keyframe_insert("influence", frame=DROP + 1)
cup.matrix_world = dropped
cup.keyframe_insert("location", frame=DROP + 1); cup.keyframe_insert("rotation_euler", frame=DROP + 1)
for k in (k for fc in cup.animation_data.action.fcurves for k in fc.keyframe_points):
    k.interpolation = "CONSTANT"                    # switches must step, not blend
for f in (PICK - 1, PICK, DROP, DROP + 1):
    scene.frame_set(f); print(f, [round(c, 3) for c in cup.matrix_world.translation])
```

The operator form also works:
`with bpy.context.temp_override(object=cup, active_object=cup): bpy.ops.constraint.childof_set_inverse(constraint="Hold", owner="OBJECT")`.
The cup's keys go in the cup's own action; the hand must actually reach the cup at `PICK` -
measure it with `blender_anim_inspect` `contacts`.

## 9. Rigify

Generating a Rigify rig, building a clean export skeleton from its `DEF-` bones and baking
actions onto it: [rigify-game-skeleton.md](rigify-game-skeleton.md).

## 10. Twist bones driven in Blender

```python
arm = bpy.data.objects["Hero"]
for side in ("L", "R"):
    c = arm.pose.bones[f"forearm_twist.{side}"].constraints.new("COPY_ROTATION")
    c.target, c.subtarget = arm, f"hand.{side}"
    c.use_x = c.use_z = False              # twist is rotation about the bone's own Y
    c.mix_mode = "REPLACE"
    c.target_space = c.owner_space = "LOCAL"   # constraint spaces: WORLD, POSE, LOCAL, ...
    c.influence = 0.5
```

The twist bone must exist (recipe 3), be deform, and be weighted to the middle band of the
forearm. Verified: a 90 degree hand twist gives the twist bone 45 degrees.

## 11. Corrective shape key driven by a bone

```python
body, arm = bpy.data.objects["HeroBody"], bpy.data.objects["Hero"]
if not body.data.shape_keys:
    body.shape_key_add(name="Basis", from_mix=False)
sk = body.data.shape_keys.key_blocks.get("knee_bend_L") or body.shape_key_add(name="knee_bend_L", from_mix=False)
# ... move sk.data[i].co for the knee verts (the fix shape at 90 degrees) ...
d = sk.driver_add("value").driver
d.type = "SCRIPTED"
v = d.variables.new(); v.name, v.type = "rot", "TRANSFORMS"
t = v.targets[0]
t.id, t.bone_target = arm, "shin.L"
t.transform_type, t.rotation_mode, t.transform_space = "ROT_X", "SWING_TWIST_Y", "LOCAL_SPACE"
d.expression = "max(0.0, min(1.0, rot / 1.5708))"   # 0 at rest, 1 at 90 degrees
```

Verified: 45 degrees of knee bend gives 0.5. Note the driver target space enum is
`LOCAL_SPACE`, unlike constraint spaces (`LOCAL`).
