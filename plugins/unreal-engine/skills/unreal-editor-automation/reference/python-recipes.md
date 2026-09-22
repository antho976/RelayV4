# Python editor recipes

Snippets for `ue_python` (UE 5.3-5.6). Level/actor recipes (spawning, arranging,
selected actors, loading levels, viewport) are in `python-level-recipes.md`. Each snippet
is self-contained: paste it as `code`, edit the constants at the top. Run a read-only version first (comment out the mutation) when
the operation touches many assets. If a call raises `AttributeError`, check the name with
`help(unreal.<Class>)` or the stub file (`Intermediate/PythonStub/unreal.py`) instead of guessing.

Shared helpers used below:
```python
import unreal
EAS = unreal.EditorAssetLibrary
AT = unreal.AssetToolsHelpers.get_asset_tools()
AR = unreal.AssetRegistryHelpers.get_asset_registry()
ACTORS = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
LEVELS = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
EDITOR = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem)
```

## Querying assets

```python
# All static meshes under a folder, via the Asset Registry (no loading, fast)
flt = unreal.ARFilter(
    class_paths=[unreal.TopLevelAssetPath("/Script/Engine", "StaticMesh")],
    package_paths=["/Game/Props"], recursive_paths=True)
for ad in AR.get_assets(flt):
    print(ad.package_name, ad.asset_name, ad.asset_class_path.asset_name)

# Everything in a folder (object paths like /Game/Props/SM_Chair.SM_Chair)
for p in EAS.list_assets("/Game/Props", recursive=True, include_folder=False):
    print(p)

# Who references this asset? (package names)
print(EAS.find_package_referencers_for_asset("/Game/Props/SM_Chair", False))
opts = unreal.AssetRegistryDependencyOptions()
print(AR.get_referencers("/Game/Props/SM_Chair", opts))
print(AR.get_dependencies("/Game/Props/SM_Chair", opts))

# Content Browser selection
for a in unreal.EditorUtilityLibrary.get_selected_assets():
    print(a.get_path_name(), a.get_class().get_name())
```
`AssetData` fields: `package_name`, `package_path`, `asset_name`, `asset_class_path`
(5.1+). Build the object path as `f"{ad.package_name}.{ad.asset_name}"`; `ad.get_asset()` loads it.

## Bulk rename / move with redirector fixup

Renames through AssetTools update in-memory referencers and leave `ObjectRedirector`s for
referencers that were not loaded. Fix them up afterwards so the old names disappear.

```python
import unreal
AT = unreal.AssetToolsHelpers.get_asset_tools()
EAS = unreal.EditorAssetLibrary
SRC, DST = "/Game/Imported", "/Game/Props/Furniture"
PREFIX = "SM_"
DRY_RUN = True

EAS.make_directory(DST)
data = []
for path in EAS.list_assets(SRC, recursive=False, include_folder=False):
    asset = EAS.load_asset(path)
    if not isinstance(asset, unreal.StaticMesh):
        continue
    name = asset.get_name()
    new_name = name if name.startswith(PREFIX) else PREFIX + name
    data.append(unreal.AssetRenameData(asset=asset, new_package_path=DST, new_name=new_name))
    print(f"{path} -> {DST}/{new_name}")

if not DRY_RUN and data:
    ok = AT.rename_assets(data)
    print("rename ok:", ok)
```

Find and fix redirectors (run after the rename, in a second call):
```python
import unreal
AR = unreal.AssetRegistryHelpers.get_asset_registry()
AT = unreal.AssetToolsHelpers.get_asset_tools()
flt = unreal.ARFilter(
    class_paths=[unreal.TopLevelAssetPath("/Script/CoreUObject", "ObjectRedirector")],
    package_paths=["/Game"], recursive_paths=True)
redirectors = []
for ad in AR.get_assets(flt):
    unreal.load_package(str(ad.package_name))
    r = unreal.find_object(None, f"{ad.package_name}.{ad.asset_name}")  # find, do not follow
    if isinstance(r, unreal.ObjectRedirector):
        redirectors.append(r)
print("redirectors:", [r.get_path_name() for r in redirectors])
if redirectors and hasattr(AT, "fixup_referencers"):
    AT.fixup_referencers(redirectors)   # resaves referencers, deletes fixed redirectors
```
If `fixup_referencers` is not exposed in this engine version, ask the human: Content Browser >
right-click the folder > **Fix Up Redirectors**. Offline alternative (editor closed):
`UnrealEditor-Cmd <Project>.uproject -run=ResavePackages -fixupredirects -projectonly -unattended`.
Commit the renamed assets, every resaved referencer and the deleted redirectors together.

