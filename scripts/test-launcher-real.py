#!/usr/bin/env python3
"""Run the actual launcher twice against a disposable engine and GTK display."""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile

# Every check here is an assert statement, and some perform the action they check;
# under -O they would vanish and the run would report success without doing anything.
if sys.flags.optimize:
    raise SystemExit("Run without python -O or PYTHONOPTIMIZE: this script checks with assert.")

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="relay-launcher-") as temporary:
    base = Path(temporary)
    runtime = base / "runtime"
    runtime.mkdir(mode=0o700)
    (runtime / "relay").mkdir()
    # A legacy socket must never satisfy the new launcher's readiness check.
    legacy = socket.socket(socket.AF_UNIX)
    legacy.bind(str(runtime / "relay/test.sock"))
    legacy.listen()
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime),
               XDG_DATA_HOME=str(base / "data"), XDG_CONFIG_HOME=str(base / "config"),
               XDG_CACHE_HOME=str(base / "cache"), GDK_BACKEND="x11",
               RELAY_NATIVE_SOCKET="/tmp/stale-native-fixture.sock",
               RELAY_NATIVE_SCREENSHOT=str(base / "window.png"),
               RELAY_NATIVE_VERIFY_CONNECTION="1", RELAY_NATIVE_SMOKE_SECONDS="3")
    for key in ("RELAY_SESSION", "RELAY_TOKEN", "RELAY_NATIVE_FIXTURE", "RELAY_NATIVE_PAGE"):
        env.pop(key, None)
    lock = runtime / "relay-v4/test.lock"
    try:
        pids = []
        for _ in range(2):
            result = subprocess.run([str(root / "run.sh"), "test"], cwd="/tmp",
                                    env=env, capture_output=True, text=True, timeout=90)
            assert result.returncode == 0, result.stdout + result.stderr
            assert "Native engine connection verified:" in result.stdout, result.stdout
            assert (base / "window.png").is_file()
            pids.append(int(lock.read_text().strip()))
        assert pids[0] == pids[1], "The second launch must reuse the same V4 engine"
        assert (base / "data/relay-v4/test/store.db").is_file()
        assert not (base / "data/relay/test/store.db").exists()
        reply = subprocess.check_output([str(root / "target/debug/relay"),
            "--instance", "test", "--actor", "user", "q", "project.list"], env=env, text=True)
        assert json.loads(reply)["projects"] == [], "Fixture store should be independent"
    finally:
        legacy.close()
        if lock.exists():
            try:
                os.kill(int(lock.read_text().strip()), signal.SIGTERM)
            except ProcessLookupError:
                pass

print("Actual GTK launcher verified: isolated store/socket, connection, and engine reuse.")
