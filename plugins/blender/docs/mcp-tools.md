# The `blender` MCP server

Served by `relay blender-mcp` over stdio. Each call starts Blender in background mode on one file,
runs a script and exits, so calls are independent and nothing is left running. Paths are relative
to the agent's checkout, and files outside it are refused. Lengths are reported in centimetres,
like Unreal.

### `blender_info` — `{ file? }`
No file: the art files in the checkout (`.blend`, `.fbx`, `.obj`, `.glb`, `.gltf`, `.abc`, `.usd`)
and which Blender was found. With a file: units, frame rate and range; every object with type,
parent (and parent bone), world location and dimensions, unapplied scale or rotation, modifiers;
mesh statistics, UV maps, vertex groups, materials, shape keys and Unreal naming roles (`UCX_`,
`SOCKET_`); armatures (bones, deform bones, roots, left/right pairs, active action); actions; images.

### `blender_python` — `{ file?, code, save?, save_as?, addons?, timeout_s? }`
Runs the code with `bpy`, `Vector`, `Matrix`, `math` and `ARGS` in scope and returns what it prints.
`save` saves the opened file; `save_as` writes a new `.blend` inside the checkout. `addons` enables
bundled add-ons first. An exception returns its traceback.

### `blender_render` — `{ file, objects?, action?, frames?, views?, engine?, color?, isolate?, width?, height? }`
Images of the objects (an armature brings the meshes skinned to it) at each frame and view.
Views are relative to the character's facing, from the rig's `.L`/`.R` pairs (Blender's front, -Y,
otherwise): `front`, `back`, `left`, `right`, `top`, `three_quarter`, `three_quarter_left`.
`workbench` (default) shows shape with cavity shading and outlines; `color` picks its colouring
(`RANDOM` shows separate parts, `VERTEX` vertex colours). `eevee` and `cycles` (CPU) show
materials. Other meshes are hidden unless `isolate` is false. The file is not modified.

### `blender_rig_check` — `{ file, armature?, meshes? }`
`problems` (must fix before export), `warnings`, `passed`, plus facts: bones, deform bones, roots,
left/right pairs, facing, height, and per skinned mesh the unweighted vertices, influence counts,
non-manifold edges, loose vertices and degenerate faces.

### `blender_mesh_check` — `{ file, objects? }`
On the evaluated meshes (modifiers applied): zero-area faces, zero-length edges and inside-out
normals are problems; loose vertices, n-gons (tangents are then skipped by the FBX exporter) and
missing UVs are warnings. `blender_export` runs it and refuses broken meshes unless
`allow_problems` is set.

### `blender_anim_inspect` — `{ file, armature?, action?, frames?, samples?, track?, attachments?, partner?, contacts?, body_radius?, touch_distance? }`
The Blender twin of `ue_anim_inspect`, run before export. References: `bone`, `bone:tail`,
`obj:<object or empty>`, `item:<attachment>:<end_a|end_b|center|origin>`, `partner:<bone>`.
Attachments are objects already attached to the rig (bone parent or Child Of) with optional
`grips`. Returns positions as `[forward, right, up]` cm with sides, grip distances, clearances,
contact checks, feet heights, `problems` and `passed`.

### `blender_export` — `{ file, objects?, path, kind?, action?, animations?, all_actions?, fbx_options? }`
FBX for Unreal. `static`: the mesh with its `SOCKET_` empties, `UCX_`/`UBX_`/`USP_` collision and
LOD children. `skeletal`: the armature and the meshes skinned to it (props parented to bones are
left out), deform bones only, no leaf bones. `animation`: the armature's action only. With an
`action` and no `all_actions`, the timeline is set to that action's range before baking.
`fbx_options` overrides any exporter option. Returns the objects exported, sockets, collision and
the size in cm.

### `blender_to_unreal` — `{ file, objects?, kind, action?, animations?, destination, name?, skeleton?, fbx_path?, materials?, socket_rotation?, importer?, normals?, allow_problems? }`
Sockets from `SOCKET_` empties arrive with a -90° roll from the axis conversion; with
`socket_rotation: "match"` (default) a socket whose empty had no rotation of its own is set back to
zero and the mesh saved (`zero` resets all, `keep` leaves them).

Exports (to `Saved/Relay/Exports/<name>.fbx` unless `fbx_path` says otherwise), imports into the
running Unreal editor through the Unreal plugin (same project guard and editor lock), and
compares: height in Unreal against Blender (a x100 difference is a unit problem), a skeletal
mesh's root bone scale (must be 1), and its facing against the facing recorded at export (a
reversed facing means mirrored or turned around; sides alone cannot show it, since both sides
derive left and right from the bone names).
Returns `problems` and `passed`. Needs the Unreal Engine plugin on for the project and the editor
open.
