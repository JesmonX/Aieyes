import AppKit

/// Main-thread position memory. Disk writes follow a quiet period, never each frame.
@MainActor final class ScrollPositionStore {
    static let shared = ScrollPositionStore()
    private let defaults: UserDefaults
    private let delay: TimeInterval
    private var positions: [String: NSPoint] = [:]
    private var saved: [String: NSPoint] = [:]
    private var dirty = Set<String>()
    private var work: [String: DispatchWorkItem] = [:]
    private var generations: [String: UUID] = [:]
    private var observers: [NSObjectProtocol] = []
    private(set) var writes = 0
    var pendingCount: Int { dirty.count }

    init(defaults: UserDefaults = .standard, delay: TimeInterval = 0.25, observeLifecycle: Bool = true) {
        self.defaults = defaults; self.delay = delay
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
        positions[key] = point; saved[key] = point
        return point
    }
    func record(_ point: NSPoint, for key: String) {
        guard point.x.isFinite, point.y.isFinite, position(for: key) != point else { return }
        positions[key] = point; dirty.insert(key); work.removeValue(forKey: key)?.cancel()
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
        // Update before set(): synchronous defaults observers may re-enter layout.
        saved[key] = point; writes += 1
        defaults.set([Double(point.x), Double(point.y)], forKey: "view.scroll.v2." + key)
    }
    func flushAll() { for key in Array(dirty) { flush(key) } }
    deinit { for observer in observers { NotificationCenter.default.removeObserver(observer) }; for item in work.values { item.cancel() } }
}
