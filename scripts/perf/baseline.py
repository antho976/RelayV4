#!/usr/bin/env python3
"""The performance baseline: every bus op, every engine path, idle cost, memory, CLI startup.

Drives `crates/relay-core/examples/perf.rs` four ways (native timing, callgrind attribution,
strace syscall counts, a memory soak), then measures the real `relay serve` process from the
outside (idle CPU and wakeups, output streaming, restart time) and the CLI's process startup.
Everything lands in one run directory as JSON, and `report.py` turns it into Markdown.

    python3 scripts/perf/baseline.py                  # full run, docs/perf/runs/<date>/
    python3 scripts/perf/baseline.py --quick          # fewer iterations, no callgrind
    python3 scripts/perf/baseline.py --only 'op.task.*'   # a subset: a name, or a prefix ending in *

No user store, no paid provider, no display. The fixture is disposable and lives in a temp dir.
"""
import argparse
import datetime
import json
import os
import platform
import re
import select
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

# The waits here end on assert statements; under -O a stalled wait would never end.
if sys.flags.optimize:
    raise SystemExit("Run without python -O or PYTHONOPTIMIZE: this script times out with assert.")

ROOT = Path(__file__).resolve().parents[2]
PERF = ROOT / "target/debug/examples/perf"
RELAY = ROOT / "target/debug/relay"
CLK_TCK = os.sysconf("SC_CLK_TCK")
PAGE = os.sysconf("SC_PAGE_SIZE")

# The list-shaped queries the shell re-runs on every state change: measured again with ten
# times the rows, so growth that is worse than linear shows up as a ratio, not a surprise.
SCALE_OPS = [
    "op.task.list", "op.task.list.filtered", "op.session.list", "op.notes.list", "op.mailbox.list",
    "op.mailbox.outbox", "op.audit.list", "op.dashboard.get", "op.module.stats", "op.module.list",
    "op.task.label.list", "op.app.status", "op.notify.list", "op.overlap.list", "op.guardrail.holds.list",
    "op.task.activity", "op.session.brief", "op.session.bootstrap", "op.task.children",
    "op.project.stats", "op.module.changelog.draft", "op.task.get", "op.settings.get",
    "path.socket.session_list", "path.socket.task_list", "path.response.serialize.session_list",
]


def log(msg):
    print(f"[baseline] {msg}", file=sys.stderr, flush=True)


def sh(cmd, **kw):
    kw.setdefault("check", True)
    kw.setdefault("text", True)
    kw.setdefault("capture_output", True)
    return subprocess.run(cmd, **kw)


def build():
    log("building relay-core example perf and relay-cli (dev profile, the one run.sh uses)")
    subprocess.run(["cargo", "build", "-p", "relay-core", "--example", "perf"], cwd=ROOT, check=True)
    subprocess.run(["cargo", "build", "-p", "relay-cli"], cwd=ROOT, check=True)


def environment():
    def out(cmd):
        try:
            return sh(cmd).stdout.strip()
        except Exception:
            return None
    cpu = None
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    mem_kb = None
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal"):
                mem_kb = int(line.split()[1])
    except OSError:
        pass
    return {
        "date": datetime.date.today().isoformat(),
        "git_sha": out(["git", "-C", str(ROOT), "rev-parse", "HEAD"]),
        "git_branch": out(["git", "-C", str(ROOT), "rev-parse", "--abbrev-ref", "HEAD"]),
        "rustc": out(["rustc", "--version"]),
        "profile": "dev (opt-level 1 workspace, 3 dependencies; the profile run.sh launches)",
        "kernel": platform.release(),
        "cpu": cpu,
        "cores": os.cpu_count(),
        "mem_gib": round(mem_kb / 1024 / 1024, 1) if mem_kb else None,
        "valgrind": out(["valgrind", "--version"]),
        "strace": out(["strace", "-V"]).splitlines()[0] if shutil.which("strace") else None,
        "load_at_start": os.getloadavg(),
        "container": Path("/.dockerenv").exists() or "container" in (Path("/proc/1/cgroup").read_text() if Path("/proc/1/cgroup").exists() else ""),
    }


