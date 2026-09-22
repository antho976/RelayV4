# ue_play and ue_profile: drive a play session. Actions: start, status, stop, shot, run, console.

les = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
ues = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem)


def in_play():
    return bool(les.is_in_play_in_editor())


def game_world():
    world = ues.get_game_world()
    if world is None:
        raise RuntimeError("no play session is running")
    return world


action = ARGS["action"]
if action == "start":
    if in_play():
        raise RuntimeError("a play session is already running; stop it first (ue_play with stop_existing=true)")
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
    emit({"requested": used})
elif action == "status":
    emit({"in_play": in_play()})
elif action == "stop":
    if in_play():
        les.editor_request_end_play()
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
else:
    raise RuntimeError("unknown action %r" % action)
