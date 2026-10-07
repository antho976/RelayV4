# ue_play and ue_profile: drive a play session. Actions: start, status, stop, shot, run, console.

les = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
TAG = "RelayPlayCapture"
ues = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem)


def in_play():
    return bool(les.is_in_play_in_editor())


def game_world():
    world = ues.get_game_world()
    if world is None:
        raise RuntimeError("no play session is running")
    return world


def perf_settings():
    return unreal.get_default_object(unreal.EditorPerformanceSettings)


def frame_count():
    try:
        return int(unreal.SystemLibrary.get_frame_count())
    except Exception:
        return None


action = ARGS["action"]
if action == "start":
    if in_play():
        raise RuntimeError("a play session is already running; stop it first (ue_play with stop_existing=true)")
    # An unfocused editor throttles itself to a few frames per second, which silently ruins
    # timed play sessions and profiles. Turn that off for the session; `stop` restores it.
    throttled = None
    try:
        settings = perf_settings()
        throttled = bool(settings.get_editor_property("throttle_cpu_when_not_foreground"))
        settings.set_editor_property("throttle_cpu_when_not_foreground", False)
        # Keep it off in the user's editor settings when this engine exposes save_config, so
        # it stays off after a restart; then there is nothing to restore at the end.
        if ARGS.get("persist", True) and hasattr(settings, "save_config"):
            settings.save_config()
            throttled = False
    except Exception:
        pass
    mode = ARGS.get("mode", "pie")
    if mode == "simulate":
        les.editor_play_simulate()
        used = "simulate"
    else:
        begin = getattr(les, "editor_request_begin_play", None)
        if begin is None:
            les.editor_play_simulate()
            used = "simulate (this engine version exposes no play-in-editor request to Python)"
        else:
            begin()
            used = "pie"
    emit({"requested": used, "was_throttled": throttled})
elif action == "status":
    emit({"in_play": in_play(), "frame": frame_count(), "seconds": unreal.SystemLibrary.get_game_time_in_seconds(ues.get_game_world()) if in_play() else None})
elif action == "stop":
    if in_play():
        les.editor_request_end_play()
    if ARGS.get("restore_throttle") is not None:
        try:
            perf_settings().set_editor_property("throttle_cpu_when_not_foreground", bool(ARGS["restore_throttle"]))
        except Exception:
            pass
    emit({"stopped": True})
elif action == "shot":
    world = game_world()
    unreal.SystemLibrary.execute_console_command(world, "HighResShot %dx%d" % (int(ARGS.get("width", 1280)), int(ARGS.get("height", 720))))
    emit({"requested": True})
elif action == "console":
    world = game_world() if in_play() else editor_world()
    for command in ARGS.get("commands", []):
        unreal.SystemLibrary.execute_console_command(world, command)
    emit({"ran": ARGS.get("commands", [])})
elif action == "run":
    # The agent's own probe, with the game world at hand: read positions, health, state.
    scope = {"unreal": unreal, "world": game_world(), "emit_value": None}
    exec(compile(ARGS["code"], "probe", "exec"), scope)
    emit({"ok": True})
elif action == "prepare_outside":
    # Python cannot spawn into a running game, but the game world is a copy of the level: a
    # capture placed in the level before play exists in the game, found by its tag. It cannot be
    # transient (transient actors are not copied into play), so it does mark the level modified;
    # a level that had no unsaved changes before is noted, and ue_editor_quit names it if its
    # save then rewrites it.
    level = editor_map_package()
    if level and level not in dirty_map_packages():
        relay_state().maps_clean_before_relay.add(level)
    for old in actor_subsystem().get_all_level_actors():
        if TAG in [str(t) for t in old.get_editor_property("tags")]:
            actor_subsystem().destroy_actor(old)
    loc, rot = ues.get_level_viewport_camera_info()
    cam = actor_subsystem().spawn_actor_from_class(unreal.SceneCapture2D, loc, rot)
    cam.set_actor_label("RelayPlayCapture")
    cam.set_folder_path("RelayPreview")
    cam.set_editor_property("tags", [unreal.Name(TAG)])
    comp = cam.get_editor_property("capture_component2d")
    comp.set_editor_property("capture_every_frame", False)
    comp.set_editor_property("capture_on_movement", False)
    emit({"prepared": cam.get_path_name()})
elif action == "cleanup_outside":
    gone = [a for a in actor_subsystem().get_all_level_actors() if TAG in [str(t) for t in a.get_editor_property("tags")]]
    for a in gone:
        actor_subsystem().destroy_actor(a)
    emit({"removed": len(gone)})
