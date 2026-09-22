# Balancing, Economy and the Data Pipeline

## 1. Pipeline: spreadsheet → CSV → UE asset

Keep the source of truth as text:

```
Content/Data/Source/Weapons.csv         <- exported from the spreadsheet; committed; diffable
Content/Data/Source/WeaponScaling.csv
/Game/Data/DT_Weapons      (UDataTable,  row struct FWeaponRow)
/Game/Data/CT_WeaponScaling (UCurveTable)
```

With CSVs under `Content/`, the editor's Auto Reimport (Editor Preferences > General > Loading &
Saving) may offer to import new files on its own. A folder outside `Content/`, such as `Data/Source/`,
avoids that prompt. Either way, keep the path stable, because an asset remembers its source path for *Reimport*.

### Data Table row struct

```cpp
// WeaponRow.h   (module deps: "Engine", "GameplayTags")
#pragma once
#include "CoreMinimal.h"
#include "Engine/DataTable.h"
#include "GameplayTagContainer.h"
#include "WeaponRow.generated.h"

UENUM(BlueprintType)
enum class EItemRarity : uint8 { Common, Rare, Epic, Legendary };

USTRUCT(BlueprintType)
struct FWeaponRow : public FTableRowBase
{
	GENERATED_BODY()

	UPROPERTY(EditAnywhere, BlueprintReadOnly) FText DisplayName;
	UPROPERTY(EditAnywhere, BlueprintReadOnly) EItemRarity Rarity = EItemRarity::Common;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, meta = (ClampMin = "0")) float BaseDamage = 10.f;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, meta = (ClampMin = "0.01")) float FireInterval = 0.2f;
	UPROPERTY(EditAnywhere, BlueprintReadOnly) int32 Price = 100;
	UPROPERTY(EditAnywhere, BlueprintReadOnly) FGameplayTagContainer Tags;
	UPROPERTY(EditAnywhere, BlueprintReadOnly) TSoftObjectPtr<UStaticMesh> Mesh;
};
```

### Matching CSV

The first column is the row name, and its header text is ignored (UE exports it as `---`). The other
headers must match the property names, **as C++ names** (`BaseDamage`, not `base_damage`).
Struct values use UE's text-import format:

```csv
---,DisplayName,Rarity,BaseDamage,FireInterval,Price,Tags,Mesh
Rifle_Basic,"Service Rifle",Common,12,0.12,150,"(GameplayTags=((TagName=""Weapon.Type.Rifle"")))",/Game/Weapons/Rifle/SM_Rifle.SM_Rifle
Shotgun_Basic,"Breacher",Rare,8,0.8,300,"(GameplayTags=((TagName=""Weapon.Type.Shotgun"")))",/Game/Weapons/Shotgun/SM_Shotgun.SM_Shotgun
```

Localization note: a plain string in an `FText` column gets a generated localization key. For
shipped player-facing text, prefer String Table references (see `unreal-narrative`).

Using rows in code:

```cpp
UPROPERTY(EditDefaultsOnly) FDataTableRowHandle WeaponRow;   // picker: table + row name

if (const FWeaponRow* Row = WeaponRow.GetRow<FWeaponRow>(TEXT("Equip")))
{
	FireInterval = Row->FireInterval;
}
// or directly: Table->FindRow<FWeaponRow>(RowName, TEXT("Context"), /*bWarnIfRowMissing*/ true);
```

### Curve Table CSV

The first row holds the X keys (level, zone, time). Each following row is one curve:

```csv
Name,1,2,3,4,5,10,20
Weapon.Rifle.Damage,12,13,14.5,16,18,26,45
Weapon.Shotgun.Damage,8,8.6,9.4,10.3,11.3,16,27
Enemy.Grunt.HP,100,115,132,152,175,350,1000
```

Evaluate it with `FCurveTableRowHandle::Eval(X, Context)`, or the Blueprint node *Evaluate Curve Table
Row*. Interpolation mode (constant, linear or cubic) is chosen at import. Use **linear** for
stats, and **constant** for step functions (unlock tiers).

### Importing and reimporting from Python (editor open)

