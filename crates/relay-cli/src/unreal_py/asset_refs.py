# ue_asset_refs: what an asset depends on and what depends on it.
registry = unreal.AssetRegistryHelpers.get_asset_registry()
options = unreal.AssetRegistryDependencyOptions(include_soft_package_references=True, include_hard_package_references=True,
                                               include_searchable_names=False, include_soft_management_references=False,
                                               include_hard_management_references=False)
path = ARGS["path"].split(".")[0]
depth = max(1, min(int(ARGS.get("depth", 1)), 4))


def walk(start, fn):
    seen, frontier, levels = set([start]), [start], []
    for _ in range(depth):
        nxt = []
        for p in frontier:
            for q in fn(p, options) or []:
                q = str(q)
                if q not in seen:
                    seen.add(q)
                    nxt.append(q)
        levels.append(sorted(nxt))
        frontier = nxt
    return levels


if not unreal.EditorAssetLibrary.does_asset_exist(path):
    raise RuntimeError("no asset %s" % path)
emit({"path": path, "depends_on": walk(path, registry.get_dependencies),
      "referenced_by": walk(path, registry.get_referencers),
      "note": "Level 1 is direct; later levels are indirect. /Script paths are C++ classes."})
