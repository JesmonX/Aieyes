import AppKit

/// Main-thread position memory. Disk writes follow a quiet period, never each frame.
@MainActor final class ScrollPositionStore {
    static let shared = ScrollPositionStore()
    private let defaults: UserDefaults
    private let file: PositionFile
    private let delay: TimeInterval
    private var positions: [String: NSPoint] = [:]
    private var saved: [String: NSPoint] = [:]
    private var dirty = Set<String>()
    private var work: [String: DispatchWorkItem] = [:]
    private var generations: [String: UUID] = [:]
    private var observers: [NSObjectProtocol] = []
    private(set) var writes = 0
    var pendingCount: Int { dirty.count }

    init(defaults: UserDefaults = .standard, fileURL: URL? = nil, delay: TimeInterval = 0.25, observeLifecycle: Bool = true) {
        self.defaults = defaults; self.delay = delay
        let url = fileURL ?? Self.storageURL()
        file = PositionFile(url: url)
        if let data = try? Data(contentsOf: url),
           let values = try? PropertyListDecoder().decode([String: [Double]].self, from: data) {
            for (key, point) in values where point.count == 2 && point.allSatisfy(\.isFinite) {
                positions[key] = NSPoint(x: point[0], y: point[1]); saved[key] = positions[key]
            }
        }
        if observeLifecycle {
            for name in [NSWindow.willCloseNotification, NSApplication.willTerminateNotification] {
                observers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.flushAll() }
                })
            }
        }
    }
    func position(for key: String) -> NSPoint? {
        if let point = positions[key] { return point }
        guard let values = defaults.array(forKey: "view.scroll.v2." + key) as? [Double],
              values.count == 2, values.allSatisfy(\.isFinite) else { return nil }
        let point = NSPoint(x: values[0], y: values[1])
        // Migrate lazily without writing to the broadly observed settings domain.
        positions[key] = point; dirty.insert(key); schedule(key)
        return point
    }
    func record(_ point: NSPoint, for key: String) {
        guard point.x.isFinite, point.y.isFinite, position(for: key) != point else { return }
        positions[key] = point; dirty.insert(key); schedule(key)
    }
    private func schedule(_ key: String) {
        work.removeValue(forKey: key)?.cancel()
        let generation = UUID(); generations[key] = generation
        let item = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.generations[key] == generation else { return }
                self.flush(key)
            }
        }
        work[key] = item; DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: item)
    }
    func flush(_ key: String) {
        work.removeValue(forKey: key)?.cancel(); generations[key] = nil
        guard dirty.remove(key) != nil, let point = positions[key], saved[key] != point else { return }
        saved[key] = point; writes += 1
        // UserDefaults.set synchronously invalidates unrelated @AppStorage views.
        // Coordinates have a separate file and serialize off the UI thread.
        file.write(saved.mapValues { [Double($0.x), Double($0.y)] })
    }
    func flushAll() { for key in Array(dirty) { flush(key) }; file.wait() }
    private static func storageURL() -> URL {
        if let path = ProcessInfo.processInfo.environment["AIEYES_DATA_DIR"]
            ?? Bundle.main.object(forInfoDictionaryKey: "AieyesDevelopmentDataDirectory") as? String {
            return URL(fileURLWithPath: path, isDirectory: true).appendingPathComponent("scroll-positions.plist")
        }
        // Unbundled verifiers and isolated audit bundles retain their own domain.
        let domain = Bundle.main.bundleIdentifier ?? ProcessInfo.processInfo.processName
        return FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(domain, isDirectory: true).appendingPathComponent("scroll-positions.plist")
    }
    deinit { for observer in observers { NotificationCenter.default.removeObserver(observer) }; for item in work.values { item.cancel() } }
}

private final class PositionFile: @unchecked Sendable {
    private let url: URL
    private let queue = DispatchQueue(label: "app.aieyes.scroll-positions", qos: .utility)
    // Accessed only on queue; a failed write remains available for the next flush.
    private var pending: [String: [Double]]?
    init(url: URL) { self.url = url }
    func write(_ values: [String: [Double]]) { queue.async { self.pending = values; self.flush() } }
    func wait() { queue.sync { flush() } }
    private func flush() {
        guard let pending else { return }
        do {
            let encoder = PropertyListEncoder(); encoder.outputFormat = .binary
            let data = try encoder.encode(pending)
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try data.write(to: url, options: .atomic); self.pending = nil
        } catch { /* Retain the last snapshot and retry on the next save or close. */ }
    }
}
