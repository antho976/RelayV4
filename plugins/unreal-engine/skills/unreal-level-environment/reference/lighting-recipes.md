# Lighting Recipes (UE 5.3–5.6, Lumen + Virtual Shadow Maps)

## 0. Decide the unit philosophy first

- **Physical**: sun of about 75,000–120,000 lux, exposure locked near EV100 14–15 at noon, and local lights
  in real units (a 60 W-equivalent bulb is 800 lm, about 64 cd). It matches references and Megascans
  well, but every emissive and local light must use real values too.
- **Relative** (the default feel of a new UE5 level): the Directional Light at its default of about 10 lux, with auto exposure
  adapting. It is easy to start with, and you then narrow the exposure range for control.

Pick one and write it into the project's art or lighting doc. Mixing the two produces
invisible or blown-out local lights. Exposure is EV100: for incident illuminance E (lux),
`EV100 ≈ log2(E / 2.5)`, so 10 lux gives about 2, 1,000 lux about 8.6, and 100,000 lux about 15.3.

## 1. Project and PPV baseline

Project Settings > Engine > Rendering (stored in `Config/DefaultEngine.ini`,
`[/Script/Engine.RendererSettings]`):
- Dynamic Global Illumination Method: Lumen. Reflection Method: Lumen.
- Shadow Map Method: Virtual Shadow Maps.
- Generate Mesh Distance Fields: on (software Lumen needs them).
- Support Hardware Ray Tracing plus *Use Hardware Ray Tracing when available* (Lumen section): optional. It improves
  reflections and thin geometry on RT-capable GPUs. Requires DX12 or SM6 on Windows.
- Auto Exposure > *Extend default luminance range in Auto Exposure settings*: on (the UE5 default). The PPV exposure
  values below assume EV100.

Post Process Volume (one per level, *Infinite Extent (Unbound)* on):
- Exposure: Metering Mode *Auto Exposure Histogram* (or *Manual* for fully authored shots), Min
  and Max EV100 per recipe, Exposure Compensation for final tuning, and Speed Up/Down around 3/1.
- Global Illumination and Reflections: leave the method on Lumen, and raise the quality settings (Lumen Scene Lighting
  Quality, Final Gather Quality, Max Trace Distance) only when you have measured a need.
- Use additional *bounded* PPVs (with Blend Radius) for interiors, caves and water, each with its own exposure.

## 2. Outdoor sky stack (all outdoor recipes)

Window > Env. Light Mixer creates any missing pieces. Otherwise place these:
1. **Directional Light**: Mobility Movable (needed for a dynamic time of day). *Atmosphere Sun Light* on,
   index 0. Source Angle about 0.5 (softer shadows need a larger value). Cast Shadows on.
2. **Sky Atmosphere**: defaults are Earth-like. The sun's rotation drives the sky color.
3. **Sky Light**: Mobility Movable, *Real Time Capture* on (captures the atmosphere and clouds). Intensity 1.
4. **Exponential Height Fog**: Fog Density about 0.01–0.05, Height Falloff about 0.2. Enable *Volumetric
   Fog* for god rays, and set the light's *Volumetric Scattering Intensity* to control them.
5. **Volumetric Cloud** (optional): uses a cloud material (the engine includes a simple default one). Clouds cost
   GPU time, so budget them.

| Recipe | Sun pitch | Sun intensity (physical / relative) | Exposure Min–Max EV100 (physical) | Notes |
|---|---|---|---|---|
| Clear noon | −60 to −80 | 100,000 lux / 10 lux | 14–15.5 | Harsh shadows. Raise the Source Angle slightly for softness |
| Morning or afternoon | −25 to −40 | 100,000 / 10 | 13–15 | Best form-reading. The default choice for gameplay |
| Golden hour | −3 to −10 | 100,000 / 10 (the atmosphere attenuates it) | 10–13 | Warm color comes from the atmosphere. Keep the light color white-ish |
| Overcast | −40 to −60 | 20,000–40,000 / 3–5 | 12–14 | Dense clouds or a lower sun. Raise Sky Light's contribution. Softer Source Angle (2–5) |
| Night (moon) | moon at −30 to −60 | 0.3–2 lux on a second directional light | −1 to 3 (game-readable) | Moon uses *Atmosphere Sun Light Index* 1. A cool light color. Readability over realism |

Rotate the light for time of day. The "sun" is the Directional Light's rotation, and pitch is negative
for above the horizon.

## 3. Interior lit through windows

- Lumen brings skylight and sun through openings. Keep walls at least 10–20 cm thick to avoid leaks.
- Use a bounded PPV inside with lower exposure (for example physical Min/Max EV100 6–10), and blend
  at the doorway with a Blend Radius of about 200–500.
- If it's too dark, first add practical local lights (lamps, window Rect Lights). Lumen *Skylight Leaking* in the PPV
  (small values, 0.05–0.2) is a stylized fallback.
- Rect Lights at windows can fake bounce. Keep *Cast Shadows* only where needed, because shadowed local lights cost VSM pages.
- Emissive materials contribute to Lumen GI, but they are noisy and unstable for small, bright sources. Use a real light.

## 4. Night interior / cave / dungeon

- No sun. Sky Light at a very low intensity or off. Height fog tinted dark.
- The key light comes from practicals (torches, lamps) as Point or Spot Lights. Use *Attenuation Radius* as tight as
  the look allows. Keep overlapping shadow-casting lights to a few in any view.
