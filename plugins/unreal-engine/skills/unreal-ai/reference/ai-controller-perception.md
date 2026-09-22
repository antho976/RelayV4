# AI controller, perception, teams and EQS (C++)

Read with `behavior-tree-cpp.md` (which defines the `MyAIKeys` blackboard key names used here).
Build.cs: `"AIModule"`, `"GameplayTasks"`, `"NavigationSystem"`.

## 1. AI controller with perception, teams and crowd following

```cpp
// MyAIController.h
#pragma once
#include "CoreMinimal.h"
#include "AIController.h"
#include "Perception/AIPerceptionTypes.h"
#include "MyAIController.generated.h"

class UBehaviorTree;
class UAISenseConfig_Sight;
class UAISenseConfig_Hearing;

UCLASS()
class MYGAME_API AMyAIController : public AAIController
{
    GENERATED_BODY()
public:
    AMyAIController(const FObjectInitializer& ObjectInitializer = FObjectInitializer::Get());

protected:
    virtual void OnPossess(APawn* InPawn) override;

    UFUNCTION()
    void HandleTargetPerceptionUpdated(AActor* Actor, FAIStimulus Stimulus);

    UPROPERTY(EditDefaultsOnly, Category = "AI") TObjectPtr<UBehaviorTree> BehaviorTree;
    UPROPERTY(VisibleAnywhere, Category = "AI") TObjectPtr<UAISenseConfig_Sight> SightConfig;
    UPROPERTY(VisibleAnywhere, Category = "AI") TObjectPtr<UAISenseConfig_Hearing> HearingConfig;
};
```

```cpp
// MyAIController.cpp
#include "MyAIController.h"
#include "MyAIKeys.h"
#include "BehaviorTree/BehaviorTree.h"
#include "BehaviorTree/BlackboardComponent.h"
#include "Navigation/CrowdFollowingComponent.h"
#include "Perception/AIPerceptionComponent.h"
#include "Perception/AISenseConfig_Sight.h"
#include "Perception/AISenseConfig_Hearing.h"
#include "Perception/AISense_Sight.h"
#include "Perception/AISense_Hearing.h"

AMyAIController::AMyAIController(const FObjectInitializer& ObjectInitializer)
    // Detour crowd avoidance instead of plain path following. Remove this line to keep the default.
    : Super(ObjectInitializer.SetDefaultSubobjectClass<UCrowdFollowingComponent>(TEXT("PathFollowingComponent")))
{
    SetPerceptionComponent(*CreateDefaultSubobject<UAIPerceptionComponent>(TEXT("Perception")));

    SightConfig = CreateDefaultSubobject<UAISenseConfig_Sight>(TEXT("SightConfig"));
    SightConfig->SightRadius = 2000.f;
    SightConfig->LoseSightRadius = 2500.f;              // must exceed SightRadius
    SightConfig->PeripheralVisionAngleDegrees = 70.f;   // half angle
    SightConfig->DetectionByAffiliation.bDetectEnemies = true;
    SightConfig->DetectionByAffiliation.bDetectNeutrals = false;
    SightConfig->DetectionByAffiliation.bDetectFriendlies = false;
    SightConfig->SetMaxAge(5.f);                        // stimulus forgotten after 5 s

    HearingConfig = CreateDefaultSubobject<UAISenseConfig_Hearing>(TEXT("HearingConfig"));
    HearingConfig->HearingRange = 1500.f;
    HearingConfig->DetectionByAffiliation.bDetectEnemies = true;
    HearingConfig->DetectionByAffiliation.bDetectNeutrals = true;
    HearingConfig->SetMaxAge(3.f);

    UAIPerceptionComponent* Perception = GetPerceptionComponent();
    Perception->ConfigureSense(*SightConfig);
    Perception->ConfigureSense(*HearingConfig);
    Perception->SetDominantSense(SightConfig->GetSenseImplementation());
    Perception->OnTargetPerceptionUpdated.AddDynamic(this, &AMyAIController::HandleTargetPerceptionUpdated);

    SetGenericTeamId(FGenericTeamId(1)); // enemies = 1; the player pawn reports 0
}

void AMyAIController::OnPossess(APawn* InPawn)
{
    Super::OnPossess(InPawn);
    if (BehaviorTree && RunBehaviorTree(BehaviorTree))   // also creates/initializes the blackboard
    {
        GetBlackboardComponent()->SetValueAsVector(MyAIKeys::HomeLocation, InPawn->GetActorLocation());
    }
}

void AMyAIController::HandleTargetPerceptionUpdated(AActor* Actor, FAIStimulus Stimulus)
{
    UBlackboardComponent* BB = GetBlackboardComponent();
    if (!Actor || !BB) { return; }

    if (Stimulus.Type == UAISense::GetSenseID<UAISense_Sight>())
    {
        if (Stimulus.WasSuccessfullySensed())
        {
            BB->SetValueAsObject(MyAIKeys::TargetActor, Actor);
        }
        else if (BB->GetValueAsObject(MyAIKeys::TargetActor) == Actor)
        {
            BB->ClearValue(MyAIKeys::TargetActor);
            BB->SetValueAsVector(MyAIKeys::LastKnownLocation, Stimulus.StimulusLocation);
        }
    }
    else if (Stimulus.Type == UAISense::GetSenseID<UAISense_Hearing>() && Stimulus.WasSuccessfullySensed())
    {
        BB->SetValueAsVector(MyAIKeys::NoiseLocation, Stimulus.StimulusLocation);
    }
}
```

