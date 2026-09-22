# ue_asset_audit: facts about textures, meshes, references and redirectors, with the problems
# they show. Loads assets, so it is bounded by `limit` and shows progress in the editor.

registry = unreal.AssetRegistryHelpers.get_asset_registry()
root = ARGS.get("path", "/Game")
checks = set(ARGS.get("checks") or ["textures", "meshes", "references", "redirectors"])
limit = int(ARGS.get("limit", 400))
findings = []
counts = {}


def finding(kind, asset, problem, fix):
    if len(findings) < 400:
        findings.append({"kind": kind, "asset": asset, "problem": problem, "fix": fix})


def class_of(d):
    cls = getattr(d, "asset_class_path", None)
    return str(cls.asset_name) if cls is not None else str(d.asset_class)


assets = registry.get_assets_by_path(root, recursive=True)
by_class = {}
for d in assets:
    by_class.setdefault(class_of(d), []).append(d)
summary = dict((k, len(v)) for k, v in sorted(by_class.items()))


def pow2(n):
    return n > 0 and (n & (n - 1)) == 0


NORMAL_HINTS = ("_n", "_normal", "_nrm")
MASK_HINTS = ("_orm", "_arm", "_rma", "_mask", "_m", "_r", "_ao", "_rough", "_metal")

if "textures" in checks:
    textures = by_class.get("Texture2D", [])[:limit]
    counts["textures_checked"] = len(textures)
    with unreal.ScopedSlowTask(len(textures), "Auditing textures") as task:
        for d in textures:
            task.enter_progress_frame(1)
            path = str(d.package_name)
            t = unreal.load_asset(path)
            if t is None:
                finding("texture", path, "failed to load", "open it in the editor and read the log")
                continue
            w, h = t.blueprint_get_size_x(), t.blueprint_get_size_y()
            name = path.rsplit("/", 1)[-1].lower()
            comp = t.get_editor_property("compression_settings")
            srgb = t.get_editor_property("srgb")
            never_stream = t.get_editor_property("never_stream")
            mips = t.get_editor_property("mip_gen_settings")
            if not (pow2(w) and pow2(h)):
                finding("texture", path, "%dx%d is not a power of two: no mips, no streaming" % (w, h), "resize to a power of two at the source")
            if max(w, h) > 4096:
                finding("texture", path, "%dx%d is larger than 4096" % (w, h), "check it needs that resolution; set a Max Texture Size or LOD bias")
            if name.endswith(NORMAL_HINTS) and comp != unreal.TextureCompressionSettings.TC_NORMALMAP:
                finding("texture", path, "named like a normal map but compression is %s" % comp, "set Compression Settings to Normalmap (and sRGB off)")
            if name.endswith(MASK_HINTS) and srgb:
                finding("texture", path, "named like a mask/packed texture but sRGB is on", "turn sRGB off for linear data")
            if never_stream and max(w, h) >= 1024:
                finding("texture", path, "Never Stream is on for a %dx%d texture" % (w, h), "turn it off unless this is UI or must stay resident")
            if str(mips).endswith("NO_MIPMAPS") and "ui" not in path.lower():
                finding("texture", path, "no mipmaps on what looks like a world texture", "use FromTextureGroup mips (keep NoMipmaps for UI)")

if "meshes" in checks:
    meshes = by_class.get("StaticMesh", [])[:limit]
    counts["meshes_checked"] = len(meshes)
    sm = None
    try:
        sm = unreal.get_editor_subsystem(unreal.StaticMeshEditorSubsystem)
    except Exception:
        sm = unreal.EditorStaticMeshLibrary
    with unreal.ScopedSlowTask(len(meshes), "Auditing static meshes") as task:
        for d in meshes:
            task.enter_progress_frame(1)
            path = str(d.package_name)
            m = unreal.load_asset(path)
            if m is None:
                continue
            try:
                verts = sm.get_number_verts(m, 0)
                lods = sm.get_lod_count(m)
                simple = sm.get_simple_collision_count(m)
            except Exception as error:
                finding("mesh", path, "could not read mesh stats: %s" % error, "")
                continue
            nanite = False
            try:
                nanite = bool(m.get_editor_property("nanite_settings").get_editor_property("enabled"))
            except Exception:
                pass
            complex_as_simple = False
            try:
                body = m.get_editor_property("body_setup")
                complex_as_simple = "USE_COMPLEX_AS_SIMPLE" in str(body.get_editor_property("collision_trace_flag")).upper()
            except Exception:
                pass
            if simple == 0 and not complex_as_simple:
                finding("mesh", path, "no simple collision", "add simple collision (or UCX_ meshes at import) unless it is decoration nobody touches")
            if verts > 50000 and not nanite and lods <= 1:
                finding("mesh", path, "%d vertices, one LOD, Nanite off" % verts, "enable Nanite (opaque, non-deforming) or generate LODs")

if "redirectors" in checks:
    redirectors = [str(d.package_name) for d in by_class.get("ObjectRedirector", [])]
    counts["redirectors"] = len(redirectors)
    if redirectors:
        finding("redirector", root, "%d redirectors under %s" % (len(redirectors), root),
                "right-click the folder > Fix Up Redirectors, then commit the removed files together with the fixed referencers")

if "references" in checks:
    options = unreal.AssetRegistryDependencyOptions(include_soft_package_references=True, include_hard_package_references=True,
                                                   include_searchable_names=False, include_soft_management_references=False,
                                                   include_hard_management_references=False)
    checked = 0
    missing = 0
    for d in assets[: limit * 2]:
        package = d.package_name
        for dep in registry.get_dependencies(package, options) or []:
            dep = str(dep)
            if not dep.startswith("/Game"):
                continue
            checked += 1
            if not unreal.EditorAssetLibrary.does_asset_exist(dep):
                missing += 1
                finding("reference", str(package), "references missing asset %s" % dep, "restore the asset, or fix the reference in the editor and resave")
    counts["references_checked"] = checked
    counts["missing_references"] = missing

emit({"path": root, "assets_by_class": summary, "counts": counts, "findings": findings,
      "passed": not findings, "limit": limit})
