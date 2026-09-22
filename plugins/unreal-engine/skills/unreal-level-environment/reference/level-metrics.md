# Level Metrics

Units: 1 Unreal unit (uu) = 1 cm. Always derive metrics from the project's real character. Read
the values with the Python snippet in `SKILL.md` §3, or from the character Blueprint's Class Defaults
(Capsule Component, Character Movement). The numbers below are engine and template defaults to
sanity-check against. Verify them in your project, because templates change between versions.

## 1. Character defaults to check

| Property | `ACharacter`/CMC C++ default | Typical UE5 Third Person template | Where |
|---|---|---|---|
| Capsule radius | 34 | 42 | CapsuleComponent |
| Capsule half-height | 88 (176 tall) | 96 (192 tall) | CapsuleComponent |
| Max Walk Speed | 600 cm/s | 500 cm/s | CharacterMovement |
| Jump Z Velocity | 420 cm/s | 700 cm/s | CharacterMovement |
| Gravity Scale | 1.0 (gravity 980 cm/s²) | 1.0 | CharacterMovement |
| Air Control | 0.05 | 0.35 | CharacterMovement |
| Max Step Height | 45 | 45 | CharacterMovement |
| Walkable Floor Angle | about 44.77° | same | CharacterMovement |
| Crouched Half Height | 40 | same unless changed | CharacterMovement |

The UE5 mannequins (Manny and Quinn) are roughly 180 cm tall. `BaseEyeHeight` (on `APawn`) defaults to 64 above
the capsule center. With the default capsule that puts the eyes about 152 cm above the floor, and first-person
projects usually tune it to about 150–170.

## 2. Jump math

```
g        = 980 * GravityScale                  (cm/s^2)
H_max    = Vz^2 / (2 g)                        apex height above take-off
t_apex   = Vz / g
t_flat   = 2 * Vz / g                          air time landing at take-off height
D_flat   = t_flat * Vxy                        horizontal distance, flat
```

This ignores `JumpMaxHoldTime` (a held jump adds height), different falling gravity, and air control
acceleration. Measure in PIE to confirm.

| Vz / Vxy | H_max | t_flat | D_flat |
|---|---|---|---|
| 420 / 600 (C++ default) | 90 cm | 0.86 s | 514 cm |
| 700 / 500 (template) | 250 cm | 1.43 s | 714 cm |

Design rules derived from the jump:
- **Guaranteed jumpable gap**: at most 70–80% of `D_flat`. Gaps meant to be *barely* makeable sit at 90% or more,
  and you should use them rarely and on purpose.
- **Jump-up ledge**: at most 80% of `H_max`, including the capsule's ability to clear the lip.
- **Never-jumpable** (to keep the player in bounds): more than 120% of `H_max`, plus a blocking volume.
- **Step-up without jumping**: at most `MaxStepHeight` (45). Stairs and curbs must stay under it.
- **Walkable slope**: under `WalkableFloorAngle` (about 44°). Steeper slopes slide, so use that for "you can't climb this" slopes.
- **Fall damage** (if any): define it in the same units, for example safe below 400 cm and lethal above 1200 cm.

## 3. Spaces (third person, capsule about 84 cm wide)

| Element | Minimum | Comfortable | Notes |
|---|---|---|---|
| Door width | 120 | 150–200 | Camera and animations need clearance. Co-op needs wider |
| Door height | 220 | 250–300 | Taller for giants or mounts |
| Corridor width | 200 | 300–400 | Combat corridors 400+ |
| Ceiling height | 300 | 400+ | Spring arm camera needs headroom |
| Stair rise / run | ≤45 rise | 15–20 rise, 25–35 run | Rise must be under MaxStepHeight. Add collision ramps for smoothness |
| Ramp angle | — | ≤ 30° | Under WalkableFloorAngle, readable as walkable |
| Low cover height | 90 | 100–110 | Hides a crouched capsule (crouched height about 80) |
| High cover height | 180 | 200+ | Hides a standing capsule |
| Window sill (vault) | 80 | 90–110 | Match the vault or mantle animation exactly |
| Mantle / ledge-grab height | — | per animation | Author and trigger heights must match the animation set |
| Railing height | 90 | 100–110 | Prevents walking off. Use collision, not only visuals |

First person: narrower spaces read as roomy, but the camera still needs clearance. Doors of 100–120 and corridors of 150+.
Vehicles: roads at least 3x the vehicle width, and turning radii from the vehicle's measured minimum.

## 4. Distances and speeds

- Seconds to cross = distance / `MaxWalkSpeed`. A 30 m room at 500 cm/s takes 6 s. Use this to pace
  corridors (about 10–20 s of travel between beats is a common target in linear games).
- Shooter engagement bands (tune per weapon ranges): close under 10 m, mid 10–30 m, long 30 m and beyond.
  Cover spacing roughly one sprint burst (3–6 m) apart on contested routes.
- Landmark visibility: large landmarks should read at their silhouette from 200 m or more. Check against fog and HLOD distance.
- Default camera horizontal FOV is 90. Most players see about 45 degrees either side of forward, so place guidance
  inside that cone at the moment it matters.

## 5. Modular grid

- Base grid: 100 cm (or 50 cm for fine interior kits). Wall modules 100/200/400 wide, 300 or 400 tall.
- Floor-to-floor: 300–400 (interiors), with a floor slab thickness of 20–50.
- Wall thickness: 20–30, and never under 10, because Lumen leaks light through thin walls.
- Pivots: bottom corner, on grid. Door and window modules have their openings centered on the grid.
- Snap settings: translation grid equal to the kit grid, rotation 90 degrees for kit pieces and 15 for props.

## 6. Texel density

Pick one target and keep to it across the kit:
- Third person: about 512 px/m (5.12 px/cm). A 4 m wall uses about 2048 px across.
- First person: about 1024 px/m (10.24 px/cm).
- Tiling materials and trim sheets make the target easy to hold. Hero props can go higher.
- Checking: the *Required Texture Resolution* view mode, or a texel-density checker material.

## 7. Greybox color language (example; document yours)

| Color | Meaning |
|---|---|
| Grey | Walkable or neutral geometry |
| Orange / yellow | Climbable, mantleable, interactive |
| Red | Hazard, damage, kill volume |
| Blue | Water or swimmable |
| Green | Pickups, objectives, checkpoints |
| Purple | Scripted events or triggers (editor-only visual) |

## 8. Offline metric calculator (plain Python)

```python
def jump_metrics(vz, vxy, gravity_scale=1.0, world_gravity=980.0):
    g = world_gravity * gravity_scale
    h = vz * vz / (2 * g)
    t = 2 * vz / g
    return {"apex_cm": round(h, 1), "air_s": round(t, 3), "flat_dist_cm": round(t * vxy, 1),
            "safe_gap_cm": round(0.75 * t * vxy, 1), "safe_ledge_cm": round(0.8 * h, 1)}

print(jump_metrics(700, 500))   # template-ish values; replace with the project's real ones
```

## 9. Metrics test map

Build `/Game/Maps/Dev/L_Metrics` (or reuse one if it exists): rows of gaps in 50 cm steps, ledges in 25 cm steps,
stairs at several rises, ramps at 10/20/30/40/45/50 degrees, doors at 100–250 widths, cover at
80–200 heights, and labeled text render actors. When the character or movement changes, replay it and
record the measured values in the project's level design doc. The layout can be scripted with the patterns in
`python-level-scripting.md` (spawn cubes on a grid, label them, and put them in a folder).
