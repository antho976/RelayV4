# StateTree

StateTree is a general-purpose hierarchical state machine: states are arranged in a tree,
**selection** walks from the root choosing the first child whose **enter conditions** pass,
active states run **tasks**, and **transitions** move execution when tasks finish, on events, or
on conditions. It is used for AI, Smart Object interactions (Gameplay Interactions) and Mass.
Plugins: `StateTree` (core) and `GameplayStateTree` (components, actor/AI schemas). Build.cs:
`"StateTreeModule"`, `"GameplayStateTreeModule"`, plus `"GameplayTags"` if you use events, and
`"AIModule"` for AI context types.

The editor and API moved a lot between 5.3 and 5.6. Before writing C++, open the plugin headers
in the project's engine (`Engine/Plugins/Runtime/StateTree/Source/StateTreeModule/Public/`,
`Engine/Plugins/Runtime/GameplayStateTree/Source/GameplayStateTreeModule/Public/`) and confirm
the signatures below; they are the stable 5.3-5.5 shapes.

## When to prefer StateTree over a Behavior Tree

- The behaviour has clear **modes** (Idle, Patrol, Alert, Combat, Flee; boss phases) with explicit
  transitions between them.
- You want one tool for AI, Smart Object interactions and possibly Mass crowds.
- You want typed, bound data (task inputs bound to evaluator/context outputs) instead of stringly
  typed blackboard keys.
- Prefer a BT when the team already has BT content/tooling, or when "re-evaluate priorities
  whenever the blackboard changes" (observer aborts) is the core of the design.

## Concepts

| Concept | What it is |
|---|---|
| Schema | Decides what context the tree has. `StateTreeComponentSchema` (Actor context), `StateTreeAIComponentSchema` (Actor + AIController context; added with the AI component, 5.4+ - verify) |
| Context data | Objects provided by the component (Actor, AIController). Task properties in category `Context` bind to them automatically by type |
| Parameters | Tree-level inputs editable per instance (e.g. patrol radius) |
| Evaluators | Global, run while the tree runs, expose outputs (e.g. current target). Newer versions favour **global tasks** for the same job |
| Global tasks | Tasks that run for the whole tree's lifetime |
| State | Has enter conditions, tasks, transitions, and child states. Types: State, Group (no tasks), Linked (runs another state / subtree), Linked Asset (another StateTree asset, 5.4+) |
| Selection behaviour | How a state picks children: try children in order (default), try enter state, follow transitions; later versions add random and utility-based selection |
| Task | Does work; `EnterState` / `Tick` / `ExitState`; returns Running, Succeeded or Failed |
| Condition | Pure test used in enter conditions and transitions |
| Transition | Trigger (On State Completed / Succeeded / Failed, On Event, On Tick; newer versions add delegate triggers) + optional conditions -> target (a state, Next State, Tree Succeeded, Tree Failed). Can have priority and delay |
| Events | `FGameplayTag` (+ optional payload struct) sent to the tree; transitions can listen for them |

State completion: a state completes when its tasks complete (by default the first task to
finish completes the state; newer versions let you require all tasks). Completion bubbles to
"On State Completed" transitions, checked from the leaf up to the root.

## Running a StateTree on an AI

1. Enable plugins `StateTree` and `GameplayStateTree` (agent edits `.uproject`; editor restart).
2. Human: Content Browser > Add > Artificial Intelligence > State Tree; pick the schema
   (`StateTreeAIComponentSchema` for AI if listed, otherwise `StateTreeComponentSchema`).
   In the schema settings set the Actor class / AIController class so context types are exact.
3. Add the component to the **AIController** (for the AI schema) or the pawn (component schema):

```cpp
// MyStateTreeAIController.h (excerpt)
#include "Components/StateTreeAIComponent.h"   // GameplayStateTreeModule; 5.4+. Older: Components/StateTreeComponent.h

UPROPERTY(VisibleAnywhere, Category = "AI") TObjectPtr<UStateTreeAIComponent> StateTreeComponent;

// Constructor
StateTreeComponent = CreateDefaultSubobject<UStateTreeAIComponent>(TEXT("StateTree"));
```

4. Human: in the controller Blueprint (or C++ subclass Blueprint), select the component and set
   its State Tree asset. Leave "Start Logic Automatically" on, or call `StartLogic()` from
   `OnPossess` after the pawn exists if the tree needs the pawn at start.