```python
import unreal

def import_csv(csv_path, dest_path, asset_name, row_struct_path=None, curve_table=False):
    task = unreal.AssetImportTask()
    task.set_editor_property("filename", csv_path)
    task.set_editor_property("destination_path", dest_path)
    task.set_editor_property("destination_name", asset_name)
    task.set_editor_property("replace_existing", True)
    task.set_editor_property("automated", True)      # no dialog
    task.set_editor_property("save", True)

    factory = unreal.CSVImportFactory()
    settings = factory.get_editor_property("automated_import_settings")
    if curve_table:
        settings.set_editor_property("import_type", unreal.CSVImportType.ECSV_CURVE_TABLE)
        settings.set_editor_property("import_curve_interp_mode", unreal.RichCurveInterpMode.RCIM_LINEAR)
    else:
        # USTRUCT FWeaponRow in module MyGame -> "/Script/MyGame.WeaponRow"
        settings.set_editor_property("import_row_struct", unreal.load_object(None, row_struct_path))
    factory.set_editor_property("automated_import_settings", settings)
    task.set_editor_property("factory", factory)

    unreal.AssetToolsHelpers.get_asset_tools().import_asset_tasks([task])
    print("imported:", task.get_editor_property("imported_object_paths"))

src = unreal.Paths.convert_relative_path_to_full(unreal.Paths.project_content_dir()) + "Data/Source/"
import_csv(src + "Weapons.csv", "/Game/Data", "DT_Weapons", row_struct_path="/Script/MyGame.WeaponRow")
import_csv(src + "WeaponScaling.csv", "/Game/Data", "CT_WeaponScaling", curve_table=True)
```

If an enum or property name differs in your engine version, check it with
`help(unreal.CSVImportFactory)` and `help(unreal.CSVImportSettings)`. To refill an *existing* Data
Table from a CSV without the import task, use
`unreal.DataTableFunctionLibrary.fill_data_table_from_csv_file(table, path)` (editor only), and then
save it with `unreal.EditorAssetLibrary.save_loaded_asset(table)`.

To read table contents back out (for checks):

```python
dt = unreal.EditorAssetLibrary.load_asset("/Game/Data/DT_Weapons")
names = unreal.DataTableFunctionLibrary.get_data_table_row_names(dt)
dmg = unreal.DataTableFunctionLibrary.get_data_table_column_as_string(dt, "BaseDamage")
for n, d in zip(names, dmg):
    print(n, d)
```

### Validate data at save time (C++)

```cpp
#if WITH_EDITOR
#include "Misc/DataValidation.h"
EDataValidationResult UWeaponDefinition::IsDataValid(FDataValidationContext& Context) const
{
	EDataValidationResult Result = Super::IsDataValid(Context);
	if (FireInterval <= 0.f)
	{
		Context.AddError(FText::FromString(TEXT("FireInterval must be > 0")));
		Result = EDataValidationResult::Invalid;
	}
	return Result;
}
#endif
```

Declare it in the header inside `#if WITH_EDITOR` as
`virtual EDataValidationResult IsDataValid(FDataValidationContext& Context) const override;`
(this is the 5.3+ signature. Older versions used `TArray<FText>&`). Run it from the Content Browser
with a right-click, then *Asset Actions > Validate Assets*.

## 2. Offline sanity checks (plain Python, no editor)

Run these on the CSVs before importing them. They are cheap, and they catch most typos.

```python
import csv, sys
def load_curves(path):
    with open(path, newline="", encoding="utf-8") as f:
        rows = list(csv.reader(f))
    xs = [float(x) for x in rows[0][1:]]
    return xs, {r[0]: [float(v) for v in r[1:]] for r in rows[1:] if r}

xs, curves = load_curves("Content/Data/Source/WeaponScaling.csv")
assert xs == sorted(xs), "X keys must increase"
for name, ys in curves.items():
    if name.endswith(".HP") or name.endswith(".Damage"):
        bad = [(xs[i], ys[i]) for i in range(1, len(ys)) if ys[i] < ys[i - 1]]
        if bad: print(f"{name}: non-monotonic at {bad}", file=sys.stderr)
```

## 3. Core formulas

Pick one formula per concept, implement it in exactly one function, and keep its constants in data.

**Progression (XP to reach the next level `n`)**
- Polynomial: `XP(n) = A * n^k`, with k between 1.5 and 2.5. The feel stays steady and grows gently.
- Exponential: `XP(n) = A * g^(n-1)`, with g between 1.08 and 1.2. The curve walls off fast, so cap it or pair it with prestige.
- Aim for the time per level (XP needed divided by XP earned per minute at that level) to grow slowly, for example
  from 5 minutes to 20 minutes across the game. Plot that, not the raw XP.

**Combat**
- `DPS = DamagePerHit * HitsPerSecond * Accuracy * (1 + CritChance * (CritMult - 1))`
- `ShotsToKill = ceil(TargetHP / DamagePerHit)`
- `TTK = (ShotsToKill - 1) * FireInterval` (plus reload time if the magazine is smaller than `ShotsToKill`)
- Armor with diminishing returns: `Damage * K / (K + Armor)`, where K is about 100. It never reaches 0.
- Effective HP: `HP / (1 - Mitigation)`.
- Enemy HP scaling per zone should track expected player DPS per zone, and TTK should stay in the
  designed band (for example 0.5–1.5 s for fodder, and 30–90 s for bosses).

