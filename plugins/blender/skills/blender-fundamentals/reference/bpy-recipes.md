# bpy recipes (verified in background mode)

Every block below was run with `blender -b --factory-startup` on Blender 4.0.2, in this order,
in one session (later blocks use objects made by earlier ones: `SM_Crate`, `Armature`,
`SK_Hero_Body`). Paste a block into `blender_python` `code`; `bpy`, `Vector`, `Matrix` and `math`
are already in scope, `bmesh` and `os` are not. Replace names with the ones `blender_info`
reports. Paths starting with `//` are relative to the opened .blend.

Contents: create - apply transforms - set origin - smooth shading - modifiers - bmesh -
collections - append/link - UV channels - materials and textures. Armatures, keys, skinning,
applying an armature's scale with its animation and the `blender_rig_check` fixes are in
`rig-recipes.md`. The operator form of
apply, selection, units and saving are short enough to live in `../SKILL.md`.

## Create a mesh object with the data API

```python
import bmesh
me = bpy.data.meshes.new("SM_Crate")
bm = bmesh.new()
bm.loops.layers.uv.new("UVMap")                        # calc_uvs needs a UV layer to fill
bmesh.ops.create_cube(bm, size=1.0, calc_uvs=True)     # 1 m cube centred on the origin
bmesh.ops.translate(bm, vec=Vector((0, 0, 0.5)), verts=bm.verts)   # base at z=0 -> pivot at the base
bm.to_mesh(me)
bm.free()
ob = bpy.data.objects.new("SM_Crate", me)              # object "SM_Crate" uses mesh data "SM_Crate"
coll = bpy.data.collections.get("Props") or bpy.data.collections.new("Props")
if coll.name not in bpy.context.scene.collection.children:
    bpy.context.scene.collection.children.link(coll)
coll.objects.link(ob)                                  # only now is it in the view layer
bpy.context.view_layer.update()                        # refresh matrix_world / dimensions
print(ob.name, ob.dimensions[:], ob.users_collection[0].name)
```

## Apply transforms with the data API (no context needed)

Handles delta transforms, shared mesh data, shape keys, negative scale and children.

```python
def apply_transform(ob, location=False, rotation=True, scale=True):
    """Bake the object's own transform into its data; nothing moves in the world."""
    basis = ob.matrix_basis.copy()                              # includes delta transforms
    ob.delta_location = (0, 0, 0); ob.delta_scale = (1, 1, 1)
    ob.delta_rotation_euler = (0, 0, 0); ob.delta_rotation_quaternion = (1, 0, 0, 0)
    ob.matrix_basis = basis                                     # deltas folded into loc/rot/scale
    loc, rot, sca = basis.decompose()
    T, R, S = Matrix.Translation(loc), rot.to_matrix().to_4x4(), Matrix.Diagonal(sca).to_4x4()
    keep = (Matrix() if location else T) @ (Matrix() if rotation else R) @ (Matrix() if scale else S)
    bake = keep.inverted() @ ob.matrix_basis                    # what moves into the data
    if ob.data is not None and hasattr(ob.data, "transform"):   # Mesh, Curve, Armature, Lattice
        if ob.data.users > 1:
            ob.data = ob.data.copy()                            # never bake into shared data
        if ob.type == 'MESH':
            ob.data.transform(bake, shape_keys=True)
            if bake.is_negative:                                # a mirror turns the mesh inside out
                ob.data.flip_normals()
        else:
            ob.data.transform(bake)
    for c in ob.children:                                       # children keep their world place
        c.matrix_parent_inverse = bake @ c.matrix_parent_inverse
    ob.matrix_basis = keep

ob = bpy.data.objects["SM_Crate"]
ob.location = (1, 2, 0); ob.rotation_euler = (0, 0, math.radians(90)); ob.scale = (2, 2, 2)
bpy.context.view_layer.update()
before = ob.matrix_world @ ob.data.vertices[0].co
apply_transform(ob)                                             # rotation + scale
apply_transform(ob, location=True, rotation=False, scale=False) # then location
bpy.context.view_layer.update()
print(ob.location[:], ob.scale[:], ob.dimensions[:], "same place:", (before - ob.matrix_world @ ob.data.vertices[0].co).length < 1e-6)
```

Do not use it on an armature that has actions (bone location keys are not rescaled) - see
"Apply an armature's scale" below. The operator does not flip normals on a negative scale
either (checked in 4.0): recalculate normals after applying any mirror.

## Set the origin (pivot) without moving the mesh

