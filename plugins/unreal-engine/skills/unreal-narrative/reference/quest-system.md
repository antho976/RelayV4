# Quest System: Design and C++ Implementation

## 1. Design model

```
Inactive ──Start (StartCondition met)──> Active[Stage 0] ─objectives done─> Active[Stage 1] ... ─last stage─> Completed
                                              │                                              
                                              └── any FailFlag set / fail event ──> Failed (or fail-forward: branch)
```

- **Quest**: a unit of story or reward with one player-facing title. Its *stages* are sequential beats.
- **Objective**: a countable condition within a stage, satisfied by **story events** (`Event.Kill.Wolf`,
  `Event.Talk.Mira`, `Event.Enter.Harbor`) sent through `UStoryStateSubsystem::SendStoryEvent`.
  Optional objectives don't block the stage.
- **Parallel objectives** belong in one stage ("Find the key *and* the map"). **Sequence** belongs in stages.
- **Branching**: model it as separate quests or stages gated by flags (`Story.Choice.*`), not as one quest
  with a graph. That keeps it easy to save and debug.
- **Fail states**: timers, a protected NPC dying (flag `Story.NPC.Mira.Dead`), or leaving an area. Prefer
  **fail-forward**: a failure sets a flag, and a different quest or stage continues the story. Use hard fail
  (reload the checkpoint) only for short, skill-based objectives.
- **Objective text** starts with a verb and stays under about 40 characters: "Ring the harbor bell", "Defeat wolves (3/5)".
- **Guidance**: each active objective gets a world marker target (an actor tag or location), resolved by the UI layer.

Tag conventions: `Quest.Harbor` (quest ID), `Quest.Harbor.Obj.Ropes` (objective ID), `Event.*` (events),
and `Quest.Harbor.Complete` / `Quest.Harbor.Failed` (put them in `FlagsOnComplete` / `FlagsOnFail`, so dialogue and levels can react).

## 2. Data

```cpp
// QuestAsset.h   Build.cs: "GameplayTags"
#pragma once
#include "CoreMinimal.h"
#include "Engine/DataAsset.h"
#include "GameplayTagContainer.h"
#include "StoryStateSubsystem.h"
#include "QuestAsset.generated.h"

USTRUCT(BlueprintType)
struct FQuestObjective
{
	GENERATED_BODY()
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FGameplayTag ObjectiveId;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FText Description;      // "Defeat wolves"
	/** An event that matches this tag (hierarchically) counts toward the objective. */
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FGameplayTag CompletionEvent;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest", meta = (ClampMin = "1")) int32 RequiredCount = 1;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") bool bOptional = false;
	/** Actor tag the UI uses to place a marker. */
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FName MarkerActorTag;
};

USTRUCT(BlueprintType)
struct FQuestStage
{
	GENERATED_BODY()
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FText JournalText;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest", meta = (TitleProperty = "Description")) TArray<FQuestObjective> Objectives;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FGameplayTagContainer FlagsOnComplete;
};

UCLASS(BlueprintType)
class MYGAME_API UQuestAsset : public UPrimaryDataAsset
{
	GENERATED_BODY()
public:
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FGameplayTag QuestId;     // Quest.Harbor
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FText Title;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FStoryCondition StartCondition;
	/** Setting any of these flags fails the quest while it is active. */
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FGameplayTagContainer FailFlags;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") TArray<FQuestStage> Stages;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FGameplayTagContainer FlagsOnComplete;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Quest") FGameplayTagContainer FlagsOnFail;

	virtual FPrimaryAssetId GetPrimaryAssetId() const override { return FPrimaryAssetId(TEXT("Quest"), GetFName()); }
};
```

## 3. Runtime

