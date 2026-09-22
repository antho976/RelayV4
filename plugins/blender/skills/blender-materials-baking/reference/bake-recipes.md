# Bake and material recipes (verified)

Every block below was run, in this order, in one `blender -b --factory-startup` session on
Blender 4.0.2 (CPU Cycles, no GPU, no denoiser). The demo high/low pair exists only so the
recipes run; in real work use your own objects. Set `OUT` to a folder in the checkout (for
example `//textures` resolved with `bpy.path.abspath`). Bakes are slow on CPU: pass a larger
`timeout_s` to `blender_python` for 2k/4k maps.

## Cycles setup for baking

```python
import bpy, bmesh, os
import numpy as np

sc = bpy.context.scene
sc.render.engine = 'CYCLES'
sc.cycles.device = 'CPU'
sc.cycles.samples = 32              # default is 4096: far too slow for bakes
sc.cycles.use_denoising = False     # REQUIRED on builds without OpenImageDenoise:
                                    # otherwise bakes silently come out empty (verified)
bk = sc.render.bake
bk.margin = 16                      # px of edge padding; ~8 at 1k, 16 at 2k, 32 at 4k
bk.margin_type = 'EXTEND'           # or 'ADJACENT_FACES' (default)
OUT = bpy.path.abspath("//textures") if bpy.data.filepath else "/tmp/textures"
os.makedirs(OUT, exist_ok=True)
```

## Helpers: target image, bake target node in every material, selection, save

```python
def new_image(name, size=2048, data=True, alpha=False, float_buffer=False):
    img = bpy.data.images.get(name) or bpy.data.images.new(name, size, size, alpha=alpha, float_buffer=float_buffer)
    img.colorspace_settings.name = 'Non-Color' if data else 'sRGB'   # data maps: Non-Color
    return img

def set_bake_target(obj, img):
    """Active, unconnected Image Texture node holding img in EVERY material of obj."""
    for slot in obj.material_slots:
        nt = slot.material.node_tree
        node = nt.nodes.get("BAKE_TARGET") or nt.nodes.new('ShaderNodeTexImage')
        node.name = "BAKE_TARGET"; node.image = img
        for n in nt.nodes:
            n.select = False
        node.select = True; nt.nodes.active = node

def select_for_bake(high_objs, low_obj):
    for o in bpy.context.view_layer.objects:
        o.select_set(False)
    for h in high_objs:
        h.select_set(True)
    low_obj.select_set(True)
    bpy.context.view_layer.objects.active = low_obj      # low = active = receives the bake

from contextlib import contextmanager
@contextmanager
def only_render(objs):
    """Hide every other object from render during a bake (AO and COMBINED see the whole scene)."""
    keep = set(objs); saved = {o: o.hide_render for o in sc.objects}
    for o in sc.objects:
        o.hide_render = o not in keep
    try:
        yield
    finally:
        for o, v in saved.items():
            o.hide_render = v

def save_png(img, path):
    img.filepath_raw = path; img.file_format = 'PNG'
    img.save()          # writes pixels as-is (8-bit, or 16-bit for float_buffer images)

def stats(img):
    a = np.empty(len(img.pixels), dtype=np.float32); img.pixels.foreach_get(a)
    return a.reshape(-1, 4)[:, :3].mean(0).round(3)
```

Do not use `img.save_render()` for data maps: it applies the scene view transform (AgX in 4.x)
and changes the values (verified).

## Demo high and low poly (replace with your own)

```python
def uv_sphere(name, u, v, r):
    me = bpy.data.meshes.new(name)
    bm = bmesh.new(); bm.loops.layers.uv.new("UVMap")      # calc_uvs needs a UV layer
    bmesh.ops.create_uvsphere(bm, u_segments=u, v_segments=v, radius=r, calc_uvs=True)
    bm.to_mesh(me); bm.free()
    me.polygons.foreach_set("use_smooth", [True] * len(me.polygons))
    ob = bpy.data.objects.new(name, me); sc.collection.objects.link(ob); return ob

high = uv_sphere("SM_Rock_high", 64, 32, 0.5)
high.modifiers.new("Sub", 'SUBSURF').levels = 2       # modifiers are evaluated by the bake
d = high.modifiers.new("Disp", 'DISPLACE')
d.texture = bpy.data.textures.new("vor", 'VORONOI'); d.texture.noise_scale = 0.15; d.strength = 0.03
low = uv_sphere("SM_Rock", 24, 12, 0.5)
mat = bpy.data.materials.new("M_Rock"); mat.use_nodes = True
low.data.materials.append(mat)
```

## High -> low: tangent normal map (DirectX for Unreal) and AO

