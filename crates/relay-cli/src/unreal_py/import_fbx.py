# Import an FBX exported from Blender, then measure what arrived: size, root bone scale, and which
# way the character faces, so axis and unit mistakes show up at the handoff.
kind = ARGS["kind"]
task = unreal.AssetImportTask()
task.set_editor_property("filename", ARGS["fbx"])
task.set_editor_property("destination_path", ARGS["destination"])
if ARGS.get("name"):
    task.set_editor_property("destination_name", ARGS["name"])
task.set_editor_property("automated", True)
task.set_editor_property("save", True)
task.set_editor_property("replace_existing", True)
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
task.set_editor_property("options", ui)
unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
paths = [str(p) for p in task.get_editor_property("imported_object_paths")]
if not paths:
    raise RuntimeError("nothing was imported; read the log (ue_log filter 'LogFbx|Interchange|Error')")

assets = []
for path in paths:
    a = unreal.load_asset(path)
    if a is None:
        continue
    entry = {"path": path, "class": a.get_class().get_name()}
    if isinstance(a, (unreal.StaticMesh, unreal.SkeletalMesh)):
        e = vec(a.get_bounds().box_extent)
        entry["size_cm"] = rnd(mul(e, 2.0))
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
    assets.append(entry)
emit({"imported": assets})
