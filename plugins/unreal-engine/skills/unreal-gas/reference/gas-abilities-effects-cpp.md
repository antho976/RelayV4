# GAS abilities and effects in C++ (melee ability, damage execution, cost MMC)

Continues `gas-setup-cpp.md`: uses its `UMyAttributeSet` (Health, MaxHealth, Mana, MaxMana,
Armor, meta attribute IncomingDamage) and native tags (`TAG_Event_Montage_Hit`,
`TAG_Data_Damage`, `TAG_State_Dead`). Replace `MYGAME_API` with the module's API macro.
The GE assets these classes plug into are listed in section 6 of `gas-setup-cpp.md`.

## 1. Melee ability with a montage and a hit event

The montage has an AnimNotify at the hit frame that calls
`UAbilitySystemBlueprintLibrary::SendGameplayEventToActor(Owner, Event.Montage.Hit, Payload)`
with `Payload.Target` set to the actor hit (trace in the notify or in the ability).

```cpp
// GA_MeleeAttack.h
#pragma once
#include "CoreMinimal.h"
#include "Abilities/GameplayAbility.h"
#include "GA_MeleeAttack.generated.h"

UCLASS()
class MYGAME_API UGA_MeleeAttack : public UGameplayAbility
{
    GENERATED_BODY()
public:
    UGA_MeleeAttack();
    virtual void ActivateAbility(const FGameplayAbilitySpecHandle Handle, const FGameplayAbilityActorInfo* ActorInfo,
        const FGameplayAbilityActivationInfo ActivationInfo, const FGameplayEventData* TriggerEventData) override;

protected:
    UPROPERTY(EditDefaultsOnly, Category = "Attack") TObjectPtr<UAnimMontage> AttackMontage;
    UPROPERTY(EditDefaultsOnly, Category = "Attack") TSubclassOf<UGameplayEffect> DamageEffect;
    UPROPERTY(EditDefaultsOnly, Category = "Attack") float BaseDamage = 20.f;

    UFUNCTION() void OnMontageFinished();
    UFUNCTION() void OnMontageCancelled();
    UFUNCTION() void OnHitEvent(FGameplayEventData Payload);
};
```
```cpp
// GA_MeleeAttack.cpp
#include "GA_MeleeAttack.h"
#include "MyGameTags.h"
#include "AbilitySystemComponent.h"
#include "AbilitySystemGlobals.h"
#include "Abilities/Tasks/AbilityTask_PlayMontageAndWait.h"
#include "Abilities/Tasks/AbilityTask_WaitGameplayEvent.h"

UGA_MeleeAttack::UGA_MeleeAttack()
{
    InstancingPolicy = EGameplayAbilityInstancingPolicy::InstancedPerActor;
    NetExecutionPolicy = EGameplayAbilityNetExecutionPolicy::LocalPredicted;
    ActivationBlockedTags.AddTag(TAG_State_Dead);
    // Asset tag identifying this ability. 5.5+: use SetAssetTags(); older: AbilityTags.AddTag(...).
}

void UGA_MeleeAttack::ActivateAbility(const FGameplayAbilitySpecHandle Handle, const FGameplayAbilityActorInfo* ActorInfo,
    const FGameplayAbilityActivationInfo ActivationInfo, const FGameplayEventData* TriggerEventData)
{
    if (!CommitAbility(Handle, ActorInfo, ActivationInfo)) // applies cost + cooldown GEs
    {
        EndAbility(Handle, ActorInfo, ActivationInfo, true, true);
        return;
    }

    UAbilityTask_PlayMontageAndWait* MontageTask = UAbilityTask_PlayMontageAndWait::CreatePlayMontageAndWaitProxy(
        this, NAME_None, AttackMontage, 1.f);
    MontageTask->OnCompleted.AddDynamic(this, &UGA_MeleeAttack::OnMontageFinished);
    MontageTask->OnBlendOut.AddDynamic(this, &UGA_MeleeAttack::OnMontageFinished);
    MontageTask->OnInterrupted.AddDynamic(this, &UGA_MeleeAttack::OnMontageCancelled);
    MontageTask->OnCancelled.AddDynamic(this, &UGA_MeleeAttack::OnMontageCancelled);
    MontageTask->ReadyForActivation();

    UAbilityTask_WaitGameplayEvent* HitTask = UAbilityTask_WaitGameplayEvent::WaitGameplayEvent(
        this, TAG_Event_Montage_Hit, nullptr, /*OnlyTriggerOnce*/ false, /*OnlyMatchExact*/ true);
    HitTask->EventReceived.AddDynamic(this, &UGA_MeleeAttack::OnHitEvent);
    HitTask->ReadyForActivation();
}

void UGA_MeleeAttack::OnHitEvent(FGameplayEventData Payload)
{
    // Damage is authoritative: only the server applies it.
    AActor* Avatar = GetAvatarActorFromActorInfo();
    if (!Avatar || !Avatar->HasAuthority() || !DamageEffect) { return; }

    const AActor* HitActor = Payload.Target;
    UAbilitySystemComponent* TargetASC = UAbilitySystemGlobals::GetAbilitySystemComponentFromActor(HitActor);
    if (!TargetASC) { return; }

    FGameplayEffectSpecHandle Spec = MakeOutgoingGameplayEffectSpec(DamageEffect, GetAbilityLevel());
    Spec.Data->SetSetByCallerMagnitude(TAG_Data_Damage, BaseDamage);
    GetAbilitySystemComponentFromActorInfo()->ApplyGameplayEffectSpecToTarget(*Spec.Data.Get(), TargetASC);
}

void UGA_MeleeAttack::OnMontageFinished()
{
    EndAbility(GetCurrentAbilitySpecHandle(), GetCurrentActorInfo(), GetCurrentActivationInfo(), true, false);
}

void UGA_MeleeAttack::OnMontageCancelled()
{
    EndAbility(GetCurrentAbilitySpecHandle(), GetCurrentActorInfo(), GetCurrentActivationInfo(), true, true);
}
```
`EndAbility` ends all tasks owned by the ability, so the event wait is cleaned up too.

