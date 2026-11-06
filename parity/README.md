# Parity test

The Rust reader (`crates/core`) must produce the same graph and excerpts as the web
version's `parser.js`. Node and link *order* is ignored; everything else must match.

```bash
parity/run.sh <obsidian-live-wallpaper checkout> [vault [ignore-path...]]
```

- Always checks a generated edge-case vault (`make-fixture.js`): duplicate names,
  case-insensitive and path links, aliases, headings, embeds and attachments, every tag
  syntax, CRLF, BOM, Unicode names and spaces, hidden and ignored folders, symlinks.
- Pass your own vault to check it too, for example
  `parity/run.sh ../obsidian-live-wallpaper ~/Vault .obsidian`.
- `make-big.js <dir> [n]` generates a synthetic vault (35,000 notes by default) for
  speed tests.

Results on 2026-11-05: identical on the fixture (3 variants), the Willy-AI vault (2,806
nodes, 9,043 links, 2,597 excerpts) and a 35,000-note synthetic vault (104,141 links).

Known difference, on purpose: notes named `toString`, or links like `[[constructor]]`,
crash `parser.js` (it uses plain objects as lookup tables) but work in Rust. The fixture
avoids those names.

## Layout comparison

`layout-js.js <web checkout> <vault> <out.json> [force|rings|timeline|galaxy]` runs the web
version's layout (`layout-core.js` + d3) with the worker's settle rule. Draw it, and the Rust
layout from `cargo run --release --example layout_bench -- <vault> --out rust.json`, with
`cargo run --release --example draw_layout -- <graph.json> <positions.json> <out.png>`.
Layouts are compared by eye: the starting points differ, so positions never match exactly.
