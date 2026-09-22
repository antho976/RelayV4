# CommonUI setup and patterns

Read this before building menus that must work with a gamepad, or when the project already has
the `CommonUI` plugin enabled. CommonUI replaces "call SetInputMode and hope" with a router that
activates one screen at a time and sends UI input only to the active widget tree.

Plugin names in `.uproject`: `CommonUI` (enabling it pulls in `CommonInput`). Build.cs:

```csharp
PublicDependencyModuleNames.AddRange(new string[] { "UMG", "CommonUI", "CommonInput" });
PrivateDependencyModuleNames.AddRange(new string[] { "Slate", "SlateCore", "EnhancedInput" });
```

Lyra (Epic's sample) uses CommonUI plus a Lyra-only `CommonGame` plugin (`UPrimaryGameLayout`,
`UCommonUIExtensions::PushContentToLayer_ForPlayer`). `CommonGame` is not part of the engine;
if the project does not already contain it, build the small root layout shown below instead.

## Step-by-step setup

Items marked (agent) can be done as text or Python; (human) need the editor UI unless you script
them through `ue_python`.

1. **Enable the plugin** (agent): add to `.uproject` `Plugins`:
   `{ "Name": "CommonUI", "Enabled": true }`. The editor must restart.
2. **Viewport client** (agent, text): in `Config/DefaultEngine.ini`
   ```ini
   [/Script/Engine.Engine]
   GameViewportClientClassName=/Script/CommonUI.CommonGameViewportClient
   ```
   Same as Project Settings > Engine > General Settings > Default Classes > Game Viewport Client
   Class. If the project already has a custom viewport client, make it derive from
   `UCommonGameViewportClient` instead. Without this, CommonUI input does nothing.
3. **Input action data table** (human, or Python): create a Data Table with row struct
   `CommonInputActionDataBase` (e.g. `/Game/UI/Input/DT_UIInputActions`). Rows at minimum:
   `Confirm` (Gamepad Face Button Bottom, Enter), `Back` (Gamepad Face Button Right, Escape),
   plus any tab/navigation actions (`TabLeft` = Gamepad Left Shoulder, `TabRight` = Right
   Shoulder). Each row has Display Name, and key info per input type (Keyboard, Gamepad, Touch).
4. **UI input data** (human): create a Blueprint class with parent `CommonUIInputData`
   (`/Game/UI/Input/B_CommonInputData`). Set **Default Click Action** -> `DT_UIInputActions:Confirm`
   and **Default Back Action** -> `DT_UIInputActions:Back`.
5. **Controller data** (human): create Blueprint classes with parent `CommonInputBaseControllerData`,
   one per input type/gamepad family: e.g. `B_Input_KBM` (Input Type = Mouse and Keyboard),
   `B_Input_Gamepad_XSX` (Input Type = Gamepad, Gamepad Name = the platform gamepad name, e.g.
   `XSX`, or `Generic` on PC). Fill the key -> icon brush map so action widgets show correct glyphs.
6. **Common Input Settings** (human, Project Settings > Game > Common Input Settings):
   - Input Data = `B_CommonInputData`.
   - Platform Input > (each platform, e.g. Windows): Default Input Type, Supports Mouse and
     Keyboard / Gamepad / Touch, Default Gamepad Name, and add the Controller Data classes.
   These land in `Config/DefaultGame.ini` and per-platform `Config/<Platform>/<Platform>Game.ini`;
   read the diff afterwards so the human sees what changed.
7. **Enhanced Input (5.2+, optional)**: Common Input Settings has an option to enable Enhanced
   Input support (`bEnableEnhancedInputSupport`). When on, `CommonUIInputData` also accepts
   `UInputAction` assets for click/back, and buttons can bind to Input Actions instead of data
   table rows. The metadata classes and exact property names moved between 5.2 and 5.5; search
   `Engine/Plugins/Runtime/CommonUI/Source/CommonInput` for `EnhancedInput` in the project's
   engine version before scripting it. Keep one scheme per project (data tables or Input Actions).
8. **Styles** (human): create `CommonTextStyle`, `CommonButtonStyle`, `CommonBorderStyle`
   Blueprint subclasses for the project's typography and buttons, and set them as defaults in
   Project Settings > Plugins > Common UI Editor (template text/button/border style).
9. **Root layout** (agent C++ + human WBP): one widget added to the viewport per local player,
   containing named stacks (layers) - see below.

## Root layout with stacks

