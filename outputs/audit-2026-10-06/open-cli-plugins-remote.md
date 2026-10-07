# Audit findings left open: CLI, plugin MCP servers, phone door, launcher

Scope: RA-060..083, RA-259..266, RA-270/271, RA-562..564, RA-576/577, fixed on branch
`relay/keen-heron` (2026-10-06). Everything in that range is fixed except the parts below. Each
says why it is open and what closing it would take.

## Cannot be fixed from Relay

- **RA-077, Unreal half: `ue_python` code is not sandboxed.** `ue_python`, `ue_console`,
  `ue_call`, `ue_play` and `ue_profile` now pass `guardrail.gate` (deny rules, holds, audit). The
  code then runs inside the user's already-open editor, with all of the editor's access, and no
  outside process can fence that in. (Blender, which Relay starts itself, now runs under
  bubblewrap.) The one stronger option is policy: hold every `ue_python` call for approval. That
  would interrupt every agent use, and the user chose not to.
- **RA-061, Unreal half: a running in-editor call cannot be stopped.** `notifications/cancelled`
  drops the answer and skips calls not yet started. Python already executing inside the editor
  has no interrupt API, so it runs to its own timeout.

## Needs a real Unreal editor (deliberately not attempted)

- **RA-082, the `ue_play` outside camera still marks the level modified.** Capture, import-preview
  and anim-preview actors are now transient and leave the level clean. The play-session camera
  cannot be transient, because transient actors are not copied into the play world, and Unreal
  has no way to clear a map's dirty flag. `ue_editor_quit` now names a map that was clean before
  `play.py` touched it, so the rewrite can be reverted. The real fix is to spawn the camera
  inside the running game world instead (e.g. `GameplayStatics` deferred spawn with the game
  world as context). The code's author believed Python cannot do that, and only a real editor can
  show whether the needed functions are exposed to Python. Without Unreal on this machine it was
  left rather than shipped untested.

## Blocked on files held by other sessions (requested from `amber-heron`)

These are fixable. They need small changes in relay-core files that `amber-heron` was rewriting at
the time (`relay/amber-heron`), so they were requested there rather than written as conflicting
edits.

- **RA-061, Blender half: cancelling a Blender call does not kill Blender.** It needs a cancellable
  variant of `relay_core::proc::output_with_timeout` (a cancel flag that triggers the existing
  TERM-then-KILL path). The Blender server would then pass the call's cancel flag through. Today a
  cancelled call that has started runs to its timeout and its answer is dropped.
- **RA-562 follow-up: the hook never fires for NotebookEdit or the plugin tools.** The Claude
  PreToolUse matcher in `crates/relay-core/src/hooks.rs` needs `NotebookEdit` and
  `mcp__blender__.*|mcp__unreal__.*`. relay-cli's adapter already translates both.
- **RA-261 follow-up: no lasting desktop notice of a new pairing.** The door sends a `ui.toast`
  (shown in any open desktop window) and logs at warn. A notification-centre entry needs a
  user-callable `notify.post` op in relay-core (`handlers/notify.rs`, `relay-bus/src/ops/notify.rs`),
  which the door would call next to the toast in `announce_pairing`.

Already covered elsewhere: the unbounded Blender output inside `relay_core::proc` is capped (64
MiB per stream) on `relay/amber-heron`.

## Fixed but not run end to end here

- Unreal Python changes (RA-081 capture diffing, RA-082 transient spawns, the animation checks) were
  run only against the stand-in `unreal_py/tests/unreal.py`, not a real editor.
- Blender 4.2-4.5 paths (RA-071 slot choice, RA-075 `BLENDER_EEVEE_NEXT` fallback) were not run;
  the fixture tests use Blender 5.2.1.
- The phone side (90 s pairing wait, `pair.declined` / `pair.unconfirmed` messages) was
  typechecked and linted only. There is no Android SDK here.
- `scripts/native-smoke.py` now runs the roadmap parts after the default run (RA-576), and
  `scripts/perf/native-baseline.py` has a new counter window (RA-577). Neither was run here, since
  both open app windows; the roadmap parts' assumptions about fixture state at the end of the run
  are untested.
- The systemd unit (RA-266) and the interactive y/N pairing prompt were not exercised by hand.
