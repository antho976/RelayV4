# Data-Driven Dialogue System (C++ sketch)

This is a compact, working design: conversations are `UDialogueAsset` Data Assets (nodes and choices),
a `UDialogueSubsystem` (world subsystem) runs them, conditions and effects go through
`UStoryStateSubsystem` (defined in `SKILL.md` §5), and UMG widgets listen to delegates. Build.cs:
`"Core","CoreUObject","Engine","GameplayTags"`. Replace `MYGAME_API` with the module's API macro.

## 1. Data

```cpp
// DialogueAsset.h
#pragma once
#include "CoreMinimal.h"
#include "Engine/DataAsset.h"
#include "GameplayTagContainer.h"
#include "StoryStateSubsystem.h"          // FStoryCondition
#include "DialogueAsset.generated.h"

class USoundBase;

USTRUCT(BlueprintType)
struct FDialogueChoice
{
	GENERATED_BODY()
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FText Text;
	/** Node to jump to. None ends the conversation. */
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FName TargetNode;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FStoryCondition Condition;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FGameplayTagContainer FlagsToSet;
	/** If the condition fails: show greyed out (true) or hide (false). */
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") bool bShowWhenLocked = false;
};

USTRUCT(BlueprintType)
struct FDialogueNode
{
	GENERATED_BODY()
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FName NodeId;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FGameplayTag Speaker;   // Character.Mira
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue", meta = (MultiLine = true)) FText Text;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") TSoftObjectPtr<USoundBase> Voice;
	/** Applied when this line starts. */
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FGameplayTagContainer FlagsToSet;
	/** Optional story event sent when this line starts (quests listen). */
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FGameplayTag StoryEvent;
	/** Non-empty: the player must choose. Empty: continue to NextNode. */
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") TArray<FDialogueChoice> Choices;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FName NextNode;
};

USTRUCT(BlueprintType)
struct FDialogueEntry
{
	GENERATED_BODY()
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FStoryCondition Condition;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") FName StartNode;
};

UCLASS(BlueprintType)
class MYGAME_API UDialogueAsset : public UPrimaryDataAsset
{
	GENERATED_BODY()
public:
	/** Checked top to bottom. The first passing entry starts the conversation (first meeting, repeat visit, post-quest...). */
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue") TArray<FDialogueEntry> Entries;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, Category = "Dialogue", meta = (TitleProperty = "NodeId")) TArray<FDialogueNode> Nodes;

	const FDialogueNode* FindNode(FName NodeId) const
	{
		if (NodeId.IsNone()) { return nullptr; }
		return Nodes.FindByPredicate([NodeId](const FDialogueNode& N) { return N.NodeId == NodeId; });
	}

#if WITH_EDITOR
	virtual EDataValidationResult IsDataValid(FDataValidationContext& Context) const override;
#endif
};
```

```cpp
// DialogueAsset.cpp
#include "DialogueAsset.h"
#if WITH_EDITOR
#include "Misc/DataValidation.h"

EDataValidationResult UDialogueAsset::IsDataValid(FDataValidationContext& Context) const
{
	EDataValidationResult Result = Super::IsDataValid(Context);
	TSet<FName> Ids;
	auto Error = [&](const FString& Msg) { Context.AddError(FText::FromString(Msg)); Result = EDataValidationResult::Invalid; };

	for (const FDialogueNode& N : Nodes)
	{
		if (N.NodeId.IsNone()) { Error(TEXT("Node with empty NodeId")); }
		else if (Ids.Contains(N.NodeId)) { Error(FString::Printf(TEXT("Duplicate NodeId %s"), *N.NodeId.ToString())); }
		Ids.Add(N.NodeId);
	}
	auto CheckTarget = [&](FName Target, FName From)
	{
		if (!Target.IsNone() && !Ids.Contains(Target))
		{
			Error(FString::Printf(TEXT("%s points to missing node %s"), *From.ToString(), *Target.ToString()));
		}
	};
	for (const FDialogueNode& N : Nodes)
	{
		CheckTarget(N.NextNode, N.NodeId);
		for (const FDialogueChoice& C : N.Choices) { CheckTarget(C.TargetNode, N.NodeId); }
		if (N.Text.IsEmpty()) { Context.AddWarning(FText::FromString(N.NodeId.ToString() + TEXT(" has no text"))); }
	}
	if (Entries.Num() == 0) { Error(TEXT("No entries")); }
	for (const FDialogueEntry& E : Entries) { CheckTarget(E.StartNode, TEXT("Entry")); }
	return Result;
}
#endif
```

