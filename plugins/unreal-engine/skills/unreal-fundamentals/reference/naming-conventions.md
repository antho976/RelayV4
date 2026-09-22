# Naming conventions

The project's existing convention always wins. Before naming anything, look at a few existing
classes (`Source/`) and assets (`ue_search_assets` with a `package_paths` filter) and copy their
pattern. The tables below are Epic's coding standard (C++, mandatory: UHT enforces some of it) and
the widely used asset prefixes from Epic's "Recommended Asset Naming Conventions" page and the
community UE style guide. Asset prefixes are a convention, not enforced by the engine.

## C++ type prefixes (enforced)

UHT checks the prefix of reflected types and fails with an error if it is wrong.

| Prefix | Used for | Example |
|---|---|---|
| `A` | Classes deriving from `AActor` (including Pawn, Character, Controller, GameMode, HUD) | `AMyCharacter` |
| `U` | Classes deriving from `UObject` but not `AActor` (components, subsystems, data assets, widgets) | `UHealthComponent`, `UInventorySubsystem` |
| `F` | Structs (`USTRUCT` and plain) and non-UObject classes | `FItemStats`, `FMyRunnable` |
| `E` | Enums | `EItemRarity` |
| `I` | Interface classes (the `I` half of a `UINTERFACE` pair) | `IInteractable` (with `UInteractable`) |
| `T` | Templates | `TArray`, `TMyPool` |
| `S` | Slate widgets (`SCompoundWidget` subclasses) | `SMyPanel` |
| `G` | Rare; some global singletons (`GEngine`, `GEditor`, `GWorld`) | engine only |

Other rules from Epic's coding standard:
- PascalCase for types, functions, variables, parameters. No underscores in type names.
- Booleans start with `b`: `bIsDead`, `bCanJump`. UHT and the editor strip the `b` for display names.
- Functions that return bool ask a question: `IsVisible()`, `HasAmmo()`, `CanFire()`.
- Out parameters may be prefixed `Out`: `bool TryGetItem(FName Id, FItem& OutItem)`.
- Template parameters: `typename T`, or `InElementType`-style descriptive names.
- `In`/`Out` prefixes on parameters are common in engine code when a name collides with a member.
- Macro / `#define` names are ALL_CAPS.
- File name matches the main type without prefix: `AMyCharacter` lives in `MyCharacter.h` /
  `MyCharacter.cpp`, and its generated header is `MyCharacter.generated.h`.
- Log categories: `LogMyGame`, `LogInventory` (prefix `Log`).
- Delegate types: `F` + `On...` + `Signature`/`Delegate` is common: `FOnHealthChangedSignature`.
- Module API macro: module name uppercased + `_API`: module `MyGame` gives `MYGAME_API`.

Class names should be descriptive and not repeat the parent unnecessarily:
`AMyProjectile` not `AMyProjectileActor`; components end in `Component`
(`UHealthComponent`), subsystems in `Subsystem`, interfaces are adjectives or roles
(`IInteractable`, `IDamageable`).

## Asset name pattern

`Prefix_BaseAssetName_Variant_Suffix`, for example `SM_Rock_02`, `T_Rock_02_N`, `MI_Rock_Mossy`.
- BaseAssetName: short, descriptive, shared across related assets (`Hero`, `Rock`, `Door`).
- Variant: `01`, `02`, or a descriptive variant (`Mossy`, `Red`).
- No spaces, no unicode, no hyphens. Use PascalCase inside segments.

## Asset prefixes