5. The pawn uses this controller (AI Controller Class, Auto Possess AI = Placed in World or Spawned).

## C++ task

```cpp
// STTask_MoveToTarget.h
#pragma once
#include "CoreMinimal.h"
#include "StateTreeTaskBase.h"
#include "STTask_MoveToTarget.generated.h"

class AAIController;

USTRUCT()
struct FSTTask_MoveToTargetInstanceData
{
    GENERATED_BODY()

    UPROPERTY(EditAnywhere, Category = "Context")   TObjectPtr<AAIController> AIController = nullptr; // auto-bound
    UPROPERTY(EditAnywhere, Category = "Input")     TObjectPtr<AActor> Target = nullptr;              // must be bound
    UPROPERTY(EditAnywhere, Category = "Parameter") float AcceptanceRadius = 100.f;                   // literal or bound
};

USTRUCT(meta = (DisplayName = "Move To Target (Game)"))
struct MYGAME_API FSTTask_MoveToTarget : public FStateTreeTaskCommonBase
{
    GENERATED_BODY()

    using FInstanceDataType = FSTTask_MoveToTargetInstanceData;
    virtual const UStruct* GetInstanceDataType() const override { return FInstanceDataType::StaticStruct(); }

    virtual EStateTreeRunStatus EnterState(FStateTreeExecutionContext& Context, const FStateTreeTransitionResult& Transition) const override;
    virtual EStateTreeRunStatus Tick(FStateTreeExecutionContext& Context, const float DeltaTime) const override;
    virtual void ExitState(FStateTreeExecutionContext& Context, const FStateTreeTransitionResult& Transition) const override;
};
```

```cpp
// STTask_MoveToTarget.cpp
#include "STTask_MoveToTarget.h"
#include "StateTreeExecutionContext.h"
#include "AIController.h"
#include "Navigation/PathFollowingComponent.h"

EStateTreeRunStatus FSTTask_MoveToTarget::EnterState(FStateTreeExecutionContext& Context, const FStateTreeTransitionResult& Transition) const
{
    FInstanceDataType& Data = Context.GetInstanceData(*this);
    if (!Data.AIController || !Data.Target) { return EStateTreeRunStatus::Failed; }

    const EPathFollowingRequestResult::Type Result = Data.AIController->MoveToActor(Data.Target, Data.AcceptanceRadius);
    if (Result == EPathFollowingRequestResult::AlreadyAtGoal) { return EStateTreeRunStatus::Succeeded; }
    return Result == EPathFollowingRequestResult::RequestSuccessful ? EStateTreeRunStatus::Running : EStateTreeRunStatus::Failed;
}

EStateTreeRunStatus FSTTask_MoveToTarget::Tick(FStateTreeExecutionContext& Context, const float DeltaTime) const
{
    const FInstanceDataType& Data = Context.GetInstanceData(*this);
    const UPathFollowingComponent* PF = Data.AIController ? Data.AIController->GetPathFollowingComponent() : nullptr;
    if (!PF) { return EStateTreeRunStatus::Failed; }
    return PF->GetStatus() == EPathFollowingStatus::Idle ? EStateTreeRunStatus::Succeeded : EStateTreeRunStatus::Running;
}

void FSTTask_MoveToTarget::ExitState(FStateTreeExecutionContext& Context, const FStateTreeTransitionResult& Transition) const
{
    const FInstanceDataType& Data = Context.GetInstanceData(*this);
    if (Data.AIController) { Data.AIController->StopMovement(); }  // state left early (event, abort)
}
```

Notes:
- The task struct itself is **const and shared**; all per-instance state goes in the instance
  data struct. Never add mutable members to the task struct.
- `Category` names on instance data properties are significant: `Context` (auto-bound to schema
  context by type), `Input` (must be bound in the editor), `Parameter` (literal or optionally bound),
  `Output` (other nodes can bind to it).
- Idle-status polling is a simple completion check; for exact results, check the move result via
  the controller's move-completed delegate and store it in instance data. Recent versions also
  ship an engine Move To task in the gameplay/AI StateTree modules - check the task picker
  before writing your own.
- Blueprint alternative: subclass `StateTreeTaskBlueprintBase` (Blueprint class), implement
  Enter State / Tick / Exit State events and call **Finish Task** (Succeeded true/false). Mark
  variables with category `Context`/`Input`/`Parameter`/`Output` the same way.

## C++ condition