Single asset: `EAS.rename_asset("/Game/A/SM_Old", "/Game/B/SM_New")` (paths without the
`.Name` suffix). Duplicate: `EAS.duplicate_asset("/Game/A/M_Base", "/Game/A/M_Base_Copy")`.

## Material instance

```python
import unreal
AT = unreal.AssetToolsHelpers.get_asset_tools()
MEL = unreal.MaterialEditingLibrary
EAS = unreal.EditorAssetLibrary

parent = EAS.load_asset("/Game/Materials/M_Master")
mi_path, mi_name = "/Game/Materials/Instances", "MI_Rock_Wet"
full = f"{mi_path}/{mi_name}"
mi = EAS.load_asset(full) if EAS.does_asset_exist(full) else AT.create_asset(
    mi_name, mi_path, unreal.MaterialInstanceConstant, unreal.MaterialInstanceConstantFactoryNew())
with unreal.ScopedEditorTransaction("Relay: configure MI_Rock_Wet"):
    MEL.set_material_instance_parent(mi, parent)
    MEL.set_material_instance_scalar_parameter_value(mi, "Roughness", 0.15)
    MEL.set_material_instance_vector_parameter_value(mi, "Tint", unreal.LinearColor(0.4, 0.4, 0.45, 1.0))
    tex = EAS.load_asset("/Game/Textures/T_Rock_BC")
    MEL.set_material_instance_texture_parameter_value(mi, "BaseColor", tex)
    MEL.update_material_instance(mi)
EAS.save_asset(full, only_if_is_dirty=False)
print(MEL.get_scalar_parameter_names(parent))   # list what the parent actually exposes
```
Parameter names must match the parent exactly; a wrong name silently does nothing, so list
them first (`get_scalar_parameter_names`, `get_vector_parameter_names`, `get_texture_parameter_names`).

## New material with a small graph

```python
import unreal
AT = unreal.AssetToolsHelpers.get_asset_tools()
MEL = unreal.MaterialEditingLibrary
mat = AT.create_asset("M_SimpleColor", "/Game/Materials", unreal.Material, unreal.MaterialFactoryNew())
color = MEL.create_material_expression(mat, unreal.MaterialExpressionVectorParameter, -400, 0)
color.set_editor_property("parameter_name", "Color")
rough = MEL.create_material_expression(mat, unreal.MaterialExpressionScalarParameter, -400, 200)
rough.set_editor_property("parameter_name", "Roughness")
rough.set_editor_property("default_value", 0.5)
MEL.connect_material_property(color, "", unreal.MaterialProperty.MP_BASE_COLOR)
MEL.connect_material_property(rough, "", unreal.MaterialProperty.MP_ROUGHNESS)
MEL.recompile_material(mat)
unreal.EditorAssetLibrary.save_loaded_asset(mat)
```
For a vector parameter, output `""` is the default (RGBA) pin; use `"RGB"` etc. when needed.
See `unreal-materials-vfx` for material design.

## Import textures

