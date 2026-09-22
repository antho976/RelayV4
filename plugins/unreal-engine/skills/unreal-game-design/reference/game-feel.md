# Game Feel: Implementation Recipes

Every value below is a starting point. Expose each one as a `UPROPERTY(EditDefaultsOnly)` on a Data
Asset (for example `UFeelTuning : UPrimaryDataAsset`) and tune it in PIE.

## Reference values

| Effect | Typical range | Notes |
|---|---|---|
| Hit-stop (light hit) | 30–60 ms | Freeze both attacker and victim |
| Hit-stop (heavy or kill) | 80–150 ms | Longer only for rare, big moments |
| Input buffer window | 100–200 ms | Per action. Attacks and jumps usually differ |
| Coyote time | 80–150 ms | Platformers toward the high end |
| Camera shake, light hit | 0.1–0.3 s, small amplitude | Scale by damage, and multiply by the user's setting |
| FOV kick (dash/sprint) | +5 to +10 degrees, 0.15 s in and 0.3 s out | Ease with a curve |
| Hit flash | 1–3 frames at full intensity, then fade over about 0.1 s | Material scalar parameter |
| Anticipation (enemy tell) | at least 300 ms for a dodgeable attack | Longer for less-skilled audiences |
| Pitch variation on repeated SFX | ±5–10% | Randomize through MetaSounds or the Sound Cue Modulator (see `unreal-audio`) |

## Hit-stop (per-actor, multiplayer-safe)

`AActor::CustomTimeDilation` scales only that actor's tick delta. The world timer manager is not
affected by it, so a world timer can restore the actor.

```cpp
// FeelLibrary.h
#pragma once
#include "CoreMinimal.h"
#include "Kismet/BlueprintFunctionLibrary.h"
#include "FeelLibrary.generated.h"

UCLASS()
class MYGAME_API UFeelLibrary : public UBlueprintFunctionLibrary
{
	GENERATED_BODY()
public:
	/** Freezes the given actors (nearly) for Duration seconds of world time. */
	UFUNCTION(BlueprintCallable, Category = "Feel")
	static void HitStop(const TArray<AActor*>& Actors, float Duration = 0.06f, float Dilation = 0.02f);
};

// FeelLibrary.cpp
#include "FeelLibrary.h"
#include "GameFramework/Actor.h"
#include "TimerManager.h"
#include "Engine/World.h"

void UFeelLibrary::HitStop(const TArray<AActor*>& Actors, float Duration, float Dilation)
{
	for (AActor* Actor : Actors)
	{
		if (!IsValid(Actor)) { continue; }
		Actor->CustomTimeDilation = Dilation;
		TWeakObjectPtr<AActor> Weak(Actor);
		FTimerHandle Handle;
		Actor->GetWorldTimerManager().SetTimer(Handle, FTimerDelegate::CreateLambda([Weak]()
		{
			if (Weak.IsValid()) { Weak->CustomTimeDilation = 1.f; }
		}), Duration, false);
	}
}
```

Notes:
- Do not use a dilation of exactly 0. Some code divides by the delta, and animation notifies can misbehave. Use about 0.01–0.05.
- `CustomTimeDilation` affects that actor's ticking components (including the skeletal mesh
  animation), but not particles owned by other actors, or sounds.
- Overlapping hit-stops: the second call's timer resets the actor to 1 when it fires, while the
  first timer can fire earlier. If that matters, keep one timer handle per actor (a small component) and extend it.
- Global slow-mo (single player only): `UGameplayStatics::SetGlobalTimeDilation(this, 0.1f)`. The world
  timer manager then runs on dilated time, so a 0.1 s real-time restore needs a timer of
  `0.1f * 0.1f`. In multiplayer, global dilation is authoritative and affects everyone.
- `CustomTimeDilation` is not replicated. Apply it on each machine from the replicated hit event.

## Camera shake

UE5 camera shakes are `UCameraShakeBase` subclasses with a root shake pattern. The pattern classes
(Perlin Noise, Wave Oscillator, Sequence, Composite) come from the **EngineCameras** plugin, which is
enabled by default.

