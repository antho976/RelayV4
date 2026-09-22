# Enhanced Input

Enhanced Input is the default input system in UE 5.1+ (the legacy Action/Axis mappings in
Project Settings > Input are deprecated). Plugin `EnhancedInput` is enabled by default in new
projects; C++ modules need `"EnhancedInput"` in `.Build.cs`.

Check the project uses it (`Config/DefaultInput.ini`):
```ini
[/Script/Engine.InputSettings]
DefaultPlayerInputClass=/Script/EnhancedInput.EnhancedPlayerInput
DefaultInputComponentClass=/Script/EnhancedInput.EnhancedInputComponent
```
If these are missing or point at `PlayerInput`/`InputComponent`, the project still uses legacy
input; `Cast<UEnhancedInputComponent>` will fail. Set them (Project Settings > Engine > Input >
Default Classes) and restart the editor.

## Concepts

| Asset / class | Role |
|---|---|
| `UInputAction` (`IA_`) | A logical action. `ValueType`: `Boolean` (digital), `Axis1D` (float), `Axis2D` (FVector2D), `Axis3D` (FVector). Can carry its own triggers and modifiers applied to every mapping |
| `UInputMappingContext` (`IMC_`) | A set of key -> action mappings, each with optional per-mapping triggers and modifiers |
| `UEnhancedInputLocalPlayerSubsystem` | Per local player; holds the active mapping contexts with priorities |
| `UEnhancedInputComponent` | Binds actions to functions (`BindAction`) |
| `FInputActionValue` | The value passed to handlers; `Get<bool>()`, `Get<float>()`, `Get<FVector2D>()`, `Get<FVector>()` |
| `UInputTrigger` subclasses | Decide when an action fires (Pressed, Hold, Tap, ...) |
| `UInputModifier` subclasses | Transform raw values (dead zone, negate, swizzle, ...) |

Contexts are stacked: higher priority contexts are evaluated first and, by default, keys they
consume are not passed to lower priority contexts for the same key. Use separate contexts for
modes (on foot, driving, menu) and add/remove them when the mode changes.

## Trigger events (`ETriggerEvent`)

| Event | When it fires |
|---|---|
| `Started` | The first evaluation after the trigger begins (e.g. key down) |
| `Ongoing` | Trigger is being evaluated but not yet met (e.g. holding before Hold threshold) |
| `Triggered` | Trigger condition met; with no triggers configured this fires **every frame** while the input is non-zero |
| `Completed` | Trigger stopped after having triggered (e.g. key released) |
| `Canceled` | Trigger stopped before completing (e.g. released before Hold threshold) |

Bind `Triggered` for continuous input (move, look). For a one-shot press, bind `Started`, or add a
`Pressed` trigger and bind `Triggered`. For jump: `Started` -> `Jump`, `Completed` -> `StopJumping`.

## Triggers (editor names; C++ `UInputTrigger...`)

| Trigger | Behavior |
|---|---|
| Down | Triggers every frame while actuated beyond threshold (implicit default) |
| Pressed | Triggers once when actuated |
| Released | Triggers once when released |
| Hold | Triggers after held for `HoldTimeThreshold` (optionally once) |
| Hold And Release | Triggers on release after holding at least the threshold |
| Tap | Triggers if released within `TapReleaseTimeThreshold` |
| Pulse | Triggers repeatedly at an interval while held |
| Chorded Action | Only triggers while another action is triggered (modifier keys like Shift+Key) |
| Combo | Triggers after a sequence of actions (5.1+; verify in your version) |

## Modifiers

| Modifier | Use |
|---|---|
| Dead Zone | Ignore small stick values (`Radial` for sticks, `Axial` per-axis) |
| Negate | Flip sign (per axis flags X/Y/Z); e.g. S key for backward, invert look Y |
| Swizzle Input Axis Values | Reorder axes, e.g. map W/S (1D key) onto Y of an Axis2D action (`YXZ`) |
| Scalar | Multiply per axis (sensitivity) |
| Response Curve - Exponential / User Defined | Non-linear stick response |
| Smooth | Averages over frames |
| FOV Scaling | Scale look input by FOV |
| To World Space | Converts axis input to world-space orientation |

Keyboard WASD on one Axis2D `IA_Move`:
- `W`: Swizzle (YXZ)  -> (0, 1)
- `S`: Swizzle (YXZ) + Negate -> (0, -1)
- `A`: Negate -> (-1, 0)
- `D`: no modifier -> (1, 0)
- Gamepad Left Thumbstick 2D-Axis: Dead Zone.
Mouse XY 2D-Axis on `IA_Look`: often Negate (Y only) so moving the mouse up looks up with
`AddControllerPitchInput`.

