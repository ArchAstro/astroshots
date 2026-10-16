import CoreServices
import Foundation

/// Independent real FSEvents observer used only to establish fixture ordering.
/// Its queue is separate from the watcher's queue and the test's main actor.
final class FilesystemEventCheckpoint: @unchecked Sendable {
    private enum CheckpointError: Error {
        case creationFailed, startFailed, eventNotDelivered(String)
    }

    private let lock = NSLock()
    private var paths = Set<String>()
    private var stream: FSEventStreamRef?

    init(root: URL) throws {
        var context = FSEventStreamContext(
            version: 0, info: Unmanaged.passUnretained(self).toOpaque(),
            retain: nil, release: nil, copyDescription: nil
        )
        let callback: FSEventStreamCallback = { _, info, _, eventPaths, _, _ in
            guard let info else { return }
            let observer = Unmanaged<FilesystemEventCheckpoint>.fromOpaque(info).takeUnretainedValue()
            let paths = unsafeBitCast(eventPaths, to: NSArray.self) as? [String] ?? []
            observer.lock.lock()
            observer.paths.formUnion(paths.map { URL(fileURLWithPath: $0).standardizedFileURL.path })
            observer.lock.unlock()
        }
        // SinceNow can miss a write while the daemon registers this stream.
        // An explicit cursor replays that registration window as well.
        guard let created = FSEventStreamCreate(
            kCFAllocatorDefault, callback, &context, [root.path] as CFArray,
            FSEventsGetCurrentEventId(), 0.01,
            FSEventStreamCreateFlags(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagUseCFTypes)
        ) else { throw CheckpointError.creationFailed }
        stream = created
        FSEventStreamSetDispatchQueue(created, DispatchQueue(label: "astroshots-test-event-checkpoint"))
        guard FSEventStreamStart(created) else {
            stop()
            throw CheckpointError.startFailed
        }
    }

    func waitForEvent(at path: String) async throws {
        for _ in 0..<500 {
            if contains(path) { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        throw CheckpointError.eventNotDelivered(path)
    }

    private func contains(_ path: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return paths.contains(URL(fileURLWithPath: path).standardizedFileURL.path)
    }

    func flushedCursor() -> FSEventStreamEventId {
        if let stream { FSEventStreamFlushSync(stream) }
        return FSEventsGetCurrentEventId()
    }

    func stop() {
        guard let stream else { return }
        FSEventStreamStop(stream)
        FSEventStreamInvalidate(stream)
        FSEventStreamRelease(stream)
        self.stream = nil
    }

    deinit { stop() }
}