- If the team ID changes at runtime, call `GetPerceptionComponent()->RequestStimuliListenerUpdate()`.
- For a Blueprint pawn: set AI Controller Class and Auto Possess AI = Placed in World or Spawned.

Player side (the perceived actor must answer the team question):

```cpp
// MyPlayerCharacter.h (excerpt)
#include "GenericTeamAgentInterface.h"

UCLASS()
class MYGAME_API AMyPlayerCharacter : public ACharacter, public IGenericTeamAgentInterface
{
    GENERATED_BODY()
public:
    virtual FGenericTeamId GetGenericTeamId() const override { return FGenericTeamId(0); }
};
```

Custom attitudes (e.g. several factions, neutral animals) - register a solver at startup:
`FGenericTeamId::SetAttitudeSolver(&MyAttitudeSolver);` with signature
`ETeamAttitude::Type MyAttitudeSolver(FGenericTeamId A, FGenericTeamId B)`; or override
`GetTeamAttitudeTowards(const AActor& Other) const` on the AI controller.

Noise from gameplay code (footsteps, gunshots):

```cpp
#include "Perception/AISense_Hearing.h"
UAISense_Hearing::ReportNoiseEvent(GetWorld(), GetActorLocation(), /*Loudness*/ 1.f, /*Instigator*/ this,
                                   /*MaxRange*/ 0.f /*0 = use listener range*/, /*Tag*/ NAME_None);
```

## 2. EQS from C++

```cpp
#include "EnvironmentQuery/EnvQueryManager.h"

UPROPERTY(EditDefaultsOnly, Category = "AI") TObjectPtr<UEnvQuery> FindCoverQuery;

void AMyAIController::FindCover()
{
    FEnvQueryRequest Request(FindCoverQuery, GetPawn());   // querier = the pawn
    Request.Execute(EEnvQueryRunMode::SingleResult,
                    FQueryFinishedSignature::CreateUObject(this, &AMyAIController::OnCoverFound));
}

void AMyAIController::OnCoverFound(TSharedPtr<FEnvQueryResult> Result)
{
    if (Result.IsValid() && Result->IsSuccessful())
    {
        MoveToLocation(Result->GetItemAsLocation(0), /*AcceptanceRadius*/ 50.f);
    }
}
```

Custom context (e.g. the current target) for tests like "not visible from target":
subclass `UEnvQueryContext` and override `ProvideContext(FEnvQueryInstance& QueryInstance,
FEnvQueryContextData& ContextData) const`, using `UEnvQueryItemType_Actor::SetContextHelper(ContextData, TargetActor)`
(`EnvironmentQuery/Items/EnvQueryItemType_Actor.h`).

