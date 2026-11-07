// swift display-modes.swift list | swift display-modes.swift switch <seconds> [lodpi]
// Switches the main display to another mode for <seconds>, then exits; .forAppOnly
// means macOS restores the original mode when this process exits.
import CoreGraphics
import Foundation
let d = CGMainDisplayID()
let opts = [kCGDisplayShowDuplicateLowResolutionModes: kCFBooleanTrue] as CFDictionary
let modes = (CGDisplayCopyAllDisplayModes(d, opts) as? [CGDisplayMode]) ?? []
let cur = CGDisplayCopyDisplayMode(d)!
print("current \(cur.width)x\(cur.height) px \(cur.pixelWidth)x\(cur.pixelHeight)")
if CommandLine.arguments.count > 1 && CommandLine.arguments[1] == "list" {
  for m in modes where m.isUsableForDesktopGUI() { print("\(m.width)x\(m.height) px \(m.pixelWidth)x\(m.pixelHeight) \(m.refreshRate)") }
  exit(0)
}
let secs = Double(CommandLine.arguments.count > 2 ? CommandLine.arguments[2] : "6")!
// A smaller HiDPI mode with the same aspect, if there is one.
let lodpi = CommandLine.arguments.count > 3 && CommandLine.arguments[3] == "lodpi"
guard let target = modes.filter({ $0.isUsableForDesktopGUI() && (lodpi ? $0.pixelWidth == $0.width : ($0.pixelWidth > $0.width && $0.width < cur.width)) })
  .max(by: { lodpi ? $0.width > $1.width : $0.width < $1.width }) else { print("no other mode"); exit(1) }
var cfg: CGDisplayConfigRef?
CGBeginDisplayConfiguration(&cfg)
CGConfigureDisplayWithDisplayMode(cfg, d, target, nil)
let r = CGCompleteDisplayConfiguration(cfg, .forAppOnly)
print("switched to \(target.width)x\(target.height) (err \(r.rawValue)) for \(secs)s")
Thread.sleep(forTimeInterval: secs)
print("exiting; macOS restores the mode")