**Economy**
- For each currency, tabulate the **sources** (drops, quests, selling) and the **sinks** (purchases,
  repairs, fees, crafting) as *per-hour* rates at each stage of the game.
- Price an item tier as roughly `hours of play you want it to cost * net income per hour at that stage`.
- Inflation check: if the net rate stays positive with no late-game sinks, currency loses meaning.
  Add sinks that scale (upgrades whose costs rise, cosmetic sinks).
- Drop tables: store weights, not percentages (`Weight / SumOfWeights`). Use pity timers (a guarantee after N
  misses) for rare items that the progression depends on.

## 4. Balancing workflow

1. Write the *intended* numbers first (target TTK, minutes per level, hours to afford tier 3).
2. Derive the data from those targets with the formulas above, in the spreadsheet.
3. Import, playtest, and collect telemetry (next section).
4. Compare actual against intended. Change **one variable at a time**, re-test, and log the change in the decision log.
5. Use outliers to spot dominant strategies: an item or weapon with a usage share far above its peers.

## 5. Telemetry subsystem (JSON lines)

```cpp
// TelemetrySubsystem.h   (module deps: "Json")
#pragma once
#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "TelemetrySubsystem.generated.h"

UCLASS()
class MYGAME_API UTelemetrySubsystem : public UGameInstanceSubsystem
{
	GENERATED_BODY()
public:
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;

	UFUNCTION(BlueprintCallable, Category = "Telemetry")
	void LogEvent(FName EventName, const TMap<FString, FString>& Properties, FVector Location);

private:
	FString SessionId;
	FString FilePath;
};

// TelemetrySubsystem.cpp
#include "TelemetrySubsystem.h"
#include "Dom/JsonObject.h"
#include "Serialization/JsonWriter.h"
#include "Serialization/JsonSerializer.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#include "HAL/FileManager.h"

void UTelemetrySubsystem::Initialize(FSubsystemCollectionBase& Collection)
{
	Super::Initialize(Collection);
	SessionId = FGuid::NewGuid().ToString(EGuidFormats::Digits);
	FilePath = FPaths::ProjectSavedDir() / TEXT("Telemetry") / (SessionId + TEXT(".jsonl"));
}

void UTelemetrySubsystem::LogEvent(FName EventName, const TMap<FString, FString>& Properties, FVector Location)
{
	TSharedRef<FJsonObject> Obj = MakeShared<FJsonObject>();
	Obj->SetStringField(TEXT("event"), EventName.ToString());
	Obj->SetStringField(TEXT("session"), SessionId);
	Obj->SetStringField(TEXT("utc"), FDateTime::UtcNow().ToIso8601());
	if (const UWorld* World = GetGameInstance()->GetWorld())
	{
		Obj->SetStringField(TEXT("map"), World->GetMapName());
		Obj->SetNumberField(TEXT("t"), World->GetTimeSeconds());
	}
	Obj->SetNumberField(TEXT("x"), Location.X);
	Obj->SetNumberField(TEXT("y"), Location.Y);
	Obj->SetNumberField(TEXT("z"), Location.Z);
	for (const TPair<FString, FString>& P : Properties) { Obj->SetStringField(P.Key, P.Value); }

	FString Line;
	TSharedRef<TJsonWriter<TCHAR, TCondensedJsonPrintPolicy<TCHAR>>> Writer =
		TJsonWriterFactory<TCHAR, TCondensedJsonPrintPolicy<TCHAR>>::Create(&Line);
	FJsonSerializer::Serialize(Obj, Writer);
	Line += TEXT("\n");
	FFileHelper::SaveStringToFile(Line, *FilePath, FFileHelper::EEncodingOptions::ForceUTF8WithoutBOM,
		&IFileManager::Get(), FILEWRITE_Append);
}
```

Log these events: `level_start`, `level_end` (duration, result), `death` (cause, killer, location),
`checkpoint`, `item_acquired` and `item_spent` (currency and amount), `ability_used`, `option_changed`,
and `quit` (location). Analyze them offline with Python or pandas. To make a death heatmap, plot the (x, y) positions
over a top-down high-resolution screenshot of the map at a known scale. For a shipped game, replace the file
sink with the project's analytics backend (UE ships an `Analytics` module interface, `IAnalyticsProvider`).
Tell players, and respect platform privacy rules.
