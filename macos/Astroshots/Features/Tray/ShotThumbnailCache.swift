import AppKit
import ImageIO

/// Downsampled thumbnails for tray rows, so a 4K PNG is decoded once at row
/// size instead of in full on every body evaluation.
///
/// Keys include the file's modification date and the requested pixel size, so
/// a re-captured shot at the same path never serves a stale image.
final class ShotThumbnailCache: @unchecked Sendable {
    static let shared = ShotThumbnailCache(countLimit: 400)

    private let cache = NSCache<NSString, NSImage>()

    init(countLimit: Int) {
        cache.countLimit = countLimit
    }

    /// Cache-only lookup. Safe to call from `body`; never touches the decoder.
    func cached(path: String, fitting size: CGSize, scale: CGFloat) -> NSImage? {
        guard let key = Self.key(path: path, size: size, scale: scale) else { return nil }
        return cache.object(forKey: key)
    }

    /// Returns the cached thumbnail or decodes a downsampled one (aspect-fill
    /// for `size`, never upscaled) and stores it. Call off the main thread.
    func thumbnail(path: String, fitting size: CGSize, scale: CGFloat) -> NSImage? {
        guard let key = Self.key(path: path, size: size, scale: scale) else { return nil }
        if let hit = cache.object(forKey: key) { return hit }
        let image = PerformanceLog.interval(PerformanceLog.imageDecode) {
            Self.downsample(path: path, fitting: size, scale: scale)
        }
        if let image { cache.setObject(image, forKey: key) }
        return image
    }

    func removeAll() {
        cache.removeAllObjects()
    }

    static func key(path: String, size: CGSize, scale: CGFloat) -> NSString? {
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: path),
              let modified = attrs[.modificationDate] as? Date
        else { return nil }
        let stamp = modified.timeIntervalSinceReferenceDate
        let pixels = "\(Int(size.width * scale))x\(Int(size.height * scale))"
        return "\(path)|\(stamp)|\(pixels)" as NSString
    }

    static func downsample(path: String, fitting size: CGSize, scale: CGFloat) -> NSImage? {
        let url = URL(fileURLWithPath: path) as CFURL
        let sourceOptions = [kCGImageSourceShouldCache: false] as CFDictionary
        guard let source = CGImageSourceCreateWithURL(url, sourceOptions),
              let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = (props[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue,
              let height = (props[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue,
              width > 0, height > 0
        else { return nil }

        // Aspect-fill: the short side must still cover the target box.
        let fill = max(size.width * scale / width, size.height * scale / height)
        let maxPixel = max(1, (max(width, height) * min(fill, 1)).rounded(.up))
        let options = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixel,
        ] as CFDictionary
        guard let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, options) else { return nil }
        return NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
    }
}