## C++ setup (Character)

```cpp
// MyCharacter.h
class UInputAction;
class UInputMappingContext;
struct FInputActionValue;

UCLASS()
class MYGAME_API AMyCharacter : public ACharacter
{
    GENERATED_BODY()
protected:
    virtual void SetupPlayerInputComponent(UInputComponent* PlayerInputComponent) override;
    virtual void PawnClientRestart() override;

    void Move(const FInputActionValue& Value);
    void Look(const FInputActionValue& Value);

    UPROPERTY(EditDefaultsOnly, Category = "Input") TObjectPtr<UInputMappingContext> DefaultMappingContext;
    UPROPERTY(EditDefaultsOnly, Category = "Input") TObjectPtr<UInputAction> MoveAction;
    UPROPERTY(EditDefaultsOnly, Category = "Input") TObjectPtr<UInputAction> LookAction;
    UPROPERTY(EditDefaultsOnly, Category = "Input") TObjectPtr<UInputAction> JumpAction;
};
```
```cpp
// MyCharacter.cpp
#include "MyCharacter.h"
#include "EnhancedInputComponent.h"
#include "EnhancedInputSubsystems.h"
#include "InputActionValue.h"
#include "GameFramework/PlayerController.h"

void AMyCharacter::PawnClientRestart()
{
    Super::PawnClientRestart();
    if (APlayerController* PC = Cast<APlayerController>(GetController()))
    {
        if (UEnhancedInputLocalPlayerSubsystem* Sub =
                ULocalPlayer::GetSubsystem<UEnhancedInputLocalPlayerSubsystem>(PC->GetLocalPlayer()))
        {
            Sub->RemoveMappingContext(DefaultMappingContext);   // avoid duplicates on re-possess
            Sub->AddMappingContext(DefaultMappingContext, /*Priority*/ 0);
        }
    }
}

void AMyCharacter::SetupPlayerInputComponent(UInputComponent* PlayerInputComponent)
{
    Super::SetupPlayerInputComponent(PlayerInputComponent);
    UEnhancedInputComponent* EIC = CastChecked<UEnhancedInputComponent>(PlayerInputComponent);
    EIC->BindAction(MoveAction, ETriggerEvent::Triggered, this, &AMyCharacter::Move);
    EIC->BindAction(LookAction, ETriggerEvent::Triggered, this, &AMyCharacter::Look);
    EIC->BindAction(JumpAction, ETriggerEvent::Started,   this, &ACharacter::Jump);
    EIC->BindAction(JumpAction, ETriggerEvent::Completed, this, &ACharacter::StopJumping);
}

void AMyCharacter::Move(const FInputActionValue& Value)
{
    const FVector2D Axis = Value.Get<FVector2D>();
    if (!Controller) return;
    const FRotator YawRot(0.f, Controller->GetControlRotation().Yaw, 0.f);
    const FVector Forward = FRotationMatrix(YawRot).GetUnitAxis(EAxis::X);
    const FVector Right   = FRotationMatrix(YawRot).GetUnitAxis(EAxis::Y);
    AddMovementInput(Forward, Axis.Y);
    AddMovementInput(Right,   Axis.X);
}

void AMyCharacter::Look(const FInputActionValue& Value)
{
    const FVector2D Axis = Value.Get<FVector2D>();
    AddControllerYawInput(Axis.X);
    AddControllerPitchInput(Axis.Y);
}
```
- Where to add the mapping context: it must run on the owning client after the pawn has a
  `PlayerController` with a `LocalPlayer`. `PawnClientRestart` satisfies that for possessed pawns;
  the 5.0-5.3 templates use `BeginPlay` (works only when possession happened before BeginPlay);
  newer templates override `NotifyControllerChanged` (verify it exists in your `Pawn.h`). Mapping
  contexts that belong to the player regardless of pawn (UI, global shortcuts) go in the
  PlayerController's `BeginPlay` (guarded by `IsLocalController()`) and its `SetupInputComponent`.
- Other binding forms: `BindAction(Action, Event, this, &AClass::FuncNoArgs)` (handler with no
  parameters), handler taking `const FInputActionInstance&` for elapsed/triggered time, and
  `BindActionValueLambda` / `BindActionInstanceLambda` (check the header for your version).
- `BindAction` returns `FEnhancedInputActionEventBinding&`; remove with
  `EIC->RemoveBindingByHandle(Binding.GetHandle())` or `ClearActionBindings()`.
- Null action pointers (asset not assigned in the BP subclass) bind nothing and silently do
  nothing: `ensure(MoveAction)` or log.
