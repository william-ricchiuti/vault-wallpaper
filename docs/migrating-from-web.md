# Migrating from the web version

Vault Wallpaper replaces [obsidian-live-wallpaper](https://github.com/william-ricchiuti/obsidian-live-wallpaper)
on macOS. It reads the same `config.json` format, so your settings and custom presets come
across. You no longer need Node, `npx`, Plash or a browser tab.

## 1. Find your old config

The web version keeps its config next to where it runs:

| How you ran it | Config file |
|---|---|
| A git clone (`npm start`) | `config.json` in the clone's folder |
| `npx obsidian-live-wallpaper` | `obsidian-live-wallpaper.config.json` in the folder you ran it from |
| `--config <file>` | that file |

Custom presets saved in its settings page are in `custom-presets.json` in the same folder.

## 2. Import it

1. Install and open Vault Wallpaper (see the [README](../README.md#install)). If it asks for a
   vault on first run, you can cancel; the import sets it.
2. In the menu bar, click the graph icon and choose **Import Web Version Settings…**.
3. Pick the old config file. If you already have settings, the app asks before replacing
   them.

What happens:

- Every setting that still means something is copied to
  `~/Library/Application Support/Vault Wallpaper/config.json`, including `vaultPath`.
- `port`, `refreshMs`, `renderer` and `fastHash` are dropped: there is no server or browser
  renderer any more. Any value that is invalid is skipped too; the rest still imports. The
  confirmation lists what was skipped.
- If the old file has no vault, your current one is kept. If the vault folder no longer
  exists, the app says so: use **Choose Vault…**.
- Presets from the old `custom-presets.json` are added to yours. A preset with a name you
  already have keeps your version.
- The old files are only read, never changed or moved.

You can also copy the file yourself while the app is quit:

```bash
mkdir -p ~/Library/Application\ Support/Vault\ Wallpaper
cp /path/to/old/config.json ~/Library/Application\ Support/Vault\ Wallpaper/config.json
```

## 3. Stop the web version

Remove it from Plash (or your wallpaper host) and stop the Node process, or both will draw.
If you started it at login with a LaunchAgent or a login item, remove that too.

## Themes: what maps to what

The web version had 16 themes drawn with Canvas2D. The native app has its own themes,
designed to stay still and cost almost nothing (see `docs/plan.md`). Your `theme` value is
kept, so nothing is lost if a web theme is ported later.

| Web theme (`theme`) | In Vault Wallpaper 1.0 beta |
|---|---|
| `default` and its presets | **Default**, the same renderer and presets |
| `celestial` | Drawn with **Default** for now. **Night Sky** is the native star chart: switch to it in Settings. |
| `aurora` | Drawn with **Default** for now. **Aurora Lite** is the native version. |
| `wash`, `sketch` | Drawn with **Default** for now. **Ink & Paper** is the closest native look. |
| `synapse`, `mycelium` | Drawn with **Default** for now. A native Pulse theme (edit-driven flashes) is in progress. |
| `bauhaus`, `led`, `metro`, `phosphor`, `pixel`, `riso`, `stained-glass`, `stitch`, `survey`, `territories` | Drawn with **Default** for now; candidates for later native themes. |

The web themes' presets (Phosphor, LED Board, Risograph and so on) are still listed in
Settings, and they set the theme id plus colours. Natively they currently look like Default
with those colours.

## Other differences

- One bad value no longer rejects the whole file: the app skips that key and uses the rest.
- `vaultPath` can't be changed from the settings page (as on the web); use **Choose Vault…**.
- Settings open in their own window from the menu instead of `http://127.0.0.1:3000/settings`.
- Edit flares and particles from the web default theme aren't drawn; new installs start with
  motion off.
