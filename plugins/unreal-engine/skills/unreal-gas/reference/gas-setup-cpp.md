# Minimal GAS setup in C++ (ASC on PlayerState, Enhanced Input)

A complete baseline: native tags, an AttributeSet with replicated Health/Mana/Armor, a
PlayerState that owns the ASC, and a Character that initializes it and binds Enhanced Input
to abilities. The ability, damage execution and cost MMC that complete the example are in
`gas-abilities-effects-cpp.md` (same folder).

Replace `MyGame` / `MYGAME_API` with the project's module name and API macro (check
`ue_project_info` or an existing header in `Source/<Module>/`). Keep header names equal to
class names minus the prefix, as UHT expects.

## 1. Build.cs and .uproject

```csharp
// Source/MyGame/MyGame.Build.cs
PublicDependencyModuleNames.AddRange(new string[] {
    "Core", "CoreUObject", "Engine", "InputCore", "EnhancedInput",
    "GameplayAbilities", "GameplayTags", "GameplayTasks" });
```
```json
"Plugins": [
  { "Name": "GameplayAbilities", "Enabled": true },
  { "Name": "EnhancedInput", "Enabled": true }
]
```
(EnhancedInput is enabled by default in UE 5.1+; listing it is harmless.)

## 2. Native tags

```cpp
// MyGameTags.h
#pragma once
#include "NativeGameplayTags.h"

UE_DECLARE_GAMEPLAY_TAG_EXTERN(TAG_Ability_Attack);
UE_DECLARE_GAMEPLAY_TAG_EXTERN(TAG_Cooldown_Attack);
UE_DECLARE_GAMEPLAY_TAG_EXTERN(TAG_State_Dead);
UE_DECLARE_GAMEPLAY_TAG_EXTERN(TAG_Event_Montage_Hit);
UE_DECLARE_GAMEPLAY_TAG_EXTERN(TAG_Data_Damage);
```
```cpp
// MyGameTags.cpp
#include "MyGameTags.h"

UE_DEFINE_GAMEPLAY_TAG_COMMENT(TAG_Ability_Attack,    "Ability.Attack",     "Melee attack ability");
UE_DEFINE_GAMEPLAY_TAG_COMMENT(TAG_Cooldown_Attack,   "Cooldown.Attack",    "Granted by the attack cooldown GE");
UE_DEFINE_GAMEPLAY_TAG_COMMENT(TAG_State_Dead,        "State.Dead",         "Owner is dead");
UE_DEFINE_GAMEPLAY_TAG_COMMENT(TAG_Event_Montage_Hit, "Event.Montage.Hit",  "Sent by an anim notify at the hit frame");
UE_DEFINE_GAMEPLAY_TAG_COMMENT(TAG_Data_Damage,       "Data.Damage",        "SetByCaller base damage");
```

## 3. AttributeSet