```python
import unreal, os
SRC_DIR, DEST = r"C:/Art/Export/Rock", "/Game/Textures/Rock"
tasks = []
for f in sorted(os.listdir(SRC_DIR)):
    if f.lower().endswith((".png", ".tga", ".jpg", ".exr")):
        t = unreal.AssetImportTask()
        t.set_editor_property("filename", os.path.join(SRC_DIR, f))
        t.set_editor_property("destination_path", DEST)
        t.set_editor_property("automated", True)          # no dialogs
        t.set_editor_property("replace_existing", True)
        t.set_editor_property("save", True)
        tasks.append(t)
unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks(tasks)
for t in tasks:
    print(t.get_editor_property("imported_object_paths"))
```
Fix up normal maps and masks after import:
```python
tex = unreal.EditorAssetLibrary.load_asset("/Game/Textures/Rock/T_Rock_N")
tex.set_editor_property("compression_settings", unreal.TextureCompressionSettings.TC_NORMALMAP)
tex.set_editor_property("srgb", False)
tex.set_editor_property("lod_group", unreal.TextureGroup.TEXTUREGROUP_WORLD_NORMAL_MAP)
mask = unreal.EditorAssetLibrary.load_asset("/Game/Textures/Rock/T_Rock_ORM")
mask.set_editor_property("compression_settings", unreal.TextureCompressionSettings.TC_MASKS)
mask.set_editor_property("srgb", False)
unreal.EditorAssetLibrary.save_loaded_assets([tex, mask])
```
The file path must be readable by the editor process (absolute path on the editor's machine).

## Import FBX (static, skeletal, animation)

```python
import unreal
def fbx_task(path, dest, options):
    t = unreal.AssetImportTask()
    t.set_editor_property("filename", path)
    t.set_editor_property("destination_path", dest)
    t.set_editor_property("automated", True)
    t.set_editor_property("replace_existing", True)
    t.set_editor_property("save", True)
    t.set_editor_property("options", options)
    return t

# Static mesh
o = unreal.FbxImportUI()
o.set_editor_property("import_mesh", True)
o.set_editor_property("import_as_skeletal", False)
o.set_editor_property("import_materials", False)
o.set_editor_property("import_textures", False)
o.set_editor_property("mesh_type_to_import", unreal.FBXImportType.FBXIT_STATIC_MESH)
o.static_mesh_import_data.set_editor_property("combine_meshes", True)
o.static_mesh_import_data.set_editor_property("generate_lightmap_u_vs", True)  # sic: bGenerateLightmapUVs
static_task = fbx_task(r"C:/Art/SM_Crate.fbx", "/Game/Props/Crate", o)

# Animation onto an existing skeleton
a = unreal.FbxImportUI()
a.set_editor_property("import_mesh", False)
a.set_editor_property("import_animations", True)
a.set_editor_property("mesh_type_to_import", unreal.FBXImportType.FBXIT_ANIMATION)
a.set_editor_property("skeleton", unreal.load_asset("/Game/Characters/Hero/SK_Hero_Skeleton"))
anim_task = fbx_task(r"C:/Art/A_Hero_Run.fbx", "/Game/Characters/Hero/Anims", a)

unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([static_task, anim_task])
for t in (static_task, anim_task):
    print(t.get_editor_property("imported_object_paths"))
```
For a skeletal mesh set `import_as_skeletal=True`, `mesh_type_to_import=FBXIT_SKELETAL_MESH`
and optionally `skeleton` to share an existing skeleton. In 5.5+ FBX import runs through the
Interchange framework by default; legacy `FbxImportUI` options may be partly ignored. Always
inspect the result (mesh count, materials, scale) rather than assuming options applied.

Enable Nanite on imported meshes:
```python
m = unreal.EditorAssetLibrary.load_asset("/Game/Props/Crate/SM_Crate")
ns = m.get_editor_property("nanite_settings")
ns.set_editor_property("enabled", True)
m.set_editor_property("nanite_settings", ns)   # structs are copies: write back
unreal.EditorAssetLibrary.save_loaded_asset(m)
```

## Data Table from CSV

Row struct in C++ (build it first, see `unreal-cpp`):
```cpp
USTRUCT(BlueprintType)
struct FItemRow : public FTableRowBase
{
    GENERATED_BODY()
    UPROPERTY(EditAnywhere, BlueprintReadOnly) FText DisplayName;
    UPROPERTY(EditAnywhere, BlueprintReadOnly) int32 Price = 0;
    UPROPERTY(EditAnywhere, BlueprintReadOnly) float Weight = 0.f;
};
```
CSV (first column is the row name; headers match property names):
```
Name,DisplayName,Price,Weight
Sword,"Iron Sword",120,3.5
Potion,"Health Potion",25,0.2
```
Python:
```python
import unreal
AT = unreal.AssetToolsHelpers.get_asset_tools()
row_struct = unreal.load_object(None, "/Script/MyGame.ItemRow")   # native: /Script/<Module>.<Name without F>
# Blueprint struct instead: unreal.load_asset("/Game/Data/S_Item")
f = unreal.DataTableFactory()
f.set_editor_property("struct", row_struct)
dt = AT.create_asset("DT_Items", "/Game/Data", unreal.DataTable, f)
ok = unreal.DataTableFunctionLibrary.fill_data_table_from_csv_file(dt, r"C:/Proj/Data/Items.csv")
print("filled:", ok, unreal.DataTableFunctionLibrary.get_data_table_row_names(dt))
unreal.EditorAssetLibrary.save_loaded_asset(dt)
```
Keep the CSV in the repo (text, diffable) and re-run the fill to update; the `.uasset`
is then a build product of the CSV. Alternative import path: `AssetImportTask` with
`unreal.CSVImportFactory()` whose `automated_import_settings.import_row_struct` is set.

## Blueprint class from a C++ parent

```python
import unreal
AT = unreal.AssetToolsHelpers.get_asset_tools()
EAS = unreal.EditorAssetLibrary
parent = unreal.load_class(None, "/Script/MyGame.HeroCharacter")
f = unreal.BlueprintFactory()
f.set_editor_property("parent_class", parent)
bp = AT.create_asset("BP_Hero", "/Game/Characters/Hero", unreal.Blueprint, f)

# Defaults of properties declared in C++ live on the generated class's CDO
gen = EAS.load_blueprint_class("/Game/Characters/Hero/BP_Hero")
cdo = unreal.get_default_object(gen)
cdo.set_editor_property("max_health", 150.0)     # UPROPERTY(EditDefaultsOnly) float MaxHealth
if hasattr(unreal, "BlueprintEditorLibrary"):
    unreal.BlueprintEditorLibrary.compile_blueprint(bp)
EAS.save_asset("/Game/Characters/Hero/BP_Hero", only_if_is_dirty=False)
```
Adding components to a Blueprint (5.x, verify with `help(unreal.SubobjectDataSubsystem)`):
```python
sds = unreal.get_engine_subsystem(unreal.SubobjectDataSubsystem)
handles = sds.k2_gather_subobject_data_for_blueprint(bp)
params = unreal.AddNewSubobjectParams(parent_handle=handles[0],
                                      new_class=unreal.StaticMeshComponent,
                                      blueprint_context=bp)
new_handle, fail_reason = sds.add_new_subobject(params)
print("fail:", fail_reason)
```
Prefer declaring components in the C++ parent (`CreateDefaultSubobject`) so the structure is
diffable; the Blueprint only sets asset references and tuning values.

## Deleting safely

```python
EAS = unreal.EditorAssetLibrary
p = "/Game/Old/SM_Unused"
refs = EAS.find_package_referencers_for_asset(p, False)
if refs:
    print("NOT deleting, referenced by:", refs)
else:
    print("deleted:", EAS.delete_asset(p))
```
Deletion is not undoable through transactions. Confirm with the human first.

## Saving summary

| Goal | Call |
|---|---|
| One asset by path | `EAS.save_asset(path, only_if_is_dirty=False)` |
| Loaded asset objects | `EAS.save_loaded_asset(obj)` / `EAS.save_loaded_assets([...])` |
| A folder | `EAS.save_directory("/Game/X", only_if_is_dirty=True, recursive=True)` |
| Current / all dirty levels | `LEVELS.save_current_level()` / `LEVELS.save_all_dirty_levels()` |
| Everything dirty | `unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)` |
