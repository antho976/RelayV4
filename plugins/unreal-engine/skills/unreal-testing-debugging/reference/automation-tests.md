# Automation test templates

UE 5.3-5.6. All C++ tests go in `.cpp` files inside `#if WITH_DEV_AUTOMATION_TESTS`.
Includes: `Misc/AutomationTest.h` (framework), `Tests/AutomationCommon.h` (latent helpers
such as `FEngineWaitLatentCommand` and `AutomationOpenMap`; module `Engine`).
Verify signatures against these headers in the engine you build with.

## Module setup

Tests can live in the game module. For a larger suite, a dedicated module keeps test-only
dependencies out of the game:
```json
// .uproject "Modules"
{ "Name": "MyGameTests", "Type": "DeveloperTool", "LoadingPhase": "Default" }
```
```csharp
// Source/MyGameTests/MyGameTests.Build.cs
public class MyGameTests : ModuleRules
{
    public MyGameTests(ReadOnlyTargetRules Target) : base(Target)
    {
        PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;
        PrivateDependencyModuleNames.AddRange(new string[] {
            "Core", "CoreUObject", "Engine", "MyGame", "FunctionalTesting" });
    }
}
```
Add `IMPLEMENT_MODULE(FDefaultModuleImpl, MyGameTests)` in one `.cpp`, and add the module to
the editor target's `ExtraModuleNames` if it is not otherwise pulled in. Classes used
from `MyGame` need `MYGAME_API`.

## Simple test

```cpp
#include "Misc/AutomationTest.h"
#if WITH_DEV_AUTOMATION_TESTS

IMPLEMENT_SIMPLE_AUTOMATION_TEST(FDamageFormulaTest, "MyGame.Combat.DamageFormula",
    EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)

bool FDamageFormulaTest::RunTest(const FString& Parameters)
{
    UTEST_EQUAL(TEXT("Base damage"), CombatMath::ComputeDamage(10.f, 0.f), 10.f);
    TestEqual(TEXT("Armor halves damage"), CombatMath::ComputeDamage(10.f, 100.f), 5.f, KINDA_SMALL_NUMBER);
    TestTrue(TEXT("Never negative"), CombatMath::ComputeDamage(1.f, 1000.f) >= 0.f);
    return true;
}
#endif
```

## Complex (parameterized) test

One test entry per item returned by `GetTests`; `RunTest` receives the command string.
```cpp
IMPLEMENT_COMPLEX_AUTOMATION_TEST(FLoadEveryMapTest, "MyGame.Content.LoadEveryMap",
    EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)

void FLoadEveryMapTest::GetTests(TArray<FString>& OutBeautifiedNames, TArray<FString>& OutTestCommands) const
{
    const TArray<FString> Maps = { TEXT("/Game/Maps/MainMenu"), TEXT("/Game/Maps/Level01") };
    for (const FString& Map : Maps)
    {
        OutBeautifiedNames.Add(FPackageName::GetShortName(Map));
        OutTestCommands.Add(Map);
    }
}

bool FLoadEveryMapTest::RunTest(const FString& Parameters)
{
    AutomationOpenMap(Parameters);                                  // Tests/AutomationCommon.h
    ADD_LATENT_AUTOMATION_COMMAND(FEngineWaitLatentCommand(2.0f));  // let it tick
    return true;
}
```
In a real project, build the map list from the Asset Registry instead of hard-coding it.

## Latent commands

Latent commands run across frames after `RunTest` returns; `Update()` returns `true` when done.
```cpp
#include "Misc/AutomationTest.h"
#include "Tests/AutomationCommon.h"
#include "Kismet/GameplayStatics.h"

DEFINE_LATENT_AUTOMATION_COMMAND_ONE_PARAMETER(FWaitForActorsOfClass, TSubclassOf<AActor>, ActorClass);

bool FWaitForActorsOfClass::Update()
{
    UWorld* World = GEngine->GetWorldContexts()[0].World();   // prefer a PIE/Game context lookup in real code
    TArray<AActor*> Found;
    UGameplayStatics::GetAllActorsOfClass(World, ActorClass, Found);
    return Found.Num() > 0;
}
```
Queue with `ADD_LATENT_AUTOMATION_COMMAND(FWaitForActorsOfClass(AEnemy::StaticClass()));`.
Other helpers in `AutomationCommon.h` include `FEngineWaitLatentCommand(Seconds)` and
`FWaitForMapToLoadCommand`. Latent commands have no built-in timeout: add one yourself
(track elapsed time and `AddError` + return `true`).

## Spec with latent steps

```cpp
#include "Misc/AutomationTest.h"
#include "Kismet/GameplayStatics.h"
#include "MySaveGame.h"
#if WITH_DEV_AUTOMATION_TESTS

BEGIN_DEFINE_SPEC(FSaveGameSpec, "MyGame.Persistence.SaveGame",
    EAutomationTestFlags::EditorContext | EAutomationTestFlags::ProductFilter)
    FString Slot;
END_DEFINE_SPEC(FSaveGameSpec)

void FSaveGameSpec::Define()
{
    BeforeEach([this]() { Slot = TEXT("AutomationSlot"); });

    Describe("Round trip", [this]()
    {
        It("saves and loads the player level", [this]()
        {
            UMySaveGame* Out = Cast<UMySaveGame>(UGameplayStatics::CreateSaveGameObject(UMySaveGame::StaticClass()));
            Out->PlayerLevel = 7;
            TestTrue(TEXT("Saved"), UGameplayStatics::SaveGameToSlot(Out, Slot, 0));
            UMySaveGame* In = Cast<UMySaveGame>(UGameplayStatics::LoadGameFromSlot(Slot, 0));
            if (TestNotNull(TEXT("Loaded"), In)) { TestEqual(TEXT("Level"), In->PlayerLevel, 7); }
        });

        LatentIt("loads asynchronously", [this](const FDoneDelegate& Done)
        {
            UGameplayStatics::AsyncLoadGameFromSlot(Slot, 0,
                FAsyncLoadGameFromSlotDelegate::CreateLambda([this, Done](const FString&, const int32, USaveGame* Game)
                {
                    TestNotNull(TEXT("Async loaded"), Game);
                    Done.Execute();
                }));
        });
    });

    AfterEach([this]() { UGameplayStatics::DeleteGameInSlot(Slot, 0); });
}
#endif
```
Spec API: `Describe`, `It`, `BeforeEach`, `AfterEach`, `LatentIt`, `LatentBeforeEach`,
`LatentAfterEach`, and disabled variants prefixed with `x`. `DEFINE_SPEC(Name, "Path", Flags)`
is the short form when the spec has no member variables.

