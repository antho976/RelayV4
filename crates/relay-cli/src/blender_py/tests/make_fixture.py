# Builds tests/fixture.blend: a small rig facing -Y (Blender's front), skinned to a blocky body,
# holding a sword on hand.R with a Grip empty, with a "Swing" action and a partner rig.
# Run: blender -b --factory-startup --python make_fixture.py -- <out.blend>
import bpy, sys, math
from mathutils import Vector

out = sys.argv[sys.argv.index("--") + 1]
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene

BONES = [  # name, head, tail, parent
    ("root", (0, 0, 0), (0, 0.2, 0), None),
    ("pelvis", (0, 0, 0.95), (0, 0, 1.1), "root"),
    ("spine", (0, 0, 1.1), (0, 0, 1.35), "pelvis"),
    ("head", (0, 0, 1.45), (0, 0, 1.7), "spine"),
    ("upper_arm.L", (0.2, 0, 1.35), (0.45, 0, 1.35), "spine"),
    ("forearm.L", (0.45, 0, 1.35), (0.7, 0, 1.35), "upper_arm.L"),
    ("hand.L", (0.7, 0, 1.35), (0.8, 0, 1.35), "forearm.L"),
    ("upper_arm.R", (-0.2, 0, 1.35), (-0.45, 0, 1.35), "spine"),
    ("forearm.R", (-0.45, 0, 1.35), (-0.7, 0, 1.35), "upper_arm.R"),
    ("hand.R", (-0.7, 0, 1.35), (-0.8, 0, 1.35), "forearm.R"),
    ("thigh.L", (0.1, 0, 0.95), (0.1, 0, 0.5), "pelvis"),
    ("shin.L", (0.1, 0, 0.5), (0.1, 0, 0.08), "thigh.L"),
    ("foot.L", (0.1, 0, 0.08), (0.1, -0.15, 0.0), "shin.L"),
    ("thigh.R", (-0.1, 0, 0.95), (-0.1, 0, 0.5), "pelvis"),
    ("shin.R", (-0.1, 0, 0.5), (-0.1, 0, 0.08), "thigh.R"),
    ("foot.R", (-0.1, 0, 0.08), (-0.1, -0.15, 0.0), "shin.R"),
]


def rig(name, location):
    data = bpy.data.armatures.new(name)
    arm = bpy.data.objects.new(name, data)
    scene.collection.objects.link(arm)
    arm.location = location
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.mode_set(mode="EDIT")
    for n, h, t, p in BONES:
        b = data.edit_bones.new(n)
        b.head, b.tail = h, t
        if p:
            b.parent = data.edit_bones[p]
        b.use_deform = n != "root"
    bpy.ops.object.mode_set(mode="OBJECT")
    return arm


def body(name, arm):
    # One box per deforming bone, each weighted fully to its bone.
    import bmesh
    me = bpy.data.meshes.new(name)
    o = bpy.data.objects.new(name, me)
    scene.collection.objects.link(o)
    bm = bmesh.new()
    owners = []
    for n, h, t, p in BONES:
        if n == "root":
            continue
        h, t = Vector(h), Vector(t)
        c = (h + t) / 2
        size = Vector((abs(t.x - h.x) + 0.08, abs(t.y - h.y) + 0.08, abs(t.z - h.z) + 0.08))
        r = bmesh.ops.create_cube(bm, size=1.0)
        for v in r["verts"]:
            v.co = Vector((v.co.x * size.x, v.co.y * size.y, v.co.z * size.z)) + c
            owners.append(n)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(me)
    bm.free()
    me.uv_layers.new(name="UVMap")
    for n, *_ in BONES[1:]:
        o.vertex_groups.new(name=n)
    for i, n in enumerate(owners):
        o.vertex_groups[n].add([i], 1.0, "REPLACE")
    o.parent = arm
    mod = o.modifiers.new("Armature", "ARMATURE")
    mod.object = arm
    return o


hero = rig("Hero", (0, 0, 0))
body("HeroBody", hero)

# A sword along its local +X, gripped at its origin, parented to hand.R.
me = bpy.data.meshes.new("Sword")
sword = bpy.data.objects.new("Sword", me)
scene.collection.objects.link(sword)
import bmesh
bm = bmesh.new()
r = bmesh.ops.create_cube(bm, size=1.0)
for v in r["verts"]:
    v.co = Vector((v.co.x * 1.0 + 0.5, v.co.y * 0.06, v.co.z * 0.02))
bm.to_mesh(me)
bm.free()
sword.parent = hero
sword.parent_type = "BONE"
sword.parent_bone = "hand.R"
# Bone-parented children sit at the bone's tail; put the grip back at the palm (the tail) with
# the blade pointing along the arm's outward direction (-X in world).
sword.matrix_world = __import__("mathutils").Matrix.Translation((-0.8, 0, 1.35)) @ __import__("mathutils").Matrix.Rotation(math.pi, 4, "Z")
grip = bpy.data.objects.new("Grip", None)
scene.collection.objects.link(grip)
grip.parent = sword

# Swing: upper_arm.R rotates 90 degrees about Z over 20 frames (arm swings forward, to -Y).
action = bpy.data.actions.new("Swing")
action.use_fake_user = True
hero.animation_data_create()
hero.animation_data.action = action
pb = hero.pose.bones["upper_arm.R"]
pb.rotation_mode = "XYZ"
for frame, angle in ((1, 0.0), (20, 90.0)):
    pb.rotation_euler = (0, 0, 0)
    # The bone's local Y runs along the arm; rotating about the bone's local X swings it.
    pb.rotation_euler = (math.radians(angle), 0, 0)
    pb.keyframe_insert("rotation_euler", frame=frame)
scene.frame_start, scene.frame_end = 1, 20

partner = rig("Partner", (0, -0.9, 0))
partner.rotation_euler = (0, 0, math.pi)
# Euler like Hero's, so Swing (made on Hero, keying rotation_euler) can play on Partner too.
partner.pose.bones["upper_arm.R"].rotation_mode = "XYZ"
body("PartnerBody", partner)

bpy.ops.wm.save_as_mainfile(filepath=out)
print("fixture written", out)