| Asset type | Prefix | Python class |
|---|---|---|
| Level / Map | `L_` (Epic also shows none; some teams use `Lvl_` or `Map_`) | `World` |
| Level Sequence | `LS_` | `LevelSequence` |
| Blueprint (Actor, generic) | `BP_` | `Blueprint` |
| Blueprint Component | `BP_` or `BPC_` | `Blueprint` |
| Blueprint Function Library | `BPFL_` | `Blueprint` |
| Blueprint Macro Library | `BPML_` | `Blueprint` |
| Blueprint Interface | `BPI_` | `Blueprint` |
| Enum (user defined) | `E_` | `UserDefinedEnum` |
| Structure (user defined) | `F_` or `S_` | `UserDefinedStruct` |
| Widget Blueprint | `WBP_` (sometimes `W_`) | `WidgetBlueprint` |
| Editor Utility Widget | `EUW_` | `EditorUtilityWidgetBlueprint` |
| Editor Utility Blueprint | `EUB_` | `EditorUtilityBlueprint` |
| Data Asset | `DA_` | subclass of `DataAsset` / `PrimaryDataAsset` |
| Data Table | `DT_` | `DataTable` |
| Curve Table | `CT_` | `CurveTable` |
| Curve (float/vector/color) | `Curve_` or `C_` | `CurveFloat`, `CurveVector`, `CurveLinearColor` |
| Static Mesh | `SM_` | `StaticMesh` |
| Skeletal Mesh | `SK_` (sometimes `SKM_`) | `SkeletalMesh` |
| Skeleton | `SKEL_` | `Skeleton` |
| Physics Asset | `PHYS_` (sometimes `PA_`) | `PhysicsAsset` |
| Animation Sequence | `A_` or `AS_` | `AnimSequence` |
| Animation Montage | `AM_` | `AnimMontage` |
| Animation Blueprint | `ABP_` | `AnimBlueprint` |
| Blend Space | `BS_` | `BlendSpace` |
| Blend Space 1D | `BS_` | `BlendSpace1D` |
| Aim Offset | `AO_` | `AimOffsetBlendSpace` |
| Control Rig | `CR_` | `ControlRigBlueprint` |
| IK Rig / IK Retargeter | `IK_` / `RTG_` | `IKRigDefinition` / `IKRetargeter` |
| Material | `M_` | `Material` |
| Material Instance | `MI_` | `MaterialInstanceConstant` |
| Material Function | `MF_` | `MaterialFunction` |
| Material Parameter Collection | `MPC_` | `MaterialParameterCollection` |
| Post Process Material | `PP_` or `M_..._PP` | `Material` |
| Physical Material | `PM_` | `PhysicalMaterial` |
| Texture | `T_` | `Texture2D` |
| Texture Cube | `TC_` or `HDR_` | `TextureCube` |
| Render Target | `RT_` | `TextureRenderTarget2D` |
| Niagara System | `NS_` | `NiagaraSystem` |
| Niagara Emitter | `NE_` | `NiagaraEmitter` |
| Sound Wave | `A_` or `SW_` | `SoundWave` |
| Sound Cue | `SC_` (Epic: `A_..._Cue`) | `SoundCue` |
| MetaSound Source | `MS_` or `MSS_` | `MetaSoundSource` |
| Sound Attenuation | `ATT_` | `SoundAttenuation` |
| Sound Class / Mix | `SCL_` / `Mix_` | `SoundClass` / `SoundMix` |
| Input Action | `IA_` | `InputAction` |
| Input Mapping Context | `IMC_` | `InputMappingContext` |
| Behavior Tree | `BT_` | `BehaviorTree` |
| Blackboard | `BB_` | `BlackboardData` |
| BT Task / Service / Decorator (Blueprint) | `BTTask_` / `BTService_` / `BTDecorator_` | `Blueprint` |
| Environment Query | `EQS_` | `EnvQuery` |
| State Tree | `ST_` | `StateTree` |
| Gameplay Ability / Effect / Cue (GAS) | `GA_` / `GE_` / `GC_` | `Blueprint` |
| Font | `Font_` | `Font` |
| Media Player / Source | `MP_` / `FMS_` | `MediaPlayer` / `FileMediaSource` |
| Camera Shake | `CS_` | `Blueprint` |
| Foliage Type | `FT_` | `FoliageType_InstancedStaticMesh` |
| Landscape Layer Info | `LL_` | `LandscapeLayerInfoObject` |
| PCG Graph | `PCG_` | `PCGGraph` |

When the class of an asset matters (e.g. filtering in `ue_search_assets`), use the class name,
not the prefix: prefixes can be wrong.

## Texture suffixes

| Suffix | Content |
|---|---|
| `_D` or `_BC` | Diffuse / Base Color |
| `_N` | Normal |
| `_R` | Roughness |
| `_M` | Metallic |
| `_AO` | Ambient occlusion |
| `_E` | Emissive |
| `_H` | Height / displacement |
| `_A` | Alpha / opacity |
| `_ORM` | Packed Occlusion (R), Roughness (G), Metallic (B) |
| `_Mask` | Generic mask |

Normal maps must use the `Normalmap` compression setting; packed masks (`_ORM`, `_Mask`) must have
sRGB disabled. Check these when importing via Python.

## Folder conventions

Typical layout (follow the project's if different):
```
Content/
  <ProjectName>/          (optional top folder, keeps project content separate from Marketplace/Fab packs)
    Blueprints/ or Core/
    Characters/<Name>/    (mesh, skeleton, anims, materials, textures together)
    Environment/
    Maps/
    UI/
    Input/                (IA_*, IMC_*)
    Data/                 (DA_*, DT_*)
    Audio/
    VFX/
  Developers/<User>/      (personal sandboxes; never ship references into these)
```
- Group by feature or asset owner (all of a character's assets in one folder), not by asset type
  at the top level of a large project.
- Imported third-party packs stay in their own top-level folders; do not rename their contents
  unless asked (it causes huge redirector churn).
- Do not create assets in `/Game/` root; put them in a folder.

## Blueprint member naming

- Variables: PascalCase, booleans with `b` prefix is optional in Blueprint (the editor displays
  C++ `bIsDead` as "Is Dead"). Follow the project.
- Functions and events: verbs (`ApplyDamage`, `OnDeath`). Events that respond to something start
  with `On`. Custom events used as delegates: `Handle...` or `On...`.
- Categories: group variables and functions with the `Category` field (C++ `Category = "Combat"`,
  subcategories with `|`: `"Combat|Weapons"`).

## Map and actor names in levels

Actor *labels* in the Outliner (what `ue_level_actors` shows as label) are editor display names
and can be changed freely; the actor's object *name* (`BP_Door_C_2`) is what paths use and should
not be relied on for gameplay lookups. Use tags (`Actor->Tags`, `ActorHasTag`) or explicit
references rather than names to find actors at runtime.
