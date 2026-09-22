# Animation C++: UAnimInstance, notifies, montages

Build.cs: `"Engine"` covers `UAnimInstance`, montages and notifies. Add `"AnimGraphRuntime"`
for `UKismetAnimationLibrary`, `USequenceEvaluatorLibrary`, `USequencePlayerLibrary` and anim
node reference types. Distance matching functions live in the Animation Locomotion Library
plugin's runtime module (`AnimationLocomotionLibraryRuntime`; confirm in its `.uplugin`).

## 1. Thread-safe UAnimInstance

Pattern: game thread copies raw data (`NativeUpdateAnimation`), worker thread derives what the
AnimGraph reads (`NativeThreadSafeUpdateAnimation`). AnimGraph reads only `UPROPERTY` members.

```cpp
// MyCharacterAnimInstance.h
#pragma once
#include "CoreMinimal.h"
#include "Animation/AnimInstance.h"
#include "MyCharacterAnimInstance.generated.h"

class ACharacter;
class UCharacterMovementComponent;

UCLASS()
class MYGAME_API UMyCharacterAnimInstance : public UAnimInstance
{
    GENERATED_BODY()
protected:
    virtual void NativeInitializeAnimation() override;
    virtual void NativeUpdateAnimation(float DeltaSeconds) override;            // game thread
    virtual void NativeThreadSafeUpdateAnimation(float DeltaSeconds) override;  // worker thread

    // --- Read by the AnimGraph ---
    UPROPERTY(BlueprintReadOnly, Category = "Locomotion") float GroundSpeed = 0.f;
    UPROPERTY(BlueprintReadOnly, Category = "Locomotion") float Direction = 0.f;      // -180..180
    UPROPERTY(BlueprintReadOnly, Category = "Locomotion") bool  bHasAcceleration = false;
    UPROPERTY(BlueprintReadOnly, Category = "Locomotion") bool  bShouldMove = false;
    UPROPERTY(BlueprintReadOnly, Category = "Locomotion") bool  bIsFalling = false;
    UPROPERTY(BlueprintReadOnly, Category = "Locomotion") bool  bIsCrouching = false;
    UPROPERTY(BlueprintReadOnly, Category = "Aim")        float AimPitch = 0.f;
    UPROPERTY(BlueprintReadOnly, Category = "Aim")        float AimYaw = 0.f;

    UPROPERTY(EditDefaultsOnly, Category = "Locomotion") float MoveSpeedThreshold = 3.f; // cm/s

private:
    UPROPERTY(Transient) TObjectPtr<ACharacter> Character;
    UPROPERTY(Transient) TObjectPtr<UCharacterMovementComponent> Movement;

    // Game-thread snapshot, consumed on the worker thread
    FVector  Velocity = FVector::ZeroVector;
    FVector  Acceleration = FVector::ZeroVector;
    FRotator ActorRotation = FRotator::ZeroRotator;
    FRotator AimRotation = FRotator::ZeroRotator;
    bool bFallingSnapshot = false;
    bool bCrouchSnapshot = false;
};
```

```cpp
// MyCharacterAnimInstance.cpp
#include "MyCharacterAnimInstance.h"
#include "GameFramework/Character.h"
#include "GameFramework/CharacterMovementComponent.h"
#include "KismetAnimationLibrary.h"   // AnimGraphRuntime

void UMyCharacterAnimInstance::NativeInitializeAnimation()
{
    Super::NativeInitializeAnimation();
    Character = Cast<ACharacter>(TryGetPawnOwner());   // null in the AnimBP editor preview
    Movement = Character ? Character->GetCharacterMovement() : nullptr;
}

void UMyCharacterAnimInstance::NativeUpdateAnimation(float DeltaSeconds)
{
    Super::NativeUpdateAnimation(DeltaSeconds);
    if (!Character || !Movement) { return; }

    Velocity = Character->GetVelocity();
    Acceleration = Movement->GetCurrentAcceleration();  // input intent (see note on simulated proxies)
    ActorRotation = Character->GetActorRotation();
    AimRotation = Character->GetBaseAimRotation();      // uses replicated RemoteViewPitch on proxies
    bFallingSnapshot = Movement->IsFalling();
    bCrouchSnapshot = Character->bIsCrouched;
}

void UMyCharacterAnimInstance::NativeThreadSafeUpdateAnimation(float DeltaSeconds)
{
    Super::NativeThreadSafeUpdateAnimation(DeltaSeconds);

    GroundSpeed = Velocity.Size2D();
    bHasAcceleration = !Acceleration.IsNearlyZero();
    bShouldMove = GroundSpeed > MoveSpeedThreshold && bHasAcceleration;
    bIsFalling = bFallingSnapshot;
    bIsCrouching = bCrouchSnapshot;
    Direction = UKismetAnimationLibrary::CalculateDirection(Velocity, ActorRotation);

    const FRotator Delta = (AimRotation - ActorRotation).GetNormalized();
    AimPitch = Delta.Pitch;
    AimYaw = Delta.Yaw;
}
```

