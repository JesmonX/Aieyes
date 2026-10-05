import Foundation
import SwiftUI

// Only lifecycle metadata is retained; prompts and tool arguments are never displayed.
enum SessionPhase: String {
    case working = "进行中", thinking = "思考中", tool = "执行工具", complete = "已完成", interrupted = "已中断", unknown = "状态待确认"
    var active: Bool { self == .working || self == .thinking || self == .tool }
    var color: Color {
        switch self { case .working: return .teal; case .thinking: return .purple; case .tool: return .blue; case .complete: return .green; case .interrupted: return .orange; case .unknown: return .secondary }
    }
    var symbol: String {
        switch self { case .working: return "waveform"; case .thinking: return "sparkles"; case .tool: return "gearshape.2"; case .complete: return "checkmark"; case .interrupted: return "pause.fill"; case .unknown: return "questionmark" }
    }
}
struct LiveSession: Identifiable {
    var id: String, source: String, phase: SessionPhase, updatedAt: Date
    static func phase(in data: Data) -> SessionPhase? {
        var phase: SessionPhase?
        for line in data.split(separator: 10) {
            guard let row = try? JSONSerialization.jsonObject(with: Data(line)) as? [String: Any], let payload = row["payload"] as? [String: Any] else { continue }
            let type = payload["type"] as? String
            if row["type"] as? String == "event_msg" {
                switch type { case "task_started": phase = .working; case "task_complete": phase = .complete; case "turn_aborted": phase = .interrupted; default: break }
            } else if row["type"] as? String == "response_item", phase != .complete, phase != .interrupted {
                switch type {
                case "reasoning": phase = .thinking
                case "function_call", "custom_tool_call": phase = .tool
                case "function_call_output", "custom_tool_call_output": phase = .working
                default: break
                }
            }
        }
        return phase
    }
}
actor SessionMonitor {
    private var files: [(URL, String)] = []
    private var discovered = Date.distantPast
    private var sourceKey = ""
    private var discoveryFailed = false
    private var cache: [String: (Date, Int, LiveSession?)] = [:]
    func read(sources: [AgentSource], now: Date = Date()) -> (sessions: [LiveSession], unavailable: Bool) {
        let local = sources.filter { $0.enabled && $0.hostId == nil && $0.provider == "codex" }
        let key = local.map { $0.id + $0.path + $0.name }.joined(separator: "|")
        var unavailable = false
        if key != sourceKey || now.timeIntervalSince(discovered) > 30 {
            files = []; sourceKey = key; discovered = now
            var seen = Set<String>()
            for source in local {
                let root = URL(fileURLWithPath: (source.path as NSString).expandingTildeInPath)
                let sessions = root.appendingPathComponent("sessions")
                let directory = FileManager.default.fileExists(atPath: sessions.path) ? sessions : root
                if directory.pathExtension == "jsonl" {
                    if seen.insert(directory.path).inserted { files.append((directory, source.name)) }
                    continue
                }
                guard let enumerator = FileManager.default.enumerator(at: directory, includingPropertiesForKeys: [.contentModificationDateKey, .isRegularFileKey], options: [.skipsHiddenFiles], errorHandler: { _, _ in unavailable = true; return true }) else { unavailable = true; continue }
                for case let url as URL in enumerator where url.pathExtension == "jsonl" {
                    guard let values = try? url.resourceValues(forKeys: [.contentModificationDateKey, .isRegularFileKey]), values.isRegularFile == true, let modified = values.contentModificationDate, now.timeIntervalSince(modified) < 3600, seen.insert(url.path).inserted else { continue }
                    files.append((url, source.name))
                }
            }
            discoveryFailed = unavailable
            cache = cache.filter { item in files.contains { $0.0.path == item.key } }
        }
        unavailable = unavailable || discoveryFailed
        var result: [LiveSession] = []
        for (url, source) in files {
            guard let values = try? URL(fileURLWithPath: url.path).resourceValues(forKeys: [.contentModificationDateKey, .fileSizeKey]), let modified = values.contentModificationDate else { unavailable = true; continue }
            let age = now.timeIntervalSince(modified)
            guard age < 3600 else { continue }
            var session: LiveSession?
            if let old = cache[url.path], old.0 == modified, old.1 == values.fileSize ?? 0 { session = old.2 }
            else {
                do {
                    let file = try FileHandle(forReadingFrom: url); defer { try? file.close() }
                    let end = try file.seekToEnd(); try file.seek(toOffset: end > 262144 ? end - 262144 : 0)
                    let data = try file.readToEnd() ?? Data()
                    if let phase = LiveSession.phase(in: data) { session = LiveSession(id: url.path, source: source, phase: phase, updatedAt: modified) }
                    cache[url.path] = (modified, values.fileSize ?? 0, session)
                } catch { unavailable = true }
            }
            if var session {
                if session.phase.active && age > 180 { session.phase = .unknown }
                if session.phase.active || session.phase == .unknown || age < 90 { result.append(session) }
            }
        }
        return (result.sorted { $0.updatedAt > $1.updatedAt }, unavailable)
    }
}

struct ActivityIndicator: View {
    var phase: SessionPhase?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        Image(systemName: phase?.symbol ?? "circle.dotted")
            .font(.system(size: 14, weight: .semibold))
            .foregroundStyle(phase?.color ?? .secondary)
            .symbolEffect(.pulse, options: .repeating, isActive: phase?.active == true && !reduceMotion)
            .frame(width: 22, height: 22)
            .background((phase?.color ?? .secondary).opacity(0.12), in: Circle())
            .accessibilityLabel(phase?.rawValue ?? "无活跃会话")
    }
}
struct MenuActivityLabel: View {
    @ObservedObject var model: AppModel
    var body: some View {
        HStack(spacing: 5) {
            BrandMark(template: true).frame(width: 22, height: 22)
            ActivityIndicator(phase: model.sessionPhase)
            if model.activeSessions.count > 0 { Text("\(model.activeSessions.count)").monospacedDigit() }
            if !model.menuText.isEmpty { Text(model.menuText).monospacedDigit() }
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
        let image = NSImage(contentsOf: url) ?? NSImage(named: NSImage.applicationIconName) ?? NSImage()
        image.isTemplate = template
        return image
    }
    var body: some View { Image(nsImage: template ? Self.monochrome : Self.full).resizable().scaledToFit().accessibilityLabel("Aieyes") }
}
