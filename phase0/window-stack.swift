// Prints the bottom of the window stack (back-most windows) with their layer, so we can
// check that the wallpaper window sits above the system wallpaper and below the icons.
import CoreGraphics
let opts: CGWindowListOption = [.optionAll]
let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as! [[String: Any]]
for w in list {
  let layer = w[kCGWindowLayer as String] as? Int ?? 0
  guard layer < 0 else { continue }
  let owner = w[kCGWindowOwnerName as String] as? String ?? "?"
  let name = w[kCGWindowName as String] as? String ?? ""
  let onscreen = w[kCGWindowIsOnscreen as String] as? Bool ?? false
  let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
  print("layer=\(layer) onscreen=\(onscreen) owner=\(owner) name=\(name) w=\(b["Width"] ?? "?")")
}
