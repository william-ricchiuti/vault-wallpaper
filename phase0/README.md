# Phase 0: baseline and platform spike

Run on 2026-11-05: MacBook Pro M3 Pro, built-in 3024×1964 Retina display only, macOS 27.0.1,
Low Power Mode off. Plan: [`docs/plan.md`](../docs/plan.md).

**Verdict: go.** The native window sits exactly where a wallpaper should, it stops all work when
covered, and its memory use is a fraction of the web version's. Two things need tuning later (below).

> **Correction (2026-11-05, later the same day):** the first run recorded the web version at 3.1% CPU
> while covered. Three repeat runs, plus a per-process breakdown over two minutes, show **0.0%** once
> loading and layout finish (about 15 s). The first run's window was visible for part of the
> measurement. WKWebView marks a covered page `hidden`, stops animation frames and throttles timers,
> and the web version already stops drawing when hidden. Covered, the web version's remaining cost is
> memory (196–273 MB), not CPU.

## What was measured and how

- **Web version (today):** the current server from
  [obsidian-live-wallpaper](https://github.com/william-ricchiuti/obsidian-live-wallpaper) (`parser.js`) on port 3999, with the page hosted in
  `webview-host.swift`, a minimal WKWebView window. WKWebView is the engine Plash uses, so this
  stands in for Plash without touching your Plash setup. Numbers include the app and the WebKit helper
  processes it starts (WebContent, GPU, Networking).
- **Native spike:** `spike/`, Rust + wgpu 30 + winit 0.30. A background gradient plus N glowing
  dots drawn by the GPU, with a 30 fps cap and a pause when covered.
- `measure.sh`: average CPU over a 30–40 s window after warm-up (percent of one core) and memory
  footprint at the end (`footprint`, which counts graphics memory too).
- Your real vault (`Willy-AI`): 2,801 notes, 9,036 links. It was only read, never changed.

## Results

| Scenario | Web version | Native spike |
|---|---|---|
| Animating, ~2,500 notes, visible | 7–9% CPU, 470–520 MB | 6.7% CPU, 280 MB |
| Animating, 35,000 notes, visible | 12.8% CPU, 441 MB (draws at most 5,000) | 6.6% CPU, 312 MB (draws all 35,000) |
| Behind your windows (desktop level) | 0.0% CPU after ~15 s of loading, 196–273 MB | **0.0% CPU, 0 frames, 71 MB** |
| Still frame | not available (idles at 10 fps by default) | 0.3% CPU, 67 MB |
| Server process | +46 MB (Node) | none |

Not a like-for-like comparison: the spike draws a much simpler scene than the real renderer (no
links, labels, layout or bloom). Read the native column as a floor, and the web column as today's
full product. CPU figures don't include GPU time.

The real-vault run with a visible window (14.6% CPU, 304 MB) isn't in the table: the window got
covered partway through, so that figure mixes visible and covered time.

## Platform checks

| Check | Result | How |
|---|---|---|
| Window sits above the system wallpaper and below the icons | ✅ | Window-server list (`window-stack.swift`): system Wallpaper at layer −2147483624, spike at −2147483623, Finder icons at −2147483603, desktop widgets at −2147483601 |
| Click-through, on all Spaces, no Dock icon | ✅ set | `ignoresMouseEvents`, `canJoinAllSpaces \| stationary \| ignoresCycle`, accessory activation policy |
| Stops drawing when covered | ✅ | winit's `WindowEvent::Occluded` works on macOS with no extra Objective-C. Covered within ~2 s of launch, then 0 fps and 0 CPU seconds over 25 s |
| Draws when uncovered again | ✅ in code | Redraw is requested on `Occluded(false)`. Needs your eyes; see below |
| Monitor detection | ✅ 1 of 1 | Only one display was attached, so two monitors are **not tested** |
| No memory leak while animating | ✅ | 280 MB flat from 5 s to 60 s |
| Low Power Mode readable | ✅ | `NSProcessInfo.isLowPowerModeEnabled` |

## Needs you (I can't drive the desktop from here)

Run the spike as a wallpaper with no time limit, then try each item below. Quit it with
Ctrl-C in the terminal.

```bash
cd spike && cargo run --release
```

- [ ] Show the desktop (spread four fingers apart, or click the wallpaper on Sonoma+). The purple dots
      appear behind your icons, the icons still work, and the dots animate.
- [ ] Swipe between Spaces: the dots are on every Space.
- [ ] Mission Control and Stage Manager: no glitches, and the dots don't show up as a window.
- [ ] Make an app fullscreen, then leave it: the terminal shows `occluded=true`, then `false`, and the
      dots resume.
- [ ] Sleep and wake: the dots come back.
- [ ] If you have an external monitor: plug it in **before** launching, and the dots appear on both.
      Hot-plugging isn't handled yet.

## Follow-ups found

1. **Graphics memory when animating.** About 180 MB of driver-owned graphics memory appears once frames
   start (it's 67 MB for a still frame). It is fixed, not a leak. Probable causes are wgpu's per-frame
   pools and the swapchain at 6K. Try fewer in-flight frames and reusing the uniform upload, and
   measure again in Phase 3.
2. **CPU while animating is higher than the scene deserves** (6.7% for a trivial scene). Samples show
   it is almost all waiting in the kernel, which suggests frame-pacing overhead rather than real work.
   Try pacing with the display link (`CADisplayLink`) instead of timers in Phase 3.
3. **Fixed: winit's occlusion events can disagree with the real window state.** In your first manual
   run, winit's last event said "visible" while `NSWindow.occlusionState` (which wgpu checks before
   every frame) said covered, for at least 28 seconds (170–198 s). The spike drew nothing and logged a
   refused frame 30 times a second. The spike now reads `occlusionState` itself, pauses when wgpu refuses
   a frame, and re-checks once a second while paused. The real app must do the same and never trust
   the event alone.
4. Not tested here: the file-access prompt for vaults in Documents or iCloud (your vault lives
   elsewhere), iCloud placeholder files, display hot-plug, and Sparkle. They move to the phases where
   they're needed.

## Files

- `webview-host.swift`: WKWebView host used for the web baseline.
- `measure.sh`: CPU and memory sampler.
- `window-stack.swift`: prints back-most windows and their layers.
- `../spike/`: the Rust + wgpu wallpaper spike.
