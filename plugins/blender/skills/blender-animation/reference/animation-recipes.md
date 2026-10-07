# Animation recipes (verified in background mode on Blender 4.0.2 and 5.2.1)

Each block runs as `blender_python` `code` (scope: `bpy`, `Vector`, `Matrix`, `math`, `ARGS`;
import `Euler`/`Quaternion` from `mathutils` yourself). `Hero`, `Partner`, `pelvis`, `hand.R` are
example names - read the real ones with `blender_info`. Save with `save_as` unless you mean to
overwrite the source.

The recipes were first verified on 4.0.2. After the F-curve and bone-selection code was made
version-tolerant, every block here, the helper included, was run in order on 5.2.1. The 4.x
branches of `action_fcurves` and recipe 6 (the code for before 4.4 and before 5.0) have not been
run on a 4.x build.

## F-curves on any version: `action_fcurves` (paste it into every block that uses it)

Blender 4.4 moved an action's F-curves into a channelbag per action slot, and 5.0 removed
`Action.fcurves` (`AttributeError: 'Action' object has no attribute 'fcurves'`). Read and write
F-curves through this helper on every version:

```python
def action_fcurves(act, obj=None, ensure=False):
    """F-curves of `act` on any Blender. Since 4.4 they live in a channelbag per action slot;
    5.0 removed Action.fcurves. Uses obj's assigned slot when obj plays `act`, else the first
    slot. ensure=True creates the slot (assigned to obj), layer, strip and channelbag."""
    if not hasattr(act, "slots"):                                   # 4.3 and older
        return act.fcurves
    ad = obj.animation_data if obj is not None else None
    mine = ad is not None and ad.action == act
    slot = (ad.action_slot if mine else None) or (act.slots[0] if len(act.slots) else None)
    if not ensure:
        bags = [st.channelbag(slot) for ly in act.layers for st in ly.strips] if slot else []
        return next((cb.fcurves for cb in bags if cb), [])
    if slot is None:
        slot = act.slots.new(id_type="OBJECT", name=obj.name if obj is not None else act.name)
    if mine and ad.action_slot != slot:
        ad.action_slot = slot
    layer = act.layers[0] if len(act.layers) else act.layers.new("Layer")
    strip = layer.strips[0] if len(layer.strips) else layer.strips.new(type="KEYFRAME")
    return strip.channelbag(slot, ensure=True).fcurves
```

`find(data_path, index=i)` and `remove(fc)` work on what it returns. `new()` names the group
differently: `group_name=` on a channelbag (4.4+), `action_group=` on the old `Action.fcurves`
(recipe 3 handles both). Without `ensure` it returns `[]` when the action has no keys yet.
`bpy_extras.anim_utils.action_get_channelbag_for_slot(act, slot)` and
`action_ensure_channelbag_for_slot(act, slot)` do the channelbag half on 5.x.

## 1. List actions, ranges and who uses them

```python
for act in bpy.data.actions:                    # action_fcurves: top of this file
    on = [o.name for o in bpy.data.objects if o.animation_data and o.animation_data.action == act]
    bones = sorted({fc.data_path.split('"')[1] for fc in action_fcurves(act) if fc.data_path.startswith("pose.bones")})
    print(f"{act.name:28s} range={tuple(act.frame_range)} manual={act.use_frame_range} "
          f"cyclic={act.use_cyclic} fake_user={act.use_fake_user} on={on} bones={len(bones)}")
s = bpy.context.scene
print("scene fps", s.render.fps / s.render.fps_base, "timeline", s.frame_start, s.frame_end)
```

Does an action fit a rig (every F-curve path resolves)? The FBX exporter's all-actions mode uses
the same test:

```python
def action_fits(arm, act):
    bad = set()
    for fc in action_fcurves(act, arm):
        try:
            arm.path_resolve(fc.data_path)
        except ValueError:
            bad.add(fc.data_path)
    return sorted(bad)
print(action_fits(bpy.data.objects["Hero"], bpy.data.actions["A_Hero_Punch"]))
```

## 2. Create an action with keys on pose bones

