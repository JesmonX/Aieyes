import AppKit
import SwiftUI
import Combine

enum PetAppearance {
    static func scale(_ value: Double) -> Double { value.isFinite ? min(2, max(0.5, value)) : 1 }
}
struct PetScaleControl: View {
    @AppStorage("aieyes.pet.scale") private var savedScale = 1.0
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        HStack {
            Text(L("桌宠缩放"))
            Slider(value: Binding(get: { PetAppearance.scale(savedScale) }, set: { savedScale = $0 }), in: 0.5...2, step: 0.05)
                .accessibilityLabel(L("桌宠缩放")).accessibilityValue("\(Int((PetAppearance.scale(savedScale) * 100).rounded()))%")
            Text("\(Int((PetAppearance.scale(savedScale) * 100).rounded()))%").monospacedDigit().frame(width: 44, alignment: .trailing)
            Button(L("恢复 100%")) { savedScale = 1 }.disabled(savedScale == 1)
        }
    }
}

private enum PetSprites {
    private final class Atlas {
        let image: CGImage, frames: [[CGImage]]
        init(_ image: CGImage, asset: ImportedPet) {
            self.image = image
            frames = (0..<asset.rows).map { row in (0..<8).compactMap { image.cropping(to: CGRect(x: $0 * asset.cellWidth, y: row * asset.cellHeight, width: asset.cellWidth, height: asset.cellHeight)) } }
        }
    }
    private static func key(_ asset: ImportedPet) -> NSString { (asset.spritePath + ":" + asset.contentHash) as NSString }
    static func cached(_ asset: ImportedPet, row: Int) -> [CGImage]? {
        guard let value = cache.object(forKey: key(asset)), value.frames.indices.contains(row) else { return nil }; return value.frames[row]
    }
    private static let cache: NSCache<NSString, Atlas> = { let cache = NSCache<NSString, Atlas>(); cache.totalCostLimit = 48 * 1024 * 1024; cache.countLimit = 8; return cache }()
    static let queue = DispatchQueue(label: "app.aieyes.pet.decode", qos: .utility)
    static func frames(for asset: ImportedPet, row: Int) -> [CGImage] {
        guard row >= 0, row < asset.rows else { return [] }
        let key = key(asset)
        let cg: CGImage
        if let known = cache.object(forKey: key) { cg = known.image }
        else {
            guard let image = NSImage(contentsOfFile: asset.spritePath), let decoded = image.cgImage(forProposedRect: nil, context: nil, hints: nil), decoded.width == asset.cellWidth * 8, decoded.height == asset.cellHeight * asset.rows else { return [] }
            cg = decoded; cache.setObject(Atlas(cg, asset: asset), forKey: key, cost: cg.bytesPerRow * cg.height)
        }
        return cached(asset, row: row) ?? (0..<8).compactMap { cg.cropping(to: CGRect(x: $0 * asset.cellWidth, y: row * asset.cellHeight, width: asset.cellWidth, height: asset.cellHeight)) }
    }
}
@MainActor private final class PetFrameStore: ObservableObject {
    @Published private(set) var images: [CGImage] = []
    private var requested = ""
    func load(_ asset: ImportedPet?, row: Int) {
        let key = (asset?.contentHash ?? "") + ":" + String(row) + ":" + (asset?.spritePath ?? "")
        guard key != requested else { return }; requested = key
        guard let asset else { images = []; return }
        if let known = PetSprites.cached(asset, row: row) { images = known; return }
        PetSprites.queue.async { [weak self] in
            let next = PetSprites.frames(for: asset, row: row)
            DispatchQueue.main.async { [weak self] in guard let self, self.requested == key else { return }; if !next.isEmpty { self.images = next } }
        }
    }
}
// Discrete contents keyframes retain the original sprite pixels and exact variable durations.
final class PetLayerImage: NSView {
    private var images: [CGImage] = []
    private var identity = "", imageIDs: [ObjectIdentifier] = []
    private var state = PetState.idle
    private var requested = false, reduced = false
    private var epoch = CACurrentMediaTime()
    private var visibility: AnyCancellable?
    private(set) var animationStarts = 0
    override init(frame: NSRect) {
        super.init(frame: frame); wantsLayer = true
        layer?.contentsGravity = .resize; layer?.magnificationFilter = .linear; layer?.minificationFilter = .trilinear
    }
    convenience init() { self.init(frame: .zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow(); visibility = nil
        if let window { visibility = NotificationCenter.default.publisher(for: NSWindow.didChangeOcclusionStateNotification, object: window).sink { [weak self] _ in self?.sync() } }
        sync()
    }
    func configure(images: [CGImage], state: PetState, identity: String, playing: Bool, reduced: Bool) {
        let ids = images.map(ObjectIdentifier.init)
        if self.identity != identity || imageIDs != ids {
            self.identity = identity; imageIDs = ids; self.images = images
            epoch = CACurrentMediaTime(); layer?.removeAnimation(forKey: "pet.frames")
            CATransaction.begin(); CATransaction.setDisableActions(true); layer?.contents = images.first; CATransaction.commit()
        }
        self.state = state; requested = playing; self.reduced = reduced; sync()
    }
    static func keyframes(images: [CGImage], durations: [Int]) -> CAKeyframeAnimation? {
        guard !durations.isEmpty, images.count >= durations.count else { return nil }
        let total = Double(durations.reduce(0,+))
        guard total > 0 else { return nil }
        let animation = CAKeyframeAnimation(keyPath: "contents")
        animation.values = Array(images.prefix(durations.count)) + [images[0]]
        var cumulative = 0.0
        animation.keyTimes = [0] + durations.map { cumulative += Double($0); return NSNumber(value: cumulative / total) }
        animation.duration = total / 1000; animation.calculationMode = .discrete
        animation.timingFunction = CAMediaTimingFunction(name: .linear)
        animation.repeatCount = .infinity; animation.isRemovedOnCompletion = false
        return animation
    }
    private func sync() {
        let visible = window?.isVisible == true && window?.occlusionState.contains(.visible) == true
        guard requested, !reduced, visible, let layer, let animation = Self.keyframes(images: images, durations: state.durations) else { stop(); return }
        guard layer.animation(forKey: "pet.frames") == nil else { return }
        animation.beginTime = layer.convertTime(CACurrentMediaTime(), from: nil) - (CACurrentMediaTime()-epoch).truncatingRemainder(dividingBy: animation.duration)
        layer.add(animation, forKey: "pet.frames"); animationStarts += 1
    }
    func stop() { layer?.removeAnimation(forKey: "pet.frames") }
}
private struct PetLayerSprite: NSViewRepresentable {
    let images: [CGImage], state: PetState, identity: String, playing: Bool, reduced: Bool
    func makeNSView(context: Context) -> PetLayerImage { PetLayerImage() }
    func updateNSView(_ view: PetLayerImage, context: Context) { view.configure(images: images, state: state, identity: identity, playing: playing, reduced: reduced) }
    static func dismantleNSView(_ view: PetLayerImage, coordinator: ()) { view.stop() }
}
private struct PetSpriteView: View {
    let state: PetState
    let asset: ImportedPet?
    let scale: Double
    let enabled: Bool
    @Environment(\.accessibilityReduceMotion) private var reduced
    @StateObject private var frames = PetFrameStore()
    private func configure() { frames.load(asset, row: state.row) }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Group {
            if !frames.images.isEmpty {
                PetLayerSprite(images: frames.images, state: state, identity: (asset?.contentHash ?? "") + ":" + state.animationIdentity, playing: enabled, reduced: reduced)
            } else if asset == nil { Text(L("请导入桌宠")).font(.system(size: 13 * scale)) } else { Color.clear }
        }.frame(width: 132 * scale, height: 143 * scale)
            .onAppear { configure() }
            .onChange(of: state.row) { _, _ in configure() }
            .onChange(of: asset) { _, _ in configure() }
    }
}
private struct PetView: View {
    let model: AppModel
    @ObservedObject var live: LiveSessionState
    init(model: AppModel) { self.model = model; live = model.live }
    @AppStorage("aieyes.pet.enabled") private var enabled = true
    @AppStorage("aieyes.pet.scale") private var savedScale = 1.0
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        let state = live.pet
        let scale = PetAppearance.scale(savedScale)
        VStack(spacing: 0) {
            PetSpriteView(state: state, asset: live.asset, scale: scale, enabled: enabled && live.petVisible && !live.suspended)
            Text(L(state.label)).font(.system(size: 12 * scale, weight: .medium))
                .multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 9 * scale).padding(.vertical, 3 * scale)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: DesignTokens.radiusOverlay * scale))
                .overlay(RoundedRectangle(cornerRadius: DesignTokens.radiusOverlay * scale).strokeBorder(Palette.cardBorder.opacity(DesignTokens.cardBorderOpacity)))
        }.frame(width: 144 * scale, height: state.height * scale)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Aieyes · " + L(state.label)).accessibilityHint(L("单击打开面板，拖动移动，右键菜单"))
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
    var update: (() -> Void)?
    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = NSMenu()
        // NSHostingView validates SwiftUI commands and disables our custom selectors.
        menu.autoenablesItems = false
        for (title, action) in NativeContextMenu.entries(pet: true, visible: true) {
            if action == "separator" { menu.addItem(.separator()); continue }
            let selector: Selector = switch action {
            case "open-panel": #selector(openPanel)
            case "settings": #selector(openSettings)
            case "pet-settings": #selector(openPetSettings)
            case "check-updates": #selector(checkUpdates)
            default: #selector(hidePet)
            }
            let item = menu.addItem(withTitle: title, action: selector, keyEquivalent: "")
            item.target = self
            if action == "open-panel" { item.isEnabled = open != nil }
        }
        return menu
    }
    override func rightMouseDown(with event: NSEvent) {
        if let menu = menu(for: event) { NSMenu.popUpContextMenu(menu, with: event, for: self) }
    }
    override func accessibilityPerformPress() -> Bool { open?(self, false); return true }
    @objc private func openPanel() { open?(self, false) }
    @objc private func openSettings() { settings?() }
    @objc private func openPetSettings() { petSettings?() }
    @objc private func checkUpdates() { update?() }
    @objc private func hidePet() { UserDefaults.standard.set(false, forKey: "aieyes.pet.enabled") }
}
@MainActor final class PetWindowController {
    private let panel: NSPanel
    private weak var model: AppModel?
    var window: NSWindow { panel }
    private var subscriptions = Set<AnyCancellable>()
    private var stateHeight: CGFloat = 168
    private var appliedPreferences: Preferences?
    private struct Preferences: Equatable { var enabled: Bool; var scale: Double }
    init(model: AppModel, checkUpdates: @escaping () -> Void = {}, open: @escaping (NSView, Bool) -> Void) {
        self.model = model
        UserDefaults.standard.register(defaults: ["aieyes.pet.enabled": true, "aieyes.pet.scale": 1.0])
        panel = PetPanel(contentRect: NSRect(x: 0,y: 0,width: 144,height: 168), styleMask: [.borderless,.nonactivatingPanel], backing: .buffered, defer: false)
        panel.title=L("Aieyes 宠物"); panel.isOpaque=false; panel.backgroundColor = .clear; panel.hasShadow=false
        panel.level = .floating; panel.collectionBehavior=[.canJoinAllSpaces,.fullScreenAuxiliary]; panel.hidesOnDeactivate=false
        let host=PetHost(rootView: PetView(model:model)); host.sizingOptions=[]; host.open=open; panel.contentView=host
        host.update = checkUpdates
        host.settings = { [weak model] in model?.openSettings(.init(category: "general")) }
        host.petSettings = { [weak model] in model?.openSettings(.init(category: "general", section: "desktop")) }
        host.setAccessibilityRole(.button); host.setAccessibilityLabel(L("Aieyes 宠物，单击打开面板"))
        if let saved=UserDefaults.standard.string(forKey:"aieyes.pet.position") { panel.setFrameOrigin(NSPointFromString(saved)) }
        else if let screen=NSScreen.main { panel.setFrameOrigin(NSPoint(x:screen.visibleFrame.maxX-168,y:screen.visibleFrame.midY)) }
        model.live.$pet.sink { [weak self] state in
            guard let self else { return }
            self.stateHeight = state.height
            self.resize()
        }.store(in: &subscriptions)
        NotificationCenter.default.publisher(for: NSWindow.didChangeOcclusionStateNotification, object: panel).sink { [weak self] _ in
            self?.publishVisibility()
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
    private func publishVisibility() {
        let visible = panel.isVisible && panel.occlusionState.contains(.visible)
        if let model, model.live.petVisible != visible { model.live.petVisible = visible }
    }
    private func sync() {
        let preferences = Preferences(enabled: UserDefaults.standard.bool(forKey: "aieyes.pet.enabled"),
                                      scale: Double(PetAppearance.scale(UserDefaults.standard.double(forKey: "aieyes.pet.scale"))))
        guard appliedPreferences != preferences else { return }
        appliedPreferences = preferences
        resize()
        if preferences.enabled && !panel.isVisible { panel.orderFrontRegardless() }
        else if !preferences.enabled && panel.isVisible { panel.orderOut(nil) }
        publishVisibility()
    }
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
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 10) {
            if !model.petLibrary.isEmpty {
                GeometryReader { viewport in
                    ScrollView { LazyVStack(alignment: .leading, spacing: 8) {
                        ForEach(model.petLibrary, id: \.libraryKey) { pet in
                            PetLibraryRow(model: model, pet: pet, viewport: viewport.frame(in: .global), playing: active)
                        }
                    }.padding(4) }
                }.frame(height: height)
            }
            if let error = model.petImportError { ErrorNotice(message: error) }
            if let error = model.petLibraryError { HStack { ErrorNotice(message: error).foregroundStyle(Palette.warn); Button(L("重试读取")) { Task { await model.loadPetLibrary() } } }.font(AppFont.secondary) }
            ForEach(model.petLibraryWarnings) { warning in ErrorNotice(message: warning.path + "：" + warning.error).font(AppFont.secondary).foregroundStyle(Palette.warn).textSelection(.enabled) }
            DisclosureGroup(L("导入桌宠")) {
            TextField(L("桌宠资源"), text: $input, prompt: Text(L("粘贴资源链接、npx 命令或 npm 包名")), axis: .vertical).labelsHidden().textFieldStyle(.aieyes).lineLimit(2...4)
            HStack {
                Button(model.petImportBusy ? L("导入中…") : L("导入并使用")) { let value = input; Task { await model.importPet(input: value); if model.petImportError == nil && input == value { input = "" } } }.disabled(input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.petImportBusy || model.petSelectionBusy != nil)
                Button(L("选择本地文件…")) {
                    let panel = NSOpenPanel(); panel.canChooseDirectories = false; panel.allowsMultipleSelection = false
                    panel.title = L("选择桌宠 ZIP 或 pet.json"); panel.showsHiddenFiles = true
                    if panel.runModal() == .OK, let url = panel.url { Task { await model.importPet(input: url.path) } }
                }.disabled(model.petImportBusy || model.petSelectionBusy != nil)
                HelpNote(title: L("桌宠资源"), text: L("支持 ZIP、pet.json、桌宠链接、npx 命令和 npm 包名。导入后立即使用，已有桌宠保留。"))
                if model.petImportBusy || (model.petLibraryBusy && model.petLibrary.isEmpty) { ProgressView().controlSize(.small) }
            }
            }
        }.task { await model.loadPetLibrary() }
    }
}