```python
def set_origin(ob, world_point):
    local = ob.matrix_world.inverted() @ Vector(world_point)
    ob.data.transform(Matrix.Translation(-local))
    for c in ob.children:
        c.matrix_parent_inverse = Matrix.Translation(-local) @ c.matrix_parent_inverse
    ob.matrix_world = ob.matrix_world @ Matrix.Translation(local)

ob = bpy.data.objects["SM_Crate"]
bpy.context.view_layer.update()
ws = [ob.matrix_world @ v.co for v in ob.data.vertices]
base = Vector(((min(v.x for v in ws) + max(v.x for v in ws)) / 2,     # centre of the base
               (min(v.y for v in ws) + max(v.y for v in ws)) / 2,
               min(v.z for v in ws)))
set_origin(ob, base)
ob.location = (0, 0, 0)                                        # base on the ground at the world origin
bpy.context.view_layer.update()
print("lowest z", min((ob.matrix_world @ v.co).z for v in ob.data.vertices))
```

## Smooth shading with hard edges by angle (4.0 and 4.1+)

```python
import bmesh
ob = bpy.data.objects["SM_Crate"]
me = ob.data
angle = math.radians(40)
bm = bmesh.new(); bm.from_mesh(me)
for f in bm.faces:
    f.smooth = True
for e in bm.edges:                        # sharp on open borders and where faces meet at > angle
    e.smooth = not (e.is_boundary or (len(e.link_faces) == 2 and e.calc_face_angle(0.0) > angle))
bm.to_mesh(me); bm.free()
if bpy.app.version < (4, 1, 0):
    me.use_auto_smooth = True             # 4.0: sharp edges split normals only with Auto Smooth
    me.auto_smooth_angle = math.pi        # let the marked edges decide
print("sharp", sum(e.use_edge_sharp for e in me.edges), "smooth faces", sum(p.use_smooth for p in me.polygons))
```

4.1+ has no `use_auto_smooth`; sharp edges always count. The live alternative there is the
"Smooth by Angle" modifier (a geometry-nodes asset); check which operators your build has with
`[n for n in dir(bpy.ops.object) if "smooth" in n]` before calling one.

## Modifiers: add, configure, reorder, read, apply

```python
ob = bpy.data.objects["SM_Crate"]
bev = ob.modifiers.new(name="Bevel", type='BEVEL')
bev.width = 0.02; bev.segments = 2; bev.limit_method = 'ANGLE'; bev.angle_limit = math.radians(30)
if bpy.app.version < (4, 1, 0):
    ob.data.use_auto_smooth = True        # 4.0: Weighted Normal errors without Auto Smooth
wn = ob.modifiers.new(name="WeightedNormal", type='WEIGHTED_NORMAL'); wn.keep_sharp = True
tri = ob.modifiers.new(name="Triangulate", type='TRIANGULATE'); tri.keep_custom_normals = True
ob.modifiers.move(ob.modifiers.find("Triangulate"), len(ob.modifiers) - 1)   # keep it last
# Read the evaluated result without applying
dg = bpy.context.evaluated_depsgraph_get()
ev = ob.evaluated_get(dg)
m = ev.to_mesh()
print("evaluated faces", len(m.polygons), "base faces", len(ob.data.polygons))
ev.to_mesh_clear()
# Apply ONE modifier (apply from the top of the stack down): operator + override
bpy.context.view_layer.objects.active = ob
with bpy.context.temp_override(object=ob, active_object=ob):
    bpy.ops.object.modifier_apply(modifier="Bevel")
# Apply the WHOLE stack with the data API. Not for skinned meshes: it would bake the pose.
dg = bpy.context.evaluated_depsgraph_get()
baked = bpy.data.meshes.new_from_object(ob.evaluated_get(dg), preserve_all_data_layers=True, depsgraph=dg)
old, name = ob.data, ob.data.name
ob.modifiers.clear()
ob.data = baked
if old.users == 0:
    bpy.data.meshes.remove(old)
baked.name = name
print("faces", len(ob.data.polygons), "modifiers", len(ob.modifiers), ob.data.name)
```

## bmesh: object-mode editing, then edit-mode editing

```python
import bmesh
ob = bpy.data.objects["SM_Crate"]
me = ob.data
bm = bmesh.new()
bm.from_mesh(me)
bm.verts.ensure_lookup_table()                              # needed before bm.verts[i]
top = [f for f in bm.faces if f.normal.z > 0.9]
bmesh.ops.inset_region(bm, faces=top, thickness=0.05, depth=0.0)
bmesh.ops.translate(bm, vec=Vector((0, 0, -0.02)), verts=list({v for f in top for v in f.verts}))
bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-4)
bmesh.ops.recalc_face_normals(bm, faces=bm.faces)          # outward normals on closed meshes
bm.to_mesh(me); me.update()
bm.free()                                                   # always free a bmesh you created
# Edit mode: from_edit_mesh shares the edit data
bpy.context.view_layer.objects.active = ob
bpy.ops.object.mode_set(mode='EDIT')
bm = bmesh.from_edit_mesh(me)
for v in bm.verts:
    v.select = v.co.z > 0.5
bm.select_flush_mode()
bmesh.update_edit_mesh(me)                                  # never bm.free() an edit-mesh bmesh
bpy.ops.object.mode_set(mode='OBJECT')                      # ob.data sees edits only after this
print("faces", len(me.polygons), "selected verts", sum(v.select for v in me.vertices))
```