```cpp
// MyAttributeSet.h
#pragma once
#include "CoreMinimal.h"
#include "AttributeSet.h"
#include "AbilitySystemComponent.h"
#include "MyAttributeSet.generated.h"

#define ATTRIBUTE_ACCESSORS(ClassName, PropertyName) \
    GAMEPLAYATTRIBUTE_PROPERTY_GETTER(ClassName, PropertyName) \
    GAMEPLAYATTRIBUTE_VALUE_GETTER(PropertyName) \
    GAMEPLAYATTRIBUTE_VALUE_SETTER(PropertyName) \
    GAMEPLAYATTRIBUTE_VALUE_INITTER(PropertyName)

UCLASS()
class MYGAME_API UMyAttributeSet : public UAttributeSet
{
    GENERATED_BODY()
public:
    UMyAttributeSet();

    virtual void GetLifetimeReplicatedProps(TArray<FLifetimeProperty>& OutLifetimeProps) const override;
    virtual void PreAttributeChange(const FGameplayAttribute& Attribute, float& NewValue) override;
    virtual void PostGameplayEffectExecute(const struct FGameplayEffectModCallbackData& Data) override;

    UPROPERTY(BlueprintReadOnly, ReplicatedUsing = OnRep_Health, Category = "Attributes")
    FGameplayAttributeData Health;
    ATTRIBUTE_ACCESSORS(UMyAttributeSet, Health)

    UPROPERTY(BlueprintReadOnly, ReplicatedUsing = OnRep_MaxHealth, Category = "Attributes")
    FGameplayAttributeData MaxHealth;
    ATTRIBUTE_ACCESSORS(UMyAttributeSet, MaxHealth)

    UPROPERTY(BlueprintReadOnly, ReplicatedUsing = OnRep_Mana, Category = "Attributes")
    FGameplayAttributeData Mana;
    ATTRIBUTE_ACCESSORS(UMyAttributeSet, Mana)

    UPROPERTY(BlueprintReadOnly, ReplicatedUsing = OnRep_MaxMana, Category = "Attributes")
    FGameplayAttributeData MaxMana;
    ATTRIBUTE_ACCESSORS(UMyAttributeSet, MaxMana)

    UPROPERTY(BlueprintReadOnly, ReplicatedUsing = OnRep_Armor, Category = "Attributes")
    FGameplayAttributeData Armor;
    ATTRIBUTE_ACCESSORS(UMyAttributeSet, Armor)

    // Meta attribute: server-only scratch value written by damage GEs. Not replicated.
    UPROPERTY(BlueprintReadOnly, Category = "Attributes|Meta")
    FGameplayAttributeData IncomingDamage;
    ATTRIBUTE_ACCESSORS(UMyAttributeSet, IncomingDamage)

protected:
    UFUNCTION() void OnRep_Health(const FGameplayAttributeData& OldValue);
    UFUNCTION() void OnRep_MaxHealth(const FGameplayAttributeData& OldValue);
    UFUNCTION() void OnRep_Mana(const FGameplayAttributeData& OldValue);
    UFUNCTION() void OnRep_MaxMana(const FGameplayAttributeData& OldValue);
    UFUNCTION() void OnRep_Armor(const FGameplayAttributeData& OldValue);
};
```
```cpp
// MyAttributeSet.cpp
#include "MyAttributeSet.h"
#include "GameplayEffectExtension.h"
#include "Net/UnrealNetwork.h"

UMyAttributeSet::UMyAttributeSet()
{
    InitHealth(100.f); InitMaxHealth(100.f);
    InitMana(50.f);    InitMaxMana(50.f);
    InitArmor(0.f);    InitIncomingDamage(0.f);
}

void UMyAttributeSet::GetLifetimeReplicatedProps(TArray<FLifetimeProperty>& OutLifetimeProps) const
{
    Super::GetLifetimeReplicatedProps(OutLifetimeProps);
    DOREPLIFETIME_CONDITION_NOTIFY(UMyAttributeSet, Health,    COND_None, REPNOTIFY_Always);
    DOREPLIFETIME_CONDITION_NOTIFY(UMyAttributeSet, MaxHealth, COND_None, REPNOTIFY_Always);
    DOREPLIFETIME_CONDITION_NOTIFY(UMyAttributeSet, Mana,      COND_None, REPNOTIFY_Always);
    DOREPLIFETIME_CONDITION_NOTIFY(UMyAttributeSet, MaxMana,   COND_None, REPNOTIFY_Always);
    DOREPLIFETIME_CONDITION_NOTIFY(UMyAttributeSet, Armor,     COND_None, REPNOTIFY_Always);
}

void UMyAttributeSet::PreAttributeChange(const FGameplayAttribute& Attribute, float& NewValue)
{
    Super::PreAttributeChange(Attribute, NewValue);
    // Clamps CurrentValue only. Do not trigger gameplay from here.
    if (Attribute == GetHealthAttribute()) { NewValue = FMath::Clamp(NewValue, 0.f, GetMaxHealth()); }
    else if (Attribute == GetManaAttribute()) { NewValue = FMath::Clamp(NewValue, 0.f, GetMaxMana()); }
}

void UMyAttributeSet::PostGameplayEffectExecute(const FGameplayEffectModCallbackData& Data)
{
    Super::PostGameplayEffectExecute(Data);

    if (Data.EvaluatedData.Attribute == GetIncomingDamageAttribute())
    {
        const float Damage = GetIncomingDamage();
        SetIncomingDamage(0.f);
        if (Damage > 0.f)
        {
            SetHealth(FMath::Clamp(GetHealth() - Damage, 0.f, GetMaxHealth()));
            if (GetHealth() <= 0.f)
            {
                // Server side. Notify the avatar (e.g. an interface call or a delegate on the
                // character) to apply a "State.Dead" GE, ragdoll, notify the GameMode, etc.
                AActor* Avatar = Data.Target.AbilityActorInfo.IsValid()
                    ? Data.Target.AbilityActorInfo->AvatarActor.Get() : nullptr;
                UE_LOG(LogTemp, Log, TEXT("%s died"), *GetNameSafe(Avatar));
            }
        }
    }
    else if (Data.EvaluatedData.Attribute == GetHealthAttribute())
    {
        SetHealth(FMath::Clamp(GetHealth(), 0.f, GetMaxHealth()));
    }
    else if (Data.EvaluatedData.Attribute == GetManaAttribute())
    {
        SetMana(FMath::Clamp(GetMana(), 0.f, GetMaxMana()));
    }
}

void UMyAttributeSet::OnRep_Health(const FGameplayAttributeData& OldValue)    { GAMEPLAYATTRIBUTE_REPNOTIFY(UMyAttributeSet, Health, OldValue); }
void UMyAttributeSet::OnRep_MaxHealth(const FGameplayAttributeData& OldValue) { GAMEPLAYATTRIBUTE_REPNOTIFY(UMyAttributeSet, MaxHealth, OldValue); }
void UMyAttributeSet::OnRep_Mana(const FGameplayAttributeData& OldValue)      { GAMEPLAYATTRIBUTE_REPNOTIFY(UMyAttributeSet, Mana, OldValue); }
void UMyAttributeSet::OnRep_MaxMana(const FGameplayAttributeData& OldValue)   { GAMEPLAYATTRIBUTE_REPNOTIFY(UMyAttributeSet, MaxMana, OldValue); }
void UMyAttributeSet::OnRep_Armor(const FGameplayAttributeData& OldValue)     { GAMEPLAYATTRIBUTE_REPNOTIFY(UMyAttributeSet, Armor, OldValue); }
```

