# Import an FBX exported from Blender, then measure what arrived: size, root bone scale, which way
# the character faces, and whether its materials are real assets. A mesh that arrived empty is
# deleted again, and the import retried with the other importer, so no broken asset stays behind.
#
# Defaults come from real failures on UE 5.8: the Interchange FBX importer produced empty static
# meshes ("Bad MeshDescription") and transient material instances, so the legacy importer is used
# unless importer="interchange"; "Import Normals and Tangents" on a Blender FBX gave a mesh that
# drew only its shadow, so normals are imported and tangents computed; and a re-import kept the
# old asset's import settings unless told to replace them.

kind = ARGS["kind"]
CVAR = "Interchange.FeatureFlags.Import.FBX"


def cvar_int(name):
    try:
        return int(unreal.SystemLibrary.get_console_variable_int_value(name))
    except Exception:
        return None


def set_cvar(name, value):
    unreal.SystemLibrary.execute_console_command(editor_world(), "%s %s" % (name, value))


def options():
    ui = unreal.FbxImportUI()
    ui.set_editor_property("import_mesh", kind != "animation")
    ui.set_editor_property("import_as_skeletal", kind in ("skeletal", "animation"))
    ui.set_editor_property("import_animations", kind == "animation" or bool(ARGS.get("animations")))
    ui.set_editor_property("import_materials", bool(ARGS.get("materials", kind != "animation")))
    ui.set_editor_property("import_textures", bool(ARGS.get("materials", kind != "animation")))
    ui.set_editor_property("mesh_type_to_import", {
        "static": unreal.FBXImportType.FBXIT_STATIC_MESH,
        "skeletal": unreal.FBXImportType.FBXIT_SKELETAL_MESH,
        "animation": unreal.FBXImportType.FBXIT_ANIMATION}[kind])
    if ARGS.get("skeleton"):
        ui.set_editor_property("skeleton", load(ARGS["skeleton"], "skeleton"))
    method = getattr(unreal.FBXNormalImportMethod, ARGS.get("normals", "FBXNIM_IMPORT_NORMALS"))
    for data_name in ("static_mesh_import_data", "skeletal_mesh_import_data"):
        try:
            ui.get_editor_property(data_name).set_editor_property("normal_import_method", method)
        except Exception:
            pass
    return ui


def run_import(destination, name):
    task = unreal.AssetImportTask()
    task.set_editor_property("filename", ARGS["fbx"])
    task.set_editor_property("destination_path", destination)
    if name:
        task.set_editor_property("destination_name", name)
    task.set_editor_property("automated", True)
    task.set_editor_property("save", True)
    task.set_editor_property("replace_existing", True)
    try:
        # Without this a re-import keeps the settings stored on the existing asset.
        task.set_editor_property("replace_existing_settings", True)
    except Exception:
        pass
    task.set_editor_property("options", options())
    unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
    return [str(p) for p in task.get_editor_property("imported_object_paths")]


def measure(path):
    a = unreal.load_asset(path)
    if a is None:
        return {"path": path, "error": "not loadable"}
    entry = {"path": path, "class": a.get_class().get_name()}
    if isinstance(a, (unreal.StaticMesh, unreal.SkeletalMesh)):
        e = vec(a.get_bounds().box_extent)
        entry["size_cm"] = rnd(mul(e, 2.0))
        entry["empty"] = max(e) < 0.01
        # Materials must be saved assets; a transient instance makes the mesh unsaveable.
        mats = []
        try:
            slots = a.get_editor_property("static_materials") if isinstance(a, unreal.StaticMesh) else a.get_editor_property("materials")
            for slot in slots:
                m = slot.get_editor_property("material_interface")
                mats.append(m.get_path_name() if m else None)
        except Exception:
            pass
        entry["materials"] = mats
        entry["transient_materials"] = [m for m in mats if m and ("/Transient" in m or not m.startswith(("/Game", "/Engine")))]
    if isinstance(a, unreal.SkeletalMesh):
        skel = Skeleton(a)
        root = skel.names[0]
        entry["root_bone"] = root
        entry["root_bone_scale"] = rnd(skel.ref_local[root][2], 3)
        f = body_frame(skel)
        entry["forward_axis_in_mesh_space"] = rnd(f["forward"], 3)
        entry["right_axis_in_mesh_space"] = rnd(f["right"], 3)
        entry["left_right_pairs"] = f["found_pairs"]
        pose = skel.component_pose(skel.ref_local)
        lefts = [n for n in skel.names if "hand" in n.lower() and mirror_name(n) in skel.index]
        hands = [lefts[0], mirror_name(lefts[0])] if lefts else []
        entry["hand_sides"] = dict((n, side(f, pose[n][0])) for n in hands)
    if isinstance(a, unreal.AnimSequence):
        entry["length_s"] = anim_length(a)
    return entry


