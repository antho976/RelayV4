# Behavior Tree C++ reference

Build.cs: `PublicDependencyModuleNames.AddRange(new[] { "AIModule", "GameplayTasks", "NavigationSystem" });`
Log categories to watch: `LogBehaviorTree`, `LogBlackboard`, `LogAINavigation`, `LogPathFollowing`.

## 1. Blackboard key names in one place

```cpp
// MyAIKeys.h
#pragma once
#include "CoreMinimal.h"

namespace MyAIKeys
{
    inline const FName TargetActor(TEXT("TargetActor"));             // Object (Actor)
    inline const FName LastKnownLocation(TEXT("LastKnownLocation")); // Vector
    inline const FName NoiseLocation(TEXT("NoiseLocation"));         // Vector
    inline const FName HomeLocation(TEXT("HomeLocation"));           // Vector
    inline const FName PatrolPoint(TEXT("PatrolPoint"));             // Vector
    inline const FName bInAttackRange(TEXT("bInAttackRange"));       // Bool
}
```

The Blackboard asset (`BB_Enemy`) must contain keys with exactly these names and types. A
missing key makes `SetValueAs*` silently do nothing (a warning in `LogBlackboard` at best).

## 2. AI controller

The controller that starts this tree, configures perception and team affiliation, and writes
perception results into these keys is in `ai-controller-perception.md` (same folder), together
with EQS-from-C++ code.

## 3. Instant task: random reachable patrol point

```cpp
// BTTask_FindPatrolPoint.h
#pragma once
#include "CoreMinimal.h"
#include "BehaviorTree/Tasks/BTTask_BlackboardBase.h"
#include "BTTask_FindPatrolPoint.generated.h"

UCLASS()
class MYGAME_API UBTTask_FindPatrolPoint : public UBTTask_BlackboardBase
{
    GENERATED_BODY()
public:
    UBTTask_FindPatrolPoint();
    virtual EBTNodeResult::Type ExecuteTask(UBehaviorTreeComponent& OwnerComp, uint8* NodeMemory) override;
    virtual FString GetStaticDescription() const override;

protected:
    UPROPERTY(EditAnywhere, Category = "Patrol", meta = (ClampMin = "100")) float Radius = 1500.f;
};
```

```cpp
// BTTask_FindPatrolPoint.cpp
#include "BTTask_FindPatrolPoint.h"
#include "AIController.h"
#include "BehaviorTree/BlackboardComponent.h"
#include "NavigationSystem.h"

UBTTask_FindPatrolPoint::UBTTask_FindPatrolPoint()
{
    NodeName = TEXT("Find Patrol Point");
    // Only vector keys may be picked in the editor dropdown.
    BlackboardKey.AddVectorFilter(this, GET_MEMBER_NAME_CHECKED(UBTTask_FindPatrolPoint, BlackboardKey));
}

EBTNodeResult::Type UBTTask_FindPatrolPoint::ExecuteTask(UBehaviorTreeComponent& OwnerComp, uint8* NodeMemory)
{
    const AAIController* AI = OwnerComp.GetAIOwner();
    const APawn* Pawn = AI ? AI->GetPawn() : nullptr;
    UNavigationSystemV1* Nav = FNavigationSystem::GetCurrent<UNavigationSystemV1>(GetWorld());
    FNavLocation Result;
    if (Pawn && Nav && Nav->GetRandomReachablePointInRadius(Pawn->GetActorLocation(), Radius, Result))
    {
        OwnerComp.GetBlackboardComponent()->SetValueAsVector(GetSelectedBlackboardKey(), Result.Location);
        return EBTNodeResult::Succeeded;
    }
    return EBTNodeResult::Failed;
}

FString UBTTask_FindPatrolPoint::GetStaticDescription() const
{
    return FString::Printf(TEXT("%s: radius %.0f"), *Super::GetStaticDescription(), Radius);
}
```

## 4. Latent task: play an attack montage and wait

Nodes are **shared across all AI using the tree** (not instanced by default). Per-AI state goes
in node memory, never in member variables. Keep node memory plain data (floats, ints, bools);
for non-trivial types, look up `InitializeMemory`/`CleanupMemory` in `BTNode.h` of your engine.

```cpp
// BTTask_PlayAttackMontage.h
#pragma once
#include "CoreMinimal.h"
#include "BehaviorTree/BTTaskNode.h"
#include "BTTask_PlayAttackMontage.generated.h"

class UAnimMontage;

struct FBTPlayAttackMemory { float TimeLeft = 0.f; };

UCLASS()
class MYGAME_API UBTTask_PlayAttackMontage : public UBTTaskNode
{
    GENERATED_BODY()
public:
    UBTTask_PlayAttackMontage();
    virtual EBTNodeResult::Type ExecuteTask(UBehaviorTreeComponent& OwnerComp, uint8* NodeMemory) override;
    virtual EBTNodeResult::Type AbortTask(UBehaviorTreeComponent& OwnerComp, uint8* NodeMemory) override;
    virtual void TickTask(UBehaviorTreeComponent& OwnerComp, uint8* NodeMemory, float DeltaSeconds) override;
    virtual uint16 GetInstanceMemorySize() const override { return sizeof(FBTPlayAttackMemory); }

protected:
    UPROPERTY(EditAnywhere, Category = "Attack") TObjectPtr<UAnimMontage> Montage;
};
```

