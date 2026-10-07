# Modeling recipes (verified)

Every block below was run, in this order, in one `blender -b --factory-startup` session on
Blender 4.0.2, and again on 5.2.1 after the geometry-nodes input fix (`set_gn_input`); its 4.x
branch has not been re-run on a 4.x build since that change. Paste the helpers you need into `blender_python` (they depend on each other
only as noted). All sizes are metres (1 unit = 1 m). Read results back with `print()`.

## Scene setup and a box with its pivot at the min corner

```python
import bpy, bmesh, math
from mathutils import Vector, Matrix

sc = bpy.context.scene
sc.unit_settings.system = 'METRIC'; sc.unit_settings.scale_length = 1.0
sc.unit_settings.length_unit = 'CENTIMETERS'   # display only; data stays in metres
GRID = 0.5                                      # 50 cm kit grid

def snap(v, g=GRID): return round(v / g) * g

def make_box_mesh(name, sx, sy, sz, origin=(0, 0, 0)):
    """Box spanning origin .. origin+size, so the object origin is the min corner."""
    me = bpy.data.meshes.new(name)
    bm = bmesh.new(); bmesh.ops.create_cube(bm, size=1.0)      # centred, -0.5..0.5
    bmesh.ops.translate(bm, verts=bm.verts, vec=(0.5, 0.5, 0.5))
    bmesh.ops.scale(bm, verts=bm.verts, vec=(sx, sy, sz))
    bmesh.ops.translate(bm, verts=bm.verts, vec=origin)
    bm.to_mesh(me); bm.free(); return me

def link(obj, coll=None):
    (coll or bpy.context.scene.collection).objects.link(obj); return obj
```

## Modular wall 400 x 300 cm, 20 cm thick, pivot at base corner, on the 50 cm grid

```python
wall = link(bpy.data.objects.new("SM_Kit_Wall_400x300", make_box_mesh("SM_Kit_Wall_400x300", 4.0, 0.2, 3.0)))
wall.location = (snap(2.2), 0.0, 0.0)         # snapped placement -> x = 2.0
bpy.context.view_layer.update()               # matrix_world is stale until this
assert all(abs(c / GRID - round(c / GRID)) < 1e-6
           for v in wall.data.vertices for c in (v.co.x, v.co.z))
print([round(d * 100, 2) for d in wall.dimensions])      # [400.0, 20.0, 300.0]
```

## Bevel + weighted normals (hard surface), 4.0 and 4.1+

```python
me = wall.data
bev = wall.modifiers.new("Bevel", 'BEVEL')
bev.width = 0.01; bev.segments = 2; bev.harden_normals = True
bev.limit_method = 'ANGLE'; bev.angle_limit = math.radians(30)
wn = wall.modifiers.new("WeightedNormal", 'WEIGHTED_NORMAL')
wn.mode = 'FACE_AREA'; wn.keep_sharp = True; wn.weight = 50
me.polygons.foreach_set("use_smooth", [True] * len(me.polygons))
if hasattr(me, "use_auto_smooth"):            # <= 4.0: custom normals need auto smooth
    me.use_auto_smooth = True; me.auto_smooth_angle = math.radians(30)
me.update()
ev = wall.evaluated_get(bpy.context.evaluated_depsgraph_get()).data
print(len(ev.polygons), ev.has_custom_normals)            # 54 True
```

## Sharp edges by angle, portable across 4.0-4.2

```python
def sharp_by_angle(mesh, angle_deg=30.0):
    bm = bmesh.new(); bm.from_mesh(mesh)
    lim = math.radians(angle_deg)
    n = 0
    for f in bm.faces:
        f.smooth = True
    for e in bm.edges:
        sharp = (not e.is_manifold) or e.calc_face_angle(0.0) > lim
        e.smooth = not sharp; n += sharp
    bm.to_mesh(mesh); bm.free()
    if hasattr(mesh, "use_auto_smooth"):      # 4.0: honour only the marked edges
        mesh.use_auto_smooth = True; mesh.auto_smooth_angle = math.pi
    mesh.update(); return n
```

## Cleanup and a mesh report

