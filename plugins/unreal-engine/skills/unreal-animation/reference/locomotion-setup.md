# Third-person locomotion setup

Read before building or fixing character locomotion. Two routes:

- **State machine + distance matching** (this file, sections 1-8): predictable, cheap, works with
  a modest animation set (the UE5 Mannequin set is enough). The default choice.
- **Motion Matching** (section 9): needs a large, well-captured animation set and the Pose Search
  plugin; start from Epic's Game Animation Sample for your engine version.

Mark every step as agent (text/Python) or human (editor graph work) when you report.

## 1. Inputs you need

Animations on one skeleton (retarget first if not, see SKILL.md IK Retargeter):

| Purpose | Clips |
|---|---|
| Idle | `Idle` (loop), optional idle breaks |
| Cycle | `Walk_Fwd`, `Jog_Fwd`; for strafing also `_Bwd`, `_Left`, `_Right` (loops, in-place) |
| Start | `Jog_Start_Fwd` (+ directional starts for strafing) - in-place with a `Distance` curve |
| Stop | `Jog_Stop_Fwd` - in-place with a `Distance` curve |
| Pivot | `Jog_Pivot` (optional) |
| Air | `Jump_Start`, `Fall_Loop`, `Land` |
| Aim | 9-pose aim offset set (CC, CU, CD, LC, LU, LD, RC, RU, RD) additive |

Check which exist with `ue_search_assets` (`class_names: ["AnimSequence"]`, the character folder).
Locomotion cycles are **in-place** (root motion disabled). Verify with Python:
`seq.get_editor_property("enable_root_motion")` should be `False` for cycles.

## 2. Movement component values (agent, C++ or Blueprint defaults)

Pick the rotation model first; it decides which animations you need:

| Style | CMC / Character settings | Animation needs |
|---|---|---|
| Free third-person (character faces movement) | `bOrientRotationToMovement = true`, `bUseControllerRotationYaw = false`, `RotationRate = (0, 500, 0)` | Forward cycles + starts/stops; 1D speed blend space |
| Strafing / shooter | `bOrientRotationToMovement = false`, `bUseControllerDesiredRotation = true` (or controller yaw) | 2D blend space (direction/speed) or 4-direction cycles + orientation warping |

Keep `MaxWalkSpeed` equal to the speed at which the run cycle's feet do not slide (read the clip's
root speed; for UE5 Mannequin jog ~ 350-400 cm/s). Set `BrakingDecelerationWalking`,
`GroundFriction`, and `bUseSeparateBrakingFriction` deliberately - stop distance prediction
reads them.

## 3. Anim instance (agent)

Use the C++ pattern in `anim-instance-cpp.md`: `GroundSpeed`, `bHasAcceleration`, `bShouldMove`,
`bIsFalling`, `Direction`, `AimPitch`, `AimYaw`, plus for distance matching:
- `DistanceToStop` - predicted distance to where the character will stop (only valid while
  decelerating), from `UAnimCharacterMovementLibrary::PredictGroundMovementStopLocation`
  (Animation Locomotion Library) using velocity and the CMC braking values captured on the game
  thread. Check its parameter list in `AnimCharacterMovementLibrary.h` for your version.
- `DisplacementSinceLastUpdate` - `(CurrentLocation - PreviousLocation).Size2D()`, used by
  `AdvanceTimeByDistanceMatching` for starts and pivots.
- `DisplacementSpeed` - displacement / DeltaSeconds; drives stride warping.

## 4. Blend spaces (agent creates, human places samples)

```python
import unreal
tools = unreal.AssetToolsHelpers.get_asset_tools()
skel = unreal.load_asset("/Game/Characters/Mannequins/Meshes/SK_Mannequin")
f = unreal.BlendSpaceFactory1D()
f.set_editor_property("target_skeleton", skel)
with unreal.ScopedEditorTransaction("Create BS_Locomotion_1D"):
    bs = tools.create_asset("BS_Locomotion_1D", "/Game/Characters/Anims/Locomotion", unreal.BlendSpace1D, f)
unreal.EditorAssetLibrary.save_asset(bs.get_path_name())
```

