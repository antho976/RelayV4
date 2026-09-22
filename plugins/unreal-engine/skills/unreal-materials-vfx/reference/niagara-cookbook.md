# Niagara cookbook

Runtime control of Niagara from C++ and Blueprint, plus effect recipes and editor steps.
Build.cs: `PublicDependencyModuleNames.Add("Niagara");`. Includes:
`NiagaraFunctionLibrary.h`, `NiagaraComponent.h`, `NiagaraSystem.h`.

Authoring emitters/modules is graph work in the Niagara editor; it cannot be done by
editing files. The agent spawns, parameterizes, assigns and audits systems; the human (or a
template) authors the module stacks. Give click-paths for authoring.

## 1. Spawning

One-shot at a location (impacts, explosions):
```cpp
UNiagaraComponent* FX = UNiagaraFunctionLibrary::SpawnSystemAtLocation(
    this,                         // world context
    ImpactSystem,                 // UNiagaraSystem*
    Hit.ImpactPoint,
    Hit.ImpactNormal.Rotation(),
    FVector(1.f),                 // scale
    true,                         // bAutoDestroy
    true,                         // bAutoActivate
    ENCPoolMethod::AutoRelease,   // reuse components for frequently spawned effects
    true);                        // bPreCullCheck: skip spawning if the Effect Type culls it
```
Attached (muzzle flash, trails, auras):
```cpp
UNiagaraComponent* FX = UNiagaraFunctionLibrary::SpawnSystemAttached(
    MuzzleSystem, WeaponMesh, TEXT("Muzzle"),     // system, attach component, socket
    FVector::ZeroVector, FRotator::ZeroRotator,
    EAttachLocation::SnapToTarget,
    true);                                         // bAutoDestroy
```
The return value can be `nullptr` (culled, invalid system, dedicated server) - always check.

Pool methods (`ENCPoolMethod`):
- `None` - new component every time; fine for rare effects.
- `AutoRelease` - component returns to the world's pool when the system completes. Do not
  keep the pointer past completion.
- `ManualRelease` - you must call `FX->ReleaseToPool()` when done; forgetting leaks pool entries.
Only one-shot systems that actually finish (Emitter State "Once"/finite loops) return to the pool.

Persistent component on an actor (engine glow, looping ambient):
```cpp
// Header
UPROPERTY(VisibleAnywhere, BlueprintReadOnly, Category = "FX")
TObjectPtr<UNiagaraComponent> ThrusterFX;

// Constructor
ThrusterFX = CreateDefaultSubobject<UNiagaraComponent>(TEXT("ThrusterFX"));
ThrusterFX->SetupAttachment(GetRootComponent());
ThrusterFX->bAutoActivate = false;

// Runtime
ThrusterFX->SetAsset(ThrusterSystem);   // or set the asset in the Blueprint defaults
ThrusterFX->Activate(true);             // true = reset
ThrusterFX->Deactivate();               // stops spawning, lets live particles die
ThrusterFX->DeactivateImmediate();      // kills everything now
```
Completion callback:
```cpp
FX->OnSystemFinished.AddDynamic(this, &AMyActor::HandleFXFinished); // UFUNCTION() void HandleFXFinished(UNiagaraComponent* PSystem);
```

## 2. User parameters

Expose inputs in the system (Niagara editor > User Parameters panel > +) and bind modules to
them. Set them from code by the name shown in that panel (the `User.` namespace prefix is
resolved by the component; passing `User.Color` or `Color` targets the same parameter - be
consistent within a project):
```cpp
FX->SetVariableFloat(TEXT("SpawnCount"), 50.f);
FX->SetVariableInt(TEXT("Seed"), 12);
FX->SetVariableBool(TEXT("Burning"), true);
FX->SetVariableVec3(TEXT("BeamEnd"), TargetLocation);
FX->SetVariableLinearColor(TEXT("Color"), FLinearColor::Green);
FX->SetVariableObject(TEXT("SomeObject"), SomeUObject);
FX->SetVariableMaterial(TEXT("SpriteMaterial"), MaterialInterface);
```
Set parameters **before** the system activates when they affect spawning: spawn with
`bAutoActivate = false`, set variables, then `FX->Activate()`.

Mesh and array data interfaces:
```cpp
#include "NiagaraDataInterfaceArrayFunctionLibrary.h"
UNiagaraFunctionLibrary::OverrideSystemUserVariableStaticMesh(FX, TEXT("User.SourceMesh"), SomeStaticMesh);
UNiagaraFunctionLibrary::OverrideSystemUserVariableSkeletalMeshComponent(FX, TEXT("User.SkelMesh"), GetMesh());
UNiagaraDataInterfaceArrayFunctionLibrary::SetNiagaraArrayVector(FX, TEXT("HitPoints"), Points); // TArray<FVector>
```
Parameter types must match exactly (a Vector user param is not a Position param in 5.x
Large World Coordinates; `SetVariablePosition` exists for Position-typed params). Mismatched
names or types fail silently - verify in PIE with the Niagara Debugger.

Blueprint equivalents: "Spawn System at Location", "Spawn System Attached",
"Set Niagara Variable (Float/Vector/LinearColor/...)", "Set Niagara Array Vector".

## 3. Multiplayer and dedicated servers

- VFX are cosmetic: never replicate Niagara components or rely on them for gameplay.
- Trigger from `NetMulticast` (Unreliable), `OnRep_` handlers, or GAS Gameplay Cues (see
  `unreal-gas`, `unreal-multiplayer`).
- On a dedicated server, `SpawnSystem*` does nothing useful; skip the call entirely with
  `if (GetNetMode() == NM_DedicatedServer) return;` to save the lookups.

