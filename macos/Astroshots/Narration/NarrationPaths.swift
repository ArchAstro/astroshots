import Foundation
import HuggingFace

enum NarrationPaths {
    static var applicationSupport: URL {
        let base = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first
            ?? FileManager.default.temporaryDirectory
        return base
            .appendingPathComponent("Astroshots", isDirectory: true)
            .appendingPathComponent("Narration", isDirectory: true)
    }

    static var workRoot: URL {
        applicationSupport.appendingPathComponent("Work", isDirectory: true)
    }

    /// Directory where `ModelUtils` stores mlx-audio snapshots (see MLXAudioCore).
    static func mlxAudioModelDirectory(
        modelID: String = NarrationDefaults.modelID,
        cache: HubCache = .default
    ) -> URL {
        let modelSubdir = modelID.replacingOccurrences(of: "/", with: "_")
        return cache.cacheDirectory
            .appendingPathComponent("mlx-audio", isDirectory: true)
            .appendingPathComponent(modelSubdir, isDirectory: true)
    }

    static func ensureDirectories() {
        let fm = FileManager.default
        for url in [applicationSupport, workRoot] {
            try? fm.createDirectory(at: url, withIntermediateDirectories: true)
        }
    }

    /// True when a non-empty safetensors weight file is already on disk.
    static func isModelOnDisk(modelID: String = NarrationDefaults.modelID) -> Bool {
        let dir = mlxAudioModelDirectory(modelID: modelID)
        guard FileManager.default.fileExists(atPath: dir.path),
              let files = try? FileManager.default.contentsOfDirectory(
                  at: dir,
                  includingPropertiesForKeys: [.fileSizeKey],
                  options: [.skipsHiddenFiles]
              )
        else { return false }
        return files.contains { file in
            guard file.pathExtension == "safetensors" else { return false }
            let size = (try? file.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? 0
            return size > 0
        }
    }
}
