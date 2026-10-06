import SwiftUI
import AppKit

struct FloatingTokenCard: View {
    var total: Double
    var tokens: Tokens
    @State private var hovering = false
    @State private var expanded = false
    var body: some View {
        Button { expanded.toggle() } label: {
            VStack(alignment: .leading, spacing: 8) {
                HStack { Text("总 Token"); Spacer(); Image(systemName: "sparkle") }.font(AppFont.secondary).foregroundStyle(.secondary)
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text(Format.compact(total)).font(.system(size: 24, weight: .semibold))
                    Text("(" + Format.percent(tokens.cacheRate.map { $0 * 100 }) + ")").font(.system(size: 12)).foregroundStyle(.secondary)
                }.monospacedDigit()
                Text("点击查看详情").font(.system(size: 11)).foregroundStyle(.secondary).opacity(hovering ? 1 : 0)
            }.frame(maxWidth: .infinity, alignment: .leading).padding(12).modifier(NeutralCard())
        }.buttonStyle(.plain).onHover { hovering = $0 }.accessibilityLabel("总 Token，点击查看详情")
            .background(TokenPanelAnchor(expanded: $expanded, tokens: tokens))
    }
}

final class TokenAnchorView: NSView {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
struct TokenPanelAnchor: NSViewRepresentable {
    @Binding var expanded: Bool
    var tokens: Tokens
    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeNSView(context: Context) -> NSView { TokenAnchorView() }
    func updateNSView(_ view: NSView, context: Context) {
        context.coordinator.dismiss = { expanded = false }
        if expanded { context.coordinator.show(view, tokens: tokens) } else { context.coordinator.close() }
    }
    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) { coordinator.close() }
    @MainActor final class Coordinator {
        var panel: PassivePanel?
        var localMonitor: Any?
        var globalMonitor: Any?
        var closeObserver: NSObjectProtocol?
        var dismiss: (() -> Void)?
        func show(_ anchor: NSView, tokens: Tokens) {
            guard let parent = anchor.window else { return }
            if panel == nil {
                let panel = PassivePanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
                panel.isReleasedWhenClosed = false; panel.hidesOnDeactivate = false; panel.level = .popUpMenu
                panel.backgroundColor = .clear; panel.hasShadow = true
                self.panel = panel
                parent.addChildWindow(panel, ordered: .above)
                localMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .keyDown, .scrollWheel]) { [weak self, weak anchor] event in
                    guard let self else { return event }
                    let point = NSEvent.mouseLocation
                    let anchorRect = anchor.map { parent.convertToScreen($0.convert($0.bounds, to: nil)) } ?? .zero
                    if event.type == .scrollWheel || (event.type == .keyDown && event.keyCode == 53) || (event.type != .keyDown && !panel.frame.contains(point) && !anchorRect.contains(point)) { self.dismiss?() }
                    return event
                }
                globalMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in self?.dismiss?() }
                closeObserver = NotificationCenter.default.addObserver(forName: PassivePanel.didHide, object: parent, queue: .main) { [weak self] _ in MainActor.assumeIsolated { self?.dismiss?(); self?.close() } }
            }
            guard let panel else { return }
            panel.contentView = PassiveHostingView(rootView: CacheSummaryCard(tokens: tokens).padding(10).frame(width: 340).background(.regularMaterial))
            let size = panel.contentView?.fittingSize ?? NSSize(width: 340, height: 170)
            let rect = parent.convertToScreen(anchor.convert(anchor.bounds, to: nil))
            let screen = parent.screen?.visibleFrame ?? parent.frame
            let x = max(screen.minX, min(rect.minX, screen.maxX - size.width))
            let y = rect.minY - size.height - 5 >= screen.minY ? rect.minY - size.height - 5 : rect.maxY + 5
            panel.setFrame(NSRect(origin: NSPoint(x: x, y: y), size: size), display: true)
            panel.orderFrontRegardless()
        }
        func close() {
            if let localMonitor { NSEvent.removeMonitor(localMonitor) }; localMonitor = nil
            if let globalMonitor { NSEvent.removeMonitor(globalMonitor) }; globalMonitor = nil
            if let closeObserver { NotificationCenter.default.removeObserver(closeObserver) }; closeObserver = nil
            if let panel { panel.parent?.removeChildWindow(panel); panel.orderOut(nil) }; panel = nil
        }
    }
}