```cpp
// MyRootLayout.h
#pragma once
#include "CoreMinimal.h"
#include "CommonUserWidget.h"
#include "MyRootLayout.generated.h"

class UCommonActivatableWidgetStack;
class UCommonActivatableWidget;

UCLASS(Abstract)
class MYGAME_API UMyRootLayout : public UCommonUserWidget
{
    GENERATED_BODY()
public:
    // Push a screen onto the menu layer. Returns the instance (cached by the stack).
    UFUNCTION(BlueprintCallable, Category = "UI")
    UCommonActivatableWidget* PushMenu(TSubclassOf<UCommonActivatableWidget> ScreenClass);

    UFUNCTION(BlueprintCallable, Category = "UI")
    UCommonActivatableWidget* PushModal(TSubclassOf<UCommonActivatableWidget> ScreenClass);

protected:
    UPROPERTY(meta = (BindWidget)) TObjectPtr<UCommonActivatableWidgetStack> GameLayer;  // HUD
    UPROPERTY(meta = (BindWidget)) TObjectPtr<UCommonActivatableWidgetStack> MenuLayer;  // pause, settings
    UPROPERTY(meta = (BindWidget)) TObjectPtr<UCommonActivatableWidgetStack> ModalLayer; // confirm dialogs
};
```

```cpp
// MyRootLayout.cpp
#include "MyRootLayout.h"
#include "Widgets/CommonActivatableWidgetContainer.h"
#include "CommonActivatableWidget.h"

UCommonActivatableWidget* UMyRootLayout::PushMenu(TSubclassOf<UCommonActivatableWidget> ScreenClass)
{
    return MenuLayer && ScreenClass ? MenuLayer->AddWidget(ScreenClass) : nullptr;
}

UCommonActivatableWidget* UMyRootLayout::PushModal(TSubclassOf<UCommonActivatableWidget> ScreenClass)
{
    return ModalLayer && ScreenClass ? ModalLayer->AddWidget(ScreenClass) : nullptr;
}
```

Typed push with initialization before activation:

```cpp
MenuLayer->AddWidget<UMyConfirmScreen>(ConfirmClass, [&](UMyConfirmScreen& Screen)
{
    Screen.SetMessage(Message); // runs before the widget is activated
});
```

Human steps for `WBP_RootLayout` (parent `UMyRootLayout`): root **Overlay**; add three
**Common Activatable Widget Stack** children named exactly `GameLayer`, `MenuLayer`,
`ModalLayer`, in that order (later children draw on top), each Fill/Fill alignment. On
`MenuLayer` and `ModalLayer`, set Transition Type (e.g. Fade Only) and Transition Duration
(0.1-0.2 s). Compile, save.

Create it from the local PlayerController:

```cpp
void AMyPlayerController::BeginPlay()
{
    Super::BeginPlay();
    if (!IsLocalController() || !RootLayoutClass) { return; }
    RootLayout = CreateWidget<UMyRootLayout>(this, RootLayoutClass);
    RootLayout->AddToPlayerScreen(1000);
    // Add a PushGame() twin of PushMenu() that targets GameLayer, and push the HUD screen there.
}
```

(`RootLayoutClass` is `UPROPERTY(EditDefaultsOnly) TSubclassOf<UMyRootLayout>`, `RootLayout` a
`UPROPERTY() TObjectPtr<UMyRootLayout>`.)

## Activatable widget base

```cpp
// MyActivatableScreen.h
#pragma once
#include "CoreMinimal.h"
#include "CommonActivatableWidget.h"
#include "MyActivatableScreen.generated.h"

UENUM(BlueprintType)
enum class EMyScreenInputMode : uint8 { Game, GameAndMenu, Menu };

UCLASS(Abstract)
class MYGAME_API UMyActivatableScreen : public UCommonActivatableWidget
{
    GENERATED_BODY()
public:
    virtual TOptional<FUIInputConfig> GetDesiredInputConfig() const override;

protected:
    virtual UWidget* NativeGetDesiredFocusTarget() const override;
    virtual void NativeOnActivated() override;
    virtual void NativeOnDeactivated() override;
    virtual bool NativeOnHandleBackAction() override;

    UPROPERTY(EditDefaultsOnly, Category = "Input")
    EMyScreenInputMode InputMode = EMyScreenInputMode::Menu;

    // Designer names the widget that should get focus on activation.
    UPROPERTY(meta = (BindWidgetOptional)) TObjectPtr<UWidget> DefaultFocus;
};
```