```python
select_for_bake([high], low)
img_n = new_image("T_Rock_N", 512, data=True)
set_bake_target(low, img_n)
bpy.ops.object.bake(type='NORMAL', normal_space='TANGENT',
                    normal_r='POS_X', normal_g='NEG_Y', normal_b='POS_Z',   # DirectX: green down
                    use_selected_to_active=True, cage_extrusion=0.05,       # metres
                    max_ray_distance=0.0, margin=16)
save_png(img_n, os.path.join(OUT, "T_Rock_N.png"))

img_ao = new_image("T_Rock_AO", 512, data=True)
set_bake_target(low, img_ao)
with only_render([high, low]):     # the factory Cube around the rock would make AO black
    bpy.ops.object.bake(type='AO', use_selected_to_active=True, cage_extrusion=0.05, margin=16)
save_png(img_ao, os.path.join(OUT, "T_Rock_AO.png"))
print(stats(img_n), stats(img_ao))       # ~[0.5 0.5 0.99] (blue near 1 = sane), AO ~1.0
```

With a hand-made cage instead of extrusion: `use_cage=True, cage_object="SM_Rock_cage"` (a
copy of the low poly, same topology, pushed outward).

## Procedural material -> textures on one object (no high poly)

```python
pm = bpy.data.materials.new("M_Proc"); pm.use_nodes = True
pt = pm.node_tree; bsdf = pt.nodes["Principled BSDF"]
noise = pt.nodes.new('ShaderNodeTexNoise'); noise.inputs['Scale'].default_value = 8.0
ramp = pt.nodes.new('ShaderNodeValToRGB')
bump = pt.nodes.new('ShaderNodeBump'); bump.inputs['Strength'].default_value = 0.5
pt.links.new(noise.outputs['Fac'], ramp.inputs['Fac'])
pt.links.new(ramp.outputs['Color'], bsdf.inputs['Base Color'])
pt.links.new(noise.outputs['Fac'], bsdf.inputs['Roughness'])
pt.links.new(noise.outputs['Fac'], bump.inputs['Height'])
pt.links.new(bump.outputs['Normal'], bsdf.inputs['Normal'])
low.data.materials.clear(); low.data.materials.append(pm)
select_for_bake([], low)

img_bc = new_image("T_Proc_BC", 512, data=False)              # colour: sRGB
set_bake_target(low, img_bc)
bpy.ops.object.bake(type='DIFFUSE', pass_filter={'COLOR'}, use_selected_to_active=False, margin=16)
save_png(img_bc, os.path.join(OUT, "T_Proc_BC.png"))           # COLOR only = albedo, no lighting

img_r = new_image("T_Proc_R", 512, data=True)
set_bake_target(low, img_r)
bpy.ops.object.bake(type='ROUGHNESS', use_selected_to_active=False, margin=16)

img_pn = new_image("T_Proc_N", 512, data=True)                # bump/normal inputs -> normal map
set_bake_target(low, img_pn)
bpy.ops.object.bake(type='NORMAL', normal_g='NEG_Y', use_selected_to_active=False, margin=16)
print(stats(img_bc), stats(img_r), stats(img_pn))
```

## Any value (metallic, masks, height) via an Emission bake

There is no METALLIC bake type (types: COMBINED AO SHADOW POSITION NORMAL UV ROUGHNESS EMIT
ENVIRONMENT DIFFUSE GLOSSY TRANSMISSION). Route the value through an Emission shader, bake
EMIT, restore the original link.

```python
def bake_value_via_emit(obj, mat, from_socket, img):
    nt = mat.node_tree; out = next(n for n in nt.nodes if n.type == 'OUTPUT_MATERIAL')
    orig = out.inputs['Surface'].links[0].from_socket
    em = nt.nodes.new('ShaderNodeEmission')            # strength 1.0 by default
    nt.links.new(from_socket, em.inputs['Color'])
    nt.links.new(em.outputs['Emission'], out.inputs['Surface'])
    set_bake_target(obj, img)
    bpy.ops.object.bake(type='EMIT', use_selected_to_active=False, margin=16)
    nt.links.new(orig, out.inputs['Surface']); nt.nodes.remove(em)

metal = pt.nodes.new('ShaderNodeValue'); metal.outputs[0].default_value = 0.0
img_m = new_image("T_Proc_M", 512, data=True)
bake_value_via_emit(low, pm, metal.outputs[0], img_m)
```

## Pack ORM (R = occlusion, G = roughness, B = metallic)

```python
def pack_orm(o_img, r_img, m_img, name, path):
    w, h = r_img.size
    def chan(img, default):
        if img is None:
            return np.full(w * h, default, dtype=np.float32)
        assert tuple(img.size) == (w, h), f"{img.name} size differs"
        a = np.empty(w * h * 4, dtype=np.float32); img.pixels.foreach_get(a)
        return a[0::4]                                   # R; grey bakes have R = G = B
    px = np.ones((w * h, 4), dtype=np.float32)
    px[:, 0] = chan(o_img, 1.0); px[:, 1] = chan(r_img, 0.5); px[:, 2] = chan(m_img, 0.0)
    img = bpy.data.images.new(name, w, h, alpha=False)
    img.colorspace_settings.name = 'Non-Color'
    img.pixels.foreach_set(px.ravel()); save_png(img, path); return img

orm = pack_orm(img_ao, img_r, img_m, "T_Rock_ORM", os.path.join(OUT, "T_Rock_ORM.png"))
print(stats(orm))
```

