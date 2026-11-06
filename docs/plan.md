# v2 Native App Plan (Rust + wgpu, macOS first)

Status: Phase 0 done (go), see [`phase0/README.md`](../phase0/README.md). Phases 1+ not started.

File names such as `parser.js`, `layout-core.js`, `settings.html`, `scripts/smoke.js` and `docs/presets/`
refer to the web version, which lives on in
[obsidian-live-wallpaper](https://github.com/william-ricchiuti/obsidian-live-wallpaper).

## The goal in one paragraph

Replace "Node script + local server on a port + third-party wallpaper app + web page" with one signed
Mac app. You open it, pick your vault, and it becomes your wallpaper. It reads the vault directly,
draws with the graphics chip, remembers the graph's shape between launches, and stops working entirely
when nothing on screen can see it. Settings live behind a menu-bar icon. It updates itself.
Windows and Linux follow later from the same code.

## Lightness first (decided 2026-11-05)

The app should be as light on the computer as possible while still looking great. This rule outranks
feature parity with the web version: a look that can't meet the budget gets redesigned or dropped.
Themes may be designed from scratch for this, not ported.

**Rules every theme and feature follows:**
1. **Still is the default state.** A visible wallpaper with nothing new to show draws no frames.
   Motion is the exception, and it should be slow and designed to look good at low frame rates
   (drifts, crossfades, breathing), not smooth 60 fps movement.
2. **Draw the expensive part once.** The graph, glow and textures are drawn into cached images when
   something changes (vault edit, config change, screen change). An animated frame only recombines
   those cached images, ideally one cheap full-screen pass with a few texture reads per pixel, such
   as fading glow intensity or slowly rotating a cached star field.
3. **Soft things at low resolution.** Glow, fog, nebula and aurora are drawn at a quarter or a
   sixteenth of screen resolution and scaled up, since they're blurry anyway. Only crisp elements
   (node cores, lines, text) use full resolution.
4. **Motion comes from events, not a clock.** A vault edit can trigger a short burst (a pulse, a
   bloom, an ink bleed) that ends and returns to still. Clock-based change (time-of-day tint, a slow
   drift) updates rarely: about once a minute, or a few frames per second at most.
5. **No per-frame CPU work.** Per-frame changes are a few numbers sent to the GPU. Layout, parsing and
   geometry happen only when the vault changes.
6. **Power aware.** On battery or in Low Power Mode, themes fall back to a still frame, or to their
   lowest motion setting if the user opts in.
7. **Measured, not assumed.** Every theme has a budget, checked automatically with off-screen renders:
   idle cost 0, an animated frame under ~1 ms of GPU time at full Retina resolution, and the whole app
   under ~100 MB.

## What stays, what goes, what changes

| Today | v2 |
|---|---|
| Node 20 + `npx` | Nothing to install except the app |
| HTTP server on `127.0.0.1:3000`, SSE, `/api/*` | Gone. Everything is inside one process |
| chokidar watcher | macOS FSEvents (via the `notify` crate) |
| Plash / Lively / Hidamari host | The app's own window sits behind the desktop icons |
| `parser.js` (1,400 lines) | Rust port with the same rules, proven identical by tests |
| D3 force sim in a web worker | Rust layout engine on a background thread, positions cached to disk |
| Canvas2D + one WebGL path | wgpu (Metal on Mac): GPU instancing, HDR, real bloom |
| `settings.html` served over HTTP | Same UI (adapted) in a settings window that opens on demand, no port |
| `config.json` next to the script | `~/Library/Application Support/Obsidian Live Wallpaper/config.json`, old file imported on first run |
| 16 themes + 18 presets of the default renderer | Default renderer first (unlocks all 18 presets), then themes in tiers |

Config keys that disappear: `port`, `refreshMs`, `renderer`, `fastHash`. Every other key keeps its name
and meaning, so existing configs and custom presets import as they are.

## How the app is put together

Plain version: five parts, each with one job, talking inside the same program.

```
 Vault folder ──▶ [1 Reader] ──notes, links, tags──▶ [2 Layout] ──positions──▶ [3 Renderer] ──▶ wallpaper window(s)
                       ▲                                   ▲                         ▲
 Obsidian plugin ──────┘ (optional, later)                 │                         │
                                                  [4 App shell: menu bar, settings, power, screens]
                                                           │
                                                   [5 Config store]
```

1. **Reader** (`core/vault`)
   - Walks the vault on launch in parallel (`rayon`), then watches it with `notify` (FSEvents), debounced.
   - Port of today's rules: `[[target]]`, `[[target|alias]]`, `[[target#heading]]`; `![[embed]]` of
     non-note files ignored; frontmatter tags (flow and block lists) and inline `#tags`; duplicate
     basenames get path-qualified ids; `ignorePaths`; hidden folders skipped; unresolved links become
     ghost nodes; file times for age/recency; short excerpt for the "note of the hour" overlay.
   - Output: an immutable `Graph` snapshot, swapped in atomically when something changes.

2. **Layout** (`core/layout`)
   - Port of the d3-force setup in use today: Barnes-Hut repulsion, link springs, collision, centring,
     tag clustering. Plus the deterministic layouts in `layout-core.js`: rings, timeline, galaxy.
   - Runs on its own thread and publishes positions over a channel. It never blocks drawing.
   - Seeded, so the same vault gives the same picture. Positions are saved to a cache file keyed by note id.
     On launch the graph appears already settled. When a note is added, it is placed next to its
     neighbours and only that region relaxes. The renderer tweens the movement.
   - Target: vaults of 35k notes / 120k links, the same target as `docs/perf-roadmap.md`.

3. **Renderer** (`render/`)
   - wgpu on Metal. Nodes and edges are GPU instance buffers, so a 120k-edge vault is one draw call,
     not 120k.
   - Layers:
     1. Background: gradient and noise shader.
     2. Static graph: edges, nodes, halos drawn into an off-screen HDR texture. Redrawn only when
        layout, camera or config changes.
     3. Live layer: particles, pulses, note flares, breathing glow (driven by a timer value, so the
        static layer isn't redrawn).
     4. Post: bloom, chromatic bloom, depth of field, grain, vignette, tone mapping.
     5. Text: labels and overlays (clock, stats, note of the hour) with `glyphon`.
   - Frame scheduler: draws only when something changed or an animation is running, caps at `maxFps`
     (ProMotion screens would otherwise run 120 fps), and drops to `idleFps` or 0.

4. **App shell** (`app/`)
   - One borderless window per display, placed at the desktop level behind the icons, ignoring mouse
     clicks, present on every Space, no Dock icon (`LSUIElement`). Uses `winit` for windows and
     `objc2-app-kit` for the Mac-specific window settings.
   - Pauses rendering when the window is covered (`NSWindow.occlusionState`): fullscreen apps,
     another Space, screen locked, display asleep.
   - Power: obeys Low Power Mode, on-battery and thermal state with a lower fps cap or a still frame.
   - Display changes: monitor plugged in or out, resolution changes, "span across monitors" as one
     layout over all screens.
   - Menu bar (`tray-icon` + `muda`): pick theme or preset, pause, open settings, export PNG, quit.
   - Start at login via `SMAppService` (macOS 13+).
   - Optional: set the system wallpaper to a matching still frame, so the lock screen and Mission
     Control match.

5. **Config store** (`core/config`)
   - The same JSON schema and validators as `parser.js` (`VALIDATORS`), ported with `serde`.
   - Changes made in settings apply to the live wallpaper immediately. The wallpaper is the preview.
   - Hand-edits to the file are picked up live, as today.

Settings window (decision 2 below): my recommendation is to reuse `settings.html` in a native web view
(`wry`) that exists only while the window is open, with its `/api/*` calls replaced by in-process
messages. That keeps the UI already built and costs nothing when closed.

## Phases

Each phase lists what you get in plain terms, the work, and what "done" means. Sizes are rough:
S ≈ a few days, M ≈ 1–2 weeks, L ≈ 2–4 weeks of focused part-time work with agents.

### Phase 0: Measure and spike (S). Go/no-go gate
**You get:** proof that the hard platform parts work before any real investment, plus real numbers
for the current version.
- Measure the current app on your vault and a synthetic 35k-note vault: memory (all processes,
  including Plash's web view), CPU at idle and animating, Energy Impact in Activity Monitor,
  `powermetrics` GPU/package power, time to first frame.
- Spike a Rust app that draws a wgpu gradient in a desktop-level window behind the icons, on all Spaces,
  on two monitors, and pauses when covered.
- Check how it behaves with Stage Manager, Mission Control, "click wallpaper to reveal desktop"
  (Sonoma+), and fullscreen apps.
- **Done when:** the spike behaves like a real wallpaper in all of the above, and the baseline numbers
  are written down in `docs/`.

### Phase 1: Reader (M)
**You get:** the app understands your vault exactly like the current version does.
- Cargo workspace: `core` (vault, layout, config), `render`, `app`.
- Port the parsing rules and the change watcher.
- **Parity tests:** run today's `parser.js` over the fixture vaults in `scripts/smoke.js` plus a copy
  of your vault, save its `graph.json`, and require the Rust reader to produce the same nodes, links
  and tags. This is the main guard against subtle regressions.
- **Done when:** parity tests pass, a full scan of a 35k-note vault takes under a second warm, and an
  edit shows up in the graph snapshot within ~200 ms.

### Phase 2: Layout (M)
**You get:** the graph has the familiar shape, opens instantly and doesn't wobble at every launch.
- Force layout matching today's parameters, plus rings, timeline and galaxy.
- Position cache, incremental placement of new notes, deterministic seeding.
- **Done when:** a side-by-side screenshot against the current version looks like the same graph,
  a 35k-note layout converges on a background thread without dropping frames, and a relaunch shows the
  settled graph in the first frame.

### Phase 3: Renderer foundation (M)
**You get:** a working native wallpaper with the plain look, already lighter than today.
- Instanced nodes and edges, background, camera and framing (`framing`, `zoom`, offsets, `safeArea`),
  the frame scheduler, the occlusion and power behaviour from Phase 0.
- An off-screen render mode that writes PNGs without a window, used for golden-image tests and
  "Export PNG".
- **Done when:** the "Plain" preset runs on your vault as your wallpaper for a full day with no
  crashes, and idle numbers beat the Phase 0 baseline clearly.

### Phase 4: Default look at full quality (L)
**You get:** everything the default renderer does today, looking better. This unlocks all 18
default-renderer presets at once (Ambient, Neon, Blueprint, Parchment, ...), since presets are just
settings.
- Glow and bloom, chromatic bloom, cluster halos, edge colouring and styles (line, curve), particles
  and ambient particles, note flares, glow breathing, depth of field, depth parallax, labels (hubs,
  recent, styles, fonts), node colour modes (tag, age, folder, recency), palettes, tag shapes,
  time-of-day tint, auto light/dark theme, grain, vignette, focus mode (`focusNote`, `focusHops`),
  overlays, theme rotation, large-vault scaling tiers.
- Golden-image tests: each preset rendered off-screen and compared with a saved reference, so later
  changes can't quietly break a look.
- **Done when:** all 18 presets render, each passes a side-by-side review with today's screenshots in
  `docs/presets/`, and Neon/Ambient stay smooth on a 35k-note vault.

### Phase 5: App shell (M)
**You get:** it behaves like a finished Mac app.
- Menu-bar icon and menu, settings window, first-run flow (pick vault with a folder picker, import an
  existing `config.json` and custom presets), start at login, multi-monitor and span mode, Export PNG,
  a "Pause wallpaper" toggle.
- **Done when:** a fresh Mac user can go from download to working wallpaper without reading the README.

### Phase 6: Launch themes (M–L)
**You get:** about five themes designed from scratch around the lightness rules, rather than
straight ports of the web themes. Web themes are a source of ideas, not a spec.
- Shortlist the concepts in [Theme concepts](#theme-concepts-lightness-first), prototype the
  favourites in the spike, measure each against the budget, and keep the ones that look great *and*
  cost almost nothing.
- Each theme is a "look": a set of shaders plus parameters on top of the shared graph data, so adding
  one later doesn't touch the core.
- **Done when:** each launch theme meets its budget in the automated check, has a golden-image test,
  and you'd keep it on your own desktop.

### Theme concepts (lightness first)

Starting ideas, each designed to cost nothing when idle and very little when moving:

| Concept | Idea | What moves, and what it costs |
|---|---|---|
| **Long Exposure** | The graph as a long-exposure photo: links are light trails, hubs glow | Nothing, by default. An edit adds one new trail that fades in |
| **Night Sky** | Notes as stars with diffraction spikes; tags as constellations | The cached sky rotates very slowly (a few fps); the rotation is one texture read per pixel |
| **Breathing Glow** | The classic glowing graph, done right | Glow intensity fades up and down on the cached bloom image, which is nearly free |
| **Pulse** | A still graph; your recent edits send light along their links | Only the small pulse sprites; cost follows your activity and is zero when you're not editing |
| **Aurora Lite** | Slow curtains of colour over tag clusters | Curtains drawn at 1/16 resolution at a few fps, then scaled up |
| **Ink & Paper** | A printed, light-background graph (riso / ink feel) | Nothing; an edit triggers a two-second ink bleed |
| **Atlas** | Tags as territories with borders, hubs as capitals (a map) | Nothing; redrawn only when the vault changes |
| **Stained Glass** | Notes as glass panes lit from behind | Optional slow light sweep across the cached image |

Default renderer: Phase 4 rebuilds it under the same rules. Options that can't meet the budget
(constant particle streams, full-rate depth of field) are reworked or dropped, and presets map onto
what remains.

### Phase 7: Packaging and release (S–M)
**You get:** a download that opens with no warnings and keeps itself up to date.
- Universal binary (Apple Silicon + Intel), app bundle and `Info.plist`, icon.
- Signing with your Developer ID, hardened runtime, notarization (`notarytool`), stapling, DMG.
- Auto-update: Sparkle 2 is the standard. Calling it from Rust needs a small spike; I'm not certain of
  the cleanest integration yet. Fallback: check GitHub Releases and offer the new DMG.
- Release workflow on GitHub Actions macOS runners (free for public repos). Signing secrets stored in
  the repo. Nothing is triggered without your OK.
- Homebrew cask in your own tap first.
- **Done when:** a clean Mac (or a new user account) installs from the DMG with no Gatekeeper warning,
  and an update from 2.0.0 to 2.0.1 works end to end.

### Phase 8: Beta and migration (S)
**You get:** existing users move over without friction.
- A few weeks of beta on your own machine plus a couple of testers.
- README for this repo; a pointer from the web version's README to the native app. The web version
  keeps its own repo and npm package and is still improved (decided 2026-11-05).
- **Done when:** 1.0.0 is released and web-version users can find it.

### Phase 9: Obsidian plugin (S–M, optional)
**You get:** the wallpaper knows what you are doing in Obsidian.
- A small plugin that writes a state file inside the vault (`.obsidian/plugins/live-wallpaper/state.json`)
  that the app already watches. No port. Contents: the note you have open, plus Obsidian's own resolved
  links when available.
- Features: highlight the open note and its neighbourhood, pulse on save, exact link resolution.
- The app keeps working with Obsidian closed using its own reader.

### Later
- The remaining themes in tiers. The GPU makes several easier (Riso, LED Board, Stained Glass via jump
  flooding), others are real work (Survey contours, Territories, Metro routing, Ink Wash, Mycelium).
- Windows: same core and renderer (Direct3D 12 via wgpu), window placed behind the icons with the
  WorkerW technique Lively uses.
- Linux: KDE and wlroots compositors via layer-shell, X11 via the desktop window type. GNOME stays hard
  because it doesn't support layer-shell.

## Performance budget (targets, to check against Phase 0 numbers)

| Situation | Target |
|---|---|
| Covered by windows or fullscreen app | 0 frames drawn, ~0% CPU |
| Visible, nothing animating | Still frame, ~0% CPU |
| Animating (any theme, 5k-note vault, Apple Silicon) | Under 1 ms GPU per frame at full Retina resolution, ~1% CPU, "Low" Energy Impact |
| Vault edit burst (pulse, bloom, bleed) | Back to a still frame within a few seconds |
| Memory (5k notes) | Under ~100 MB for the whole app |
| Launch to settled picture (cached layout) | Under 1 second |
| 35k notes / 120k links, Neon preset | Steady 30 fps |

These are goals, not measurements. Phase 0 supplies the real "before" numbers.

## Testing

- **Parity tests** (Phase 1): the Rust reader against `parser.js` output on the same vaults.
- **Unit tests:** config validation (port the cases in `scripts/smoke.js`), layout determinism,
  incremental updates.
- **Golden-image tests:** every preset and theme rendered off-screen to PNG and compared within a
  tolerance. These replace `smoke-render.js`.
- **Soak test:** a day as the real wallpaper with memory sampled every minute, to catch leaks.
- **Platform checklist** (manual, each release): Spaces, Stage Manager, Mission Control, fullscreen
  apps, two monitors, plug/unplug, sleep/wake, lock screen, Low Power Mode, light/dark switch.

## Risks and unknowns

- **Desktop-level window details.** The exact window level that sits below the desktop icons but above
  the system wallpaper, and how it interacts with Sonoma's "click wallpaper to reveal desktop", must
  be confirmed in the Phase 0 spike. I'm confident it can be done (Plash does it) but not of the exact
  settings.
- **File access prompts.** Vaults in Documents, Desktop or iCloud Drive (Obsidian's iCloud vaults live
  under `~/Library/Mobile Documents/iCloud~md~obsidian/`) will probably trigger a macOS permission
  prompt the first time. Picking the vault with the system folder picker should keep this to one
  clear moment. Verify in Phase 0.
- **iCloud placeholders.** Files that iCloud hasn't downloaded yet can't be read. The reader should
  skip them and pick them up when they arrive, not stall.
- **Look fidelity.** Some themes rely on Canvas2D quirks (hand-drawn wobble, paper textures). Rebuilt
  versions will look different, ideally better. Side-by-side reviews are the check.
- **Sparkle from Rust.** Needs a spike. A fallback exists.
- **Scope creep.** 34 looks is a lot. The phase gates are there so v2.0 can ship with the default
  renderer plus about 5 themes.

## Decisions

1. **Repo.** ✅ Decided 2026-11-05: separate repos. This repo (`vault-wallpaper`) is the native app;
   `obsidian-live-wallpaper` stays the web version and keeps getting targeted improvements.
2. **Settings UI.** ✅ Decided 2026-11-05: reuse `settings.html` in an on-demand web view (option A), its
   `/api/*` calls answered inside the app. Can be swapped for SwiftUI later.
3. **Launch themes.** ✅ Direction decided 2026-11-05: themes designed from scratch for lightness (see
   [Theme concepts](#theme-concepts-lightness-first)). The exact five are picked after prototyping.
4. **Minimum macOS.** ✅ Decided 2026-11-05: macOS 13 Ventura and up, universal binary (Apple Silicon +
   Intel). README says "tested on macOS 26–27".
5. **Name and bundle id.** For example "Obsidian Live Wallpaper" / `com.williamricchiuti.olw`. Using
   "Obsidian" in an app name may need a check against Obsidian's brand guidelines before release.
6. **Distribution.** DMG + Homebrew first (recommended), Mac App Store later. The App Store requires
   sandboxing, which changes how vault access works.
7. **The npm package.** ✅ Decided 2026-11-05: it stays with the web version, which is still maintained.

## Rough total

Phases 0–8 (a polished v2.0 with the default renderer, all 18 presets and about 5 themes): very roughly
2–3 months of part-time work with agents doing most of the typing. Phase 0 is short and tells us early
whether anything on the platform side is harder than expected.
