import CryptoKit
import Foundation

/// Installs the app's bundled Tools payload (skills + CLI) onto the user's
/// machine. Offline and in-process: no network, npm or subprocess.
///
/// Layout contract (see `macos/scripts/tools-payload.py`):
/// `<payload>/{manifest.json, bin/astroshot, skills/<name>/SKILL.md, ...}`.
///
/// Components install independently and each one is atomic: a failure restores
/// that component's previous state and is reported without stopping the others.
///   - skills, one component per detected agent home
///   - the `astroshot` shim under `<appSupport>/bin`
///   - a managed PATH entry for the login shell (fish, zsh, bash)
///
/// Files the installer did not write are never replaced. Ownership is a marker:
/// `.astroshots-managed.json` inside a skill directory, `astroshots-managed` in
/// the first lines of a whole-file artifact, and a delimited block in shared rc
/// files. Reinstalling an unchanged payload writes nothing.
struct ToolsInstaller: Sendable {
    // MARK: Model

    struct Harness: Sendable, Equatable {
        let id: String
        let name: String
        /// Agent home, relative to the user's home directory.
        let home: String
        /// Skills directory, relative to the agent home.
        let skills: String
    }

    /// Agent homes that receive skills when they already exist.
    static let harnesses: [Harness] = [
        Harness(id: "claude", name: "Claude Code", home: ".claude", skills: "skills"),
        Harness(id: "codex", name: "Codex", home: ".codex", skills: "skills"),
        Harness(id: "cursor", name: "Cursor", home: ".cursor", skills: "skills"),
        Harness(id: "rovo", name: "Rovo Dev", home: ".rovodev", skills: "skills"),
        Harness(id: "grok", name: "Grok", home: ".grok", skills: "skills"),
        Harness(id: "opencode", name: "OpenCode", home: ".config/opencode", skills: "skills"),
        Harness(id: "pi", name: "Pi", home: ".pi/agent", skills: "skills"),
        Harness(id: "agents", name: ".agents", home: ".agents", skills: "skills"),
    ]

    enum Component: Sendable, Equatable {
        case skills(harness: String)
        case cliShim
        case shellPath
    }

    enum Outcome: Sendable, Equatable {
        case installed
        case unchanged
        /// A file we do not own is in the way; it was left untouched.
        case userOwned(path: String)
        /// Nothing was written; the user finishes setup by hand.
        case manual(steps: [String])
        /// The component was rolled back to its previous state.
        case failed(reason: String)
    }

    struct ComponentResult: Sendable, Equatable {
        let component: Component
        let outcome: Outcome
        /// Skill names left alone because a user-owned entry occupies them.
        let skippedUserOwned: [String]
    }

    struct Report: Sendable, Equatable {
        let results: [ComponentResult]
        func result(for component: Component) -> ComponentResult? {
            results.first { $0.component == component }
        }
    }

    enum PayloadError: Error, Equatable {
        case invalid(String)
    }

    // MARK: Configuration

    let payload: URL
    let home: URL
    let appSupport: URL
    /// Value of `$SHELL` (path or name). Nil or unrecognised means manual setup.
    let loginShell: String?

    /// `home` and `appSupport` are explicit so callers (and tests) choose the
    /// filesystem this touches; nothing here reads the process environment.
    init(payload: URL, home: URL, appSupport: URL? = nil, loginShell: String?) {
        self.payload = payload
        self.home = home
        self.appSupport = appSupport
            ?? home.appendingPathComponent("Library/Application Support/Astroshots")
        self.loginShell = loginShell
    }

    var binDirectory: URL { appSupport.appendingPathComponent("bin") }

    // MARK: Install

    func install() throws -> Report {
        let skills = try loadPayloadSkills()
        var results: [ComponentResult] = []

        for harness in Self.harnesses {
            let agentHome = home.appendingPathComponent(harness.home)
            guard Self.isDirectory(agentHome) else { continue }
            let skillsDir = agentHome.appendingPathComponent(harness.skills)
            results.append(run(.skills(harness: harness.id)) {
                try installSkills(skills, into: skillsDir)
            })
        }
        results.append(run(.cliShim) { try installShim() })
        results.append(run(.shellPath) { try installShellPath() })
        return Report(results: results)
    }