def remove(paths):
    removed = []
    for p in paths:
        package = p.split(".")[0]
        ok = False
        try:
            ok = unreal.EditorAssetLibrary.delete_asset(package)
        except Exception:
            pass
        removed.append({"path": package, "deleted": bool(ok)})
    return removed


def fix_sockets(paths):
    """Sockets that came from empties arrive with the FBX axis conversion's roll (-90 degrees)
    on them, which turns whatever is attached. Put back what the empties meant: no rotation for
    an empty with none of its own (or for every socket with socket_rotation="zero"). They also
    arrive at 100x scale from the unit conversion, which scales whatever is attached; that is
    divided back for every socket unless socket_rotation="keep"."""
    mode = ARGS.get("socket_rotation", "match")
    if mode == "keep" or kind != "static":
        return []
    meant = dict((s["name"].lower(), s) for s in ARGS.get("sockets") or [])
    fixed = []
    for path in paths:
        mesh = unreal.load_asset(path)
        if not isinstance(mesh, unreal.StaticMesh):
            continue
        changed = False
        for socket in mesh.get_editor_property("sockets"):
            name = str(socket.get_editor_property("socket_name"))
            scale = undo_blender_socket_scale(socket)
            if scale:
                fixed.append({"mesh": path, "socket": name, "scale": scale})
                changed = True
            key = name.lower()
            key = key[len("socket_"):] if key.startswith("socket_") else key
            spec = meant.get(key)
            if mode == "zero" or (spec is not None and spec.get("identity")):
                before = socket.get_editor_property("relative_rotation")
                if abs(before.roll) + abs(before.pitch) + abs(before.yaw) > 0.01:
                    socket.set_editor_property("relative_rotation", unreal.Rotator(roll=0.0, pitch=0.0, yaw=0.0))
                    fixed.append({"mesh": path, "socket": name, "was": [round(before.pitch, 2), round(before.yaw, 2), round(before.roll, 2)]})
                    changed = True
            elif spec is not None:
                r = socket.get_editor_property("relative_rotation")
                fixed.append({"mesh": path, "socket": name, "left_as_imported": [round(r.pitch, 2), round(r.yaw, 2), round(r.roll, 2)],
                              "blender_rotation_deg": spec.get("rotation_deg"), "note": "rotated in Blender; check it with ue_screenshot"})
        if changed:
            save_asset(mesh)
    return fixed


importer = ARGS.get("importer", "legacy")
previous = cvar_int(CVAR)
attempts = []
try:
    order = [importer] + ([x for x in ("legacy", "interchange") if x != importer] if ARGS.get("retry", True) else [])
    for attempt in order:
        if previous is not None:
            set_cvar(CVAR, 1 if attempt == "interchange" else 0)
        paths = run_import(ARGS["destination"], ARGS.get("name"))
        measured = [measure(p) for p in paths]
        broken = [m["path"] for m in measured if m.get("empty") or m.get("error") or m.get("transient_materials")]
        attempts.append({"importer": attempt, "imported": measured, "broken": broken})
        if paths and not broken:
            break
        # Leave nothing broken behind before trying again (or giving up).
        attempts[-1]["cleaned_up"] = remove(broken or paths)
finally:
    if previous is not None:
        set_cvar(CVAR, previous)

final = attempts[-1]
sockets_fixed = fix_sockets([m["path"] for m in final["imported"]]) if not final.get("cleaned_up") else []
emit({"imported": final["imported"] if not final.get("cleaned_up") else [], "importer": final["importer"],
      "sockets": sockets_fixed,
      "attempts": attempts, "interchange_fbx_cvar": previous,
      "failed": bool(final.get("cleaned_up")) or not final["imported"]})