Recommended authoring: one asset per conversation (`DA_Dlg_Mira_Harbor`), with node IDs like `Mira_Harbor_010` that
match the VO file names and script line IDs. The last entry should have an empty condition as a fallback.

## 2. Runner

```cpp
// DialogueSubsystem.h
#pragma once
#include "CoreMinimal.h"
#include "Subsystems/WorldSubsystem.h"
#include "DialogueAsset.h"
#include "DialogueSubsystem.generated.h"

class UAudioComponent;

USTRUCT(BlueprintType)
struct FDialogueChoiceView
{
	GENERATED_BODY()
	UPROPERTY(BlueprintReadOnly, Category = "Dialogue") int32 Index = INDEX_NONE;   // pass to Choose()
	UPROPERTY(BlueprintReadOnly, Category = "Dialogue") FText Text;
	UPROPERTY(BlueprintReadOnly, Category = "Dialogue") bool bAvailable = true;
};

DECLARE_DYNAMIC_MULTICAST_DELEGATE_TwoParams(FOnDialogueNode, const FDialogueNode&, Node, const TArray<FDialogueChoiceView>&, Choices);
DECLARE_DYNAMIC_MULTICAST_DELEGATE(FOnDialogueEnded);

UCLASS()
class MYGAME_API UDialogueSubsystem : public UWorldSubsystem
{
	GENERATED_BODY()
public:
	UFUNCTION(BlueprintCallable, Category = "Dialogue") bool StartDialogue(UDialogueAsset* Dialogue);
	/** Continue a line without choices (the player pressed "next", or the voice finished). */
	UFUNCTION(BlueprintCallable, Category = "Dialogue") void Advance();
	UFUNCTION(BlueprintCallable, Category = "Dialogue") void Choose(int32 ChoiceIndex);
	UFUNCTION(BlueprintCallable, Category = "Dialogue") void EndDialogue();
	UFUNCTION(BlueprintPure, Category = "Dialogue") bool IsInDialogue() const { return CurrentDialogue != nullptr; }

	UPROPERTY(BlueprintAssignable, Category = "Dialogue") FOnDialogueNode OnNode;
	UPROPERTY(BlueprintAssignable, Category = "Dialogue") FOnDialogueEnded OnEnded;

	/** Seconds after the voice ends before auto-advancing (0 = wait for input). */
	UPROPERTY(EditAnywhere, BlueprintReadWrite, Category = "Dialogue") float AutoAdvanceDelay = 0.4f;

private:
	void EnterNode(FName NodeId);
	class UStoryStateSubsystem* GetStory() const;

	UPROPERTY() TObjectPtr<UDialogueAsset> CurrentDialogue;
	UPROPERTY() TObjectPtr<UAudioComponent> Voice;
	FName CurrentNodeId;
	FTimerHandle AutoAdvanceHandle;
};
```