## Collections, view layer, visibility

```python
scene = bpy.context.scene
col = bpy.data.collections.get("Export") or bpy.data.collections.new("Export")
if col.name not in scene.collection.children:
    scene.collection.children.link(col)
ob = bpy.data.objects["SM_Crate"]
for c in list(ob.users_collection):          # move = link to the new one, unlink from the old
    c.objects.unlink(ob)
col.objects.link(ob)
lc = bpy.context.view_layer.layer_collection.children[col.name]
lc.exclude = False                           # excluded collections take their objects out of the view layer
ob.hide_set(False)                           # eye icon (per view layer)
ob.hide_viewport = False                     # monitor icon (global)
ob.hide_render = False                       # camera icon; blender_export skips hide_render roots
bpy.context.view_layer.update()
print(ob.name in bpy.context.view_layer.objects, [c.name for c in ob.users_collection])
```

## Append (copy in) or link (reference) from another .blend

```python
src = bpy.path.abspath("//crate_only.blend")
bpy.data.libraries.write(src, {bpy.data.objects["SM_Crate"]}, fake_user=True)   # a .blend holding just these blocks
with bpy.data.libraries.load(src, link=False) as (data_from, data_to):   # link=True to link
    print("objects there:", list(data_from.objects))
    data_to.objects = [n for n in data_from.objects if n.startswith("SM_")]
for o in data_to.objects:                   # loaded, not yet in any scene
    if o is not None:
        bpy.context.scene.collection.objects.link(o)
        print("appended as", o.name)        # a clash gives "SM_Crate.001"
```

## UV channels (UV0 textures, UV1 lightmap)

```python
ob = bpy.data.objects["SM_Crate"]
me = ob.data
lm = me.uv_layers.get("UVLightmap") or me.uv_layers.new(name="UVLightmap")   # order = Unreal UV index
me.uv_layers.active = lm                                    # UV operators write to the active layer
bpy.context.view_layer.objects.active = ob
bpy.ops.object.mode_set(mode='EDIT')
bpy.ops.mesh.select_all(action='SELECT')
bpy.ops.uv.lightmap_pack(PREF_CONTEXT='ALL_FACES', PREF_MARGIN_DIV=0.2)   # or uv.smart_project(island_margin=0.02)
bpy.ops.object.mode_set(mode='OBJECT')
lm = me.uv_layers["UVLightmap"]                             # re-fetch: references die across mode switches
me.uv_layers.active = me.uv_layers[0]
for uv in me.uv_layers:
    uv.active_render = (uv == me.uv_layers[0])
print([u.name for u in me.uv_layers], min(d.uv.x for d in lm.data), max(d.uv.x for d in lm.data))
```

## Materials, slots, image textures

```python
import os
ob = bpy.data.objects["SM_Crate"]
me = ob.data
for name, color in (("M_Crate_Wood", (0.4, 0.25, 0.1, 1)), ("M_Crate_Metal", (0.6, 0.6, 0.6, 1))):
    mat = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    mat.use_nodes = True
    mat.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = color
    if mat.name not in me.materials:
        me.materials.append(mat)
for p in me.polygons:                                       # faces choose a slot by index
    p.material_index = 1 if p.normal.z > 0.9 else 0
nt = bpy.data.materials["M_Crate_Wood"].node_tree
path = bpy.path.abspath("//textures/T_Crate_N.png")
if os.path.exists(path):                                    # images.load raises on a missing file
    img = bpy.data.images.load(path, check_existing=True)
    img.colorspace_settings.name = 'Non-Color'              # normal/ORM/masks: data, not colour
    tex = nt.nodes.new("ShaderNodeTexImage"); tex.image = img
    nmap = nt.nodes.new("ShaderNodeNormalMap")              # Blender normal maps are OpenGL (+Y)
    nt.links.new(tex.outputs["Color"], nmap.inputs["Color"])
    nt.links.new(nmap.outputs["Normal"], nt.nodes["Principled BSDF"].inputs["Normal"])
for img in (i for i in bpy.data.images if i.type == 'IMAGE'):   # skip Render Result / Viewer Node
    print(img.name, bpy.path.abspath(img.filepath), "packed", img.packed_file is not None, img.colorspace_settings.name)
print([(i, s.material.name) for i, s in enumerate(ob.material_slots)])
```
