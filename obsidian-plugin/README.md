# Vault Wallpaper for Obsidian

An optional companion plugin. It tells the Vault Wallpaper app which note you have
open, so the wallpaper can highlight it: the open note grows, gets a label and its
direct neighbours brighten, while the rest of the graph dims a little. Saving the
open note starts the theme's edit burst (Ink & Paper's ink bleed) for exactly that
note.

There is no port and no network. The plugin writes one small file inside your vault:

```
<vault>/.obsidian/plugins/vault-wallpaper/state.json
```

and the app watches it. Without the plugin, or when Obsidian is closed, the wallpaper
behaves exactly as before.

## Build

Needs Node 18 or newer. Dev dependencies only, pinned in `package-lock.json`.

```bash
cd obsidian-plugin
npm install
npm run build
```

This produces `main.js` next to `manifest.json`.

## Install (manual)

This plugin is not in the Obsidian community store. Copy it into the vault yourself:

1. Make the folder `<vault>/.obsidian/plugins/vault-wallpaper/`. The folder name
   must be exactly `vault-wallpaper`: the app looks for the state file there.
2. Copy `main.js` and `manifest.json` into it.
3. In Obsidian: Settings → Community plugins. Turn off Restricted mode if it is on,
   then enable **Vault Wallpaper** (use the refresh button if it doesn't show up).
4. Open a note. `state.json` appears in the plugin folder, and the wallpaper app
   (pointed at the same vault) highlights the note within a fraction of a second.

To try it without touching your real vault, copy the vault first and point both
Obsidian (Open folder as vault) and the app (`--vault <copy>`, or Choose Vault…) at
the copy.

To remove it, disable the plugin and delete its folder. Disabling writes a final
"nothing open" state, so the highlight goes away at once.

## Settings

One switch: **Share the open note with the wallpaper**. Off writes "nothing open"
and keeps it that way. Disabling the plugin does the same.

## The state file

```json
{
  "version": 1,
  "active": "Projects/Wallpaper.md",
  "links": ["Ideas/Graph art.md", "Rust.md"],
  "saved": "Projects/Wallpaper.md",
  "savedAt": 1762400000500,
  "updated": 1762400000000
}
```

- `active`: the note in the active editor, vault-relative (`null` when none).
- `links`: Obsidian's resolved outgoing links of that note
  (`metadataCache.resolvedLinks`). The app's own reader already matches Obsidian's
  link rules, so these only add to the neighbours it finds itself.
- `saved` / `savedAt`: the last note modified while open in an Obsidian tab, and when.
  Changes from sync clients to notes you don't have open don't count.
- `updated`: when the file was written, in ms since the epoch.

Behaviour:

- Written atomically (a temp file, then a rename), debounced by 250 ms, and only when
  something in it changed.
- A heartbeat rewrites `updated` every two minutes while Obsidian is open. The app
  ignores a state older than five minutes, so if Obsidian quits without the plugin
  unloading, the highlight goes away on its own.
- While a fresh state is present, the app starts edit bursts from the plugin's save
  reports instead of from file events, so background syncs don't animate.

## Limits

- Desktop only (it writes with Node's `fs`).
- The app expects the default config folder `.obsidian`. If you changed Obsidian's
  config folder (Settings → Files and links → Override config folder), the app won't
  find the state file.
