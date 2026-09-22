# Unreal prep recipes (verified in background mode)

Run with `blender -b --factory-startup` on Blender 4.0.2, in this order, on a scene that
already has the `SM_Crate` mesh (base at z=0) and the `Armature` rig with `hand.R` from
`blender-fundamentals` (`reference/bpy-recipes.md`, `reference/rig-recipes.md`). Paste a block
into `blender_python` `code`; `bpy`, `Vector`, `Matrix`, `math` are in scope; import `bmesh`
and `os` yourself. Replace the names with the ones `blender_info` reports.

Contents: UCX_ collision - SOCKET_ empties - LOD copies - held-item pivot at the grip -
n-gons before tangents - comparing a rig with the shared skeleton's rig - a hand-rolled FBX
export with the tool's settings.

## UCX_ collision from convex pieces

```python
import bmesh
def add_ucx(render_ob, points_world, index):
    """One convex piece (hull of the points), named UCX_<RenderMesh>_NN, parented to the mesh."""
    bm = bmesh.new()
    inv = render_ob.matrix_world.inverted()
    for p in points_world:
        bm.verts.new(inv @ Vector(p))
    hull = bmesh.ops.convex_hull(bm, input=bm.verts)
    bmesh.ops.delete(bm, geom=hull["geom_interior"] + hull["geom_unused"], context='VERTS')
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    name = "UCX_%s_%02d" % (render_ob.name, index)
    me = bpy.data.meshes.new(name); bm.to_mesh(me); bm.free()
    col = bpy.data.objects.new(name, me)
    for c in render_ob.users_collection:
        c.objects.link(col)
    col.parent = render_ob                         # blender_export (static) takes children along
    col.matrix_world = render_ob.matrix_world.copy()
    col.display_type = 'WIRE'
    col.hide_render = True                         # out of renders; export still includes it
    return col

ob = bpy.data.objects["SM_Crate"]
bpy.context.view_layer.update()
ws = [ob.matrix_world @ v.co for v in ob.data.vertices]
lo = Vector((min(p.x for p in ws), min(p.y for p in ws), min(p.z for p in ws)))
hi = Vector((max(p.x for p in ws), max(p.y for p in ws), max(p.z for p in ws)))
box = [Vector((x, y, z)) for x in (lo.x, hi.x) for y in (lo.y, hi.y) for z in (lo.z, hi.z)]
u = add_ucx(ob, box, 0)                            # more pieces: add_ucx(ob, part_points, 1), ...
print(u.name, "verts", len(u.data.vertices), "faces", len(u.data.polygons))
```

For a concave object (an L-shaped counter, a table), call `add_ucx` once per convex part with
the points of that part (its vertices, or a hand-made box around it). One hull around an
L-shape fills the inside corner.

## SOCKET_ empties (static meshes)

```python
ob = bpy.data.objects["SM_Crate"]
s = bpy.data.objects.new("SOCKET_Top", None)       # object data None = an empty
s.empty_display_type = 'ARROWS'; s.empty_display_size = 0.2
for c in ob.users_collection:
    c.objects.link(s)
s.parent = ob
s.matrix_world = ob.matrix_world @ Matrix.Translation((0, 0, 1.0))   # on top of a 1 m crate
print(s.name, s.parent.name, tuple(round(v, 3) for v in s.matrix_world.translation))
```

The socket arrives in Unreal as `Top`. Its rotation matters for anything attached: point the
empty's +X where the attached item's +X should point.

## LOD copies named <Mesh>_LODn

```python
ob = bpy.data.objects["SM_Crate"]
for i, ratio in ((1, 0.5), (2, 0.25)):
    lod = ob.copy()                                 # new object...
    lod.data = ob.data.copy()                       # ...with its own mesh
    lod.name = "%s_LOD%d" % (ob.name, i)
    for c in ob.users_collection:
        c.objects.link(lod)
    lod.modifiers.clear()
    d = lod.modifiers.new("Decimate", 'DECIMATE'); d.ratio = ratio   # applied at export
    lod.parent = ob; lod.matrix_world = ob.matrix_world.copy()
dg = bpy.context.evaluated_depsgraph_get()
for o in [ob] + [c for c in ob.children if "_LOD" in c.name]:
    m = o.evaluated_get(dg).to_mesh(); m.calc_loop_triangles()
    print(o.name, "tris", len(m.loop_triangles))
    o.evaluated_get(dg).to_mesh_clear()
```

Check in `blender_to_unreal`'s `imported` list what Unreal made of them (see SKILL.md, LODs).

## Held item: origin at the grip, long axis +X