Notes:
- Only touch other objects (character, components, world) in `NativeUpdateAnimation` or
  `NativeInitializeAnimation`. The worker-thread function must use only this instance's members.
- `CalculateDirection` replaced the deprecated `UAnimInstance::CalculateDirection`.
- Whether `GetCurrentAcceleration()` is meaningful on simulated proxies depends on the CMC
  version/settings; if remote characters never "start", derive intent from velocity change for
  proxies. Test with two clients.
- Human step: create an Anim Blueprint with **Parent Class** = `MyCharacterAnimInstance` (Content
  Browser > Add > Animation > Animation Blueprint, pick the skeleton and the parent class), or
  reparent an existing one (Class Settings > Parent Class). Set it on the character mesh's Anim
  Class. Python: `unreal.BlueprintEditorLibrary.reparent_blueprint(anim_bp, cls)`.

## 2. Thread-safe functions callable from AnimGraph node bindings

```cpp
// In the anim instance header
float DistanceToStop = 0.f;   // filled in NativeThreadSafeUpdateAnimation from a game-thread snapshot

UFUNCTION(BlueprintCallable, Category = "Locomotion", meta = (BlueprintThreadSafe))
void UpdateStopState(const FAnimUpdateContext& Context, const FAnimNodeReference& Node);
```

```cpp
#include "SequenceEvaluatorLibrary.h"        // AnimGraphRuntime
#include "AnimDistanceMatchingLibrary.h"     // AnimationLocomotionLibraryRuntime

void UMyCharacterAnimInstance::UpdateStopState(const FAnimUpdateContext& Context, const FAnimNodeReference& Node)
{
    EAnimNodeReferenceConversionResult Result;
    const FSequenceEvaluatorReference Evaluator = USequenceEvaluatorLibrary::ConvertToSequenceEvaluator(Node, Result);
    if (Result == EAnimNodeReferenceConversionResult::Succeeded)
    {
        // DistanceToStop computed in NativeThreadSafeUpdateAnimation (e.g. from
        // UAnimCharacterMovementLibrary::PredictGroundMovementStopLocation, gathered on the game thread)
        UAnimDistanceMatchingLibrary::DistanceMatchToTarget(Evaluator, DistanceToStop, TEXT("Distance"));
    }
}
```

Bind it in the AnimGraph (human): select the Sequence Evaluator node in the Stop state >
Details > Functions > On Update > pick `UpdateStopState`. Check the exact parameter list of
`DistanceMatchToTarget` / `AdvanceTimeByDistanceMatching` in the plugin headers for your version.

## 3. AnimNotify

```cpp
// AnimNotify_Footstep.h
#pragma once
#include "CoreMinimal.h"
#include "Animation/AnimNotifies/AnimNotify.h"
#include "AnimNotify_Footstep.generated.h"

UCLASS(meta = (DisplayName = "Footstep"))
class MYGAME_API UAnimNotify_Footstep : public UAnimNotify
{
    GENERATED_BODY()
public:
    virtual void Notify(USkeletalMeshComponent* MeshComp, UAnimSequenceBase* Animation,
                        const FAnimNotifyEventReference& EventReference) override;
    virtual FString GetNotifyName_Implementation() const override { return TEXT("Footstep"); }

    UPROPERTY(EditAnywhere, Category = "Footstep") FName FootSocket = TEXT("foot_l");
};
```

```cpp
// AnimNotify_Footstep.cpp
#include "AnimNotify_Footstep.h"
#include "Components/SkeletalMeshComponent.h"
#include "FootstepInterface.h"   // your UINTERFACE implemented by the character

void UAnimNotify_Footstep::Notify(USkeletalMeshComponent* MeshComp, UAnimSequenceBase* Animation,
                                  const FAnimNotifyEventReference& EventReference)
{
    Super::Notify(MeshComp, Animation, EventReference);
    if (!MeshComp) { return; }
    AActor* Owner = MeshComp->GetOwner();   // in the asset editor preview this is a preview actor
    if (Owner && Owner->Implements<UFootstepInterface>())
    {
        IFootstepInterface::Execute_HandleFootstep(Owner, FootSocket);
    }
}
```

- The notify object is **shared by every character** playing that animation. Never store
  per-character state on it; route to the owner (interface or component).
- Notifies run in the editor preview too; guard gameplay code (interface check as above, or
  `MeshComp->GetWorld()->IsGameWorld()`).
- 5.0+ signatures take `const FAnimNotifyEventReference&`; the older two-argument overloads are
  deprecated.

## 4. AnimNotifyState (hit window)

