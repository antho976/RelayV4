# blender_render: images of objects from named views and frames. Nothing is saved to the file.
import os

out_dir = ARGS["out_dir"]
width, height = int(ARGS.get("width", 640)), int(ARGS.get("height", 480))
fov = math.radians(float(ARGS.get("fov", 40)))

names = ARGS.get("objects") or [o.name for o in scene.objects if o.type in ("MESH", "ARMATURE") and not o.hide_render]
targets = [obj(n) for n in names]
# An armature frames its meshes; meshes are what render.
framed = []
for o in targets:
    framed.append(o)
    if o.type == "ARMATURE":
        framed += [c for c in scene.objects if c.type == "MESH" and (c.parent == o or any(m.type == "ARMATURE" and m.object == o for m in c.modifiers))]
meshes = [o for o in framed if o.type == "MESH"] or framed

arm = next((o for o in targets if o.type == "ARMATURE"), None)
if arm is not None:
    set_action(arm, ARGS.get("action"))
    frame = body_frame(arm)
    forward, right = frame["forward"], frame["right"]
else:
    forward, right = Vector((0, -1, 0)), Vector((-1, 0, 0))
if ARGS.get("forward"):
    forward = Vector(ARGS["forward"]).normalized()
    right = forward.cross(Vector((0, 0, 1))).normalized()

if ARGS.get("isolate", True):
    keep = set(o.name for o in framed)
    for o in scene.objects:
        if o.type in ("MESH", "CURVE", "SURFACE", "META", "FONT") and o.name not in keep:
            o.hide_render = True

# 4.2 to 4.5 call EEVEE Next "BLENDER_EEVEE_NEXT"; 5.0 took the plain name back.
eevee = "BLENDER_EEVEE_NEXT" if (4, 2, 0) <= bpy.app.version < (5, 0, 0) else "BLENDER_EEVEE"
engine = {"workbench": "BLENDER_WORKBENCH", "eevee": eevee, "cycles": "CYCLES"}[ARGS.get("engine", "workbench")]
scene.render.engine = engine
if engine == "CYCLES":
    scene.cycles.samples = int(ARGS.get("samples", 16))
    scene.cycles.device = "CPU"
    scene.cycles.use_denoising = False
if engine == "BLENDER_WORKBENCH":
    shading = scene.display.shading
    shading.light = "STUDIO"
    shading.color_type = ARGS.get("color", "MATERIAL")
    shading.show_cavity = True
    shading.show_object_outline = True
scene.render.resolution_x, scene.render.resolution_y = width, height
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = "PNG"
scene.render.film_transparent = False

if engine != "BLENDER_WORKBENCH" and not any(o.type == "LIGHT" for o in scene.objects):
    sun = bpy.data.objects.new("RelaySun", bpy.data.lights.new("RelaySun", "SUN"))
    scene.collection.objects.link(sun)
    sun.rotation_euler = (math.radians(50), 0, math.radians(30))

cam_data = bpy.data.cameras.new("RelayCam")
cam_data.angle = fov
cam = bpy.data.objects.new("RelayCam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam

up = Vector((0, 0, 1))
DIRS = {"front": forward, "back": -forward, "right": right, "left": -right, "top": (up + forward * 0.001).normalized(),
        "three_quarter": (forward + right + up * 0.5).normalized(), "three_quarter_left": (forward - right + up * 0.5).normalized()}

frames = [int(f) for f in ARGS.get("frames") or [scene.frame_current]]
files = []
for f in frames[:12]:
    scene.frame_set(f)
    lo, hi = world_bbox(meshes)
    center = (lo + hi) / 2
    radius = max((hi - lo).length / 2, 0.05)
    dist = radius / math.tan(fov / 2) * 1.15
    for view in ARGS.get("views") or ["front", "right", "three_quarter"]:
        if view not in DIRS:
            raise RuntimeError("unknown view %r; use %s" % (view, ", ".join(sorted(DIRS))))
        eye = center + DIRS[view] * dist
        cam.location = eye
        cam.rotation_euler = (center - eye).to_track_quat("-Z", "Y").to_euler()
        cam_data.clip_end = dist * 4
        cam_data.clip_start = max(dist / 1000, 0.001)
        path = os.path.join(out_dir, "f%04d_%s.png" % (f, view))
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        files.append({"view": "frame %d %s" % (f, view), "file": path})

emit({"files": files, "engine": engine, "objects": [o.name for o in framed],
      "facing": {"forward": rnd(forward, 3), "right": rnd(right, 3)}})