## 4. PlayerState owning the ASC

```cpp
// MyPlayerState.h
#pragma once
#include "CoreMinimal.h"
#include "GameFramework/PlayerState.h"
#include "AbilitySystemInterface.h"
#include "MyPlayerState.generated.h"

class UAbilitySystemComponent;
class UMyAttributeSet;

UCLASS()
class MYGAME_API AMyPlayerState : public APlayerState, public IAbilitySystemInterface
{
    GENERATED_BODY()
public:
    AMyPlayerState();
    virtual UAbilitySystemComponent* GetAbilitySystemComponent() const override { return AbilitySystemComponent; }
    UMyAttributeSet* GetAttributeSet() const { return AttributeSet; }

protected:
    UPROPERTY(VisibleAnywhere, BlueprintReadOnly, Category = "Abilities")
    TObjectPtr<UAbilitySystemComponent> AbilitySystemComponent;

    UPROPERTY()
    TObjectPtr<UMyAttributeSet> AttributeSet;
};
```
```cpp
// MyPlayerState.cpp
#include "MyPlayerState.h"
#include "AbilitySystemComponent.h"
#include "MyAttributeSet.h"
#include "Misc/EngineVersionComparison.h"

AMyPlayerState::AMyPlayerState()
{
    AbilitySystemComponent = CreateDefaultSubobject<UAbilitySystemComponent>(TEXT("AbilitySystemComponent"));
    AbilitySystemComponent->SetIsReplicated(true);
    AbilitySystemComponent->SetReplicationMode(EGameplayEffectReplicationMode::Mixed);

    // Subobject of the ASC's owner actor, so the ASC registers it automatically.
    AttributeSet = CreateDefaultSubobject<UMyAttributeSet>(TEXT("AttributeSet"));

#if UE_VERSION_OLDER_THAN(5, 5, 0)
    NetUpdateFrequency = 100.f;
#else
    SetNetUpdateFrequency(100.f);
#endif
}
```
Set `PlayerStateClass = AMyPlayerState::StaticClass();` in the GameMode constructor (or in
the GameMode Blueprint's Classes section).

## 5. Character

```cpp
// MyCharacter.h
#pragma once
#include "CoreMinimal.h"
#include "GameFramework/Character.h"
#include "AbilitySystemInterface.h"
#include "MyCharacter.generated.h"

class UAbilitySystemComponent;
class UGameplayAbility;
class UGameplayEffect;
class UInputAction;
class UInputMappingContext;

UENUM(BlueprintType)
enum class EAbilityInputID : uint8
{
    None, Confirm, Cancel, Attack, Ability1, Ability2
};

USTRUCT(BlueprintType)
struct FAbilityInputBinding
{
    GENERATED_BODY()
    UPROPERTY(EditDefaultsOnly) TSubclassOf<UGameplayAbility> Ability;
    UPROPERTY(EditDefaultsOnly) EAbilityInputID InputID = EAbilityInputID::None;
    UPROPERTY(EditDefaultsOnly) TObjectPtr<UInputAction> InputAction;
};

UCLASS()
class MYGAME_API AMyCharacter : public ACharacter, public IAbilitySystemInterface
{
    GENERATED_BODY()
public:
    AMyCharacter();
    virtual UAbilitySystemComponent* GetAbilitySystemComponent() const override;

    virtual void PossessedBy(AController* NewController) override; // server
    virtual void OnRep_PlayerState() override;                      // owning client (and sim proxies)
    virtual void PawnClientRestart() override;
    virtual void SetupPlayerInputComponent(UInputComponent* PlayerInputComponent) override;

protected:
    void InitAbilitySystem();
    void GrantStartupAbilitiesAndEffects();
    void AbilityInputPressed(int32 InputID);
    void AbilityInputReleased(int32 InputID);

    UPROPERTY(EditDefaultsOnly, Category = "Abilities")
    TArray<FAbilityInputBinding> StartupAbilities;

    UPROPERTY(EditDefaultsOnly, Category = "Abilities")
    TArray<TSubclassOf<UGameplayEffect>> StartupEffects; // e.g. GE_DefaultAttributes (Instant, Override)

    UPROPERTY(EditDefaultsOnly, Category = "Input")
    TObjectPtr<UInputMappingContext> DefaultMappingContext;

private:
    TWeakObjectPtr<UAbilitySystemComponent> AbilitySystem;
};
```
```cpp
// MyCharacter.cpp
#include "MyCharacter.h"
#include "MyPlayerState.h"
#include "AbilitySystemComponent.h"
#include "EnhancedInputComponent.h"
#include "EnhancedInputSubsystems.h"
#include "Engine/LocalPlayer.h"
#include "GameFramework/PlayerController.h"

AMyCharacter::AMyCharacter() {}

UAbilitySystemComponent* AMyCharacter::GetAbilitySystemComponent() const
{
    return AbilitySystem.Get();
}

void AMyCharacter::PossessedBy(AController* NewController)
{
    Super::PossessedBy(NewController);
    InitAbilitySystem();
    GrantStartupAbilitiesAndEffects();
}

void AMyCharacter::OnRep_PlayerState()
{
    Super::OnRep_PlayerState();
    InitAbilitySystem();
}

void AMyCharacter::InitAbilitySystem()
{
    AMyPlayerState* PS = GetPlayerState<AMyPlayerState>();
    if (!PS) { return; }
    UAbilitySystemComponent* ASC = PS->GetAbilitySystemComponent();
    ASC->InitAbilityActorInfo(PS, this); // Owner = PlayerState, Avatar = this pawn
    AbilitySystem = ASC;
}

void AMyCharacter::GrantStartupAbilitiesAndEffects()
{
    UAbilitySystemComponent* ASC = AbilitySystem.Get();
    if (!ASC || !HasAuthority()) { return; }

    for (const FAbilityInputBinding& B : StartupAbilities)
    {
        // The ASC lives on the PlayerState and survives respawn: do not grant twice.
        if (B.Ability && !ASC->FindAbilitySpecFromClass(B.Ability))
        {
            ASC->GiveAbility(FGameplayAbilitySpec(B.Ability, 1, static_cast<int32>(B.InputID), this));
        }
    }

    FGameplayEffectContextHandle Ctx = ASC->MakeEffectContext();
    Ctx.AddSourceObject(this);
    for (const TSubclassOf<UGameplayEffect>& EffectClass : StartupEffects)
    {
        FGameplayEffectSpecHandle Spec = ASC->MakeOutgoingSpec(EffectClass, 1.f, Ctx);
        if (Spec.IsValid()) { ASC->ApplyGameplayEffectSpecToSelf(*Spec.Data.Get()); }
    }
}

void AMyCharacter::PawnClientRestart()
{
    Super::PawnClientRestart();
    if (APlayerController* PC = Cast<APlayerController>(GetController()))
    {
        if (UEnhancedInputLocalPlayerSubsystem* Subsystem =
                ULocalPlayer::GetSubsystem<UEnhancedInputLocalPlayerSubsystem>(PC->GetLocalPlayer()))
        {
            Subsystem->AddMappingContext(DefaultMappingContext, 0);
        }
    }
}

void AMyCharacter::SetupPlayerInputComponent(UInputComponent* PlayerInputComponent)
{
    Super::SetupPlayerInputComponent(PlayerInputComponent);
    UEnhancedInputComponent* EIC = CastChecked<UEnhancedInputComponent>(PlayerInputComponent);
    for (const FAbilityInputBinding& B : StartupAbilities)
    {
        if (!B.InputAction) { continue; }
        const int32 ID = static_cast<int32>(B.InputID);
        EIC->BindAction(B.InputAction, ETriggerEvent::Started,   this, &AMyCharacter::AbilityInputPressed,  ID);
        EIC->BindAction(B.InputAction, ETriggerEvent::Completed, this, &AMyCharacter::AbilityInputReleased, ID);
    }
}

// The ASC may not be initialized yet when input is bound on the client, so resolve it at press time.
void AMyCharacter::AbilityInputPressed(int32 InputID)
{
    if (UAbilitySystemComponent* ASC = AbilitySystem.Get()) { ASC->AbilityLocalInputPressed(InputID); }
}

void AMyCharacter::AbilityInputReleased(int32 InputID)
{
    if (UAbilitySystemComponent* ASC = AbilitySystem.Get()) { ASC->AbilityLocalInputReleased(InputID); }
}
```

Notes:
- Movement/look actions bind normally; only ability actions go through the ASC.
- The `EnhancedInputComponent` must be the project's default input component class
  (default since 5.1; check `DefaultInput.ini` `DefaultInputComponentClass`).
- If the character is ever used as an AI pawn, `PossessedBy` still initializes it on the server.

## 6. Editor assets to create (tell the human, or script creation via `ue_python`)

| Asset | Settings |
|---|---|
| `GE_DefaultAttributes` | Instant; Override modifiers for Health, MaxHealth, Mana, MaxMana, Armor |
| `GE_Damage` | Instant; Execution `DamageExecution` |
| `GE_Cost_Attack` | Instant; Mana Add with `MMC_ManaCost` |
| `GE_Cooldown_Attack` | Has Duration 1.0; Components: Grant Tags to Target Actor = `Cooldown.Attack` |
| `GA_MeleeAttack` (BP child) | AttackMontage, DamageEffect = GE_Damage, Cost/Cooldown GE classes |
| `IA_Attack` + mapping context | Bound in the Character Blueprint's StartupAbilities with InputID = Attack |

Then build (`ue_build`), open PIE with Net Mode = Play As Client, 2 players, and run
`showdebug abilitysystem` to confirm attributes and the granted ability appear on the client.
