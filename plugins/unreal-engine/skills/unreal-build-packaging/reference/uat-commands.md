# UAT, UBT and commandlet reference

Paths assume `<Engine>` is the engine root and `<Proj>` the absolute path to the `.uproject`.
Always pass absolute project paths. When a flag's behaviour matters, check it against the
engine you are on: `RunUAT BuildCookRun -help` prints the options that version accepts.

Executables:
- UAT: `<Engine>/Engine/Build/BatchFiles/RunUAT.bat` (Windows) / `RunUAT.sh` (Linux, Mac).
- UBT via script: `<Engine>/Engine/Build/BatchFiles/Build.bat`, `.../Linux/Build.sh`, `.../Mac/Build.sh`.
- Editor for commandlets: `<Engine>/Engine/Binaries/Win64/UnrealEditor-Cmd.exe` (console
  output, Windows); Linux `<Engine>/Engine/Binaries/Linux/UnrealEditor`; Mac
  `<Engine>/Engine/Binaries/Mac/UnrealEditor.app/Contents/MacOS/UnrealEditor`.

## Building (UBT)

```
Build.bat <Target> <Platform> <Config> -Project="<Proj>" -WaitMutex
```
- `<Target>`: `MyGameEditor`, `MyGame`, `MyGameServer`, `MyGameClient`.
- `<Platform>`: `Win64`, `Linux`, `LinuxArm64`, `Mac`, `Android`, `IOS`.
- `<Config>`: `Debug`, `DebugGame`, `Development`, `Test`, `Shipping`.
- Extras: `-DisableUnity`, `-Clean`, `-verbose`, `-NoHotReloadFromIDE`.
- Project files: `Build.bat -projectfiles -project="<Proj>" -game -rocket -progress`
  (installed engine) or `GenerateProjectFiles.bat -project="<Proj>" -game -engine` (source engine).

## BuildCookRun

Typical Windows Shipping package:
```
RunUAT.bat BuildCookRun -project="<Proj>" -noP4 -platform=Win64 -clientconfig=Shipping ^
  -build -cook -stage -pak -iostore -compressed -prereqs -nodebuginfo ^
  -archive -archivedirectory="D:/Builds/MyGame" -unattended -utf8output
```
Linux client from Linux:
```
./RunUAT.sh BuildCookRun -project="<Proj>" -noP4 -platform=Linux -clientconfig=Development \
  -build -cook -stage -pak -iostore -archive -archivedirectory="$PWD/out" -unattended -utf8output
```
Dedicated Linux server only:
```
RunUAT BuildCookRun -project="<Proj>" -noP4 -server -noclient -serverplatform=Linux \
  -serverconfig=Development -build -cook -stage -pak -iostore -archive \
  -archivedirectory="<out>" -unattended
```
Android:
```
RunUAT BuildCookRun -project="<Proj>" -noP4 -platform=Android -cookflavor=ASTC \
  -clientconfig=Development -build -cook -stage -pak -package -archive \
  -archivedirectory="<out>" -unattended
```
Cook only (no compile, reuse binaries):
```
RunUAT BuildCookRun -project="<Proj>" -noP4 -platform=Win64 -cook -skipstage -unattended
```

### Flags

| Flag | Meaning |
|---|---|
| `-project=<abs path>` | Project to build (required) |
| `-noP4` | Do not use Perforce (use for git projects) |
| `-platform=<P>` | Client/game platform (`Win64`, `Linux`, `Mac`, `Android`, `IOS`); `+` separates several |
| `-clientconfig=<C>` | Configuration of the game/client target |
| `-server` / `-noclient` | Also build a server / skip the client |
| `-serverplatform=<P>` / `-serverconfig=<C>` | Server platform and configuration |
| `-target=<Name>` | Choose the target when a project has several of the same type |
| `-build` | Compile the targets (omit to use existing binaries) |
| `-cook` | Cook content |
| `-skipcook` | Reuse content cooked by a previous run |
| `-map=/Game/A+/Game/B` | Cook only these maps (plus their references) |
| `-cookflavor=<F>` | Texture format flavour (Android: `ASTC`, `ETC2`, `DXT`, `Multi`) |
| `-stage` / `-skipstage` | Copy build + content into `Saved/StagedBuilds/<Platform>` |
| `-stagingdirectory=<dir>` | Custom staging directory |
| `-pak` | Put content in `.pak` files |
| `-iostore` | Use IoStore containers (`.utoc/.ucas`) |
| `-compressed` | Compress pak/IoStore content |
| `-package` | Platform packaging step (required for Android `.apk/.aab`, iOS `.ipa`, Mac `.app`) |
| `-archive` / `-archivedirectory=<dir>` | Copy the final build to a folder |
| `-distribution` | Distribution build (store signing, e.g. Android release keystore, iOS distribution) |
| `-prereqs` | Include the prerequisites installer (Windows) |
| `-nodebuginfo` | Do not stage debug symbols (keep `.pdb` separately for crash analysis) |
| `-crashreporter` | Include the Crash Report Client |
| `-createreleaseversion=<v>` | Record a release (for later patches/DLC) |
| `-basedonreleaseversion=<v>` + `-generatepatch` | Build a patch against a recorded release |
| `-dlcname=<Plugin>` | Build a DLC plugin against a release |
| `-cookcultures=en+fr+de` | Localizations to cook |
| `-run` / `-deploy` / `-device=<id>` | Launch or deploy after building |
| `-addcmdline="<args>"` | Extra arguments for the launched game |
| `-clean` | Full rebuild and clean cook |
| `-nocompileeditor` | Do not build the editor target first (it must already be built) |
| `-unattended` | No dialogs; required on CI |
| `-utf8output` | UTF-8 log output (CI log parsing) |

