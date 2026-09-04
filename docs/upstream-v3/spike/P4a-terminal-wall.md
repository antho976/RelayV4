# P4a: eleven-pane terminal wall

Observed 2026-09-04 on the machine in `docs/READINESS.md`. The implementation is a temporary
four-column GTK grid behind explicit `--wall` session names. All panes share one socket client;
each VTE is its own scroll owner. Hide keeps the attachment, while Remove detaches without
stopping the engine-owned session.

## Outcome: repaired, proceed gate passed

The required eleven simultaneous 50 MiB streams now complete without a sequence gap, duplicate,
re-attach, client queue drop, or engine subscriber-lag warning. Every pane received 819 frames and
53,544,387 PTY bytes, including the fixture prompt and PTY newline translation. All eleven
`stream complete` prompts were visible after the burst.

The repair is at the producer boundary in `relay-core::pty`: raw kernel reads are coalesced for
17 ms, capped at 64 KiB, and only then assigned one sequence number, retained, and broadcast.
This makes the existing bounded engine and client queues carry about 60 display-cadence frames per
second instead of thousands of tiny reads. It changes no bus operation or frame schema and adds no
dependency. The brief §14 eleven-pane feasibility condition is now met.

## Fixture and procedure

- Release builds of `relay-cli` and `relay-native` from this repository.
- A scratch `test` instance and store under `/tmp/relay-p4a-amber-zebra-eWOAs2`; `dev` and
  `stable` were untouched.
- Eleven synthetic provider sessions. Each `stream` command ran exactly
  `yes 'relay-p4a-0123456789abcdefghijklmnopqrstuvwxyz' | head -c 52428800` once.
- A nested 1920×1080 KWin/Xwayland display (`DISPLAY=:7`), with the app window at 1880×1010.
  This isolates the proof but is not the real desktop display, so it cannot validate perceived
  smoothness, physical input latency, colours, or compositor behaviour.
- A 60 fps capture and GTK logs were used for lifecycle inspection. GTK Inspector's frame overlay
  was not available in the nested setup.
- The final repair retest used another isolated KWin/Xwayland display (`DISPLAY=:1`), maximized to
  1920×1052. `top -b -d 1 -n 45` sampled the burst; final pane counters were emitted by the normal
  graceful-close path.

Representative launch, using fish syntax:

```fish
set -x RELAY_INSTANCE test
set -x DISPLAY :1
set -x GDK_BACKEND x11
set -x RUST_LOG relay_native::app=info,relay_native::terminal=info,relay_native::bus_client=warn
target/release/relay-native --wall calm-narwhal,olive-bison,umber-llama,ivory-gannet,sly-raven,sharp-seal,quiet-crane,coral-bison,quiet-seal,olive-marten,silver-osprey
```

Measurements follow `docs/VERIFICATION.md` §4.

| measurement | observed |
|---|---|
| idle wall | all 11 panes attached in a 4×3 grid; original run was 51×11 cells, repaired maximized run was 46×12 |
| idle CPU | 13 samples at 5 s intervals: 0.0% for the native process in every sample |
| eleven-pane RSS | original: 195,044 KiB before streaming, 203,996 KiB after recovery; final repaired retest: 195,480 KiB before, 201,164 KiB peak and settled |
| six-pane RSS | 197,040 KiB after six 256 KiB catch-ups; not directly comparable to the clean eleven-pane baseline |
| exact stream load, before repair | 13 sequence gaps and 13 re-attaches across 11 panes; every pane gapped at least once |
| exact stream load, repaired | 9,009 total frames; zero gaps, duplicates, re-attaches, client drops, and engine subscriber-lag warnings; all prompts visible after about 15 s |
| native CPU during repaired load | 45 one-second `top` samples: 28.0–29.9% through the first 14 samples, 21.0% on the fifteenth, then 0.0%; the engine was 17.0–26.0% over the same interval. This is the full 550 MiB render, not the earlier dropped-work result |
| engine RSS during repaired load | 20,864 KiB before, 204,564 KiB peak, 203,988 KiB settled; eleven 8 MiB scrollback rings account for 88 MiB of retained payload |
| delivered terminal bytes, repaired | 53,544,387 per pane, 588,988,257 total; the total exceeds the commands' 550 MiB because PTY output processing expands newlines and the fixture adds its prompt |
| hide/reveal | attachment stayed live; a re-entrant `RefCell` panic found by the first click was fixed by dropping the pane borrow before GTK emits `map`; the repeated capture completed without a crash |
| remove | pane disappeared, detached once, and `session.list` still showed the session as `running`; the finish review then found and fixed retained pane ownership, but destruction and RSS were not remeasured |
| client restart | after the intentional lifecycle-crash discovery, all 11 engine-owned sessions remained running; relaunch delivered one bounded catch-up per requested pane, with zero duplicate frames reported |
| input latency | not measured: no physical 240 fps or `evtest` path was available in the nested display |
| engine restart | not run after the stream stop-gate; restarting the engine would end these engine-owned PTYs and cannot prove continuity |

The first diagnostic run accidentally sent twelve bursts per pane. It produced the same queue-gap
mechanism but is excluded from the table. The exact one-burst run above is the gate evidence.

## Failure mechanism and repair

`bus_client::FRAME_CAPACITY` is 256. `route_frame` uses `try_send`; a full pane channel drops the
frame. `terminal::drain` then observes the sequence jump and immediately detaches/re-attaches.
The original overload also logged `relay_core::socket: pty subscriber lagged`, proving the
pressure began before the client queue. `relay-core::pty` used a 64 KiB read buffer but assigned a
sequence and broadcast every raw read, which can be far smaller than the buffer.

The repair coalesces available reads for one 17 ms display interval, stops at 64 KiB, and applies
backpressure to the child if that bound is reached early. Sequence numbers still describe exactly
the frames retained by the engine and sent over the socket, so detach/resume semantics are
unchanged. The smaller live-frame cap leaves room for several complete live frames inside one 256
KiB attach catch-up. The repaired burst produced 818 load frames per pane at no more than 60 frames
per second, so neither the 1,024-frame engine subscriber nor 256-frame client queue filled.

No sampling profiler (`perf`, Sysprof, Valgrind, strace, or bpftrace) was installed. The diagnosis
used the explicit client gap counters, engine subscriber-lag warning, per-pane frame/byte counters,
and one-second `top` samples. The repair targeted that measured queue pressure; no unrelated
rendering optimisation was applied.

## Captures

- [`wall-idle.png`](artifacts/p4a/wall-idle.png): all eleven attached panes.
- [`wall-lifecycle.mp4`](artifacts/p4a/wall-lifecycle.mp4): 60 fps nested-display hide, reveal,
  and remove capture after the panic fix.
- [`wall-after-remove.png`](artifacts/p4a/wall-after-remove.png): pane gone while the sidebar still
  shows its session running.
- [`wall-repair-after-burst.png`](artifacts/p4a/wall-repair-after-burst.png): the final reviewed
  17 ms / 64 KiB build after all eleven exact bursts, with every pane attached at epoch/seq 1/819
  and every `stream complete` prompt visible.

These captures prove layout and state transitions only. They are not a real-display visual signoff.
The lifecycle capture predates the ownership and accessible-name follow-up; that follow-up has
compile/test coverage only. A keyboard and assistive-technology pass remains open.
