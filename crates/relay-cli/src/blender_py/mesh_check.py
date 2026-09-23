# blender_mesh_check: the mesh problems that make an export draw wrongly in Unreal, measured on
# the evaluated mesh (modifiers applied), which is what the exporter writes.
names = ARGS.get("objects") or [o.name for o in scene.objects if o.type == "MESH"]
reports = [mesh_report(obj(n, "MESH")) for n in names]
emit({"meshes": reports, "passed": not any(r["problems"] for r in reports)})
