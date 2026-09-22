# Project layout, modules, targets and plugins

## Top-level folders and files

| Path | What it is | Commit? | Agent action |
|---|---|---|---|
| `<Project>.uproject` | JSON project descriptor: engine version, modules, plugins | Yes | Edit as JSON; keep it valid |
| `Source/` | C++ modules and `*.Target.cs` | Yes | Edit freely |
| `Content/` | Assets (`.uasset`, `.umap`), `__ExternalActors__/`, `__ExternalObjects__/` for World Partition / One File Per Actor | Yes (usually via Git LFS or Perforce) | Editor only |
| `Config/` | `Default*.ini` project settings, platform subfolders | Yes | Edit as text |
| `Plugins/` | Project plugins, each with a `.uplugin` | Yes | Edit code/config as text, content via editor |
| `Saved/` | Logs (`Saved/Logs/<Project>.log`), autosaves, per-user config, crash reports, SaveGames, screenshots | No | Read logs; never edit config here |
| `Intermediate/` | UHT output (`*.generated.h`, `*.gen.cpp`), build products, project files | No | Never edit; do not delete unless asked |
| `Binaries/` | Compiled DLLs/executables per platform (`Binaries/Win64/UnrealEditor-<Module>.dll`) | Usually no (some teams commit editor binaries for artists) | Never edit; do not delete unless asked |
| `DerivedDataCache/` | Local cache of cooked/compiled data (shaders, textures) | No | Never edit |
| `Build/` | Platform packaging resources (icons, manifests), sometimes build scripts | Yes | Edit if needed |
| `*.sln`, `.vscode/`, `*.code-workspace` | Generated IDE files | No | Regenerate with "Generate Project Files" |

Deleting `Intermediate/` and `Binaries/` forces a full rebuild and project-file regeneration. It
is a legitimate fix for some stale-UHT or stale-DLL problems, but it is slow and destroys the
human's local build: explain and ask, do not do it on your own.

A C++ module inside `Source/`:
```
Source/
  MyGame.Target.cs
  MyGameEditor.Target.cs
  MyGame/
    MyGame.Build.cs
    Public/            (optional split; headers other modules may include)
    Private/           (.cpp and private headers)
    MyGame.h / MyGame.cpp   (module implementation)
  MyGameEditor/        (optional editor-only module)
    MyGameEditor.Build.cs
    ...
```
Small projects often have a flat module (headers and .cpp side by side). Match what exists.

## `.uproject`

```json
{
    "FileVersion": 3,
    "EngineAssociation": "5.4",
    "Category": "",
    "Description": "",
    "Modules": [
        {
            "Name": "MyGame",
            "Type": "Runtime",
            "LoadingPhase": "Default",
            "AdditionalDependencies": [ "Engine" ]
        },
        {
            "Name": "MyGameEditor",
            "Type": "Editor",
            "LoadingPhase": "Default"
        }
    ],
    "Plugins": [
        { "Name": "EnhancedInput", "Enabled": true },
        { "Name": "PythonScriptPlugin", "Enabled": true },
        { "Name": "ModelingToolsEditorMode", "Enabled": true, "TargetAllowList": [ "Editor" ] }
    ]
}
```
- `EngineAssociation` is a version (`"5.4"`) for launcher builds or a GUID / empty string for source
  builds registered on the machine. Do not change it unless asked; it switches engine versions.
- A project with no `Modules` is Blueprint-only. Adding a C++ class through the editor
  (Tools > New C++ Class) creates the module, targets and `.uproject` entry for you; doing it by
  hand requires all of: module folder + `.Build.cs` + module implementation `.cpp` + `.uproject`
  entry + `ExtraModuleNames` in both targets.
- Disabling a default-enabled plugin is written as `"Enabled": false`.

## Module types (`Type` in `.uproject` / `.uplugin`)

