import AppKit
import Foundation

/// Copies local screenshot files onto the general pasteboard as images.
enum ImageClipboard {
    /// Load the image at `path` and place it on `NSPasteboard.general`.
    ///
    /// Returns `false` when the file cannot be read as an image or the
    /// pasteboard rejects the write.
    @discardableResult
    static func copyImage(atPath path: String) -> Bool {
        guard let image = NSImage(contentsOfFile: path) else { return false }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        return pasteboard.writeObjects([image])
    }
}
