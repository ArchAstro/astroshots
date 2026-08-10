import Foundation
import OSLog
import Sparkle

/// Unified Console.app logging for the Sparkle upgrade path.
///
/// Filter in Console (or `log stream`):
/// ```
/// subsystem:ai.archastro.Astroshots category:SoftwareUpdate
/// ```
///
/// Or from a shell while testing releases:
/// ```
/// log stream --style compact --predicate \
///   'subsystem == "ai.archastro.Astroshots" AND category == "SoftwareUpdate"'
/// ```
enum SoftwareUpdateLog {
    static let subsystem = "ai.archastro.Astroshots"
    static let category = "SoftwareUpdate"

    static let logger = Logger(subsystem: subsystem, category: category)

    static func installedVersionSummary(
        bundle: Bundle = .main
    ) -> String {
        let short = bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
            ?? "?"
        let build = bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "?"
        let id = bundle.bundleIdentifier ?? "?"
        return "\(id) \(short) (\(build))"
    }

    static func feedURLString(bundle: Bundle = .main) -> String {
        (bundle.object(forInfoDictionaryKey: "SUFeedURL") as? String) ?? "(missing SUFeedURL)"
    }

    static func describe(_ check: SPUUpdateCheck) -> String {
        switch check {
        case .updates: return "user-initiated"
        case .updatesInBackground: return "background"
        case .updateInformation: return "information-probe"
        @unknown default: return "unknown(\(check.rawValue))"
        }
    }

    static func describe(_ choice: SPUUserUpdateChoice) -> String {
        switch choice {
        case .skip: return "skip"
        case .install: return "install"
        case .dismiss: return "dismiss"
        @unknown default: return "unknown(\(choice.rawValue))"
        }
    }

    static func describe(_ item: SUAppcastItem) -> String {
        let url = item.fileURL?.absoluteString ?? "(no fileURL)"
        let length = item.contentLength
        let critical = item.isCriticalUpdate ? " critical" : ""
        let major = item.isMajorUpgrade ? " major" : ""
        let delta = item.isDeltaUpdate ? " delta" : ""
        let channel = item.channel.map { " channel=\($0)" } ?? ""
        return "\(item.displayVersionString) [\(item.versionString)]\(critical)\(major)\(delta)\(channel) \(url) (\(length) bytes)"
    }

    static func describe(_ error: Error) -> String {
        let ns = error as NSError
        var parts = [
            "domain=\(ns.domain)",
            "code=\(ns.code)",
            ns.localizedDescription,
        ]
        if let reason = ns.localizedFailureReason, !reason.isEmpty {
            parts.append("reason=\(reason)")
        }
        if let underlying = ns.userInfo[NSUnderlyingErrorKey] as? NSError {
            parts.append(
                "underlying={domain=\(underlying.domain) code=\(underlying.code) \(underlying.localizedDescription)}"
            )
        }
        return parts.joined(separator: " | ")
    }
}
