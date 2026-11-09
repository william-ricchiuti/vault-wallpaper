# App Store listing draft

Paste into App Store Connect > Vault Wallpaper > macOS App > 1.0. Limits are Apple's; counts
are for this text. Everything marked **TODO** needs William.

## App information

| Field | Value |
|---|---|
| Name (30) | **Vault Wallpaper** (15). If taken: "Vault Wallpaper: Note Graph" (27) or "Vault Graph Wallpaper" (21) |
| Subtitle (30) | **Your notes, as a living graph** (29) |
| Bundle ID | `com.williamricchiuti.vaultwallpaper` |
| SKU | `vaultwallpaper-mac` |
| Primary category | Productivity |
| Secondary category | Utilities |
| Price | Free |
| Age rating | 4+ (every question answered None/No) |
| Copyright | 2026 William Ricchiuti |
| Support URL | `https://github.com/william-ricchiuti/vault-wallpaper/issues` |
| Marketing URL (optional) | `https://github.com/william-ricchiuti/vault-wallpaper` |
| Privacy policy URL (required) | `https://github.com/william-ricchiuti/vault-wallpaper/blob/main/PRIVACY.md` (the text below, in [`PRIVACY.md`](../../PRIVACY.md)) |

Don't use "Obsidian" in the name, subtitle or keywords: it's Obsidian's trademark
(guideline 5.2.1; 2.3.7 for keywords). Saying the app works with Obsidian vaults in the
description is fine.

## Promotional text (170)

> Turn your Markdown notes into a calm, living desktop. Every note is a point, every link a
> line, and the picture changes as you write.

(140 characters. Promotional text can be changed at any time without a review.)

## Description (4000)

> Vault Wallpaper draws the graph of your notes behind your desktop icons. Every note is a
> point, every link between notes is a line, and the picture updates as you write: save a
> note and the wallpaper follows.
>
> It works with any folder of Markdown notes and understands Obsidian vaults: wikilinks,
> Markdown links, tags and folders.
>
> MADE TO BE LEFT ON
> • Still by default. A wallpaper with nothing new to show draws nothing at all, so it costs
>   no battery while you work.
> • Motion is opt-in and gentle: slow drifts and breathing glow, designed for low frame
>   rates.
> • Motion pauses in Low Power Mode and on battery, and everything stops while the screen
>   is covered, asleep or locked.
> • It remembers the graph's shape, so it's back in a fraction of a second after a restart.
>
> LOOKS
> • Night Sky: your notes as a field of stars, with the busiest notes shining brightest.
> • Aurora: soft curtains of light drifting over the graph.
> • Ink & Paper: a pen drawing on warm paper; saving a note bleeds a little ink.
> • Pulse: light travels along the links from the note you just saved.
> • Default: the classic glowing graph, coloured by tag, folder or age.
> Colours, sizes, labels, clustering by tag and which folders to leave out are all in
> Settings, with presets to start from.
>
> PRIVATE BY DESIGN
> • Your notes never leave your Mac. The app has no account, no analytics and makes no
>   network connections.
> • It only reads the folder you choose, and never changes your notes.
>
> FITS IN
> • Lives in the menu bar, with no Dock icon.
> • Every display, every Space, behind your icons; clicks go straight through to the
>   desktop.
> • Export the current picture as a PNG.
> • Optional companion Obsidian plugin: highlight the note you have open and react the moment
>   you save.
> • Start at Login, if you like.
>
> Vault Wallpaper is free and has no in-app purchases.
>
> Obsidian is a trademark of Dynalist Inc. Vault Wallpaper is an independent app and is not
> affiliated with or endorsed by Obsidian.

(About 1,900 characters.) **Check before submitting:** that the Obsidian plugin is released
(else drop that bullet), and the trademark owner's name on obsidian.md.

## Keywords (100)

```
notes,graph,wallpaper,markdown,desktop,knowledge,zettelkasten,links,second brain,live,backlinks
```