```python
def clean_mesh(mesh, dist=0.0001):
    bm = bmesh.new(); bm.from_mesh(mesh)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=dist)          # merge by distance
    bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=dist)
    bmesh.ops.delete(bm, geom=[e for e in bm.edges if not e.link_faces], context='EDGES')
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_edges], context='VERTS')
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)                # outward
    bm.to_mesh(mesh); bm.free(); mesh.update()

def mesh_report(obj):   # the evaluated mesh (modifiers applied), as the export writes it
    bm = bmesh.new(); bm.from_object(obj, bpy.context.evaluated_depsgraph_get())
    r = dict(non_manifold_edges=sum(not e.is_manifold for e in bm.edges),
             zero_area=sum(f.calc_area() < 1e-8 for f in bm.faces),
             ngons=sum(len(f.verts) > 4 for f in bm.faces),
             loose_verts=sum(not v.link_faces for v in bm.verts))
    bm.free(); return r
```

## Apply modifiers without operators; boolean door cut

```python
def apply_all_modifiers(obj):
    dg = bpy.context.evaluated_depsgraph_get()
    new = bpy.data.meshes.new_from_object(obj.evaluated_get(dg), preserve_all_data_layers=True, depsgraph=dg)
    old = obj.data
    obj.modifiers.clear(); obj.data = new; new.name = old.name
    if old.users == 0: bpy.data.meshes.remove(old)

cutter = link(bpy.data.objects.new("CUT_Door", make_box_mesh("CUT_Door", 1.0, 1.0, 2.0, origin=(1.5, -0.5, 0.0))))
cutter.display_type = 'WIRE'; cutter.hide_render = True
door_wall = link(bpy.data.objects.new("SM_Kit_WallDoor_400x300", make_box_mesh("SM_Kit_WallDoor_400x300", 4.0, 0.2, 3.0)))
b = door_wall.modifiers.new("Door", 'BOOLEAN')
b.operation = 'DIFFERENCE'; b.solver = 'EXACT'; b.object = cutter
apply_all_modifiers(door_wall)
clean_mesh(door_wall.data)
print(mesh_report(door_wall))   # {'non_manifold_edges': 0, 'zero_area': 0, 'ngons': 2, 'loose_verts': 0}
# operator route: with bpy.context.temp_override(object=o, active_object=o, selected_objects=[o]):
#                     bpy.ops.object.modifier_apply(modifier="Door")
```

## Mirror modifier and bmesh symmetrize

```python
half = link(bpy.data.objects.new("SM_Mirror", make_box_mesh("SM_Mirror", 0.5, 0.5, 0.5)))
mi = half.modifiers.new("Mirror", 'MIRROR')
mi.use_axis = (True, False, False); mi.use_clip = True
mi.use_mirror_merge = True; mi.merge_threshold = 0.0001
# destructive alternative: copy the -X half onto +X
# bmesh.ops.symmetrize(bm, input=bm.verts[:] + bm.edges[:] + bm.faces[:], direction='-X', dist=1e-4)
```

## UVs: smart project, seams + unwrap + pack, lightmap channel

```python
def edit_mode_on(obj):
    bpy.context.view_layer.objects.active = obj
    for o in bpy.context.view_layer.objects: o.select_set(o == obj)
    bpy.ops.object.mode_set(mode='EDIT')
    bpy.ops.mesh.select_all(action='SELECT')

def smart_uv(obj, angle=66.0, margin=0.02):
    if not obj.data.uv_layers: obj.data.uv_layers.new(name="UVMap")
    edit_mode_on(obj)
    bpy.ops.uv.smart_project(angle_limit=math.radians(angle), island_margin=margin)
    bpy.ops.object.mode_set(mode='OBJECT')

smart_uv(door_wall)

crate = link(bpy.data.objects.new("SM_Crate", make_box_mesh("SM_Crate", 1, 1, 1, origin=(-0.5, -0.5, 0))))
crate.data.uv_layers.new(name="UVMap")
bm = bmesh.new(); bm.from_mesh(crate.data)
for e in bm.edges: e.seam = True               # every hard edge is a seam on a box
bm.to_mesh(crate.data); bm.free()
edit_mode_on(crate)
bpy.ops.uv.unwrap(method='ANGLE_BASED', margin=0.02)
bpy.ops.uv.pack_islands(margin=0.02, rotate=True)
bpy.ops.object.mode_set(mode='OBJECT')

lm = crate.data.uv_layers.new(name="UVMap_Lightmap")   # second layer = UV1 in Unreal
crate.data.uv_layers.active = lm                       # UV ops write the active layer
edit_mode_on(crate)
bpy.ops.uv.smart_project(angle_limit=math.radians(66), island_margin=0.05)
bpy.ops.uv.pack_islands(margin=0.05, rotate=False)
bpy.ops.object.mode_set(mode='OBJECT')
crate.data.uv_layers.active = crate.data.uv_layers["UVMap"]
```

## Texel density: measure and scale to target

