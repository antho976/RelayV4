# Source boundaries

Inspected on 2026-09-04:

| Source | Revision | Use here |
| --- | --- | --- |
| https://github.com/antho976/Relay-2 | `1745dd3f68b7bc786f48142ad7da591537aeb057` | Engine, bus, CLI, generated schema and tests; visual authority |
| https://github.com/antho976/Relay-V3 | `0086ac25385e418cd9167616403e19d3f452831d` | Native dependency versions, protocol lessons, archived roadmap and supporting docs |

`crates/` and `schema/` originate from the pinned Relay-2 revision. Any subsequent
changes appear in this repository's history. `apps/relay-native/src/` is written
fresh here; V3's application implementation was not copied. Relay-2's
`Icon.svelte` geometry is ported directly into native `icons.rs`. The bundled
scrcpy server and its upstream license are documented in
`apps/relay-native/resources/README.md`.

`docs/upstream-v3/` is a verbatim snapshot. Its dates, task numbers, paths,
completed gates, decisions, ownership rules and instructions describe V3, not
this checkout. Relative links between archived documents remain intact.
`docs/engine/` began as Relay-2's bus/spec/decision reference and is now this
checkout's current contract: `BUS.md` and `DECISIONS.md` are kept in step with
the code, which cites them, and decisions are appended, not rewritten. `SPEC.md`
is the product spec they refer to and is updated less often; where it still
describes Relay-2's Tauri and Svelte app, the code and `BUS.md` are the truth.

Both reference repositories remain untouched. No state migration or retirement
of either application is implied by this rebuild.
