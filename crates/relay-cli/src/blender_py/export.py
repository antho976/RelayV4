# blender_export: FBX for Unreal. Static meshes bring their UCX_/UBX_/USP_ collision and SOCKET_
# empties (children of the mesh); skeletal exports carry only deform bones and no leaf bones.
import os

path = ARGS["path"]
names = ARGS.get("objects") or []
# Objects in a collection excluded from the view layer cannot be selected, so the exporter
# never sees them. Named objects are brought in below; found ones are reported, not forced.
in_layer = set(bpy.context.view_layer.objects)
parents = {}
for c in [scene.collection] + list(scene.collection.children_recursive):
    for child in c.children:
        parents.setdefault(child, []).append(c)


def render_hidden(o):
    """Whether the object stays out of a render: its own flag, or every collection it is in
    is turned off for render somewhere up the tree (Rigify does this to its widgets)."""
    def off(c, seen=()):
        if c.hide_render:
            return True
        ups = [p for p in parents.get(c, []) if p not in seen]
        return bool(ups) and all(off(p, seen + (c,)) for p in ups)
    return o.hide_render or bool(o.users_collection) and all(off(c) for c in o.users_collection)


excluded = []
if names:
    chosen = [obj(n) for n in names]
else:
    roots = [o for o in scene.objects if o.type in ("MESH", "ARMATURE") and not render_hidden(o) and o.parent is None]
    chosen = [o for o in roots if o in in_layer]
    excluded += [o.name for o in roots if o not in in_layer]
if not chosen:
    raise RuntimeError("nothing to export" + (" (objects in collections excluded from the view layer: %s)" % ", ".join(excluded) if excluded else ""))
selected = set()


def take(o):
    if o in in_layer:
        selected.add(o)
    elif o.name not in excluded:
        excluded.append(o.name)


for o in chosen:
    selected.add(o)
    # A static mesh brings its SOCKET_ empties, UCX_ collision and _LODn children. An armature
    # brings the meshes skinned to it, not props parented to its bones: those are items with
    # their own export and attach to a socket in Unreal.
    if o.type != "ARMATURE" and ARGS.get("children", True):
        for c in o.children_recursive:
            take(c)
    if o.type == "ARMATURE":
        for m in skinned_meshes(o):
            take(m)

kind = ARGS.get("kind", "auto")
if kind == "auto":
    kind = "skeletal" if any(o.type == "ARMATURE" for o in selected) else "static"
arm = next((o for o in selected if o.type == "ARMATURE"), None)
if kind in ("skeletal", "animation") and arm is None:
    raise RuntimeError("%s export needs an armature among the objects" % kind)
if arm is not None and ARGS.get("action"):
    # Without all_actions the exporter bakes the scene timeline, so match it to the action.
    act = set_action(arm, ARGS["action"])
    scene.frame_start, scene.frame_end = int(act.frame_range[0]), int(act.frame_range[1])
if kind == "animation":
    selected = set([arm])

# Hidden objects export as nothing, and four switches hide one: the object's eye and monitor,
# its collection's eye (per view layer) and monitor, and hide_select on either. Linking each
# object into the scene's own collection as well clears every collection-level one at once,
# and puts a named object from an excluded collection into the view layer (and the depsgraph
# the checks below read). The .blend is not saved, so the extra links go nowhere.
for o in selected:
    if o.name not in scene.collection.objects:
        scene.collection.objects.link(o)
bpy.context.view_layer.update()

# Check what will be written before writing it: a broken mesh exports "fine" and then imports
# empty, invisible or shaded wrong.
mesh_checks = [mesh_report(o) for o in selected if o.type == "MESH" and not o.name.startswith(("UCX_", "UBX_", "USP_"))]
export_problems = [dict(object=r["object"], problem=p) for r in mesh_checks for p in r["problems"]]
export_warnings = [dict(object=r["object"], problem=p) for r in mesh_checks for p in r["warnings"]]
if excluded:
    export_warnings.append(dict(object=", ".join(sorted(excluded)), problem="in a collection excluded from the view layer, so not exported: tick the collection in the outliner, or name the objects in objects"))
if arm is not None:
    if any(abs(c - 1.0) > 1e-3 for c in arm.scale):
        export_problems.append(dict(object=arm.name, problem="armature object scale is %s (the UE mannequin imports at 0.01): apply scale (with its meshes and actions) so the rig exports at 1" % rnd(arm.scale, 3)))
    act = arm.animation_data.action if arm.animation_data else None
    if act is not None and kind != "static":
        deform = set(b.name for b in arm.data.bones if b.use_deform)
        curves = action_fcurves(act, getattr(arm.animation_data, "action_slot", None))
        keyed = set(fc.data_path.split('"')[1] for fc in curves if fc.data_path.startswith("pose.bones["))
        control = sorted(keyed - deform)
        if control:
            export_warnings.append(dict(object=arm.name, problem="action %s keys non-deform bones %s (IK or controls): only the evaluated pose of deform bones is exported - check the result with blender_anim_inspect, or bake to deform bones (nla.bake with visual keying) first" % (act.name, ", ".join(control[:8]))))