def strace_blocked():
    """Why `perf run --strace` cannot attach here, or None. perf.rs starts strace as its own child
    and points it at its parent. Under Yama ptrace_scope 1 it lets any process attach first
    (PR_SET_PTRACER_ANY); under 2 that needs CAP_SYS_PTRACE, and under 3 nothing may attach.
    Running the pass anyway costs nothing wrong: perf.rs writes `"strace": null` for every
    scenario it could not trace, but skipping it says why up front."""
    try:
        scope = int(Path("/proc/sys/kernel/yama/ptrace_scope").read_text())
    except (OSError, ValueError):
        return None  # no Yama: ordinary ptrace rules, a process may trace its parent
    cap_sys_ptrace = False
    try:
        for line in Path("/proc/self/status").read_text().splitlines():
            if line.startswith("CapEff:"):
                cap_sys_ptrace = bool(int(line.split()[1], 16) >> 19 & 1)
    except (OSError, ValueError):
        pass
    if scope in (0, 1) or (scope == 2 and cap_sys_ptrace):
        return None
    return (f"kernel.yama.ptrace_scope is {scope}"
            + ("" if scope == 3 else " and this process lacks CAP_SYS_PTRACE")
            + ": strace cannot attach to the perf process (try `sudo sysctl kernel.yama.ptrace_scope=0` for the run)")


def scenarios():
    return sh([str(PERF), "list"]).stdout.split()


def matches(name, filters):
    """`perf run`'s rule: a filter is a scenario name, or a prefix when it ends in `*`."""
    return not filters or any(name == f or (f.endswith("*") and name.startswith(f.rstrip("*"))) for f in filters)


def perf_run(args, out, extra_env=None):
    env = dict(os.environ, **(extra_env or {}))
    with open(out.with_suffix(".log"), "w") as logf:
        subprocess.run([str(PERF), "run", *args, "--out", str(out)], check=True, stderr=logf, env=env)


