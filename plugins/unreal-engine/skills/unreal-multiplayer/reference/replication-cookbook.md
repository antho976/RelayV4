# Replication cookbook

Copy-ready patterns. Replace `MYGAME_API` and class names. Every replicated class needs
`#include "Net/UnrealNetwork.h"` in its .cpp. After changes: `ue_build`, then PIE with
Net Mode = Play As Client, 2+ players, network emulation on.

## 1. Replicated health with OnRep (and listen-server host reaction)

```cpp
// Header
UPROPERTY(ReplicatedUsing = OnRep_Health, BlueprintReadOnly, Category = "Health")
float Health = 100.f;

UFUNCTION()
void OnRep_Health(float OldHealth);

void ApplyDamage(float Amount); // call on server
void HandleHealthChanged(float OldHealth); // shared reaction (UI, effects)
```
```cpp
AMyCharacter::AMyCharacter() { bReplicates = true; } // ACharacter already replicates; shown for plain actors

void AMyCharacter::GetLifetimeReplicatedProps(TArray<FLifetimeProperty>& OutLifetimeProps) const
{
    Super::GetLifetimeReplicatedProps(OutLifetimeProps);
    DOREPLIFETIME(AMyCharacter, Health);
}

void AMyCharacter::ApplyDamage(float Amount)
{
    if (!HasAuthority()) { return; }
    const float Old = Health;
    Health = FMath::Clamp(Health - Amount, 0.f, 100.f);
    HandleHealthChanged(Old); // C++ OnRep does not run on the server; call the reaction explicitly
}

void AMyCharacter::OnRep_Health(float OldHealth) { HandleHealthChanged(OldHealth); }
```
For damage from the engine's damage pipeline, override `TakeDamage` (runs where
`UGameplayStatics::ApplyDamage` is called - call it on the server).

## 2. Client requests an action: Server RPC with validation

```cpp
UFUNCTION(Server, Reliable, WithValidation)
void ServerFire(FVector_NetQuantize Origin, FVector_NetQuantizeNormal Direction);
```
```cpp
void AMyCharacter::OnFirePressed() // bound to input on the owning client
{
    const FVector Origin = GetPawnViewLocation();
    const FVector Dir = GetBaseAimRotation().Vector();
    if (!HasAuthority()) { PlayLocalFireEffects(); } // instant local feedback on the client
    ServerFire(Origin, Dir);                          // on a listen host this just runs locally
}

bool AMyCharacter::ServerFire_Validate(FVector_NetQuantize Origin, FVector_NetQuantizeNormal Direction)
{
    // Only reject impossible input (cheats). Returning false kicks the client.
    return Direction.IsNormalized();
}

void AMyCharacter::ServerFire_Implementation(FVector_NetQuantize Origin, FVector_NetQuantizeNormal Direction)
{
    if (Ammo <= 0 || !CanFire()) { return; } // normal gameplay rejection: just ignore
    // Do not trust Origin blindly: clamp it near the server's view location.
    const FVector ServerOrigin = GetPawnViewLocation();
    const FVector UseOrigin = FVector::Dist(Origin, ServerOrigin) < 200.f ? FVector(Origin) : ServerOrigin;
    --Ammo;
    FHitResult Hit;
    FCollisionQueryParams Params(SCENE_QUERY_STAT(Fire), false, this);
    if (GetWorld()->LineTraceSingleByChannel(Hit, UseOrigin, UseOrigin + Direction * 10000.f, ECC_Visibility, Params))
    {
        MulticastPlayImpact(Hit.ImpactPoint, Hit.ImpactNormal);
        // apply damage here (server)
    }
}
```

## 3. Cosmetic event to everyone: Unreliable Multicast

