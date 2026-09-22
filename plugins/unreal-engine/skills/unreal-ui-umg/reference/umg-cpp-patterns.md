# UMG C++ patterns

Copy-ready patterns for C++-backed widgets. Replace `MYGAME_API` with the module's API macro.
Module dependencies: `UMG` (public), `Slate`, `SlateCore` (private); add `CommonUI`, `CommonInput`
for CommonUI, `ModelViewViewModel`, `FieldNotification` for the MVVM plugin.

## 1. Base widget with bound children, event-driven updates

```cpp
// HealthBarWidget.h
#pragma once
#include "CoreMinimal.h"
#include "Blueprint/UserWidget.h"
#include "HealthBarWidget.generated.h"

class UProgressBar;
class UTextBlock;
class UWidgetAnimation;

UCLASS(Abstract)
class MYGAME_API UHealthBarWidget : public UUserWidget
{
    GENERATED_BODY()
public:
    UFUNCTION(BlueprintCallable, Category = "HUD")
    void SetHealth(float Current, float Max);

protected:
    virtual void NativeOnInitialized() override;
    virtual void NativeConstruct() override;
    virtual void NativeDestruct() override;

    UPROPERTY(meta = (BindWidget)) TObjectPtr<UProgressBar> HealthBar;
    UPROPERTY(meta = (BindWidget)) TObjectPtr<UTextBlock> HealthText;
    UPROPERTY(Transient, meta = (BindWidgetAnimOptional)) TObjectPtr<UWidgetAnimation> DamageFlash;

private:
    UFUNCTION() void HandleHealthChanged(float NewHealth, float NewMax);
    float LastHealth = -1.f;
};
```

```cpp
// HealthBarWidget.cpp
#include "HealthBarWidget.h"
#include "Components/ProgressBar.h"
#include "Components/TextBlock.h"
#include "Animation/WidgetAnimation.h"
#include "MyHealthComponent.h"   // your component exposing a dynamic multicast OnHealthChanged(float, float)

#define LOCTEXT_NAMESPACE "HealthBar"

void UHealthBarWidget::NativeOnInitialized()
{
    Super::NativeOnInitialized(); // one-time setup; bound widgets are valid from here on
}

void UHealthBarWidget::NativeConstruct()
{
    Super::NativeConstruct();
    if (APawn* Pawn = GetOwningPlayerPawn())
    {
        if (UMyHealthComponent* Health = Pawn->FindComponentByClass<UMyHealthComponent>())
        {
            Health->OnHealthChanged.AddUniqueDynamic(this, &UHealthBarWidget::HandleHealthChanged);
            SetHealth(Health->GetHealth(), Health->GetMaxHealth()); // initial state
        }
    }
}

void UHealthBarWidget::NativeDestruct()
{
    if (APawn* Pawn = GetOwningPlayerPawn())
    {
        if (UMyHealthComponent* Health = Pawn->FindComponentByClass<UMyHealthComponent>())
        {
            Health->OnHealthChanged.RemoveDynamic(this, &UHealthBarWidget::HandleHealthChanged);
        }
    }
    Super::NativeDestruct();
}

void UHealthBarWidget::HandleHealthChanged(float NewHealth, float NewMax) { SetHealth(NewHealth, NewMax); }

void UHealthBarWidget::SetHealth(float Current, float Max)
{
    const float Pct = Max > 0.f ? Current / Max : 0.f;
    HealthBar->SetPercent(Pct);

    FFormatNamedArguments Args;
    Args.Add(TEXT("Current"), FText::AsNumber(FMath::CeilToInt(Current)));
    Args.Add(TEXT("Max"), FText::AsNumber(FMath::CeilToInt(Max)));
    HealthText->SetText(FText::Format(LOCTEXT("HealthFmt", "{Current} / {Max}"), Args));

    if (DamageFlash && LastHealth >= 0.f && Current < LastHealth)
    {
        PlayAnimation(DamageFlash); // StartAtTime 0, 1 loop, Forward, speed 1
    }
    LastHealth = Current;
}

#undef LOCTEXT_NAMESPACE
```