```cpp
// STCondition_InRange.h
#pragma once
#include "CoreMinimal.h"
#include "StateTreeConditionBase.h"
#include "STCondition_InRange.generated.h"

USTRUCT()
struct FSTCondition_InRangeInstanceData
{
    GENERATED_BODY()
    UPROPERTY(EditAnywhere, Category = "Context")   TObjectPtr<AActor> Actor = nullptr;
    UPROPERTY(EditAnywhere, Category = "Input")     TObjectPtr<AActor> Target = nullptr;
    UPROPERTY(EditAnywhere, Category = "Parameter") float Range = 200.f;
};

USTRUCT(meta = (DisplayName = "Target In Range (Game)"))
struct MYGAME_API FSTCondition_InRange : public FStateTreeConditionCommonBase
{
    GENERATED_BODY()
    using FInstanceDataType = FSTCondition_InRangeInstanceData;
    virtual const UStruct* GetInstanceDataType() const override { return FInstanceDataType::StaticStruct(); }

    virtual bool TestCondition(FStateTreeExecutionContext& Context) const override
    {
        const FInstanceDataType& Data = Context.GetInstanceData(*this);
        return Data.Actor && Data.Target
            && FVector::Dist2D(Data.Actor->GetActorLocation(), Data.Target->GetActorLocation()) <= Data.Range;
    }
};
```

(Include `StateTreeExecutionContext.h` where `TestCondition` is defined.) Conditions must be
side-effect free - they are evaluated during selection and for transitions, possibly many times.

## Evaluator / global task producing a target

An evaluator (`FStateTreeEvaluatorCommonBase`, overrides `TreeStart`, `TreeStop`,
`Tick(Context, DeltaTime) const`) or a global task can read the AI controller's perception and
write an `Output` property `TargetActor` every tick. States and tasks bind their `Target` input to
it. This replaces the blackboard: data flows by binding, and type mismatches are editor errors
rather than silent runtime failures.

## Events

- Send from gameplay code: `StateTreeComponent->SendStateTreeEvent(Tag)` (a `FGameplayTag`, with
  an optional payload `FConstStructView` / `FInstancedStruct`). E.g. `AI.Event.Damaged`,
  `AI.Event.HeardNoise` from the perception callback.
- Send from a task: `Context.SendEvent(Tag, Payload, Origin)`.
- Transitions with trigger **On Event** and the tag; newer versions can also require the event on
  state entry and consume it. Define tags in `Config/DefaultGameplayTags.ini` or a tag table.

## Example layout (enemy)

```
Root (Try Select Children In Order)
  Dead          enter: IsDead                          tasks: Play Death, Stop Logic
  Combat        enter: TargetActor is valid (Object Is Valid condition bound to evaluator output)
    Attack      enter: Target In Range                 tasks: Play Attack Montage   on completed -> Combat
    Chase                                              tasks: Move To Target        on completed -> Combat
    transitions: On Event AI.Event.LostTarget -> Investigate
  Investigate   enter: LastKnownLocation is set        tasks: Move To Location, Delay 3 s  on completed -> Patrol
  Patrol                                               tasks: Find Patrol Point, Move To, Delay 2 s  on completed -> Patrol
Root transition: On Event AI.Event.Damaged -> Combat (priority High)
```

Group states (no tasks) are useful to share enter conditions and transitions across children.
Re-evaluation: unlike BT observer aborts, a StateTree only reselects on transitions. To react to
"target acquired" while patrolling, add an On Tick transition with a condition, or (better) an
On Event transition driven by the perception callback.

## Debugging

- StateTree editor during PIE: pick the debugged instance; active states highlight. 5.4+ has a
  StateTree Debugger panel with a trace timeline; traces also show in the Rewind Debugger.
- `LogStateTree` (use `ue_log` with `filter: "LogStateTree"`). Validation errors at compile
  (unbound Input properties, schema mismatch) show in the StateTree editor's compiler results.
- Visual Logger records StateTree activity for AI actors.

## Pitfalls

- Task stores state in the task struct -> shared by every AI, data races/bugs. Use instance data.
- `Input` property left unbound -> compile error, or silently null in older versions.
- Tree never starts: component on the wrong actor for the schema, asset not set, or Start Logic
  Automatically off with no `StartLogic()` call.
- Every task in a state finishing immediately (returning Succeeded from EnterState) with an
  On Completed transition back to the same state -> loops every frame. Add a Delay task or a
  latent task.
- Changing the schema of an existing asset invalidates bindings; re-check the whole tree.
