import AppKit
import SwiftUI

/// Loads a local image path into a SwiftUI view. Falls back to a dark placeholder.
///
/// With `rowSize` the image comes from `ShotThumbnailCache` (downsampled,
/// decoded off the main thread); without it the full image is decoded inline.
struct ShotThumbnail: View {
    let path: String
    var contentMode: ContentMode = .fill
    var rowSize: CGSize?

    @Environment(\.displayScale) private var displayScale
    @State private var loaded: NSImage?

    var body: some View {
        Group {
            if let image = displayImage {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: contentMode)
            } else {
                ZStack {
                    Color(hex: 0x1C1B19)
                    Image(systemName: "photo")
                        .foregroundStyle(Theme.muted)
                }
            }
        }
        .background(Color(hex: 0x1C1B19))
        .task(id: path) { await loadRowThumbnail() }
    }

    private var displayImage: NSImage? {
        guard let rowSize else {
            return PerformanceLog.interval(PerformanceLog.imageDecode) {
                NSImage(contentsOfFile: path)
            }
        }
        return ShotThumbnailCache.shared.cached(path: path, fitting: rowSize, scale: displayScale)
            ?? loaded
    }

    private func loadRowThumbnail() async {
        guard let rowSize,
              ShotThumbnailCache.shared.cached(path: path, fitting: rowSize, scale: displayScale) == nil
        else { return }
        let path = path
        let scale = displayScale
        let image = await Task.detached(priority: .userInitiated) {
            ShotThumbnailCache.shared.thumbnail(path: path, fitting: rowSize, scale: scale)
        }.value
        loaded = image
    }
}