## 4. Scalability and culling

- Create an **Effect Type** asset per category (Content Browser > Add > FX > Niagara Effect
  Type). Configure: Cull Distance per quality, Max Instances, Significance Handler (distance
  or age), Cull Reaction (Deactivate / Deactivate Immediate / Deactivate + resume), update
  frequency. Assign it in each system (System Properties > Effect Type).
- Per-emitter Scalability overrides (Scalability mode in the Niagara editor toolbar) reduce
  spawn counts or disable emitters on low quality.
- The engine "Effects" scalability group (`sg.EffectsQuality 0-3`/`4`, Settings >
  Engine Scalability Settings > Effects) selects Niagara quality levels. Test at Low.
- Fixed bounds: System Properties or Emitter Properties > Fixed Bounds. **Required for GPU
  emitters** (otherwise culled incorrectly); recommended for CPU emitters with known size to
  skip bounds calculation.
- Warmup (System Properties > Warmup Time) for ambient effects that must look "already running".
- Prefer Lightweight Emitters / Stateless emitters (5.4+, experimental in some versions -
  check) only if the project already uses them.

## 5. Particle materials

- Material usage flags: "Used with Niagara Sprites", "Used with Niagara Ribbons", "Used with
  Niagara Mesh Particles" (set automatically in editor if "Automatically Set Usage" is on;
  set them explicitly for cooked builds).
- `Particle Color` node reads per-particle color (Color module). `Dynamic Parameter` node
  reads per-particle custom values written by the "Dynamic Material Parameters" module.
- Most sprites: Unlit, Translucent or Additive; keep instruction counts very low.
  Soft particles via `Depth Fade` node cost extra; use only where edges visibly clip.
- Flipbooks: `SubUV` sprite renderer settings + "SubUV Animation" module.

## 6. Recipes (how to author, then how to drive)

**Impact burst** (sparks + dust): template "Fountain" or "Simple Sprite Burst"; Emitter State
Once; Spawn Burst Instantaneous; Add Velocity in Cone with its cone axis set to (1,0,0)
(`ImpactNormal.Rotation()` points the component's +X along the normal; make sure the module
works in local space); Gravity Force; Scale Color over life to fade. Drive with
`SpawnSystemAtLocation(..., ENCPoolMethod::AutoRelease)` and `User.Color` for surface type.

**Muzzle flash**: short-lived sprite burst + light renderer (limit light count); spawn
attached to the `Muzzle` socket with `SnapToTarget`. On the owning client only for first-person
meshes, via multicast for others.

**Weapon/sword trail**: Ribbon renderer, spawn rate, particles store position each frame;
attach to the blade socket; `Activate`/`Deactivate` from anim notify states
(built-in "Timed Niagara Effect" notify state handles attach/detach).

**Beam / laser**: template "Dynamic Beam"; expose `User.BeamStart` / `User.BeamEnd` (or
`BeamEnd` only, relative to the component); update each tick with `SetVariableVec3` (or
`SetVariablePosition` if the parameter is Position type).

**Looping ambient** (fire, smoke, dust motes): persistent `UNiagaraComponent` on the actor
or a placed Niagara actor; Effect Type with cull distance; fixed bounds; warmup.

**Footstep dust**: small burst spawned from an anim notify ("Play Niagara Effect" notify
with socket `foot_l`/`foot_r`), or from code after a trace to pick the system by physical
surface (map `EPhysicalSurface` to `UNiagaraSystem*`). Pair with the footstep sound (see `unreal-audio`).

**Hit decal + FX**: `UGameplayStatics::SpawnDecalAtLocation` plus the burst; give the decal a
lifespan and fade.

## 7. Editor work through MCP

Find systems:
```text
ue_search_assets { "class_names": ["NiagaraSystem"], "query": "Impact", "limit": 50 }
```
Place a system in the level for preview and set a user parameter:
```python
import unreal
eas = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
system = unreal.load_asset("/Game/FX/NS_Campfire")
with unreal.ScopedEditorTransaction("Place NS_Campfire"):
    actor = eas.spawn_actor_from_object(system, unreal.Vector(0, 0, 100))
    comp = actor.get_component_by_class(unreal.NiagaraComponent)
    comp.set_variable_float("Intensity", 2.0)
print(actor.get_actor_label())
```
Audit systems for missing Effect Types:
```python
for p in unreal.EditorAssetLibrary.list_assets("/Game/FX", recursive=True):
    a = unreal.load_asset(p)
    if isinstance(a, unreal.NiagaraSystem):
        try:
            print(p, a.get_editor_property("effect_type"))
        except Exception as e:
            print(p, "effect_type not readable:", e)
```
Property names on `NiagaraSystem` can differ per version; use `dir(a)` if a name fails.
Creating a new system from a template is a human task: Content Browser > Add > FX > Niagara
System > "New system from selected emitters" or "From template", pick the template, name it
`NS_<Name>`, then open it to expose User Parameters.

## 8. Debugging and profiling

- Niagara Debugger: Tools > Debug > Niagara Debugger - live list of systems, particle counts,
  culling state, user parameter values; can pause and step.
- `stat Niagara` for counts and ticking cost; `stat gpu` for GPU sim/render cost.
- Viewport > Show > Bounds (or selection) to see system bounds when effects pop out.
- `ue_log` filter `LogNiagara` for errors (missing data interfaces, compile failures).
- Common failures: nothing spawns (culled by Effect Type, bAutoActivate false, finished before
  parameters set), particles vanish at screen edge (bounds), effect only on server (spawned in
  server-only code), pooled component reused with stale parameters (always set all parameters
  after each spawn).
