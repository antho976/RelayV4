#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
export RELAY_INSTANCE="${1:-dev}"
if [[ $# -gt 1 || ! "$RELAY_INSTANCE" =~ ^(dev|stable|test)$ ]]; then
    echo "Usage: $0 [dev|stable|test]" >&2
    exit 2
fi

# Never inherit a stale screenshot/test socket when launching the actual app.
unset RELAY_NATIVE_SOCKET

# Keep the build and executable paths together, including with Cargo overrides.
mkdir -p target
echo "Building Relay (compiler output: $PWD/target/launcher-build.log)…"
if cargo build --target-dir "$PWD/target" -p relay-cli -p relay-native >target/launcher-build.log 2>&1; then
    # The desktop entry is one file per user, not per checkout or instance. Only the primary
    # checkout's default instance keeps it current, so `./run.sh test` or a run from an agent's
    # worktree (removed later) does not repoint the user's launcher; RELAY_INSTALL_DESKTOP=1
    # repoints it on purpose. A launcher that cannot be written is no reason not to open the app.
    if [[ "${RELAY_INSTALL_DESKTOP:-}" == 1 ]] || { [[ "$RELAY_INSTANCE" == dev ]] \
        && [[ "$(git rev-parse --path-format=absolute --git-dir 2>/dev/null)" == "$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" ]]; }; then
        python3 scripts/install-native-desktop.py "$RELAY_INSTANCE" \
            || echo "Could not register the desktop launcher; continuing." >&2
    fi
    echo "Opening Relay V4 ($RELAY_INSTANCE)…"
else
    result=$?
    cat target/launcher-build.log >&2
    exit "$result"
fi
relay="$PWD/target/debug/relay"
ready() { timeout 2 "$relay" --instance "$RELAY_INSTANCE" ping >/dev/null 2>&1; }
q() { timeout 5 "$relay" --instance "$RELAY_INSTANCE" q "$@" 2>/dev/null; }

# A running engine is reused, but it may predate the build above (or come from another
# checkout). /proc/<pid>/exe is the image the engine was started from, so it is the same file
# as $relay only when nothing has rebuilt it since. A stale engine with no live sessions is
# restarted; one holding sessions is left alone, since quitting it would end them, and the
# mismatch is said out loud instead of silently running old engine code.
if ready && status=$(q app.status); then
    read -r engine_pid live < <(python3 -c 'import json,sys; s=json.load(sys.stdin); print(s["pid"], s["sessions_live"])' <<<"$status") || engine_pid=
    if [[ -n "$engine_pid" && -e "/proc/$engine_pid/exe" && ! "/proc/$engine_pid/exe" -ef "$relay" ]]; then
        if [[ "$live" == 0 ]]; then
            echo "The running $RELAY_INSTANCE engine (pid $engine_pid) is not this build; restarting it…"
            q app.quit '{}' >/dev/null || true
            for _ in $(seq 50); do ready || break; sleep 0.1; done
        else
            echo "Warning: the running $RELAY_INSTANCE engine (pid $engine_pid) is not this build and holds $live live sessions;" >&2
            echo "  it keeps running its old code. Restart it deliberately: relay --instance $RELAY_INSTANCE q app.quit '{\"force\":true}'" >&2
        fi
    fi
fi

if ! ready; then
    log="$PWD/target/engine-$RELAY_INSTANCE.log"
    # --remote also opens the phone door (docs/MOBILE.md): the desktop and the phone share
    # this engine, and pairing is `relay remote pair` with nothing else to start. The door
    # answers nobody until a phone is paired or a pairing window is open.
    #
    # setsid puts the engine in its own session: Ctrl+C or Ctrl+Z in this terminal must not
    # reach it, since it outlives the window and holds every agent's terminal. This script runs
    # without job control, so the background child is not a group leader, setsid does not fork,
    # and $! is still the engine's pid.
    setsid nohup "$relay" --instance "$RELAY_INSTANCE" serve --remote >>"$log" 2>&1 </dev/null &
    engine_pid=$!
    deadline=$((SECONDS + 15))
    until ready; do
        if ! kill -0 "$engine_pid" 2>/dev/null || (( SECONDS >= deadline )); then
            echo "Relay engine did not become ready. See $log" >&2
            tail -n 20 "$log" >&2
            exit 1
        fi
        sleep 0.2
    done
fi

# The engine outlives the window so closing the UI preserves agent sessions.
exec "$PWD/target/debug/relay-native"
