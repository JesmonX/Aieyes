import Foundation
import SwiftUI
import Combine

// Only lifecycle metadata is retained; prompts and tool arguments are never displayed.
enum SessionPhase: String {
    case idle = "空闲", working = "进行中", thinking = "思考中", tool = "执行工具", complete = "已完成", interrupted = "已中断", unknown = "状态待确认", needsInput = "等待输入", failed = "执行失败"
    var active: Bool { self == .working || self == .thinking || self == .tool || self == .needsInput }
    var color: Color {
        switch self { case .idle: return .secondary; case .working: return Palette.accent; case .thinking: return Palette.accent; case .tool: return Palette.accent; case .complete: return Palette.ok; case .interrupted: return Palette.warn; case .needsInput: return Palette.warn; case .failed: return Palette.danger; case .unknown: return .secondary }
    }
    var symbol: String {
        switch self { case .idle: return "circle.dotted"; case .working: return "waveform"; case .thinking: return "sparkles"; case .tool: return "gearshape.2"; case .complete: return "checkmark"; case .interrupted: return "pause.fill"; case .needsInput: return "hand.raised.fill"; case .failed: return "exclamationmark.triangle"; case .unknown: return "questionmark" }
    }
}
struct LiveSession: Identifiable, Decodable, Equatable {
    var provider = "codex", machine = "本机", machineId = "local", sourceId = "", turnId = ""
    var id: String, source: String, phase: SessionPhase, updatedAt: Date
    init(id: String, source: String, phase: SessionPhase, updatedAt: Date) {
        self.id = id; self.source = source; self.phase = phase; self.updatedAt = updatedAt
    }
    enum CodingKeys: String, CodingKey { case id, source, phase, updatedAt, provider, machine, machineId, sourceId, turnId }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        id = try row.decode(String.self, forKey: .id)
        source = try row.decode(String.self, forKey: .source)
        provider = try row.decodeIfPresent(String.self, forKey: .provider) ?? "codex"
        machine = try row.decodeIfPresent(String.self, forKey: .machine) ?? "本机"
        machineId = try row.decodeIfPresent(String.self, forKey: .machineId) ?? "local"
        sourceId = try row.decodeIfPresent(String.self, forKey: .sourceId) ?? ""
        turnId = try row.decodeIfPresent(String.self, forKey: .turnId) ?? ""
        let key = try row.decode(String.self, forKey: .phase)
        phase = ["working": .working, "thinking": .thinking, "tool": .tool, "complete": .complete, "interrupted": .interrupted, "unknown": .unknown, "needsInput": .needsInput, "failed": .failed][key] ?? .unknown
        updatedAt = Date(timeIntervalSince1970: try row.decode(Double.self, forKey: .updatedAt))
    }
}
struct SessionSnapshot: Decodable { var sessions: [LiveSession]; var unavailable: Bool; var pet: PetState? = nil; var summary: String? = nil; var phase: String? = nil; var sources: [SessionSourceHealth]? = nil }
struct SessionSourceHealth: Decodable, Identifiable, Equatable {
    var sourceId: String, provider: String, machineId: String, state: String
    var error: String?
    var hooks: String?
    var id: String { machineId + ":" + sourceId }
    var message: String { error ?? (state == "connecting" ? "正在连接…" : "状态暂不可用") }
}
struct SessionHookStatus: Decodable {
    var sourceId: String, provider: String, machineId: String, configPath: String, state: String
    var error: String?
    var label: String { ["installed":"已接入", "missing":"等待监测安装", "disabled":"已停用", "error":"接入失败"][state] ?? state }
}

