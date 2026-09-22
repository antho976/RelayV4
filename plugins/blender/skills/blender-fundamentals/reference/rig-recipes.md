# Rig recipes (verified in background mode)

Armature, keys, skinning and rig repair snippets, run in this order with
`blender -b --factory-startup` on Blender 4.0.2 (each block uses what the previous one made).
Paste into `blender_python` `code`; `bpy`, `Vector`, `Matrix`, `math` are in scope, `bmesh` is
not. For the general recipes (create, apply transforms, origin, modifiers, bmesh, UVs,
materials) see `bpy-recipes.md`. Rigging depth lives in `blender-rigging`; export rules for the
rig in `blender-to-unreal`.

## Armature: bones, bone collections, keys (no pose mode needed)

```python
data = bpy.data.armatures.new("SK_Hero")
arm = bpy.data.objects.new("Armature", data)
bpy.context.scene.collection.objects.link(arm)
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode='EDIT')                        # edit_bones exist only in edit mode
eb = data.edit_bones
root = eb.new("root"); root.head, root.tail = (0, 0, 0), (0, 0.25, 0); root.use_deform = False
pelvis = eb.new("pelvis"); pelvis.head, pelvis.tail = (0, 0, 0.95), (0, 0, 1.10); pelvis.parent = root
for side, x in (("L", 1), ("R", -1)):                       # facing -Y: the character's left is +X
    up = eb.new("upperarm.%s" % side); up.head, up.tail = (0.20 * x, 0, 1.40), (0.45 * x, 0, 1.40); up.parent = pelvis
    hand = eb.new("hand.%s" % side); hand.head, hand.tail = up.tail, (0.60 * x, 0, 1.40)
    hand.parent = up; hand.use_connect = True
bpy.ops.object.mode_set(mode='OBJECT')
deform = data.collections.new("DEF")                        # 4.0+: bone collections
for b in data.bones:
    if b.use_deform:
        deform.assign(b)
act = bpy.data.actions.new("A_Hero_Wave")
arm.animation_data_create().action = act
act.use_fake_user = True                                    # survives while unassigned
pb = arm.pose.bones["upperarm.R"]
pb.rotation_mode = 'XYZ'
for frame, deg in ((1, 0), (10, -60), (20, 0)):
    pb.rotation_euler = (0, math.radians(deg), 0)
    pb.keyframe_insert("rotation_euler", frame=frame, group=pb.name)
arm.pose.bones["root"].keyframe_insert("location", frame=1)
arm.pose.bones["root"].location = (0, -1.0, 0)
arm.pose.bones["root"].keyframe_insert("location", frame=20)
bpy.context.scene.frame_set(10)                             # evaluate a frame before reading poses
print([b.name for b in data.bones], tuple(act.frame_range), arm.matrix_world @ arm.pose.bones["hand.R"].tail)
```

## Skin a mesh (automatic weights) and check for unweighted vertices

```python
import bmesh
me = bpy.data.meshes.new("SK_Hero_Body")
bm = bmesh.new(); bmesh.ops.create_cube(bm, size=1.0)
for v in bm.verts:
    v.co = Vector((v.co.x * 1.2, v.co.y * 0.2, v.co.z * 0.2 + 1.4))
bmesh.ops.subdivide_edges(bm, edges=bm.edges, cuts=6, use_grid_fill=True)
bm.to_mesh(me); bm.free()
body = bpy.data.objects.new("SK_Hero_Body", me)
bpy.context.scene.collection.objects.link(body)
arm = bpy.data.objects["Armature"]
bpy.context.scene.frame_set(1)
for o in bpy.context.view_layer.objects:
    o.select_set(False)
body.select_set(True); arm.select_set(True)
bpy.context.view_layer.objects.active = arm                 # armature active, mesh selected
bpy.ops.object.parent_set(type='ARMATURE_AUTO')             # Armature modifier + weighted groups
print([(m.type, m.object.name) for m in body.modifiers], [g.name for g in body.vertex_groups])
print("unweighted", sum(1 for v in me.vertices if not any(g.weight > 0 for g in v.groups)))
```

## Apply an armature's scale, with its animation

`transform_apply` rescales rest bones and children but not bone location keys; scale them.

```python
arm = bpy.data.objects["Armature"]
arm.scale = (0.5, 0.5, 0.5)                                  # e.g. 0.01 after an FBX import
bpy.context.view_layer.update()
assert max(arm.scale) - min(arm.scale) < 1e-6, "non-uniform armature scale: fix by hand"
s = arm.scale.x
sel = [arm] + list(arm.children)
with bpy.context.temp_override(active_object=arm, object=arm, selected_objects=sel, selected_editable_objects=sel):
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
bones = set(arm.pose.bones.keys())
for act in bpy.data.actions:                                 # every action written for this rig
    for fc in act.fcurves:
        if fc.data_path.endswith(".location") and fc.data_path.startswith("pose.bones"):
            if fc.data_path.split('"')[1] in bones:
                for k in fc.keyframe_points:
                    k.co.y *= s; k.handle_left.y *= s; k.handle_right.y *= s
                fc.update()
print(arm.scale[:])
```

## Python for blender_rig_check's fixes

```python
import bmesh
arm = bpy.data.objects["Armature"]
body = bpy.data.objects["SK_Hero_Body"]
bpy.context.view_layer.objects.active = body
with bpy.context.temp_override(object=body, active_object=body):       # >8 influences
    bpy.ops.object.vertex_group_limit_total(group_select_mode='BONE_DEFORM', limit=8)   # 4 for mobile
    bpy.ops.object.vertex_group_normalize_all(group_select_mode='BONE_DEFORM', lock_active=False)
for g in [g for g in body.vertex_groups if g.name not in arm.data.bones]:   # groups without bones
    body.vertex_groups.remove(g)
bm = bmesh.new(); bm.from_mesh(body.data)                    # loose verts, degenerate faces, inside-out
bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_edges], context='VERTS')
bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=1e-5)
bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
bm.to_mesh(body.data); bm.free()
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode='EDIT')
for b in [b for b in arm.data.edit_bones if b.name.endswith("_end") and not b.children]:
    arm.data.edit_bones.remove(b)                            # leftover leaf bones
for b in arm.data.edit_bones:
    b.select = b.select_head = b.select_tail = True
bpy.ops.armature.symmetrize(direction='POSITIVE_X')          # +X (.L) overwrites -X (.R); NEGATIVE_X the reverse
bpy.ops.object.mode_set(mode='OBJECT')
print("non-deform groups", [g.name for g in body.vertex_groups if not arm.data.bones[g.name].use_deform])
```
