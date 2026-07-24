import Foundation

/// User-tunable settings persisted in UserDefaults.
@MainActor
final class Preferences {
    static let shared = Preferences()

    private let defaults = UserDefaults.standard
    private enum Key {
        static let watchRoot = "watchRoot"
        static let overlayEnabled = "overlayEnabled"
        static let autoDismiss = "autoDismiss"
        static let autoDismissSeconds = "autoDismissSeconds"
    }

    /// Absolute path of the recursive watch root. Default: `~/archastro`.
    var watchRootPath: String {
        get {
            if let stored = defaults.string(forKey: Key.watchRoot), !stored.isEmpty {
                return (stored as NSString).expandingTildeInPath
            }
            return Self.defaultWatchRoot.path
        }
        set {
            defaults.set(newValue, forKey: Key.watchRoot)
        }
    }

    var watchRootURL: URL {
        URL(fileURLWithPath: watchRootPath, isDirectory: true)
    }

    var overlayEnabled: Bool {
        get {
            if defaults.object(forKey: Key.overlayEnabled) == nil { return true }
            return defaults.bool(forKey: Key.overlayEnabled)
        }
        set { defaults.set(newValue, forKey: Key.overlayEnabled) }
    }

    var autoDismiss: Bool {
        get {
            if defaults.object(forKey: Key.autoDismiss) == nil { return true }
            return defaults.bool(forKey: Key.autoDismiss)
        }
        set { defaults.set(newValue, forKey: Key.autoDismiss) }
    }

    var autoDismissSeconds: Double {
        get {
            let value = defaults.double(forKey: Key.autoDismissSeconds)
            return value > 0 ? value : 5.5
        }
        set { defaults.set(newValue, forKey: Key.autoDismissSeconds) }
    }

    static var defaultWatchRoot: URL {
        let home = FileManager.default.homeDirectoryForCurrentUser
        return home.appendingPathComponent("archastro", isDirectory: true)
    }
}
