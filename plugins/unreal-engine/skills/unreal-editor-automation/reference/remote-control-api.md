# Remote Control HTTP API

The `unreal` MCP server's live tools (`ue_call`, `ue_property`, `ue_python`, and the tools
built on editor Python such as `ue_search_assets` and `ue_console`) sit on the Remote Control
API plugin's HTTP server. You normally
use the tools, not raw HTTP; this file explains what they send, which object paths are
valid, and how to read errors.

## Server basics

- Plugin: **Remote Control API** (`RemoteControl` in the `.uproject`). The editor also needs
  the web server running: console `WebControl.StartServer`, or Project Settings > Plugins >
  Remote Control > **Auto Start Web Server**.
- Default HTTP port **30010** (`http://127.0.0.1:30010`). A separate WebSocket server
  defaults to port 30020. Both ports are configurable in the same settings page.
- Bodies are JSON with `Content-Type: application/json`. Most object routes use `PUT`.
- By default only local clients should talk to it. Recent versions add access controls
  (restricting remote access, an optional passphrase for non-local clients, and a switch that
  must be on for remote Python execution). If a request is refused with a message about
  permissions, passphrase or Python, the human has to change Project Settings > Plugins >
  Remote Control; do not try to work around it.
- Everything a request does runs on the game thread, between frames. A slow call freezes the editor.

## Routes

### `GET /remote/info`
Lists every registered route with verb and description. Use it to confirm the server is up
and to check which routes this engine version has.
```
curl -s http://127.0.0.1:30010/remote/info
```

### `PUT /remote/object/call`
Call a `UFUNCTION` on an object.
```json
{
  "objectPath": "/Game/Maps/Main.Main:PersistentLevel.BP_Door_C_2",
  "functionName": "Open",
  "parameters": { "bInstant": true },
  "generateTransaction": true
}
```
- `parameters` keys are the C++ parameter names (not snake_case). Out parameters and the
  return value come back in the response body (`ReturnValue` for the return).
- `generateTransaction: true` wraps the call in an undoable editor transaction.
- Static functions (Blueprint function libraries) are called on the class default object:
```json
{
  "objectPath": "/Script/EditorScriptingUtilities.Default__EditorAssetLibrary",
  "functionName": "ListAssets",
  "parameters": { "DirectoryPath": "/Game/Props", "bRecursive": true, "bIncludeFolder": false }
}
```
- Running Python (what `ue_python` does under the hood):
```json
{
  "objectPath": "/Script/PythonScriptPlugin.Default__PythonScriptLibrary",
  "functionName": "ExecutePythonCommand",
  "parameters": { "PythonCommand": "import unreal; unreal.log('hi')" }
}
```
  `ExecutePythonCommandEx` additionally returns `CommandResult` and `LogOutput`. Both are
  refused when remote Python execution is disabled in the Remote Control settings.
- Console command through `KismetSystemLibrary::ExecuteConsoleCommand`:
```json
{
  "objectPath": "/Script/Engine.Default__KismetSystemLibrary",
  "functionName": "ExecuteConsoleCommand",
  "parameters": { "WorldContextObject": "/Game/Maps/Main.Main", "Command": "stat unit" }
}
```

Only functions that are reflected (`UFUNCTION`) and, in practice, `BlueprintCallable` are
reachable. Plain C++ methods are not.

### `PUT /remote/object/property`
Read or write a property.
```json
{ "objectPath": "/Game/Maps/Main.Main:PersistentLevel.PointLight_0.LightComponent0",
  "propertyName": "Intensity", "access": "READ_ACCESS" }
```
```json
{ "objectPath": "/Game/Maps/Main.Main:PersistentLevel.PointLight_0.LightComponent0",
  "propertyName": "Intensity", "access": "WRITE_TRANSACTION_ACCESS",
  "propertyValue": { "Intensity": 5000.0 } }
```
- `access`: `READ_ACCESS`, `WRITE_ACCESS`, or `WRITE_TRANSACTION_ACCESS` (undoable; prefer it).
- Omit `propertyName` with `READ_ACCESS` to read all readable properties of the object.
- `propertyValue` is an object keyed by the property name. Structs are nested objects:
  `{"RelativeLocation": {"X": 0, "Y": 0, "Z": 100}}`. Object references are path strings.
