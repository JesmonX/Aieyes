import SwiftUI
import AppKit

/// Mouse-operated surfaces must leave the frontmost application's keyboard focus alone.
final class PassivePanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    static let didHide = Notification.Name("AieyesPassivePanelDidHide")
    override func orderOut(_ sender: Any?) {
        NotificationCenter.default.post(name: Self.didHide, object: self)
        super.orderOut(sender)
    }
}
final class PassiveHostingView<Content: View>: NSHostingView<Content> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

@MainActor final class MenuPanel {
    var contentSize = NSSize(width: 450, height: 720)
    var contentViewController: NSViewController?
    var behavior: NSPopover.Behavior = .transient
    var onClose: (() -> Void)?
    private var window: PassivePanel?
    private var localMonitor: Any?
    private var globalMonitor: Any?
    private weak var anchor: NSView?
    var isShown: Bool { window?.isVisible == true }

    func show(relativeTo rect: NSRect, of view: NSView, preferredEdge: NSRectEdge) {
        guard let parent = view.window else { return }
        anchor = view
        if window == nil {
            let panel = PassivePanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
            panel.level = .popUpMenu; panel.isReleasedWhenClosed = false; panel.hidesOnDeactivate = false
            panel.hasShadow = true; panel.backgroundColor = .clear
            window = panel
        }
        guard let window else { return }
        window.contentViewController = contentViewController
        let target = parent.convertToScreen(view.convert(rect, to: nil))
        let screen = parent.screen?.visibleFrame ?? target
        let x = min(max(screen.minX, target.midX - contentSize.width / 2), screen.maxX - contentSize.width)
        window.setFrame(NSRect(x: x, y: max(screen.minY, target.minY - contentSize.height - 5), width: contentSize.width, height: contentSize.height), display: true)
        window.orderFrontRegardless()
        localMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] event in
            guard let self, self.behavior != .applicationDefined else { return event }
            let point = NSEvent.mouseLocation
            let onAnchor = self.anchor.map { anchor in anchor.window.map { $0.convertToScreen(anchor.convert(anchor.bounds, to: nil)).contains(point) } ?? false } ?? false
            if !window.frame.contains(point) && !onAnchor { self.performClose(nil) }
            return event
        }
        globalMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            guard let self, self.behavior != .applicationDefined else { return }
            self.performClose(nil)
        }
    }
    func performClose(_ sender: Any?) {
        guard isShown else { return }
        if let localMonitor { NSEvent.removeMonitor(localMonitor) }; localMonitor = nil
        if let globalMonitor { NSEvent.removeMonitor(globalMonitor) }; globalMonitor = nil
        window?.orderOut(nil); onClose?()
    }
}
