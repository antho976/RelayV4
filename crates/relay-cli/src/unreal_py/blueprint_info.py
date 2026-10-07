# ue_blueprint_info: what a Blueprint is made of, as text. Blueprint graphs are binary; this reads
# what the editor exposes to Python and says plainly what it could not read.

registry = unreal.AssetRegistryHelpers.get_asset_registry()
TAGS = ("ParentClass", "NativeParentClass", "BlueprintType", "IsDataOnly", "ImplementedInterfaces",
        "NumReplicatedProperties", "BlueprintDescription", "BlueprintCategory")


def short(value, limit=160):
    text = str(value)
    return text if len(text) <= limit else text[:limit] + "..."


def public_names(obj):
    names = set()
    for n in dir(obj):
        if not n.startswith("_"):
            names.add(n)
    return names


def describe(path):
    bp = load(path, "Blueprint")
    out = {"path": path, "class": bp.get_class().get_name()}
    data = registry.get_asset_by_object_path(path if "." in path.rsplit("/", 1)[-1] else path + "." + path.rsplit("/", 1)[-1])
    tags = {}
    for tag in TAGS:
        try:
            value = data.get_tag_value(tag)
            if value:
                tags[tag] = str(value)
        except Exception:
            pass
    out["tags"] = tags
    notes = []

    # Variables and functions: what the generated class adds over its native parent.
    try:
        cls = unreal.EditorAssetLibrary.load_blueprint_class(path)
        cdo = unreal.get_default_object(cls)
        native_path = tags.get("NativeParentClass", "")
        native_name = native_path.split(".")[-1].strip("'\"")
        native_cls = getattr(unreal, native_name[1:] if native_name[:1] in "AU" and not hasattr(unreal, native_name) else native_name, None)
        base = public_names(unreal.get_default_object(native_cls)) if native_cls else set()
        variables, functions = {}, []
        for name in sorted(public_names(cdo) - base):
            try:
                value = getattr(cdo, name)
            except Exception:
                continue
            if callable(value):
                functions.append(name)
            else:
                variables[name] = short(value)
        out["variables"] = variables
        out["functions_and_events"] = functions
        if not native_cls:
            notes.append("native parent class not resolved; variables list may include inherited ones")
    except Exception as error:
        notes.append("class defaults unreadable: %s" % error)

    # Components, through the subobject data subsystem (5.0+).
    try:
        sub = unreal.get_engine_subsystem(unreal.SubobjectDataSubsystem)
        handles = sub.k2_gather_subobject_data_for_blueprint(bp)
        lib = unreal.SubobjectDataBlueprintFunctionLibrary
        components = []
        for handle in handles:
            d = lib.get_data(handle)
            obj = lib.get_object(d)
            if obj is None:
                continue
            components.append({"name": str(lib.get_variable_name(d)), "class": obj.get_class().get_name()})
        out["components"] = components
    except Exception as error:
        notes.append("components unreadable: %s" % error)

    # Graphs and nodes: exposed on some versions only.
    graphs = []
    try:
        lib = unreal.BlueprintEditorLibrary
        graph = lib.find_event_graph(bp)
        if graph is not None:
            entry = {"name": graph.get_name()}
            try:
                nodes = graph.get_editor_property("nodes")
                entry["nodes"] = [n.get_class().get_name() + ": " + short(n.get_name(), 80) for n in nodes][:300]
            except Exception:
                entry["nodes"] = None
                notes.append("graph nodes are not readable from Python on this engine version; describe graph changes to the human as steps")
            graphs.append(entry)
    except Exception as error:
        notes.append("graphs unreadable (is the Blueprint Editor Library available?): %s" % error)
    out["graphs"] = graphs

    if ARGS.get("compile"):
        try:
            unreal.BlueprintEditorLibrary.compile_blueprint(bp)
            out["compiled"] = True
        except Exception as error:
            out["compiled"] = False
            notes.append("compile failed to run: %s" % error)
    out["notes"] = notes
    return out


paths = ARGS.get("paths") or []
if ARGS.get("folder"):
    for d in registry.get_assets_by_path(ARGS["folder"], recursive=True):
        cls = getattr(d, "asset_class_path", None)
        name = str(cls.asset_name) if cls is not None else str(d.asset_class)
        if name in ("Blueprint", "WidgetBlueprint", "AnimBlueprint"):
            paths.append(str(d.package_name))
limit = int(ARGS.get("limit", 50))
results = []
cancelled = False
with unreal.ScopedSlowTask(min(len(paths), limit), "Reading Blueprints") as task:
    task.make_dialog(True)  # Cancel stops it and returns what was read so far
    for path in paths[:limit]:
        if task.should_cancel():
            cancelled = True
            break
        task.enter_progress_frame(1)
        try:
            results.append(describe(path))
        except Exception as error:
            results.append({"path": path, "error": str(error)})
emit({"blueprints": results, "count": len(results), "truncated": len(paths) > limit, "cancelled": cancelled})