private struct PetLibraryRow: View {
    @ObservedObject var model: AppModel
    let pet: ImportedPet, viewport: CGRect, playing: Bool
    @Environment(\.petPreviewViewport) private var enclosingViewport
    @State private var visible = false
    private var selected: Bool { model.importedPet?.libraryKey == pet.libraryKey }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        HStack(spacing: 10) {
            Group { if visible && playing { PetSpriteView(state: .idle, asset: pet, scale: 0.5, enabled: true) } else { Color.clear.frame(width: 66, height: 71.5) } }.accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(pet.displayName).lineLimit(2).help(pet.displayName)
                if let creator = pet.creator, !creator.isEmpty { Text(creator).font(AppFont.secondary).foregroundStyle(.secondary) }
                if let source = pet.source, !source.isEmpty { Text(source).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle).help(source) }
                if let date = pet.importedAt { Text(date.replacingOccurrences(of: "T", with: " ").prefix(16)).font(AppFont.secondary).foregroundStyle(.secondary) }
            }.frame(maxWidth: .infinity, alignment: .leading)
            Button(selected ? L("使用中") : model.petSelectionBusy == pet.libraryKey ? L("切换中…") : L("使用")) { Task { await model.selectPet(pet) } }.disabled(selected || model.petImportBusy || model.petSelectionBusy != nil)
        }.padding(8).background(selected ? Palette.accent.opacity(0.08) : Color.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 8))
        .background { GeometryReader { geometry in
            let showing = PetPreviewVisibility.isVisible(geometry.frame(in: .global), viewport: viewport, enclosing: enclosingViewport)
            Color.clear.onChange(of: showing, initial: true) { _, next in visible = next }
        } }
        .onDisappear { visible = false }
    }
}
