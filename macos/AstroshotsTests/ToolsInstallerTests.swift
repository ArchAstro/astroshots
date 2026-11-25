import Foundation
import Testing
@testable import Astroshots

/// Canonical proof for the managed tools install: real filesystem, isolated
/// HOME. Nothing here touches the developer's agent homes or shell rc files.
struct ToolsInstallerTests {
    typealias Component = ToolsInstaller.Component

    // MARK: Sandbox

    /// Temp HOME plus a fake sealed payload (manifest, launcher, two skills).
    final class Sandbox {
        let root: URL
        let home: URL
        let payload: URL
        private let fm = FileManager.default

        init() throws {
            root = fm.temporaryDirectory.appendingPathComponent("astroshots-tools-\(UUID().uuidString)")
            home = root.appendingPathComponent("home")
            payload = root.appendingPathComponent("payload")
            try fm.createDirectory(at: home, withIntermediateDirectories: true)
            try writePayload()
        }

        deinit {
            // Rollback tests leave unreadable files behind on purpose.
            if let walker = fm.enumerator(at: root, includingPropertiesForKeys: nil) {
                for case let url as URL in walker {
                    try? fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
                }
            }
            try? fm.removeItem(at: root)
        }

        /// Rewrite the payload; `revision` changes every skill file hash.
        func writePayload(revision: Int = 1) throws {
            try? fm.removeItem(at: payload)
            var files: [String: [String: String]] = [:]
            for skill in ["alpha", "beta"] {
                try write("skills/\(skill)/SKILL.md", "---\nname: \(skill)\n---\nrev \(revision)\n")
                try write("skills/\(skill)/refs/notes.md", "notes \(skill)\n")
                for rel in ["SKILL.md", "refs/notes.md"] {
                    files["skills/\(skill)/\(rel)"] = ["sha256": "\(skill)-\(rel)-\(revision)"]
                }
            }
            try write("bin/astroshot", "#!/bin/sh\necho astroshot\n")
            try fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: path("payload/bin/astroshot"))
            let manifest = try JSONSerialization.data(withJSONObject: ["format": 1, "files": files])
            try manifest.write(to: payload.appendingPathComponent("manifest.json"))
        }

        func path(_ relative: String) -> String { root.appendingPathComponent(relative).path }
        func homePath(_ relative: String) -> String { home.appendingPathComponent(relative).path }

        func write(_ relative: String, _ text: String) throws {
            let url = root.appendingPathComponent(relative.hasPrefix("home/") ? relative : "payload/\(relative)")
            try fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data(text.utf8).write(to: url)
        }

        func writeHome(_ relative: String, _ text: String) throws {
            try write("home/\(relative)", text)
        }

        func mkdirHome(_ relative: String) throws {
            try fm.createDirectory(atPath: homePath(relative), withIntermediateDirectories: true)
        }

        func read(_ homeRelative: String) -> String? {
            fm.contents(atPath: homePath(homeRelative)).flatMap { String(data: $0, encoding: .utf8) }
        }

        func exists(_ homeRelative: String) -> Bool { fm.fileExists(atPath: homePath(homeRelative)) }

