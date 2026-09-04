# Source boundaries

Inspected on 2026-09-04:

| Source | Revision | Use here |
| --- | --- | --- |
| https://github.com/antho976/Relay-2 | `1745dd3f68b7bc786f48142ad7da591537aeb057` | Engine, bus, CLI, generated schema and tests; visual authority |
| https://github.com/antho976/Relay-V3 | `0086ac25385e418cd9167616403e19d3f452831d` | Native dependency versions, protocol lessons, archived roadmap and supporting docs |

`crates/` and `schema/` originate from the pinned Relay-2 revision. Any subsequent
changes appear in this repository's history. `apps/relay-native/src/` is written
fresh here; V3's application implementation was not copied.

`docs/upstream-v3/` is a verbatim snapshot. Its dates, task numbers, paths,
completed gates, decisions, ownership rules and instructions describe V3, not
this checkout. Relative links between archived documents remain intact.
`docs/engine/` preserves Relay-2's bus/spec/decision reference.

Both reference repositories remain untouched. No state migration or retirement
of either application is implied by this rebuild.