Notes:
- The pawn may change (respawn). For a HUD that outlives pawns, listen to
  `AController::OnPossessedPawnChanged` (dynamic multicast with old/new pawn) and
  rebind; or let the pawn push data to the HUD.
- With GAS, bind to `UAbilitySystemComponent::GetGameplayAttributeValueChangeDelegate(Attribute)`
  instead (see `unreal-gas`).

Designer steps for the human (`WBP_HealthBar`, parent `UHealthBarWidget`): root Overlay; child
Progress Bar named `HealthBar` (Fill/Fill); child Text Block named `HealthText` (Center/Center,
font from the project text style); optional animation named `DamageFlash` animating the Overlay's
Render Opacity or a tint. Compile, save.

## 2. HUD ownership

Create the HUD in the PlayerController's `BeginPlay` behind `IsLocalController()` (the same shape
as the root-layout code in `commonui-setup.md`), keep it in a `UPROPERTY(Transient)` pointer,
remove it in `EndPlay`. Expose the class as `UPROPERTY(EditDefaultsOnly) TSubclassOf<UUserWidget>
HUDClass` and set it in the PlayerController Blueprint defaults, or from Python:
`unreal.get_default_object(unreal.EditorAssetLibrary.load_blueprint_class(pc_bp_path)).set_editor_property("hud_class", unreal.EditorAssetLibrary.load_blueprint_class(wbp_path))`
followed by saving the PlayerController Blueprint.

## 3. List View entries

```cpp
// InventoryEntryWidget.h
#pragma once
#include "CoreMinimal.h"
#include "Blueprint/UserWidget.h"
#include "Blueprint/IUserObjectListEntry.h"
#include "InventoryEntryWidget.generated.h"

class UTextBlock; class UImage;

UCLASS(Abstract)
class MYGAME_API UInventoryEntryWidget : public UUserWidget, public IUserObjectListEntry
{
    GENERATED_BODY()
protected:
    virtual void NativeOnListItemObjectSet(UObject* ListItemObject) override; // called on every (re)use
    UPROPERTY(meta = (BindWidget)) TObjectPtr<UTextBlock> NameText;
    UPROPERTY(meta = (BindWidgetOptional)) TObjectPtr<UImage> IconImage;
};
```

- List items are `UObject`s (e.g. a `UInventoryItemData` per slot). Feed with
  `ListView->SetListItems(Items)` or `AddItem`. Keep the items referenced (`UPROPERTY` array)
  or they are garbage-collected.
- Entries are pooled and reused: `NativeOnListItemObjectSet` must fully reset the entry (clear
  the icon when the item has none). Never store per-item state in the entry.
- The List View's **Entry Widget Class** must implement `UserObjectListEntry`; a WBP parented to
  this C++ class does.
- Load icons asynchronously from `TSoftObjectPtr<UTexture2D>`: `UImage::SetBrushFromSoftTexture`.

## 4. MVVM viewmodel (UMG Viewmodel plugin, Beta)