```cpp
// BTTask_PlayAttackMontage.cpp
#include "BTTask_PlayAttackMontage.h"
#include "AIController.h"
#include "GameFramework/Character.h"
#include "Animation/AnimInstance.h"

namespace
{
    UAnimInstance* GetAnim(const UBehaviorTreeComponent& OwnerComp)
    {
        const AAIController* AI = OwnerComp.GetAIOwner();
        const ACharacter* Char = AI ? Cast<ACharacter>(AI->GetPawn()) : nullptr;
        return Char && Char->GetMesh() ? Char->GetMesh()->GetAnimInstance() : nullptr;
    }
}

UBTTask_PlayAttackMontage::UBTTask_PlayAttackMontage()
{
    NodeName = TEXT("Play Attack Montage");
    bNotifyTick = true;
}

EBTNodeResult::Type UBTTask_PlayAttackMontage::ExecuteTask(UBehaviorTreeComponent& OwnerComp, uint8* NodeMemory)
{
    UAnimInstance* Anim = GetAnim(OwnerComp);
    const float Length = (Anim && Montage) ? Anim->Montage_Play(Montage) : 0.f;
    if (Length <= 0.f) { return EBTNodeResult::Failed; }
    CastInstanceNodeMemory<FBTPlayAttackMemory>(NodeMemory)->TimeLeft = Length;
    return EBTNodeResult::InProgress;
}

void UBTTask_PlayAttackMontage::TickTask(UBehaviorTreeComponent& OwnerComp, uint8* NodeMemory, float DeltaSeconds)
{
    FBTPlayAttackMemory* Mem = CastInstanceNodeMemory<FBTPlayAttackMemory>(NodeMemory);
    Mem->TimeLeft -= DeltaSeconds;
    UAnimInstance* Anim = GetAnim(OwnerComp);
    if (!Anim || !Anim->Montage_IsPlaying(Montage) || Mem->TimeLeft <= 0.f)
    {
        FinishLatentTask(OwnerComp, EBTNodeResult::Succeeded);
    }
}

EBTNodeResult::Type UBTTask_PlayAttackMontage::AbortTask(UBehaviorTreeComponent& OwnerComp, uint8* NodeMemory)
{
    if (UAnimInstance* Anim = GetAnim(OwnerComp)) { Anim->Montage_Stop(0.2f, Montage); }
    return EBTNodeResult::Aborted;
}
```

Rules for latent tasks: every `InProgress` path must end in `FinishLatentTask`; `AbortTask`
must clean up (stop montage, stop movement, release tokens). An abort that returns `InProgress`
must later call `FinishLatentAbort`.

## 5. Service: keep a bool key up to date (drives observer aborts)

```cpp
// BTService_UpdateAttackRange.h
#pragma once
#include "CoreMinimal.h"
#include "BehaviorTree/BTService.h"
#include "BTService_UpdateAttackRange.generated.h"

UCLASS()
class MYGAME_API UBTService_UpdateAttackRange : public UBTService
{
    GENERATED_BODY()
public:
    UBTService_UpdateAttackRange();
    virtual void InitializeFromAsset(UBehaviorTree& Asset) override;
protected:
    virtual void TickNode(UBehaviorTreeComponent& OwnerComp, uint8* NodeMemory, float DeltaSeconds) override;

    UPROPERTY(EditAnywhere, Category = "Blackboard") FBlackboardKeySelector TargetKey;   // Object
    UPROPERTY(EditAnywhere, Category = "Blackboard") FBlackboardKeySelector InRangeKey;  // Bool
    UPROPERTY(EditAnywhere, Category = "Attack") float AttackRange = 200.f;
};
```

```cpp
// BTService_UpdateAttackRange.cpp
#include "BTService_UpdateAttackRange.h"
#include "AIController.h"
#include "BehaviorTree/BlackboardComponent.h"
#include "BehaviorTree/BlackboardData.h"

UBTService_UpdateAttackRange::UBTService_UpdateAttackRange()
{
    NodeName = TEXT("Update Attack Range");
    Interval = 0.2f;
    RandomDeviation = 0.05f;
    TargetKey.AddObjectFilter(this, GET_MEMBER_NAME_CHECKED(UBTService_UpdateAttackRange, TargetKey), AActor::StaticClass());
    InRangeKey.AddBoolFilter(this, GET_MEMBER_NAME_CHECKED(UBTService_UpdateAttackRange, InRangeKey));
}

void UBTService_UpdateAttackRange::InitializeFromAsset(UBehaviorTree& Asset)
{
    Super::InitializeFromAsset(Asset);
    if (const UBlackboardData* BBAsset = GetBlackboardAsset())
    {
        TargetKey.ResolveSelectedKey(*BBAsset);   // required for custom FBlackboardKeySelector members
        InRangeKey.ResolveSelectedKey(*BBAsset);
    }
}

void UBTService_UpdateAttackRange::TickNode(UBehaviorTreeComponent& OwnerComp, uint8* NodeMemory, float DeltaSeconds)
{
    Super::TickNode(OwnerComp, NodeMemory, DeltaSeconds);
    UBlackboardComponent* BB = OwnerComp.GetBlackboardComponent();
    const APawn* Pawn = OwnerComp.GetAIOwner() ? OwnerComp.GetAIOwner()->GetPawn() : nullptr;
    const AActor* Target = Cast<AActor>(BB->GetValueAsObject(TargetKey.SelectedKeyName));
    const bool bInRange = Pawn && Target && FVector::Dist2D(Pawn->GetActorLocation(), Target->GetActorLocation()) <= AttackRange;
    BB->SetValueAsBool(InRangeKey.SelectedKeyName, bInRange);
}
```

