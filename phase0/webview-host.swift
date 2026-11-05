// Phase 0 baseline harness: hosts a URL in a WKWebView the way Plash does, so the
// current web version can be measured without touching the user's Plash setup.
//
//   swiftc -O webview-host.swift -o webview-host
//   ./webview-host <url> <front|desktop> <seconds>
//
// front   = normal window covering the screen (guaranteed visible, measures render cost)
// desktop = desktop-level window behind the icons (what a wallpaper host does)
// Prints occlusion changes so the measurement script knows whether it was visible.
import AppKit
import WebKit

enum Opts {
  static let args = CommandLine.arguments
  static let url = URL(string: args.count > 1 ? args[1] : "")
  static let seconds = Double(args.count > 3 ? args[3] : "") ?? 0
  static let desktop = args.count > 2 && args[2] == "desktop"
}
guard Opts.url != nil, Opts.seconds > 0 else {
  FileHandle.standardError.write("usage: webview-host <url> <front|desktop> <seconds>\n".data(using: .utf8)!)
  exit(2)
}

final class Delegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
  var window: NSWindow!
  func applicationDidFinishLaunching(_ note: Notification) {
    let frame = NSScreen.main!.frame
    window = NSWindow(contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false)
    if Opts.desktop {
      window.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopWindow)))
      window.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
      window.ignoresMouseEvents = true
    } else {
      window.level = .normal
    }
    window.delegate = self
    let web = WKWebView(frame: frame, configuration: WKWebViewConfiguration())
    window.contentView = web
    web.load(URLRequest(url: Opts.url!))
    window.orderFrontRegardless()
    print("pid=\(getpid()) mode=\(Opts.desktop ? "desktop" : "front")"); fflush(stdout)
    DispatchQueue.main.asyncAfter(deadline: .now() + Opts.seconds) { NSApp.terminate(nil) }
  }
  func windowDidChangeOcclusionState(_ note: Notification) {
    let visible = window.occlusionState.contains(.visible)
    print("occlusion visible=\(visible) t=\(Date().timeIntervalSince1970)"); fflush(stdout)
  }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = Delegate()
app.delegate = delegate
app.run()
