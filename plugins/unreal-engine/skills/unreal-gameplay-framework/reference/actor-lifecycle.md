# Actor and component lifecycle

Knowing which function runs when (and in which world: editor, PIE, game) prevents most
initialization bugs. When exact ordering matters for your engine version, read
`Engine/Source/Runtime/Engine/Private/Actor.cpp`, `ActorConstruction.cpp` and
`LevelActor.cpp` (`UWorld::SpawnActor`).

## Where to put code

| Function | Runs in editor? | Has world? | Put here |
|---|---|---|---|
| Constructor | Yes (CDO + every instance, including on load) | No (treat as none) | Defaults, `CreateDefaultSubobject`, `SetupAttachment`, tick settings. Nothing else |
| `PostInitProperties()` | Yes | Maybe | Rare: derive values from properties after config/defaults load |
| `PostLoad()` | Yes | Maybe | Fix-ups on objects loaded from disk (data migration) |
| `OnConstruction(const FTransform&)` | Yes, on every move/property edit of placed actors | Yes | Procedural setup that depends on properties (the C++ side of the Construction Script). Must be deterministic and cheap |
| `PostInitializeComponents()` | Yes (placed actors in editor worlds too) | Yes | Wiring between components, binding component delegates. No gameplay |
| `BeginPlay()` | No (game/PIE only) | Yes | Gameplay start: timers, spawning, finding other actors, UI |
| `Tick(float)` | Only if `ShouldTickIfViewportsOnly()` | Yes | Per-frame work; avoid when possible |
| `EndPlay(EEndPlayReason::Type)` | No | Yes | Cleanup: clear timers, unbind delegates, save state |
| `Destroyed()` | Also in editor when deleting | Yes | Rarely needed; prefer EndPlay |
| `BeginDestroy()` / `FinishDestroy()` | Yes | No | Release non-UObject resources at GC time only |

## Spawned actor (`UWorld::SpawnActor`)

1. Object allocated, **constructor** runs (default subobjects created), `PostInitProperties`.
2. `PostSpawnInitialize`: owner/instigator set, transform applied, native components get
   `OnComponentCreated`, then are registered (`OnRegister`), then **`PostActorCreated()`**.
3. `FinishSpawning` (immediately, unless deferred) calls `ExecuteConstruction`:
   - Blueprint-added components (Simple Construction Script) are created and registered.
   - Blueprint **Construction Script** runs.
   - C++ **`OnConstruction(Transform)`** runs.
4. `PostActorConstruction`:
   - **`PreInitializeComponents()`**
   - **`InitializeComponents()`**: `UActorComponent::InitializeComponent()` for components with
     `bWantsInitializeComponent = true`.
   - **`PostInitializeComponents()`**
5. If the world has already begun play (normal at runtime): **`DispatchBeginPlay` -> `BeginPlay()`**.
   `SpawnActor` returns after BeginPlay has run.

### Deferred spawn

```cpp
AMyActor* A = GetWorld()->SpawnActorDeferred<AMyActor>(Class, Transform, Owner, Instigator,
    ESpawnActorCollisionHandlingMethod::AdjustIfPossibleButAlwaysSpawn);
A->InitData = Data;                     // runs between steps 2 and 3
A->FinishSpawning(Transform);           // steps 3-5
```
Use it when construction script / BeginPlay must see values set by the spawner. Blueprint "Spawn
Actor from Class" does this automatically for `ExposeOnSpawn` properties. Always call
`FinishSpawning`, or the actor stays half-initialized.

## Actor placed in a level

In the editor: placing/dragging runs the constructor, then construction (`OnConstruction` + BP
Construction Script) again on each move or property edit (`bRunConstructionScriptOnDrag` controls
reruns during drags). The result (including components created by the construction script) is
saved into the level.

At runtime / PIE:
1. Level loads: constructor, deserialization, **`PostLoad()`**. (PIE duplicates the editor world
   instead: `PostDuplicate` / `PostLoad`-like path.)
2. Components registered (`OnRegister`), `PostRegisterAllComponents()`.
3. The construction script is **not** rerun for actors loaded from a cooked level; its saved result
   is used. Do not rely on `OnConstruction` running at game start for placed actors.
4. `ULevel::RouteActorInitialize`: `PreInitializeComponents`, `InitializeComponents`,
   `PostInitializeComponents` for every actor in the level.
5. When the world begins play (`UWorld::BeginPlay` via GameMode `StartPlay`), `BeginPlay` is called
   on all actors. Actors in levels streamed in later get BeginPlay when their level becomes
   visible and initialized.

Order of BeginPlay between actors is not guaranteed. If A needs B ready, have B broadcast a
delegate or register with a subsystem, or look B up lazily.

## BeginPlay details

```cpp
void AMyActor::BeginPlay()
{
    // code here runs BEFORE components' BeginPlay and the Blueprint "Event BeginPlay"
    Super::BeginPlay();   // AActor::BeginPlay: components' BeginPlay, then Blueprint Event BeginPlay
    // code here runs AFTER the Blueprint BeginPlay
}
```
Component BeginPlay is called from the owner's `AActor::BeginPlay` for registered components;
components added at runtime (`NewObject` + `RegisterComponent()`) after the owner began play get
BeginPlay during registration.

## Components