```python
def texel_density(obj, tex_px=2048, uv_name=None):
    """Pixels per cm for a tex_px texture (world space, applied scale included)."""
    bm = bmesh.new(); bm.from_mesh(obj.data); bm.transform(obj.matrix_world)
    uv = bm.loops.layers.uv[uv_name] if uv_name else bm.loops.layers.uv.active
    a3d = sum(f.calc_area() for f in bm.faces); auv = 0.0
    for f in bm.faces:
        p = [l[uv].uv for l in f.loops]
        auv += abs(sum(p[i].x * p[i - 1].y - p[i - 1].x * p[i].y for i in range(len(p)))) * 0.5
    bm.free()
    return math.sqrt(auv / a3d) * tex_px / 100.0 if a3d else 0.0

def scale_uvs_to_density(obj, target_px_per_cm, tex_px, uv_name="UVMap"):
    k = target_px_per_cm / texel_density(obj, tex_px, uv_name)
    uvl = obj.data.uv_layers[uv_name]
    co = [0.0] * (len(uvl.data) * 2); uvl.data.foreach_get("uv", co)
    uvl.data.foreach_set("uv", [c * k for c in co]); obj.data.update()

scale_uvs_to_density(crate, 5.12, 2048)        # 512 px/m
print(round(texel_density(crate, 2048, "UVMap"), 3))      # 5.12
```

## LOD chain with Decimate (collapse)

```python
def make_lods(src, ratios=(1.0, 0.5, 0.25, 0.1)):
    base = src.name; src.name = base + "_LOD0"; out = [src]
    for i, r in enumerate(ratios[1:], start=1):
        o = bpy.data.objects.new(f"{base}_LOD{i}", src.data.copy())
        # Parent to LOD0 so blender_export of LOD0 brings its _LODn children along.
        o.parent = src; o.matrix_world = src.matrix_world.copy()
        for c in src.users_collection: c.objects.link(o)
        d = o.modifiers.new("Decimate", 'DECIMATE')
        d.decimate_type = 'COLLAPSE'; d.ratio = r; d.use_collapse_triangulate = True
        apply_all_modifiers(o); out.append(o)
    return out

rme = bpy.data.meshes.new("SM_Rock")
bm = bmesh.new(); bm.loops.layers.uv.new("UVMap")
bmesh.ops.create_uvsphere(bm, u_segments=48, v_segments=24, radius=0.5, calc_uvs=True)
bm.to_mesh(rme); bm.free()
rock = link(bpy.data.objects.new("SM_Rock", rme))
for o in make_lods(rock):
    o.data.calc_loop_triangles(); print(o.name, len(o.data.loop_triangles))
# SM_Rock_LOD0 2208, _LOD1 1104, _LOD2 552, _LOD3 220
```

## UCX collision: box and reduced convex hull

```python
def ucx_box(render_obj, idx=1, pad=0.0):
    bb = [Vector(c) for c in render_obj.bound_box]                    # local space
    mn = Vector([min(v[i] for v in bb) - pad for i in range(3)])
    mx = Vector([max(v[i] for v in bb) + pad for i in range(3)])
    name = f"UCX_{render_obj.name}_{idx:02d}"
    col = bpy.data.objects.new(name, make_box_mesh(name, *(mx - mn), origin=tuple(mn)))
    for c in render_obj.users_collection: c.objects.link(col)
    col.parent = render_obj; col.matrix_parent_inverse = Matrix.Identity(4)
    col.display_type = 'WIRE'; return col

def hull_mesh_from_points(name, points, cell=None):
    """Convex hull of points as a new mesh; cell (m) snaps points to a grid to cut the count."""
    pts = [Vector(p) for p in points]
    if cell:
        pts = list({tuple(round(c / cell) for c in p): p for p in pts}.values())
    bm = bmesh.new(); verts = [bm.verts.new(p) for p in pts]
    res = bmesh.ops.convex_hull(bm, input=verts, use_existing_faces=False)
    bmesh.ops.delete(bm, geom=[g for g in res["geom_interior"] + res["geom_unused"]
                               if isinstance(g, bmesh.types.BMVert)], context='VERTS')
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    me = bpy.data.meshes.new(name); bm.to_mesh(me); bm.free(); return me

ucx_box(wall)                                  # UCX_SM_Kit_Wall_400x300_01, 400x20x300 cm
lod0 = bpy.data.objects["SM_Rock_LOD0"]
h = bpy.data.objects.new("UCX_SM_Rock_LOD0_01", hull_mesh_from_points("UCX_SM_Rock_LOD0_01", [v.co for v in lod0.data.vertices], cell=0.3))
link(h); h.parent = lod0; h.display_type = 'WIRE'
print(len(h.data.vertices))                    # 50 (1106 without cell)
```