def read_jsonl(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


# ------------------------------------------------------------------ callgrind

BUCKETS = [
    ("harness", lambda s: "perf::" in s and "Counting" not in s),
    ("allocation", lambda s: "Counting as core::alloc" in s or "GlobalAlloc" in s or re.search(r"\b(_int_malloc|_int_free|malloc|free|realloc|calloc|__libc_malloc|__libc_free|malloc_consolidate|unlink_chunk|tcache)", s) is not None or "alloc::alloc::" in s),
    ("sqlite", lambda s: "sqlite3" in s or "sqlite3.c" in s),
    ("rusqlite", lambda s: "rusqlite" in s),
    ("json", lambda s: "serde_json" in s),
    ("serde (typed)", lambda s: "serde::" in s or "serde_derive" in s),
    ("handlers", lambda s: "relay_core::handlers" in s),
    ("pipeline", lambda s: "relay_core::engine" in s),
    ("audit", lambda s: "relay_core::audit" in s),
    ("guardrail", lambda s: "relay_core::guardrail" in s),
    ("pty/ring", lambda s: "relay_core::pty" in s),
    ("socket", lambda s: "relay_core::socket" in s),
    ("worktree/git glue", lambda s: "relay_core::worktree" in s),
    ("providers", lambda s: "relay_core::providers" in s or "relay_core::hooks" in s or "relay_core::skills" in s),
    ("store", lambda s: "relay_core::store" in s),
    ("relay_core (other)", lambda s: "relay_core" in s),
    ("bus types/registry", lambda s: "relay_bus" in s),
    ("gix (git)", lambda s: "gix" in s),
    ("tokio/mio", lambda s: "tokio" in s or "mio::" in s),
    ("time (jiff)", lambda s: "jiff" in s),
    ("hashing (sha2)", lambda s: "sha2" in s or "digest" in s),
    ("ids (uuid/rand)", lambda s: "uuid" in s or "rand" in s or "getrandom" in s),
    ("regex", lambda s: "regex" in s or "aho_corasick" in s or "memchr" in s),
    ("tree-sitter", lambda s: "tree_sitter" in s or "ts_" in s and "tree-sitter" in s),
    ("diff (similar)", lambda s: "similar" in s),
    ("base64", lambda s: "base64" in s),
    ("schema (schemars)", lambda s: "schemars" in s),
    ("hashbrown/hash", lambda s: "hashbrown" in s or "foldhash" in s or "SipHash" in s or "Hasher" in s),
    ("memcpy/str ops", lambda s: re.search(r"(memcpy|memmove|memset|memcmp|strlen|strcmp|strchr|memchr|bcmp|__mem|__str)", s) is not None),
    ("std/core", lambda s: "core::" in s or "std::" in s or "alloc::" in s),
    ("loader", lambda s: "ld-linux" in s or "_dl_" in s),
    ("libc (other)", lambda s: "libc.so" in s),
    ("kernel/vdso", lambda s: "vdso" in s),
    ("unknown", lambda s: s.startswith("???")),
]


# Inclusive cost of a few well-known entry points, as a share of the scenario. These overlap
# (a statement step sits inside the handler) and are read as "how much of this op is X", not
# as parts of a whole.
MARKERS = {
    "sql compile (sqlite3_prepare)": ["sqlite3_prepare_v2", "sqlite3_prepare_v3", "sqlite3Prepare", "sqlite3RunParser"],
    "sql execute (sqlite3_step)": ["sqlite3_step"],
    "sql commit + WAL write": ["sqlite3VdbeHalt", "sqlite3BtreeCommitPhaseOne", "sqlite3WalFrames", "sqlite3PagerCommitPhaseOne"],
    "disk write (write syscalls)": ["unixWrite", "__GI___libc_write", "std::io::Write::write_all", "std::sys::pal::unix::fd::FileDesc::write"],
    "fsync": ["unixSync", "fdatasync", "fsync"],
    "json to_value/to_string": ["serde_json::value::to_value", "serde_json::ser::to_string", "serde_json::value::ser"],
    "json from_str/from_value": ["serde_json::de::from_str", "serde_json::value::de::from_value", "serde_json::value::from_value"],
    "audit row": ["relay_core::audit::"],
    "guardrail": ["relay_core::guardrail::"],
    "actor resolve": ["Engine::resolve_actor"],
    "mail hint": ["Engine::mail_hint"],
    "event emit": ["Engine::emit", "broadcast::Sender"],
    "git (gix)": ["gix::", "gix_"],
    "subprocess": ["relay_core::proc::output_with_timeout", "std::process::Command::spawn", "std::process::Command::output"],
    "sha256": ["sha2::", "Sha256"],
    "timestamp (jiff)": ["jiff::"],
    "uuid": ["uuid::"],
    "schema (schemars)": ["schemars::"],
    "regex": ["regex::", "regex_automata"],
    "tree-sitter": ["tree_sitter"],
    "fs walk (read_dir)": ["std::fs::read_dir", "ReadDir", "ignore::", "walkdir"],
}


def bucket_of(symbol):
    for name, test in BUCKETS:
        if test(symbol):
            return name
    return "other"


def annotate(path, inclusive):
    out = sh(["callgrind_annotate", f"--inclusive={'yes' if inclusive else 'no'}", "--threshold=100", "--auto=no", "--context=0", str(path)]).stdout
    rows = []
    for line in out.splitlines():
        m = re.match(r"^\s*([\d,]+)\s+\(\s*([\d.]+)%\)\s+(.*)$", line)
        if not m or "PROGRAM TOTALS" in line:
            continue
        rows.append((int(m.group(1).replace(",", "")), m.group(3).strip()))
    return rows


def parse_callgrind(out_dir, rows_by_name):
    profiles = {}
    for path in sorted(Path(out_dir, "callgrind").glob("callgrind.out.*")):
        head = path.read_text(errors="replace")[:4000]
        m = re.search(r"^desc: Trigger: dump (.+)$", head, re.M)
        if not m:
            continue
        name = m.group(1).strip()
        iters = rows_by_name.get(name, {}).get("iters", 1) or 1
        self_rows = annotate(path, inclusive=False)
        total = sum(ir for ir, _ in self_rows)
        if total == 0:
            continue
        buckets = {}
        for ir, sym in self_rows:
            buckets[bucket_of(sym)] = buckets.get(bucket_of(sym), 0) + ir
        incl = annotate(path, inclusive=True)
        def short(sym):
            sym = sym.split(":", 1)[1] if ":" in sym and not sym.startswith("???") else sym
            sym = re.sub(r"\s*\[.*\]$", "", sym)
            sym = re.sub(r"::h[0-9a-f]{16}$", "", sym)
            return sym[:120]
        # The pipeline frames wrap every op identically; what differs is below them.
        generic = ("Engine::dispatch", "Engine::register", "perf_measured", "perf::", "Engine::system_write")
        relay, seen = [], set()
        for ir, s in incl:
            if not ("relay_core" in s or "relay_bus" in s) or any(g in s for g in generic):
                continue
            fn = short(s)
            if fn in seen:
                continue
            seen.add(fn)
            relay.append((ir, fn))
        markers = {}
        for label, needles in MARKERS.items():
            best = 0
            for ir, s in incl:
                if any(n in s for n in needles):
                    best = max(best, ir)
            if best:
                markers[label] = round(min(best * 100.0 / total, 100.0), 1)
        profiles[name] = {
            "ir_per_iter": total // iters,
            "iters": iters,
            "buckets": {k: round(v * 100.0 / total, 1) for k, v in sorted(buckets.items(), key=lambda kv: -kv[1]) if v * 100.0 / total >= 0.5},
            "top_self": [{"pct": round(ir * 100.0 / total, 1), "fn": short(s)} for ir, s in self_rows[:12]],
            "top_inclusive_relay": [{"pct": round(min(ir * 100.0 / total, 100.0), 1), "fn": fn} for ir, fn in relay[:10]],
            "markers": markers,
        }
    return profiles


def shards(names, jobs):
    """Split `names` into `jobs` groups. `perf run` matches a name without `*` exactly, so each
    scenario runs (and dumps) in its own group only."""
    return [names[i::jobs] for i in range(jobs)]


def run_callgrind(out_dir, names, jobs, iters_div):
    cg_dir = out_dir / "callgrind"
    cg_dir.mkdir(exist_ok=True)
    groups = shards(names, jobs)

    def one(i, group):
        if not group:
            return
        out = cg_dir / f"rows-{i}.jsonl"
        cmd = ["valgrind", "--tool=callgrind", "--instr-atstart=no", "--dump-instr=no", "--compress-strings=no",
               f"--callgrind-out-file={cg_dir}/callgrind.out.%p", str(PERF), "run", *group, "--callgrind",
               "--iters-div", str(iters_div), "--warmup", "1", "--out", str(out)]
        with open(cg_dir / f"valgrind-{i}.log", "w") as logf:
            subprocess.run(cmd, check=False, stdout=logf, stderr=logf)

    with ThreadPoolExecutor(max_workers=jobs) as pool:
        list(pool.map(lambda ig: one(*ig), enumerate(groups)))
    rows = []
    for i in range(jobs):
        rows += read_jsonl(cg_dir / f"rows-{i}.jsonl")
    # The termination dumps carry nothing: remove them so a rerun does not re-parse them.
    for path in cg_dir.glob("callgrind.out.*"):
        if "Trigger: Program termination" in path.read_text(errors="replace")[:4000]:
            path.unlink()
    return rows


# ------------------------------------------------------------------ the real process

PROVIDER_PY = r'''#!/usr/bin/env python3
import os, sys, time
if "--version" in sys.argv:
    print("perf-fixture 1.0"); sys.exit()
if "auth" in sys.argv or "login" in sys.argv:
    print('{"loggedIn":true}'); sys.exit()
print("hello-from-pty " + os.environ.get("RELAY_SESSION", "?"), flush=True)
for line in sys.stdin:
    line = line.rstrip("\n")
    if line.startswith("tick "):
        n, ms = line.split()[1:3]
        for i in range(int(n)):
            print(f"tick {i} " + "x" * 60, flush=True)
            time.sleep(int(ms) / 1000)
    elif line.startswith("burst "):
        for i in range(int(line.split()[1])):
            print(f"B{i:06d} " + "x" * 90)
        print("BURST-END", flush=True)
    else:
        print("echo: " + line, flush=True)
'''


class Engine:
    """A disposable `relay serve` with its own XDG dirs, one socket client, N fake sessions."""

    def __init__(self, base: Path):
        self.base = base
        runtime = base / "runtime"
        runtime.mkdir(mode=0o700, exist_ok=True)
        self.env = dict(os.environ, XDG_DATA_HOME=str(base / "data"), XDG_CONFIG_HOME=str(base / "config"),
                        XDG_CACHE_HOME=str(base / "cache"), XDG_RUNTIME_DIR=str(runtime), RELAY_INSTANCE="test",
                        RELAY_BIN=str(RELAY), RELAY_LOG="warn")
        for key in ("RELAY_SESSION", "RELAY_TOKEN", "RELAY_BRIEF"):
            self.env.pop(key, None)
        self.socket_path = runtime / "relay-v4/test.sock"
        self.log = open(base / "engine.log", "a")
        self.proc = None
        self.conn = None
        self.stream = None

    def start(self):
        t0 = time.monotonic()
        self.proc = subprocess.Popen([str(RELAY), "--instance", "test", "serve"], env=self.env, stdout=self.log, stderr=self.log)
        deadline = t0 + 30
        while True:
            try:
                self.conn = socket.socket(socket.AF_UNIX)
                self.conn.connect(str(self.socket_path))
                break
            except (FileNotFoundError, ConnectionRefusedError):
                self.conn.close()
                if self.proc.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("engine did not come up; see engine.log")
                time.sleep(0.005)
        # Buffered: an unbuffered readline is one recv per byte, which the timings would include.
        self.stream = self.conn.makefile("rwb")
        # Ready means answering, not just accepting.
        self.call("bus.ping", {})
        return time.monotonic() - t0

    def call(self, op, payload, actor="user"):
        self.stream.write((json.dumps(dict(v=1, id=str(uuid.uuid4()), actor=actor, op=op, payload=payload)) + "\n").encode())
        self.stream.flush()
        while True:
            line = self.stream.readline()
            if not line:
                raise RuntimeError("engine closed the connection")
            msg = json.loads(line)
            if "ev" in msg or "stream" in msg:
                continue
            if not msg["ok"]:
                raise RuntimeError(f"{op}: {msg['error']}")
            return msg["result"]

    def stop(self):
        if self.conn:
            self.conn.close()
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()

    def fixture(self, sessions):
        repo = self.base / "workspace" / "app"
        repo.mkdir(parents=True, exist_ok=True)
        # Only the repository's own config: a global commit.gpgsign or hooksPath would otherwise
        # sign (or prompt for) the fixture commit and run the developer's hooks on it.
        git_env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        def git(*args):
            subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, env=git_env)
        git("init", "-q", "-b", "main")
        git("config", "user.name", "perf")
        git("config", "user.email", "perf@relay.test")
        (repo / "README.md").write_text("# perf\n")
        git("add", ".")
        git("commit", "-qm", "fixture")
        provider = self.base / "provider.py"
        provider.write_text(PROVIDER_PY)
        provider.chmod(0o755)
        ws = self.call("workspace.create", {"path": str(self.base / "workspace")})
        project = self.call("project.add", {"workspace_id": ws["id"], "path": str(repo)})
        for name in ("claude", "codex"):
            self.call("settings.set", {"path": f"providers.{name}.path", "value": str(provider)})
        names = []
        for i in range(sessions):
            s = self.call("session.create", {"project_id": project["id"], "provider": "claude" if i % 2 == 0 else "codex", "role": "builder"})
            self.call("session.spawn", {"session": s["name"]})
            names.append(s["name"])
        for name in names:
            deadline = time.monotonic() + 30
            while "hello-from-pty" not in self.call("session.scrollback", {"session": name, "lines": 5})["text"]:
                assert time.monotonic() < deadline, name
                time.sleep(0.01)
        return names