- Blueprint: Input Action events appear as "EnhancedInputAction IA_Move" nodes with Triggered,
  Started, Ongoing, Canceled, Completed exec pins.

## Creating input assets with Python

Input Actions and Mapping Contexts are binary assets. Through `ue_python` (verify the factory
class names first: `print([n for n in dir(unreal) if "Input" in n and "Factory" in n])`):

```python
import unreal
tools = unreal.AssetToolsHelpers.get_asset_tools()
folder = "/Game/MyGame/Input"

def make(name, cls, factory_name):
    full = f"{folder}/{name}"
    if unreal.EditorAssetLibrary.does_asset_exist(full):
        return unreal.EditorAssetLibrary.load_asset(full)
    factory = getattr(unreal, factory_name)()
    return tools.create_asset(name, folder, cls, factory)

with unreal.ScopedEditorTransaction("Create input assets"):
    ia_move = make("IA_Move", unreal.InputAction, "InputAction_Factory")
    ia_move.set_editor_property("value_type", unreal.InputActionValueType.AXIS2D)
    ia_jump = make("IA_Jump", unreal.InputAction, "InputAction_Factory")
    ia_jump.set_editor_property("value_type", unreal.InputActionValueType.BOOLEAN)
    imc = make("IMC_Default", unreal.InputMappingContext, "InputMappingContext_Factory")

    def key(name):
        k = unreal.Key()
        k.set_editor_property("key_name", name)
        return k

    imc.map_key(ia_jump, key("SpaceBar"))
    m_d = imc.map_key(ia_move, key("D"))
    m_a = imc.map_key(ia_move, key("A"))
    # modifiers on a mapping: set the mapping's "modifiers" array to new modifier objects
    # (e.g. unreal.InputModifierNegate / unreal.InputModifierSwizzleAxis); verify with help(m_a)

unreal.EditorAssetLibrary.save_directory(folder, only_if_is_dirty=True, recursive=True)
```
- `map_key` returns the `EnhancedActionKeyMapping` struct by reference in C++; in Python you may
  get a copy. If modifiers set on it do not stick, read back `imc.get_editor_property("mappings")`,
  modify the list and write it back with `set_editor_property("mappings", ...)` (5.3/5.4 layout;
  newer versions may nest mappings differently: inspect with `dir()`/`help()`).
- Key names are FKey names: `W`, `A`, `S`, `D`, `SpaceBar`, `LeftShift`, `LeftMouseButton`,
  `Mouse2D`, `MouseX`, `MouseY`, `Gamepad_Left2D`, `Gamepad_Right2D`, `Gamepad_FaceButton_Bottom`,
  `Gamepad_LeftTrigger`. Verify an unfamiliar one against `EKeys` in
  `Engine/Source/Runtime/InputCore/Classes/InputCoreTypes.h`.
- If scripting modifiers proves unreliable, create the assets by script and give the human the exact
  per-mapping modifier list to add in the IMC editor (open `IMC_Default` > Mappings > expand the
  key > Modifiers > +).
- Then assign the assets to the character's Blueprint defaults (CDO) as shown in
  `unreal-blueprints` (`default_mapping_context`, `move_action`, ...), compile and save.

## Runtime rebinding and user settings

- Simple approach: add/remove whole mapping contexts per mode.
- Player-remappable keys (5.3+): enable "Enable User Settings" in Project Settings > Engine >
  Enhanced Input, mark mappings as player-mappable (Player Mappable Key Settings on the IA or
  mapping), and use `UEnhancedInputUserSettings` (from the local player subsystem's
  `GetUserSettings()`). `UPlayerMappableInputConfig` is deprecated in 5.3 in favor of this system.
  API names changed between 5.2 and 5.4: check `EnhancedInputUserSettings.h` in your engine.
- `Sub->RequestRebuildControlMappings()` forces re-evaluation after changing mappings at runtime
  (context add/remove already does this).

## Debugging

- Console (`ue_console` during PIE): `showdebug enhancedinput` shows active contexts, actions and
  values on screen.
- Nothing happens on input: (1) IMC never added for this local player, (2) IA properties not
  assigned on the Blueprint subclass, (3) project still on legacy input classes, (4) a higher
  priority context consumes the key, (5) UI input mode (`FInputModeUIOnly`) swallows game input,
  (6) pawn not possessed by the local player controller.
- Axis values wrong direction: fix modifiers on the mapping (Negate/Swizzle), not in code, so
  gamepad and keyboard stay consistent.
- Continuous action firing only once: bound to `Started` instead of `Triggered`, or a `Pressed`
  trigger on the action.