elif action == "outside_capture":
    world = game_world()
    caps = unreal.GameplayStatics.get_all_actors_with_tag(world, unreal.Name(TAG))
    if not caps:
        raise RuntimeError("no play capture in the game world; ue_play places it before play starts")
    cam = caps[0]
    comp = cam.get_editor_property("capture_component2d")
    width, height = int(ARGS.get("width", 960)), int(ARGS.get("height", 540))
    fov = float(ARGS.get("fov", 60.0))
    rt = unreal.RenderingLibrary.create_render_target2d(world, width, height, unreal.TextureRenderTargetFormat.RTF_RGBA8)
    comp.set_editor_property("texture_target", rt)
    comp.set_editor_property("capture_source", unreal.SceneCaptureSource.SCS_FINAL_COLOR_LDR)
    comp.set_editor_property("fov_angle", fov)
    # The target: the player's pawn, or an actor named by label, name or class fragment.
    want = ARGS.get("target") or "player"
    if want == "player":
        target = unreal.GameplayStatics.get_player_pawn(world, 0)
    else:
        target = None
        for a in unreal.GameplayStatics.get_all_actors_of_class(world, unreal.Actor):
            if want in (a.get_actor_label(), a.get_name()) or want.lower() in a.get_class().get_name().lower():
                target = a
                break
    if target is None:
        raise RuntimeError("no target %r in the game world" % want)
    # Where to look from, in the target's own frame: [forward, right, up] cm from its origin.
    base = vec(target.get_actor_location())
    fwd = vec(target.get_actor_forward_vector())
    fwd = normalize((fwd[0], fwd[1], 0.0)) if abs(fwd[2]) < 0.99 else (1.0, 0.0, 0.0)
    right = normalize(cross((0.0, 0.0, 1.0), fwd))
    up = (0.0, 0.0, 1.0)

    def along(o):
        return add(add(mul(fwd, o[0]), mul(right, o[1])), mul(up, o[2]))

    # Offsets and views are measured from the point looked at (default: ahead of the chest,
    # where first-person hands and guns sit), in the target's [forward, right, up] frame.
    look_at = [float(c) for c in (ARGS.get("look_at") or [30.0, 0.0, 50.0])]
    look = add(base, along(look_at))
    d = float(ARGS.get("distance", 150.0))
    presets = {"front": [d, 0.0, 15.0], "back": [-d, 0.0, 30.0], "right": [0.0, d, 10.0], "left": [0.0, -d, 10.0],
               "three_quarter": [d * 0.7, d * 0.7, 25.0], "three_quarter_left": [d * 0.7, -d * 0.7, 25.0], "top": [1.0, 0.0, d * 1.5]}
    shots = []
    if ARGS.get("offset"):
        shots.append(("offset", [float(c) for c in ARGS["offset"]]))
    for name in ARGS.get("views") or ([] if ARGS.get("offset") else ["right", "front"]):
        if name not in presets:
            raise RuntimeError("unknown view %r; use %s" % (name, ", ".join(sorted(presets))))
        shots.append((name, presets[name]))
    # First-person arms and guns are usually "only owner see": hidden from any other view.
    # Show them for the capture, then put the flags back.
    flipped = []
    for c in target.get_components_by_class(unreal.PrimitiveComponent):
        try:
            if c.get_editor_property("only_owner_see"):
                c.set_only_owner_see(False)
                flipped.append((c, "only_owner_see"))
            if ARGS.get("show_owner_hidden", True) and c.get_editor_property("owner_no_see"):
                c.set_owner_no_see(False)
                flipped.append((c, "owner_no_see"))
        except Exception:
            pass
    files = []
    try:
        for name, o in shots:
            eye = add(look, along(o))
            d = sub(look, eye)
            rot = unreal.Rotator(roll=0.0, pitch=math.degrees(math.atan2(d[2], math.sqrt(d[0] * d[0] + d[1] * d[1]))), yaw=math.degrees(math.atan2(d[1], d[0])))
            cam.set_actor_location_and_rotation(unreal.Vector(*eye), rot, False, True)
            comp.capture_scene()
            file_name = "%s_%s.png" % (ARGS.get("prefix", "outside"), name)
            unreal.RenderingLibrary.export_render_target(world, rt, ARGS["out_dir"], file_name)
            files.append({"view": "outside " + name, "file": ARGS["out_dir"] + "/" + file_name, "eye": rnd(eye)})
    finally:
        for c, flag in flipped:
            try:
                (c.set_only_owner_see if flag == "only_owner_see" else c.set_owner_no_see)(True)
            except Exception:
                pass
    emit({"files": files, "target": target.get_name(), "shown_for_capture": len(flipped)})
else:
    raise RuntimeError("unknown action %r" % action)