Editor steps for the human: Content Browser > Add > Blueprint Class > search `CameraShakeBase` >
name it `CS_HitLight`. In its defaults, set **Root Shake Pattern** to *Perlin Noise Camera Shake
Pattern*, set Duration 0.2 and Blend In/Out 0.02/0.1, and add Location Amplitude about 2–5 and Rotation
Amplitude about 0.5–1.5 with Frequency about 10–25.

Triggering it:

```cpp
#include "Camera/CameraShakeBase.h"
#include "Kismet/GameplayStatics.h"

// Local player only (UI, own weapon recoil):
if (APlayerController* PC = Cast<APlayerController>(GetController()))
{
	PC->ClientStartCameraShake(HitShakeClass, ShakeScale * UserShakeMultiplier);
}

// Every local player near a world event (explosion). Call it on each machine:
UGameplayStatics::PlayWorldCameraShake(this, ExplosionShakeClass, GetActorLocation(),
	/*InnerRadius*/ 300.f, /*OuterRadius*/ 2500.f, /*Falloff*/ 1.f);
```

`HitShakeClass` is a `UPROPERTY(EditDefaultsOnly) TSubclassOf<UCameraShakeBase>`. Always multiply the
scale by an accessibility setting (0 disables it).

## Force feedback

Create a Force Feedback Effect asset (Content Browser > Add > Miscellaneous > Force Feedback
Effect). Play it on the owning client:

```cpp
#include "GameFramework/ForceFeedbackEffect.h"
if (APlayerController* PC = Cast<APlayerController>(GetController()))
{
	FForceFeedbackParameters Params;
	Params.Tag = TEXT("Hit");       // the same tag replaces a running effect instead of stacking it
	PC->ClientPlayForceFeedback(HitRumble, Params);
}
```

Also respect a user "vibration" toggle.

## Input buffering (Enhanced Input)

Record intent on press, and consume it when the character can act. Enhanced Input setup: an Input
Action with `ETriggerEvent::Started` for "pressed this frame".

```cpp
// In the character or a combat component header:
UPROPERTY(EditDefaultsOnly, Category = "Feel") float AttackBufferWindow = 0.15f;
double AttackPressedTime = -1.0;   // not a UPROPERTY; transient

void OnAttackStarted(const FInputActionValue& Value)
{
	AttackPressedTime = GetWorld()->GetTimeSeconds();
	TryConsumeAttack();
}

void TryConsumeAttack()   // also call it when an attack's cancel window opens (AnimNotify) or on landing
{
	if (AttackPressedTime < 0.0) { return; }
	const double Age = GetWorld()->GetTimeSeconds() - AttackPressedTime;
	if (Age > AttackBufferWindow) { AttackPressedTime = -1.0; return; }
	if (CanAttackNow())
	{
		AttackPressedTime = -1.0;
		StartAttack();
	}
}

// SetupPlayerInputComponent:
if (UEnhancedInputComponent* EIC = Cast<UEnhancedInputComponent>(PlayerInputComponent))
{
	EIC->BindAction(AttackAction, ETriggerEvent::Started, this, &AMyCharacter::OnAttackStarted);
}
```

Module dependency: `"EnhancedInput"`. Headers: `EnhancedInputComponent.h`, `InputActionValue.h`.
Open "cancel windows" with an Anim Notify State on the attack montage, and call `TryConsumeAttack`
from its begin. If Gameplay Abilities are in use, GAS has its own input and activation model (see `unreal-gas`).

A jump buffer works the same way. Store the press time, and in `Landed(const FHitResult&)` (a
virtual on `ACharacter`) call `Jump()` if the press is within the window.

## Coyote time (ACharacter)

The stock `ACharacter` allows only `JumpMaxCount` jumps, and counts a jump started while falling
as already one jump used (see `ACharacter::CheckJumpInput`). To allow a jump shortly after walking
off a ledge, override `CanJumpInternal`:

```cpp
// Header
UPROPERTY(EditDefaultsOnly, Category = "Feel") float CoyoteTime = 0.12f;
double LeftGroundTime = -1.0;
bool bJumpedSinceGrounded = false;

virtual bool CanJumpInternal_Implementation() const override;
virtual void OnMovementModeChanged(EMovementMode PrevMovementMode, uint8 PreviousCustomMode = 0) override;
virtual void OnJumped_Implementation() override;
virtual void Landed(const FHitResult& Hit) override;

// Source
void AMyCharacter::OnMovementModeChanged(EMovementMode PrevMovementMode, uint8 PreviousCustomMode)
{
	Super::OnMovementModeChanged(PrevMovementMode, PreviousCustomMode);
	if (PrevMovementMode == MOVE_Walking && GetCharacterMovement()->IsFalling())
	{
		LeftGroundTime = GetWorld()->GetTimeSeconds();
	}
}
void AMyCharacter::OnJumped_Implementation() { Super::OnJumped_Implementation(); bJumpedSinceGrounded = true; }
void AMyCharacter::Landed(const FHitResult& Hit) { Super::Landed(Hit); bJumpedSinceGrounded = false; LeftGroundTime = -1.0; }

bool AMyCharacter::CanJumpInternal_Implementation() const
{
	if (Super::CanJumpInternal_Implementation()) { return true; }
	const UCharacterMovementComponent* Move = GetCharacterMovement();
	const bool bInCoyoteWindow = LeftGroundTime >= 0.0
		&& (GetWorld()->GetTimeSeconds() - LeftGroundTime) <= CoyoteTime;
	return Move && Move->IsFalling() && !bJumpedSinceGrounded && bInCoyoteWindow;
}
```

Caveats:
- Check this against `ACharacter::CheckJumpInput` and `CanJumpInternal_Implementation` in *your*
  engine version (open `Engine/Source/Runtime/Engine/Private/Character.cpp`). The jump counting
  logic has changed between versions.
- In networked games, jump is predicted and replayed through `UCharacterMovementComponent`. A
  wall-clock coyote timer can disagree between client and server. For multiplayer, track the
  timer in movement-component time, or accept occasional corrections. See `unreal-multiplayer`.

## Variable jump height and better falling

- `JumpMaxHoldTime` (on `ACharacter`, for example 0.2–0.35) makes a longer press give a higher jump.
- Falling faster than you rise feels better. In Tick, set
  `GetCharacterMovement()->GravityScale = Velocity.Z < 0 ? FallGravity : RiseGravity;`, or use a custom movement mode.
- `AirControl` (0.3–0.8 for platformers) and `BrakingDecelerationFalling` control how steerable the character is in the air.

## Hit flash and material feedback

Add a scalar parameter `HitFlash` to the character material (emissive = `HitFlash * FlashColor`).
On a hit, set it on a dynamic material instance, and fade it with a Timeline or a timer. Use Custom
Primitive Data (`SetCustomPrimitiveDataFloat`) to avoid a separate MID per mesh. See `unreal-materials-vfx`.

## FOV kick and camera lag

- `USpringArmComponent`: `bEnableCameraLag`, `CameraLagSpeed` (about 10–15), and `bEnableCameraRotationLag`.
- FOV kick: lerp `UCameraComponent::SetFieldOfView` toward the target with a `UCurveFloat` over time.
- Look-ahead: offset the spring arm's `SocketOffset` or `TargetOffset` by a clamped fraction of the velocity.

## Checklist for "does it feel good?"

1. Record gameplay at 60 fps and step through it frame by frame. Count the frames from press to the first visible response (aim for 3 frames or fewer at 60 Hz).
2. Try it with every juice layer off. The mechanic should still read.
3. Turn each layer on one at a time, and remove any layer that doesn't add clarity or weight.
4. Test with a gamepad and with keyboard and mouse, and in a packaged build.
5. Confirm the accessibility multiplier zeroes out shake, flash and rumble.