## 2. Damage execution (armor mitigation)

```cpp
// DamageExecution.h
#pragma once
#include "CoreMinimal.h"
#include "GameplayEffectExecutionCalculation.h"
#include "DamageExecution.generated.h"

UCLASS()
class MYGAME_API UDamageExecution : public UGameplayEffectExecutionCalculation
{
    GENERATED_BODY()
public:
    UDamageExecution();
    virtual void Execute_Implementation(const FGameplayEffectCustomExecutionParameters& ExecutionParams,
        FGameplayEffectCustomExecutionOutput& OutExecutionOutput) const override;
};
```
```cpp
// DamageExecution.cpp
#include "DamageExecution.h"
#include "MyAttributeSet.h"
#include "MyGameTags.h"

struct FDamageStatics
{
    DECLARE_ATTRIBUTE_CAPTUREDEF(Armor);
    FDamageStatics() { DEFINE_ATTRIBUTE_CAPTUREDEF(UMyAttributeSet, Armor, Target, false); }
};
static const FDamageStatics& DamageStatics() { static FDamageStatics S; return S; }

UDamageExecution::UDamageExecution()
{
    RelevantAttributesToCapture.Add(DamageStatics().ArmorDef);
}

void UDamageExecution::Execute_Implementation(const FGameplayEffectCustomExecutionParameters& ExecutionParams,
    FGameplayEffectCustomExecutionOutput& OutExecutionOutput) const
{
    const FGameplayEffectSpec& Spec = ExecutionParams.GetOwningSpec();
    FAggregatorEvaluateParameters EvalParams;
    EvalParams.SourceTags = Spec.CapturedSourceTags.GetAggregatedTags();
    EvalParams.TargetTags = Spec.CapturedTargetTags.GetAggregatedTags();

    float Armor = 0.f;
    ExecutionParams.AttemptCalculateCapturedAttributeMagnitude(DamageStatics().ArmorDef, EvalParams, Armor);
    Armor = FMath::Max(Armor, 0.f);

    const float BaseDamage = Spec.GetSetByCallerMagnitude(TAG_Data_Damage, false, 0.f);
    const float FinalDamage = BaseDamage * (100.f / (100.f + Armor));
    if (FinalDamage > 0.f)
    {
        OutExecutionOutput.AddOutputModifier(FGameplayModifierEvaluatedData(
            UMyAttributeSet::GetIncomingDamageAttribute(), EGameplayModOp::Additive, FinalDamage));
    }
}
```
Editor: create `GE_Damage` (Blueprint of GameplayEffect), Duration Policy = Instant,
Executions > + > Calculation Class = `DamageExecution`. Leave the Modifiers array empty.

## 3. Mana cost MMC (10% of max mana)

```cpp
// MMC_ManaCost.h
#pragma once
#include "CoreMinimal.h"
#include "GameplayModMagnitudeCalculation.h"
#include "MMC_ManaCost.generated.h"

UCLASS()
class MYGAME_API UMMC_ManaCost : public UGameplayModMagnitudeCalculation
{
    GENERATED_BODY()
public:
    UMMC_ManaCost();
    virtual float CalculateBaseMagnitude_Implementation(const FGameplayEffectSpec& Spec) const override;
private:
    FGameplayEffectAttributeCaptureDefinition MaxManaDef;
};
```
```cpp
// MMC_ManaCost.cpp
#include "MMC_ManaCost.h"
#include "MyAttributeSet.h"

UMMC_ManaCost::UMMC_ManaCost()
{
    MaxManaDef.AttributeToCapture = UMyAttributeSet::GetMaxManaAttribute();
    MaxManaDef.AttributeSource = EGameplayEffectAttributeCaptureSource::Source;
    MaxManaDef.bSnapshot = false;
    RelevantAttributesToCapture.Add(MaxManaDef);
}

float UMMC_ManaCost::CalculateBaseMagnitude_Implementation(const FGameplayEffectSpec& Spec) const
{
    FAggregatorEvaluateParameters EvalParams;
    EvalParams.SourceTags = Spec.CapturedSourceTags.GetAggregatedTags();
    EvalParams.TargetTags = Spec.CapturedTargetTags.GetAggregatedTags();
    float MaxMana = 0.f;
    GetCapturedAttributeMagnitude(MaxManaDef, Spec, EvalParams, MaxMana);
    return -0.1f * FMath::Max(MaxMana, 0.f); // negative: used with an Additive modifier on Mana
}
```
Editor: `GE_Cost_Attack` Instant, Modifier: Attribute = `MyAttributeSet.Mana`, Op = Add,
Magnitude Calculation Type = Custom Calculated Magnitude, Calculation Class = `MMC_ManaCost`.