```cpp
// AnimNotifyState_WeaponTrace.h
#pragma once
#include "CoreMinimal.h"
#include "Animation/AnimNotifies/AnimNotifyState.h"
#include "AnimNotifyState_WeaponTrace.generated.h"

UCLASS(meta = (DisplayName = "Weapon Trace Window"))
class MYGAME_API UAnimNotifyState_WeaponTrace : public UAnimNotifyState
{
    GENERATED_BODY()
public:
    virtual void NotifyBegin(USkeletalMeshComponent* MeshComp, UAnimSequenceBase* Animation, float TotalDuration,
                             const FAnimNotifyEventReference& EventReference) override;
    virtual void NotifyTick(USkeletalMeshComponent* MeshComp, UAnimSequenceBase* Animation, float FrameDeltaTime,
                            const FAnimNotifyEventReference& EventReference) override;
    virtual void NotifyEnd(USkeletalMeshComponent* MeshComp, UAnimSequenceBase* Animation,
                           const FAnimNotifyEventReference& EventReference) override;
};
```

Implementation: find the owner's melee component (`MeshComp->GetOwner()->FindComponentByClass<UMyMeleeComponent>()`)
and call `BeginTrace()` / `TickTrace()` / `EndTrace()`. The component holds per-swing state
(already-hit actors). Only apply damage where you have authority (`Owner->HasAuthority()`).
`NotifyEnd` is not guaranteed to fire if the montage is interrupted in some edge cases; also
reset the component's state when the montage ends (end delegate below).

## 5. Playing montages from gameplay code

```cpp
// AMyCharacter (excerpt)
UPROPERTY(EditDefaultsOnly, Category = "Combat") TObjectPtr<UAnimMontage> AttackMontage;
void OnAttackMontageEnded(UAnimMontage* Montage, bool bInterrupted);

void AMyCharacter::PlayAttack()
{
    UAnimInstance* Anim = GetMesh() ? GetMesh()->GetAnimInstance() : nullptr;
    if (!Anim || !AttackMontage) { return; }

    const float Length = Anim->Montage_Play(AttackMontage, 1.f); // 0 => failed (no slot node, wrong skeleton)
    if (Length > 0.f)
    {
        FOnMontageEnded EndDelegate;
        EndDelegate.BindUObject(this, &AMyCharacter::OnAttackMontageEnded);
        Anim->Montage_SetEndDelegate(EndDelegate, AttackMontage);
    }
}

void AMyCharacter::OnAttackMontageEnded(UAnimMontage* Montage, bool bInterrupted)
{
    // reset combo / melee state
}
```

Other calls: `Montage_JumpToSection(FName("Recovery"), Montage)`, `Montage_SetNextSection(From, To, Montage)`,
`Montage_Stop(BlendOutTime, Montage)`, `Montage_IsPlaying(Montage)`, `Montage_SetBlendingOutDelegate`,
`Montage_GetCurrentSection(Montage)`. Montage notifies of type **Montage Notify** (Play Montage
Notify) broadcast through `UAnimInstance::OnPlayMontageNotifyBegin` / `OnPlayMontageNotifyEnd`.

Multiplayer: montages do not replicate. Options, best first:
1. GAS `UAbilityTask_PlayMontageAndWait` (predicted, replicated to simulated proxies) - see `unreal-gas`.
2. Server-authoritative state (e.g. replicated `AttackCounter` with `OnRep`) that each machine
   turns into `Montage_Play`.
3. `NetMulticast` RPC from the server (unreliable for cosmetic, reliable only if gameplay depends on it).

## 6. Linked anim layers from C++

```cpp
UPROPERTY(EditDefaultsOnly, Category = "Animation") TSubclassOf<UAnimInstance> RifleAnimLayers;

void AMyCharacter::OnWeaponEquipped()
{
    if (RifleAnimLayers) { GetMesh()->LinkAnimClassLayers(RifleAnimLayers); }
}
void AMyCharacter::OnWeaponUnequipped()
{
    if (RifleAnimLayers) { GetMesh()->UnlinkAnimClassLayers(RifleAnimLayers); }
}
```

`RifleAnimLayers` is an AnimBP that implements the same Anim Layer Interface the main AnimBP
uses. Linking replaces every implemented layer at once; unimplemented layers fall back to the
main AnimBP's default implementation.

## 7. Mesh tick settings that affect notifies and cost

`USkeletalMeshComponent::VisibilityBasedAnimTickOption`:
- `AlwaysTickPoseAndRefreshBones` - always tick and update bones (player characters, server
  hit detection against bones).
- `AlwaysTickPose` - tick pose (notifies fire) but skip bone refresh when not rendered.
- `OnlyTickMontagesWhenNotRendered` - montages advance (their notifies fire) when off-screen.
- `OnlyTickPoseWhenRendered` - cheapest; nothing advances off-screen. Not for anything the server
  depends on.
On a dedicated server nothing is rendered, so the option decides whether notifies fire there.