(95 characters. Commas without spaces; don't repeat words from the name or category.)

## What's New (first version)

> First release.

## Screenshots

Mac screenshots must be 16:10 at 1280×800, 1440×900, 2560×1600 or 2880×1800; 1 to 10 of them.
These five are 2880×1800 JPEGs in `screenshots/`, rendered with `render_vault` from William's
real vault (2,825 notes, read only), in this order:

1. `1-night-sky.jpg`: Night Sky
2. `2-aurora-lite.jpg`: Aurora
3. `3-ink-paper.jpg`: Ink & Paper
4. `4-pulse-burst.jpg`: Pulse, mid-way through the light that runs out from a saved note
5. `5-default.jpg`: the default look

They show the wallpaper alone. No note names are drawn (labels are off by default), so
nothing private is in them. Regenerate:

```bash
cargo build --release -p vw-render --example render_vault
target/release/examples/render_vault /path/to/vault out.png --theme night-sky --size 2880 1800 2
# --burst 0.45 adds Pulse's edit burst; then convert:
sips -s format jpeg -s formatOptions 92 out.png --out 1-night-sky.jpg
```

Stronger later (follow-up): the same pictures on a real desktop, with the menu bar, a few
icons and the open menu, plus one of the Settings window. Those need a screen capture on
William's Mac.

## App privacy ("nutrition label")

App Store Connect > App Privacy > Get Started:

- "Do you or your third-party partners collect data from this app?" **No, we do not collect
  data from this app.** → the page shows **Data Not Collected**.
- No tracking; the app links no third-party SDKs that collect anything.

This matches `PrivacyInfo.xcprivacy` (no tracking, no collected data types).

## Privacy policy text

> **Vault Wallpaper privacy policy**
>
> Vault Wallpaper does not collect, store or share any personal data. It reads the notes
> folder you choose, on your Mac, to draw your wallpaper, and keeps its settings and a cache of
> the graph's layout on your Mac. Nothing is sent anywhere: the app has no account, no
> analytics, no advertising and makes no network connections. Exported pictures are saved
> where you can see them, in Pictures > Vault Wallpaper.
>
> Questions: open an issue at https://github.com/william-ricchiuti/vault-wallpaper/issues.

The full policy is [`PRIVACY.md`](../../PRIVACY.md); keep the two in step.

## App Review information

- Sign-in required: **No**.
- Contact: **TODO** William's name, phone and email.
- Attachment (optional): a short screen recording of the first run helps; **TODO** if wanted.
- Notes (4000):

> Vault Wallpaper turns a folder of Markdown notes (for example an Obsidian vault) into the
> desktop wallpaper: each note is a dot and each link between notes a line, drawn with Metal.
>
> HOW TO TEST
> 1. Open the app. It has no Dock icon and no main window; it lives in the menu bar (an icon of
>    three linked dots). On first launch an Open panel asks for the notes folder.
> 2. Pick any folder containing .md files. A small sample vault: **TODO link to a zip of a
>    demo vault** (or use any folder of Markdown files). The graph appears behind the
>    desktop icons within a few seconds.
> 3. Menu bar icon > Settings… changes the look (try the theme tiles); the wallpaper updates
>    live. Export PNG saves a picture to Pictures > Vault Wallpaper.
> 4. Edit and save a note in the folder: the wallpaper updates.
>
> WHY IT DRAWS BEHIND THE DESKTOP
> It is a live wallpaper. Its windows sit at the desktop window level (below the icons), on
> every Space, ignore mouse clicks so the desktop works normally, and stay out of Mission
> Control. It draws nothing when nothing changes and stops when the screen is covered or
> asleep.
>
> ENTITLEMENTS
> • User-selected files, read-only: to read the notes folder the user picks (and, optionally,
>   an old settings file to import). The app never writes to it.
> • App-scoped bookmarks: to keep access to that folder after a restart.
> • Pictures folder: Export PNG saves into Pictures > Vault Wallpaper.
> • Outgoing network connections: required by WKWebView in the sandbox to display the Settings
>   window, whose page is bundled in the app and served in-process. The app itself makes no
>   network connections.
>
> The app is free, has no account, collects no data and contains no in-app purchases.