struct ActivityIndicator: View {
    var phase: SessionPhase?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.surfaceActive) private var surfaceActive
    var body: some View {
        Image(systemName: phase?.symbol ?? "circle.dotted")
            .font(.system(size: 14, weight: .semibold))
            .foregroundStyle(phase?.color ?? .secondary)
            .symbolEffect(.pulse, options: .repeating, isActive: phase?.active == true && !reduceMotion && surfaceActive)
            .frame(width: 22, height: 22)
            .background((phase?.color ?? .secondary).opacity(0.12), in: Circle())
            .accessibilityLabel(phase?.rawValue ?? "无活跃会话")
    }
}
// AppKit exposes the same system pulse effect without SwiftUI's per-frame body updates.
struct MenuActivityIndicator: NSViewRepresentable {
    var phase: SessionPhase?
    var enabled = true
    @Environment(\.accessibilityReduceMotion) private var reduced
    @Environment(\.surfaceActive) private var active
    func makeNSView(context: Context) -> MenuActivityImage { MenuActivityImage() }
    func updateNSView(_ view: MenuActivityImage, context: Context) { view.configure(phase: phase, playing: enabled && active && !reduced) }
    static func dismantleNSView(_ view: MenuActivityImage, coordinator: ()) { view.stop() }
}
final class MenuActivityImage: NSView {
    private let icon = NSImageView()
    private var phase: SessionPhase?
    private var initialized = false, requested = false, playing = false
    private var visibility: AnyCancellable?
    private(set) var animationStarts = 0
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true; layer?.cornerRadius = 11
        icon.imageScaling = .scaleNone; icon.symbolConfiguration = .init(pointSize: 14, weight: .semibold)
        addSubview(icon)
    }
    convenience init() { self.init(frame: NSRect(x: 0, y: 0, width: 22, height: 22)) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func layout() { super.layout(); icon.frame = bounds }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow(); visibility = nil
        if let window { visibility = NotificationCenter.default.publisher(for: NSWindow.didChangeOcclusionStateNotification, object: window).sink { [weak self] _ in self?.sync() } }
        sync()
    }
    override func viewDidChangeEffectiveAppearance() { super.viewDidChangeEffectiveAppearance(); colors() }
    func configure(phase: SessionPhase?, playing: Bool) {
        if !initialized || self.phase != phase {
            self.phase = phase; initialized = true
            icon.image = NSImage(systemSymbolName: phase?.symbol ?? "circle.dotted", accessibilityDescription: phase?.rawValue ?? "无活跃会话")
            colors()
        }
        requested = playing && phase?.active == true; sync()
        setAccessibilityElement(true); setAccessibilityRole(.image); setAccessibilityLabel(phase?.rawValue ?? "无活跃会话")
    }
    private func colors() {
        let color = NSColor(phase?.color ?? .secondary)
        icon.contentTintColor = color; layer?.backgroundColor = color.withAlphaComponent(0.12).cgColor
    }
    private func sync() {
        let next = requested && window?.isVisible == true && window?.occlusionState.contains(.visible) == true
        guard next != playing else { return }; playing = next
        if next { icon.addSymbolEffect(.pulse, options: .repeating); animationStarts += 1 }
        else { icon.removeAllSymbolEffects(animated: false) }
    }
    func stop() { requested = false; sync() }
}
struct MenuStatusSnapshot: Equatable {
    var phase: SessionPhase?, count = 0, text = "", sampling = "", samplingCount = 0, attention = false, animationEnabled = true
}
@MainActor final class MenuStatusStore: ObservableObject {
    @Published private(set) var snapshot = MenuStatusSnapshot()
    private var subscriptions = Set<AnyCancellable>()
    private var pending = false
    private func refresh(_ model: AppModel) {
        let value = MenuStatusSnapshot(phase: model.sessionPhase, count: model.activeSessions.count, text: model.menuText, sampling: model.samplingSummary, samplingCount: model.runningEstimates.count, attention: model.samplingNeedsAttention, animationEnabled: !model.installingUpdate)
        if snapshot != value { snapshot = value }
    }
    private func schedule(_ model: AppModel) {
        guard !pending else { return }; pending = true
        DispatchQueue.main.async { [weak self, weak model] in
            guard let self, let model else { return }; self.pending = false; self.refresh(model)
        }
    }
    init(model: AppModel) {
        model.objectWillChange.sink { [weak self, weak model] _ in if let model { self?.schedule(model) } }.store(in: &subscriptions)
        model.live.objectWillChange.sink { [weak self, weak model] _ in if let model { self?.schedule(model) } }.store(in: &subscriptions)
        refresh(model)
    }
}
// Retained controls keep their local state, while hidden surfaces defer model publications.
@MainActor private final class ActiveModelObservation: ObservableObject {
    var active = true
    private var subscription: AnyCancellable?
    init(_ model: AppModel) {
        subscription = model.objectWillChange.sink { [weak self] _ in
            guard let self, self.active else { return }; self.objectWillChange.send()
        }
    }
}
@MainActor @propertyWrapper struct SurfaceModel: DynamicProperty {
    let wrappedValue: AppModel
    @StateObject private var observation: ActiveModelObservation
    @Environment(\.surfaceActive) private var active
    init(wrappedValue: AppModel) { self.wrappedValue = wrappedValue; _observation = StateObject(wrappedValue: ActiveModelObservation(wrappedValue)) }
    nonisolated mutating func update() { MainActor.assumeIsolated { observation.active = active } }
    var projectedValue: ObservedObject<AppModel>.Wrapper { ObservedObject(wrappedValue: wrappedValue).projectedValue }

}
struct WindowNavigationSnapshot: Equatable {
    var page: String, category: String, section: String, requested: String?, windows: Set<String>
}
@MainActor final class WindowNavigationStore: ObservableObject {
    @Published private(set) var snapshot: WindowNavigationSnapshot
    private var subscription: AnyCancellable?
    private var pending = false
    private static func value(_ model: AppModel) -> WindowNavigationSnapshot {
        WindowNavigationSnapshot(page: model.detailPage, category: model.settingsTab, section: model.generalPage, requested: model.requestedDetailPage, windows: model.visibleWindows)
    }
    init(model: AppModel) {
        snapshot = Self.value(model)
        subscription = model.objectWillChange.sink { [weak self, weak model] _ in
            guard let self, !self.pending else { return }; self.pending = true
            DispatchQueue.main.async { [weak self, weak model] in
                guard let self, let model else { return }; self.pending = false
                let next = Self.value(model); if self.snapshot != next { self.snapshot = next }
            }
        }
    }
}
// Retain pages and their state, but do not invalidate hidden surfaces on every model publication.
@MainActor final class SurfaceObservation: ObservableObject {
    private var subscription: AnyCancellable?
    private var pending = false
    init(model: AppModel, compact: Bool, page: String) {
        var wasActive = model.isWindowVisible(compact ? "panel" : "detail") && (compact || model.detailPage == page)
        subscription = model.objectWillChange.sink { [weak self, weak model] _ in
            guard let self, !self.pending else { return }; self.pending = true
            DispatchQueue.main.async { [weak self, weak model] in
                guard let self, let model else { return }; self.pending = false
                let active = model.isWindowVisible(compact ? "panel" : "detail") && (compact || model.detailPage == page)
                let transitioned = wasActive != active; wasActive = active
                if active || transitioned { self.objectWillChange.send() }
            }
        }
    }
}
struct MenuActivityLabel: View {
    @ObservedObject private var status: MenuStatusStore
    init(model: AppModel) { status = model.menuStatus }
    @AppStorage(NativePanelKey.showCount.rawValue) private var showCount = false
    var body: some View {
        let value = status.snapshot
        HStack(spacing: 5) {
            BrandMark(template: true).frame(width: 22, height: 22)
            MenuActivityIndicator(phase: value.phase, enabled: value.animationEnabled).frame(width: 22, height: 22)
            if showCount && value.count > 0 { Text("\(value.count)").monospacedDigit().padding(.horizontal, 5).background(.quaternary, in: Capsule()).accessibilityLabel("\(value.count) 个活跃会话") }
            if !value.text.isEmpty { Text(value.text).monospacedDigit() }
            if value.samplingCount > 0 {
                Image(systemName: value.attention ? "pause.circle.fill" : "record.circle")
                    .foregroundStyle(value.attention ? Palette.warn : Palette.ok).accessibilityLabel(value.sampling)
                if value.samplingCount > 1 { Text("\(value.samplingCount)").monospacedDigit() }
            }
        }.font(.system(size: 14, weight: .medium)).padding(.horizontal, 5)
    }
}