(`BlendSpaceFactoryNew` + `unreal.BlendSpace` for 2D; confirm factory names with
`help(unreal.BlendSpaceFactory1D)` - they have been stable through 5.x.) Sample placement is
editor work. Human steps for a 1D speed blend space:
1. Open `BS_Locomotion_1D`. Asset Details > Axis Settings > Horizontal Axis: Name `Speed`,
   Minimum 0, Maximum = `MaxWalkSpeed` (e.g. 400), Grid Divisions 4. Smoothing Time 0.1-0.2 s.
2. Drag `Idle` to 0, `Walk_Fwd` to its root speed (~150-200), `Jog_Fwd` to its root speed.
3. Scrub with Ctrl held on the graph; feet should not slide at any speed.

2D strafing: Horizontal `Direction` -180..180 (Grid 4), Vertical `Speed` 0..Max. Put backward
clips at both -180 and 180 so the blend wraps.

## 5. AnimGraph layout (human, exact order)

```
[Locomotion state machine] -> Save Cached Pose "Locomotion"

Layered blend per bone  (Branch Filter: bone spine_01, depth 0; Mesh Space Rotation Blend on)
    Base Pose    <- Use Cached Pose "Locomotion"
    Blend Pose 0 <- Use Cached Pose "Locomotion" -> Slot 'UpperBody'   (reload, melee upper body)
  -> Slot 'DefaultSlot'                        (full-body montages: dodges, hit reacts)
  -> Aim Offset (AO_Aim, Pitch = AimPitch, Yaw = AimYaw)
  -> Orientation Warping / Stride Warping      (section 7, optional)
  -> Inertialization
  -> Foot IK (Leg IK / Foot Placement / Control Rig)   (section 8, optional)
  -> Output Pose
```

The Slot names must exist in the Skeleton's Anim Slot Manager. Aim offset before IK so IK
corrects final feet.

## 6. State machine (human)

States and transition rules (all rules read anim instance variables, nothing else):

| From -> To | Rule | Blend |
|---|---|---|
| Entry -> Idle | - | - |
| Idle -> Start | `bShouldMove` | 0.2 s or Inertialization |
| Start -> Cycle | Automatic Rule Based on Sequence Player in State, or `GroundSpeed` near max | 0.2 s |
| Start -> Stop | `not bHasAcceleration` | 0.2 s |
| Cycle -> Stop | `not bHasAcceleration` | 0.2 s |
| Cycle -> Pivot (optional) | acceleration opposes velocity (dot < -0.5, computed in update) | Inertialization |
| Stop -> Idle | Automatic Rule / Distance to stop ~ 0 | 0.2 s |
| Stop -> Start | `bShouldMove` | 0.2 s |
| any grounded -> Jump/Fall (state alias) | `bIsFalling` | 0.1 s |
| Fall -> Land | `not bIsFalling` | 0.1 s |
| Land -> Idle / Cycle | Automatic Rule, or `bShouldMove` to Cycle | 0.2 s |

- Cycle state content: the blend space player with `GroundSpeed` (and `Direction` for 2D).
- Use a **State Alias** named `Grounded` that aliases Idle/Start/Cycle/Stop/Pivot to one
  "-> Jump" transition instead of five.

## 7. Distance matching, stride and orientation warping

Plugins: **Animation Locomotion Library** (`AnimationLocomotionLibrary`) and **Animation
Warping** (`AnimationWarping`). Enable both (agent: `.uproject` plugins list; editor restart).

1. **Distance curves** (human): select the Start/Stop/Pivot sequences > right-click > Animation
   Modifiers (menu wording varies slightly by version) > add **Distance Curve Modifier** and apply.
   It writes a `Distance` float curve (distance of root from the stop point / start point).
   Check the curve exists in each sequence's Curves panel.
2. **Stop** state: use a **Sequence Evaluator** (not Sequence Player) with the Stop clip. Bind On
   Become Relevant to set the sequence / reset time, and On Update to a thread-safe function
   calling `DistanceMatchToTarget(Evaluator, DistanceToStop, "Distance")`. When the prediction is
   invalid (`DistanceToStop <= 0`), advance time normally instead.
3. **Start** state: Sequence Evaluator with the Start clip; On Update calls
   `AdvanceTimeByDistanceMatching(Context, Evaluator, DisplacementSinceLastUpdate, "Distance", PlayRateClamp)`.
