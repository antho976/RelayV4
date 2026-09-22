# ue_anim_inspect and ue_anim_preview recipes

Asset paths below are placeholders; find real ones with `ue_search_assets`. Every
`ue_anim_inspect` call below can be sent to `ue_anim_preview` with the same `mesh`,
`animation`, `attachments` and `partner` to get pictures of the same setup.

Point references used in `track` and `contacts`:

| Reference | Means |
|---|---|
| `hand_r` | a bone or a socket of the character |
| `partner:spine_03` | a bone or socket of the partner |
| `item:hammer:end_a` | an end of the item's bounds along its longest axis (`end_a` is the + end, `end_b` the - end) |
| `item:hammer:Grip` | a socket on the item mesh |
| `item:hammer:origin` / `center` | the item's pivot / bounds centre |

## Reference pose sanity check (any character)

```json
{"mesh": "/Game/Characters/Hero/SKM_Hero", "track": ["hand_l", "hand_r", "foot_l", "foot_r", "head"]}
```

Expect `hand_r` side `right`, `hand_l` side `left`, feet up near 0, head up near the character's
height, `left_right_pairs_found` > 0.

## One-handed item (sword, torch, tool, phone)

```json
{"mesh": "/Game/Characters/Hero/SKM_Hero",
 "animation": "/Game/Characters/Hero/Anims/AS_Attack_01",
 "samples": 12,
 "attachments": [{"name": "item", "mesh": "/Game/Items/SM_Torch", "socket": "hand_r_tool",
                  "grips": [{"socket": "Grip", "bone": "hand_r_palm", "tolerance": 4}]}],
 "track": ["item:item:end_a", "item:item:end_b"]}
```

`hand_r_palm` is a socket at the centre of the palm (add one per hand once; see the Python at the
end). A grip `bone` can be any bone or socket; comparing to `hand_r` itself measures to the wrist.

Read: `attachments.item.side`, grip distances per sample, `clearance to own body` for `end_a`,
`end_b` and `center`. For which way the item points, compare `end_a_at_start` with
`fwd_right_up` of the attachment: for a sword held forward, `end_a` (the tip) should be more
forward than the hand.

## Two-handed item (hammer, rifle, spear, axe)

```json
{"mesh": "/Game/Characters/Hero/SKM_Hero",
 "animation": "/Game/Characters/Hero/Anims/AS_Hammer_Slam",
 "samples": 16,
 "attachments": [{"name": "hammer", "mesh": "/Game/Items/SM_Hammer", "socket": "hand_r_twohand",
                  "grips": [{"socket": "Grip", "bone": "hand_r_palm"},
                            {"socket": "OffHand", "bone": "hand_l_palm", "tolerance": 6}]}]}
```

An off-hand grip that fails in the raw clip but is fine in game means the Animation Blueprint's
IK fixes it at runtime; the inspect tool reads the clip, not the AnimBP. Say so in the report and
check the in-game result with `ue_screenshot` during Simulate/PIE if needed.

## Shield or forearm item

```json
{"attachments": [{"name": "shield", "mesh": "/Game/Items/SM_Shield", "socket": "lowerarm_l_shield"}],
 "track": ["item:shield:center"]}
```

A shield should be on the `left` side (for a left-arm shield) and in front during a block:
`item:shield:center` forward > the chest's forward.

## Worn item (backpack, hat, holster)

```json
{"attachments": [{"name": "pack", "mesh": "/Game/Items/SM_Backpack", "socket": "spine_05_back"}],
 "body_radius": 12}
```

For worn items, clearance to the attaching chain is ignored by design; look at `closest_approach`
for arms passing through the item during big motions (run, climb).

## Two characters (hit and reaction, grab, hug, carry)

```json
{"mesh": "/Game/Characters/Hero/SKM_Hero",
 "animation": "/Game/Anims/AS_Hug_Giver",
 "samples": 12,
 "partner": {"mesh": "/Game/Characters/NPC/SKM_NPC", "animation": "/Game/Anims/AS_Hug_Receiver",
             "location": [0, 70, 0], "yaw": 180, "time_offset": 0.0},
 "contacts": [
   {"a": "hand_r", "b": "partner:spine_04", "expect": "touch", "distance": 8, "window": [0.6, 1.4]},
   {"a": "head", "b": "partner:head", "expect": "apart", "distance": 15}]}
```

`location` is in this character's mesh space: for a mesh facing +Y (UE mannequin), `[0, 70, 0]`
is 70 cm in front. `frame.forward_axis_in_mesh_space` tells you which axis is forward. `yaw: 180`
makes them face each other. Sweep `location` and `time_offset` to find the spacing and timing
where contacts pass and nothing else clips; those numbers are what the game code (Motion Warping
target distance, sync offset) needs.

## Python: list and edit sockets

```python
import unreal
mesh = unreal.load_asset("/Game/Characters/Hero/SKM_Hero")
skeleton = mesh.get_editor_property("skeleton")
for owner in (mesh, skeleton):
    for s in owner.get_editor_property("sockets"):
        print(owner.get_name(), s.get_editor_property("socket_name"), s.get_editor_property("bone_name"),
              s.get_editor_property("relative_location"), s.get_editor_property("relative_rotation"))
```

Changing a socket (verify the property names with `help(unreal.SkeletalMeshSocket)` in your version):

```python
with unreal.ScopedEditorTransaction("Adjust hand_r_tool socket"):
    socket = mesh.find_socket("hand_r_tool")
    socket.set_editor_property("relative_location", unreal.Vector(4.0, -2.0, 0.5))
    socket.set_editor_property("relative_rotation", unreal.Rotator(roll=0.0, pitch=0.0, yaw=90.0))
    owner = socket.get_outer()
    owner.modify()
unreal.EditorAssetLibrary.save_loaded_asset(owner)
```

Creating a new socket from Python is not reliably exposed across versions; if `help()` shows no
add-socket function, give the human the steps: Skeleton editor → right-click the bone → Add
Socket → name it → set its transform to the numbers you measured.

## Reading a result

- Start with `passed` and `problems` (they are sorted by sample time, capped at 80).
- `attachments` gives the side, the long axis (`X`/`Y`/`Z` of the item mesh), the length, and
  where each end is at the first sample.
- `closest_approach` gives the worst moment for each item end and partner probe: fix the smallest
  negative clearance first; the time tells you which part of the clip.
- `samples[i].points` are only the points you listed in `track`.
