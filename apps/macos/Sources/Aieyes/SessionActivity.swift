import Foundation
import SwiftUI

// Only lifecycle metadata is retained; prompts and tool arguments are never displayed.
enum SessionPhase: String {
    case idle = "空闲", working = "进行中", thinking = "思考中", tool = "执行工具", complete = "已完成", interrupted = "已中断", unknown = "状态待确认"
    var active: Bool { self == .working || self == .thinking || self == .tool }
    var color: Color {
        switch self { case .idle: return .secondary; case .working: return Palette.accent; case .thinking: return Palette.accent; case .tool: return Palette.accent; case .complete: return Palette.ok; case .interrupted: return Palette.warn; case .unknown: return .secondary }
    }
    var symbol: String {
        switch self { case .idle: return "circle.dotted"; case .working: return "waveform"; case .thinking: return "sparkles"; case .tool: return "gearshape.2"; case .complete: return "checkmark"; case .interrupted: return "pause.fill"; case .unknown: return "questionmark" }
    }
}
struct LiveSession: Identifiable, Decodable {
    var id: String, source: String, phase: SessionPhase, updatedAt: Date
    init(id: String, source: String, phase: SessionPhase, updatedAt: Date) {
        self.id = id; self.source = source; self.phase = phase; self.updatedAt = updatedAt
    }
    enum CodingKeys: String, CodingKey { case id, source, phase, updatedAt }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        id = try row.decode(String.self, forKey: .id)
        source = try row.decode(String.self, forKey: .source)
        let key = try row.decode(String.self, forKey: .phase)
        phase = ["working": .working, "thinking": .thinking, "tool": .tool, "complete": .complete, "interrupted": .interrupted, "unknown": .unknown][key] ?? .unknown
        updatedAt = Date(timeIntervalSince1970: try row.decode(Double.self, forKey: .updatedAt))
    }
}
struct SessionSnapshot: Decodable { var sessions: [LiveSession]; var unavailable: Bool }

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