## A world inside a non-map test

For logic that needs actors but not a specific map:
```cpp
UWorld* World = UWorld::CreateWorld(EWorldType::Game, false);
FWorldContext& Ctx = GEngine->CreateNewWorldContext(EWorldType::Game);
Ctx.SetCurrentWorld(World);
World->InitializeActorsForPlay(FURL());
World->BeginPlay();

AMyPickup* Pickup = World->SpawnActor<AMyPickup>();
TestNotNull(TEXT("Spawned"), Pickup);

GEngine->DestroyWorldContext(World);
World->DestroyWorld(false);
```
Ticking this world requires calling `World->Tick(LEVELTICK_All, DeltaSeconds)` yourself.
For anything more involved, prefer a functional test map.

## Functional test (C++)

```cpp
// FTest_DoorOpens.h - requires the FunctionalTesting module (Developer; not in Shipping)
#pragma once
#include "FunctionalTest.h"
#include "Door.h"
#include "FTest_DoorOpens.generated.h"

UCLASS()
class AFTest_DoorOpens : public AFunctionalTest
{
    GENERATED_BODY()
protected:
    virtual void StartTest() override;

    UPROPERTY(EditInstanceOnly, Category = "Test")
    TObjectPtr<ADoor> Door;

    FTimerHandle CheckHandle;
};
```
```cpp
// FTest_DoorOpens.cpp
#include "FTest_DoorOpens.h"
#include "TimerManager.h"

void AFTest_DoorOpens::StartTest()
{
    Super::StartTest();
    if (!Door)
    {
        FinishTest(EFunctionalTestResult::Failed, TEXT("No Door assigned in the test map"));
        return;
    }
    Door->Open();
    GetWorldTimerManager().SetTimer(CheckHandle, [this]()
    {
        const bool bOpen = Door->IsOpen();
        FinishTest(bOpen ? EFunctionalTestResult::Succeeded : EFunctionalTestResult::Failed,
                   bOpen ? TEXT("Door opened") : TEXT("Door still closed after 1s"));
    }, 1.0f, false);
}
```
Place it in a test map, assign `Door`, set the actor's **Time Limit** (fails on timeout),
save the map (through `unreal-editor-automation` or the human). `AFunctionalTest` also has
Blueprint-callable assertion helpers (search `FunctionalTest.h` for `Assert`), which log
failures without finishing the test.

Blueprint route (no C++ module needed): create a Blueprint subclass of *Functional Test*,
implement *Event On Test Start*, do the work, then call *Finish Test*.

## Running

| Where | How |
|---|---|
| Editor UI | Tools > Test Automation (or Session Frontend > Automation); filter by name; Start Tests |
| Live editor via MCP | `ue_console {"command": "Automation RunTests MyGame.Combat"}` then `ue_log {"filter": "LogAutomation"}` |
| Headless | see the command below |

```
UnrealEditor-Cmd "<abs>/MyGame.uproject" ^
  -ExecCmds="Automation RunTests MyGame+Project.Functional Tests;Quit" ^
  -unattended -nullrhi -nopause -nosplash -log ^
  -TestExit="Automation Test Queue Empty" ^
  -ReportExportPath="<abs>/Saved/Automation/Reports"
```
- `-nullrhi` disables rendering; drop it for tests that capture screenshots or need the GPU.
- `Automation List` logs every registered test; `Automation RunAll` runs everything
  (slow; includes engine tests from enabled plugins).
- Output: `<ReportExportPath>/index.json` summarising succeeded/failed/not-run counts and
  per-test entries with errors, plus an HTML viewer. Fail CI when the failed count is
  non-zero or the process exit code is non-zero.
- On Linux and Mac run the `UnrealEditor` binary with the same arguments.

## Gauntlet

Gauntlet (`RunUAT RunUnreal`) is the framework for tests that launch processes: packaged
build boot tests, client/server sessions, device farms. It runs C# test nodes from
`Engine/Source/Programs/AutomationTool/Gauntlet` and project test nodes you add.
Shape of a run:
```
RunUAT RunUnreal -project="<abs>/MyGame.uproject" -platform=Win64 -configuration=Development ^
  -build="<abs>/Saved/StagedBuilds/Windows" -test=<TestNodeName>
```
Look up available test node names in the engine source (search for classes deriving
from `UnrealTestNode`) rather than guessing them.

## Test design rules

- Deterministic: fixed seeds (`FRandomStream`), no reliance on frame timing without a
  timeout, no dependence on test order.
- Fast tests (`SmokeFilter`) run on every change; slow content tests (`ProductFilter`,
  `StressFilter`) in nightly CI.
- Clean up: delete save slots, destroy spawned actors, remove temp assets.
- A test that loads assets by path must use assets that are committed, not local-only.