```cpp
UFUNCTION(NetMulticast, Unreliable)
void MulticastPlayImpact(FVector_NetQuantize Location, FVector_NetQuantizeNormal Normal);
```
```cpp
void AMyCharacter::MulticastPlayImpact_Implementation(FVector_NetQuantize Location, FVector_NetQuantizeNormal Normal)
{
    if (IsNetMode(NM_DedicatedServer)) { return; } // no cosmetics on a dedicated server
    // Spawn Niagara / play sound (see unreal-materials-vfx, unreal-audio)
}
```
Late joiners never see it - fine for impacts, wrong for "door is open" (use a property).

## 4. Owner-only data and a client-only notification

```cpp
UPROPERTY(ReplicatedUsing = OnRep_Ammo) int32 Ammo = 30;
UFUNCTION(Client, Reliable) void ClientNotifyKill(const FString& VictimName);
```
```cpp
DOREPLIFETIME_CONDITION(AMyCharacter, Ammo, COND_OwnerOnly);
```
Call `ClientNotifyKill` on the server on the killer's PlayerController (or pawn); it runs on
that player's machine only.

## 5. Replicated component

```cpp
UCLASS(ClassGroup=(Custom), meta=(BlueprintSpawnableComponent))
class MYGAME_API UInventoryComponent : public UActorComponent
{
    GENERATED_BODY()
public:
    UInventoryComponent() { SetIsReplicatedByDefault(true); }
    virtual void GetLifetimeReplicatedProps(TArray<FLifetimeProperty>& OutLifetimeProps) const override;

    UPROPERTY(Replicated) int32 Gold = 0;

    UFUNCTION(Server, Reliable) void ServerBuy(FName ItemId); // works because the owning actor is owned by the client
};
```
```cpp
void UInventoryComponent::GetLifetimeReplicatedProps(TArray<FLifetimeProperty>& OutLifetimeProps) const
{
    Super::GetLifetimeReplicatedProps(OutLifetimeProps);
    DOREPLIFETIME_CONDITION(UInventoryComponent, Gold, COND_OwnerOnly);
}
```
The owning actor must replicate too.

## 6. Arrays with per-item callbacks: FFastArraySerializer

Build.cs: add `"NetCore"`. Use for inventories, buff lists, anything where clients need to
know which element was added/changed/removed.

```cpp
#include "Net/Serialization/FastArraySerializer.h"
#include "InventoryList.generated.h"

USTRUCT(BlueprintType)
struct FInventoryEntry : public FFastArraySerializerItem
{
    GENERATED_BODY()
    UPROPERTY() FName ItemId;
    UPROPERTY() int32 Count = 0;

    void PreReplicatedRemove(const struct FInventoryList& InArraySerializer);
    void PostReplicatedAdd(const struct FInventoryList& InArraySerializer);
    void PostReplicatedChange(const struct FInventoryList& InArraySerializer);
};

USTRUCT(BlueprintType)
struct FInventoryList : public FFastArraySerializer
{
    GENERATED_BODY()
    UPROPERTY() TArray<FInventoryEntry> Items;

    bool NetDeltaSerialize(FNetDeltaSerializeInfo& DeltaParms)
    {
        return FFastArraySerializer::FastArrayDeltaSerialize<FInventoryEntry, FInventoryList>(Items, DeltaParms, *this);
    }
};

template<>
struct TStructOpsTypeTraits<FInventoryList> : public TStructOpsTypeTraitsBase2<FInventoryList>
{
    enum { WithNetDeltaSerializer = true };
};
```
Server-side mutation:
```cpp
FInventoryEntry& E = Inventory.Items.AddDefaulted_GetRef();
E.ItemId = Id; E.Count = 1;
Inventory.MarkItemDirty(E);          // after add or change
// ...
Inventory.Items.RemoveAt(Index);
Inventory.MarkArrayDirty();          // after remove
```
Register with `DOREPLIFETIME(AMyActor, Inventory);` (`UPROPERTY(Replicated) FInventoryList Inventory;`).
Callbacks run on clients only. Element order on clients is not guaranteed to match the server.
Under Iris, fast arrays are supported but custom `NetDeltaSerialize` code beyond this
standard form may need an Iris serializer.

## 7. Push Model

