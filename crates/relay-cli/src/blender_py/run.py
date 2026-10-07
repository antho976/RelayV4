# blender_python: the agent's own script, with `bpy` and `ARGS`, then an optional save.
import addon_utils, collections, io, os
for module in ARGS.get("addons") or []:
    addon_utils.enable(module, default_set=True)
# No .blend1 backups: an overwrite would leave a full-size untracked copy next to the art, which
# nothing ignores or puts in LFS. Factory-startup preferences are never saved, so this stays here.
bpy.context.preferences.filepaths.save_version = 0
OUTPUT_CAP = 64000


class TailBuffer(io.TextIOBase):
    """What the script prints, stdout and stderr together, keeping only the last OUTPUT_CAP
    characters: a print loop that never ends costs neither the MCP server's memory nor the reply."""
    def __init__(self):
        self.parts, self.size, self.dropped = collections.deque(), 0, 0

    def writable(self):
        return True

    @property
    def encoding(self):
        return "utf-8"

    def fileno(self):
        return sys.__stdout__.fileno()

    def write(self, text):
        self.parts.append(text)
        self.size += len(text)
        while self.size > OUTPUT_CAP:
            first = self.parts[0]
            cut = min(len(first), self.size - OUTPUT_CAP)
            if cut == len(first):
                self.parts.popleft()
            else:
                self.parts[0] = first[cut:]
            self.size -= cut
            self.dropped += cut
        return len(text)

    def text(self):
        head = "[%d earlier characters of output dropped]\n" % self.dropped if self.dropped else ""
        return head + "".join(self.parts)


# __name__ so a script's `if __name__ == "__main__":` block runs. The result must be an object
# (the server adds output and saved to it), so emit(3) or emit([...]) arrives as {"value": ...}.
scope = {"__name__": "__main__", "bpy": bpy, "ARGS": ARGS, "Vector": Vector, "Matrix": Matrix, "math": math,
         "emit": lambda value: emit(value if isinstance(value, dict) else {"value": value})}
print("RELAY_OUT_BEGIN", flush=True)
captured = TailBuffer()
sys.stdout = sys.stderr = captured
try:
    exec(compile(ARGS["code"], "script", "exec"), scope)
finally:
    sys.stdout, sys.stderr = sys.__stdout__, sys.__stderr__
    text = captured.text()
    print(text, end="" if not text or text.endswith("\n") else "\n")
    print("RELAY_OUT_END", flush=True)


def refuse_if_changed(target):
    """blender.rs records the opened file's mtime and size before Blender reads it. If the file on
    disk is no longer that one, someone else saved it meanwhile, and saving would undo their work."""
    opened = ARGS.get("_opened")
    if not opened or not os.path.exists(target) or not os.path.samefile(target, opened["path"]):
        return
    st = os.stat(target)
    if (st.st_mtime_ns, st.st_size) != (opened["mtime_ns"], opened["size"]):
        raise RuntimeError("not saved: %s changed on disk after this run opened it (another agent or program saved it), and saving would overwrite that work. Run the change again on the current file, or save_as a new file." % target)


saved = None
if ARGS.get("save_as"):
    os.makedirs(os.path.dirname(os.path.abspath(ARGS["save_as"])), exist_ok=True)
    refuse_if_changed(ARGS["save_as"])
    bpy.ops.wm.save_as_mainfile(filepath=ARGS["save_as"], copy=False)
    saved = ARGS["save_as"]
elif ARGS.get("save"):
    if not bpy.data.filepath:
        raise RuntimeError("save needs an opened file; use save_as for a new one")
    refuse_if_changed(bpy.data.filepath)
    bpy.ops.wm.save_mainfile()
    saved = bpy.data.filepath
print("RELAY_SAVED:" + json.dumps(saved))