```python
from mathutils import Euler
scene = bpy.context.scene
scene.render.fps, scene.render.fps_base = 30, 1.0
arm = bpy.data.objects["Hero"]

def new_action(arm, name, start, end, loop=False):
    act = bpy.data.actions.get(name) or bpy.data.actions.new(name)
    act.use_fake_user = True                      # survives save without users
    act.use_frame_range = True                    # manual range: export, loops, inspect use it
    act.frame_start, act.frame_end = start, end
    act.use_cyclic = loop                         # cyclic-aware auto handles at the loop seam
    arm.animation_data_create()
    arm.animation_data.action = act
    for pb in arm.pose.bones:                     # no pose leaking in from the previous action
        pb.matrix_basis = Matrix.Identity(4)
    return act

def key_rot(arm, bone, frame, xyz_deg):
    """Rotation in the bone's LOCAL axes (Y runs along the bone), stored as a quaternion."""
    pb = arm.pose.bones[bone]
    pb.rotation_mode = "QUATERNION"
    q = Euler([math.radians(a) for a in xyz_deg], "XYZ").to_quaternion()
    q.make_compatible(pb.rotation_quaternion)     # shortest path from the current value
    pb.rotation_quaternion = q
    pb.keyframe_insert("rotation_quaternion", frame=frame, group=bone)

def key_loc(arm, bone, frame, xyz_m):
    pb = arm.pose.bones[bone]
    pb.location = xyz_m                            # bone-local axes, metres
    pb.keyframe_insert("location", frame=frame, group=bone)

act = new_action(arm, "A_Hero_Punch", 1, 30)
for f, rot in ((1, (0, 0, 0)), (8, (0, 0, -20)), (12, (10, 0, 110)), (16, (10, 0, 110)), (30, (0, 0, 0))):
    key_rot(arm, "upper_arm.R", f, rot)            # rest, anticipation, strike, hold, recover
print(act.name, tuple(act.frame_range), len(action_fcurves(act, arm)))
```

Pose-bone `location` and rotation are in the bone's own rest axes, not world axes. To find which
local axis is world forward for a bone: `arm.data.bones[b].matrix_local.to_3x3().inverted() @ Vector((0, -1, 0))`.

## 3. Write many keys fast with the F-curve API

```python
def set_keys(act, data_path, index, frame_values, group, interp="BEZIER", obj=None):
    """obj: the object that plays act (its slot gets the keys). Needs action_fcurves."""
    fcs = action_fcurves(act, obj, ensure=True)
    grp = {"group_name": group} if hasattr(act, "slots") else {"action_group": group}
    fc = fcs.find(data_path, index=index) or fcs.new(data_path, index=index, **grp)
    fc.keyframe_points.clear()
    fc.keyframe_points.add(len(frame_values))
    fc.keyframe_points.foreach_set("co", [c for fv in frame_values for c in fv])
    for k in fc.keyframe_points:
        k.interpolation = interp
        k.handle_left_type = k.handle_right_type = "AUTO_CLAMPED"
    fc.update()
    return fc

arm, act = bpy.data.objects["Hero"], bpy.data.actions["A_Hero_Punch"]
# pelvis bob: pelvis bone points up, so its local Y is world up
set_keys(act, 'pose.bones["pelvis"].location', 1, [(1, 0), (12, -0.03), (30, 0)], "pelvis", obj=arm)
```

Interpolation per key: `BEZIER` (default), `LINEAR` (root travel, mechanical), `CONSTANT`
(stepped holds, switches). Easing on Bezier keys: `k.easing` with `EASE_IN`/`EASE_OUT`.

## 4. Make a loop and check the seam

```python
act = bpy.data.actions["A_Hero_Idle"]
arm = bpy.data.objects["Hero"]
arm.animation_data.action = act
act.use_frame_range, act.use_cyclic = True, True
f0, f1 = map(int, act.frame_range)
for fc in action_fcurves(act, arm):            # last key = first key, per channel
    k0 = fc.keyframe_points[0]
    if fc.keyframe_points[-1].co.x < f1:
        fc.keyframe_points.insert(f1, k0.co.y)
    else:
        fc.keyframe_points[-1].co.y = k0.co.y
    if not any(m.type == "CYCLES" for m in fc.modifiers):
        fc.modifiers.new("CYCLES")              # preview past the range; FBX bakes the range only
    fc.update()

def loop_error(arm, act, skip=("root",)):
    scene = bpy.context.scene
    f0, f1 = map(int, act.frame_range)
    def pose(f):
        scene.frame_set(f)
        return {pb.name: pb.matrix_basis.copy() for pb in arm.pose.bones}
    a, b = pose(f0), pose(f1)
    out = []
    for n in a:
        if n in skip:
            continue
        dl = (a[n].translation - b[n].translation).length * 100
        da = math.degrees(a[n].to_quaternion().rotation_difference(b[n].to_quaternion()).angle)
        if dl > 0.1 or da > 0.5:
            out.append((n, round(dl, 2), round(da, 2)))
    return out
print("seam errors (bone, cm, deg):", loop_error(arm, act))
```