```cpp
// HealthViewModel.h
#pragma once
#include "CoreMinimal.h"
#include "MVVMViewModelBase.h"
#include "HealthViewModel.generated.h"

UCLASS(BlueprintType)
class MYGAME_API UHealthViewModel : public UMVVMViewModelBase
{
    GENERATED_BODY()
public:
    void SetCurrentHealth(float NewValue)
    {
        if (UE_MVVM_SET_PROPERTY_VALUE(CurrentHealth, NewValue))
        {
            UE_MVVM_BROADCAST_FIELD_VALUE_CHANGED(GetHealthPercent);
        }
    }
    float GetCurrentHealth() const { return CurrentHealth; }

    void SetMaxHealth(float NewValue)
    {
        if (UE_MVVM_SET_PROPERTY_VALUE(MaxHealth, NewValue))
        {
            UE_MVVM_BROADCAST_FIELD_VALUE_CHANGED(GetHealthPercent);
        }
    }
    float GetMaxHealth() const { return MaxHealth; }

    UFUNCTION(BlueprintPure, FieldNotify)
    float GetHealthPercent() const { return MaxHealth > 0.f ? CurrentHealth / MaxHealth : 0.f; }

private:
    UPROPERTY(BlueprintReadWrite, FieldNotify, Setter, Getter, meta = (AllowPrivateAccess))
    float CurrentHealth = 0.f;

    UPROPERTY(BlueprintReadWrite, FieldNotify, Setter, Getter, meta = (AllowPrivateAccess))
    float MaxHealth = 100.f;
};
```

- `Setter`/`Getter` require functions named exactly `Set<Prop>`/`Get<Prop>`.
- `UE_MVVM_SET_PROPERTY_VALUE` compares, assigns and broadcasts; it returns true on change.
- Derived values are `UFUNCTION(BlueprintPure, FieldNotify)` and must be broadcast manually when
  an input changes.
- Human steps: open the WBP, Window > Viewmodels, add `HealthViewModel`, choose its creation type
  (Create Instance, Manual, Global Viewmodel Collection, Property Path or Resolver), then in
  Window > View Bindings bind `HealthBar.Percent` <- `HealthViewModel.GetHealthPercent` (one way
  to widget). Compile.
- Setting a Manual viewmodel from C++ goes through the widget's `UMVVMView` extension
  (`View/MVVMView.h`, `SetViewModel(FName, TScriptInterface<INotifyFieldValueChanged>)`); the
  exact API changed across 5.1-5.5, so read that header in the project's engine before using it.
  The Blueprint route (the generated setter node for the viewmodel on the widget) is stable.

## 5. World-space / nameplate widget

```cpp
// In the character constructor
#include "Components/WidgetComponent.h"

Nameplate = CreateDefaultSubobject<UWidgetComponent>(TEXT("Nameplate"));
Nameplate->SetupAttachment(GetRootComponent());
Nameplate->SetRelativeLocation(FVector(0.f, 0.f, 110.f));
Nameplate->SetWidgetSpace(EWidgetSpace::Screen);  // readable, cheap; World = 3D quad with render target
Nameplate->SetDrawAtDesiredSize(true);
Nameplate->SetCollisionEnabled(ECollisionEnabled::NoCollision);
```

```cpp
// BeginPlay: the widget object exists once the component has initialized (not on dedicated servers)
if (UNameplateWidget* W = Cast<UNameplateWidget>(Nameplate->GetUserWidgetObject()))
{
    W->SetDisplayName(DisplayName);
}
```

- Set the Widget Class in the Blueprint defaults (`SetWidgetClass` in C++ also works).
- World space: set Draw Size, enable Two Sided if visible from behind, and for static panels
  reduce updates (the component's redraw-time / manual-redraw settings; check the Details panel
  of `UWidgetComponent` in your version). Each World-space component owns a render target.
- Hide nameplates beyond a distance and for the local player's own pawn.
- Interaction with World-space widgets needs a `UWidgetInteractionComponent` on the player.

## 6. Settings with UGameUserSettings

