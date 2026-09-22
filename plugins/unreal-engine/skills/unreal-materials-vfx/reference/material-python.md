# Scripting materials with Python (`ue_python`)

Needs the editor open with PythonScriptPlugin and EditorScriptingUtilities enabled
(`ue_setup_check`). All snippets run through `ue_python`. Wrap mutations in
`unreal.ScopedEditorTransaction` and save what you touch. When unsure of a name, introspect
first instead of guessing:

```python
import unreal
mel = unreal.MaterialEditingLibrary
print([n for n in dir(mel) if not n.startswith("_")])
print([n for n in dir(unreal) if n.startswith("MaterialExpression")][:400])
help(unreal.MaterialExpressionTextureSampleParameter2D)
```

## Core API (unreal.MaterialEditingLibrary)

| Function | Purpose |
|---|---|
| `create_material_expression(material, ExprClass, x, y)` | Add a node to a material, returns it |
| `create_material_expression_in_function(func, ExprClass, x, y)` | Add a node to a material function |
| `connect_material_expressions(from_expr, from_output, to_expr, to_input)` | Wire node to node (`""` = first output/input) |
| `connect_material_property(from_expr, from_output, unreal.MaterialProperty.X)` | Wire node to a material output pin |
| `delete_material_expression(material, expr)` / `delete_all_material_expressions(material)` | Remove nodes |
| `layout_material_expressions(material)` | Auto-arrange the graph |
| `recompile_material(material)` | Compile after edits (required) |
| `update_material_function(func, preview_material)` | Propagate function edits |
| `set_material_instance_parent(mi, parent)` | Re-parent an instance |
| `set_material_instance_scalar_parameter_value(mi, name, value)` | Also `_vector_`, `_texture_`, `_static_switch_` variants |
| `get_material_instance_scalar_parameter_value(mi, name)` | Read back (also vector/texture variants) |
| `update_material_instance(mi)` | Refresh after parameter changes |
| `get_scalar_parameter_names(material)` | Also `get_vector_`, `get_texture_`, `get_static_switch_parameter_names` |

Material output pins (`unreal.MaterialProperty`): `MP_BASE_COLOR`, `MP_METALLIC`,
`MP_SPECULAR`, `MP_ROUGHNESS`, `MP_NORMAL`, `MP_EMISSIVE_COLOR`, `MP_OPACITY`,
`MP_OPACITY_MASK`, `MP_AMBIENT_OCCLUSION`, `MP_WORLD_POSITION_OFFSET`.

Common output names: texture samples `"RGB"`, `"R"`, `"G"`, `"B"`, `"A"`, `"RGBA"`;
most math nodes have one output (`""`). Common input names: `"A"`, `"B"` (Add, Multiply,
Subtract, Divide), `"A"`, `"B"`, `"Alpha"` (LinearInterpolate), `"UVs"` (texture samples),
`""` for single-input nodes (OneMinus, ComponentMask). The functions return `False` on a bad
pin name - check the return value.

## 1. Master material with parameters (PBR, ORM packed)

Every texture parameter node needs a default texture, and the node's sampler type must match
that texture's compression/sRGB settings or the material fails to compile ("Sampler type is
X, should be Y"). New `TextureSampleParameter2D` nodes default to an sRGB color engine
texture, which is only valid for `SAMPLERTYPE_COLOR`. So assign the project's own textures
and derive the sampler type from them:

