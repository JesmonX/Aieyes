import AppKit

/// Keep a Dock entry for standalone windows, including minimized or hidden ones.
/// Menu-bar and floating panels do not participate in Dock visibility.
@MainActor final class DockPresence {
    private var windows = Set<ObjectIdentifier>()
    private var regular = false
    private let apply: @MainActor (NSApplication.ActivationPolicy) -> Void

    init(apply: @escaping @MainActor (NSApplication.ActivationPolicy) -> Void = { NSApp.setActivationPolicy($0) }) {
        self.apply = apply
        NotificationCenter.default.addObserver(self, selector: #selector(windowWillClose(_:)), name: NSWindow.willCloseNotification, object: nil)
    }

    deinit { NotificationCenter.default.removeObserver(self) }

    func open(_ window: NSWindow) {
        windows.insert(ObjectIdentifier(window))
        update()
    }

    @objc private func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow,
              windows.remove(ObjectIdentifier(window)) != nil else { return }
        updateAfterClose()
    }

    private func updateAfterClose() {
        // Coalesce window transitions so the Dock entry doesn't disappear while
        // another standalone window is being opened.
        DispatchQueue.main.async { [weak self] in self?.update() }
    }

    private func update() {
        let wanted = !windows.isEmpty
        guard regular != wanted else { return }
        regular = wanted
        apply(wanted ? .regular : .accessory)
    }
}