```cpp
// MyActivatableScreen.cpp
#include "MyActivatableScreen.h"
#include "Input/CommonUIInputTypes.h"

TOptional<FUIInputConfig> UMyActivatableScreen::GetDesiredInputConfig() const
{
    switch (InputMode)
    {
    case EMyScreenInputMode::Game:        return FUIInputConfig(ECommonInputMode::Game, EMouseCaptureMode::CapturePermanently);
    case EMyScreenInputMode::GameAndMenu: return FUIInputConfig(ECommonInputMode::All,  EMouseCaptureMode::NoCapture);
    case EMyScreenInputMode::Menu:
    default:                              return FUIInputConfig(ECommonInputMode::Menu, EMouseCaptureMode::NoCapture);
    }
}

UWidget* UMyActivatableScreen::NativeGetDesiredFocusTarget() const
{
    return DefaultFocus ? DefaultFocus.Get() : Super::NativeGetDesiredFocusTarget();
}

void UMyActivatableScreen::NativeOnActivated()
{
    Super::NativeOnActivated();
    // Refresh from model; pause the game here if this is the pause screen.
}

void UMyActivatableScreen::NativeOnDeactivated()
{
    // Unpause / stop listening here.
    Super::NativeOnDeactivated();
}

bool UMyActivatableScreen::NativeOnHandleBackAction()
{
    // Default implementation deactivates the widget when bIsBackHandler is true.
    return Super::NativeOnHandleBackAction();
}
```

Notes:
- Tick **Is Back Handler** (`bIsBackHandler`) in the WBP class defaults (or set it in the C++
  constructor) for screens that close on Back. The HUD screen must not be a back handler.
- `FUIInputConfig` constructors differ slightly across 5.x (some versions add a mouse lock
  argument); check `CommonUIInputTypes.h` in the engine if the call does not compile.
- The input config of the **topmost active** widget wins. Pushing a Menu-mode pause screen over a
  Game-mode HUD switches to menu input and shows the cursor; popping it restores game input.
- Do not call `APlayerController::SetInputMode` for screens managed by CommonUI.
- `DeactivateWidget()` closes the screen; a stack then removes it and activates the one below.
- Activation focus: CommonUI focuses `GetDesiredFocusTarget()` on activation when the input type
  is gamepad. If nothing gets focus on gamepad, this function is returning null.

## Buttons and actions

- Derive button Blueprints from `CommonButtonBase` (Blueprint class `WBP_Button_Base`), assign a
  `CommonButtonStyle`. In C++, bind `OnClicked()` (a `FCommonButtonEvent` accessor):
  `ConfirmButton->OnClicked().AddUObject(this, &UMyScreen::HandleConfirm);` in `NativeOnInitialized`.
- To make a button fire from a gamepad face button anywhere on the screen, set its
  **Triggering Input Action** (data table row, or Input Action with Enhanced Input support).
- `UCommonBoundActionBar` in the root layout shows prompts for all actions bound on the active
  screen (set its Action Button Class to a `CommonBoundActionButton` WBP).
- Screen-level actions without a visible button: `RegisterUIActionBinding(FBindUIActionArgs(...))`
  in `NativeOnActivated`/`NativeOnInitialized`, storing the returned `FUIActionBindingHandle` and
  unregistering it in `NativeDestruct`. `FBindUIActionArgs` has constructors taking a data table
  row handle, a `FUIActionTag`, or (with Enhanced Input support) a `UInputAction*`; check
  `Input/CommonUIInputTypes.h` in the CommonUI plugin for the exact overloads in your version.

## Tabs

`UCommonTabListWidgetBase` + `UCommonAnimatedSwitcher` (or `UCommonActivatableWidgetSwitcher`):
register tabs with `RegisterTab(TabId, ButtonClass, ContentWidget)`, set Next/Previous Tab Input
Action rows (shoulder buttons) on the tab list, and link the switcher. Tab content that is itself
activatable gets activated as tabs change.

## Input type switching

`UCommonInputSubsystem::Get(LocalPlayer)` exposes `GetCurrentInputType()` and
`OnInputMethodChangedNative`. Use it to swap glyphs or hide mouse-only hints. CommonUI hides the
cursor automatically when the input type becomes gamepad (with Menu input config).

## Troubleshooting

| Symptom | Cause |
|---|---|
| Back / Confirm do nothing; gamepad ignored | Viewport client not `CommonGameViewportClient`, or Input Data not set in Common Input Settings |
| Log warning about missing default click/back action | `CommonUIInputData` rows unset |
| Button icons blank | No Controller Data for current platform/input type, or icon map missing the key |
| Game still moves while menu open | Menu screen's input config is `All`/`Game`, or the menu is not the active widget |
| Two menus both react | They are in different stacks and both active; put modal dialogs in a layer above and keep one active screen per layer |
| Focus lost after closing a dialog | Underlying screen's `NativeGetDesiredFocusTarget` returns null |

Debugging: `CommonUI.DumpActivatableTree` console command exists in recent versions (check with
`ue_console` - an unknown command just logs a warning); the Widget Reflector shows focus.