```python
import bmesh
me = bpy.data.meshes.new("SM_Sword")
bm = bmesh.new(); bmesh.ops.create_cube(bm, size=1.0)
for v in bm.verts:                                 # handle x=-0.1..0, blade to x=0.9: along +X
    v.co = Vector((v.co.x + 0.4, v.co.y * 0.05, v.co.z * 0.01))
bm.to_mesh(me); bm.free()
sword = bpy.data.objects.new("SM_Sword", me)
bpy.context.scene.collection.objects.link(sword)
sword.location = (3, 0, 1)
bpy.context.view_layer.update()
grip_world = sword.matrix_world @ Vector((-0.05, 0, 0))   # where the palm closes
local = sword.matrix_world.inverted() @ grip_world         # set_origin() from blender-fundamentals
sword.data.transform(Matrix.Translation(-local))
sword.matrix_world = sword.matrix_world @ Matrix.Translation(local)
# To check the hold in Blender (blender_render / blender_anim_inspect), parent it to the hand.
# A skeletal export does NOT include it: the item ships as its own SM_ and attaches to a socket.
arm = bpy.data.objects["Armature"]
bpy.context.scene.frame_set(1)
sword.parent = arm; sword.parent_type = 'BONE'; sword.parent_bone = "hand.R"
bpy.context.view_layer.update()
sword.matrix_world = arm.matrix_world @ arm.pose.bones["hand.R"].matrix   # grip at the bone head; offset to the palm as needed
bpy.context.view_layer.update()
print("origin", tuple(round(v, 3) for v in sword.matrix_world.translation), "parent bone", sword.parent_bone)
```

Export the item with its rotation and location zeroed (unparent a copy, or keep a clean source
object): `apply_transform` from the fundamentals recipes on the un-parented item.

## N-gons before exporting tangents

```python
ob = bpy.data.objects["SM_Crate"]
ngons = [p.index for p in ob.data.polygons if len(p.vertices) > 4]
print("ngons", len(ngons))
if ngons and not any(m.type == 'TRIANGULATE' for m in ob.modifiers):
    t = ob.modifiers.new("Triangulate", 'TRIANGULATE')       # last in the stack, applied at export
    t.quad_method = 'BEAUTY'; t.ngon_method = 'BEAUTY'
    t.min_vertices = 5                                       # only n-gons; 4 also splits quads
    t.keep_custom_normals = True                             # 4.0: needs mesh.use_auto_smooth
```

Triangulate quads too (`min_vertices = 4`) when a normal map was baked from this mesh: Blender
and Unreal may split a quad along different diagonals otherwise.

## Compare a rig with the shared skeleton's source rig

```python
REF = bpy.path.abspath("//../base/SK_Base.blend")   # the .blend the shared Skeleton came from
arm = bpy.data.objects["Armature"]
with bpy.data.libraries.load(REF, link=True) as (src, dst):
    dst.armatures = list(src.armatures)
ref = dst.armatures[0]
def table(a):
    return {b.name: (b.parent.name if b.parent else None, b.head_local.copy()) for b in a.bones}
mine, theirs = table(arm.data), table(ref)
print("missing here:", sorted(set(theirs) - set(mine)))
print("extra here:", sorted(set(mine) - set(theirs)))       # merged into the Skeleton on import
for n in sorted(set(mine) & set(theirs)):
    if mine[n][0] != theirs[n][0]:
        print("parent differs:", n, mine[n][0], "vs", theirs[n][0])
    elif (mine[n][1] - theirs[n][1]).length > 0.01:
        print("rest head moved %.1f cm: %s" % ((mine[n][1] - theirs[n][1]).length * 100, n))
```

## A hand-rolled export with the tool's settings

Use `blender_export` normally. When you need something it does not offer, call the operator
with the same settings (these are the ones `blender_export` uses for a static mesh):

```python
import os
out = bpy.path.abspath("//export/SM_Crate.fbx")
os.makedirs(os.path.dirname(out), exist_ok=True)
ob = bpy.data.objects["SM_Crate"]
sel = [ob] + list(ob.children_recursive)          # UCX_, SOCKET_, _LODn children
with bpy.context.temp_override(selected_objects=sel, active_object=ob, object=ob):
    bpy.ops.export_scene.fbx(
        filepath=out, use_selection=True, object_types={'MESH', 'EMPTY', 'ARMATURE'},
        apply_unit_scale=True, apply_scale_options='FBX_SCALE_NONE',   # skeletal: FBX_SCALE_ALL
        axis_forward='-Z', axis_up='Y', mesh_smooth_type='FACE', use_tspace=True,
        use_mesh_modifiers=True, add_leaf_bones=False, use_armature_deform_only=True,
        bake_anim=False)
print("fbx bytes", os.path.getsize(out))
```

Check the operator's arguments in your version with
`bpy.ops.export_scene.fbx.get_rna_type().properties.keys()`.
