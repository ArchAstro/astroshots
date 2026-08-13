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
        /// Explicit gate for the first-run watch-folder startup sequence.
        static let hasCompletedFirstRunSetup = "hasCompletedFirstRunSetup"
        /// Bumps when first-run migration rules change.
        static let firstRunMigrationVersion = "firstRunMigrationVersion"
        static let overlayEnabled = "overlayEnabled"
        static let autoDismiss = "autoDismiss"
        static let autoDismissSeconds = "autoDismissSeconds"
        /// Stable `worktreePath::slug` identities hidden from the friction-log UX.
        static let hiddenFrictionLogIDs = "hiddenFrictionLogIDs"
        /// Opt-in friction-log narrated videos (MLX Audio + Qwen3-TTS).
        static let narrationEnabled = "narrationEnabled"
        /// Cached flag that the Qwen3 model finished downloading at least once.
        static let narrationModelReady = "narrationModelReady"
        /// Named Qwen3 CustomVoice speaker used for previews and every render.
        static let narrationVoice = "narrationVoice"
    }

    /// Migration schema for first-run detection. Increment when adoption rules
    /// for upgrades change.
    private static let currentFirstRunMigrationVersion = 1

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        migrateFirstRunStateIfNeeded()
    }

    /// True after the user finishes first-run folder setup (or after an upgrade
    /// migration that already had watch roots).
    ///
    /// This is the source of truth for “should we run the first-run startup
    /// sequence,” not the absence of watch roots alone.
    var hasCompletedFirstRunSetup: Bool {
        get { defaults.bool(forKey: Key.hasCompletedFirstRunSetup) }
        set { defaults.set(newValue, forKey: Key.hasCompletedFirstRunSetup) }
    }

    /// Open the tray + folder panel on launch only when first-run is incomplete.
    var shouldPresentFirstRunStartup: Bool {
        !hasCompletedFirstRunSetup
    }

    /// Whether any watch-root preference key has been written.
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

    /// Friction logs hidden from the app without removing their files on disk.
    var hiddenFrictionLogIDs: [String] {
        get {
            Self.normalizeFrictionLogIDs(
                defaults.stringArray(forKey: Key.hiddenFrictionLogIDs) ?? []
            )
        }
        set {
            defaults.set(
                Self.normalizeFrictionLogIDs(newValue),
                forKey: Key.hiddenFrictionLogIDs
            )
        }
    }

    /// When true, Settings enables the MLX narration pipeline (download + render).
    var narrationEnabled: Bool {
        get { defaults.bool(forKey: Key.narrationEnabled) }
        set { defaults.set(newValue, forKey: Key.narrationEnabled) }
    }

    /// True after a successful model download (used to restore Ready quickly).
    var narrationModelReady: Bool {
        get { defaults.bool(forKey: Key.narrationModelReady) }
        set { defaults.set(newValue, forKey: Key.narrationModelReady) }
    }

    var narrationVoice: String {
        get { NarrationVoice.normalized(defaults.string(forKey: Key.narrationVoice)) }
        set { defaults.set(NarrationVoice.normalized(newValue), forKey: Key.narrationVoice) }
    }

    /// Call when the user successfully chooses initial watch folders.
    func markFirstRunSetupComplete() {
        hasCompletedFirstRunSetup = true
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

    static func normalizeFrictionLogIDs(_ ids: [String]) -> [String] {
        Array(Set(ids.filter { !$0.isEmpty })).sorted()
    }

    // MARK: - First-run detection

    /// Upgrades that already had watch roots must not see first-run UI.
    /// Fresh installs keep `hasCompletedFirstRunSetup == false` until the user
    /// finishes the folder panel (or equivalent Settings path).
    private func migrateFirstRunStateIfNeeded() {
        let version = defaults.integer(forKey: Key.firstRunMigrationVersion)
        guard version < Self.currentFirstRunMigrationVersion else { return }

        if !watchRootPaths.isEmpty {
            // Prior install that already chose or persisted roots.
            hasCompletedFirstRunSetup = true
        }
        // Else leave incomplete so launch presents first-run startup.

        defaults.set(
            Self.currentFirstRunMigrationVersion,
            forKey: Key.firstRunMigrationVersion
        )
    }

    private static func contains(root: String, path: String) -> Bool {
        guard root != path else { return true }
        let prefix = root == "/" ? "/" : root + "/"
        return path.hasPrefix(prefix)
    }
}