Incremental ("iterative") cooking flags changed across 5.x versions; check `-help` for
your engine rather than copying a flag from another version.

Logs: UAT prints the log directory; per-step logs (cook, UBT) are under
`<Engine>/Engine/Programs/AutomationTool/Saved/Logs` or the directory printed at the end.

## Other UAT commands

| Command | Example | Notes |
|---|---|---|
| `BuildPlugin` | `RunUAT BuildPlugin -Plugin="<abs>/MyTools.uplugin" -Package="<abs>/Out" -TargetPlatforms=Win64+Linux` | Builds a plugin standalone for distribution; also a good check that it compiles outside the project |
| `BuildGraph` | `RunUAT BuildGraph -Script="<abs>/Build/Pipeline.xml" -Target="Package Game" -set:ProjectPath="<Proj>"` | XML-defined build pipelines (Epic's CI tool); `-ListOnly` prints the node graph |
| `RunUnreal` | `RunUAT RunUnreal -project="<Proj>" -test=<GauntletTest> -build=<staged build dir>` | Gauntlet test runner (see `unreal-testing-debugging`) |
| `Turnkey` | `RunUAT Turnkey -command=VerifySdk -platform=Android` | Checks/installs platform SDKs in versions that ship Turnkey |

## Commandlets (editor binary, headless)

Common arguments: `-unattended -nullrhi -nosplash -nopause -stdout -FullStdOutLogOutput`.

| Task | Command |
|---|---|
| Cook by the book | `UnrealEditor-Cmd "<Proj>" -run=cook -targetplatform=Windows -unattended` (Linux: `-targetplatform=Linux`; output in `Saved/Cooked/<Platform>`) |
| Compile every Blueprint (CI check) | `UnrealEditor-Cmd "<Proj>" -run=CompileAllBlueprints -unattended -nullrhi` |
| Fix up redirectors | `UnrealEditor-Cmd "<Proj>" -run=ResavePackages -fixupredirects -projectonly -unattended` |
| Fill the DDC | `UnrealEditor-Cmd "<Proj>" -run=DerivedDataCache -fill -unattended` |
| Build World Partition HLODs | `UnrealEditor-Cmd "<Proj>" /Game/Maps/OpenWorld -run=WorldPartitionBuilderCommandlet -Builder=WorldPartitionHLODsBuilder -AllowCommandletRendering -unattended` |
| Data validation | `UnrealEditor-Cmd "<Proj>" -run=DataValidation -unattended` (Data Validation plugin enabled) |
| Localization gather | `UnrealEditor-Cmd "<Proj>" -run=GatherText -config="Config/Localization/Game_Gather.ini" -unattended` |
| Python script headless | `UnrealEditor-Cmd "<Proj>" -run=pythonscript -script="<abs>/tools/audit.py" -unattended -nullrhi` |
| Run automation tests | `UnrealEditor-Cmd "<Proj>" -ExecCmds="Automation RunTests MyGame;Quit" -unattended -nullrhi -nopause -TestExit="Automation Test Queue Empty" -ReportExportPath="<abs>/TestReports" -log` |

`-run=pythonscript` runs without the editor UI; scripts that need a viewport or a loaded
level in the editor must run in the full editor instead (`-ExecutePythonScript="<abs>.py"`
at editor startup, or `ue_python` against a running editor).

## Inspecting a packaged build

| Task | Command |
|---|---|
| List a pak's contents | `<Engine>/Engine/Binaries/Win64/UnrealPak.exe "<pak>" -List` |
| Run packaged game with log window | `MyGame.exe -log` |
| Packaged logs (Windows) | `%LOCALAPPDATA%/<Project>/Saved/Logs/` |
| Packaged logs (Linux) | `~/.config/Epic/<Project>/Saved/Logs/` |

## Exit codes and CI parsing

- UAT and UBT return non-zero on failure; always check the exit code.
- Grep logs for `Error:` and `LogCook: Error`, `LogLinker: Error` (missing references),
  `Warning: .* Failed to load` to surface content problems early.
- Keep the full log as a CI artifact; the tail is rarely enough.
