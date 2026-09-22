# Delegates and timers

## Choosing a delegate kind

| Kind | Declare | Bind to | Blueprint? | Cost |
|---|---|---|---|---|
| Single-cast | `DECLARE_DELEGATE[_RetVal][_NParams]` | One function; can return a value | No | Fast |
| Multicast | `DECLARE_MULTICAST_DELEGATE[_NParams]` | Many functions; no return value | No | Fast |
| Event | `DECLARE_EVENT[_NParams](OwnerType, Name, ...)` | Like multicast, but only `OwnerType` can Broadcast | No | Fast |
| Dynamic single | `DECLARE_DYNAMIC_DELEGATE[_RetVal][_NParams]` | One UFUNCTION by name; serializable | As a function parameter (event pin) | Slower (reflection) |
| Dynamic multicast | `DECLARE_DYNAMIC_MULTICAST_DELEGATE[_NParams]` | Many UFUNCTIONs | `BlueprintAssignable` property | Slowest |
| Sparse dynamic multicast | `DECLARE_DYNAMIC_MULTICAST_SPARSE_DELEGATE[_NParams](Name, OwnerClass, PropertyName, ...)` | Same as dynamic multicast | Yes | Saves memory when rarely bound (engine uses it on components) |

Rules:
- Need Blueprint to bind? Dynamic multicast, exposed with `UPROPERTY(BlueprintAssignable)`.
- C++ only? Non-dynamic (faster, supports lambdas, raw and shared pointers, payload variables).
- `_NParams` suffixes: `_OneParam` ... `_NineParams`. `_RetVal` variants: `DECLARE_DELEGATE_RetVal_OneParam(bool, FCanUse, AActor*)`.
- Dynamic delegate declarations take **type and name** for each parameter; non-dynamic ones take
  **types only**.
- Delegate declarations go at file scope in a header (before the UCLASS that uses them), or inside
  a class for non-dynamic ones. The type name must start with `F`.

## Declaring

```cpp
// non-dynamic: types only
DECLARE_DELEGATE(FOnSimpleSignature);
DECLARE_DELEGATE_OneParam(FOnAmmoChangedSignature, int32);
DECLARE_DELEGATE_RetVal_OneParam(bool, FCanInteractSignature, AActor*);
DECLARE_MULTICAST_DELEGATE_TwoParams(FOnScoreChanged, AController* /*Scorer*/, int32 /*NewScore*/);

// dynamic: type AND name for each param
DECLARE_DYNAMIC_DELEGATE_OneParam(FOnQueryDone, bool, bSuccess);
DECLARE_DYNAMIC_MULTICAST_DELEGATE_TwoParams(FOnHealthChangedSignature, float, NewHealth, float, Delta);

UCLASS()
class MYGAME_API UWeaponComponent : public UActorComponent
{
    GENERATED_BODY()
public:
    // C++-only listeners
    FOnScoreChanged OnScoreChanged;

    // Blueprint-bindable
    UPROPERTY(BlueprintAssignable, Category = "Weapon")
    FOnHealthChangedSignature OnHealthChanged;

    // Blueprint passes a callback (shows an event pin on the node)
    UFUNCTION(BlueprintCallable, Category = "Weapon")
    void RunQuery(FOnQueryDone OnDone);
};
```

## Binding (non-dynamic)

| Function | Target | Safety |
|---|---|---|
| `BindUObject(Obj, &UClass::Func)` / `AddUObject` | UObject member | Weak: not called if Obj is garbage; auto-safe |
| `BindSP(SharedRef, &FClass::Func)` / `AddSP` | `TSharedFromThis` object | Weak |
| `BindRaw(Ptr, &FClass::Func)` / `AddRaw` | Raw C++ object | UNSAFE: you must unbind before it dies |
| `BindStatic(&Func)` / `AddStatic` | Free/static function | Safe |
| `BindLambda(Lambda)` / `AddLambda` | Lambda | Unsafe if it captures `this` or UObjects |
| `BindWeakLambda(Obj, Lambda)` / `AddWeakLambda` | Lambda tied to a UObject | Not called once Obj is gone; preferred for lambdas capturing `this` |
| `BindUFunction(Obj, FName("Func"))` / `AddUFunction` | UFUNCTION by name | Weak |

Payload: extra trailing args are stored and appended at call time:
```cpp
FTimerDelegate D = FTimerDelegate::CreateUObject(this, &AMyActor::OnTimerWithArg, 42);
```