def proc_counters(pid):
    stat = Path(f"/proc/{pid}/stat").read_text().split(") ", 1)[1].split()
    cpu_ticks = int(stat[11]) + int(stat[12])
    rss = int(stat[21]) * PAGE
    switches = {}
    for task in Path(f"/proc/{pid}/task").iterdir():
        vol = invol = 0
        try:
            for line in (task / "status").read_text().splitlines():
                if line.startswith("voluntary_ctxt_switches"):
                    vol = int(line.split()[1])
                elif line.startswith("nonvoluntary_ctxt_switches"):
                    invol = int(line.split()[1])
        except OSError:
            continue
        switches[task.name] = (vol, invol)
    return {"cpu_ticks": cpu_ticks, "rss": rss, "switches": switches, "threads": len(switches)}


def switch_delta(a, b):
    """Context switches between two samples, per thread alive in both, so a thread that exited
    in between does not subtract its lifetime from the window."""
    vol = invol = 0
    for tid, (v1, i1) in b["switches"].items():
        if tid in a["switches"]:
            v0, i0 = a["switches"][tid]
            vol += v1 - v0
            invol += i1 - i0
        else:
            vol += v1
            invol += i1
    return vol, invol


def sample(pid, seconds):
    a = proc_counters(pid)
    t0 = time.monotonic()
    time.sleep(seconds)
    b = proc_counters(pid)
    elapsed = time.monotonic() - t0
    vol, invol = switch_delta(a, b)
    return {
        "seconds": round(elapsed, 2),
        "cpu_percent": round((b["cpu_ticks"] - a["cpu_ticks"]) / CLK_TCK / elapsed * 100, 3),
        "cpu_ms_total": round((b["cpu_ticks"] - a["cpu_ticks"]) / CLK_TCK * 1000, 1),
        "wakeups_per_s": round(vol / elapsed, 1),
        "preemptions_per_s": round(invol / elapsed, 1),
        "rss_mb": round(b["rss"] / 1024 / 1024, 1),
        "threads": b["threads"],
    }


