# Vault Wallpaper

Your Obsidian vault's graph as a native macOS wallpaper: one app, drawn by the GPU, no Node, no
local server, no third-party wallpaper host. Rust + wgpu, macOS first.

Status: early. Phase 0 (baseline measurements and a platform spike), Phase 1 (the vault
reader) and Phase 2 (layout) are done. The full plan is in [`docs/plan.md`](docs/plan.md).

The web version, which runs today through Plash or Lively, lives on at
[obsidian-live-wallpaper](https://github.com/william-ricchiuti/obsidian-live-wallpaper).

## Layout

- `docs/plan.md`: architecture, phases, performance budget, open decisions.
- `phase0/`: baseline tools for the web version and the Phase 0 findings.
- `crates/core/`: the vault reader (scan, parse, link resolution, watcher) and the layout
  engine (force simulation, rings/timeline/galaxy, position cache). `cargo test`.
- `parity/`: proves the reader matches the web version's `parser.js`.
- `spike/`: Rust + wgpu desktop-wallpaper spike.

## Try the spike

```bash
cd spike && cargo run --release
```

It places a window behind your desktop icons and draws animated dots. Quit with Ctrl-C.