| Type | Loaded in |
|---|---|
| `Runtime` | Everything except programs (game, editor, server, client, commandlets) |
| `RuntimeNoCommandlet` | Runtime, but not commandlets |
| `RuntimeAndProgram` | Runtime and standalone programs |
| `CookedOnly` | Only cooked (packaged) games |
| `UncookedOnly` | Only uncooked (editor / uncooked game) - e.g. Blueprint node modules (`K2Node`) |
| `Developer` | Development builds and editor; deprecated in favor of `DeveloperTool` |
| `DeveloperTool` | Editor and programs, and development game builds when `bBuildDeveloperTools` is on |
| `Editor` | Editor only |
| `EditorNoCommandlet` | Editor only, not commandlets |
| `EditorAndProgram` | Editor and programs |
| `Program` | Standalone programs only |
| `ServerOnly` | Everything except client-only targets |
| `ClientOnly` | Everything except dedicated servers |
| `ClientOnlyNoCommandlet` | Client, not commandlets |

Rule: game code is `Runtime`; editor tooling (factories, detail customizations, editor
utilities, anything depending on `UnrealEd`) is `Editor` in its own module.

## Loading phases (`LoadingPhase`)

In order: `EarliestPossible`, `PostConfigInit`, `PostSplashScreen`, `PreEarlyLoadingScreen`,
`PreLoadingScreen`, `PreDefault`, `Default`, `PostDefault`, `PostEngineInit`, `None` (never
auto-loaded). Use `Default` unless you have a reason. Modules that register custom shader
directories need `PostConfigInit`. Modules that must see engine objects fully initialized
can use `PostEngineInit`. A module whose classes are referenced by assets must be loaded before
those assets load, which `Default` satisfies for normal content.

## `.Target.cs`

```csharp
using UnrealBuildTool;
using System.Collections.Generic;

public class MyGameTarget : TargetRules
{
    public MyGameTarget(TargetInfo Target) : base(Target)
    {
        Type = TargetType.Game;
        DefaultBuildSettings = BuildSettingsVersion.Latest;
        IncludeOrderVersion = EngineIncludeOrderVersion.Latest;
        ExtraModuleNames.Add("MyGame");
    }
}

public class MyGameEditorTarget : TargetRules   // in MyGameEditor.Target.cs
{
    public MyGameEditorTarget(TargetInfo Target) : base(Target)
    {
        Type = TargetType.Editor;
        DefaultBuildSettings = BuildSettingsVersion.Latest;
        IncludeOrderVersion = EngineIncludeOrderVersion.Latest;
        ExtraModuleNames.AddRange(new string[] { "MyGame", "MyGameEditor" });
    }
}
```
- Target class name = file name without `.Target.cs` + `Target`.
- Templates from the editor pin concrete values (`BuildSettingsVersion.V5`,
  `EngineIncludeOrderVersion.Unreal5_4`) rather than `Latest`. Keep whatever the project uses;
  when you upgrade engine versions, UBT warnings tell you which values to bump.
- `TargetType`: `Game`, `Editor`, `Client`, `Server`, `Program`. A dedicated server needs a
  `<Project>Server.Target.cs` and a source-built engine.

## `.Build.cs`

```csharp
using UnrealBuildTool;

public class MyGame : ModuleRules
{
    public MyGame(ReadOnlyTargetRules Target) : base(Target)
    {
        PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;

        PublicDependencyModuleNames.AddRange(new string[]
        {
            "Core", "CoreUObject", "Engine", "InputCore", "EnhancedInput"
        });

        PrivateDependencyModuleNames.AddRange(new string[] { "UMG", "Slate", "SlateCore" });

        if (Target.bBuildEditor)
        {
            PrivateDependencyModuleNames.Add("UnrealEd");   // only if runtime code needs editor APIs under WITH_EDITOR
        }
    }
}
```
- **Public** dependencies: modules whose headers appear in *your public headers*. **Private**:
  used only in `.cpp` files / private headers. Prefer private.
