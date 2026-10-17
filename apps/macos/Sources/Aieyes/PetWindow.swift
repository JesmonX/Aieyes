import AppKit
import SwiftUI
import Combine

enum PetAppearance {
    static func scale(_ value: Double) -> Double { value.isFinite ? min(2, max(0.5, value)) : 1 }
}
struct PetScaleControl: View {
    @AppStorage("aieyes.pet.scale") private var savedScale = 1.0
    var body: some View {
        HStack {
            Text("桌宠缩放")
            Slider(value: Binding(get: { PetAppearance.scale(savedScale) }, set: { savedScale = $0 }), in: 0.5...2, step: 0.05)
                .accessibilityLabel("桌宠缩放").accessibilityValue("\(Int((PetAppearance.scale(savedScale) * 100).rounded()))%")
            Text("\(Int((PetAppearance.scale(savedScale) * 100).rounded()))%").monospacedDigit().frame(width: 44, alignment: .trailing)
            Button("恢复 100%") { savedScale = 1 }.disabled(savedScale == 1)
        }
    }
}

@MainActor private enum PetSprites {
    private final class Frames { let images: [CGImage]; init(_ images: [CGImage]) { self.images = images } }
    private static let cache: NSCache<NSString, Frames> = { let cache = NSCache<NSString, Frames>(); cache.totalCostLimit = 48 * 1024 * 1024; cache.countLimit = 36; return cache }()
    static func frames(for asset: ImportedPet?, row: Int) -> [CGImage] {
        guard let asset, row >= 0, row < asset.rows else { return [] }
        let key = (asset.spritePath + ":" + String(row)) as NSString
        if let cached = cache.object(forKey: key) { return cached.images }
        guard let image = NSImage(contentsOfFile: asset.spritePath),
              let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil),
              cg.width == asset.cellWidth * 8, cg.height == asset.cellHeight * asset.rows else { return [] }
        let frames = (0..<8).compactMap {
            cg.cropping(to: CGRect(x: $0 * asset.cellWidth, y: row * asset.cellHeight, width: asset.cellWidth, height: asset.cellHeight))
        }
        cache.setObject(Frames(frames), forKey: key, cost: cg.bytesPerRow * cg.height)
        return frames
    }
}
// The timer checks a monotonic timeline; only actual frame changes publish to SwiftUI.
@MainActor final class PetAnimationClock: ObservableObject {
    @Published private(set) var frame = 0
    private var state = PetState.idle
    private var start = ProcessInfo.processInfo.systemUptime
    private var reduced = false
    private var timer: Timer?
    func configure(_ state: PetState, playing: Bool, reduced: Bool) {
        if self.state.animationIdentity != state.animationIdentity { start = ProcessInfo.processInfo.systemUptime }
        self.state = state; self.reduced = reduced
        tick()
        if playing && !reduced {
            guard timer == nil else { return }
            let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.tick() }
            }
            timer.tolerance = 0.002
            self.timer = timer
            RunLoop.main.add(timer, forMode: .common)
        } else { stop() }
    }
    private func tick() {
        let next = state.frame(elapsed: ProcessInfo.processInfo.systemUptime - start, reduced: reduced)
        if next != frame { frame = next }
    }
    func stop() { timer?.invalidate(); timer = nil }
    deinit { timer?.invalidate() }
}
private struct PetSpriteView: View {
    let state: PetState
    let asset: ImportedPet?
    let scale: Double
    let enabled: Bool
    @Environment(\.accessibilityReduceMotion) private var reduced
    @StateObject private var clock = PetAnimationClock()
    private func configure() { clock.configure(state, playing: enabled, reduced: reduced) }
    var body: some View {
        let frames = enabled ? PetSprites.frames(for: asset, row: state.row) : []
        let index = clock.frame - state.row * 8
        Group {
            if frames.indices.contains(index) {
                Image(decorative: frames[index], scale: 1).resizable().interpolation(.high)
            } else { Text("请导入桌宠").font(.system(size: 13 * scale)) }
        }.frame(width: 132 * scale, height: 143 * scale)
            .onAppear { configure() }
            .onChange(of: state) { _, _ in configure() }
            .onChange(of: enabled) { _, _ in configure() }
            .onChange(of: reduced) { _, _ in configure() }
            .onDisappear { clock.stop() }
    }
}
private struct PetView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var live: LiveSessionState
    init(model: AppModel) { self.model = model; live = model.live }
    @AppStorage("aieyes.pet.enabled") private var enabled = true
    @AppStorage("aieyes.pet.scale") private var savedScale = 1.0
    var body: some View {
        let state = live.pet
        let scale = PetAppearance.scale(savedScale)
        VStack(spacing: 0) {
            PetSpriteView(state: state, asset: model.importedPet, scale: scale, enabled: enabled)
            Text(state.label).font(.system(size: 12 * scale, weight: .medium))
                .multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 9 * scale).padding(.vertical, 3 * scale)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: DesignTokens.radiusOverlay * scale))
                .overlay(RoundedRectangle(cornerRadius: DesignTokens.radiusOverlay * scale).strokeBorder(Palette.cardBorder.opacity(DesignTokens.cardBorderOpacity)))
        }.frame(width: 144 * scale, height: state.height * scale)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Aieyes · " + state.label).accessibilityHint("单击打开面板，拖动移动，右键菜单")
    }
}
private final class PetPanel: NSPanel { override var canBecomeKey: Bool { false } }
private final class PetHost: NSHostingView<PetView> {
    var open: ((NSView, Bool) -> Void)?
    var settings: (() -> Void)?
    var petSettings: (() -> Void)?
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { bounds.contains(point) ? self : nil }
    private var press: (point: NSPoint, origin: NSPoint)?
    private var dragged = false
    override func mouseDown(with event: NSEvent) {
        guard let window else { return }
        press = (NSEvent.mouseLocation, window.frame.origin)
        dragged = false
    }
    override func mouseDragged(with event: NSEvent) {
        guard let window, let press else { return }
        let point = NSEvent.mouseLocation
        if hypot(point.x - press.point.x, point.y - press.point.y) > 4 { dragged = true }
        if dragged { window.setFrameOrigin(NSPoint(x: press.origin.x + point.x - press.point.x, y: press.origin.y + point.y - press.point.y)) }
    }
    override func mouseUp(with event: NSEvent) {
        guard let window, press != nil else { return }
        press = nil
        if dragged { PetWindowController.clamp(window, snap: true, preferredPoint: NSEvent.mouseLocation); UserDefaults.standard.set(NSStringFromPoint(window.frame.origin), forKey: "aieyes.pet.position") }
        else { open?(self, true) }
    }
    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = NSMenu()
        // NSHostingView validates SwiftUI commands and disables our custom selectors.
        menu.autoenablesItems = false
        let open = NSMenuItem(title: "打开面板", action: #selector(openPanel), keyEquivalent: ""); open.target=self; menu.addItem(open)
        open.isEnabled = self.open != nil
        let settings = NSMenuItem(title: "设置", action: #selector(openSettings), keyEquivalent: ""); settings.target = self; menu.addItem(settings)
        let petSettings = NSMenuItem(title: "桌宠设置", action: #selector(openPetSettings), keyEquivalent: ""); petSettings.target = self; menu.addItem(petSettings)
        let hide = NSMenuItem(title: "隐藏宠物（可在设置中恢复）", action: #selector(hidePet), keyEquivalent: ""); hide.target=self; menu.addItem(hide)
        return menu
    }
    override func rightMouseDown(with event: NSEvent) {
        if let menu = menu(for: event) { NSMenu.popUpContextMenu(menu, with: event, for: self) }
    }
    override func accessibilityPerformPress() -> Bool { open?(self, false); return true }
    @objc private func openPanel() { open?(self, false) }
    @objc private func openSettings() { settings?() }
    @objc private func openPetSettings() { petSettings?() }
    @objc private func hidePet() { UserDefaults.standard.set(false, forKey: "aieyes.pet.enabled") }
}
@MainActor final class PetWindowController {
    private let panel: NSPanel
    var window: NSWindow { panel }
    private var subscriptions = Set<AnyCancellable>()
    private var stateHeight: CGFloat = 168
    init(model: AppModel, open: @escaping (NSView, Bool) -> Void) {
        UserDefaults.standard.register(defaults: ["aieyes.pet.enabled": true, "aieyes.pet.scale": 1.0])
        panel = PetPanel(contentRect: NSRect(x: 0,y: 0,width: 144,height: 168), styleMask: [.borderless,.nonactivatingPanel], backing: .buffered, defer: false)
        panel.title="Aieyes 宠物"; panel.isOpaque=false; panel.backgroundColor = .clear; panel.hasShadow=false
        panel.level = .floating; panel.collectionBehavior=[.canJoinAllSpaces,.fullScreenAuxiliary]; panel.hidesOnDeactivate=false
        let host=PetHost(rootView: PetView(model:model)); host.sizingOptions=[]; host.open=open; panel.contentView=host
        host.settings = { [weak model] in model?.openSettings() }
        host.petSettings = { [weak model] in model?.openSettings(.init(category: "general", section: "desktop")) }
        host.setAccessibilityRole(.button); host.setAccessibilityLabel("Aieyes 宠物，单击打开面板")
        if let saved=UserDefaults.standard.string(forKey:"aieyes.pet.position") { panel.setFrameOrigin(NSPointFromString(saved)) }
        else if let screen=NSScreen.main { panel.setFrameOrigin(NSPoint(x:screen.visibleFrame.maxX-168,y:screen.visibleFrame.midY)) }
        model.live.$pet.sink { [weak self] state in
            guard let self else { return }
            self.stateHeight = state.height
            self.resize()
        }.store(in: &subscriptions)
        Self.clamp(panel); sync()
        NotificationCenter.default.publisher(for:UserDefaults.didChangeNotification).sink { [weak self] _ in self?.sync() }.store(in:&subscriptions)
        NotificationCenter.default.publisher(for:NSApplication.didChangeScreenParametersNotification).sink { [weak self] _ in if let self { Self.clamp(self.panel) } }.store(in:&subscriptions)
    }
    private func resize() {
        let scale = PetAppearance.scale(UserDefaults.standard.double(forKey: "aieyes.pet.scale"))
        let size = NSSize(width: 144 * scale, height: stateHeight * scale)
        guard panel.frame.size != size else { return }
        var frame = panel.frame
        frame.origin.y += frame.height - size.height
        frame.size = size
        panel.setFrame(frame, display: true)
        Self.clamp(panel)
    }
    private func sync() { resize(); if UserDefaults.standard.bool(forKey:"aieyes.pet.enabled") { panel.orderFrontRegardless() } else { panel.orderOut(nil) } }
    static func clamp(_ window: NSWindow, snap: Bool = false, preferredPoint: NSPoint? = nil) {
        window.setFrameOrigin(PetWindowGeometry.origin(frame: window.frame, areas: NSScreen.screens.map(\.visibleFrame), snap: snap, preferredPoint: preferredPoint))
    }
}

enum PetWindowGeometry {
    static func origin(frame: NSRect, areas: [NSRect], snap: Bool, preferredPoint: NSPoint? = nil) -> NSPoint {
        let center = NSPoint(x: frame.midX, y: frame.midY)
        let ranked = areas.sorted {
            let a = $0.intersection(frame), b = $1.intersection(frame)
            let left = a.isNull ? 0 : a.width * a.height, right = b.isNull ? 0 : b.width * b.height
            if left != right { return left > right }
            return hypot($0.midX - center.x, $0.midY - center.y) < hypot($1.midX - center.x, $1.midY - center.y)
        }
        guard let area = preferredPoint.flatMap({ point in areas.first { $0.contains(point) } }) ?? ranked.first else { return frame.origin }
        let right = max(area.minX, area.maxX - frame.width), top = max(area.minY, area.maxY - frame.height)
        var point = NSPoint(x: min(max(frame.minX, area.minX), right), y: min(max(frame.minY, area.minY), top))
        if snap {
            if point.x - area.minX <= 48 { point.x = area.minX } else if right - point.x <= 48 { point.x = right }
            if point.y - area.minY <= 48 { point.y = area.minY } else if top - point.y <= 48 { point.y = top }
        }
        return point
    }
}

private struct PetPreviewViewportKey: EnvironmentKey {
    static let defaultValue: CGRect? = nil
}
extension EnvironmentValues {
    var petPreviewViewport: CGRect? {
        get { self[PetPreviewViewportKey.self] }
        set { self[PetPreviewViewportKey.self] = newValue }
    }
}
enum PetPreviewVisibility {
    static func isVisible(_ frame: CGRect, viewport: CGRect, enclosing: CGRect?) -> Bool {
        let clipped = viewport.intersection(enclosing ?? viewport).intersection(frame)
        return !clipped.isNull && clipped.width > 0 && clipped.height > 0
    }
}

struct PetLibraryView: View {
    @ObservedObject var model: AppModel
    @State private var input = ""
    @Environment(\.surfaceActive) private var active
    private var height: CGFloat { min(260, max(100, CGFloat(model.petLibrary.count) * 96)) }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField("桌宠资源", text: $input, prompt: Text("粘贴资源链接、npx 命令或 npm 包名"), axis: .vertical).labelsHidden().textFieldStyle(.aieyes).lineLimit(2...4)
            HStack {
                Button(model.petImportBusy ? "导入中…" : "导入") { let value = input; Task { await model.importPet(input: value); if model.petImportError == nil && input == value { input = "" } } }.disabled(input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.petImportBusy || model.petSelectionBusy != nil)
                Button("选择本地文件…") {
                    let panel = NSOpenPanel(); panel.canChooseDirectories = false; panel.allowsMultipleSelection = false
                    panel.title = "选择桌宠 ZIP 或 pet.json"; panel.showsHiddenFiles = true
                    if panel.runModal() == .OK, let url = panel.url { Task { await model.importPet(input: url.path) } }
                }.disabled(model.petImportBusy || model.petSelectionBusy != nil)
                if model.petImportBusy || (model.petLibraryBusy && model.petLibrary.isEmpty) { ProgressView().controlSize(.small) }
            }
            Text("导入后立即使用，已有桌宠保留。").font(AppFont.secondary).foregroundStyle(.secondary)
            if let error = model.petImportError { ErrorNotice(message: error) }
            if let error = model.petLibraryError { HStack { ErrorNotice(message: error).foregroundStyle(Palette.warn); Button("重试读取") { Task { await model.loadPetLibrary() } } }.font(AppFont.secondary) }
            if !model.petLibrary.isEmpty {
                GeometryReader { viewport in
                    ScrollView { LazyVStack(alignment: .leading, spacing: 8) {
                        ForEach(model.petLibrary, id: \.libraryKey) { pet in
                            PetLibraryRow(model: model, pet: pet, viewport: viewport.frame(in: .global), playing: active)
                        }
                    }.padding(4) }
                }.frame(height: height)
            }
            ForEach(model.petLibraryWarnings) { warning in ErrorNotice(message: warning.path + "：" + warning.error).font(AppFont.secondary).foregroundStyle(Palette.warn).textSelection(.enabled) }
        }.task { await model.loadPetLibrary() }
    }
}

private struct PetLibraryRow: View {
    @ObservedObject var model: AppModel
    let pet: ImportedPet, viewport: CGRect, playing: Bool
    @Environment(\.petPreviewViewport) private var enclosingViewport
    @State private var visible = false
    private var selected: Bool { model.importedPet?.libraryKey == pet.libraryKey }
    var body: some View {
        HStack(spacing: 10) {
            Group { if visible && playing { PetSpriteView(state: .idle, asset: pet, scale: 0.5, enabled: true) } else { Color.clear.frame(width: 66, height: 71.5) } }.accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(pet.displayName).lineLimit(2).help(pet.displayName)
                if let creator = pet.creator, !creator.isEmpty { Text(creator).font(AppFont.secondary).foregroundStyle(.secondary) }
                if let source = pet.source, !source.isEmpty { Text(source).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle).help(source) }
                if let date = pet.importedAt { Text(date.replacingOccurrences(of: "T", with: " ").prefix(16)).font(AppFont.secondary).foregroundStyle(.secondary) }
            }.frame(maxWidth: .infinity, alignment: .leading)
            Button(selected ? "使用中" : model.petSelectionBusy == pet.libraryKey ? "切换中…" : "使用") { Task { await model.selectPet(pet) } }.disabled(selected || model.petImportBusy || model.petSelectionBusy != nil)
        }.padding(8).background(selected ? Palette.accent.opacity(0.08) : Color.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 8))
        .background { GeometryReader { geometry in
            let showing = PetPreviewVisibility.isVisible(geometry.frame(in: .global), viewport: viewport, enclosing: enclosingViewport)
            Color.clear.onChange(of: showing, initial: true) { _, next in visible = next }
        } }
        .onDisappear { visible = false }
    }
}