## Principled material from BC + ORM + N (preview in Blender; rebuild in Unreal)

```python
def load_tex(path, data):
    img = bpy.data.images.load(path, check_existing=True)
    img.colorspace_settings.name = 'Non-Color' if data else 'sRGB'; return img

def build_pbr_material(name, bc, orm, n, emissive_rgb=None, emissive_strength=0.0):
    m = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    m.use_nodes = True; nt = m.node_tree; nt.nodes.clear(); L = nt.links.new
    out = nt.nodes.new('ShaderNodeOutputMaterial'); out.location = (600, 0)
    p = nt.nodes.new('ShaderNodeBsdfPrincipled'); p.location = (300, 0)
    L(p.outputs['BSDF'], out.inputs['Surface'])
    t_bc = nt.nodes.new('ShaderNodeTexImage'); t_bc.image = load_tex(bc, False); t_bc.location = (-400, 300)
    L(t_bc.outputs['Color'], p.inputs['Base Color'])
    t_orm = nt.nodes.new('ShaderNodeTexImage'); t_orm.image = load_tex(orm, True); t_orm.location = (-400, 0)
    sep = nt.nodes.new('ShaderNodeSeparateColor'); sep.location = (-100, 0)
    L(t_orm.outputs['Color'], sep.inputs['Color'])
    L(sep.outputs['Green'], p.inputs['Roughness']); L(sep.outputs['Blue'], p.inputs['Metallic'])
    t_n = nt.nodes.new('ShaderNodeTexImage'); t_n.image = load_tex(n, True); t_n.location = (-400, -300)
    nm = nt.nodes.new('ShaderNodeNormalMap'); nm.location = (-100, -300)
    L(t_n.outputs['Color'], nm.inputs['Color']); L(nm.outputs['Normal'], p.inputs['Normal'])
    # a DirectX (green-down) map looks inverted here; that is expected, Unreal wants it
    if emissive_rgb:
        p.inputs['Emission Color'].default_value = (*emissive_rgb, 1.0)   # 3.x: 'Emission'
        p.inputs['Emission Strength'].default_value = emissive_strength
    return m

m = build_pbr_material("M_Rock", os.path.join(OUT, "T_Proc_BC.png"),
                       os.path.join(OUT, "T_Rock_ORM.png"), os.path.join(OUT, "T_Rock_N.png"))
low.data.materials.clear(); low.data.materials.append(m)
```

## Opacity (masked / translucent) across 4.0-4.2

```python
leaf = bpy.data.materials.new("MI_Leaf"); leaf.use_nodes = True
leaf.node_tree.nodes["Principled BSDF"].inputs['Alpha'].default_value = 0.5
if hasattr(leaf, "surface_render_method"):      # 4.2+ EEVEE Next
    leaf.surface_render_method = 'DITHERED'      # 'BLENDED' for true translucency
else:                                            # 4.0/4.1 EEVEE Legacy
    leaf.blend_method = 'CLIP'; leaf.alpha_threshold = 0.5
```

This only affects Blender's viewport/EEVEE. In Unreal set Blend Mode Masked (Opacity Mask) or
Translucent (Opacity) on the material.

## Vertex colour mask (R = top half) for Unreal material masks

```python
me = low.data
ca = me.color_attributes.new(name="Mask", type='BYTE_COLOR', domain='CORNER')
cols = np.zeros((len(me.loops), 4), dtype=np.float32); cols[:, 3] = 1.0
zs = np.array([me.vertices[l.vertex_index].co.z for l in me.loops])
cols[:, 0] = (zs > 0).astype(np.float32)
ca.data.foreach_set("color", cols.ravel())
me.color_attributes.active_color = ca
me.color_attributes.render_color_index = me.color_attributes.active_color_index
```

## Texture audit

```python
def audit_images():
    for img in bpy.data.images:
        if img.source not in {'FILE', 'GENERATED'} or img.name in {'Render Result', 'Viewer Node'}:
            continue
        w, h = img.size; pot = w > 0 and (w & (w - 1)) == 0 and (h & (h - 1)) == 0
        print(f"{img.name:14} {w}x{h} pot={pot} cs={img.colorspace_settings.name} "
              f"file={bpy.path.basename(img.filepath) or '<unsaved>'} packed={bool(img.packed_file)}")
audit_images()
```

Expect: `_BC`/`_E` sRGB, `_N`/`_ORM`/`_M`/`_H` Non-Color, power-of-two sizes, every image
saved to a file (unsaved generated images are lost when Blender quits unless packed).
