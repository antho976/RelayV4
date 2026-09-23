# ue_data_table: Data Tables as text. export returns the table as CSV or JSON; import fills the
# table from text the agent wrote, then saves it.
lib = unreal.DataTableFunctionLibrary
table = load(ARGS["path"], "Data Table")
fmt = ARGS.get("format", "csv")
if ARGS["action"] == "export":
    text = lib.export_data_table_to_csv_string(table) if fmt == "csv" else lib.export_data_table_to_json_string(table)
    emit({"path": ARGS["path"], "format": fmt, "rows": len(lib.get_data_table_row_names(table)), "text": text})
elif ARGS["action"] == "import":
    with unreal.ScopedEditorTransaction("Relay: fill data table"):
        ok = lib.fill_data_table_from_csv_string(table, ARGS["text"]) if fmt == "csv" else lib.fill_data_table_from_json_string(table, ARGS["text"])
    if not ok:
        raise RuntimeError("the engine rejected the %s; read the log (ue_log filter LogDataTable) for the row and column" % fmt)
    save_asset(table)
    emit({"path": ARGS["path"], "rows": len(lib.get_data_table_row_names(table)), "saved": True})
else:
    raise RuntimeError("action must be export or import")
