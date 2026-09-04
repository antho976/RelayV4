#!/usr/bin/env python3
"""Exercise launcher lifecycle without opening a UI or touching real engines."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

with tempfile.TemporaryDirectory(prefix="relay launcher ") as directory:
    root = Path(directory)
    shutil.copy2(Path(__file__).resolve().parents[1] / "run.sh", root)
    binaries = root / "target/debug"
    binaries.mkdir(parents=True)
    fake_bin = root / "bin"
    fake_bin.mkdir()

    def executable(path, source):
        path.write_text("#!/usr/bin/env bash\nset -eu\n" + source)
        path.chmod(0o755)

    executable(fake_bin / "cargo", 'exit "${BUILD_EXIT:-0}"\n')
    executable(binaries / "relay", '''
[[ "$1" == --instance && "$2" == "$RELAY_INSTANCE" ]]
case "$3" in
    ping) test -f ready ;;
    serve)
        echo start >> starts
        if [[ "${FAIL_START:-0}" == 1 ]]; then echo fixture-start-failure; exit 1; fi
        touch ready
        ;;
esac
''')
    executable(binaries / "relay-native", 'echo "$RELAY_INSTANCE" >> windows\n')
    env = dict(os.environ, PATH=f"{fake_bin}:{os.environ['PATH']}")

    def launch(*args, **overrides):
        return subprocess.run([str(root / "run.sh"), *args], cwd="/tmp",
                              env=dict(env, **overrides), capture_output=True,
                              text=True, timeout=25)

    assert launch().returncode == 0
    assert launch().returncode == 0
    assert (root / "starts").read_text() == "start\n", "must reuse engine"
    assert (root / "windows").read_text() == "dev\ndev\n"
    assert launch("test").returncode == 0
    assert (root / "windows").read_text().endswith("test\n")
    windows = (root / "windows").read_text()
    assert launch(BUILD_EXIT="7").returncode == 7
    assert launch("invalid").returncode == 2
    (root / "ready").unlink()
    failure = launch(FAIL_START="1")
    assert failure.returncode == 1 and "fixture-start-failure" in failure.stderr
    assert (root / "windows").read_text() == windows, "must not open after failure"

print("Launcher startup, reuse, instance selection and failure checks passed.")
