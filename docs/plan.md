# v2 Native App Plan (Rust + wgpu, macOS first)

Status: Phase 0 done (go), see [`phase0/README.md`](../phase0/README.md). Phase 1 (reader) and
Phase 2 (layout) done, see [Phase 1 results](#phase-1-results) and [Phase 2 results](#phase-2-results).
Phase 3 (renderer foundation) done, see [Phase 3 results](#phase-3-results). Phases 4+ in progress.

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

#### Phase 1 results
Done 2026-11-05 in `crates/core` (`vw_core::vault`).
- Parity with `parser.js`: identical graphs and excerpts on the edge-case fixture, the
  Willy-AI vault and a 35,000-note synthetic vault ([`parity/`](../parity/README.md)).
- Full scan, warm cache: about 30–60 ms for the real vault (2,597 notes) and 0.5–0.9 s for
  35,000 notes. Building the graph takes 3.4 ms and 45 ms.
- Save to new graph: 110–140 ms on small vaults, 170–300 ms on 35,000 notes. The web version
  waits at least 700 ms by design (300 ms write-settle plus 400 ms debounce).
- Lessons: macOS reports a moved folder by its new name only, so the reader re-checks which
  notes still exist after any folder event, and does a full rescan when FSEvents says
  events were dropped.

### Phase 2: Layout (M)
**You get:** the graph has the familiar shape, opens instantly and doesn't wobble at every launch.
- Force layout matching today's parameters, plus rings, timeline and galaxy.
- Position cache, incremental placement of new notes, deterministic seeding.
- **Done when:** a side-by-side screenshot against the current version looks like the same graph,
  a 35k-note layout converges on a background thread without dropping frames, and a relaunch shows the
  settled graph in the first frame.

#### Phase 2 results
Done 2026-11-06 in `crates/core` (`vw_core::layout`).
- A port of the web version's d3 setup: link springs, Barnes-Hut repulsion, centring,
  collision and tag clustering, with the same size tiers, plus the rings, timeline and galaxy
  target layouts. Deterministic: the same vault always gives the same picture.
- Side by side with d3 on the real vault, the shapes match (images in
  [`docs/phase2/`](phase2/)). Same number of ticks to settle; 2.7–4× faster per tick, and
  about 5× on 35,000 notes (3.7 s versus 19.7 s).
- Runs on its own thread at utility priority (efficiency cores), publishes moving frames at a
  set rate and always a final settled frame.
- Positions are cached. A relaunch shows the settled layout in 3 ms for the real vault and
  42 ms for 35,000 notes, with no layout work.
- Change from the web version, for lightness: after an edit, only changed notes and their
  direct neighbours move; everything else stays pinned. One new note relaxes in 72 ms on the
  real vault (550 ms at 35,000) and the rest of the graph moves 0 px. A big change (over a
  fifth of the graph) still relaxes everything.
- Possible later: at 35,000 notes the local relax rebuilds the whole quadtree every tick;
  building it once for the pinned nodes would make edits there much cheaper.

### Phase 3: Renderer foundation (M)
**You get:** a working native wallpaper with the plain look, already lighter than today.
- Instanced nodes and edges, background, camera and framing (`framing`, `zoom`, offsets, `safeArea`),
  the frame scheduler, the occlusion and power behaviour from Phase 0.
- An off-screen render mode that writes PNGs without a window, used for golden-image tests and
  "Export PNG".
- **Done when:** the "Plain" preset runs on your vault as your wallpaper for a full day with no
  crashes, and idle numbers beat the Phase 0 baseline clearly.

#### Phase 3 results
Done 2026-11-06: `crates/render` (`vw-render`) and the app, `crates/app` (`vault-wallpaper`).
- The renderer draws the background, links as anti-aliased quads and notes as anti-aliased
  discs, all instanced. Framing is a port of the web version's (centre, fill, left, right,
  none, zoom, offsets, safe area, small-vault scale cap). It also renders off-screen to PNG,
  for export and image tests.
- The app: a wallpaper window per display; the vault scan, watcher and layout engine on
  background threads. It **draws only when something changed** and sleeps otherwise; a
  hidden screen with pending changes re-checks visibility once a second.
- Real vault (2,604 notes), visible window: first run settles in 1.7 s; a relaunch from the
  layout cache is settled and drawn 0.3 s after launch. Memory 58–85 MB in one process,
  versus 300–520 MB plus a 46 MB Node server for the web version. About 0.2% CPU once
  settled.
- Soak test, 30 minutes as the real wallpaper with live vault edits: memory 57–85 MB (it rises
  on a redraw and drops back; no leak), 3.3 s CPU in total (about 0.18%), 8 redraws, no
  crashes.
- Not yet: display hot-plug.

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

#### Phase 4 results
Done 2026-11-06 (`crates/render`: `look.rs`, `scene.rs`, `text.rs`, `post.wgsl`).
- Layered renderer: the scene (gradient, halos, links, notes, labels) and a quarter-resolution
  blurred glow layer are drawn only when something changes. Full Retina scene in ~19 ms.
- All 18 default-renderer presets of the real vault, rendered natively and by the web version
  (each with its own server config), match closely: backgrounds, palettes, colour modes, glow,
  light themes, label badges. Contact sheets (web left, native right): `docs/phase4/presets-*.png`.
- **Breathing without frames.** Redrawing frames for glow breathing cost +216 MB driver
  memory and ~2.5% app CPU at any frame rate. The glow now lives in a quarter-resolution
  Core Animation layer whose opacity the window server animates: app memory stays ~96 MB and
  the app draws nothing while it breathes. The window server does some work (noisy to measure);
  stepping the opacity from the app at 8 fps measured worse. New installs start with motion
  off; breathing is opt-in.
- Not ported: note flares (edit highlights) and particles; they come back as event-driven
  effects in the Phase 6 Pulse theme. The web default on a dense vault is a purple haze of
  overlapping halos in both versions: faithful, and a reason for the new themes.

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

#### Phase 5 and 6 progress (2026-11-06)
- App shell: menu-bar icon (pause, settings, choose vault, export PNG, open config, start at
  login, quit), a vault folder picker on first run, and the settings window (`settings.html`
  in an on-demand wry web view, answered in-process over `vw://app`, no port).
- Themes, with a framework (style + scene builder + window-server motion): **Night Sky**,
  **Aurora Lite** and **Ink & Paper** (still, with an edit-triggered 2 s ink bleed). All four
  looks on the real vault: `docs/phase6/all-themes.png`. Themed scenes render in ~35 ms.
- Framing now fits the linked core and pushes unlinked notes to an outer ring, so dense
  vaults fill the screen.
- **Pulse** (`pulse`): a still graph in slate grey with a hint of tag hue per note (not the
  default look's colours, which turn a big vault into a purple haze), fine links, faint glow
  on hubs only. Saving a note opens a ring on it, sends pulses along its drawn links to up
  to 20 neighbours (which glint), then dimmer pulses one hop further (up to 28), and is
  still again after 2.6 s. `motionMode: still` (and Low Power Mode, pause): the edit just
  redraws. Images: `docs/phase6/pulse.png`, `pulse-burst.png` (off-screen, hub at 30 %),
  `pulse-live.png` (the running app, 1.3 s after a save).
- **Bursts are baked, not presented.** A theme describes a burst as a few actors (a sprite
  image or a short strip of frames, plus position and opacity keys); `BurstRenderer::bake`
  draws them all into one small atlas in one pass (~1 ms), and the app turns each actor
  into a small Core Animation layer with keyframe animations (position, opacity,
  `contentsRect` frames), capped at the burst fps. The app presents no frames and wakes
  only when a burst ends. Atlases: Pulse ~1.2 MB on the fixture, ~2.2 MB for a hub on
  the real vault (98 actors at 1 px/pt); ink bleed ~2.2 MB (25 frames at 2 px/pt).
- **Burst cost, measured 2026-11-06** (fixture vault, one save, `footprint` and `ps`
  sampled around the edit, two runs each, scripts in the Pulse session notes):

  | | Peak | Time at peak | 3 s after | Settled | App CPU per edit |
  |---|---|---|---|---|---|
  | Ink bleed, before (24 presented frames) | 293 MB | ~3.2 s | 290 MB | 102 MB | 0.06–0.08 s |
  | Ink bleed, baked | 313–318 MB | ~1.4 s | 113 MB | 113 MB | 0.02–0.03 s |
  | Pulse, baked | 328 MB | ~1.4 s | 115 MB | 115 MB | 0.02–0.05 s |
  | Edit with `motionMode: still` (no burst) | 292–316 MB | ~1.2 s | 109–209 MB | 101–138 MB | 0.02–0.03 s |

  WindowServer CPU per burst: ~0.1–0.2 s before and after (the 0.1 s resolution of `ps`).
  Six saves in a row stay flat at ~115 MB (no leak).
- **What the measurements showed.** The ~294 MB was never the burst layer: any GPU work,
  even one scene redraw, dirties ~210 MB of Metal driver memory ("owned physical footprint
  (graphics)"), which the driver gives back ~1.1 s after the GPU goes idle. Presenting
  burst frames kept the GPU busy, so the spike lasted the whole burst; a smaller layer or
  fewer drawables would not have helped (the old layer was already ~150 px). Baking does
  its GPU work inside the redraw's own window, so the spike is no longer than a still
  edit's. Left over: the baked path settles ~10 MB higher than before (Malloc Large and
  Core Animation; dropping the bake pipelines after each burst did not change it).
- Possible later: the redraw spike itself (~210 MB for ~1 s on every edit, in every
  theme), probably the driver's working set after drawing the full-resolution scene.
- Still to do: a second look at the settings window with real typing.

#### App robustness (2026-11-06)
New modules in `crates/app`: `system.rs` (notification observers), `power.rs` (IOKit power
source + motion policy), `displays.rs` (display list, diff, span framing). Nothing polls:
every change arrives as a notification on the main queue. The old once-a-minute Low Power
Mode poll while breathing is gone (`NSProcessInfoPowerStateDidChangeNotification` instead).

| Behaviour | How it works | Verified |
|---|---|---|
| Display added / removed / resized / rescaled | `NSApplicationDidChangeScreenParametersNotification` → diff winit's monitors by `CGDirectDisplayID` → add, drop or move+resize windows (surface, glow overlay, burst layer) | Unit tests for the diff. On the real vault, a helper (`CGConfigureDisplayWithDisplayMode`, `.forAppOnly`, so macOS restores the mode on exit) switched 1512×982@2× → 1352×845@2× → back, and → 1920×1200@1× → back. Each logged `display change: Update(..)`; screenshots show the graph re-fitted full-screen at the new size and scale. **Not exercised:** plugging or unplugging a second monitor (only one attached); the add/remove paths run only in unit tests. |
| Battery | `IOPSNotificationCreateRunLoopSource` on the main run loop; `IOPSGetProvidingPowerSourceType == "Battery Power"`. Off mains and not Showcase → no breathing, no edit bursts | Policy unit tests; source read at startup (`on battery false`). **Not exercised:** an actual unplug, or toggling Low Power Mode (needs `sudo pmset`). |
| Sleep, lock, fast user switching | `NSWorkspaceScreensDidSleep/WillSleep/SessionDidResignActive` and `com.apple.screenIsLocked` → *away*: no draws, bursts stopped, breathing removed. `ScreensDidWake/DidWake/SessionDidBecomeActive` and `screenIsUnlocked` → back: re-read power, re-sync displays, re-derive the look (recency colours age) and redraw only if it changed, re-check visibility. A 60 s visibility check while away covers a missed wake notification | `pmset displaysleepnow`, 15 s, `caffeinate -u`: log shows `away`, `visible=false`, `back`, `visible=true`; 0.03 s CPU over the 15 s asleep; screenshot after wake shows the right picture. **Not exercised:** screen lock (would need William's password to undo) and fast user switching. |
| Spaces / Mission Control | `NSWorkspaceActiveSpaceDidChangeNotification` → re-check visibility | Desktop-level run: Ctrl-→ to a desktop Space logged `space changed`, wallpaper behind icons and widgets (screenshot); back to a full-screen app Space logged `visible=false`. Mission Control: the wallpaper fills every Space thumbnail and is not listed as a window. **Not checked:** Stage Manager, leaving a full-screen app by hand, icon clicks (needs a person). |
| Span mode | `VW_SPAN=1`: frame the union of all displays (points), then shift each screen's framing by its offset in the union (exact: the framing is linear after the outer ring). Used for scenes, burst sprites and Export PNG | Unit tests: slices tile the whole-span picture, single display equals the plain fit. Run with `VW_SPAN=1` logs `span true` and draws normally. **Not exercised** with two displays. No config key yet. |

Cost, same `phase0/measure.sh` run (25 s warm-up, 30 s window, real vault, default config,
mains power), before → after: `--front` 0.2% CPU, 97 MB → 0.2%, 97 MB; desktop 0.2%, 97 MB →
0.2%, 96 MB. No change.

#### Live checks (2026-11-07, `main` at c6b53f2, display awake)
- Edit bursts (Pulse, Ink & Paper): play with no frames presented, ~0.05 s CPU each.
- Obsidian plugin (simulated `state.json`): a reported save triggers a burst within ~0.1 s; the
  active note gets its label and highlight (window screenshots before and after).
- Launch with the display asleep: 0 screens at start; on wake the app adds the display and
  draws the right picture within ~5 s (82 MB).
- Finding: **any full redraw** (edit, config change, theme change) briefly spikes process memory
  to ~316 MB for about a second, then it settles around 150 MB. It is the graphics driver's
  working memory for a full Retina scene redraw, not the bursts. Worth trimming (lower-resolution
  internal passes, fewer full-size intermediate textures, reusing targets).

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

### Phase 9: Obsidian plugin (S–M, optional) — built: `obsidian-plugin/`, `vw_core::presence`
**You get:** the wallpaper knows what you are doing in Obsidian.
- A small plugin that writes a state file inside the vault (`.obsidian/plugins/vault-wallpaper/state.json`)
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

From 2026-11-06 William asked for the remaining phases to run without stopping for approval;
remaining open choices take the recommended default below. Anything that publishes or uses his
credentials (notarization, making the repo public, releases) still waits for him.

1. **Repo.** ✅ Decided 2026-11-05: separate repos. This repo (`vault-wallpaper`) is the native app;
   `obsidian-live-wallpaper` stays the web version and keeps getting targeted improvements.
2. **Settings UI.** ✅ Decided 2026-11-05: reuse `settings.html` in an on-demand web view (option A), its
   `/api/*` calls answered inside the app. Can be swapped for SwiftUI later.
3. **Launch themes.** ✅ Direction decided 2026-11-05: themes designed from scratch for lightness (see
   [Theme concepts](#theme-concepts-lightness-first)). Default set taken 2026-11-06 for prototyping:
   Breathing Glow (the rebuilt default), Night Sky, Pulse, Aurora Lite and Ink & Paper (the light one),
   for variety across glow, calm, activity, colour and light. Any that miss the budget are swapped.
4. **Minimum macOS.** ✅ Decided 2026-11-05: macOS 13 Ventura and up, universal binary (Apple Silicon +
   Intel). README says "tested on macOS 26–27".
5. **Name and bundle id.** ✅ Default taken 2026-11-06: "Vault Wallpaper", bundle id
   `com.williamricchiuti.vaultwallpaper`. No "Obsidian" in the app name.
6. **Distribution.** ✅ Default taken 2026-11-06: signed and notarized DMG on GitHub Releases plus a
   Homebrew cask in William's own tap. Mac App Store later, if ever.
   Updated 2026-11-08: the app is free on all three: the DMG with Sparkle, Homebrew, and the
   Mac App Store (sandboxed, no updater; see `packaging/appstore/README.md`).
7. **The npm package.** ✅ Decided 2026-11-05: it stays with the web version, which is still maintained.

## Rough total

Phases 0–8 (a polished v2.0 with the default renderer, all 18 presets and about 5 themes): very roughly
2–3 months of part-time work with agents doing most of the typing. Phase 0 is short and tells us early
whether anything on the platform side is harder than expected.