```cpp
// Build.cs: PublicDependencyModuleNames.Add("NetCore");
#include "Net/Core/PushModel/PushModel.h"

void AMyActor::GetLifetimeReplicatedProps(TArray<FLifetimeProperty>& OutLifetimeProps) const
{
    Super::GetLifetimeReplicatedProps(OutLifetimeProps);
    FDoRepLifetimeParams Params;
    Params.bIsPushBased = true;
    DOREPLIFETIME_WITH_PARAMS_FAST(AMyActor, Score, Params);

    FDoRepLifetimeParams OwnerParams;
    OwnerParams.bIsPushBased = true;
    OwnerParams.Condition = COND_OwnerOnly;
    DOREPLIFETIME_WITH_PARAMS_FAST(AMyActor, Secret, OwnerParams);
}

void AMyActor::SetScore(int32 NewScore)
{
    Score = NewScore;
    MARK_PROPERTY_DIRTY_FROM_NAME(AMyActor, Score, this);
}
```
Make push-based properties `private`/`protected` with setters so no write forgets the dirty
mark. A missed mark means the change never replicates (when push model is active).
If push model is disabled in the build, these properties fall back to normal comparison.

## 8. Dormancy for rarely-changing actors (doors, chests)

```cpp
AChest::AChest()
{
    bReplicates = true;
    NetDormancy = DORM_Initial;      // placed in level, unchanged until opened
}

void AChest::Open() // server
{
    FlushNetDormancy();              // must come before the change
    bIsOpen = true;
    OnRep_IsOpen();                  // host-side reaction
}
```
For actors that change in bursts: `SetNetDormancy(DORM_Awake)` while active, then
`SetNetDormancy(DORM_DormantAll)` when idle.

## 9. Replicated UObject subobjects (5.1+ registered list)

```cpp
UCLASS()
class UItemInstance : public UObject
{
    GENERATED_BODY()
public:
    virtual bool IsSupportedForNetworking() const override { return true; }
    virtual void GetLifetimeReplicatedProps(TArray<FLifetimeProperty>& OutLifetimeProps) const override;
    UPROPERTY(Replicated) int32 Durability = 100;
};

// Owning actor constructor
bReplicateUsingRegisteredSubObjectList = true;

// Server, when creating the item
UItemInstance* Item = NewObject<UItemInstance>(this);
AddReplicatedSubObject(Item);          // RemoveReplicatedSubObject(Item) before discarding it
Items.Add(Item);                       // UPROPERTY(Replicated) TArray<TObjectPtr<UItemInstance>> Items;
```
For subobjects owned by a component, call `AddReplicatedSubObject` on that component
(components have the same API).

## 10. Spawning a replicated projectile

```cpp
void AMyCharacter::ServerSpawnProjectile_Implementation(FVector_NetQuantize Loc, FRotator Rot)
{
    FActorSpawnParameters P;
    P.Owner = this;                 // lets the projectile use owner relevancy / owner-only data
    P.Instigator = this;
    P.SpawnCollisionHandlingOverride = ESpawnActorCollisionHandlingMethod::AlwaysSpawn;
    GetWorld()->SpawnActor<AMyProjectile>(ProjectileClass, Loc, Rot, P);
}

AMyProjectile::AMyProjectile()
{
    bReplicates = true;
    SetReplicateMovement(true);
    // UProjectileMovementComponent simulates on clients too, giving smooth motion
}
```

## 11. Networked sprint in CharacterMovementComponent (custom compressed flag)

