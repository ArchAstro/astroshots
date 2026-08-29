import Foundation
import OSLog

/// Unified Console.app logging for click-to-tray and pane-switch latency.
///
/// Filter in Console (or `log stream`):
/// ```
/// subsystem:ai.archastro.Astroshots category:Performance
/// ```
///
/// Or from a shell while measuring latency:
/// ```
/// log stream --style compact --predicate \
///   'subsystem == "ai.archastro.Astroshots" AND category == "Performance"'
/// ```
enum PerformanceLog {
    static let subsystem = "ai.archastro.Astroshots"
    static let category = "Performance"

    static let logger = Logger(subsystem: subsystem, category: category)
    static let signposter = OSSignposter(logger: logger)

    static let clickToShown: StaticString = "tray.click_to_shown"
    static let paneSwitch: StaticString = "tray.pane_switch"
    static let imageDecode: StaticString = "tray.image_decode"
    static let reviewOpen: StaticString = "review.open"
    static let reviewNavigate: StaticString = "review.navigate"

    static let intervalNames: [String] = [
        "\(clickToShown)",
        "\(paneSwitch)",
        "\(imageDecode)",
        "\(reviewOpen)",
        "\(reviewNavigate)",
    ]

    /// Begin/end an `os_signpost` interval around `work`. Unique IDs so
    /// overlapping decodes (many thumbnails) do not collapse onto `.exclusive`.
    @discardableResult
    static func interval<R>(_ name: StaticString, _ work: () throws -> R) rethrows -> R {
        let id = signposter.makeSignpostID()
        let state = signposter.beginInterval(name, id: id)
        defer { signposter.endInterval(name, state) }
        return try work()
    }
}