- Common module names: `Core`, `CoreUObject`, `Engine`, `InputCore`, `EnhancedInput`, `UMG`,
  `Slate`, `SlateCore`, `AIModule`, `NavigationSystem`, `GameplayTasks`, `GameplayTags`,
  `GameplayAbilities`, `Niagara`, `PhysicsCore`, `Chaos`, `NetCore`, `OnlineSubsystem`,
  `DeveloperSettings`, `AssetRegistry`, `UnrealEd` (editor), `EditorSubsystem` (editor),
  `Json`, `JsonUtilities`, `HTTP`. If unsure which module owns a class, find its header in engine
  source; the folder directly under `Runtime/`, `Editor/`, `Developer/` or a plugin's `Source/`
  is the module name.
- `PublicIncludePaths` / `PrivateIncludePaths` are rarely needed for normal `Public/`/`Private/`
  layouts; UBT adds them.

## Module implementation file

Primary game module (exactly one per project):
```cpp
// MyGame.cpp
#include "MyGame.h"
#include "Modules/ModuleManager.h"

IMPLEMENT_PRIMARY_GAME_MODULE(FDefaultGameModuleImpl, MyGame, "MyGame");
```
Other modules:
```cpp
#include "Modules/ModuleManager.h"

class FMyGameEditorModule : public IModuleInterface
{
public:
    virtual void StartupModule() override {}
    virtual void ShutdownModule() override {}
};

IMPLEMENT_MODULE(FMyGameEditorModule, MyGameEditor);
```
The second argument must equal the module name. `FDefaultModuleImpl` is available when no
startup code is needed.

## `.uplugin`

```json
{
    "FileVersion": 3,
    "Version": 1,
    "VersionName": "1.0",
    "FriendlyName": "My Tools",
    "Description": "",
    "Category": "Other",
    "CreatedBy": "",
    "CanContainContent": true,
    "Installed": false,
    "Modules": [
        { "Name": "MyTools", "Type": "Runtime", "LoadingPhase": "Default" },
        { "Name": "MyToolsEditor", "Type": "Editor", "LoadingPhase": "Default" }
    ],
    "Plugins": [
        { "Name": "EnhancedInput", "Enabled": true }
    ]
}
```
- Plugin layout: `Plugins/MyTools/MyTools.uplugin`, `Plugins/MyTools/Source/MyTools/MyTools.Build.cs`,
  `Plugins/MyTools/Content/` (mounted at `/MyTools/` when `CanContainContent` is true).
- Plugin modules do not go in the project's targets' `ExtraModuleNames`; enabling the plugin is
  enough. The plugin's `Plugins` array declares plugin-to-plugin dependencies.
- Game Feature plugins (`Plugins/GameFeatures/...`) are a separate system (Game Features plugin);
  only use them if the project already does.

## Where things get written at runtime

- Logs: `Saved/Logs/<Project>.log` (current), `<Project>-backup-<date>.log` (previous runs).
- Crash reports: `Saved/Crashes/`.
- SaveGame slots: `Saved/SaveGames/<Slot>.sav` in editor/PIE and development builds on desktop.
- Per-user settings: `Saved/Config/<Platform>Editor/` in the editor (e.g. `WindowsEditor`),
  `Saved/Config/<Platform>/` in games.
- Autosaves and backups: `Saved/Autosaves/`, `Saved/Backup/`.
- Packaged builds write to the user's platform save directory, not the project folder.

## Building

- `ue_build` runs UnrealBuildTool for `<Project>Editor`, host platform, `Development` by default.
  Close the editor first (the running editor locks its DLLs), or have the human use Live Coding
  (Ctrl+Alt+F11) for .cpp-only changes.
- Configurations: `Debug`, `DebugGame` (engine optimized, game code debuggable), `Development`,
  `Test`, `Shipping`. `DebugGame Editor` is the usual choice for debugging game code.
- After a successful build, the editor must be restarted to pick up new reflected types.
- UBT errors about "Unable to find target" usually mean a `.Target.cs` class/file name mismatch.
