# Blender to Unreal troubleshooting

Symptom -> cause -> fix. Fix in Blender, re-run the same `blender_export` / `blender_to_unreal`
call, and compare the numbers. Fix one cause at a time. Python for the fixes:
`blender-fundamentals` (`reference/bpy-recipes.md`, `reference/rig-recipes.md`) and
`prep-recipes.md` here.

## Size and scale

| Symptom | Cause | Fix |
|---|---|---|
| Asset 100x too big (height ratio ~100) | mesh modelled in cm-sized BU (180 BU human) at `scale_length` 1.0, or an object scale of 100 left unapplied | scale the data to metres (apply scale), keep `scale_length` 1.0; check sizes in `blender_info` |
| Asset 100x too small (ratio ~0.01) | object scale 0.01 left from an FBX import; `apply_unit_scale` overridden off; `scale_length` 0.01 with metre-sized geometry | apply the object scale; restore default export options; set `scale_length` 1.0 only after the geometry is truly metre-sized |
| Ratio neither ~1 nor x100 | unapplied object scale; Import Uniform Scale set in Unreal; bounds include a stray far-away vertex or empty | apply scale; reset the import scale; delete loose geometry; export with `objects` |
| `root_bone_scale` not 1 | armature object scale not applied (0.01 or 100 after a round-trip) | apply the armature scale with its animation (rig recipes: scales location keys too), export with the default `FBX_SCALE_ALL` |
| Animations grow or shrink the character | scale keys on bones, or a root bone scale | remove bone scale fcurves unless intended; fix the root scale as above |
| Character correct, animations move it 100x too far | location keys not rescaled after applying the armature scale | scale the `pose.bones[...].location` keys by the same factor (rig recipes) |

## Facing, axes, mirroring

| Symptom | Cause | Fix |
|---|---|---|
| `forward_axis_in_mesh_space` about `[0,-1,0]`; character faces backwards in the Blueprint | rig and mesh faced +Y in Blender; or the rig is mirrored, or its `.L`/`.R` names are swapped (Unreal derives facing from the name pairs) | `blender_rig_check` facing warning and `.L` bones at +X tell which: rotate armature and meshes 180 about Z and apply, or fix the mirror/names; re-export |
| Forward `[1,0,0]` or `[-1,0,0]` | built facing +-X | rotate 90 about Z to face -Y, apply |
| Lying on its back or face (rotated 90 about X) | `axis_forward`/`axis_up` overridden, or a rotation of 90 on the object left unapplied (common after importing from Y-up tools) | restore default axes (`-Z` / `Y`); apply rotation in Blender |
| Facing reversed after import (problem says "mirrored or turned around") | negative scale on the armature or mesh; `.L`/`.R` names on the wrong sides (left must be +X when facing -Y) | apply scale and recalculate normals; rename bones and vertex groups consistently; never mirror with scale -1 |
| Props point the wrong way in Unreal | modelled along Blender -Y or +Y for an item Unreal expects along +X | rotate the mesh data so the long axis is +X, apply; or fix the socket rotation (`unreal-animation-verification`) |
| Rotations off by 90 on imported bones | exporter bone axes overridden (`primary_bone_axis`) | keep defaults (Y / X); do not rotate bones to "fix" it |

## Skeleton and bones

| Symptom | Cause | Fix |
|---|---|---|
| Extra root bone named after the armature object | armature object not named `Armature` | rename the object `Armature`; `root_bone` must read your root |
| Several roots / "multiple roots" import error | IK targets, props or control bones without a parent | parent every bone under `root`; mark controls non-deform (they are dropped) |
| `_end` bones in Unreal | exported with `add_leaf_bones` (or left from an earlier import) | delete `_end` bones (rig recipes); keep `add_leaf_bones` off |
| Control/IK bones in the Skeleton | `use_deform` on for non-deform bones | turn Deform off on controls and mechanism bones |
| Mesh collapses or stretches to the origin in Unreal | vertices weighted to non-deform bones (not exported) or unweighted | `blender_rig_check`: move weights to deform bones; weight every vertex |
| Spiky skin in Unreal, fine in Blender | more than 8 influences, weights not normalized | limit total 8 (4 for mobile), normalize all |
| Import onto a shared Skeleton fails or bones are missing | renamed or re-parented bones; different root | compare with the source rig (prep recipes); rename to match |
| Animation plays on the wrong bones or twisted | bone names differ from the Skeleton; different rest pose (A vs T) | match names; retarget in Unreal (IK Retargeter) for different rest poses |
| Root motion does nothing / character slides | root motion keyed on the armature object, not the `root` bone; root motion not enabled on the clip | animate the root bone; enable root motion on the AnimSequence (`unreal-animation`) |
| Props attached in Blender missing from the skeletal mesh | skeletal exports take only skinned meshes | export the prop as `static`, attach to a socket in Unreal |

## Animation export