4. **Pivot**: same as Start, plus predict the pivot point with `PredictGroundMovementPivotLocation`
   and match to it until the pivot passes, then advance by distance.
5. **Cycle**: Sequence Player / blend space; optional `SetPlayrateToMatchSpeed` from the
   Sequence Player library to keep playback matched to `DisplacementSpeed`.
6. **Stride Warping** node (after locomotion): Locomotion Speed = `DisplacementSpeed`, set pelvis
   and IK foot bone definitions (`pelvis`, `ik_foot_l`/`ik_foot_r` + thigh/foot bones for the
   Mannequin). Clamps stride so feet match actual speed when gameplay speed differs from clip
   speed (sprint buffs, slow effects).
7. **Orientation Warping** node (strafing): Locomotion Angle = `Direction` (relative to the
   facing direction), set spine bones and IK foot root. Lets 4 cardinal cycles cover all
   directions without diagonal clips.

Stride/orientation warping and Foot Placement need IK foot bones in the skeleton (`ik_foot_root`,
`ik_foot_l`, `ik_foot_r`); the UE5 Mannequin has them. Custom skeletons may need virtual bones or
the IK-bone-free settings of the node (check the node's details).

## 8. Foot IK and turn in place

- **Foot IK**: on slopes/stairs, trace down from each foot, offset pelvis and feet. Options: the
  Foot Placement node (Animation Warping plugin; label experimental in early 5.x), Leg IK + your
  own traces (do traces on the game thread in `NativeUpdateAnimation`, never in the AnimGraph),
  or a Control Rig with its own traces. Disable when falling.
- **Turn in place** (strafing/aiming while standing): keep a `RootYawOffset` that counter-rotates
  the mesh when the actor rotates in place; when the offset exceeds ~90 degrees, play a turn
  animation whose `Remaining Turn Yaw` curve consumes the offset. Lyra's `ABP_Mannequin_Base`
  is a complete reference; replicate its structure rather than inventing one.

## 9. Motion Matching route (summary)

1. Check engine version and Pose Search plugin status in Edit > Plugins. Install the
   **Game Animation Sample** project for the same engine version (Fab / Epic launcher) and
   migrate its character (Content Browser > right-click > Asset Actions > Migrate) rather than
   building from scratch - the schema, databases, chooser tables and AnimBP are tuned together.
2. Components: a trajectory source (Character Trajectory component in 5.4; later versions
   generate the trajectory in the AnimBP via Pose Search trajectory functions - follow the sample
   for your version), Pose Search Schema (bones + trajectory samples), Pose Search Databases
   (grouped clips: idle, starts, loops, stops, pivots), Chooser Table to pick databases by state.
3. AnimGraph: Motion Matching node (databases from the chooser) -> pose history -> Inertialization
   -> warping -> output.
4. Debug with the Rewind Debugger (the Pose Search track shows the chosen pose and its costs).
   Pose Search also has debug console variables; find them by typing `PoseSearch` into the
   editor console's autocomplete - names vary by version, so do not hard-code them.
5. Budget: databases increase memory and search cost; profile with `stat anim` and Insights.

## 10. Multiplayer and debugging

- Everything above runs per machine from replicated movement; no animation state replicates.
  Variables must be derivable from replicated data (velocity, `bIsCrouched`, aim pitch). For
  simulated proxies, acceleration may be zero - derive start/stop intent from velocity change.
- Debug: AnimBP editor > debug object dropdown (pick the PIE instance) to watch states live;
  Rewind Debugger to scrub; `showdebug animation` in PIE for on-screen anim info. The warping
  nodes have debug-draw console variables; find them with console autocomplete (type `Warping`)
  in the running editor before using them.

## Checklist

- [ ] Cycles in-place; start/stop/pivot have a `Distance` curve.
- [ ] Blend space axis max equals CMC max speed; no sliding at walk, jog, and in-between.
- [ ] Stop lands exactly at the capsule's stop position (no slide, no early stop).
- [ ] Jump/Fall/Land from every grounded state via the alias.
- [ ] Remote client (PIE 2 players) looks the same as the local one.