    private func run(
        _ component: Component,
        _ body: () throws -> (Outcome, [String])
    ) -> ComponentResult {
        do {
            let (outcome, skipped) = try body()
            return ComponentResult(component: component, outcome: outcome, skippedUserOwned: skipped)
        } catch {
            return ComponentResult(
                component: component,
                outcome: .failed(reason: String(describing: error)),
                skippedUserOwned: []
            )
        }
    }

    // MARK: Payload

    private struct PayloadSkill {
        let name: String
        let source: URL
        /// Digest of this skill's manifest entries; changes when any file does.
        let digest: String
    }

    private struct Manifest: Decodable {
        struct Entry: Decodable { let sha256: String }
        let format: Int
        let files: [String: Entry]
    }

    private func loadPayloadSkills() throws -> [PayloadSkill] {
        let fm = FileManager.default
        guard let manifestData = fm.contents(atPath: payload.appendingPathComponent("manifest.json").path),
              let manifest = try? JSONDecoder().decode(Manifest.self, from: manifestData),
              manifest.format == 1
        else { throw PayloadError.invalid("missing or unsupported manifest.json") }
        guard fm.isExecutableFile(atPath: payload.appendingPathComponent("bin/astroshot").path) else {
            throw PayloadError.invalid("missing bin/astroshot")
        }
        let skillsRoot = payload.appendingPathComponent("skills")
        let names = ((try? fm.contentsOfDirectory(atPath: skillsRoot.path)) ?? [])
            .filter { !$0.hasPrefix(".") }
            .sorted()
        var skills: [PayloadSkill] = []
        for name in names {
            let source = skillsRoot.appendingPathComponent(name)
            guard fm.fileExists(atPath: source.appendingPathComponent("SKILL.md").path) else { continue }
            let prefix = "skills/\(name)/"
            let entries = manifest.files
                .filter { $0.key.hasPrefix(prefix) }
                .sorted { $0.key < $1.key }
                .map { "\($0.key):\($0.value.sha256)" }
            guard !entries.isEmpty else {
                throw PayloadError.invalid("manifest does not list skill \(name)")
            }
            let digest = SHA256.hash(data: Data(entries.joined(separator: "\n").utf8))
                .map { String(format: "%02x", $0) }.joined()
            skills.append(PayloadSkill(name: name, source: source, digest: digest))
        }
        guard !skills.isEmpty else { throw PayloadError.invalid("payload has no skills") }
        return skills
    }

    // MARK: Skills

    private static let skillMarker = ".astroshots-managed.json"

    private struct SkillMarker: Codable { let digest: String }

    private func installSkills(
        _ skills: [PayloadSkill],
        into skillsDir: URL
    ) throws -> (Outcome, [String]) {
        let fm = FileManager.default
        var journal = Journal()
        var changed = false
        var skipped: [String] = []
        do {
            if !Self.exists(skillsDir) {
                try fm.createDirectory(at: skillsDir, withIntermediateDirectories: true)
                journal.onRollback { try? fm.removeItem(at: skillsDir) }
                changed = true
            } else if !Self.isDirectory(skillsDir) {
                return (.userOwned(path: skillsDir.path), [])
            }
            for skill in skills {
                let dest = skillsDir.appendingPathComponent(skill.name)
                let existing = Self.exists(dest)
                if existing {
                    guard Self.isPlainDirectory(dest), let marker = Self.readSkillMarker(dest) else {
                        skipped.append(skill.name)
                        continue
                    }
                    if marker.digest == skill.digest { continue }
                }
                let token = UUID().uuidString
                let staged = skillsDir.appendingPathComponent(".\(skill.name).astroshots-staging-\(token)")
                defer { try? fm.removeItem(at: staged) }
                try fm.copyItem(at: skill.source, to: staged)
                try JSONEncoder().encode(SkillMarker(digest: skill.digest))
                    .write(to: staged.appendingPathComponent(Self.skillMarker))
                if existing {
                    let backup = skillsDir.appendingPathComponent(".\(skill.name).astroshots-backup-\(token)")
                    try fm.moveItem(at: dest, to: backup)
                    journal.onRollback {
                        try? fm.removeItem(at: dest)
                        try? fm.moveItem(at: backup, to: dest)
                    }
                    journal.onCommit { try? fm.removeItem(at: backup) }
                }
                try fm.moveItem(at: staged, to: dest)
                journal.onRollback { try? fm.removeItem(at: dest) }
                changed = true
            }
        } catch {
            journal.rollback()
            throw error
        }
        journal.commit()
        return (changed ? .installed : .unchanged, skipped)
    }