```cpp
// DialogueSubsystem.cpp
#include "DialogueSubsystem.h"
#include "StoryStateSubsystem.h"
#include "Components/AudioComponent.h"
#include "Engine/GameInstance.h"
#include "Engine/World.h"
#include "Kismet/GameplayStatics.h"
#include "Sound/SoundBase.h"
#include "TimerManager.h"

UStoryStateSubsystem* UDialogueSubsystem::GetStory() const
{
	const UGameInstance* GI = GetWorld() ? GetWorld()->GetGameInstance() : nullptr;
	return GI ? GI->GetSubsystem<UStoryStateSubsystem>() : nullptr;
}

bool UDialogueSubsystem::StartDialogue(UDialogueAsset* Dialogue)
{
	if (!Dialogue || IsInDialogue()) { return false; }
	const UStoryStateSubsystem* Story = GetStory();
	for (const FDialogueEntry& Entry : Dialogue->Entries)
	{
		if (!Story || Story->MeetsCondition(Entry.Condition))
		{
			CurrentDialogue = Dialogue;
			EnterNode(Entry.StartNode);
			return true;
		}
	}
	return false;
}

void UDialogueSubsystem::EnterNode(FName NodeId)
{
	GetWorld()->GetTimerManager().ClearTimer(AutoAdvanceHandle);
	if (Voice) { Voice->Stop(); Voice = nullptr; }

	const FDialogueNode* Node = CurrentDialogue ? CurrentDialogue->FindNode(NodeId) : nullptr;
	if (!Node) { EndDialogue(); return; }
	CurrentNodeId = NodeId;

	UStoryStateSubsystem* Story = GetStory();
	if (Story)
	{
		Story->ApplyFlags(Node->FlagsToSet);
		Story->SendStoryEvent(Node->StoryEvent);          // ignored if the tag is invalid
	}

	TArray<FDialogueChoiceView> Views;
	for (int32 i = 0; i < Node->Choices.Num(); ++i)
	{
		const FDialogueChoice& C = Node->Choices[i];
		const bool bOk = !Story || Story->MeetsCondition(C.Condition);
		if (!bOk && !C.bShowWhenLocked) { continue; }
		FDialogueChoiceView& View = Views.AddDefaulted_GetRef();
		View.Index = i; View.Text = C.Text; View.bAvailable = bOk;
	}

	// Synchronous load keeps the sketch short. For production, preload the conversation's VO
	// with UAssetManager::GetStreamableManager().RequestAsyncLoad when the player approaches the speaker.
	if (USoundBase* Sound = Node->Voice.LoadSynchronous())
	{
		Voice = UGameplayStatics::SpawnSound2D(GetWorld(), Sound);
		if (Node->Choices.Num() == 0 && AutoAdvanceDelay > 0.f)
		{
			GetWorld()->GetTimerManager().SetTimer(AutoAdvanceHandle, this, &UDialogueSubsystem::Advance,
				Sound->GetDuration() + AutoAdvanceDelay, false);
		}
	}
	OnNode.Broadcast(*Node, Views);    // UI shows speaker, text (subtitle) and choices
}

void UDialogueSubsystem::Advance()
{
	const FDialogueNode* Node = CurrentDialogue ? CurrentDialogue->FindNode(CurrentNodeId) : nullptr;
	if (!Node || Node->Choices.Num() > 0) { return; }   // choices require Choose()
	EnterNode(Node->NextNode);                           // None -> EndDialogue
}

void UDialogueSubsystem::Choose(int32 ChoiceIndex)
{
	const FDialogueNode* Node = CurrentDialogue ? CurrentDialogue->FindNode(CurrentNodeId) : nullptr;
	if (!Node || !Node->Choices.IsValidIndex(ChoiceIndex)) { return; }
	const FDialogueChoice& Choice = Node->Choices[ChoiceIndex];
	UStoryStateSubsystem* Story = GetStory();
	if (Story && !Story->MeetsCondition(Choice.Condition)) { return; }   // locked
	if (Story) { Story->ApplyFlags(Choice.FlagsToSet); }
	EnterNode(Choice.TargetNode);
}

void UDialogueSubsystem::EndDialogue()
{
	if (!CurrentDialogue) { return; }
	GetWorld()->GetTimerManager().ClearTimer(AutoAdvanceHandle);
	if (Voice) { Voice->Stop(); Voice = nullptr; }
	CurrentDialogue = nullptr;
	CurrentNodeId = NAME_None;
	OnEnded.Broadcast();
}
```

Design notes: dialogue is a *world* subsystem because it owns audio and timers and should reset on map change,
while flags live in the *game instance* subsystem and survive it. Choice indices refer to the authored array
(`View.Index`), so hidden choices don't shift the index the UI sends back.

