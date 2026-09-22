# blender_info: what a .blend holds, in the terms a game pipeline cares about.
import bmesh


def transform_flags(o):
    flags = []
    if any(abs(s - 1.0) > 1e-4 for s in o.scale):
        flags.append("scale not applied (%s)" % rnd(o.scale, 3))
    if o.type in ("MESH", "ARMATURE") and any(abs(r) > 1e-4 for r in o.rotation_euler):
        flags.append("rotation not applied (%s deg)" % rnd([math.degrees(r) for r in o.rotation_euler]))
    return flags


objects = []
for o in scene.objects:
    entry = {"name": o.name, "type": o.type, "parent": o.parent.name if o.parent else None,
             "parent_bone": o.parent_bone or None, "location_cm": cm(o.matrix_world.translation),
             "dimensions_cm": cm(o.dimensions), "collections": [c.name for c in o.users_collection],
             "hidden": o.hide_get() or o.hide_render, "flags": transform_flags(o)}
    if o.modifiers:
        entry["modifiers"] = ["%s (%s)" % (m.name, m.type) for m in o.modifiers]
    if o.type == "MESH":
        me = o.data
        tris = sum(len(p.vertices) - 2 for p in me.polygons)
        entry["mesh"] = {"vertices": len(me.vertices), "faces": len(me.polygons), "triangles": tris,
                         "uv_maps": [uv.name for uv in me.uv_layers], "vertex_groups": len(o.vertex_groups),
                         "materials": [m.name if m else None for m in me.materials],
                         "shape_keys": [k.name for k in me.shape_keys.key_blocks] if me.shape_keys else []}
        if o.name.startswith("UCX_") or o.name.startswith("UBX_") or o.name.startswith("USP_"):
            entry["unreal_role"] = "collision"
    if o.type == "EMPTY" and o.name.startswith("SOCKET_"):
        entry["unreal_role"] = "socket"
    if o.type == "ARMATURE":
        bones = o.data.bones
        entry["armature"] = {"bones": len(bones), "deform_bones": sum(1 for b in bones if b.use_deform),
                             "roots": [b.name for b in bones if b.parent is None],
                             "left_right_pairs": len(left_right_pairs([b.name for b in bones])),
                             "pose_position": o.data.pose_position,
                             "action": o.animation_data.action.name if o.animation_data and o.animation_data.action else None,
                             "sample_bones": [b.name for b in bones][:40]}
    objects.append(entry)

actions = []
for a in bpy.data.actions:
    groups = set(fc.data_path.split('"')[1] for fc in a.fcurves if fc.data_path.startswith("pose.bones["))
    actions.append({"name": a.name, "frame_range": rnd(a.frame_range), "fcurves": len(a.fcurves),
                    "bones_animated": len(groups), "users": a.users, "fake_user": a.use_fake_user})

images = [{"name": i.name, "size": list(i.size), "file": i.filepath, "packed": i.packed_file is not None}
          for i in bpy.data.images if i.type == "IMAGE"]

emit({"file": bpy.data.filepath, "blender": bpy.app.version_string,
      "units": {"system": scene.unit_settings.system, "scale_length": scene.unit_settings.scale_length,
                "note": "lengths below are centimetres (Unreal units)"},
      "frames": {"start": scene.frame_start, "end": scene.frame_end, "fps": scene.render.fps / scene.render.fps_base},
      "objects": objects, "actions": actions, "materials": [m.name for m in bpy.data.materials],
      "images": images})