For root-motion loops, skip `root` in the seam check (it travels) and check that its per-cycle
displacement is what the design asks for (recipe 5).

## 5. Root motion: animate the root, or move hip travel onto it

Author root motion directly: key `root` location (linear) and keep `pelvis` relative to it.

```python
arm, act = bpy.data.objects["Hero"], bpy.data.actions["A_Hero_Walk"]
fwd_local = arm.data.bones["root"].matrix_local.to_3x3().inverted() @ Vector((0, -1, 0))
f0, f1 = map(int, act.frame_range)
for i in range(3):   # 1.2 m per cycle, constant speed
    set_keys(act, 'pose.bones["root"].location', i, [(f0, 0.0), (f1, fwd_local[i] * 1.2)], "root", "LINEAR", arm)
```

(`set_keys` from recipe 3.) Speed = distance / ((f1 - f0) / fps); give it to the gameplay
programmer - the character's max walk speed has to match or feet slide.

Mocap and Mixamo-style clips carry the travel in the hips. Move the ground-plane part onto
`root`, keeping every pose identical in world space (verified: pelvis positions unchanged,
root follows it on the ground):

```python
def pelvis_travel_to_root(arm, act, root="root", hips="pelvis"):
    scene = bpy.context.scene
    arm.animation_data.action = act
    rb, hb = arm.pose.bones[root], arm.pose.bones[hips]
    f0, f1 = map(int, act.frame_range)
    rows = []
    for f in range(f0, f1 + 1):                   # read everything first
        scene.frame_set(f)
        rows.append((f, hb.matrix.copy()))
    for f, hips_pose in rows:
        scene.frame_set(f)
        t = hips_pose.translation
        rb.matrix = Matrix.Translation((t.x, t.y, 0.0)) @ rb.bone.matrix_local
        bpy.context.view_layer.update()
        hb.matrix = hips_pose                     # restore the hips in armature space
        rb.keyframe_insert("location", frame=f, group=root)
        hb.keyframe_insert("location", frame=f, group=hips)
pelvis_travel_to_root(bpy.data.objects["Hero"], bpy.data.actions["A_Hero_WalkMocap"])
```

This keys every frame; heading (yaw) stays on the hips. To make a clip in-place instead, delete
the root's location F-curves after the transfer:

```python
arm, act = bpy.data.objects["Hero"], bpy.data.actions["A_Hero_WalkMocap"]
fcs = action_fcurves(act, arm)
for fc in [fc for fc in fcs if fc.data_path == 'pose.bones["root"].location']:
    fcs.remove(fc)
```

## 6. Bake constraints / IK to keys

```python
scene = bpy.context.scene
arm, act = bpy.data.objects["Hero"], bpy.data.actions["A_Hero_Step"]
arm.animation_data.action = act
bake_bones = ["thigh.L", "shin.L", "foot.L"]          # the bones the constraints move
for o in bpy.context.view_layer.objects:
    o.select_set(o == arm)
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")
for pb in arm.pose.bones:
    if hasattr(pb, "select"):                         # 5.0+: selection is on the pose bone
        pb.select = pb.name in bake_bones
    else:                                             # 4.x: on the armature's Bone
        pb.bone.select = pb.name in bake_bones
f0, f1 = map(int, act.frame_range)
bpy.ops.nla.bake(frame_start=f0, frame_end=f1, step=1, only_selected=True, visual_keying=True,
                 clear_constraints=True, use_current_action=True, bake_types={"POSE"})
bpy.ops.object.mode_set(mode="OBJECT")
print("constraints left:", {b: len(arm.pose.bones[b].constraints) for b in bake_bones})
```

`use_current_action=True` writes into the assigned action; `False` makes a new action named
`Action` - rename it. `clear_constraints=True` removes the constraints from the baked bones; do it
on a `save_as` copy, or use `False` and mute them before export. Delete the IK target empties'
animation too if the export file should be clean.

## 7. Two characters: partner action on one timeline

