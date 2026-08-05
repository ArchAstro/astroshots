import Foundation

/// User-tunable settings persisted in UserDefaults.
@MainActor
final class Preferences {
    static let shared = Preferences()

    private let defaults: UserDefaults
    private enum Key {
        static let watchRoots = "watchRoots"
        /// Retained as a migration and downgrade path for pre-multi-root builds.
        static let watchRoot = "watchRoot"
        static let overlayEnabled = "overlayEnabled"
        static let autoDismiss = "autoDismiss"
        static let autoDismissSeconds = "autoDismissSeconds"
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    /// Whether the user has ever chosen watch folders (or upgraded from a
    /// build that stored a single `watchRoot`).
    ///
    /// Fresh installs leave this false so launch can prompt for directories
    /// instead of silently watching a guessed path.
    var hasConfiguredWatchRoots: Bool {
        defaults.object(forKey: Key.watchRoots) != nil
            || defaults.object(forKey: Key.watchRoot) != nil
    }

    /// Absolute paths of the recursive watch roots.
    ///
    /// Empty until first-run (or Settings) configures folders — there is no
    /// automatic default root. Reads the former singular preference when
    /// necessary so upgrades retain the directory the user already chose.
    var watchRootPaths: [String] {
        get {
            if defaults.object(forKey: Key.watchRoots) != nil {
                let stored = defaults.stringArray(forKey: Key.watchRoots) ?? []
                return Self.normalizeWatchRootPaths(stored)
            }

            if let legacy = defaults.string(forKey: Key.watchRoot), !legacy.isEmpty {
                return Self.normalizeWatchRootPaths([legacy])
            }

            return []
        }
        set {
            let normalized = Self.normalizeWatchRootPaths(newValue)
            defaults.set(normalized, forKey: Key.watchRoots)
            if let first = normalized.first {
                defaults.set(first, forKey: Key.watchRoot)
            } else {
                defaults.removeObject(forKey: Key.watchRoot)
            }
        }
    }

    var watchRootURLs: [URL] {
        watchRootPaths.map {
            URL(fileURLWithPath: $0, isDirectory: true)
        }
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

    /// Where the watch-folder open panel should start browsing.
    ///
    /// Not a watch root. Prefers `~/Projects` when it exists; otherwise home
    /// so the panel never opens on a missing path.
    static var folderPickerStartURL: URL {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let projects = home.appendingPathComponent("Projects", isDirectory: true)
        if FileManager.default.fileExists(atPath: projects.path) {
            return projects
        }
        return home
    }

    /// Preserve selection order while dropping duplicates and roots already
    /// covered recursively by another selected root.
    static func normalizeWatchRootPaths(_ paths: [String]) -> [String] {
        var result: [String] = []

        for rawPath in paths where !rawPath.isEmpty {
            let expanded = (rawPath as NSString).expandingTildeInPath
            let candidate = URL(
                fileURLWithPath: expanded,
                isDirectory: true
            )
            .standardizedFileURL
            .resolvingSymlinksInPath()
            .path

            if result.contains(where: { contains(root: $0, path: candidate) }) {
                continue
            }
            result.removeAll { contains(root: candidate, path: $0) }
            result.append(candidate)
        }

        return result
    }

    static func isPath(_ path: String, coveredBy roots: [String]) -> Bool {
        let normalizedPath = URL(fileURLWithPath: path)
            .standardizedFileURL
            .resolvingSymlinksInPath()
            .path
        return roots.contains {
            contains(root: $0, path: normalizedPath)
        }
    }

    private static func contains(root: String, path: String) -> Bool {
        guard root != path else { return true }
        let prefix = root == "/" ? "/" : root + "/"
        return path.hasPrefix(prefix)
    }
}
