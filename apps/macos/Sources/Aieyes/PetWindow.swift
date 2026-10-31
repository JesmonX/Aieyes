import AppKit
import SwiftUI
import Combine

enum PetAppearance {
    static func scale(_ value: Double) -> Double { value.isFinite ? min(2, max(0.5, value)) : 1 }
    static func labelHeight(_ label:String, scale:Double) -> CGFloat {
        let font = NSFont.systemFont(ofSize:max(11,12 * scale),weight:.medium)
        return ceil((L(label) as NSString).boundingRect(with:NSSize(width:126 * scale,height:1000),options:[.usesLineFragmentOrigin,.usesFontLeading],attributes:[.font:font]).height) + 6 * scale
    }
    static func height(_ state:PetState, scale:Double) -> CGFloat { 143 * scale + labelHeight(state.label,scale:scale) }

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

struct PetTimingControl: View {
    @AppStorage("aieyes.pet.uniformTiming") private var enabled = false
    var body: some View { DisclosureGroup(L("调试")) { Toggle(L("匀速动画（调试）"), isOn: $enabled).help(L("所有动作每帧 100ms，用于检查停顿")) } }
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
    private(set) var row = 0
    private(set) var assetIdentity = ""
    private var requested = ""
    func load(_ asset: ImportedPet?, row: Int) {
        let key = (asset?.contentHash ?? "") + ":" + String(row) + ":" + (asset?.spritePath ?? "")
        guard key != requested else { return }; requested = key
        guard let asset else { images = []; return }
        if let known = PetSprites.cached(asset, row: row) { self.row = row; assetIdentity = asset.contentHash; images = known; return }
        PetSprites.queue.async { [weak self] in
            let next = PetSprites.frames(for: asset, row: row)
            DispatchQueue.main.async { [weak self] in guard let self, self.requested == key else { return }; if !next.isEmpty { self.row = row; self.assetIdentity = asset.contentHash; self.images = next } }
        }
    }
}
// Discrete contents keyframes retain the original sprite pixels and exact variable durations.
final class PetLayerImage: NSView {
    private var images: [CGImage] = []
    private var identity = "", imageIDs: [ObjectIdentifier] = []
    private var masks: [[UInt8]] = []
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
            masks = images.map { image in
                var pixels = [UInt8](repeating:0,count:image.width * image.height * 4)
                pixels.withUnsafeMutableBytes { bytes in
                    if let context = CGContext(data:bytes.baseAddress,width:image.width,height:image.height,bitsPerComponent:8,bytesPerRow:image.width * 4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) { context.draw(image,in:CGRect(x:0,y:0,width:image.width,height:image.height)) }
                }
                return stride(from:3,to:pixels.count,by:4).map { pixels[$0] }
            }
            epoch = CACurrentMediaTime(); layer?.removeAnimation(forKey: "pet.frames")
            CATransaction.begin(); CATransaction.setDisableActions(true); layer?.contents = images.first; CATransaction.commit()
        }
        self.state = state; requested = playing; self.reduced = reduced; sync()
    }
    func opaque(at point:NSPoint) -> Bool {
        guard bounds.contains(point),!images.isEmpty,!masks.isEmpty,bounds.width > 0,bounds.height > 0 else { return false }
        let durations = state.durations
        var frame = 0
        if requested && !reduced && !durations.isEmpty {
            let total = durations.reduce(0,+)
            if total > 0 {
                var remaining = Int((CACurrentMediaTime()-epoch) * 1000) % total
                for (index,duration) in durations.enumerated() { if remaining < duration { frame = index;break };remaining -= duration }
            }
        }
        frame = min(frame,images.count-1)
        let image = images[frame]
        let x = min(image.width-1,max(0,Int(point.x / bounds.width * CGFloat(image.width))))
        let y = min(image.height-1,max(0,Int((bounds.height-point.y) / bounds.height * CGFloat(image.height))))
        return masks[frame][y * image.width + x] > 20
    }
    static func keyframes(images: [CGImage], durations: [Int]) -> CAKeyframeAnimation? {
        guard !durations.isEmpty, images.count >= durations.count else { return nil }
        let total = Double(durations.reduce(0,+))
        guard total > 0 else { return nil }
        let animation = CAKeyframeAnimation(keyPath: "contents")
        animation.values = Array(images.prefix(durations.count))
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
    @AppStorage("aieyes.pet.uniformTiming") private var uniformTiming = false
    private var displayedState: PetState {
        var value = state
        if value.row != frames.row {
            value.row = frames.row
            value.frames = CoreContract.petFrameDurations[frames.row].count
            value.frameDurationsMs = CoreContract.petFrameDurations[frames.row]
        }
        if uniformTiming { value.frameDurationsMs = Array(repeating: 100, count: value.frames) }
        return value
    }
    private func configure() { frames.load(asset, row: state.row) }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Group {
            if !frames.images.isEmpty {
                PetLayerSprite(images: frames.images, state: displayedState, identity: frames.assetIdentity + ":" + displayedState.animationIdentity, playing: enabled, reduced: reduced)
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
            Text(L(state.label)).font(.system(size: max(11,12 * scale), weight: .medium))
                .multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 9 * scale).padding(.vertical, 3 * scale)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: DesignTokens.radiusOverlay * scale))
                .overlay(RoundedRectangle(cornerRadius: DesignTokens.radiusOverlay * scale).strokeBorder(Palette.cardBorder.opacity(DesignTokens.cardBorderOpacity)))
        }.frame(width: 144 * scale, height: PetAppearance.height(state,scale:scale))
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
    override func hitTest(_ point: NSPoint) -> NSView? { opaque(at:point) || press != nil ? self : nil }
    var dragging:Bool { press != nil }
    func opaque(at point:NSPoint) -> Bool {
        guard bounds.contains(point) else { return false }
        func image(in view:NSView) -> PetLayerImage? { if let image = view as? PetLayerImage { return image };return view.subviews.compactMap { image(in:$0) }.first }
        if let sprite = image(in:self) {
            let local = sprite.convert(point,from:self)
            if sprite.bounds.contains(local) { return sprite.opaque(at:local) }
            // The measured status label is independently clickable.
            let rect = sprite.convert(sprite.bounds,to:self)
            return isFlipped ? point.y >= rect.maxY : point.y <= rect.minY
        }
        return true
    }
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
    private var petState = PetState.idle
    private var appliedPreferences: Preferences?
    private struct Preferences: Equatable { var enabled: Bool; var scale: Double; var language:String; var hideInFullscreen:Bool }
    private var mouseLocal:Any?, mouseGlobal:Any?, hitTimer:Timer?
    private var fullscreen = false

    init(model: AppModel, checkUpdates: @escaping () -> Void = {}, open: @escaping (NSView, Bool) -> Void) {
        self.model = model
        UserDefaults.standard.register(defaults: ["aieyes.pet.enabled": true, "aieyes.pet.scale": 1.0,"aieyes.pet.hideInFullscreen":true])
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
            self.petState = state
            self.resize()
        }.store(in: &subscriptions)
        NotificationCenter.default.publisher(for: NSWindow.didChangeOcclusionStateNotification, object: panel).sink { [weak self] _ in
            self?.publishVisibility()
        }.store(in: &subscriptions)
        mouseLocal = NSEvent.addLocalMonitorForEvents(matching:[.mouseMoved,.leftMouseDragged,.leftMouseUp]) { [weak self] event in self?.updateHit();return event }
        mouseGlobal = NSEvent.addGlobalMonitorForEvents(matching:[.mouseMoved,.leftMouseDragged,.leftMouseUp]) { [weak self] _ in DispatchQueue.main.async { self?.updateHit() } }
        Timer.publish(every:1,on:.main,in:.common).autoconnect().sink { [weak self] _ in self?.updateFullscreen() }.store(in:&subscriptions)
        NSWorkspace.shared.notificationCenter.publisher(for:NSWorkspace.activeSpaceDidChangeNotification).sink { [weak self] _ in self?.updateFullscreen() }.store(in:&subscriptions)
        // Respond to application switches immediately, rather than waiting for the fullscreen timer.
        NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.didActivateApplicationNotification).sink { [weak self] _ in
            guard let self else { return }
            self.updateFullscreen()
            if self.panel.isVisible {
                self.panel.level = .floating
                self.panel.orderFrontRegardless()
            }
        }.store(in: &subscriptions)
        Self.clamp(panel); sync()
        NotificationCenter.default.publisher(for:UserDefaults.didChangeNotification).sink { [weak self] _ in self?.sync() }.store(in:&subscriptions)
        NotificationCenter.default.publisher(for:NSApplication.didChangeScreenParametersNotification).sink { [weak self] _ in if let self { Self.clamp(self.panel) } }.store(in:&subscriptions)
    }
    private func resize() {
        let scale = PetAppearance.scale(UserDefaults.standard.double(forKey: "aieyes.pet.scale"))
        let size = NSSize(width: 144 * scale, height: PetAppearance.height(petState,scale:scale))
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
                                      scale: Double(PetAppearance.scale(UserDefaults.standard.double(forKey: "aieyes.pet.scale"))),
                                      language:UserDefaults.standard.string(forKey:"aieyes.language") ?? "zh-CN",
                                      hideInFullscreen:UserDefaults.standard.bool(forKey:"aieyes.pet.hideInFullscreen"))
        guard appliedPreferences != preferences else { return }
        appliedPreferences = preferences
        resize()
        panel.title = L("Aieyes 宠物")
        panel.contentView?.setAccessibilityLabel(L("Aieyes 宠物，单击打开面板"))
        updateFullscreen()
        if preferences.enabled && !fullscreen && !panel.isVisible { panel.orderFrontRegardless() }
        else if (!preferences.enabled || fullscreen) && panel.isVisible { panel.orderOut(nil) }
        publishVisibility()
    }
    private func updateHit() {
        guard let host = panel.contentView as? PetHost,panel.isVisible else { hitTimer?.invalidate();hitTimer = nil;return }
        let screenPoint = NSEvent.mouseLocation
        let inside = panel.frame.contains(screenPoint)
        let local = host.convert(panel.convertPoint(fromScreen:screenPoint),from:nil)
        panel.ignoresMouseEvents = !host.dragging && (!inside || !host.opaque(at:local))
        if inside && hitTimer == nil { hitTimer = Timer.scheduledTimer(withTimeInterval:0.05,repeats:true) { [weak self] _ in MainActor.assumeIsolated { self?.updateHit() } } }
        else if !inside && !host.dragging { hitTimer?.invalidate();hitTimer = nil }
    }
    private func updateFullscreen() {
        guard let prefs = appliedPreferences else { return }
        var hidden = false
        if prefs.enabled && prefs.hideInFullscreen, let screen = NSScreen.screens.first(where:{$0.frame.intersects(panel.frame)}), let display = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID {
            let area = CGDisplayBounds(display)
            let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
            hidden = windows.contains { row in
                guard (row[kCGWindowLayer as String] as? Int) == 0,
                    let value = row[kCGWindowBounds as String] as? NSDictionary,
                    let rect = CGRect(dictionaryRepresentation:value) else { return false }
                return abs(rect.minX-area.minX) < 2 && abs(rect.minY-area.minY) < 2 && rect.width >= area.width-2 && rect.height >= area.height-2
            }
        }
        fullscreen = hidden
        if prefs.enabled && !hidden { if !panel.isVisible { panel.orderFrontRegardless() };updateHit() }
        else { if panel.isVisible { panel.orderOut(nil) };hitTimer?.invalidate();hitTimer = nil }
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