        func installer(shell: String?) -> ToolsInstaller {
            ToolsInstaller(payload: payload, home: home, loginShell: shell)
        }
    }

    private let appBin = "Library/Application Support/Astroshots/bin"

    // MARK: Harness detection

    @Test
    func absentHomesReceiveNothingAndAreNotCreated() throws {
        let box = try Sandbox()
        let report = try box.installer(shell: "/bin/zsh").install()

        #expect(report.results.filter { if case .skills = $0.component { true } else { false } }.isEmpty)
        for dir in [".claude", ".codex", ".cursor", ".rovodev", ".grok", ".config/opencode", ".pi", ".agents"] {
            #expect(!box.exists(dir), "\(dir) must not be created")
        }
    }

    @Test
    func skillsGoOnlyToExistingHomesAcrossEverySupportedHarness() throws {
        let box = try Sandbox()
        let present = [".claude", ".codex", ".cursor", ".rovodev", ".grok", ".config/opencode", ".pi/agent", ".agents"]
        for dir in present.dropLast() { try box.mkdirHome(dir) }
        // .agents is the one left absent.

        let report = try box.installer(shell: nil).install()

        for dir in present.dropLast() {
            #expect(box.exists("\(dir)/skills/alpha/SKILL.md"), "\(dir) alpha")
            #expect(box.exists("\(dir)/skills/beta/refs/notes.md"), "\(dir) beta")
        }
        #expect(!box.exists(".agents"))
        #expect(report.result(for: .skills(harness: "agents")) == nil)
        #expect(report.result(for: .skills(harness: "pi"))?.outcome == .installed)
    }

    // MARK: Shells

    @Test
    func fishGetsAManagedConfDFile() throws {
        let box = try Sandbox()
        let report = try box.installer(shell: "/opt/homebrew/bin/fish").install()

        #expect(report.result(for: .shellPath)?.outcome == .installed)
        let conf = try #require(box.read(".config/fish/conf.d/astroshots.fish"))
        #expect(conf.contains("astroshots-managed"))
        #expect(conf.contains("Library/Application Support/Astroshots/bin"))
    }

    @Test
    func zshBlockIsAppendedToZshenvPreservingExistingContent() throws {
        let box = try Sandbox()
        let original = "export FOO=1\nalias ll='ls -l'   # trailing spaces  \n"
        try box.writeHome(".zshenv", original)

        let report = try box.installer(shell: "/bin/zsh").install()

        #expect(report.result(for: .shellPath)?.outcome == .installed)
        let rc = try #require(box.read(".zshenv"))
        #expect(rc.hasPrefix(original))
        #expect(rc.components(separatedBy: "astroshots managed (do not edit)").count == 2)
        #expect(rc.contains("Application Support/Astroshots/bin"))
    }

    @Test
    func bashUsesTheFirstExistingLoginFileAndKeepsItsMode() throws {
        let box = try Sandbox()
        try box.writeHome(".profile", "umask 077\n")
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: box.homePath(".profile"))

        _ = try box.installer(shell: "/bin/bash").install()

        #expect(box.read(".profile")?.hasPrefix("umask 077\n") == true)
        #expect(box.read(".profile")?.contains("astroshots managed") == true)
        #expect(!box.exists(".bash_profile"))
        let mode = try FileManager.default.attributesOfItem(atPath: box.homePath(".profile"))[.posixPermissions] as? NSNumber
        #expect(mode?.intValue == 0o600)
    }

    @Test
    func bashCreatesBashProfileWhenNoLoginFileExists() throws {
        let box = try Sandbox()
        _ = try box.installer(shell: "/bin/bash").install()
        #expect(box.read(".bash_profile")?.contains("astroshots managed") == true)
    }

    @Test
    func symlinkedRcFileIsWrittenThroughAndStaysASymlink() throws {
        let box = try Sandbox()
        try box.writeHome("dotfiles/zshenv", "export KEEP=1\n")
        try FileManager.default.createSymbolicLink(
            atPath: box.homePath(".zshenv"), withDestinationPath: box.homePath("dotfiles/zshenv"))

        _ = try box.installer(shell: "/bin/zsh").install()

        #expect((try? FileManager.default.destinationOfSymbolicLink(atPath: box.homePath(".zshenv"))) != nil)
        #expect(box.read("dotfiles/zshenv")?.hasPrefix("export KEEP=1\n") == true)
        #expect(box.read("dotfiles/zshenv")?.contains("astroshots managed") == true)
    }

    @Test
    func unknownShellGetsManualStepsAndNoFiles() throws {
        let box = try Sandbox()
        for shell in ["/usr/bin/nu", "/bin/tcsh", nil] as [String?] {
            let report = try box.installer(shell: shell).install()
            guard case .manual(let steps)? = report.result(for: .shellPath)?.outcome else {
                Issue.record("expected manual outcome for \(shell ?? "nil")")
                continue
            }
            #expect(steps.contains { $0.contains("Application Support/Astroshots/bin") })
        }
        for rc in [".zshenv", ".bash_profile", ".profile", ".config/fish"] { #expect(!box.exists(rc)) }
    }

    // MARK: Shim

    @Test
    func shimExecsTheBundledLauncher() throws {
        let box = try Sandbox()
        _ = try box.installer(shell: nil).install()

        let shim = try #require(box.read("\(appBin)/astroshot"))
        #expect(shim.contains("exec '\(box.path("payload/bin/astroshot"))' \"$@\""))
        let mode = try FileManager.default.attributesOfItem(atPath: box.homePath("\(appBin)/astroshot"))[.posixPermissions] as? NSNumber
        #expect(mode?.intValue == 0o755)
    }

    // MARK: User-owned files

    @Test
    func userOwnedSkillsShimAndFishFileAreRefusedAndKeptByteForByte() throws {
        let box = try Sandbox()
        try box.mkdirHome(".claude")
        try box.writeHome(".claude/skills/alpha/SKILL.md", "my own alpha\n")  // no marker
        try box.writeHome("\(appBin)/astroshot", "#!/bin/sh\necho mine\n")
        try box.writeHome(".config/fish/conf.d/astroshots.fish", "echo mine\n")
        try FileManager.default.createSymbolicLink(  // a linked skill (e.g. skills.sh) is also user-owned
            atPath: box.homePath(".claude/skills/beta"), withDestinationPath: box.path("payload/skills/beta"))

        let report = try box.installer(shell: "/usr/local/bin/fish").install()

        let skills = try #require(report.result(for: .skills(harness: "claude")))
        #expect(skills.skippedUserOwned == ["alpha", "beta"])
        #expect(box.read(".claude/skills/alpha/SKILL.md") == "my own alpha\n")
        #expect(!box.exists(".claude/skills/alpha/.astroshots-managed.json"))
        #expect((try? FileManager.default.destinationOfSymbolicLink(atPath: box.homePath(".claude/skills/beta"))) != nil)
        #expect(box.read("\(appBin)/astroshot") == "#!/bin/sh\necho mine\n")
        #expect(box.read(".config/fish/conf.d/astroshots.fish") == "echo mine\n")
        guard case .userOwned? = report.result(for: .cliShim)?.outcome,
              case .userOwned? = report.result(for: .shellPath)?.outcome
        else { Issue.record("expected userOwned for shim and fish file"); return }
    }

    @Test
    func halfEditedRcBlockIsLeftAlone() throws {
        let box = try Sandbox()
        let broken = "export A=1\n# >>> astroshots managed (do not edit) >>>\nexport MINE=1\n"
        try box.writeHome(".zshenv", broken)

        let report = try box.installer(shell: "/bin/zsh").install()

        guard case .userOwned? = report.result(for: .shellPath)?.outcome else {
            Issue.record("expected userOwned"); return
        }
        #expect(box.read(".zshenv") == broken)
    }

    // MARK: Rollback

    @Test
    func failedComponentRestoresItsPreviousSkillsAndOthersStillInstall() throws {
        let box = try Sandbox()
        try box.mkdirHome(".claude")
        try box.mkdirHome(".codex")
        let first = box.installer(shell: "/bin/zsh")
        _ = try first.install()
        let claudeBefore = try #require(box.read(".claude/skills/alpha/SKILL.md"))

        // New payload revision: alpha updates fine, beta's source is unreadable,
        // so the copy fails after alpha was already swapped in.
        try box.writePayload(revision: 2)
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o000], ofItemAtPath: box.path("payload/skills/beta/refs/notes.md"))

        let report = try first.install()

        for harness in ["claude", "codex"] {
            guard case .failed? = report.result(for: .skills(harness: harness))?.outcome else {
                Issue.record("expected failed for \(harness)"); return
            }
            #expect(box.read(".\(harness)/skills/alpha/SKILL.md") == claudeBefore, "\(harness) alpha restored")
            #expect(box.exists(".\(harness)/skills/beta/SKILL.md"), "\(harness) beta restored")
            #expect(box.read(".\(harness)/skills/beta/SKILL.md")?.contains("rev 1") == true)
        }
        // No staging or backup debris.
        let debris = try FileManager.default.contentsOfDirectory(atPath: box.homePath(".claude/skills"))
            .filter { $0.contains("astroshots-") }
        #expect(debris.isEmpty)
        // Independent components were not blocked by the failures.
        #expect(report.result(for: .cliShim)?.outcome == .unchanged)
        #expect(report.result(for: .shellPath)?.outcome == .unchanged)
    }

    @Test
    func failedFirstInstallLeavesNoPartialSkillsDirectory() throws {
        let box = try Sandbox()
        try box.mkdirHome(".claude")
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o000], ofItemAtPath: box.path("payload/skills/beta/refs/notes.md"))

        let report = try box.installer(shell: nil).install()

        guard case .failed? = report.result(for: .skills(harness: "claude"))?.outcome else {
            Issue.record("expected failed"); return
        }
        #expect(!box.exists(".claude/skills"))
        #expect(report.result(for: .cliShim)?.outcome == .installed)
    }

    @Test
    func unwritableShellTargetFailsOnlyTheShellComponent() throws {
        let box = try Sandbox()
        try box.mkdirHome(".config/fish/conf.d")
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o555], ofItemAtPath: box.homePath(".config/fish/conf.d"))

        let report = try box.installer(shell: "/usr/bin/fish").install()

        guard case .failed? = report.result(for: .shellPath)?.outcome else {
            Issue.record("expected failed"); return
        }
        #expect(report.result(for: .cliShim)?.outcome == .installed)
        #expect(try FileManager.default.contentsOfDirectory(atPath: box.homePath(".config/fish/conf.d")).isEmpty)
    }

    // MARK: Idempotence and update

    @Test
    func reinstallWritesNothingAndUpdatedPayloadRefreshesManagedSkills() throws {
        let box = try Sandbox()
        try box.mkdirHome(".codex")
        try box.writeHome(".zshenv", "export A=1\n")
        let installer = box.installer(shell: "/bin/zsh")
        _ = try installer.install()
        let rcAfterFirst = box.read(".zshenv")
        let skillFile = box.homePath(".codex/skills/alpha/SKILL.md")
        let mtime = try FileManager.default.attributesOfItem(atPath: skillFile)[.modificationDate] as? Date

        let second = try installer.install()

        #expect(second.results.allSatisfy { $0.outcome == .unchanged })
        #expect(box.read(".zshenv") == rcAfterFirst)
        #expect(try FileManager.default.attributesOfItem(atPath: skillFile)[.modificationDate] as? Date == mtime)

        try box.writePayload(revision: 2)
        let third = try installer.install()
        #expect(third.result(for: .skills(harness: "codex"))?.outcome == .installed)
        #expect(box.read(".codex/skills/alpha/SKILL.md")?.contains("rev 2") == true)
        #expect(third.result(for: .shellPath)?.outcome == .unchanged)
    }

    @Test
    func movedAppRefreshesTheShimAndPathBlockInPlace() throws {
        let box = try Sandbox()
        try box.writeHome(".zshenv", "export A=1\n")
        _ = try box.installer(shell: "/bin/zsh").install()

        let moved = ToolsInstaller(
            payload: box.payload, home: box.home,
            appSupport: box.home.appendingPathComponent("elsewhere"), loginShell: "/bin/zsh")
        _ = try moved.install()

        let rc = try #require(box.read(".zshenv"))
        #expect(rc.components(separatedBy: "astroshots managed (do not edit)").count == 2)
        #expect(rc.contains("elsewhere/bin"))
        #expect(!rc.contains("Application Support"))
    }

    // MARK: Payload contract

    @Test
    func invalidPayloadThrowsBeforeWritingAnything() throws {
        let box = try Sandbox()
        try box.mkdirHome(".claude")
        try FileManager.default.removeItem(atPath: box.path("payload/manifest.json"))

        #expect(throws: ToolsInstaller.PayloadError.self) { try box.installer(shell: "/bin/zsh").install() }
        #expect(!box.exists(".claude/skills"))
        #expect(!box.exists("Library"))
    }

    @Test
    func installerSourceHasNoNetworkOrSubprocessAPIs() throws {
        let source = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Astroshots/Tools/ToolsInstaller.swift")
        let text = try String(contentsOf: source, encoding: .utf8)
        for banned in ["URLSession", "Process()", "NSTask", "posix_spawn", "system("] {
            #expect(!text.contains(banned), "ToolsInstaller must not use \(banned)")
        }
    }
}
