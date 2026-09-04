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
    echo "Opening Relay V4 ($RELAY_INSTANCE)…"
else
    result=$?
    cat target/launcher-build.log >&2
    exit "$result"
fi
relay="$PWD/target/debug/relay"
ready() { timeout 2 "$relay" --instance "$RELAY_INSTANCE" ping >/dev/null 2>&1; }

if ! ready; then
    log="$PWD/target/engine-$RELAY_INSTANCE.log"
    nohup "$relay" --instance "$RELAY_INSTANCE" serve >>"$log" 2>&1 </dev/null &
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