```python
from mathutils import Euler
scene = bpy.context.scene
hero, partner = bpy.data.objects["Hero"], bpy.data.objects["Partner"]
IMPACT = 12
# ... new_action / key_rot from recipe 2 ...
new_action(hero, "A_Hero_Punch", 1, 30)
for f, rot in ((1, (0, 0, 0)), (8, (0, 0, -20)), (IMPACT, (10, 0, 110)), (16, (10, 0, 110)), (30, (0, 0, 0))):
    key_rot(hero, "upper_arm.R", f, rot)
new_action(partner, "A_Partner_HitReact", 1, 30)
for f, rot in ((1, (0, 0, 0)), (IMPACT, (0, 0, 0)), (IMPACT + 3, (-25, 0, 0)), (24, (-10, 0, 0)), (30, (0, 0, 0))):
    key_rot(partner, "spine", f, rot)            # reaction starts after the contact frame
# Slide the partner along the attacker's facing so the contact lands with a 3 cm gap
fwd = Vector((0, -1, 0))
scene.frame_set(IMPACT)
hand = hero.matrix_world @ hero.pose.bones["hand.R"].tail
head = partner.matrix_world @ partner.pose.bones["head"].head
partner.location += fwd * (hand + fwd * 0.03 - head).dot(fwd)
bpy.context.view_layer.update()
print("root distance cm", round((hero.location - partner.location).length * 100, 1),
      "| partner yaw deg", round(math.degrees(partner.rotation_euler.z), 1))
```

Verified with `blender_anim_inspect`: contact 3.0 cm at frame 12. The printed distance and yaw
are the placement gameplay must reproduce in Unreal; write them down with the actions.

## 8. Foot contact and sliding

```python
def foot_report(arm, act, bone="foot.L", point="head", contact_cm=2.0):
    """Frames where the foot is planted (within contact_cm of its lowest height) and how far it
    slides between planted frames. World space: correct for root-motion clips."""
    scene = bpy.context.scene
    arm.animation_data.action = act
    f0, f1 = map(int, act.frame_range)
    pts = []
    for f in range(f0, f1 + 1):
        scene.frame_set(f)
        pb = arm.pose.bones[bone]
        pts.append((f, arm.matrix_world @ (pb.head if point == "head" else pb.tail)))
    floor = min(p.z for _, p in pts)
    prev = None
    for f, p in pts:
        planted = (p.z - floor) * 100 < contact_cm
        slide = (p.xy - prev.xy).length * 100 if planted and prev is not None else 0.0
        print(f"{f:4d} h={(p.z - floor) * 100:5.1f}cm {'PLANT' if planted else '     '} slide={slide:4.1f}cm")
        prev = p if planted else None
foot_report(bpy.data.objects["Hero"], bpy.data.actions["A_Hero_Walk"])
```

Root motion: a planted foot should slide < 0.5 cm per frame. In-place clips: a planted foot
moves backwards at exactly the walk speed (that is correct - the capsule moves it forward in
game); check that speed is constant over the contact.

## 9. Retarget between rigs with the same bone names and rest orientations

For different proportions but matching names and rolls (e.g. two mannequin-based characters).
Different rest orientations or names need Unreal's IK Retargeter or an add-on instead.

```python
src, dst = bpy.data.objects["Hero"], bpy.data.objects["Partner"]
act = bpy.data.actions["A_Hero_Punch"]
src.animation_data.action = act
hips_ratio = dst.data.bones["pelvis"].head_local.z / src.data.bones["pelvis"].head_local.z
for pb in dst.pose.bones:
    if pb.name not in src.pose.bones:
        continue
    c = pb.constraints.new("COPY_ROTATION"); c.name = "RT_rot"
    c.target, c.subtarget = src, pb.name
    c.target_space = c.owner_space = "LOCAL"
    if pb.name in ("root", "pelvis"):
        c = pb.constraints.new("COPY_LOCATION"); c.name = "RT_loc"
        c.target, c.subtarget = src, pb.name
        c.target_space = c.owner_space = "LOCAL"
        c.influence = 1.0   # scale translation below instead if hips_ratio differs a lot
print("hip height ratio", round(hips_ratio, 3))
```

Then bake `dst` (recipe 6, all bones, `use_current_action=False`, `clear_constraints=True`),
rename the new action, and scale its root/pelvis location keys by `hips_ratio` so strides fit
the legs. Check feet (recipe 8) and hands (`blender_anim_inspect`) afterwards.
