# Which project the running editor has open, as an absolute path.
path = unreal.Paths.get_project_file_path()
emit({"project": unreal.Paths.convert_relative_path_to_full(path), "engine": unreal.SystemLibrary.get_engine_version()})
