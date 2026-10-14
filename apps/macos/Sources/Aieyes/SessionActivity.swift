import Foundation
import SwiftUI

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
    var label: String { ["installed":"已接入", "missing":"等待自动接入", "disabled":"已停用", "error":"接入失败"][state] ?? state }
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
struct MenuActivityLabel: View {
    @ObservedObject var model: AppModel
    @ObservedObject var live: LiveSessionState
    init(model: AppModel) { self.model = model; live = model.live }
    @AppStorage(NativePanelKey.showCount.rawValue) private var showCount = false
    var body: some View {
        HStack(spacing: 5) {
            BrandMark(template: true).frame(width: 22, height: 22)
            ActivityIndicator(phase: model.sessionPhase)
            if showCount && model.activeSessions.count > 0 { Text("\(model.activeSessions.count)").monospacedDigit().padding(.horizontal, 5).background(.quaternary, in: Capsule()).accessibilityLabel("\(model.activeSessions.count) 个活跃会话") }
            if !model.menuText.isEmpty { Text(model.menuText).monospacedDigit() }
            if !model.runningEstimates.isEmpty {
                Image(systemName: model.samplingNeedsAttention ? "pause.circle.fill" : "record.circle")
                    .foregroundStyle(model.samplingNeedsAttention ? Palette.warn : Palette.ok)
                    .accessibilityLabel(model.samplingSummary)
                if model.runningEstimates.count > 1 { Text("\(model.runningEstimates.count)").monospacedDigit() }
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
    @Published var pet = PetState.idle
    @Published var summary: String?
}