```python
import unreal
at = unreal.AssetToolsHelpers.get_asset_tools()
mel = unreal.MaterialEditingLibrary
TCS, ST = unreal.TextureCompressionSettings, unreal.MaterialSamplerType

def sampler_for(tex):
    cs, srgb = tex.get_editor_property("compression_settings"), tex.get_editor_property("srgb")
    if cs == TCS.TC_NORMALMAP: return ST.SAMPLERTYPE_NORMAL
    if cs == TCS.TC_MASKS:     return ST.SAMPLERTYPE_MASKS
    if cs in (TCS.TC_GRAYSCALE, TCS.TC_ALPHA):
        return ST.SAMPLERTYPE_GRAYSCALE if srgb else ST.SAMPLERTYPE_LINEAR_GRAYSCALE
    return ST.SAMPLERTYPE_COLOR if srgb else ST.SAMPLERTYPE_LINEAR_COLOR

def tex_param(mat, name, tex_path, x, y):
    node = mel.create_material_expression(mat, unreal.MaterialExpressionTextureSampleParameter2D, x, y)
    tex = unreal.load_asset(tex_path)
    node.set_editor_property("parameter_name", name)
    node.set_editor_property("texture", tex)
    node.set_editor_property("sampler_type", sampler_for(tex))
    return node

path, name = "/Game/Materials", "M_Master_Surface"
with unreal.ScopedEditorTransaction("Create M_Master_Surface"):
    mat = at.create_asset(name, path, unreal.Material, unreal.MaterialFactoryNew())

    base_tex = tex_param(mat, "BaseColorTex", "/Game/Textures/T_Rock_BC", -800, -300)
    tint = mel.create_material_expression(mat, unreal.MaterialExpressionVectorParameter, -800, -100)
    tint.set_editor_property("parameter_name", "Tint")
    tint.set_editor_property("default_value", unreal.LinearColor(1, 1, 1, 1))
    mul = mel.create_material_expression(mat, unreal.MaterialExpressionMultiply, -400, -200)
    assert mel.connect_material_expressions(base_tex, "RGB", mul, "A")
    assert mel.connect_material_expressions(tint, "", mul, "B")
    assert mel.connect_material_property(mul, "", unreal.MaterialProperty.MP_BASE_COLOR)

    normal = tex_param(mat, "NormalTex", "/Game/Textures/T_Rock_N", -800, 150)
    assert mel.connect_material_property(normal, "RGB", unreal.MaterialProperty.MP_NORMAL)

    orm = tex_param(mat, "ORMTex", "/Game/Textures/T_Rock_ORM", -800, 450)
    assert mel.connect_material_property(orm, "R", unreal.MaterialProperty.MP_AMBIENT_OCCLUSION)
    rough_scale = mel.create_material_expression(mat, unreal.MaterialExpressionScalarParameter, -600, 650)
    rough_scale.set_editor_property("parameter_name", "RoughnessScale")
    rough_scale.set_editor_property("default_value", 1.0)
    rough_mul = mel.create_material_expression(mat, unreal.MaterialExpressionMultiply, -350, 550)
    assert mel.connect_material_expressions(orm, "G", rough_mul, "A")
    assert mel.connect_material_expressions(rough_scale, "", rough_mul, "B")
    assert mel.connect_material_property(rough_mul, "", unreal.MaterialProperty.MP_ROUGHNESS)
    assert mel.connect_material_property(orm, "B", unreal.MaterialProperty.MP_METALLIC)

    mel.layout_material_expressions(mat)
    mel.recompile_material(mat)
    unreal.EditorAssetLibrary.save_asset(mat.get_path_name())
print(mat.get_path_name())
```
Fix the texture settings first (section 7) - the sampler type is derived from them. If the
project has no suitable textures yet, use `VectorParameter`/`ScalarParameter` nodes instead
of texture parameters. Check `ue_log` (filter `Material|Error`) after recompiling.

## 2. Material settings (domain, blend mode, shading model)

```python
mat = unreal.load_asset("/Game/Materials/M_Glow")
with unreal.ScopedEditorTransaction("Configure M_Glow"):
    mat.set_editor_property("blend_mode", unreal.BlendMode.BLEND_ADDITIVE)
    mat.set_editor_property("shading_model", unreal.MaterialShadingModel.MSM_UNLIT)
    mat.set_editor_property("two_sided", True)
    # mat.set_editor_property("material_domain", unreal.MaterialDomain.MD_POST_PROCESS)
    # mat.set_editor_property("used_with_niagara_sprites", True)  # usage flags
    unreal.MaterialEditingLibrary.recompile_material(mat)
    unreal.EditorAssetLibrary.save_asset(mat.get_path_name())
```
Enum members vary slightly by version; list them with
`print([m for m in dir(unreal.BlendMode) if m.isupper()])` (same for `MaterialDomain`,
`MaterialShadingModel`, `BlendableLocation`).

## 3. Emissive pulse with time and a static switch