    private static func readSkillMarker(_ dir: URL) -> SkillMarker? {
        guard let data = FileManager.default.contents(atPath: dir.appendingPathComponent(skillMarker).path)
        else { return nil }
        return try? JSONDecoder().decode(SkillMarker.self, from: data)
    }

    // MARK: CLI shim

    private static let ownershipLine = "# astroshots-managed: written by the Astroshots app; safe to delete."

    private func installShim() throws -> (Outcome, [String]) {
        let target = payload.appendingPathComponent("bin/astroshot").path
        let content = """
        #!/bin/sh
        \(Self.ownershipLine)
        exec \(Self.shSingleQuote(target)) "$@"

        """
        let url = binDirectory.appendingPathComponent("astroshot")
        return (try writeOwnedFile(url, content: content, mode: 0o755), [])
    }

    // MARK: Shell PATH

    private static let blockBegin = "# >>> astroshots managed (do not edit) >>>"
    private static let blockEnd = "# <<< astroshots managed <<<"

    private enum Shell { case fish, zsh, bash }

    private func detectedShell() -> Shell? {
        switch loginShell.map({ ($0 as NSString).lastPathComponent }) {
        case "fish": return .fish
        case "zsh": return .zsh
        case "bash": return .bash
        default: return nil
        }
    }

    private func installShellPath() throws -> (Outcome, [String]) {
        guard let shell = detectedShell() else {
            return (.manual(steps: manualSteps()), [])
        }
        switch shell {
        case .fish:
            let url = home.appendingPathComponent(".config/fish/conf.d/astroshots.fish")
            let dir = Self.fishQuote(binDirectory.path)
            let content = """
            \(Self.ownershipLine)
            contains -- \(dir) $PATH; or set -gx PATH \(dir) $PATH

            """
            return (try writeOwnedFile(url, content: content, mode: 0o644), [])
        case .zsh:
            return (try updateBlock(in: home.appendingPathComponent(".zshenv")), [])
        case .bash:
            return (try updateBlock(in: bashProfile()), [])
        }
    }

    /// Bash login shells read the first of these that exists.
    private func bashProfile() -> URL {
        let candidates = [".bash_profile", ".bash_login", ".profile"].map { home.appendingPathComponent($0) }
        return candidates.first(where: Self.exists) ?? candidates[0]
    }

    private func manualSteps() -> [String] {
        let dir = binDirectory.path
        return [
            "Add this directory to your PATH in your shell's startup file: \(dir)",
            "POSIX shells: export PATH=\(Self.shSingleQuote(dir)):\"$PATH\"",
            "Then open a new terminal and run: astroshot --help",
        ]
    }

    private func pathBlock() -> String {
        let dir = Self.shDoubleQuote(binDirectory.path)
        return """
        \(Self.blockBegin)
        case ":$PATH:" in *":"\(dir)":"*) ;; *) export PATH=\(dir)":$PATH" ;; esac
        \(Self.blockEnd)
        """
    }