## 3. Hooking up gameplay and UI

- **Starting**: an interactable NPC component holds `TObjectPtr<UDialogueAsset> Dialogue`. On interact, call
  `GetWorld()->GetSubsystem<UDialogueSubsystem>()->StartDialogue(Dialogue)`. In Blueprint use
  *Get World Subsystem (DialogueSubsystem)*.
- **Widget** (UMG): on construct, bind `OnNode` and `OnEnded`. On `OnNode`, look up the speaker's `UCharacterInfo`
  by tag (display name and color), set the subtitle text, and build choice buttons (disabled when
  `bAvailable` is false) that call `Choose(View.Index)`. With no choices, a "continue" input calls `Advance()`.
  Set the input mode to UI (or Game and UI) while in dialogue, and restore it in `OnEnded`.
- **Camera and animation**: optional. Blend to a dialogue camera, or play a Level Sequence per important
  conversation (`SKILL.md` §10). Speaker gestures can be keyed off a `Mood` tag added to the node.
- **Multiplayer**: run dialogue locally per player, and send flag changes to the server through an RPC on the
  PlayerController that validates them.

## 4. Data Table variant (spreadsheet-authored)

For writers working in a spreadsheet, keep *linear* lines in a `UDataTable` whose row name is the node ID (a row
struct like `FBarkRow` below, with `Conversation`, `NextNode` and a `DirectionNote` for VO instead of the bark
fields), and look rows up with `Table->FindRow<FRow>(NodeId, TEXT("Dialogue"))`. Choices don't fit a flat table
well, so keep branching conversations in Data Assets, or put choices in a second table keyed by `FromNode`.

## 5. Barks

```cpp
USTRUCT(BlueprintType)
struct FBarkRow : public FTableRowBase
{
	GENERATED_BODY()
	UPROPERTY(EditAnywhere, BlueprintReadOnly) FGameplayTag Speaker;    // Character.Guard (or a role tag)
	UPROPERTY(EditAnywhere, BlueprintReadOnly) FGameplayTag Context;    // Bark.Combat.Reload
	UPROPERTY(EditAnywhere, BlueprintReadOnly) FText Line;
	UPROPERTY(EditAnywhere, BlueprintReadOnly) TSoftObjectPtr<USoundBase> Voice;
	UPROPERTY(EditAnywhere, BlueprintReadOnly) FStoryCondition Condition;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, meta = (ClampMin = "0")) float Weight = 1.f;
	UPROPERTY(EditAnywhere, BlueprintReadOnly, meta = (ClampMin = "0")) float CooldownSeconds = 20.f;
	UPROPERTY(EditAnywhere, BlueprintReadOnly) int32 Priority = 0;     // higher interrupts lower
};
```

Selection (inside a `UBarkSubsystem : UWorldSubsystem`, with `TMap<FName, double> LastPlayed` per row and
`TMap<TWeakObjectPtr<AActor>, double> SpeakerBusyUntil`):
1. Reject the request if the speaker is busy (unless its priority is higher than the playing bark's) or the global cap is reached (for example 2 barks at once).
2. Candidates are the rows whose `Context` matches the request (`MatchesTag`, so `Bark.Combat.Reload.Shotgun` can fall
   back to `Bark.Combat.Reload`), whose `Speaker` matches, whose `Condition` passes, and that are off cooldown.
3. Remove the row played most recently by this speaker (no back-to-back repeats).
4. Pick a weighted random row (`FMath::FRandRange(0, TotalWeight)`), play it attached to the speaker
   (`UGameplayStatics::SpawnSoundAttached`), show a world-space or subtitle line, and record the times.
Iterate the table with `Table->GetAllRows<FBarkRow>(TEXT("Barks"), OutRows)` once, and cache the rows by context at startup.

Verify: Validate Assets passes (unique IDs, no dangling targets, a fallback entry with an empty condition),
VO file names match node IDs, and flags set by choices survive save and load.
