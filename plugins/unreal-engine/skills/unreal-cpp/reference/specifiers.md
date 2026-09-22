# Reflection specifiers and meta keys

Specifiers are validated by UHT: a misspelled plain specifier is a build error. `meta = (...)`
keys are NOT validated: a misspelled meta key silently does nothing. When unsure, grep engine
source for an existing use (`grep -rn "meta = (ClampMin" Engine/Source/Runtime/Engine/Classes`) or
check `Engine/Source/Runtime/CoreUObject/Public/UObject/ObjectMacros.h`, which lists the
specifier enums (`namespace UC`, `UP`, `UF`, `US`, `UM`).

## UPROPERTY

### Editor visibility (pick at most one)

| Specifier | Details panel on Blueprint defaults | On placed instances |
|---|---|---|
| `EditAnywhere` | Editable | Editable |
| `EditDefaultsOnly` | Editable | Hidden |
| `EditInstanceOnly` | Hidden | Editable |
| `VisibleAnywhere` | Read-only | Read-only |
| `VisibleDefaultsOnly` | Read-only | Hidden |
| `VisibleInstanceOnly` | Hidden | Read-only |

Component pointers created with `CreateDefaultSubobject` use `VisibleAnywhere` (the pointer is
fixed; the component's own properties are still editable through it). Using `EditAnywhere` on a
component pointer lets users replace it with None and is a common bug.

### Blueprint access

| Specifier | Effect |
|---|---|
| `BlueprintReadOnly` | Get node only |
| `BlueprintReadWrite` | Get and Set nodes |
| `BlueprintAssignable` | Multicast dynamic delegate: Blueprint can bind (Assign/Bind Event) |
| `BlueprintCallable` | Multicast dynamic delegate: Blueprint can call Broadcast |
| `BlueprintAuthorityOnly` | Delegate: only binds on authority |
| `BlueprintGetter = Func` / `BlueprintSetter = Func` | Route Blueprint access through UFUNCTIONs |

### Serialization, config, networking

| Specifier | Effect |
|---|---|
| `Transient` | Not saved to disk; zero-filled on load |
| `DuplicateTransient` | Reset when the object is duplicated (copy/paste, PIE) |
| `TextExportTransient` | Not exported to text (copy/paste) |
| `NonPIEDuplicateTransient` | Reset on duplicate except for PIE |
| `SaveGame` | Included when serializing with `ArIsSaveGame` (see SaveGame in `unreal-gameplay-framework`) |
| `Config` | Loaded from the class's config file (class needs `UCLASS(Config=...)`); writes need `SaveConfig()`/`TryUpdateDefaultConfigFile()` |
| `GlobalConfig` | Like Config, but subclasses cannot override the value |
| `Replicated` | Replicated; register in `GetLifetimeReplicatedProps` with `DOREPLIFETIME` |
| `ReplicatedUsing = OnRep_Func` | Replicated with a RepNotify UFUNCTION called on clients |
| `NotReplicated` | Skip in a replicated struct |
| `Instanced` | Object property owns an instanced subobject (edit inline); pairs with `UCLASS(EditInlineNew)` |
| `Export` | Export the referenced object's properties on copy (for subobjects) |
| `NoClear` | Details panel cannot set it to None |
| `SkipSerialization` | Not serialized in binary, still exported to text |

### Details panel layout

| Specifier | Effect |
|---|---|
| `Category = "A\|B"` | Category and subcategory |
| `AdvancedDisplay` | In the collapsed "advanced" section |
| `SimpleDisplay` | Always visible |
| `Interp` | Animatable in Sequencer |
| `EditFixedSize` | Array size cannot change in the editor |
| `AssetRegistrySearchable` | Value is added as an Asset Registry tag (searchable without loading) |

### UPROPERTY meta keys (commonly used, verified in engine code)

| Meta | Effect |
|---|---|
| `ClampMin = "0"`, `ClampMax = "1"` | Hard clamp in the editor (not enforced at runtime) |
| `UIMin`, `UIMax` | Slider range only |
| `Units = "cm"` (also `"s"`, `"deg"`, `"kg"`, `"Percent"`...) | Displays units |
| `ForceUnits = "cm"` | Show in exactly these units |
| `EditCondition = "bUseX"` | Editable only when the bool/expression is true (5.x supports expressions like `"Mode == EMode::A"`) |
| `EditConditionHides` | Hide rather than grey out when EditCondition is false |
| `InlineEditConditionToggle` | On the bool: show it as a checkbox next to the dependent property |
| `DisplayName = "Nice Name"` | Details/BP display name |
| `ToolTip = "..."` | Tooltip (the `//` or `/** */` comment above the property is used by default) |
| `AllowPrivateAccess = "true"` | Allow Blueprint access to private members |
| `ExposeOnSpawn = "true"` | Appears as a pin on SpawnActor / Construct Object / Create Widget (needs BlueprintReadWrite or ReadOnly + Editable) |
| `MakeEditWidget = "true"` | 3D widget in the viewport for FVector/FTransform |
| `AllowedClasses = "/Script/Engine.StaticMesh"` | Restrict asset picker (FSoftObjectPath / object) |
| `MetaClass = "/Script/Engine.Actor"` | Base class for `FSoftClassPath` pickers |
| `MustImplement = "/Script/MyGame.Interactable"` | Class picker requires the interface |
| `GetOptions = "FuncName"` | Dropdown from a UFUNCTION returning `TArray<FString>` or `TArray<FName>` |
| `TitleProperty = "Name"` | Array of structs shows this member as element title |
| `RowType = "/Script/MyGame.ItemRow"` | Restrict `FDataTableRowHandle` to one row struct |
| `Bitmask`, `BitmaskEnum = "/Script/MyGame.EMyFlags"` | Int property edited as flags |
| `BindWidget` / `BindWidgetOptional` | UMG: bind to a same-named widget in the Widget Blueprint |
| `BindWidgetAnim` | UMG: bind a `UWidgetAnimation*` (Transient) |
| `ShowOnlyInnerProperties` | Flatten a struct into the parent category |
| `Categories = "Tag.Parent"` | Filter a `FGameplayTag`/container picker |
| `DeprecatedProperty`, `DeprecationMessage` | Mark deprecated |
| `NoResetToDefault` | Hide the reset arrow |
| `MultiLine = "true"` | Multi-line text box for FString/FText |

## UFUNCTION

| Specifier | Effect |
|---|---|
| `BlueprintCallable` | Callable from Blueprint, exec pins |
| `BlueprintPure` | No exec pins; must not change state; re-evaluated for each connected pin. `BlueprintPure = false` on a const function forces exec pins |
| `BlueprintImplementableEvent` | Declared in C++, implemented only in Blueprint; no C++ body. Calling it with no BP implementation does nothing (returns default) |
| `BlueprintNativeEvent` | C++ default in `Name_Implementation`, Blueprint can override (and call Parent) |
| `BlueprintAuthorityOnly` | Only runs with network authority |
| `BlueprintCosmetic` | Does not run on dedicated servers |
| `CallInEditor` | Button in the Details panel of the object/actor (editor only use) |
| `Exec` | Console command (works on PlayerController, Pawn, HUD, GameMode, CheatManager, GameInstance) |
| `Category = "..."` | Palette category |
| `Server` / `Client` / `NetMulticast` | RPC; implement `Name_Implementation` |
| `Reliable` / `Unreliable` | RPC reliability (required on RPCs) |
| `WithValidation` | Also implement `bool Name_Validate(...)`; false disconnects the caller |
| `SealedEvent` | Cannot be overridden (events) |

Notes: `BlueprintCallable`/`BlueprintPure` functions on actors may be `const`. Pass structs as
`const FFoo&`; non-const reference parameters become **output pins** in Blueprint unless marked
`UPARAM(ref)`. `UPARAM(DisplayName = "X")` renames a pin.

### UFUNCTION meta keys

| Meta | Effect |
|---|---|
| `DisplayName = "..."` | Node title |
| `Keywords = "a b"` | Extra search terms |
| `CompactNodeTitle = "+"` | Compact node look |
| `ToolTip` | Tooltip |
| `WorldContext = "WorldContextObject"` | Parameter auto-filled with the caller's world context (for static library functions) |
| `DefaultToSelf = "Target"` | Parameter defaults to self |
| `HidePin = "Param"` | Hide a pin |
| `AdvancedDisplay = "Param1,Param2"` or `= "2"` | Collapse pins |
| `AutoCreateRefTerm = "Param"` | Allow a const-ref pin to be unconnected |
| `ExpandEnumAsExecs = "Result"` | Turn an enum param/return into exec outputs (also `ExpandBoolAsExecs`, 5.x) |
| `DeterminesOutputType = "Class"` | Return type follows a class param (e.g. typed Get Actor Of Class) |
| `DynamicOutputParam = "OutActors"` | Which output uses DeterminesOutputType |
| `ReturnDisplayName = "Success"` | Rename the return pin |
| `DevelopmentOnly` | Node does nothing in shipping |
| `BlueprintProtected` | Callable only from the owning Blueprint |
| `BlueprintInternalUseOnly = "true"` | Hidden from palette (used by async action nodes) |
| `Latent`, `LatentInfo = "LatentInfo"` | Latent node with `FLatentActionInfo` parameter |
| `UnsafeDuringActorConstruction` | Not callable in construction script |
| `DeprecatedFunction`, `DeprecationMessage` | Mark deprecated |
| `CallableWithoutWorldContext` | With WorldContext, still callable where no world context exists |

## UCLASS

| Specifier | Effect |
|---|---|
| `Blueprintable` / `NotBlueprintable` | Can/cannot be a Blueprint parent (inherited) |
| `BlueprintType` / `NotBlueprintType` | Usable as a Blueprint variable type |
| `Abstract` | Cannot be instantiated or placed; shown as abstract in pickers |
| `Const` | Properties/functions are const in Blueprint |
| `Config = Game` (or `Engine`, `Input`, `Editor`, `EditorPerProjectUserSettings`) | Config file for `UPROPERTY(Config)` |
| `DefaultConfig` | Save to `Default<Config>.ini` instead of Saved |
| `GlobalUserConfig`, `ProjectUserConfig` | Save to the user/project-user config |
| `Transient` | Never saved |
| `MinimalAPI` | Export only type info (cast works), not functions; smaller DLL |
| `EditInlineNew` | Can be created inline in an `Instanced` property |
| `DefaultToInstanced` | All properties of this type are instanced |
| `Within = OuterClass` | Outer must be of this class |
| `HideCategories = (A, B)` / `ShowCategories = (...)` | Details panel categories |
| `HideDropdown` | Hidden from class pickers |
| `ClassGroup = (Name)` | Group in the Add Component menu (with `BlueprintSpawnableComponent`) |
| `Placeable` / `NotPlaceable` | Can be placed in levels (actors) |
| `Deprecated` | Class is deprecated (rename to `UDEPRECATED_Name`) |
| `CollapseCategories`, `DontCollapseCategories` | Details layout |
| `ConversionRoot` | Actor conversion root in the editor |

UCLASS meta: `BlueprintSpawnableComponent` (component appears in Add Component),
`DisplayName`, `ShortTooltip`, `IsBlueprintBase = "true"/"false"`, `ChildCanTick`,
`ChildCannotTick`, `DisplayThumbnail`, `PrioritizeCategories = "A B"` (5.x).

## USTRUCT

| Specifier | Effect |
|---|---|
| `BlueprintType` | Usable in Blueprint (variables, pins, Make/Break nodes) |
| `Atomic` | Always serialized as a single unit |
| `NoExport` | UHT does not generate the struct's C++ (engine math types) |
| `Immutable` | Only legal in `Object.h`; do not use |

Meta: `HasNativeMake = "/Script/Module.Library.MakeFunc"`, `HasNativeBreak = "..."`,
`DisableSplitPin`. A struct used as a Data Table row must derive from `FTableRowBase`.
USTRUCTs cannot have `UFUNCTION`s; expose helpers via a `UBlueprintFunctionLibrary`.

## UENUM

```cpp
UENUM(BlueprintType)
enum class EWeaponType : uint8
{
    None     UMETA(Hidden),
    Rifle    UMETA(DisplayName = "Assault Rifle"),
    Shotgun,
    MAX      UMETA(Hidden)
};

UENUM(BlueprintType, meta = (Bitflags, UseEnumValuesAsMaskValuesInEditor = "true"))
enum class EDamageFlags : uint8
{
    None     = 0 UMETA(Hidden),
    Fire     = 1 << 0,
    Poison   = 1 << 1,
};
ENUM_CLASS_FLAGS(EDamageFlags);
```
- Blueprint-exposed enums must be `uint8`.
- `UMETA(DisplayName = "...")`, `UMETA(Hidden)`, `UMETA(ToolTip = "...")` per value.
- String conversion: `UEnum::GetValueAsString(Value)`, `StaticEnum<EWeaponType>()->GetDisplayNameTextByValue((int64)Value)`.
- Bitmask property: `UPROPERTY(EditAnywhere, meta = (Bitmask, BitmaskEnum = "/Script/MyGame.EDamageFlags")) uint8 Flags;`
  (older code uses the enum name without path; the path form is preferred in 5.x).

## UINTERFACE

`UINTERFACE(MinimalAPI, Blueprintable)` on the `U` class, `BlueprintType` to use as a variable
type, `meta = (CannotImplementInterfaceInBlueprint)` for C++-only interfaces. Functions are
declared on the `I` class. See the Interfaces section of `SKILL.md`.

## UPARAM

`UPARAM(ref)` makes a non-const reference parameter an input pin (passed by reference).
`UPARAM(DisplayName = "Out Value")` renames a pin. `UPARAM(meta = (Categories = "Tag"))` filters a
gameplay tag pin.

## Supported UPROPERTY types (quick check)

Supported: `bool` (or `uint8 bFlag : 1`), `uint8`, `int32`, `int64`, `float`, `double`, `FString`,
`FName`, `FText`, `enum class : uint8` with UENUM, USTRUCTs, `TObjectPtr<U>`, `U*` (legacy),
`TSubclassOf<U>`, `TSoftObjectPtr<U>`, `TSoftClassPtr<U>`, `TWeakObjectPtr<U>`, `TScriptInterface<I>`,
`TArray`, `TMap`, `TSet` (one level), `FSoftObjectPath`, `FGameplayTag`, dynamic delegates.
Blueprint-exposed integer types: `uint8` (Byte), `int32`, `int64`; `double` shows as "Float (double
precision)" in UE5.

Not supported: `TSharedPtr`, `TUniquePtr`, raw non-UObject pointers, `std::` types, nested
containers, `TOptional` (5.x has limited support; avoid), unsigned 16/32-bit ints in Blueprint.
