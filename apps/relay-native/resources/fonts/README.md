These fonts use the exact Latin font data from Relay-2 at
`1745dd3f68b7bc786f48142ad7da591537aeb057`, distributed by its installed
`@fontsource/fira-sans`, `@fontsource/fira-sans-condensed`, and
`@fontsource/fira-mono` packages. The WOFF containers were decompressed
losslessly to native TrueType using FontTools (TTFont, flavor=None);
no outlines, metrics or weights were edited. Each family's
SIL Open Font License is included. Pango registers them only for Relay.

The one naming change: fontsource names each weight as its own family
(name ID 1 `Fira Sans Medium`, `Fira Mono Medium`, ...), so fontconfig never
offered them for `font-family: 'Fira Sans'; font-weight: 500`. The 500 and 600
files carry the typographic family and subfamily the upstream Fira TTFs have
(name ID 16 `Fira Sans` / `Fira Sans Condensed` / `Fira Mono`, name ID 17
`Medium` or `SemiBold`), added with FontTools `name.setName` on the same
platform records as name ID 1. The 400 and Condensed 700 files already use the
base family and are unchanged.
