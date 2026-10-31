import AppKit
import SwiftUI

@MainActor final class PanelPopover: NSPopover {
    override func show(relativeTo positioningRect: NSRect, of positioningView: NSView, preferredEdge: NSRectEdge) {
        super.show(relativeTo: positioningRect, of: positioningView, preferredEdge: preferredEdge)
        // AppKit assigns the anchor's window level during show, after viewDidAppear.
        if let window = contentViewController?.view.window {
            PanelContainer.keepAboveApplications(window)
            window.orderFrontRegardless()
        }
    }
}

/// Keep the full-panel tint outside SwiftUI's hover/content damage regions.
/// NSPopover supplies the single system material behind this transparent container.
@MainActor final class PanelContainer: NSViewController {
    private let model: AppModel
    init(model: AppModel) { self.model = model; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
    func prepare(size: NSSize) {
        model.panelWidth = size.width; model.panelHeight = size.height
        preferredContentSize = size
        view.setFrameSize(size)
        // Auto Layout may not solve an unattached popover tree before its first show.
        // Seed both layers explicitly so the very first drawing uses the full bounds.
        for child in view.subviews { child.frame = view.bounds }
        view.needsLayout = true
        view.layoutSubtreeIfNeeded()
    }
    // Popovers anchored to a nonactivating pet otherwise inherit its low window level.
    static func keepAboveApplications(_ window: NSWindow) {
        window.level = .statusBar
        window.hidesOnDeactivate = false
        window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
    }
    override func loadView() {
        let container = NSView(frame: NSRect(x: 0, y: 0, width: model.panelWidth, height: model.panelHeight))
        let background = PanelTintView()
        let content = NSHostingView(rootView: RootView(model: model, nativePanelBackground: true))
        // The popover owns geometry. An old SwiftUI ideal size must not constrain
        // the first frame after creation or a move to a smaller display.
        content.sizingOptions = []
        for child in [background, content] {
            child.frame = container.bounds
            child.translatesAutoresizingMaskIntoConstraints = false
            container.addSubview(child)
            NSLayoutConstraint.activate([
                child.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                child.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                child.topAnchor.constraint(equalTo: container.topAnchor),
                child.bottomAnchor.constraint(equalTo: container.bottomAnchor)
            ])
        }
        view = container
    }
}

@MainActor final class PanelTintView: NSView {
    private(set) var colorUpdates = 0
    private var observer: NSObjectProtocol?
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        observer = NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.updateColor() }
        }
        updateColor()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override func viewDidChangeEffectiveAppearance() { super.viewDidChangeEffectiveAppearance(); updateColor() }
    private func updateColor() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            let opaque = NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency || NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast
            let color = NSColor.windowBackgroundColor.withAlphaComponent(opaque ? 1 : 0.12).cgColor
            guard layer?.backgroundColor != color else { return }
            CATransaction.begin(); CATransaction.setDisableActions(true)
            layer?.backgroundColor = color
            CATransaction.commit(); colorUpdates += 1
        }
    }
    deinit { if let observer { NSWorkspace.shared.notificationCenter.removeObserver(observer) } }
}