- Many properties are private with setter functions (e.g. a component's `RelativeLocation`);
  writing them may be refused or may not refresh the actor. Prefer calling the setter
  (`K2_SetActorLocation`, `SetIntensity`) or use `ue_python`.

### `PUT /remote/object/describe`
Returns the class, properties and functions of an object: useful to discover exact names.
```json
{ "objectPath": "/Game/Maps/Main.Main:PersistentLevel.BP_Door_C_2" }
```

### `PUT /remote/search/assets`
Asset Registry search.
```json
{
  "Query": "Chair",
  "Filter": {
    "ClassNames": ["StaticMesh"],
    "PackagePaths": ["/Game/Props"],
    "RecursivePaths": true
  },
  "Limit": 50
}
```
Returns asset name, class and object path for each hit. Depending on engine version,
`ClassNames` may need full class paths (`/Script/Engine.StaticMesh`); if a short name returns
nothing, retry with the full path.

`ue_search_assets` does not use this route. It runs an Asset Registry query as editor Python
(through `ExecutePythonCommandEx`, so remote Python must be enabled) and compares short class
names: pass `StaticMesh`, `Blueprint`, `Material`; a full path such as `/Script/Engine.StaticMesh`
matches nothing.

### `PUT /remote/batch`
Several requests in one round trip, executed in order.
```json
{
  "Requests": [
    { "RequestId": 1, "URL": "/remote/object/property", "Verb": "PUT",
      "Body": { "objectPath": "/Game/Maps/Main.Main:PersistentLevel.PointLight_0.LightComponent0",
                "propertyName": "Intensity", "access": "READ_ACCESS" } },
    { "RequestId": 2, "URL": "/remote/object/call", "Verb": "PUT",
      "Body": { "objectPath": "/Script/EditorScriptingUtilities.Default__EditorAssetLibrary",
                "functionName": "DoesAssetExist", "parameters": { "AssetPath": "/Game/Maps/Main" } } }
  ]
}
```
The response has one entry per request with its `RequestId`, status code and body. For
multi-step logic prefer a single `ue_python` script instead.

### Presets
`GET /remote/presets` lists Remote Control Presets (assets that expose chosen properties and
functions under friendly names); `GET /remote/preset/<Name>` describes one. Useful for
virtual production setups; game projects rarely need them.

## Object path formats

| What | Path |
|---|---|
| Asset (object path) | `/Game/Props/SM_Chair.SM_Chair` (package path `.` object name) |
| Asset (package path) | `/Game/Props/SM_Chair` - accepted by EditorAssetLibrary functions, not as an `objectPath` |
| Blueprint generated class | `/Game/BP/BP_Door.BP_Door_C` |
| Blueprint CDO | `/Game/BP/BP_Door.Default__BP_Door_C` |
| Native class | `/Script/Engine.StaticMeshActor` (`/Script/<Module>.<ClassName without prefix>`) |
| Native CDO (static functions) | `/Script/Engine.Default__KismetSystemLibrary`, `/Script/EditorScriptingUtilities.Default__EditorAssetLibrary`, `/Script/MyGame.Default__MyFunctionLibrary` |
| Level actor | `/Game/Maps/Main.Main:PersistentLevel.StaticMeshActor_3` |
| Actor component | `/Game/Maps/Main.Main:PersistentLevel.StaticMeshActor_3.StaticMeshComponent0` |
| Actor in a streaming sublevel | `/Game/Maps/Main_Audio.Main_Audio:PersistentLevel.AmbientSound_1` (the sublevel's package) |
| PIE copy (avoid) | `/Game/Maps/UEDPIE_0_Main.Main:PersistentLevel.StaticMeshActor_3` |

Rules:
- The segment after `PersistentLevel.` is the actor's object **name**, not its Outliner
  label. Get real paths from `ue_level_actors` or Python `actor.get_path_name()`.
- With World Partition / One File Per Actor the actor's path is still
  `<Map>.<Map>:PersistentLevel.<Name>` while the actor is loaded; unloaded actors are not
  addressable. Load the region (or use Python with the World Partition editor tools) first.
- Component names come from `CreateDefaultSubobject(TEXT("..."))` in C++ or the component
  name in the Blueprint; list them with `/remote/object/describe` or
  `actor.get_components_by_class(unreal.ActorComponent)` in Python.
- Native struct: `/Script/MyGame.ItemRow` for `FItemRow`. Enum: `/Script/MyGame.EItemType`.

## Responses and errors

- `200` with a JSON body: success. Call results are keyed by parameter/return names.
- `400` with `errorMessage`: bad path, unknown function/property, wrong parameter type,
  access denied by settings. Read the message; it names the failing part.
- `404`: the route does not exist in this version (check `/remote/info`).
- Connection refused: web server not started or wrong port - see SKILL.md section 1.
- A call that returns 200 but changed nothing: often a private property written directly,
  a PIE object path, or a non-transactional write the editor then overwrote. Verify by
  reading back, and prefer setter functions or `ue_python`.