## 6. Decorators

Prefer the built-in **Blackboard** decorator with Observer Aborts on keys written by services
and perception - it re-evaluates when the key changes. A custom C++ decorator overriding
`CalculateRawConditionValue(UBehaviorTreeComponent&, uint8*) const` is evaluated when the branch
is entered; it does not re-check by itself. To make it abort reactively you must request
re-evaluation yourself (tick it and call the flow-abort helper in `BTDecorator.h` for your
version). Usually a service + bool key + Blackboard decorator is simpler and debuggable.

Observer aborts cheat sheet (on a decorator of branch B):
- **Self**: abort B when the condition becomes false while B runs.
- **Lower Priority**: abort branches to the right of B when the condition becomes true.
- **Both**: both of the above. Typical for "Has Target" on the combat branch.

## 7. Attack tokens (fair group combat)

```cpp
// AttackTokenSubsystem.h
#pragma once
#include "CoreMinimal.h"
#include "Subsystems/WorldSubsystem.h"
#include "AttackTokenSubsystem.generated.h"

UCLASS()
class MYGAME_API UAttackTokenSubsystem : public UWorldSubsystem
{
    GENERATED_BODY()
public:
    UFUNCTION(BlueprintCallable, Category = "AI")
    bool TryAcquire(AActor* Target, AActor* Attacker, int32 MaxAttackers = 2)
    {
        TArray<TWeakObjectPtr<AActor>>& Holders = Tokens.FindOrAdd(Target).Attackers;
        Holders.RemoveAll([](const TWeakObjectPtr<AActor>& A) { return !A.IsValid(); });
        if (Holders.Contains(Attacker)) { return true; }
        if (Holders.Num() >= MaxAttackers) { return false; }
        Holders.Add(Attacker);
        return true;
    }

    UFUNCTION(BlueprintCallable, Category = "AI")
    void Release(AActor* Target, AActor* Attacker)
    {
        if (FTokenHolders* H = Tokens.Find(Target)) { H->Attackers.Remove(Attacker); }
    }

private:
    struct FTokenHolders { TArray<TWeakObjectPtr<AActor>> Attackers; };
    TMap<TWeakObjectPtr<AActor>, FTokenHolders> Tokens;
};
```

Use it from a task ("Acquire Attack Token" fails when none is free -> the Selector falls back
to a "Circle / Reposition" branch) and release it in the attack task's finish and abort paths,
and when the AI dies.

## 8. Python: create BT and Blackboard assets

```python
import unreal
tools = unreal.AssetToolsHelpers.get_asset_tools()
with unreal.ScopedEditorTransaction("Create enemy AI assets"):
    bb = tools.create_asset("BB_Enemy", "/Game/AI/Enemy", unreal.BlackboardData, unreal.BlackboardDataFactory())
    bt = tools.create_asset("BT_Enemy", "/Game/AI/Enemy", unreal.BehaviorTree, unreal.BehaviorTreeFactory())
unreal.EditorAssetLibrary.save_asset(bb.get_path_name())
unreal.EditorAssetLibrary.save_asset(bt.get_path_name())
```

Confirm the factory names with `help(unreal.BlackboardDataFactory)` first. Adding blackboard keys
and wiring the tree from Python is not reliably exposed; give the human exact steps:
open `BB_Enemy` > New Key > Object (Base Class Actor) `TargetActor`, Vector `LastKnownLocation`,
... ; open `BT_Enemy` > Details > Blackboard Asset = `BB_Enemy`; then the node layout, e.g.:

```
ROOT
 Selector
  [Blackboard: TargetActor Is Set, Observer aborts Both]  Sequence "Combat"
      Service: Update Attack Range (Target=TargetActor, InRange=bInAttackRange)
      Selector
        [Blackboard: bInAttackRange Is Set, aborts Both] Sequence: Play Attack Montage -> Wait 0.5
        Move To (TargetActor, Acceptance Radius 150)
  [Blackboard: LastKnownLocation Is Set, aborts Lower Priority] Sequence "Investigate"
      Move To (LastKnownLocation) -> Wait 3 -> (task clears LastKnownLocation)
  Sequence "Patrol": Find Patrol Point (PatrolPoint) -> Move To (PatrolPoint) -> Wait 2 (deviation 1)
```
