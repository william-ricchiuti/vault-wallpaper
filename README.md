# Vault Wallpaper

Your Obsidian vault's graph as your Mac's desktop wallpaper. Every note is a point, every
link a line, and the picture updates as you write. It's a small native app that lives in
the menu bar: no browser, no Node, no local server, no wallpaper host app.

![The four launch looks: Default, Night Sky, Aurora Lite and Ink & Paper](docs/phase6/all-themes.png)

**Status: 1.0 beta.** macOS 13 Ventura or later, Apple Silicon or Intel.

## Install

### From the DMG

1. Download `Vault-Wallpaper-<version>.dmg` from
   [Releases](https://github.com/william-ricchiuti/vault-wallpaper/releases).
2. Open it and drag **Vault Wallpaper** to **Applications**.
3. Open Vault Wallpaper from Applications. A small graph icon (three linked dots) appears in
   the menu bar. The app has no Dock icon.

### With Homebrew

Coming soon:

```bash
brew install --cask william-ricchiuti/tap/vault-wallpaper
```

## First run

The first time it opens, Vault Wallpaper asks you to choose your Obsidian vault (the folder
that holds your notes). It reads the vault, lays out the graph and draws it behind your
desktop icons, on every Space and every display. The layout is cached, so later launches
don't have to lay it out again.

New installs start still: the picture only changes when your notes do. Motion (glow that
breathes, edit effects) is opt-in in Settings.

Used the web version (obsidian-live-wallpaper)? Choose **Import Web Version Settings…** in the
menu to bring your old `config.json` and custom presets over. See
[Migrating from the web version](docs/migrating-from-web.md).

## The menu

| Item | What it does |
|---|---|
| Pause Wallpaper | Freezes the picture (no animation) until you untick it. |
| Settings… | Themes, presets, colours, labels, layout and motion. Changes apply as you make them. |
| Choose Vault… | Switch to another vault. |
| Import Web Version Settings… | Read a `config.json` from the web version. |
| Export PNG | Saves the current picture to `~/Pictures/Vault Wallpaper/` and shows it in Finder. |
| Open Config File | Opens `config.json` in your text editor; edits apply when you save. |
| Start at Login | Opens Vault Wallpaper when you log in. |

## Themes

| Theme | Look |
|---|---|
| **Default** | The classic glowing graph. Presets (Neon, Blueprint, Parchment, Synthwave and more) tune its colours, glow and labels. |
| **Night Sky** | Notes as stars, your busiest notes as the brightest, tags as constellations. |
| **Aurora Lite** | Soft aurora curtains rising over your biggest tag clusters. |
| **Ink & Paper** | A printed plate: ink on warm paper. With motion on, saving a note makes its ink bleed for a moment. |

| Night Sky | Aurora Lite | Ink & Paper |
|---|---|---|
| ![Night Sky](docs/phase6/night-sky.png) | ![Aurora Lite](docs/phase6/aurora-lite.png) | ![Ink & Paper](docs/phase6/ink-paper.png) |

Pick a theme or preset in **Settings…**. You can save your own presets there too.

## Settings

Settings are stored in `~/Library/Application Support/Vault Wallpaper/config.json`, in the
same format the web version used, so you can also edit the file by hand. The app watches it
and applies changes when the file is saved. Invalid values are skipped (the rest of the file
still applies) and logged.

Notable settings: accent and background colours, tag colours, which folders to ignore
(`ignorePaths`), whether to show links to notes that don't exist yet
(`showUnresolvedLinks`), label size and source, layout (force, rings, timeline, galaxy),
and motion.

## Privacy

Everything stays on your Mac. Vault Wallpaper reads your vault's Markdown files to find notes,
links and tags, and never changes them. It makes no network connections: no analytics, no
crash reports, no update checks. Its own files are:

- `~/Library/Application Support/Vault Wallpaper/`: `config.json`, `custom-presets.json` and
  the cached layout (`layout.json`).
- `~/Pictures/Vault Wallpaper/`: PNGs you export.

## Light on your Mac

It draws only when something changes. With the wallpaper covered by windows, or still, it
draws nothing. Measured on a 2,600-note vault (1.0 beta, MacBook Air display): about 0.2% CPU
and a 97 MB memory footprint at idle after a launch from the cached layout; the very first
run, which lays the graph out from scratch, settles at about 130 MB.

## Uninstall

Quit it from the menu, untick Start at Login first if you turned it on, and move the app to the
Bin. To remove its settings too, delete `~/Library/Application Support/Vault Wallpaper/`.

## Build from source

Needs Rust (rustup) and the Xcode command line tools.

```bash
cargo run --release -p vault-wallpaper -- --vault ~/path/to/vault   # run without bundling
packaging/build-app.sh                                              # dist/Vault Wallpaper.app (universal)
packaging/make-dmg.sh                                               # dist/Vault-Wallpaper-<version>.dmg
cargo test --workspace
```

`--front` shows the graph in a normal window instead of on the desktop. Signing,
notarization and the release steps are in [`packaging/README.md`](packaging/README.md) and
[`docs/release-checklist.md`](docs/release-checklist.md).

Repository layout: `crates/core` (vault reader, layout engine, config), `crates/render` (the
wgpu renderer and themes), `crates/app` (the macOS app: wallpaper windows, menu, settings
window), `packaging/` (bundle, DMG, signing, Homebrew cask draft), `docs/plan.md` (architecture
and roadmap).

## The web version

The original, browser-based version lives on at
[obsidian-live-wallpaper](https://github.com/william-ricchiuti/obsidian-live-wallpaper) and
still runs on Windows and Linux through wallpaper hosts like Lively.

## Licence

MIT. See [LICENSE](LICENSE).
