# Changelog

All notable changes to Vault Wallpaper. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.0.0-beta.1] - unreleased

The first beta of the native macOS app, a rewrite of the web version
([obsidian-live-wallpaper](https://github.com/william-ricchiuti/obsidian-live-wallpaper)).

### Added

- Your Obsidian vault's graph drawn behind the desktop icons, on every Space and display,
  by the GPU (wgpu on Metal). No Node, local server or wallpaper host.
- Vault reader with the web version's rules (wikilinks and Markdown links, aliases, tags, ignored paths),
  checked against `parser.js` by parity tests. It watches the vault and updates the picture
  when notes change.
- Layout engine on a background thread (force, rings, timeline, galaxy), with positions
  cached to disk.
- Themes: Default (the web renderer and its presets), Night Sky, Aurora Lite and
  Ink & Paper, with Ink & Paper's ink bleed when a note is saved (motion on).
- Menu-bar app (no Dock icon): Pause, Settings, Choose Vault, Import Web Version Settings,
  Export PNG, Open Config File, Start at Login.
- First run asks for the vault; new installs start with motion off.
- Settings window with the web version's settings page: themes, presets, custom presets,
  colours, labels, layout and motion. Changes apply live.
- Import of the web version's `config.json` and `custom-presets.json`.
- Config at `~/Library/Application Support/Vault Wallpaper/config.json`, in the web format,
  reloaded when edited.
- Draws nothing while covered or still; glow breathing runs in the window server.
- Universal (Apple Silicon and Intel) app bundle for macOS 13+ and app icon, as a signed and
  notarized DMG and a Homebrew cask.
- Automatic updates for the DMG and Homebrew versions (Sparkle 2): one small HTTPS check a
  day, and Check for Updates… in the menu. The Mac App Store build has no updater.

### Not yet

- Native versions of most web themes (they draw with Default for now).

[Unreleased]: https://github.com/william-ricchiuti/vault-wallpaper/compare/v1.0.0-beta.1...HEAD
[1.0.0-beta.1]: https://github.com/william-ricchiuti/vault-wallpaper/releases/tag/v1.0.0-beta.1
