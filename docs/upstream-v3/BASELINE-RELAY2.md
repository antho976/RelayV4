# Relay 2 baseline

Observed 2026-09-03/04 on the build machine at pinned Relay 2 commit
`1745dd3f68b7bc786f48142ad7da591537aeb057`. Relay 2 was not modified. The run used a clean
`git archive` at `/tmp/relay2-baseline.TyBNEM`, a copied `node_modules`, and isolated release
artifacts. Its app and store used the `test` instance, leaving Antho's running `dev` instance
untouched.

This is a usable comparison baseline, not a full visual-latency benchmark. Measurements that
need a 60/240 fps camera, root page-cache control, or desktop input automation are called out as
missing instead of being inferred from control-plane timings.

## Build and fixture

```sh
git -C /home/anthony/dev/Relay-2 archive 1745dd3 | tar -x -C /tmp/relay2-baseline.TyBNEM
npm --prefix /tmp/relay2-baseline.TyBNEM/apps/relay-app run build
cargo build --release -p relay-app --target-dir /tmp/relay2-baseline.TyBNEM/target
cargo build --release -p relay-cli --target-dir /tmp/relay2-baseline.TyBNEM/target
```

Both release builds passed. The isolated app ran on the real KDE display through XWayland with
`GDK_BACKEND=x11`, `WEBKIT_DISABLE_DMABUF_RENDERER=1`, and
`XDG_DATA_HOME=/tmp/relay2-baseline.TyBNEM/data`. A disposable provider emitted deterministic
50 MiB streams. Six, then eleven, live sessions were attached in the Agents wall.

Visual evidence:

- [six live panes](baseline/artifacts/relay2-six-pane.png)
- [eleven live panes](baseline/artifacts/relay2-eleven-pane.png)
- [eleven-pane wall after engine completion](baseline/artifacts/relay2-eleven-stream-settled.png)

## Six-pane idle

The Relay window was unfocused. The app, WebKit network process, and WebKit web process were
sampled every five seconds for one minute. The first exploratory run was discarded because the
window still held focus. Agents' own processes are excluded.

```sh
top -b -d 5 -n 13 -p 952225,952237,952240
ps -o pid=,rss=,comm= -p 952225,952237,952240
awk '/voluntary_ctxt_switches/ { print FILENAME, $0 }' \
  /proc/952225/status /proc/952237/status /proc/952240/status
```

| valid run | mean app CPU | mean web CPU | total CPU | summed RSS | context-switch delta, app / network / web |
|---|---:|---:|---:|---:|---:|
| 1 | 0.850% | 2.000% | 2.850% | 1,016,216 KiB | 4,209 / 2 / 3,506 |
| 2 | 1.683% | 3.883% | 5.567% | 1,101,708 KiB | 12,681 / 2 / 6,399 |
| 3 | 0.933% | 1.917% | 2.850% | 1,147,392 KiB | 5,063 / 3 / 4,252 |

Network-process CPU rounded to 0.000% in every run. The web process grew from 668,480 KiB to
799,968 KiB across the three consecutive idle measurements. This is process RSS and can double
count shared pages, but it is the same procedure the native comparison must use.

## Terminal creation

Six `session.create` plus `session.spawn` operations were timed from the release CLI. The first
three were 3,419 ms, 3,160 ms, and 4,141 ms; all six were 3,419, 3,160, 4,141, 3,525, 3,121, and
3,840 ms. The resulting panes were visually confirmed on the real display.

These are control-plane spans, not the required 60 fps measurement from click to first visible
prompt. Relay 2's logs did not expose a trustworthy first-painted-frame timestamp, so no visual
latency number is claimed.

## Eleven-pane streaming

Each of eleven sessions emitted exactly 50 MiB concurrently. Sampling used:

```sh
top -b -d 1 -n 25 -p 952225,952237,952240
ps -o pid=,rss=,comm= -p 952225,952237,952240
```

| run | successful inputs | mean total CPU | peak component sum | summed RSS |
|---|---:|---:|---:|---:|
| 1 | 11/11 | 8.667% | 61.0% | 1,363,948 KiB |
| 2 | 11/11 | 22.067% | 297.7% | 1,446,184 KiB |
| 3 | not completed | not available | not available | not available |

For run 1, `session.scrollback` showed `stream complete` for all eleven sessions. More than one
minute later the visible wall still showed intermediate output in every sampled pane while CPU
had returned to roughly 3% total. The settled screenshot above captures that divergence. Run 2
raised peak combined component CPU to nearly three cores and RSS to about 1.45 GiB. Before run 3,
the isolated processes became unreachable from the measurement process namespace while their
socket remained, so the third run was stopped rather than publishing a false sample.

## Measurements not established

| measurement | result |
|---|---|
| cold and warm startup with six attached panes | not measured; cold cache eviction requires root, and no first-frame instrumentation exists |
| terminal removal visual latency | not measured; CLI close timing is not a substitute for a 60 fps pane-removal recording |
| scroll and resize correctness | eleven-pane layouts were observed at 2048×1120, but no desktop input automation or capture timing was available |
| session survives pane removal | Relay 2 exposes Park and destructive Close in this wall, not the specified non-destructive visible-pane removal contract |

## Comparison contract for Relay 3

Relay 3 should repeat the three idle and eleven-pane runs with one native process, then add real
60/240 fps capture for interaction latency. Its first user-visible milestone must already keep
the engine authoritative, so hiding or closing a view cannot destroy a running session. The most
important regression test from this baseline is not only lower CPU or memory: after a 50 MiB
burst, every visible pane must converge to the same final marker returned by `session.scrollback`.