def run_process(out_dir, seconds):
    """The engine as the desktop sees it: a process that sits there between requests."""
    result = {"window_seconds": seconds, "phases": {}}
    with tempfile.TemporaryDirectory(prefix="relay-perf-process-") as tmp:
        base = Path(tmp)
        engine = Engine(base)
        try:
            result["startup_s_fresh_store"] = round(engine.start(), 3)
            result["rss_mb_fresh"] = round(proc_counters(engine.proc.pid)["rss"] / 1024 / 1024, 1)
            names = engine.fixture(6)
            result["rss_mb_six_sessions"] = round(proc_counters(engine.proc.pid)["rss"] / 1024 / 1024, 1)
            pid = engine.proc.pid
            log(f"process: idle, six quiet sessions ({seconds}s)")
            time.sleep(2)
            result["phases"]["idle_six_quiet_sessions"] = sample(pid, seconds)

            log(f"process: idle with the resources panel watching ({seconds}s)")
            engine.call("app.resources.watch", {"on": True})
            time.sleep(1)
            result["phases"]["idle_resources_watch_on"] = sample(pid, seconds)
            engine.call("app.resources.watch", {"on": False})

            log(f"process: six sessions each printing 10 lines/s, nobody attached ({seconds}s)")
            for name in names:
                engine.call("session.input", {"session": name, "data": f"tick {seconds * 10 + 20} 100\n"})
            time.sleep(1)
            result["phases"]["output_60_lines_per_s_unattached"] = sample(pid, seconds)
            time.sleep(3)

            log(f"process: same output with a client attached to every session ({seconds}s)")
            sub = socket.socket(socket.AF_UNIX)
            sub.connect(str(engine.socket_path))
            stream = sub.makefile("rwb", buffering=0)
            for name in names:
                stream.write((json.dumps(dict(v=1, id=str(uuid.uuid4()), actor="user", op="session.attach", payload={"session": name})) + "\n").encode())
            received = {"bytes": 0, "frames": 0}
            stop = threading.Event()

            def reader():
                while not stop.is_set():
                    line = stream.readline()
                    if not line:
                        break
                    received["bytes"] += len(line)
                    if b'"stream"' in line:
                        received["frames"] += 1
            t = threading.Thread(target=reader, daemon=True)
            t.start()
            for name in names:
                engine.call("session.input", {"session": name, "data": f"tick {seconds * 10 + 20} 100\n"})
            time.sleep(1)
            result["phases"]["output_60_lines_per_s_attached"] = sample(pid, seconds)
            result["phases"]["output_60_lines_per_s_attached"]["client_frames"] = received["frames"]
            result["phases"]["output_60_lines_per_s_attached"]["client_bytes"] = received["bytes"]
            stop.set()
            sub.close()
            time.sleep(3)

            log("process: one session bursting 50 000 lines while attached")
            sub = socket.socket(socket.AF_UNIX)
            sub.connect(str(engine.socket_path))
            # Read in large chunks and split the lines here. A raw makefile's readline is one
            # recv per byte, slow enough to be what this phase measured, and a socket timeout
            # under a makefile leaves it refusing every read after the first expiry.
            lines, partial = [], [b""]

            def next_line(timeout):
                """The next complete line, or None after `timeout` seconds of silence."""
                while not lines:
                    if not select.select([sub], [], [], timeout)[0]:
                        return None
                    chunk = sub.recv(1 << 16)
                    if not chunk:
                        raise RuntimeError("engine closed the attached stream")
                    *complete, partial[0] = (partial[0] + chunk).split(b"\n")
                    lines.extend(line + b"\n" for line in reversed(complete))
                return lines.pop()

            sub.sendall((json.dumps(dict(v=1, id=str(uuid.uuid4()), actor="user", op="session.attach", payload={"session": names[0]})) + "\n").encode())
            if next_line(10) is None:
                raise RuntimeError("session.attach did not answer")
            before = proc_counters(pid)
            t0 = time.monotonic()
            engine.call("session.input", {"session": names[0], "data": "burst 50000\n"})
            wire = payload = frames = dropped = 0
            last_seq = None
            finished_at = None
            while True:
                line = next_line(1.0)
                if line:
                    wire += len(line)
                    if b'"stream"' in line:
                        frames += 1
                        try:
                            frame = json.loads(line)
                        except ValueError:
                            continue
                        data = frame.get("data")
                        if isinstance(data, str):
                            payload += len(data) * 3 // 4
                        # The engine drops frames for a reader that falls behind; seq says how many.
                        seq = frame.get("seq")
                        if isinstance(seq, int):
                            if last_seq is not None and seq > last_seq + 1:
                                dropped += seq - last_seq - 1
                            last_seq = seq
                    continue
                # A second of silence on the stream: the burst is over once the bus says so.
                if "BURST-END" in engine.call("session.scrollback", {"session": names[0], "lines": 2})["text"]:
                    finished_at = finished_at or time.monotonic()
                    break
                assert time.monotonic() < t0 + 120, "burst did not finish"
            after = proc_counters(pid)
            elapsed = (finished_at or time.monotonic()) - t0
            result["phases"]["burst_50k_lines_attached"] = {
                "seconds_until_stream_quiet": round(elapsed, 3),
                "engine_cpu_ms": round((after["cpu_ticks"] - before["cpu_ticks"]) / CLK_TCK * 1000, 1),
                "payload_bytes": 50000 * 98,
                "payload_bytes_received": payload,
                "wire_bytes_received": wire,
                "frames": frames,
                "frames_dropped": dropped,
                "mib_per_s_to_client": round(wire / 1024 / 1024 / max(elapsed, 0.001), 1),
            }
            sub.close()

            log("process: restart after a kill with six live sessions (crash recovery path), then clean")
            # SIGKILL, not SIGTERM: a terminated engine closes its own PTYs, a killed one leaves
            # them for recovery to find and reap, which is the path an app crash takes.
            engine.conn.close()
            engine.proc.kill()
            engine.proc.wait()
            restarts = []
            for i in range(3):
                engine = Engine(base)
                restarts.append(round(engine.start(), 3))
                if i == 0:
                    result["recovery_report"] = engine.call("app.recovery.last", {})
                engine.stop()
            result["restart_s"] = {"with_six_live_sessions": restarts[0], "clean": restarts[1:]}

            log("process: CLI startup")
            engine = Engine(base)
            engine.start()
            cli = {}
            for label, args in (("ping", ["ping"]), ("q_session_list", ["q", "session.list", "{}"]), ("q_app_status", ["q", "app.status", "{}"])):
                times = []
                for _ in range(20):
                    t0 = time.monotonic()
                    subprocess.run([str(RELAY), "--instance", "test", *args], env=engine.env, check=False, capture_output=True)
                    times.append((time.monotonic() - t0) * 1000)
                times.sort()
                cli[label] = {"min_ms": round(times[0], 2), "p50_ms": round(times[len(times) // 2], 2), "max_ms": round(times[-1], 2)}
            result["cli"] = cli
            result["engine_binary_bytes"] = RELAY.stat().st_size
        finally:
            engine.stop()
    (out_dir / "process.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


# ------------------------------------------------------------------ main

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out-dir", type=Path, default=None, help="default docs/perf/runs/<date>")
    ap.add_argument("--only", action="append", default=[], help="scenario name, or a prefix ending in * (repeatable)")
    ap.add_argument("--quick", action="store_true", help="a quarter of the iterations, no callgrind, short process windows")
    ap.add_argument("--skip-callgrind", action="store_true")
    ap.add_argument("--skip-strace", action="store_true")
    ap.add_argument("--skip-soak", action="store_true")
    ap.add_argument("--skip-process", action="store_true")
    ap.add_argument("--skip-scale", action="store_true")
    ap.add_argument("--skip-build", action="store_true")
    ap.add_argument("--jobs", type=int, default=max(1, min(4, (os.cpu_count() or 2) - 1)), help="parallel callgrind processes")
    ap.add_argument("--callgrind-iters-div", type=int, default=10)
    ap.add_argument("--process-seconds", type=int, default=30)
    args = ap.parse_args()

    out_dir = args.out_dir or ROOT / "docs/perf/runs" / datetime.date.today().isoformat()
    out_dir.mkdir(parents=True, exist_ok=True)
    if not args.skip_build:
        build()
    env = environment()
    env["quick"] = args.quick  # compare.py warns when two runs differ in this
    strace_skip = None
    if not args.skip_strace and shutil.which("strace"):
        strace_skip = strace_blocked()
        if strace_skip:
            env["strace_skipped"] = strace_skip  # report.py says why the column is empty
    (out_dir / "environment.json").write_text(json.dumps(env, indent=2) + "\n")
    names = [n for n in scenarios() if matches(n, args.only)]
    log(f"{len(names)} scenarios")

    native_args = ["--iters-div", "4"] if args.quick else []
    log("native timing pass")
    perf_run([*names, *native_args], out_dir / "native.jsonl")
    if not args.skip_scale:
        scale_names = [n for n in SCALE_OPS if n in names]
        if scale_names:
            log("scale pass: the list queries with ten times the rows")
            perf_run([*scale_names, "--scale", "10", *native_args], out_dir / "scale10.jsonl")
    if strace_skip:
        log(f"strace pass skipped: {strace_skip}")
    elif not args.skip_strace and shutil.which("strace"):
        log("strace pass: syscalls per iteration")
        perf_run([*names, "--strace", "--iters-div", "8" if args.quick else "4"], out_dir / "strace.jsonl")
    if not args.skip_soak:
        log("memory soak: 20 000 mixed ops")
        with open(out_dir / "soak.log", "w") as logf:
            subprocess.run([str(PERF), "soak", "--ops", "4000" if args.quick else "20000", "--out", str(out_dir / "soak.json")], check=True, stderr=logf)
    if not args.skip_process:
        run_process(out_dir, 8 if args.quick else args.process_seconds)
    if not args.skip_callgrind and not args.quick and shutil.which("valgrind"):
        log(f"callgrind pass: {args.jobs} processes, instruction attribution per scenario")
        rows = run_callgrind(out_dir, names, args.jobs, args.callgrind_iters_div)
        profiles = parse_callgrind(out_dir, {r["name"]: r for r in rows})
        (out_dir / "profiles.json").write_text(json.dumps(profiles, indent=1) + "\n")
        log(f"{len(profiles)} profiles parsed")
    log("report")
    subprocess.run([sys.executable, str(ROOT / "scripts/perf/report.py"), str(out_dir)], check=True)
    log(f"done: {out_dir}")


if __name__ == "__main__":
    main()
