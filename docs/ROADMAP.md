# Native rebuild and later roadmap

The requested V3 roadmap is preserved verbatim at
[upstream-v3/ROADMAP.md](upstream-v3/ROADMAP.md), with its
[feature backlog](upstream-v3/BACKLOG.md) and supporting documents.
V3 completion marks are historical, not this project's status.

## Current rebuild

Fresh GTK4 presentation with Relay-2's compact console and terminal wall, over
the retained engine. Priorities are live terminals, bounded asynchronous I/O,
safe session lifecycle, task dispatch, builder/reviewer separation, mailbox and
explicit guardrail decisions. GtkSourceView remains the native code surface.

Current feature coverage is recorded in [PARITY.md](PARITY.md).

Acceptance requires engine contract tests, native transport tests, a display
smoke pass and honest reporting of visual and performance limits.

## After the rebuild

Keep the original requirements in the archived roadmap. Several overlapping
features now have native implementations; their historical V3 checkmarks do
not certify native validation. Advanced docking, migration, packaging and
release acceptance still follow the archived plan. No archived document is changed.

The archived product decisions remain open unless Antho resolves them in this
project. Same native stack and Relay-2 visual direction are confirmed here.