if export_problems and not ARGS.get("allow_problems"):
    raise RuntimeError("not exported, the result would be broken in Unreal: %s. Fix them (or pass allow_problems=true)." % json.dumps(export_problems))

for o in bpy.context.view_layer.objects:
    o.select_set(False)
for o in selected:
    o.hide_set(False)
    o.hide_viewport = False
    o.hide_select = False
    o.select_set(True)
bpy.context.view_layer.objects.active = next(iter(selected))
unselectable = sorted(o.name for o in selected - set(bpy.context.selected_objects))
if unselectable:
    raise RuntimeError("not exported: %s could not be selected, so the FBX would leave them out" % ", ".join(unselectable))

options = dict(
    filepath=path, use_selection=True, check_existing=False,
    object_types={"ARMATURE", "MESH", "EMPTY"} if kind != "animation" else {"ARMATURE"},
    apply_unit_scale=True,
    # Skeletal exports bake the unit conversion into the objects so the rig arrives with a scale
    # of 1 instead of a 100x root; blender_to_unreal measures the result either way.
    apply_scale_options="FBX_SCALE_ALL" if kind != "static" else "FBX_SCALE_NONE",
    axis_forward="-Z", axis_up="Y",
    use_space_transform=True, bake_space_transform=False,
    mesh_smooth_type="FACE", use_tspace=True, use_mesh_modifiers=True,
    add_leaf_bones=False, use_armature_deform_only=True, primary_bone_axis="Y", secondary_bone_axis="X",
    bake_anim=kind != "static" and bool(ARGS.get("animations", kind == "animation")),
    bake_anim_use_all_actions=bool(ARGS.get("all_actions", False)),
    bake_anim_use_nla_strips=False, bake_anim_force_startend_keying=True, bake_anim_simplify_factor=0.0,
)
for key, value in (ARGS.get("fbx_options") or {}).items():
    options[key] = set(value) if key == "object_types" else value
os.makedirs(os.path.dirname(path), exist_ok=True)
result = bpy.ops.export_scene.fbx(**options)
if "FINISHED" not in result:
    raise RuntimeError("the FBX exporter returned %s" % result)
meshes = [o for o in selected if o.type == "MESH" and not o.name.startswith(("UCX_", "UBX_", "USP_"))]
# In bind pose: the FBX holds skinned meshes at rest, and that is what Unreal measures.
with rest_pose([o for o in selected if o.type == "ARMATURE"]):
    lo, hi = world_bbox(meshes) if meshes else (None, None)
# How each socket empty is turned relative to its mesh. Unreal receives sockets from empties
# with a -90 degree roll from the axis conversion, so the import puts back what was meant: an
# empty with no rotation of its own becomes a socket with no rotation.
socket_details = []
for o in selected:
    if o.type == "EMPTY" and o.name.startswith("SOCKET_"):
        rel = (o.parent.matrix_world.inverted() @ o.matrix_world) if o.parent else o.matrix_world
        e = rel.to_euler()
        socket_details.append({"name": o.name[len("SOCKET_"):], "empty": o.name,
                               "rotation_deg": rnd([math.degrees(a) for a in e], 2),
                               "identity": max(abs(a) for a in e) < 1e-3})
facing = None
if arm is not None and kind != "static":
    f = body_frame(arm)
    if f["pairs"]:
        facing = rnd(f["forward"], 3)
emit({"path": path, "bytes": os.path.getsize(path), "kind": kind, "forward_world": facing,
      "mesh_checks": mesh_checks, "problems": export_problems, "warnings": export_warnings,
      "objects": sorted(o.name for o in selected),
      "excluded": sorted(excluded),
      "sockets": sorted(o.name for o in selected if o.type == "EMPTY" and o.name.startswith("SOCKET_")),
      "socket_details": socket_details,
      "collision": sorted(o.name for o in selected if o.name.startswith(("UCX_", "UBX_", "USP_"))),
      "action": action_used(arm)["action"], "action_slot": action_used(arm)["slot"],
      "size_cm": cm(hi - lo) if lo is not None else None,
      "options": dict((k, sorted(v) if isinstance(v, set) else v) for k, v in options.items() if k != "filepath")})
