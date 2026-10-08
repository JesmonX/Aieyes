import AppKit
import SwiftUI

/// Keep the full-panel tint outside SwiftUI's hover/content damage regions.
/// NSPopover supplies the single system material behind this transparent container.
@MainActor final class PanelContainer: NSViewController {
    private let model: AppModel
    init(model: AppModel) { self.model = model; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
    override func loadView() {
        let container = NSView()
        let background = PanelTintView()
        let content = NSHostingView(rootView: RootView(model: model, nativePanelBackground: true))
        for child in [background, content] {
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
