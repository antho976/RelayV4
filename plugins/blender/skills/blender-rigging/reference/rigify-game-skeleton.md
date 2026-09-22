# Rigify to a clean game skeleton (verified in Blender 4.0 background mode)

Read this when a character is rigged, or should be rigged, with Rigify and has to ship to
Unreal. The generated rig has ~700 bones and several top-level bones, and its `DEF-` bones are
parented to `ORG-`/`MCH-` bones rather than to each other. A deform-only FBX export keeps every
non-deform ancestor of a deform bone, so exporting `rig` directly gives a messy hierarchy with
extra roots. The procedure below builds a separate armature from the `DEF-` bones with a proper
parent chain and a single `root`, drives it from the Rigify rig, and that armature is what you
bake actions onto and export.

Every `blender_python` call that touches Rigify needs `addons: ["rigify"]` (Blender runs with
factory settings). Third-party add-ons automate the same idea; they are not bundled, so do not
assume they are installed.

## 1. Generate

The metarig must be the real active, selected object. Do not wrap `rigify_generate` in
`temp_override`: Rigify changes the active object itself and fails with an AssertionError when
it is pinned. Fit the metarig's bones to the mesh before this (edit bones by code, check with
`blender_render`).

```python
if "metarig" not in bpy.data.objects:
    bpy.ops.object.armature_human_metarig_add()
meta = bpy.data.objects["metarig"]
for pb in meta.pose.bones:                          # no B-Bones: Unreal cannot reproduce them
    if pb.rigify_type and hasattr(pb.rigify_parameters, "bbones"):
        pb.rigify_parameters.bbones = 1
for o in bpy.context.view_layer.objects:
    o.select_set(False)
bpy.context.view_layer.objects.active = meta
meta.select_set(True)
print(bpy.ops.pose.rigify_generate())
rig = meta.data.rigify_target_rig
print(rig.name, len(rig.data.bones), "deform", sum(b.use_deform for b in rig.data.bones))
```

Some face `DEF-` bones keep B-Bone segments after this; they are skipped below. Skin the mesh to
`rig` with automatic weights (rigging-recipes.md recipe 5): only `DEF-` bones get weights.

## 2. Build the export skeleton

Verified on the default human metarig: 74 bones (face skipped), one root, rest matrices equal to
the `DEF-` bones within 1e-5, the skinned mesh moved over with its groups renamed.

```python
rig = bpy.data.objects["rig"]
FACE = ("DEF-brow", "DEF-lid", "DEF-ear", "DEF-cheek", "DEF-chin", "DEF-jaw", "DEF-lip",
        "DEF-nose", "DEF-tongue", "DEF-forehead", "DEF-temple", "DEF-teeth", "DEF-eye")
keep = {b.name for b in rig.data.bones if b.use_deform and not b.name.startswith(FACE)}

def deform_parent(b):                 # nearest DEF ancestor, mapping ORG-x / MCH-x to DEF-x
    a = b.parent
    while a:
        if a.name in keep and a.name != b.name:
            return a.name
        base = a.name[4:] if a.name[:4] in ("ORG-", "MCH-") else a.name
        if "DEF-" + base in keep and "DEF-" + base != b.name:
            return "DEF-" + base
        a = a.parent
    return None

parents = {n: deform_parent(rig.data.bones[n]) for n in keep}
bpy.context.view_layer.objects.active = rig          # exact head/tail/roll from edit bones
bpy.ops.object.mode_set(mode="EDIT")
rest = {n: (rig.data.edit_bones[n].head.copy(), rig.data.edit_bones[n].tail.copy(),
            rig.data.edit_bones[n].roll) for n in keep}
bpy.ops.object.mode_set(mode="OBJECT")

exp = bpy.data.objects.new("SK_Hero", bpy.data.armatures.new("SK_Hero"))
bpy.context.scene.collection.objects.link(exp)
exp.matrix_world = rig.matrix_world.copy()
bpy.context.view_layer.objects.active = exp
bpy.ops.object.mode_set(mode="EDIT")
eb = exp.data.edit_bones
root = eb.new("root"); root.head, root.tail, root.use_deform = (0, 0, 0), (0, 0.2, 0), False
for n, (h, t, r) in rest.items():
    e = eb.new(n[4:]); e.head, e.tail, e.roll, e.use_deform = h, t, r, True
for n, p in parents.items():
    eb[n[4:]].parent = eb[p[4:]] if p else root
bpy.ops.object.mode_set(mode="OBJECT")
for n in keep:                                        # follow the Rigify rig
    c = exp.pose.bones[n[4:]].constraints.new("COPY_TRANSFORMS")
    c.target, c.subtarget = rig, n
for o in bpy.data.objects:                            # move the skin over, renaming groups
    for m in o.modifiers if o.type == "MESH" else []:
        if m.type == "ARMATURE" and m.object == rig:
            m.object = exp
            for g in o.vertex_groups:
                if g.name.startswith("DEF-"):
                    g.name = g.name[4:]
            o.parent = exp
print(len(exp.data.bones), "bones; roots", [b.name for b in exp.data.bones if not b.parent])
```