```cpp
// QuestSubsystem.h
#pragma once
#include "CoreMinimal.h"
#include "Subsystems/GameInstanceSubsystem.h"
#include "QuestAsset.h"
#include "QuestSubsystem.generated.h"

UENUM(BlueprintType)
enum class EQuestState : uint8 { Inactive, Active, Completed, Failed };

USTRUCT(BlueprintType)
struct FQuestRuntimeState
{
	GENERATED_BODY()
	UPROPERTY(SaveGame, BlueprintReadOnly, Category = "Quest") TSoftObjectPtr<UQuestAsset> Quest;
	UPROPERTY(SaveGame, BlueprintReadOnly, Category = "Quest") EQuestState State = EQuestState::Inactive;
	UPROPERTY(SaveGame, BlueprintReadOnly, Category = "Quest") int32 StageIndex = 0;
	UPROPERTY(SaveGame, BlueprintReadOnly, Category = "Quest") TMap<FGameplayTag, int32> Progress;   // ObjectiveId -> count
};

DECLARE_DYNAMIC_MULTICAST_DELEGATE_TwoParams(FOnQuestUpdated, FGameplayTag, QuestId, EQuestState, State);

UCLASS()
class MYGAME_API UQuestSubsystem : public UGameInstanceSubsystem
{
	GENERATED_BODY()
public:
	virtual void Initialize(FSubsystemCollectionBase& Collection) override;

	UFUNCTION(BlueprintCallable, Category = "Quest") bool StartQuest(UQuestAsset* Quest);
	UFUNCTION(BlueprintCallable, Category = "Quest") void FailQuest(FGameplayTag QuestId);
	UFUNCTION(BlueprintPure, Category = "Quest") EQuestState GetQuestState(FGameplayTag QuestId) const;
	UFUNCTION(BlueprintPure, Category = "Quest") bool GetQuestRuntime(FGameplayTag QuestId, FQuestRuntimeState& Out) const;

	UPROPERTY(BlueprintAssignable, Category = "Quest") FOnQuestUpdated OnQuestUpdated;

	// Save integration: copy into / out of the game's USaveGame (a UPROPERTY TMap there).
	const TMap<FGameplayTag, FQuestRuntimeState>& GetAllQuests() const { return Quests; }
	void RestoreQuests(const TMap<FGameplayTag, FQuestRuntimeState>& Saved) { Quests = Saved; }

private:
	UFUNCTION() void HandleStoryEvent(FGameplayTag Event, int32 Count);
	UFUNCTION() void HandleFlagChanged(FGameplayTag Flag, bool bIsSet);
	// Both take the ID and re-find the entry: listeners may add quests (reallocating the map) mid-call.
	void TryAdvanceStage(FGameplayTag QuestId);
	void Finish(FGameplayTag QuestId, bool bSuccess);
	UStoryStateSubsystem* Story() const { return GetGameInstance()->GetSubsystem<UStoryStateSubsystem>(); }

	UPROPERTY() TMap<FGameplayTag, FQuestRuntimeState> Quests;
};
```

