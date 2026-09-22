# blender_python: the agent's own script, with `bpy` and `ARGS`, then an optional save.
import addon_utils
for module in ARGS.get("addons") or []:
    addon_utils.enable(module, default_set=True)
scope = {"bpy": bpy, "ARGS": ARGS, "Vector": Vector, "Matrix": Matrix, "math": math, "emit": emit}
print("RELAY_OUT_BEGIN")
try:
    exec(compile(ARGS["code"], "script", "exec"), scope)
finally:
    print("RELAY_OUT_END")
saved = None
if ARGS.get("save_as"):
    bpy.ops.wm.save_as_mainfile(filepath=ARGS["save_as"], copy=False)
    saved = ARGS["save_as"]
elif ARGS.get("save"):
    if not bpy.data.filepath:
        raise RuntimeError("save needs an opened file; use save_as for a new one")
    bpy.ops.wm.save_mainfile()
    saved = bpy.data.filepath
print("RELAY_SAVED:" + json.dumps(saved))
