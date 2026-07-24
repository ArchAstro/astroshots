import AppKit
import SwiftUI

/// Loads a local image path into a SwiftUI view. Falls back to a dark placeholder.
struct ShotThumbnail: View {
    let path: String

    var body: some View {
        Group {
            if let image = NSImage(contentsOfFile: path) {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else {
                ZStack {
                    Color(hex: 0x1C1B19)
                    Image(systemName: "photo")
                        .foregroundStyle(Theme.muted)
                }
            }
        }
        .background(Color(hex: 0x1C1B19))
    }
}