| Symptom | Cause | Fix |
|---|---|---|
| Clip has the wrong length or extra frames | scene range used instead of the action's | pass `action`; check `act.frame_range`; keep the scene fps equal to the project's |
| Clip is the wrong action | the armature had another action assigned | pass `action`; with `all_actions`, delete or unfake stale actions |
| Many unwanted clips | `all_actions` exports every action with a fake user | pass `action`, or clean up actions first |
| Jitter or foot sliding compared to Blender | fps mismatch between scene and project (resampling), keys on subframes, or simplification from an `fbx_options` override | set `scene.render.fps` to the project's rate; keys on whole frames; the tool samples every frame (constraints and IK included) with no simplification - leave it so |
| Import error: no skeleton | animation-only FBX imported without `skeleton` | pass `skeleton` with the Skeleton asset path |

## Mesh, normals, UVs, materials

| Symptom | Cause | Fix |
|---|---|---|
| Faceted shading | faces shaded flat; no sharp-edge setup | shade smooth, mark sharp edges by angle (4.0: Auto Smooth on) |
| Soft, smeared hard edges | everything smooth, no sharp edges | mark sharp edges; Weighted Normal modifier for hard-surface |
| Log: no smoothing group information | `mesh_smooth_type` overridden to `OFF` | keep the default `FACE` |
| Normal map seams / lighting wrong | tangents not exported (n-gons), triangulation differs from the bake, or green channel convention | triangulate (all faces for baked assets); Import Normals and Tangents; flip green on the texture |
| Black or see-through faces | flipped normals (often after applying a negative scale) | recalculate normals outside (bmesh `recalc_face_normals`); `blender_rig_check` flags inside-out closed skinned meshes |
| Lightmap errors ("overlapping UVs") | no second UV channel, or it overlaps | add a non-overlapping `UVLightmap` as layer 2 and set Light Map Coordinate Index 1, or let Unreal generate lightmap UVs |
| Textures stretched or missing | no UV map, wrong UV layer order, or image paths absolute/missing | unwrap; put the texture UV first; make image paths relative in the repo |
| Too many material slots or wrong names | stray materials, `.001` duplicates, empty slots | merge duplicates, remove unused slots, name `M_...` |
| Duplicate materials/textures created on every import | `blender_to_unreal` imports materials by default (static, skeletal) | `materials: false` once the project has its materials; assign in Unreal |
| Material slots reassigned wrongly after reimport | slot names changed or reordered | keep slot names stable; reassign once in Unreal |

## Collision, sockets, LODs, pivots

| Symptom | Cause | Fix |
|---|---|---|
| No custom collision (auto box or none) | `UCX_` name does not match the render mesh object name exactly; UCX not exported (not a child, or not in `objects`) | name `UCX_<ExactMeshName>_00`; parent it to the mesh; check `exported.collision` |
| Collision "fills in" a concave shape | one hull around a concave object | split into several convex pieces |
| Collision offset from the mesh | UCX transform differs (unapplied) or the pivot changed after making it | apply the UCX transform in the mesh's space; rebuild after changing the pivot |
| Socket missing | empty not named `SOCKET_`, not a child of the mesh, or it is a skeletal mesh | fix name and parent; for skeletal meshes add sockets on the Skeleton in Unreal |
| Attached item 100x too big, or rolled 90 degrees | the socket kept the FBX unit conversion (scale 100, roll -90) | import with `blender_to_unreal`, which divides the scale back and zeroes the rotation of sockets whose empty had none (`sockets`); otherwise set the socket's scale to 1 and fix its roll in the Static Mesh editor |
| LODs arrive as separate assets or merged into LOD0 | importer does not group `_LODn` children | per-LOD export and LOD import in the Static Mesh editor, or Unreal-generated LODs / Nanite |
| Prop floats above or sinks into the floor when placed | origin not at the base | set origin to the centre of the base, re-export |
| Held item offset from the hand | pivot not at the grip, or socket not at the palm | origin at the grip (prep recipes); tune the socket (`unreal-animation-verification`) |

## Tool and import errors

| Symptom | Cause | Fix |
|---|---|---|
| `nothing to export` | no root mesh/armature that renders, or all in excluded collections | pass `objects` |
| a mesh is missing and listed in `excluded` | its collection is excluded from the view layer (unticked in the outliner) | tick the collection, or name the mesh in `objects` |
| `skeletal export needs an armature among the objects` | `objects` lists only meshes | list the armature (its skinned meshes come along) |
| `nothing was imported` | FBX rejected; read `import_log` | common: animation without `skeleton`, bone mismatch, empty mesh |
| Import settings seem ignored; `Interchange` in the log | UE 5.5+ Interchange pipeline | set the options in the Interchange pipeline; import one asset and inspect; record settings in the project docs |
| Tool cannot reach Unreal | editor closed, or the Unreal plugin/Remote Control not running | `blender_export` and hand over the FBX; `ue_setup_check` (`unreal-editor-automation`) |