struct BrandMark: View {
    var template = false
    private static let full = load(template: false)
    private static let monochrome = load(template: true)
    private static func load(template: Bool) -> NSImage {
        let name = template ? "BrandTemplate" : "Brand"
        let sourceResources = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Resources")
        let url = Bundle.main.url(forResource: name, withExtension: "png") ?? sourceResources.appendingPathComponent(name + ".png")
        let image = NSImage(contentsOf: url) ?? NSImage(systemSymbolName: "eye", accessibilityDescription: "Aieyes") ?? NSImage()
        image.isTemplate = template
        return image
    }
    var body: some View { Image(nsImage: template ? Self.monochrome : Self.full).resizable().scaledToFit().accessibilityLabel("Aieyes") }
}

private struct SurfaceActiveKey: EnvironmentKey { static let defaultValue = true }
extension EnvironmentValues {
    var surfaceActive: Bool {
        get { self[SurfaceActiveKey.self] }
        set { self[SurfaceActiveKey.self] = newValue }
    }
}
struct SurfaceTimeContext { let date: Date }
struct SurfaceTimeline<Content: View>: View {
    var interval: TimeInterval
    @ViewBuilder var content: (SurfaceTimeContext) -> Content
    @Environment(\.surfaceActive) private var active
    var body: some View {
        if active { TimelineView(.periodic(from: .now, by: interval)) { content(SurfaceTimeContext(date: $0.date)) } }
        else { content(SurfaceTimeContext(date: .now)) }
    }
}

