# Vault Wallpaper

Your Obsidian vault's graph as a native macOS wallpaper: one app, drawn by the GPU, no Node, no
local server, no third-party wallpaper host. Rust + wgpu, macOS first.

Status: early. Phases 0–3 are done: platform spike, vault reader, layout, and a first
working wallpaper with the plain look. The full plan is in [`docs/plan.md`](docs/plan.md).

The web version, which runs today through Plash or Lively, lives on at
[obsidian-live-wallpaper](https://github.com/william-ricchiuti/obsidian-live-wallpaper).

## Layout

- `docs/plan.md`: architecture, phases, performance budget, open decisions.
- `phase0/`: baseline tools for the web version and the Phase 0 findings.
- `crates/app/`: the wallpaper app (`vault-wallpaper`).
- `crates/render/`: the GPU renderer (wgpu), including off-screen PNG rendering.
- `crates/core/`: the vault reader (scan, parse, link resolution, watcher) and the layout
  engine (force simulation, rings/timeline/galaxy, position cache). `cargo test`.
- `parity/`: proves the reader matches the web version's `parser.js`.
- `spike/`: the Phase 0 desktop-wallpaper spike (kept for reference).

## Run it

```bash
cargo run --release -p vault-wallpaper -- --vault ~/path/to/your/vault
```

It puts your vault's graph behind the desktop icons. Add `--front` to show it in a normal
window instead. Quit with Ctrl-C.

## Build the app

```bash
packaging/build-app.sh   # dist/Vault Wallpaper.app (universal, macOS 13+)
packaging/make-dmg.sh    # dist/Vault-Wallpaper-<version>.dmg
```

Signing (`sign.sh`) and notarization (`notarize.sh`) need a Developer ID; see
[`packaging/README.md`](packaging/README.md) for the order and the one-time setup.