```cpp
// MyCharacterMovementComponent.h
UCLASS()
class MYGAME_API UMyCharacterMovementComponent : public UCharacterMovementComponent
{
    GENERATED_BODY()
public:
    UPROPERTY(EditDefaultsOnly) float SprintSpeed = 900.f;
    uint8 bWantsToSprint : 1;

    virtual float GetMaxSpeed() const override;
    virtual void UpdateFromCompressedFlags(uint8 Flags) override;
    virtual FNetworkPredictionData_Client* GetPredictionData_Client() const override;
};

class FSavedMove_My : public FSavedMove_Character
{
public:
    uint8 bSavedWantsToSprint : 1;
    virtual void Clear() override { FSavedMove_Character::Clear(); bSavedWantsToSprint = 0; }
    virtual uint8 GetCompressedFlags() const override
    {
        uint8 Result = FSavedMove_Character::GetCompressedFlags();
        if (bSavedWantsToSprint) { Result |= FLAG_Custom_0; }
        return Result;
    }
    virtual bool CanCombineWith(const FSavedMovePtr& NewMove, ACharacter* Character, float MaxDelta) const override
    {
        if (bSavedWantsToSprint != static_cast<FSavedMove_My*>(NewMove.Get())->bSavedWantsToSprint) { return false; }
        return FSavedMove_Character::CanCombineWith(NewMove, Character, MaxDelta);
    }
    virtual void SetMoveFor(ACharacter* C, float InDeltaTime, FVector const& NewAccel, FNetworkPredictionData_Client_Character& ClientData) override
    {
        FSavedMove_Character::SetMoveFor(C, InDeltaTime, NewAccel, ClientData);
        bSavedWantsToSprint = Cast<UMyCharacterMovementComponent>(C->GetCharacterMovement())->bWantsToSprint;
    }
    virtual void PrepMoveFor(ACharacter* C) override
    {
        FSavedMove_Character::PrepMoveFor(C);
        Cast<UMyCharacterMovementComponent>(C->GetCharacterMovement())->bWantsToSprint = bSavedWantsToSprint;
    }
};

class FNetworkPredictionData_Client_My : public FNetworkPredictionData_Client_Character
{
public:
    explicit FNetworkPredictionData_Client_My(const UCharacterMovementComponent& CMC) : FNetworkPredictionData_Client_Character(CMC) {}
    virtual FSavedMovePtr AllocateNewMove() override { return FSavedMovePtr(new FSavedMove_My()); }
};
```
```cpp
// MyCharacterMovementComponent.cpp
float UMyCharacterMovementComponent::GetMaxSpeed() const
{
    return (bWantsToSprint && MovementMode == MOVE_Walking) ? SprintSpeed : Super::GetMaxSpeed();
}

void UMyCharacterMovementComponent::UpdateFromCompressedFlags(uint8 Flags)
{
    Super::UpdateFromCompressedFlags(Flags);
    bWantsToSprint = (Flags & FSavedMove_Character::FLAG_Custom_0) != 0;
}

FNetworkPredictionData_Client* UMyCharacterMovementComponent::GetPredictionData_Client() const
{
    if (!ClientPredictionData)
    {
        UMyCharacterMovementComponent* MutableThis = const_cast<UMyCharacterMovementComponent*>(this);
        MutableThis->ClientPredictionData = new FNetworkPredictionData_Client_My(*this);
    }
    return ClientPredictionData;
}
```
Use it from the character: constructor
`AMyCharacter(const FObjectInitializer& OI) : Super(OI.SetDefaultSubobjectClass<UMyCharacterMovementComponent>(ACharacter::CharacterMovementComponentName)) {}`;
input sets `bWantsToSprint` on the **owning client** only - the flag reaches the server inside the move.
Only 4 custom flags exist; for more state, newer engines support custom move data
(`FCharacterNetworkMoveData` subclass) - check `CharacterMovementComponent.h` in the project's engine.

## 12. Choosing the right tool

| Need | Use |
|---|---|
| Persistent state everyone sees | Replicated property (+ OnRep for reactions) |
| Persistent state only owner sees | `COND_OwnerOnly` property |
| Client asks server to do something | Server RPC on an owned actor |
| One-off cosmetic for everyone nearby | Unreliable NetMulticast |
| One-off message to one player | Client RPC on their controller/pawn |
| List with add/remove callbacks | FFastArraySerializer |
| Many rarely-changing actors | Dormancy + low NetUpdateFrequency |
| Hot actor, few changes, CPU bound server | Push Model |
| Player movement variations | CMC saved move flags |