```cpp
// QuestSubsystem.cpp
#include "QuestSubsystem.h"
#include "Engine/GameInstance.h"

void UQuestSubsystem::Initialize(FSubsystemCollectionBase& Collection)
{
	Super::Initialize(Collection);
	Collection.InitializeDependency(UStoryStateSubsystem::StaticClass());   // make sure it exists first
	if (UStoryStateSubsystem* S = Story())
	{
		S->OnStoryEvent.AddDynamic(this, &UQuestSubsystem::HandleStoryEvent);
		S->OnFlagChanged.AddDynamic(this, &UQuestSubsystem::HandleFlagChanged);
	}
}

bool UQuestSubsystem::StartQuest(UQuestAsset* Quest)
{
	if (!Quest || !Quest->QuestId.IsValid() || Quest->Stages.Num() == 0) { return false; }
	if (GetQuestState(Quest->QuestId) != EQuestState::Inactive) { return false; }
	if (!Story()->MeetsCondition(Quest->StartCondition)) { return false; }

	const FGameplayTag QuestId = Quest->QuestId;
	FQuestRuntimeState& R = Quests.Add(QuestId);
	R.Quest = Quest;
	R.State = EQuestState::Active;
	R.StageIndex = 0;
	OnQuestUpdated.Broadcast(QuestId, EQuestState::Active);   // R must not be used after a broadcast
	TryAdvanceStage(QuestId);       // a stage whose objectives are all optional completes immediately
	return true;
}

void UQuestSubsystem::HandleStoryEvent(FGameplayTag Event, int32 Count)
{
	TArray<FGameplayTag> Ids;
	Quests.GetKeys(Ids);           // copy the keys: handlers may start new quests while we iterate
	for (const FGameplayTag& Id : Ids)
	{
		FQuestRuntimeState* R = Quests.Find(Id);
		if (!R || R->State != EQuestState::Active) { continue; }
		const UQuestAsset* Quest = R->Quest.LoadSynchronous();
		if (!Quest || !Quest->Stages.IsValidIndex(R->StageIndex)) { continue; }

		bool bChanged = false;
		for (const FQuestObjective& Obj : Quest->Stages[R->StageIndex].Objectives)
		{
			if (Obj.CompletionEvent.IsValid() && Event.MatchesTag(Obj.CompletionEvent))
			{
				int32& Value = R->Progress.FindOrAdd(Obj.ObjectiveId);
				const int32 NewValue = FMath::Min(Value + Count, Obj.RequiredCount);
				bChanged |= NewValue != Value;
				Value = NewValue;
			}
		}
		if (bChanged)
		{
			OnQuestUpdated.Broadcast(Id, EQuestState::Active);
			TryAdvanceStage(Id);
		}
	}
}

void UQuestSubsystem::TryAdvanceStage(FGameplayTag QuestId)
{
	for (;;)
	{
		FQuestRuntimeState* R = Quests.Find(QuestId);
		const UQuestAsset* Quest = R ? R->Quest.LoadSynchronous() : nullptr;
		if (!Quest || R->State != EQuestState::Active || !Quest->Stages.IsValidIndex(R->StageIndex)) { return; }

		const FQuestStage& Stage = Quest->Stages[R->StageIndex];   // points into the asset: stable
		for (const FQuestObjective& Obj : Stage.Objectives)
		{
			const int32* Have = R->Progress.Find(Obj.ObjectiveId);
			if (!Obj.bOptional && (!Have || *Have < Obj.RequiredCount)) { return; }   // stage not done yet
		}
		R->Progress.Reset();
		const bool bLastStage = ++R->StageIndex >= Quest->Stages.Num();
		Story()->ApplyFlags(Stage.FlagsOnComplete);   // may re-enter (fail flags, new quests): R is stale now
		if (bLastStage) { Finish(QuestId, true); return; }
		OnQuestUpdated.Broadcast(QuestId, EQuestState::Active);
	}
}

void UQuestSubsystem::HandleFlagChanged(FGameplayTag Flag, bool bIsSet)
{
	if (!bIsSet) { return; }
	TArray<FGameplayTag> Ids;
	Quests.GetKeys(Ids);
	for (const FGameplayTag& Id : Ids)
	{
		FQuestRuntimeState* R = Quests.Find(Id);
		const UQuestAsset* Quest = R ? R->Quest.LoadSynchronous() : nullptr;
		if (Quest && R->State == EQuestState::Active && Quest->FailFlags.HasTagExact(Flag))
		{
			Finish(Id, false);
		}
	}
}

void UQuestSubsystem::FailQuest(FGameplayTag QuestId)
{
	Finish(QuestId, false);
}

void UQuestSubsystem::Finish(FGameplayTag QuestId, bool bSuccess)
{
	FQuestRuntimeState* R = Quests.Find(QuestId);
	const UQuestAsset* Quest = R ? R->Quest.LoadSynchronous() : nullptr;
	if (!Quest || R->State != EQuestState::Active) { return; }   // already finished (e.g. failed during a stage flag)
	const EQuestState NewState = bSuccess ? EQuestState::Completed : EQuestState::Failed;
	R->State = NewState;
	// Applying flags can re-enter the handlers or start quests, so R is not touched after this line.
	Story()->ApplyFlags(bSuccess ? Quest->FlagsOnComplete : Quest->FlagsOnFail);
	OnQuestUpdated.Broadcast(QuestId, NewState);
}

EQuestState UQuestSubsystem::GetQuestState(FGameplayTag QuestId) const
{
	const FQuestRuntimeState* R = Quests.Find(QuestId);
	return R ? R->State : EQuestState::Inactive;
}

bool UQuestSubsystem::GetQuestRuntime(FGameplayTag QuestId, FQuestRuntimeState& Out) const
{
	if (const FQuestRuntimeState* R = Quests.Find(QuestId)) { Out = *R; return true; }
	return false;
}
```

