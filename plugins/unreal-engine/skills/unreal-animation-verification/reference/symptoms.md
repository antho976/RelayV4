# Symptoms, causes and fixes

Measure first (`ue_anim_inspect`), look second (`ue_anim_preview`), then use this table.

| Symptom | Likely causes | How to confirm | Fix |
|---|---|---|---|
| Item in the wrong hand | Wrong socket name in the attach call; socket parented to the other hand bone; left/right swapped in a data table | `attachments[x].side`, the socket's `bone_name` | Attach to the right socket; re-parent the socket |
| Item in the right hand, pointing backwards | Item's long axis differs from the project convention; socket rotated 180 degrees | `end_a_at_start` forward component vs the hand's | Rotate the socket (not the item actor); re-export the item to the convention |
| Item rotated 90 degrees (blade flat, handle sideways) | Up/forward axis mismatch at export; socket roll | Images from `front` and `top` | Socket rotation or export settings; document the convention |
| Grip floats off the palm | Item pivot not at grip; socket at the wrist bone origin | `grips` distance, constant across samples | Move the socket to the palm; add a `Grip` socket and offset |
| Grip right in one clip, off in others | Clips animated with different hand poses | `grips` per clip | Hand IK to the item; per-clip socket override; re-author the outliers |
| Off hand misses a two-handed item | No off-hand IK; IK target on the wrong socket; IK alpha 0 in that state | `grips` for the off hand in the raw clip; AnimBP IK node | Add or fix the IK node, driven by the item's off-hand socket |
| Item passes through the torso | Socket rotation; hold style unsuitable for this move; body bulkier than the animation's source | `clearance to own body`, time of worst approach | Different socket for the move; IK adjustment; additive widen |
| Hands through the chest (bulky character) | Animations authored on a slimmer body | Preview at key poses; arm-to-spine distance via `contacts` | Arm spread additive / Control Rig offset per character |
| Character raises the wrong arm | Retarget chains swapped; mirrored export; Mirror node active | Reference-pose inspect; retarget chain mapping | Fix the chain map; re-export; remove the mirror |
| Arms float away from or into the body after retargeting | Retarget pose mismatch (A vs T) | Compare reference poses of source and target | Align the retarget poses in the IK Retargeter |
| Whole character faces sideways in game | Mesh component yaw in the Character Blueprint | `frame.forward_axis_in_mesh_space` vs actor forward | Set the mesh component's relative yaw (commonly -90 for +Y-facing meshes) |
| Feet below ground or floating | Capsule half-height vs mesh offset; root not at the feet | `feet_height` at the reference pose | Offset the mesh component Z; fix the root at export |
| Feet slide | Movement speed does not match the clip's stride | Planted foot moving between samples | Match speed, stride warping, distance matching |
| Two characters clip during an interaction | Wrong spacing or facing; timing offset | `partner_clipping`, `closest_approach` time | Enforce spacing with Motion Warping or alignment; `time_offset` |
| Hit lands before or after the visible impact | Hit notify placed off the action frames | Sample around the notify time with `times` | Move the notify; hit window as a notify state over the action phase |
| Pop at the start or end of a move | Missing transition blend; start pose differs from the previous pose | Preview at 0 s and at the previous clip's end | Blend time or inertialization; matching start poses |
| Movement feels floaty or weightless | Play rate raised on a heavy clip; no anticipation; no follow-through | Clip length vs design; preview of wind-up and recovery | Use a heavier clip or retime; add hit-stop and camera shake (`unreal-game-design`) |
