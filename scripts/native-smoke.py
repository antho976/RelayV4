#!/usr/bin/env python3
"""Real native client + disposable engine + fake providers. No user store or paid CLI."""
import json
import os
from pathlib import Path
import socket
import subprocess
import struct
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / ".impeccable/review"
OUT.mkdir(parents=True, exist_ok=True)
ENGINE = ROOT / "target/debug/relay"
NATIVE = ROOT / "target/debug/relay-native"


def git(repo, *args):
    subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True)


with tempfile.TemporaryDirectory(prefix="relay-native-smoke-") as temporary:
    base = Path(temporary)
    runtime = base / "runtime"
    runtime.mkdir(mode=0o700)
    env = dict(os.environ, XDG_DATA_HOME=str(base / "data"),
               XDG_CONFIG_HOME=str(base / "config"), XDG_CACHE_HOME=str(base / "cache"),
               XDG_RUNTIME_DIR=str(runtime), RELAY_INSTANCE="test", RELAY_BIN=str(ENGINE))
    for key in ("RELAY_SESSION", "RELAY_TOKEN", "RELAY_BRIEF"):
        env.pop(key, None)
    provider = base / "provider"
    provider.write_text('''#!/usr/bin/env python3
import os, sys
if "--version" in sys.argv:
    print("native-smoke 1.0"); sys.exit()
if "auth" in sys.argv or "login" in sys.argv:
    print('{"loggedIn":true}'); sys.exit()
print("\\033[1;37mRELAY NATIVE VERIFICATION\\033[0m", flush=True)
print("Disposable provider fixture, no model connected.", flush=True)
print("Session: " + os.environ.get("RELAY_SESSION", "fixture"), flush=True)
print("\\nEngine owns this PTY. VTE renders its output.", flush=True)
print("  [ok] isolated worktree\\n  [ok] ordered output\\n  [ok] waiting for input", flush=True)
for line in sys.stdin:
    if line.strip()=="native-burst":
        for i in range(2048): print(f"B{i:06d} " + "x"*88)
        print("BURST-END-"+os.environ["RELAY_SESSION"],flush=True)
    else: print("echo: " + line.rstrip(), flush=True)
''')
    provider.chmod(0o755)
    log = (OUT / "engine.log").open("w")
    engine = subprocess.Popen([str(ENGINE), "--instance", "test", "serve"], env=env, stdout=log, stderr=log)
    connection = socket.socket(socket.AF_UNIX)
    path = runtime / "relay-v4/test.sock"
    try:
        deadline = time.monotonic() + 15
        while True:
            try:
                connection.connect(str(path))
                break
            except (FileNotFoundError, ConnectionRefusedError):
                if engine.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Fixture engine failed; see engine.log")
                time.sleep(0.05)
        stream = connection.makefile("rwb", buffering=0)

        def call(op, payload):
            stream.write((json.dumps(dict(v=1, id=str(uuid.uuid4()), actor="user", op=op, payload=payload)) + "\n").encode())
            response = json.loads(stream.readline())
            if not response["ok"]:
                raise RuntimeError(f"{op}: {response['error']}")
            return response["result"]

        workspace = base / "workspace"
        repo = workspace / "Native verification with a long repository name"
        repo.mkdir(parents=True)
        git(repo, "init", "-q", "-b", "main")
        git(repo, "config", "user.name", "Native fixture")
        git(repo, "config", "user.email", "fixture@relay.test")
        (repo / "README.md").write_text("# Native verification\n\nDisposable smoke-test project.\n")
        git(repo, "add", ".")
        git(repo, "commit", "-qm", "fixture")
        ws = call("workspace.create", {"path": str(workspace)})
        project = call("project.add", {"workspace_id": ws["id"], "path": str(repo)})
        for name in ("claude", "codex"):
            call("settings.set", {"path": f"providers.{name}.path", "value": str(provider)})
        for title in ("Native terminal lifecycle", "Mailbox and review contracts", "Code workspace"):
            task = call("task.create", {"project_id": project["id"], "title": title, "body": "Verification fixture.", "column": "ready"})
            builder = call("session.create", {"project_id": project["id"], "provider": "claude", "role": "builder"})
            reviewer = call("session.create", {"project_id": project["id"], "provider": "codex", "role": "reviewer", "pair_with": builder["name"]})
            call("task.dispatch", {"task_id": task["id"], "session": builder["name"], "start": False})
            for session in (builder, reviewer):
                call("session.spawn", {"session": session["name"]})
        call("mailbox.send", {"project_id": project["id"], "to": reviewer["name"],
                              "text": "Terminal changes are ready for review. Check input ordering and teardown.", "priority": True})
        call("notes.create", {"project_id": project["id"], "title": "Native rebuild",
                              "body": "GTK4, VTE and GtkSourceView. Keep approval authority separate from review."})
        call("guardrail.config.set", {"project_id": project["id"], "patch": {"protected_paths": ["secret/*"]}})
        try:
            call("guardrail.gate", {"session": builder["name"], "kind": "write", "path": "secret/key", "new_text": "fixture"})
            raise AssertionError("The fixture action should be held")
        except RuntimeError as error:
            assert "held" in str(error), error
        assert len(call("guardrail.holds.list", {"project_id": project["id"]})["holds"]) == 1
        desktop_env = dict(os.environ, RELAY_NATIVE_SOCKET=str(path), RELAY_INSTANCE="test")
        for key in ("RELAY_SESSION", "RELAY_TOKEN", "RELAY_BRIEF"):
            desktop_env.pop(key, None)
        initial_names={s["name"] for s in call("session.list",{"project_id":project["id"]})["sessions"]}
        if os.environ.get("RELAY_SMOKE_ROADMAP_ONLY"):
            for part in os.environ["RELAY_SMOKE_ROADMAP_ONLY"].split(","):
                with (OUT / f"roadmap-{part}.log").open("w") as native_log:
                    native = subprocess.Popen([str(NATIVE)], env=dict(desktop_env,
                        RELAY_NATIVE_SCREENSHOT=str(OUT / f"roadmap-{part}.png"),
                        RELAY_NATIVE_FIXTURE="1", RELAY_NATIVE_ROADMAP=part),
                        stdout=native_log, stderr=native_log)
                    try:
                        assert native.wait(timeout=90) == 0, part
                    finally:
                        if native.poll() is None:
                            native.terminate()
                            native.wait(timeout=5)
                assert f"ROADMAP_OK={part}" in (OUT / f"roadmap-{part}.log").read_text(), part
            print("Roadmap native regressions passed: " + os.environ["RELAY_SMOKE_ROADMAP_ONLY"])
            raise SystemExit(0)
        measurements=[]
        captured=[]
        for viewport, size, page in (("desktop", "1440,900", "agents"), ("compact", "1024,768", "agents"), ("launch-preview", "1024,768", "launch-preview"), ("palette", "1024,768", "palette"), ("layouts", "1024,768", "layouts"), ("board", "1440,900", "board"), ("board-compact", "1024,768", "board"), ("mailbox", "1024,768", "mailbox"), ("guardrails", "1024,768", "guardrails"), ("code", "1440,900", "code"), *((name,"1440,900",name) for name in ("notes","modules","settings","skills","dashboard","notifications","devices","launch"))):
            output = OUT / f"{viewport}.png"
            output.unlink(missing_ok=True)
            native_env = dict(desktop_env, RELAY_NATIVE_SCREENSHOT=str(output), RELAY_NATIVE_SIZE=size, RELAY_NATIVE_FIXTURE="1",
                              RELAY_NATIVE_PAGE=page, RELAY_NATIVE_SMOKE_SECONDS="8" if viewport in ("desktop","launch") else "5")
            with (OUT / f"{viewport}.log").open("w") as native_log:
                native=subprocess.Popen([str(NATIVE)], env=native_env, stdout=native_log, stderr=native_log)
                if viewport=="desktop":
                    def sample():
                        stat=Path(f"/proc/{native.pid}/stat").read_text().split(") ",1)[1].split()
                        return int(stat[11])+int(stat[12]), int(stat[21])*os.sysconf("SC_PAGE_SIZE")
                    time.sleep(3)
                    ticks, _=sample(); started=time.monotonic()
                    time.sleep(3)
                    end_ticks,rss=sample();elapsed=time.monotonic()-started
                    measurements.append(dict(cpu_percent=round((end_ticks-ticks)/os.sysconf("SC_CLK_TCK")/elapsed*100,2),
                                             rss_mib=round(rss/1024/1024,1), sample_seconds=round(elapsed,2)))
                try:
                    assert native.wait(timeout=30)==0
                finally:
                    if native.poll() is None:
                        native.terminate()
                        native.wait(timeout=5)
            assert output.is_file(), f"No screenshot at {output}"
            captured.append(output.name)
            expected_size = (1080, 760) if page == "notes" else tuple(map(int, size.split(",")))
            assert struct.unpack(">II", output.read_bytes()[16:24]) == expected_size, viewport
            contents = (OUT / f"{viewport}.log").read_text()
            assert "Shell layouts verified" in contents, contents
            saved_layout = call("settings.get", {"path": f"native.layout.current.{project['id']}"})["value"]
            assert saved_layout["agent_layout"] == "grid", saved_layout
            assert "stylesheet:" not in contents and "gtk_widget_add_css_class:" not in contents, contents
        (OUT/"measurements.json").write_text(json.dumps(measurements,indent=2)+"\n")
        live = call("session.list", {"project_id": project["id"]})["sessions"]
        assert len(live) == 9 and all(s["state"] == "running" for s in live), live
        builders = [s for s in live if s["role"]=="builder"]
        reviewers = [s for s in live if s["role"]=="reviewer"]
        assert len(builders)==5 and len(reviewers)==4, live
        launched=[s for s in live if s["name"] not in initial_names]
        assert len(launched)==3 and len({s["worktree"] for s in launched})==1, launched
        assert any(s["task_id"] is not None for s in launched), launched
        assert any(t["title"]=="Native task edit verified" for t in call("task.list",{"project_id":project["id"]})["tasks"])
        assert any(n["body"]=="Native note save verified." for n in call("notes.list",{"project_id":project["id"]})["notes"])
        for _ in range(2):
            extra=call("session.create",{"project_id":project["id"],"provider":"claude","role":"builder"})
            call("session.spawn",{"session":extra["name"]})
        burst_env=dict(desktop_env,RELAY_NATIVE_SCREENSHOT=str(OUT/"burst.png"),RELAY_NATIVE_SIZE="1440,900",RELAY_NATIVE_FIXTURE="1",RELAY_NATIVE_PAGE="agents",RELAY_NATIVE_BURST="1",RELAY_NATIVE_SMOKE_SECONDS="7")
        with (OUT/"burst.log").open("w") as burst_log:
            burst=subprocess.Popen([str(NATIVE)],env=burst_env,stdout=burst_log,stderr=burst_log)
            try: assert burst.wait(timeout=20)==0
            finally:
                if burst.poll() is None: burst.terminate();burst.wait(timeout=5)
        assert "BURST_RENDERED=11" in (OUT/"burst.log").read_text()
        live=call("session.list",{"project_id":project["id"]})["sessions"]
        for session in live:
            output=call("session.scrollback", {"session": session["name"]})["text"]
            if session["name"] in initial_names:
                assert f"echo: native-paste-check-{session['name']}" in output, output
            call("session.input", {"session": session["name"], "data": "survived-window-close\n"})
            call("session.park", {"session": session["name"]})
        saved=call("file.read", {"project_id":project["id"],"path":"README.md"})["text"]
        assert "Native editor save verified." in saved, saved
        print(json.dumps({"screenshots": captured + ["burst.png"], "sessions_survived_window_close": len(live), "native_paste_echoes_verified":6,"native_editor_saved":True,"native_task_saved":True,"native_note_saved":True,"native_review_group_launched":True,"burst_native_tail_markers":11,"burst_lines_per_session":2048,"burst_completion_budget_seconds":5,"real_provider_calls": 0}))
    finally:
        connection.close()
        engine.terminate()
        try:
            engine.wait(timeout=10)
        except subprocess.TimeoutExpired:
            engine.kill()
            engine.wait()
        log.close()
