#!/usr/bin/env python3
"""Exercise launcher lifecycle without opening a UI or touching real engines."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

# Every check here is an assert statement, and some perform the action they check;
# under -O they would vanish and the run would report success without doing anything.
if sys.flags.optimize:
    raise SystemExit("Run without python -O or PYTHONOPTIMIZE: this script checks with assert.")

with tempfile.TemporaryDirectory(prefix="relay launcher ") as directory:
    root = Path(directory)
    shutil.copy2(Path(__file__).resolve().parents[1] / "run.sh", root)
    # run.sh registers the desktop entry on a dev launch. A stub stands in for the installer,
    # and XDG_DATA_HOME points into the fixture, so the user's own launcher is never touched.
    (root / "scripts").mkdir()
    (root / "scripts/install-native-desktop.py").write_text(
        "import sys\nwith open('desktop-installs', 'a') as f:\n    f.write(sys.argv[1] + '\\n')\n")
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
    executable(binaries / "relay-native", '[[ -z "${RELAY_NATIVE_SOCKET+x}" ]]\necho "$RELAY_INSTANCE" >> windows\n')
    env = dict(os.environ, PATH=f"{fake_bin}:{os.environ['PATH']}",
               RELAY_NATIVE_SOCKET="/tmp/stale-fixture.sock", XDG_DATA_HOME=str(root / "data"))
    env.pop("RELAY_INSTALL_DESKTOP", None)

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
    installs = (root / "desktop-installs").read_text().split() if (root / "desktop-installs").exists() else []
    assert "test" not in installs, "only the dev instance may repoint the desktop entry"
    assert not (root / "data").exists(), "the launcher test must not write a desktop entry"
    windows = (root / "windows").read_text()
    assert launch(BUILD_EXIT="7").returncode == 7
    assert launch("invalid").returncode == 2
    (root / "ready").unlink()
    failure = launch(FAIL_START="1")
    assert failure.returncode == 1 and "fixture-start-failure" in failure.stderr
    assert (root / "windows").read_text() == windows, "must not open after failure"

print("Launcher startup, reuse, instance selection and failure checks passed.")
