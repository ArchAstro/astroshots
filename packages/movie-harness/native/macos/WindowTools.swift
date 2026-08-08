/**
 * Window inventory + Screen Recording TCC helpers for desktop.window.
 *
 * Usage:
 *   swift WindowTools.swift list
 *   swift WindowTools.swift screen-access          # preflight only (JSON)
 *   swift WindowTools.swift screen-access --request  # may show system prompt
 */
import AppKit
import CoreGraphics
import Foundation

struct WindowRow: Encodable {
  let id: Int
  let pid: Int
  let owner: String
  let title: String
  let bundleId: String?
  let width: Int
  let height: Int
  let x: Int
  let y: Int
  let onScreen: Bool
  /// CGWindow layer (0 = normal). Popovers / floating chrome often use > 0.
  let layer: Int
}

struct ScreenAccessReport: Encodable {
  let granted: Bool
  /// True when we invoked CGRequestScreenCaptureAccess (may have shown a prompt).
  let requested: Bool
  let hostApp: String
  let hostBundleId: String?
  let settingsHint: String
}

func listWindows() -> [WindowRow] {
  // Include off-screen windows so agents can still target them.
  let opts = CGWindowListOption(arrayLiteral: .excludeDesktopElements)
  guard let info = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]]
  else {
    return []
  }

  var bundleByPid: [Int32: String] = [:]
  for app in NSWorkspace.shared.runningApplications {
    if let bid = app.bundleIdentifier {
      bundleByPid[app.processIdentifier] = bid
    }
  }

  var rows: [WindowRow] = []
  for w in info {
    let layer = w[kCGWindowLayer as String] as? Int ?? -1
    // Layer 0 = normal windows. Menu-bar trays / NSPopover often sit above 0
    // (floating ~3, modal panel ~8). Skip Dock / main menu / status chrome
    // (typically ≥20) which are huge, blank under screencapture, or unusable.
    guard layer >= 0, layer <= 15 else { continue }

    let id = w[kCGWindowNumber as String] as? Int ?? 0
    let pid = w[kCGWindowOwnerPID as String] as? Int ?? 0
    let owner = w[kCGWindowOwnerName as String] as? String ?? ""
    let title = w[kCGWindowName as String] as? String ?? ""
    let onScreen = w[kCGWindowIsOnscreen as String] as? Bool ?? false
    let bounds = w[kCGWindowBounds as String] as? [String: Any]
    let width = (bounds?["Width"] as? NSNumber)?.intValue ?? 0
    let height = (bounds?["Height"] as? NSNumber)?.intValue ?? 0
    let x = (bounds?["X"] as? NSNumber)?.intValue ?? 0
    let y = (bounds?["Y"] as? NSNumber)?.intValue ?? 0
    // Popovers can be small; keep a low floor but drop 1×1 placeholders.
    if width < 2 || height < 2 { continue }

    rows.append(
      WindowRow(
        id: id,
        pid: pid,
        owner: owner,
        title: title,
        bundleId: bundleByPid[Int32(pid)],
        width: width,
        height: height,
        x: x,
        y: y,
        onScreen: onScreen,
        layer: layer
      )
    )
  }

  // Prefer larger + on-screen + lower layer so normal windows win over
  // tiny floating helpers when multiple match a bundle id.
  return rows.sorted {
    let area0 = $0.width * $0.height
    let area1 = $1.width * $1.height
    if area0 != area1 { return area0 > area1 }
    if $0.onScreen != $1.onScreen { return $0.onScreen && !$1.onScreen }
    return $0.layer < $1.layer
  }
}

func hostIdentity() -> (name: String, bundleId: String?) {
  let app = NSRunningApplication.current
  let name =
    app.localizedName
    ?? ProcessInfo.processInfo.processName
  return (name, app.bundleIdentifier)
}

func screenAccess(request: Bool) -> ScreenAccessReport {
  let host = hostIdentity()
  var granted = CGPreflightScreenCaptureAccess()
  var didRequest = false
  if request && !granted {
    // May show the system consent dialog (not always; Settings may still be required).
    granted = CGRequestScreenCaptureAccess()
    didRequest = true
  }
  return ScreenAccessReport(
    granted: granted,
    requested: didRequest,
    hostApp: host.name,
    hostBundleId: host.bundleId,
    settingsHint:
      "System Settings → Privacy & Security → Screen Recording → enable \(host.name), then quit & reopen it"
  )
}

func emitJSON<T: Encodable>(_ value: T) {
  let encoder = JSONEncoder()
  encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
  let data = try! encoder.encode(value)
  FileHandle.standardOutput.write(data)
  fputs("\n", stdout)
}

let args = Array(CommandLine.arguments.dropFirst())
let command = args.first ?? "list"

if command == "list" || command == "--json" {
  emitJSON(listWindows())
  exit(0)
}

if command == "screen-access" {
  let request = args.contains("--request") || args.contains("-r")
  let report = screenAccess(request: request)
  emitJSON(report)
  // Exit 0 when granted, 2 when denied (easy for shells).
  exit(report.granted ? 0 : 2)
}

fputs(
  """
  usage:
    WindowTools.swift list
    WindowTools.swift screen-access [--request]

  list          JSON windows for desktop.window matching
  screen-access Screen Recording TCC preflight (optional --request prompt)

  """,
  stderr
)
exit(2)