    /// Insert or refresh the delimited PATH block, leaving every other byte of
    /// the file alone. Symlinked rc files (dotfile managers) are written through.
    private func updateBlock(in rc: URL) throws -> Outcome {
        let fm = FileManager.default
        let file = rc.resolvingSymlinksInPath()
        let original: String?
        if fm.fileExists(atPath: file.path) {
            guard let data = fm.contents(atPath: file.path),
                  let text = String(data: data, encoding: .utf8)
            else { return .userOwned(path: rc.path) }
            original = text
        } else {
            original = nil
        }
        let block = pathBlock()
        let updated: String
        if let text = original, let begin = text.range(of: Self.blockBegin) {
            guard let end = text.range(of: Self.blockEnd, range: begin.upperBound..<text.endIndex) else {
                return .userOwned(path: rc.path)  // half-edited block: do not guess
            }
            updated = text.replacingCharacters(in: begin.lowerBound..<end.upperBound, with: block)
        } else if let text = original, !text.isEmpty {
            let separator = text.hasSuffix("\n") ? "\n" : "\n\n"
            updated = text + separator + block + "\n"
        } else {
            updated = block + "\n"
        }
        if updated == original { return .unchanged }

        let mode = (try? fm.attributesOfItem(atPath: file.path)[.posixPermissions]) as? NSNumber
        try fm.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(updated.utf8).write(to: file, options: .atomic)
        if let mode { try? fm.setAttributes([.posixPermissions: mode], ofItemAtPath: file.path) }
        return .installed
    }

    // MARK: Whole-file artifacts

    /// Atomically write a file this installer owns; refuse to replace one it does not.
    private func writeOwnedFile(_ url: URL, content: String, mode: Int) throws -> Outcome {
        let fm = FileManager.default
        if Self.exists(url) {
            guard Self.isPlainFile(url),
                  let existing = fm.contents(atPath: url.path).flatMap({ String(data: $0, encoding: .utf8) }),
                  existing.split(separator: "\n", maxSplits: 3).prefix(2).contains(Substring(Self.ownershipLine))
            else { return .userOwned(path: url.path) }
            if existing == content { return .unchanged }
        }
        try fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(content.utf8).write(to: url, options: .atomic)
        try fm.setAttributes([.posixPermissions: mode], ofItemAtPath: url.path)
        return .installed
    }

    // MARK: Helpers

    /// Undo log for one component. Rollback runs newest-first.
    private struct Journal {
        private var undo: [() -> Void] = []
        private var finalize: [() -> Void] = []
        mutating func onRollback(_ action: @escaping () -> Void) { undo.append(action) }
        mutating func onCommit(_ action: @escaping () -> Void) { finalize.append(action) }
        func rollback() { undo.reversed().forEach { $0() } }
        func commit() { finalize.forEach { $0() } }
    }

    /// Existence without following a final symlink (a dangling link still counts).
    private static func exists(_ url: URL) -> Bool {
        (try? FileManager.default.attributesOfItem(atPath: url.path)) != nil
    }

    /// Follows symlinks: a symlinked agent home is still a home.
    private static func isDirectory(_ url: URL) -> Bool {
        var isDir: ObjCBool = false
        return FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir) && isDir.boolValue
    }

    private static func fileType(_ url: URL) -> FileAttributeType? {
        (try? FileManager.default.attributesOfItem(atPath: url.path))?[.type] as? FileAttributeType
    }

    private static func isPlainDirectory(_ url: URL) -> Bool { fileType(url) == .typeDirectory }
    private static func isPlainFile(_ url: URL) -> Bool { fileType(url) == .typeRegular }

    private static func shSingleQuote(_ s: String) -> String {
        "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    private static func shDoubleQuote(_ s: String) -> String {
        var out = ""
        for ch in s {
            if "\\\"$`".contains(ch) { out.append("\\") }
            out.append(ch)
        }
        return "\"" + out + "\""
    }

    private static func fishQuote(_ s: String) -> String {
        "'" + s.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "'", with: "\\'") + "'"
    }
}
