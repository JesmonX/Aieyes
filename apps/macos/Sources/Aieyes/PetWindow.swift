import AppKit
import SwiftUI
import Combine

@MainActor private enum PetSprites {
    static var key: String?
    static var cached: [CGImage] = []
    static func frames(for asset: ImportedPet?) -> [CGImage] {
        guard let asset else { key = nil; cached = []; return [] }
        if key == asset.spritePath { return cached }
        key = asset.spritePath; cached = []
        guard let image = NSImage(contentsOfFile: asset.spritePath),
              let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil),
              cg.width == asset.cellWidth * 8, cg.height == asset.cellHeight * asset.rows else { return [] }
        cached = (0..<(8 * asset.rows)).compactMap {
            cg.cropping(to: CGRect(x: ($0 % 8) * asset.cellWidth, y: ($0 / 8) * asset.cellHeight, width: asset.cellWidth, height: asset.cellHeight))
        }
        return cached
    }
}
private struct PetView: View {
    @ObservedObject var model: AppModel
    @AppStorage("aieyes.pet.enabled") private var enabled = true
    @Environment(\.accessibilityReduceMotion) private var reduced
    @State private var start = Date()
    var body: some View {
        TimelineView(.animation(minimumInterval: 0.1, paused: reduced || !enabled)) { tick in
            let state = model.petState
            let frames = PetSprites.frames(for: model.importedPet)
            let frame = state.frame(elapsed: tick.date.timeIntervalSince(start), reduced: reduced)
            VStack(spacing: 0) {
                if frames.indices.contains(frame) {
                    Image(decorative: frames[frame], scale: 1).resizable().interpolation(.high).frame(width: 132, height: 143)
                } else { Text("请导入桌宠").font(.callout).frame(width: 132, height: 143) }
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
        UserDefaults.standard.register(defaults: ["aieyes.pet.enabled": true])
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