```python
mat = unreal.load_asset("/Game/Materials/M_Master_Surface")
mel = unreal.MaterialEditingLibrary
with unreal.ScopedEditorTransaction("Add emissive pulse"):
    t = mel.create_material_expression(mat, unreal.MaterialExpressionTime, -1000, 900)
    sine = mel.create_material_expression(mat, unreal.MaterialExpressionSine, -850, 900)
    mel.connect_material_expressions(t, "", sine, "")
    color = mel.create_material_expression(mat, unreal.MaterialExpressionVectorParameter, -850, 1050)
    color.set_editor_property("parameter_name", "EmissiveColor")
    color.set_editor_property("default_value", unreal.LinearColor(0, 0.5, 4, 1))
    mul = mel.create_material_expression(mat, unreal.MaterialExpressionMultiply, -650, 950)
    mel.connect_material_expressions(sine, "", mul, "A")
    mel.connect_material_expressions(color, "", mul, "B")
    switch = mel.create_material_expression(mat, unreal.MaterialExpressionStaticSwitchParameter, -450, 950)
    switch.set_editor_property("parameter_name", "UsePulse")
    switch.set_editor_property("default_value", False)
    zero = mel.create_material_expression(mat, unreal.MaterialExpressionConstant, -650, 1100)
    mel.connect_material_expressions(mul, "", switch, "True")
    mel.connect_material_expressions(zero, "", switch, "False")
    mel.connect_material_property(switch, "", unreal.MaterialProperty.MP_EMISSIVE_COLOR)
    mel.recompile_material(mat)
    unreal.EditorAssetLibrary.save_asset(mat.get_path_name())
```
Remember: `UsePulse` doubles the permutations of every instance that toggles it.

## 4. Material instances

```python
import unreal
at = unreal.AssetToolsHelpers.get_asset_tools()
mel = unreal.MaterialEditingLibrary
parent = unreal.load_asset("/Game/Materials/M_Master_Surface")

with unreal.ScopedEditorTransaction("Create MI_Rock"):
    factory = unreal.MaterialInstanceConstantFactoryNew()
    factory.set_editor_property("initial_parent", parent)
    mi = at.create_asset("MI_Rock", "/Game/Materials/Instances", unreal.MaterialInstanceConstant, factory)
    mel.set_material_instance_vector_parameter_value(mi, "Tint", unreal.LinearColor(0.6, 0.55, 0.5, 1))
    mel.set_material_instance_scalar_parameter_value(mi, "RoughnessScale", 0.9)
    mel.set_material_instance_texture_parameter_value(mi, "BaseColorTex",
        unreal.load_asset("/Game/Textures/T_Rock_BC"))
    mel.set_material_instance_static_switch_parameter_value(mi, "UsePulse", False)
    mel.update_material_instance(mi)
    unreal.EditorAssetLibrary.save_asset(mi.get_path_name())

print(mel.get_scalar_parameter_names(parent), mel.get_vector_parameter_names(parent))
```
Setting a parameter name that does not exist returns `False` - check it.

## 5. Assign materials

```python
eas = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
mi = unreal.load_asset("/Game/Materials/Instances/MI_Rock")
with unreal.ScopedEditorTransaction("Assign MI_Rock"):
    for actor in eas.get_selected_level_actors():
        for comp in actor.get_components_by_class(unreal.StaticMeshComponent):
            comp.set_material(0, mi)          # per-actor override (saved in the level)
# Asset default instead (affects every use of the mesh):
mesh = unreal.load_asset("/Game/Meshes/SM_Rock")
mesh.set_material(0, mi)
unreal.EditorAssetLibrary.save_asset(mesh.get_path_name())
unreal.get_editor_subsystem(unreal.LevelEditorSubsystem).save_current_level()
```

## 6. Material function