Name collision after the mesh as it will be named in Unreal (LOD0 object or the render mesh).

## Apply rotation/scale, set origin (data API and operator)

```python
def apply_rot_scale(obj):
    q = obj.rotation_quaternion if obj.rotation_mode == 'QUATERNION' else obj.rotation_euler.to_quaternion()
    obj.data.transform(Matrix.LocRotScale(None, q, obj.scale))
    obj.rotation_euler = (0, 0, 0); obj.rotation_quaternion = (1, 0, 0, 0); obj.scale = (1, 1, 1)
    obj.data.update()          # mesh data shared by other objects is changed for them too

def set_origin(obj, local_point):
    """Move the pivot to local_point without moving the geometry in the world."""
    obj.data.transform(Matrix.Translation(-Vector(local_point)))
    obj.matrix_world = obj.matrix_world @ Matrix.Translation(Vector(local_point))
# operator: with temp_override(object=o, active_object=o, selected_objects=[o], selected_editable_objects=[o]):
#               bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
```

## Array along the kit grid; a geometry-nodes scatter

```python
arr = link(bpy.data.objects.new("Wall_Run", wall.data))
am = arr.modifiers.new("Array", 'ARRAY')
am.count = 3; am.use_relative_offset = False
am.use_constant_offset = True; am.constant_offset_displace = (4.0, 0, 0)

ng = bpy.data.node_groups.new("GN_ScatterBolts", 'GeometryNodeTree')
ng.interface.new_socket(name="Geometry", in_out='INPUT', socket_type='NodeSocketGeometry')
ng.interface.new_socket(name="Geometry", in_out='OUTPUT', socket_type='NodeSocketGeometry')
dens = ng.interface.new_socket(name="Density", in_out='INPUT', socket_type='NodeSocketFloat')
dens.default_value = 20.0
N, L = ng.nodes, ng.links
gi, go = N.new('NodeGroupInput'), N.new('NodeGroupOutput')
dist, inst = N.new('GeometryNodeDistributePointsOnFaces'), N.new('GeometryNodeInstanceOnPoints')
ico = N.new('GeometryNodeMeshIcoSphere'); ico.inputs['Radius'].default_value = 0.02
real = N.new('GeometryNodeRealizeInstances')    # unrealized instances do not export
join = N.new('GeometryNodeJoinGeometry')
L.new(gi.outputs['Geometry'], dist.inputs['Mesh']); L.new(gi.outputs['Density'], dist.inputs['Density'])
L.new(dist.outputs['Points'], inst.inputs['Points']); L.new(ico.outputs['Mesh'], inst.inputs['Instance'])
L.new(inst.outputs['Instances'], real.inputs['Geometry'])
L.new(gi.outputs['Geometry'], join.inputs['Geometry']); L.new(real.outputs['Geometry'], join.inputs['Geometry'])
L.new(join.outputs['Geometry'], go.inputs['Geometry'])
gm = crate.modifiers.new("Bolts", 'NODES'); gm.node_group = ng
def set_gn_input(mod, ident, value):            # ident: the interface socket's .identifier
    if hasattr(mod, "properties"):              # 5.x: mod[ident] = v raises TypeError
        getattr(mod.properties.inputs, ident).value = value
    else:                                       # 4.x: inputs are keyed by socket identifier
        mod[ident] = value
set_gn_input(gm, dens.identifier, 40.0)
```

## Measuring (evaluated, world space, cm) and triangle count

```python
def world_bbox_cm(obj):
    dg = bpy.context.evaluated_depsgraph_get(); oe = obj.evaluated_get(dg)
    pts = [oe.matrix_world @ Vector(c) for c in oe.bound_box]
    mn = [min(p[i] for p in pts) * 100 for i in range(3)]
    mx = [max(p[i] for p in pts) * 100 for i in range(3)]
    return [round(a, 1) for a in mn], [round(b - a, 1) for a, b in zip(mn, mx)]

def tri_count(obj):
    m = obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).data
    m.calc_loop_triangles(); return len(m.loop_triangles)

bpy.context.view_layer.update()
print(world_bbox_cm(wall), tri_count(wall))    # ([200.0, 0.0, 0.0], [400.0, 20.0, 300.0]) ...
print(world_bbox_cm(arr))                       # x size 1200.0
```