1. Constructor (via `CreateDefaultSubobject` in the owner's constructor, or `NewObject`).
2. `OnComponentCreated()`.
3. `OnRegister()` (render/physics state created; happens in editor too).
4. `InitializeComponent()` if `bWantsInitializeComponent`.
5. `BeginPlay()`.
6. `TickComponent(DeltaTime, TickType, ThisTickFunction)` if `PrimaryComponentTick.bCanEverTick`.
7. `EndPlay(Reason)`.
8. `OnUnregister()`, `OnComponentDestroyed(bDestroyingHierarchy)`.

Runtime creation:
```cpp
UStaticMeshComponent* C = NewObject<UStaticMeshComponent>(this, TEXT("RuntimeMesh"));
C->SetupAttachment(GetRootComponent());
C->RegisterComponent();          // required, or it never renders/ticks
AddInstanceComponent(C);         // optional: show in the Details panel / save with the actor in editor
```
`CreateDefaultSubobject` only works inside a constructor.

## Destruction and EndPlay

`AActor::Destroy()` -> `UWorld::DestroyActor` -> `Destroyed()`, which routes
**`EndPlay(EEndPlayReason::Destroyed)`** (actor and its components) and broadcasts `OnDestroyed`;
components are unregistered, the actor is marked as garbage, and memory is reclaimed at the next GC
(`BeginDestroy`, `FinishDestroy`). After `Destroy()`, `IsValid(Actor)` is false.

`EEndPlayReason`:
| Value | When |
|---|---|
| `Destroyed` | Explicit `Destroy()` |
| `LevelTransition` | Map change (OpenLevel / ServerTravel) |
| `EndPlayInEditor` | PIE session ended |
| `RemovedFromWorld` | Streaming level unloaded / World Partition cell unloaded |
| `Quit` | Application exit |

Always call `Super::EndPlay(Reason)` (usually last). Do not assume EndPlay means "died": check the
reason before, e.g., dropping loot. `RemovedFromWorld` actors in World Partition come back as new
instances when the cell reloads: persist their state elsewhere if it matters.

`InitialLifeSpan` / `SetLifeSpan(Seconds)` destroy the actor automatically.

## Pawn and controller startup (player)

1. GameMode `InitGame`, then for each player: `PreLogin` -> `Login` (creates PlayerController) ->
   `PostLogin` -> `HandleStartingNewPlayer` -> `RestartPlayer`.
2. `RestartPlayer` picks a start (`FindPlayerStart` / `ChoosePlayerStart`), spawns
   `DefaultPawnClass` via `SpawnDefaultPawnFor`, then `Controller->Possess(Pawn)`.
3. Possession: `AController::OnPossess(Pawn)` and `APawn::PossessedBy(Controller)` (server);
   `APawn::Restart()`; for local players `PawnClientRestart()` creates the input component and
   calls `SetupPlayerInputComponent`. `APawn::NotifyControllerChanged()` fires on controller changes
   in recent engine versions (check it exists in your `Pawn.h`).
4. `ReceivePossessed` (BP "Event Possessed", server) / `OnRep_Controller` on clients.

Pawn `BeginPlay` may run before or after possession depending on the path: players joining at
map load are usually spawned and possessed before the world begins play (BeginPlay comes later),
while a pawn spawned mid-game (respawn) gets BeginPlay inside `SpawnActor`, before `Possess`.
Clients receive the controller by replication at an arbitrary time. Code that needs the
controller belongs in `PossessedBy` / `NotifyControllerChanged` / `SetupPlayerInputComponent`, not
BeginPlay.

## Tick configuration

```cpp
AMyActor::AMyActor()
{
    PrimaryActorTick.bCanEverTick = true;           // false = never registers a tick function (cheapest)
    PrimaryActorTick.bStartWithTickEnabled = false; // enable later with SetActorTickEnabled(true)
    PrimaryActorTick.TickInterval = 0.2f;           // seconds; 0 = every frame
    PrimaryActorTick.TickGroup = TG_PrePhysics;
}

void AMyActor::Tick(float DeltaSeconds)
{
    Super::Tick(DeltaSeconds);
}
```
- Tick groups in order: `TG_PrePhysics` (default; movement/input), `TG_StartPhysics`,
  `TG_DuringPhysics` (work that does not depend on this frame's physics), `TG_EndPhysics`,
  `TG_PostPhysics` (needs final physics transforms: cameras, IK targets, traces against moved
  objects), `TG_PostUpdateWork`, `TG_LastDemotable`.
- Ordering between specific actors: `AddTickPrerequisiteActor(Other)` /
  `AddTickPrerequisiteComponent(Comp)`.
- Paused game: actors do not tick unless `SetTickableWhenPaused(true)` (`PrimaryActorTick.bTickEvenWhenPaused`).
- Editor viewport ticking (tools, previews): override `ShouldTickIfViewportsOnly()` to return true.
- Runtime changes: `SetActorTickEnabled`, `SetActorTickInterval`, component `SetComponentTickEnabled`.
- `bCanEverTick = false` in C++ cannot be overridden by a Blueprint subclass (the Blueprint Event
  Tick then never fires); Blueprint "Start with Tick Enabled" controls `bStartWithTickEnabled`.

## Editor-only hooks

- `PostEditChangeProperty(FPropertyChangedEvent&)` (inside `#if WITH_EDITOR`): react to Details
  panel edits.
- `PostEditMove(bool bFinished)`: after the actor is moved in the viewport.
- Construction script reruns cover most "update when edited" needs; use these for anything more
  selective.

## Checklist

- [ ] No world access or spawning in constructors or `OnConstruction`.
- [ ] Component cross-wiring in `PostInitializeComponents`; gameplay start in `BeginPlay`.
- [ ] Every override calls `Super::` (and BeginPlay code is placed before/after it on purpose).
- [ ] EndPlay clears timers/delegates and checks the reason.
- [ ] Tick disabled unless needed; interval and group chosen deliberately.
- [ ] Code that needs the controller runs on possession, not BeginPlay.
