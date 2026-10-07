#!/usr/bin/env python3
"""Register this native checkout's launcher and icon for the current user."""
import os
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
launcher = root / "run.sh"
instance = sys.argv[1] if len(sys.argv) > 1 else "dev"
if instance not in ("dev", "test", "stable"):
    raise SystemExit("Expected dev, test, or stable")
data = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
if not data.is_absolute():
    raise SystemExit("XDG_DATA_HOME must be an absolute path")

# Desktop Entry arguments are parsed without a shell. Escape field codes and
# reserved characters so a checkout path with spaces or punctuation stays one argv.
argument = str(launcher).replace("%", "%%")
for character in ("\\", '"', "`", "$"):
    argument = argument.replace(character, "\\" + character)
# The key file's string escapes are undone before the quoting above is read, so every
# backslash written so far has to survive that first pass as itself.
argument = argument.replace("\\", "\\\\")
template = root / "apps/relay-native/resources/com.quietsoftware.Relay4.desktop"
lines = template.read_text().splitlines(keepends=True)
execs = [at for at, line in enumerate(lines) if line.rstrip("\n") == "Exec=relay-native"]
if len(execs) != 1:
    raise SystemExit(f"{template}: expected exactly one 'Exec=relay-native' line, found {len(execs)}")
lines[execs[0]] = f'Exec="{argument}" {instance}\n'
desktop = "".join(lines)
assets = {
    data / "applications/com.quietsoftware.Relay4.desktop": desktop.encode(),
    data / "icons/hicolor/scalable/apps/com.quietsoftware.Relay4.svg":
        (root / "apps/relay-native/resources/com.quietsoftware.Relay4.svg").read_bytes(),
}
for target, content in assets.items():
    target.parent.mkdir(parents=True, exist_ok=True)
    if not target.exists() or target.read_bytes() != content:
        target.write_bytes(content)