// Core-selected activity and shared sprite timing; no client-side priority rules.
struct PetState: Decodable, Equatable {
    var behavior: String, label: String, row: Int, frames: Int, frameMs: Double, key: String
    var frameDurationsMs: [Int]? = nil
    var activeCount: Int?
    var phaseCounts: [String: Int]?
    var height: CGFloat { 168 + CGFloat(max(0, label.components(separatedBy: "\n").count - 1)) * 18 }
    static let idle = PetState(behavior: "idle", label: "空闲", row: 0, frames: 6, frameMs: 100, key: "idle")
    static let unknown = PetState(behavior: "unknown", label: "状态不可用", row: 6, frames: 6, frameMs: 100, key: "unknown")
    var durations: [Int] {
        if let frameDurationsMs, frameDurationsMs.count == frames, frameDurationsMs.allSatisfy({ $0 > 0 }) { return frameDurationsMs }
        if frameDurationsMs?.isEmpty != false, CoreContract.petFrameDurations.indices.contains(row), CoreContract.petFrameDurations[row].count == frames { return CoreContract.petFrameDurations[row] }
        return Array(repeating: max(1, Int(frameMs.isFinite ? frameMs : 100)), count: max(1, frames))
    }
    var animationIdentity: String { "\(key):\(row):" + durations.map(String.init).joined(separator: ",") }
    func frame(elapsed: TimeInterval, reduced: Bool) -> Int {
        if reduced { return row * 8 }
        let values = durations, cycle = values.reduce(0, +)
        var position = max(0, elapsed.isFinite ? elapsed * 1000 : 0).truncatingRemainder(dividingBy: Double(cycle))
        for (index, duration) in values.enumerated() {
            if position < Double(duration) { return row * 8 + index }
            position -= Double(duration)
        }
        return row * 8
    }
}

struct ImportedPet: Decodable, Equatable, Identifiable {
    var id: String, displayName: String, contentHash: String, spritePath: String
    var spriteVersionNumber: Int, rows: Int, cellWidth: Int, cellHeight: Int
    var libraryId: String? = nil, importedAt: String? = nil, source: String? = nil, creator: String? = nil
    var libraryKey: String { libraryId ?? contentHash }
}
struct PetSelection: Decodable { var pet: ImportedPet?; var error: String? }
struct PetLibraryWarning: Decodable, Identifiable { var path: String, error: String; var id: String { path } }
struct PetLibrary: Decodable { var pets: [ImportedPet]; var selectedLibraryId: String?; var warnings: [PetLibraryWarning] }

// High-frequency lifecycle updates only invalidate the views that display them.
@MainActor final class LiveSessionState: ObservableObject {
    @Published var sessions: [LiveSession] = []
    @Published var sources: [SessionSourceHealth] = []
    @Published var unavailable = false
    @Published var petVisible = true
    @Published var suspended = false
    @Published var asset: ImportedPet?
    @Published var pet = PetState.idle
    @Published var summary: String?
}