Single-cast:
```cpp
FOnAmmoChangedSignature OnAmmoChanged;
OnAmmoChanged.BindUObject(this, &AMyHUD::HandleAmmo);
OnAmmoChanged.ExecuteIfBound(12);              // safe if unbound
if (OnAmmoChanged.IsBound()) { OnAmmoChanged.Execute(12); }   // Execute on unbound asserts
OnAmmoChanged.Unbind();
```

Multicast:
```cpp
FDelegateHandle Handle = Weapon->OnScoreChanged.AddUObject(this, &AMyHUD::HandleScore);
Weapon->OnScoreChanged.Broadcast(Controller, 10);
Weapon->OnScoreChanged.Remove(Handle);        // remove one binding
Weapon->OnScoreChanged.RemoveAll(this);       // remove all bindings for this object
```
Store the `FDelegateHandle` if you bind lambdas; lambdas can only be removed by handle.

## Binding (dynamic)

```cpp
// the bound function MUST be a UFUNCTION with a matching signature
UFUNCTION()
void HandleHealthChanged(float NewHealth, float Delta);

// BeginPlay (not the constructor)
Health->OnHealthChanged.AddDynamic(this, &AMyHUD::HandleHealthChanged);
Health->OnHealthChanged.AddUniqueDynamic(this, &AMyHUD::HandleHealthChanged);   // no duplicates
Health->OnHealthChanged.RemoveDynamic(this, &AMyHUD::HandleHealthChanged);
Health->OnHealthChanged.Broadcast(50.f, -10.f);

// dynamic single-cast parameter from Blueprint
void UWeaponComponent::RunQuery(FOnQueryDone OnDone)
{
    OnDone.ExecuteIfBound(true);
}
// binding a dynamic single-cast in C++
FOnQueryDone Done;
Done.BindDynamic(this, &AMyActor::HandleQueryDone);   // HandleQueryDone must be a UFUNCTION
```
- `AddDynamic`/`BindDynamic` are macros: the second argument must be `&Class::Function` written
  out (no variables), and the function must be a `UFUNCTION()`. Missing `UFUNCTION()` compiles
  but fails at runtime with an ensure "Unable to bind delegate to ... (function might not be
  marked as a UFUNCTION or is private)".
- Binding dynamic delegates in a constructor serializes the binding into the CDO and can cause
  double bindings; bind in `BeginPlay`/`PostInitializeComponents`/`NativeConstruct`.
- Unbinding is optional for UObject targets (weak), but do it in `EndPlay` when the source
  outlives the listener, for clarity and to avoid calls into torn-down state.

## Common engine delegates

| Delegate | Signature (params) | Bind with |
|---|---|---|
| `AActor::OnActorBeginOverlap` | `(AActor* OverlappedActor, AActor* OtherActor)` | `AddDynamic` |
| `AActor::OnActorHit` | `(AActor* SelfActor, AActor* OtherActor, FVector NormalImpulse, const FHitResult& Hit)` | `AddDynamic` |
| `AActor::OnDestroyed` | `(AActor* DestroyedActor)` | `AddDynamic` |
| `AActor::OnTakeAnyDamage` | `(AActor* DamagedActor, float Damage, const UDamageType* DamageType, AController* InstigatedBy, AActor* DamageCauser)` | `AddDynamic` |
| `UPrimitiveComponent::OnComponentBeginOverlap` | `(UPrimitiveComponent* OverlappedComponent, AActor* OtherActor, UPrimitiveComponent* OtherComp, int32 OtherBodyIndex, bool bFromSweep, const FHitResult& SweepResult)` | `AddDynamic` |
| `UPrimitiveComponent::OnComponentEndOverlap` | `(UPrimitiveComponent* OverlappedComponent, AActor* OtherActor, UPrimitiveComponent* OtherComp, int32 OtherBodyIndex)` | `AddDynamic` |
| `UPrimitiveComponent::OnComponentHit` | `(UPrimitiveComponent* HitComponent, AActor* OtherActor, UPrimitiveComponent* OtherComp, FVector NormalImpulse, const FHitResult& Hit)` | `AddDynamic` |
| `FWorldDelegates::OnPostWorldInitialization` | `(UWorld*, const UWorld::InitializationValues)` | `AddUObject`/`AddStatic` |
| `FCoreUObjectDelegates::PostLoadMapWithWorld` | `(UWorld*)` | `AddUObject` |
| `FEditorDelegates::BeginPIE` / `EndPIE` (editor only) | `(const bool bIsSimulating)` | `AddUObject`/`AddRaw` |

