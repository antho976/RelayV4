#!/usr/bin/env python3
"""Exercise setup and footer utilities with real GTK + engine, a fake GitHub CLI,
and local clones. No user credentials, store, device builds or network calls."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / ".impeccable/review"
OUT.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix="relay-setup-") as temporary:
    base = Path(temporary)
    runtime = base / "runtime"
    runtime.mkdir(mode=0o700)
    workspace = base / "projects"
    repo = workspace if os.environ.get("RELAY_SETUP_ROOT_REPO") else workspace / "local-project"
    repo.mkdir(parents=True)
    def git(*args):
        subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True)
    git("init", "-q", "-b", "main")
    git("config", "user.name", "Fixture")
    git("config", "user.email", "fixture@relay.test")
    (repo / "README.md").write_text("# Fixture\n")
    git("add", ".")
    git("commit", "-qm", "fixture")
    fakebin = base / "bin"
    fakebin.mkdir()
    repos = [dict(name="github-project", full_name="fixture/github-project", description="Private repository fixture", clone_url=str(repo), ssh_url=str(repo), private=True, archived=False, updated_at="2026-09-04T00:00:00Z")]
    gh = fakebin / "gh"
    auth = base / "github-connected"
    auth.touch()
    gh.write_text("#!/usr/bin/env python3\nimport sys,json\nfrom pathlib import Path\nauth=Path(" + repr(str(auth)) + ")\nif 'auth' in sys.argv:\n auth.touch();sys.exit(0)\nif 'user' in sys.argv:\n print('fixture' if auth.exists() else '');sys.exit(0 if auth.exists() else 1)\nprint(" + repr(json.dumps(repos)) + ")\n")
    gh.chmod(0o755)
    env = dict(os.environ, PATH=f"{fakebin}:{os.environ['PATH']}", XDG_DATA_HOME=str(base / "data"), XDG_CONFIG_HOME=str(base / "config"), XDG_CACHE_HOME=str(base / "cache"), XDG_RUNTIME_DIR=str(runtime), RELAY_INSTANCE="test")
    for key in ("RELAY_SESSION", "RELAY_TOKEN", "RELAY_BRIEF"):
        env.pop(key, None)
    provider = base / "provider"
    provider.write_text("#!/usr/bin/env python3\nimport sys\nif '--version' in sys.argv: print('fixture 1.0');sys.exit()\nif 'auth' in sys.argv or 'login' in sys.argv: print('{\"loggedIn\":true}');sys.exit()\nprint('LOCAL WORKFLOW FIXTURE READY',flush=True)\nfor line in sys.stdin: print(line,flush=True)\n")
    provider.chmod(0o755)
    launcher = base / "removed-launch-worktree"
    launcher.mkdir()
    with (OUT / "setup-engine.log").open("w") as log:
        engine = subprocess.Popen([str(ROOT / "target/debug/relay"), "--instance", "test", "serve"], cwd=launcher, env=env, stdout=log, stderr=log)
        connection = socket.socket(socket.AF_UNIX)
        path = runtime / "relay-v4/test.sock"
        try:
            deadline = time.monotonic() + 15
            while True:
                try:
                    connection.connect(str(path))
                    break
                except (FileNotFoundError, ConnectionRefusedError):
                    if time.monotonic() > deadline or engine.poll() is not None:
                        raise RuntimeError("Fixture engine did not start")
                    time.sleep(.05)
            launcher.rmdir()
            stream = connection.makefile("rwb", buffering=0)
            def call(op, payload={}):
                stream.write((json.dumps(dict(v=1,id=str(uuid.uuid4()),actor="user",op=op,payload=payload))+"\n").encode())
                result = json.loads(stream.readline())
                assert result["ok"], result
                return result["result"]
            assert call("app.first_run.state")["needed"]
            for name in ("claude", "codex"):
                call("settings.set", {"path":f"providers.{name}.path", "value":str(provider)})
            for page in ("setup", "setup-local", "setup-github", "workspace-project-submit", "setup-github-connect-submit", "project-launch", "device-run", "device-release", "resources", "toggles"):
                if os.environ.get("RELAY_SETUP_CASES") and page not in os.environ["RELAY_SETUP_CASES"].split(","):
                    continue
                if page == "setup-github-connect-submit":
                    auth.unlink()
                output = OUT / f"{page}.png"
                runenv = dict(os.environ, RELAY_NATIVE_SOCKET=str(path), RELAY_INSTANCE="test", RELAY_NATIVE_SCREENSHOT=str(output), RELAY_NATIVE_SIZE="1024,768", RELAY_NATIVE_PAGE=page, RELAY_NATIVE_SETUP_PATH=str(workspace), RELAY_NATIVE_SMOKE_SECONDS="5")
                if os.environ.get("RELAY_SETUP_POINTERS"):
                    runenv.update(GDK_BACKEND="x11", RELAY_NATIVE_POINTER_DRIVER=str(ROOT / "scripts/native-pointer-click.py"))
                with (OUT / f"{page}.log").open("w") as native_log:
                    subprocess.run([str(ROOT / "target/debug/relay-native")], env=runenv, stdout=native_log, stderr=native_log, timeout=20, check=True)
                assert output.is_file()
                contents = (OUT / f"{page}.log").read_text()
                assert "panicked" not in contents and "stylesheet:" not in contents, contents
                assert all("org.a11y.atspi.Registry" in line for line in contents.splitlines() if "CRITICAL" in line), contents
                if page == "toggles":
                    assert "Panel toggles verified" in contents
                if page == "setup":
                    assert not call("workspace.list")["workspaces"]
                elif page == "setup-github-connect-submit":
                    assert auth.exists(), "Connect must complete the GitHub browser flow"
                    assert (workspace / "github-project/.git").exists()
                    assert len(call("project.list")["projects"]) == 2
                    assert not call("app.first_run.state")["needed"]
                elif page == "workspace-project-submit":
                    projects = call("project.list")["projects"]
                    assert len(projects) == 1 and projects[0]["path"] == str(repo), projects
                elif page == "project-launch":
                    sessions = call("session.list", {"project_id":1})["sessions"]
                    assert len(sessions) == 1 and sessions[0]["state"] == "running", sessions
                    output = call("session.scrollback", {"session":sessions[0]["name"]})["text"]
                    assert "LOCAL WORKFLOW FIXTURE READY" in output, output
                    call("session.park", {"session":sessions[0]["name"]})
                print(f"Verified {page}", flush=True)
            assert len(call("workspace.list")["workspaces"]) == 1, "Retry must reuse the workspace"
        finally:
            connection.close()
            engine.terminate()
            engine.wait(timeout=5)
