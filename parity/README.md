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

## Config

`config-js.js <web checkout>` (also run by `run.sh`) feeds configs through `parser.js`'s
`sanitizePersistedConfig` and `sanitizeConfigPatch` and checks `vw_core::config` against
them: every `presets.json` entry (as a patch and as a whole file), `config.example.json`,
every theme id, and about 150 deliberately bad values (wrong types, out-of-range
numbers, `null`s, bad nested `labelStyle`/`safeArea`/`rotate`, UTF-16 length and JS
`trim` edge cases, non-object roots).

- Patches must match exactly: same accept/reject, same cleaned values.
- Whole files differ on purpose: the web version rejects a file with one bad key, the
  native app drops that key and keeps the rest. So Rust must flag a file as clean exactly
  when `parser.js` accepts it, drop exactly the keys `parser.js` refuses, and produce what
  `parser.js` gives for the file without them.
- `port`, `refreshMs`, `fastHash` and `renderer` are checked for accept/reject only;
  the native app never keeps them.
- Not compared: `vaultPath` existence (`parser.js` requires the folder to exist; Rust
  leaves that to the app) and a missing `vaultPath` (required on the web, optional
  natively until the user picks a vault).

Results on 2026-11-05: 414 cases identical (213 files, 201 patches, 34 presets).

## Layout comparison

`layout-js.js <web checkout> <vault> <out.json> [force|rings|timeline|galaxy]` runs the web
version's layout (`layout-core.js` + d3) with the worker's settle rule. Draw it, and the Rust
layout from `cargo run --release --example layout_bench -- <vault> --out rust.json`, with
`cargo run --release --example draw_layout -- <graph.json> <positions.json> <out.png>`.
Layouts are compared by eye: the starting points differ, so positions never match exactly.