```python
at = unreal.AssetToolsHelpers.get_asset_tools()
mel = unreal.MaterialEditingLibrary
with unreal.ScopedEditorTransaction("Create MF_Desaturate"):
    fn = at.create_asset("MF_Desaturate", "/Game/Materials/Functions",
                         unreal.MaterialFunction, unreal.MaterialFunctionFactoryNew())
    fn.set_editor_property("expose_to_library", True)
    inp = mel.create_material_expression_in_function(fn, unreal.MaterialExpressionFunctionInput, -600, 0)
    inp.set_editor_property("input_name", "Color")
    amt = mel.create_material_expression_in_function(fn, unreal.MaterialExpressionFunctionInput, -600, 200)
    amt.set_editor_property("input_name", "Amount")
    amt.set_editor_property("input_type", unreal.FunctionInputType.FUNCTION_INPUT_SCALAR)
    desat = mel.create_material_expression_in_function(fn, unreal.MaterialExpressionDesaturation, -300, 0)
    mel.connect_material_expressions(inp, "", desat, "")
    mel.connect_material_expressions(amt, "", desat, "Fraction")
    out = mel.create_material_expression_in_function(fn, unreal.MaterialExpressionFunctionOutput, 0, 0)
    out.set_editor_property("output_name", "Result")
    mel.connect_material_expressions(desat, "", out, "")
    mel.update_material_function(fn, None)
    unreal.EditorAssetLibrary.save_asset(fn.get_path_name())
```
Using the function in a material: create `MaterialExpressionMaterialFunctionCall`. Setting
its `material_function` property from Python may not rebuild the node's pins in every engine
version; after setting it, verify with a connect call's return value. If pins are missing,
ask the human to place the function node in the editor (drag from the palette) instead.

## 7. Texture settings and import

```python
import unreal
def configure_texture(path, kind):
    tex = unreal.load_asset(path)
    if kind == "normal":
        tex.set_editor_property("compression_settings", unreal.TextureCompressionSettings.TC_NORMALMAP)
        tex.set_editor_property("srgb", False)
    elif kind == "masks":
        tex.set_editor_property("compression_settings", unreal.TextureCompressionSettings.TC_MASKS)
        tex.set_editor_property("srgb", False)
    elif kind == "ui":
        tex.set_editor_property("compression_settings", unreal.TextureCompressionSettings.TC_EDITOR_ICON)
        tex.set_editor_property("lod_group", unreal.TextureGroup.TEXTUREGROUP_UI)
        tex.set_editor_property("mip_gen_settings", unreal.TextureMipGenSettings.TMGS_NO_MIPMAPS)
    unreal.EditorAssetLibrary.save_asset(path)

task = unreal.AssetImportTask()
task.set_editor_property("filename", "C:/Art/T_Rock_N.png")
task.set_editor_property("destination_path", "/Game/Textures")
task.set_editor_property("automated", True)
task.set_editor_property("replace_existing", True)
task.set_editor_property("save", True)
unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
print(task.get_editor_property("imported_object_paths"))
configure_texture("/Game/Textures/T_Rock_N", "normal")
```
`TC_EDITOR_ICON` is the Python name of the "UserInterface2D (RGBA)" setting. Audit a folder:
```python
for p in unreal.EditorAssetLibrary.list_assets("/Game/Textures", recursive=True):
    a = unreal.load_asset(p)
    if isinstance(a, unreal.Texture2D):
        print(p, a.get_editor_property("compression_settings"), a.get_editor_property("srgb"))
```
Textures named `*_N` with sRGB on, or `*_ORM`/`*_M` with compression Default, are bugs.

## 8. Inspect an existing material

```python
mat = unreal.load_asset("/Game/Materials/M_Master_Surface")
mel = unreal.MaterialEditingLibrary
print("blend", mat.get_editor_property("blend_mode"), "shading", mat.get_editor_property("shading_model"))
print("scalars", mel.get_scalar_parameter_names(mat))
print("textures", [t.get_path_name() for t in mel.get_used_textures(mat)])
if hasattr(mel, "get_statistics"):
    s = mel.get_statistics(mat)
    print(s)  # instruction counts / samplers where available
```
The graph itself is not fully exposed to Python in every version (listing expressions and
their links is limited). For complex graph edits, describe the change to the human with the
exact node names and pins, or rebuild the material from scratch with a script.

## Pitfalls

- Forgetting `recompile_material` - the asset keeps the old shader and the graph looks unconnected in-game.
- Normal-type sampler with a non-normal default texture, or color sampler on a normal map:
  compile error "Sampler type is Normal, should be Color" (or vice versa).
- Creating the same asset name twice: `create_asset` returns `None`. Check with
  `unreal.EditorAssetLibrary.does_asset_exist(path)` first.
- Editing a master material used by hundreds of instances triggers long shader compiles;
  warn the human.
