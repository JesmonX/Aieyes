import AppKit
import SwiftUI
import Combine

private enum PetSprites {
    static let frames: [CGImage] = {
        let source = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("apps/desktop/web/pet-cat.png")
        guard let image = NSImage(contentsOf: Bundle.main.url(forResource: "pet-cat", withExtension: "png") ?? source),
              let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return [] }
        return (0..<72).compactMap { cg.cropping(to: CGRect(x: ($0 % 8) * 192, y: ($0 / 8) * 208, width: 192, height: 208)) }
    }()
}
private struct PetView: View {
    @ObservedObject var model: AppModel
    @AppStorage("aieyes.pet.enabled") private var enabled = false
    @Environment(\.accessibilityReduceMotion) private var reduced
    @State private var start = Date()
    var body: some View {
        TimelineView(.animation(minimumInterval: 0.1, paused: reduced || !enabled)) { tick in
            let state = model.petState
            let frame = state.frame(elapsed: tick.date.timeIntervalSince(start), reduced: reduced)
            VStack(spacing: 0) {
                if PetSprites.frames.indices.contains(frame) {
                    Image(decorative: PetSprites.frames[frame], scale: 1).resizable().interpolation(.high).frame(width: 132, height: 143)
                } else { Image(systemName: "cat.fill").font(.system(size: 72)).frame(width: 132, height: 143) }
                Text(state.label).font(.system(size: 12, weight: .medium)).padding(.horizontal, 9).padding(.vertical, 3).background(.regularMaterial, in: Capsule())
            }.frame(width: 144, height: 168).accessibilityElement(children: .ignore)
                .accessibilityLabel("Aieyes · " + state.label).accessibilityHint("单击打开面板，拖动移动，右键菜单")
        }.onChange(of: model.petState.key) { _, _ in start = Date() }
    }
}
private final class PetPanel: NSPanel { override var canBecomeKey: Bool { false } }
private final class PetHost: NSHostingView<PetView> {
    var open: ((NSView) -> Void)?
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { bounds.contains(point) ? self : nil }
    override func mouseDown(with event: NSEvent) {
        guard let window else { return }
        let start = NSEvent.mouseLocation, origin = window.frame.origin
        var dragged = false
        while let next = window.nextEvent(matching: [.leftMouseDragged, .leftMouseUp]) {
            let point = NSEvent.mouseLocation
            if hypot(point.x-start.x, point.y-start.y) > 4 { dragged = true }
            if next.type == .leftMouseUp { break }
            if dragged { window.setFrameOrigin(NSPoint(x: origin.x+point.x-start.x, y: origin.y+point.y-start.y)) }
        }
        if dragged { PetWindowController.clamp(window); UserDefaults.standard.set(NSStringFromPoint(window.frame.origin), forKey: "aieyes.pet.position") }
        else { open?(self) }
    }
    override func rightMouseDown(with event: NSEvent) {
        let menu = NSMenu()
        let open = NSMenuItem(title: "打开面板", action: #selector(openPanel), keyEquivalent: ""); open.target=self; menu.addItem(open)
        let hide = NSMenuItem(title: "隐藏宠物（可在设置中恢复）", action: #selector(hidePet), keyEquivalent: ""); hide.target=self; menu.addItem(hide)
        NSMenu.popUpContextMenu(menu, with: event, for: self)
    }
    override func accessibilityPerformPress() -> Bool { open?(self); return true }
    @objc private func openPanel() { open?(self) }
    @objc private func hidePet() { UserDefaults.standard.set(false, forKey: "aieyes.pet.enabled") }
}
@MainActor final class PetWindowController {
    private let panel: NSPanel
    var window: NSWindow { panel }
    private var subscriptions = Set<AnyCancellable>()
    init(model: AppModel, open: @escaping (NSView) -> Void) {
        panel = PetPanel(contentRect: NSRect(x: 0,y: 0,width: 144,height: 168), styleMask: [.borderless,.nonactivatingPanel], backing: .buffered, defer: false)
        panel.title="Aieyes 宠物"; panel.isOpaque=false; panel.backgroundColor = .clear; panel.hasShadow=false
        panel.level = .floating; panel.collectionBehavior=[.canJoinAllSpaces,.fullScreenAuxiliary]; panel.hidesOnDeactivate=false
        let host=PetHost(rootView: PetView(model:model)); host.open=open; panel.contentView=host
        host.setAccessibilityRole(.button); host.setAccessibilityLabel("Aieyes 宠物，单击打开面板")
        if let saved=UserDefaults.standard.string(forKey:"aieyes.pet.position") { panel.setFrameOrigin(NSPointFromString(saved)) }
        else if let screen=NSScreen.main { panel.setFrameOrigin(NSPoint(x:screen.visibleFrame.maxX-168,y:screen.visibleFrame.midY)) }
        Self.clamp(panel); sync()
        NotificationCenter.default.publisher(for:UserDefaults.didChangeNotification).sink { [weak self] _ in self?.sync() }.store(in:&subscriptions)
        NotificationCenter.default.publisher(for:NSApplication.didChangeScreenParametersNotification).sink { [weak self] _ in if let self { Self.clamp(self.panel) } }.store(in:&subscriptions)
    }
    private func sync() { if UserDefaults.standard.bool(forKey:"aieyes.pet.enabled") { panel.orderFrontRegardless() } else { panel.orderOut(nil) } }
    static func clamp(_ window:NSWindow) {
        guard let screen=NSScreen.screens.first(where:{$0.visibleFrame.intersects(window.frame)}) ?? NSScreen.main else { return }
        let area=screen.visibleFrame, frame=window.frame
        window.setFrameOrigin(NSPoint(x:min(max(frame.minX,area.minX),max(area.minX,area.maxX-frame.width)), y:min(max(frame.minY,area.minY),max(area.minY,area.maxY-frame.height))))
    }
}