Always copy the exact signature from the engine header for the version you build against
(search `DECLARE_DYNAMIC_MULTICAST_SPARSE_DELEGATE` / `DECLARE_DYNAMIC_MULTICAST_DELEGATE` for the
delegate type name, e.g. `FComponentBeginOverlapSignature` in `PrimitiveComponent.h`).
Overlap events need `SetGenerateOverlapEvents(true)` on both components and compatible collision
responses; hit events need `SetNotifyRigidBodyCollision(true)` ("Simulation Generates Hit Events")
for physics hits.

## Timers

The timer manager lives on the `UWorld` (and a separate one on `UGameInstance`). Timers tick with
world time: they pause with the game and are dilated by time dilation.

```cpp
#include "TimerManager.h"

// header
FTimerHandle FireTimerHandle;

// start a looping timer: first call after 0.5 s, then every 0.1 s
GetWorldTimerManager().SetTimer(FireTimerHandle, this, &AMyWeapon::FireOnce, 0.1f, /*bLoop*/ true, /*FirstDelay*/ 0.5f);

// one-shot with a lambda, safe against this being destroyed
FTimerHandle Handle;
GetWorld()->GetTimerManager().SetTimer(Handle,
    FTimerDelegate::CreateWeakLambda(this, [this]() { Explode(); }), 3.0f, false);

// with a payload
FTimerDelegate Del = FTimerDelegate::CreateUObject(this, &AMyActor::SpawnWave, WaveIndex);
GetWorldTimerManager().SetTimer(WaveTimerHandle, Del, 10.f, false);

// next tick
GetWorldTimerManager().SetTimerForNextTick(this, &AMyActor::DeferredInit);

// query and control
GetWorldTimerManager().IsTimerActive(FireTimerHandle);
GetWorldTimerManager().GetTimerRemaining(FireTimerHandle);   // -1 if not active
GetWorldTimerManager().PauseTimer(FireTimerHandle);
GetWorldTimerManager().UnPauseTimer(FireTimerHandle);
GetWorldTimerManager().ClearTimer(FireTimerHandle);          // invalidates the handle

// on teardown
void AMyWeapon::EndPlay(const EEndPlayReason::Type Reason)
{
    GetWorldTimerManager().ClearAllTimersForObject(this);
    Super::EndPlay(Reason);
}
```
- `GetWorldTimerManager()` is an `AActor` convenience; from components and other UObjects use
  `GetWorld()->GetTimerManager()` (check `GetWorld()` is non-null).
- A rate `<= 0` clears the timer instead of setting it.
- Calling `SetTimer` with an active handle restarts it.
- Timers bound via `this, &Func` are removed automatically if the object is destroyed, but clearing
  in `EndPlay` is still good practice.
- For work that must continue across level transitions, use the GameInstance timer manager
  (`GetGameInstance()->GetTimerManager()`).
- Timer rate is at best once per frame: a 0.001 s looping timer fires multiple times in one frame
  to catch up. For per-frame work use Tick.
- Real-time (ignoring pause/dilation) delays: there is no flag on SetTimer; use a ticking object
  with `bTickEvenWhenPaused` or `FTSTicker::GetCoreTicker()` (core ticker, UE5 name) for
  non-world, real-time callbacks.

## Blueprint equivalents (for describing to the human)

- Delay node: latent, one-shot, per node instance; not cancellable. Use "Set Timer by Event" /
  "Set Timer by Function Name" for cancellable or looping timers; they return a Timer Handle.
- "Bind Event to On X" on a `BlueprintAssignable` delegate, fed by a Custom Event with a matching
  signature (drag from the red Event pin and choose "Add Custom Event").

## Pitfalls

- Capturing raw `this` in `AddLambda`/`BindLambda`/`CreateLambda` and outliving the object:
  crash. Use weak versions.
- Broadcasting during destruction: listeners may already be gone; guard with `IsValid`.
- Removing bindings from inside a broadcast is allowed for multicast delegates (the invocation
  list is protected), but adding new bindings during broadcast may or may not fire this round.
- Signature mismatch on `AddDynamic` gives a long template error pointing at the macro; compare
  parameter types exactly (including `const` and `&`).
- A dynamic delegate property without `UPROPERTY(BlueprintAssignable)` is invisible to Blueprint.