```cpp
// MyGameUserSettings.h
#pragma once
#include "CoreMinimal.h"
#include "GameFramework/GameUserSettings.h"
#include "MyGameUserSettings.generated.h"

UCLASS(config = GameUserSettings, configdonotcheckdefaults)
class MYGAME_API UMyGameUserSettings : public UGameUserSettings
{
    GENERATED_BODY()
public:
    static UMyGameUserSettings* Get() { return Cast<UMyGameUserSettings>(UGameUserSettings::GetGameUserSettings()); }

    virtual void SetToDefaults() override
    {
        Super::SetToDefaults();
        bSubtitlesEnabled = true; SubtitleScale = 1.f; UIScale = 1.f; MouseSensitivity = 1.f;
    }

    UPROPERTY(Config, BlueprintReadWrite, Category = "Accessibility") bool  bSubtitlesEnabled = true;
    UPROPERTY(Config, BlueprintReadWrite, Category = "Accessibility") float SubtitleScale = 1.f;
    UPROPERTY(Config, BlueprintReadWrite, Category = "Accessibility") float UIScale = 1.f;
    UPROPERTY(Config, BlueprintReadWrite, Category = "Controls")      float MouseSensitivity = 1.f;
};
```

Register it in `Config/DefaultEngine.ini`:

```ini
[/Script/Engine.Engine]
GameUserSettingsClassName=/Script/MyGame.MyGameUserSettings
```

Apply from the settings screen:

```cpp
#include "Kismet/KismetSystemLibrary.h"

UMyGameUserSettings* S = UMyGameUserSettings::Get();
TArray<FIntPoint> Resolutions;
UKismetSystemLibrary::GetSupportedFullscreenResolutions(Resolutions);   // fill the dropdown

S->SetFullscreenMode(EWindowMode::WindowedFullscreen);
S->SetScreenResolution(ChosenResolution);
S->SetVSyncEnabled(bVSync);
S->SetFrameRateLimit(FrameCap);            // 0 = unlimited
S->SetOverallScalabilityLevel(Quality);    // 0 Low, 1 Medium, 2 High, 3 Epic, 4 Cinematic
S->ApplySettings(false);                   // applies resolution + scalability, then saves to GameUserSettings.ini
```

- "Auto-detect": `S->RunHardwareBenchmark(); S->ApplyHardwareBenchmarkResults();`.
- For resolution/window mode, show a confirm countdown and call `S->RevertVideoMode()` if not
  confirmed, `S->ConfirmVideoMode()` if confirmed.
- Edit a copy of the values in the screen and write them to settings only on Apply; Back
  discards. Custom values take effect where you read them (subtitle widget, input sensitivity).
- The saved file is `Saved/Config/<Platform>/GameUserSettings.ini` - per user, not in source control.

## 7. Python for Widget Blueprints

Find widget blueprints and their parents:

```python
import unreal
ar = unreal.AssetRegistryHelpers.get_asset_registry()
flt = unreal.ARFilter(
    class_paths=[unreal.TopLevelAssetPath("/Script/UMGEditor", "WidgetBlueprint")],
    package_paths=["/Game/UI"], recursive_paths=True)
for ad in ar.get_assets(flt):
    print(ad.package_name, ad.get_tag_value("ParentClass"))
```

Create one parented to C++ (see SKILL.md), reparent an existing one, compile:

```python
import unreal
bp = unreal.load_asset("/Game/UI/HUD/WBP_HealthBar")
new_parent = unreal.load_class(None, "/Script/MyGame.HealthBarWidget")
with unreal.ScopedEditorTransaction("Reparent WBP_HealthBar"):
    unreal.BlueprintEditorLibrary.reparent_blueprint(bp, new_parent)
unreal.BlueprintEditorLibrary.compile_blueprint(bp)
unreal.EditorAssetLibrary.save_asset(bp.get_path_name())
```

Then read `ue_log` with `filter: "Error|Warning|binding"` - missing `BindWidget` names appear
there. Reparenting to a class with new required bindings will fail to compile until the human
adds the widgets; tell them the exact names and types.

What Python cannot do reliably: add/remove/rename widgets in the tree, set slot anchors,
author UMG animations. Hand those to the human with exact click-paths:
Palette (search by widget name) -> drag onto the Hierarchy -> rename in Details (top field) ->
tick/untick **Is Variable** -> set Slot properties (anchors, alignment, padding, size) ->
Compile -> Save.