Notes:

- Copy rest data from **edit bones** (head, tail, roll). `EditBone.align_roll(bone.z_axis)` gave
  180-degree flips in testing, and `Bone.z_axis` is parent-relative anyway.
- Skipping face bones leaves face vertex groups with no bone: drop them
  (`drop_non_deform_groups` in [rigging-recipes.md](rigging-recipes.md) recipe 7), or set
  `FACE = ()` to keep the face.
- Names lose the `DEF-` prefix (`upper_arm.L`, `upper_arm.L.001`, ...). Rename further only
  before any action is baked, and rename vertex groups with the bones.

## 3. Animate on `rig`, bake onto the export skeleton

Animate with Rigify's controls on `rig`. For each action, assign it to `rig`, then bake the
export skeleton with visual keying. The Copy Transforms constraints stay (`clear_constraints=False`)
so the next action bakes the same way:

```python
rig, exp = bpy.data.objects["rig"], bpy.data.objects["SK_Hero"]
src = bpy.data.actions["A_Hero_Wave_ctrl"]          # the action authored on the Rigify rig
rig.animation_data.action = src
f0, f1 = map(int, src.frame_range)
for o in bpy.context.view_layer.objects:
    o.select_set(o == exp)
bpy.context.view_layer.objects.active = exp
exp.animation_data_create()
exp.animation_data.action = None
bpy.ops.object.mode_set(mode="POSE")
bpy.ops.pose.select_all(action="SELECT")
bpy.ops.nla.bake(frame_start=f0, frame_end=f1, only_selected=True, visual_keying=True,
                 clear_constraints=False, use_current_action=False, bake_types={"POSE"})
bpy.ops.object.mode_set(mode="OBJECT")
baked = exp.animation_data.action                  # created as "Action": name it
baked.name, baked.use_fake_user = "A_Hero_Wave", True
print(baked.name, tuple(baked.frame_range), len(baked.fcurves))
```

Name the control-rig actions differently from the baked ones (here `_ctrl`) so nobody exports
the wrong one. `all_actions` only exports actions whose F-curve paths all resolve on the exported
armature, so `_ctrl` actions are skipped by it - but a baked action of another character with the
same bone names is not. Export `SK_Hero`, never `rig`:
`blender_export {objects: ["SK_Hero"], kind: "skeletal"}`, or `kind: "animation"` with `action`.

**Mute the constraints before exporting.** The FBX exporter samples the evaluated pose, so with
the Copy Transforms constraints live every exported take would show whatever action `rig` has
assigned, not the baked one. Mute them (and save with `save_as`), export, and unmute to bake more:

```python
exp = bpy.data.objects["SK_Hero"]
for pb in exp.pose.bones:
    for c in pb.constraints:
        if c.type == "COPY_TRANSFORMS":
            c.mute = True        # False to follow the Rigify rig again
```

## 4. Check

- `blender_rig_check {armature: "SK_Hero"}`: one root, no vertex groups without bones.
- `blender_anim_inspect {armature: "SK_Hero", action: "A_Hero_Wave", track: ["hand.L", "hand.R"]}`
  and the same call on `rig` (with `action: "A_Hero_Wave_ctrl"`, `track: ["DEF-hand.L", "DEF-hand.R"]`):
  the positions must match at every sample.
- `blender_render {objects: [<mesh>], action: "A_Hero_Wave", frames: [...]}` at the key frames.
- `blender_to_unreal kind: skeletal` once: height, root scale 1, hands on the right sides.