Notes and pitfalls:
- **Re-entrancy.** Applying flags broadcasts `OnFlagChanged`, and listeners can fail quests or start new ones (adding
  to `Quests`, which may reallocate it). That is why the handlers iterate over a *copy of the keys*,
  `TryAdvanceStage` and `Finish` take a quest ID and re-find the entry, and no `FQuestRuntimeState*` is used
  after a broadcast. Keep that rule when you extend this code.
- **Hierarchical events.** `Event.Kill.Wolf.Alpha` satisfies an objective listening for `Event.Kill.Wolf`
  (`MatchesTag`). Objectives listening for `Event.Kill` count every kill, so be deliberate.
- **Auto-start** quests from dialogue (a choice sets a flag, and a listener calls `StartQuest`), from triggers, or from the
  `StartCondition` evaluated on `OnFlagChanged`.
- **Save.** Put `UPROPERTY() TMap<FGameplayTag, FQuestRuntimeState> Quests;` in the game's `USaveGame`. Soft
  pointers serialize as paths, and `LoadSynchronous` resolves them after loading. Save the story flags in the same
  save object, so they can't disagree.
- **Loading cost.** `LoadSynchronous` on quest assets is fine because they are small. Keep heavy references (icons, VO) soft
  inside the quest asset.
- **Multiplayer.** Run the quest subsystem logic on the server (or the host) and replicate a summary (IDs, state, stage,
  progress) through the GameState or PlayerState for UI. See `unreal-multiplayer`.

## 4. Sending events from gameplay

```cpp
// On enemy death (server):
if (UStoryStateSubsystem* S = GetGameInstance()->GetSubsystem<UStoryStateSubsystem>())
{
	S->SendStoryEvent(EnemyDefinition->KillEventTag);   // e.g. Event.Kill.Wolf
}
```

Other good emitters: pickup actors (`Event.Item.Pickup.<Item>`), trigger volumes (`Event.Enter.<Area>`, sent once and
guarded by a flag), dialogue nodes (the `StoryEvent` field), and Sequencer event keys (`Event.Cutscene.<Name>.Done`).
Level Blueprints should only *send* events, and never contain quest logic.

## 5. Journal and HUD

- Bind `OnQuestUpdated`, then rebuild the journal from `GetAllQuests()` and each asset's `Title`, the current stage's
  `JournalText`, and the objectives with progress `(n/Required)`. Format with `FText::Format`, never by concatenation.
- Show at most 1–3 tracked quests on the HUD. The player picks which one is tracked.
- Completion feedback: a sound, a toast, and the journal update. Failure feedback states the cause.

## 6. Test checklist

- [ ] Each quest can be completed start to finish in PIE using only in-game actions.
- [ ] Events from unrelated systems don't advance the wrong objectives (check tag hierarchy).
- [ ] Save mid-stage, load, and progress and stage are restored.
- [ ] Fail flags fail only *active* quests, and the fail-forward path is reachable.
- [ ] No quest is left unstartable: its `StartCondition` flags are actually set somewhere (search for the tags).