- Guide the player with light. The exit or goal is the brightest or warmest area.
- Exposure: a narrow range (physical EV100 about 2–6, or tune relative), so walking past a torch doesn't
  pump. For a horror tone, fully *Manual* exposure is an option.
- Volumetric fog with a high Volumetric Scattering Intensity on key lights gives shafts and atmosphere.

## 5. Common problems → fixes

| Symptom | Likely cause | Fix |
|---|---|---|
| Light bleeding through walls | Walls under about 10 cm, or single-sided planes | Thicken walls, add a hidden blocker mesh, and check the mesh's distance field (Visualize > Mesh Distance Fields) |
| Blotchy or noisy GI | Low final gather quality, or small emissive sources | Raise *Final Gather Quality* in the PPV, replace emissive sources with lights, and check large-mesh Lumen cards |
| Dark interiors | Low skylight inside, or auto exposure pinned at Max | Local lights, an interior PPV exposure range, and small Skylight Leaking |
| Exposure pumping | A wide Min/Max EV100 range | Narrow the range per area. Slow Speed Down |
| Blurry or swimming shadows on foliage | WPO invalidating VSM pages | Limit WPO distance on non-Nanite foliage, and use Nanite foliage where supported |
| Shadow popping at distance | Distance limits, or HLOD without shadows | Check the light's dynamic shadow distance settings and HLOD shadow settings |
| Reflections missing detail | Software Lumen limits | Hardware ray tracing where available, or add a Sphere or Box Reflection Capture for non-Lumen fallbacks |
| Emissive not lighting the scene | Emissive too dim relative to exposure | Use real lights, or increase emissive in the same unit philosophy |
| Sky too dark or too bright | Sky Light not capturing, or exposure out of range | *Real Time Capture* on, or `recapture_sky()`, and reset exposure |

Debug views: viewport *Lit* dropdown > Lumen (Overview, Lumen Scene, Surface Cache), *Mesh Distance
Fields*, *Virtual Shadow Map*, *Nanite Visualization*, and *Exposure* (Visualize HDR (Eye
Adaptation) under Show > Visualize). Console: `stat unit`, `stat gpu`, `ProfileGPU`.

## 6. Scripted outdoor stack (Python, editor open)

```python
import unreal

eas = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
existing = {type(a).__name__ for a in eas.get_all_level_actors()}

def spawn(cls, label, loc=unreal.Vector(0, 0, 0), rot=unreal.Rotator(roll=0, pitch=0, yaw=0)):
    a = eas.spawn_actor_from_class(cls, loc, rot)
    a.set_actor_label(label)
    a.set_folder_path("Lighting")
    return a

with unreal.ScopedEditorTransaction("Outdoor lighting stack"):
    if "DirectionalLight" not in existing:
        sun = spawn(unreal.DirectionalLight, "Sun", rot=unreal.Rotator(roll=0, pitch=-35, yaw=40))
        lc = sun.get_component_by_class(unreal.DirectionalLightComponent)
        lc.set_mobility(unreal.ComponentMobility.MOVABLE)
        lc.set_editor_property("atmosphere_sun_light", True)
        lc.set_intensity(10.0)             # relative philosophy; use about 100000 for physical
    if "SkyAtmosphere" not in existing:
        spawn(unreal.SkyAtmosphere, "SkyAtmosphere")
    if "SkyLight" not in existing:
        sky = spawn(unreal.SkyLight, "SkyLight")
        sc = sky.get_component_by_class(unreal.SkyLightComponent)
        sc.set_mobility(unreal.ComponentMobility.MOVABLE)
        sc.set_editor_property("real_time_capture", True)
    if "ExponentialHeightFog" not in existing:
        fog = spawn(unreal.ExponentialHeightFog, "HeightFog")
        fc = fog.get_component_by_class(unreal.ExponentialHeightFogComponent)
        fc.set_fog_density(0.02)
        fc.set_volumetric_fog(True)
    if "PostProcessVolume" not in existing:
        ppv = spawn(unreal.PostProcessVolume, "PPV_Global")
        ppv.set_editor_property("unbound", True)
        s = ppv.get_editor_property("settings")
        s.set_editor_property("override_auto_exposure_min_brightness", True)
        s.set_editor_property("auto_exposure_min_brightness", 0.0)   # EV100 when the extended range is on
        s.set_editor_property("override_auto_exposure_max_brightness", True)
        s.set_editor_property("auto_exposure_max_brightness", 4.0)
        ppv.set_editor_property("settings", s)

unreal.get_editor_subsystem(unreal.LevelEditorSubsystem).save_current_level()
```

If a property name raises an error, inspect it with `help(unreal.PostProcessSettings)` or
`help(unreal.SkyLightComponent)`. Python names are the snake_case form of the C++ UPROPERTY (with the `b`
prefix dropped). The EV100 values in the script suit the relative philosophy. For physical lighting use the table above.

## 7. Performance budget notes

- Lumen and VSM costs scale with resolution. Use TSR and a screen percentage for lower-end targets.
  Scalability groups (`sg.GlobalIlluminationQuality`, `sg.ShadowQuality`) switch Lumen and shadow quality per preset.
- Limit shadow-casting movable local lights in view, and turn off *Cast Shadows* on fill lights.
- Volumetric fog and clouds are fixed-cost-ish. Measure them with `stat gpu` before and after.
- If the target platform can't run Lumen (mobile, low-end), plan baked lighting (Lightmass with Static or
  Stationary lights) from the start. It is a different workflow: lightmap UVs, and Build > Build Lighting.
