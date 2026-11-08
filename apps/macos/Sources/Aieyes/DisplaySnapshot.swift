import Foundation

// This small, versioned payload is also compiled into the WidgetKit extension.
// It intentionally contains no account identifiers, paths, credentials or session text.
enum DisplayPhase: String, Codable, CaseIterable {
    case idle, working, thinking, tool, needsInput, complete, failed, unknown

    var title: String {
        switch self {
        case .idle: return "空闲"
        case .working: return "进行中"
        case .thinking: return "思考中"
        case .tool: return "执行工具"
        case .needsInput: return "等待输入"
        case .complete: return "已完成"
        case .failed: return "执行失败"
        case .unknown: return "状态待确认"
        }
    }
    var symbol: String {
        switch self {
        case .idle: return "moon.zzz"
        case .working: return "waveform"
        case .thinking: return "sparkles"
        case .tool: return "gearshape.2"
        case .needsInput: return "hand.raised.fill"
        case .complete: return "checkmark"
        case .failed: return "exclamationmark.triangle.fill"
        case .unknown: return "wifi.slash"
        }
    }
    var isAnimated: Bool { [.working, .thinking, .tool, .needsInput].contains(self) }
}

struct DisplayQuota: Codable, Equatable, Identifiable {
    var provider: String
    var title: String
    var remaining: Double
    var resetsAt: TimeInterval?
    var stale = false
    var id: String { provider + ":" + title }
    var fraction: Double { remaining.isFinite ? min(1, max(0, remaining / 100)) : 0 }
}

struct DisplaySnapshot: Codable, Equatable {
    static let schemaVersion = 1
    static let widgetKind = "AieyesOverview"
    static let staleAfter: TimeInterval = 120
    var version = Self.schemaVersion
    var capturedAt = Date().timeIntervalSince1970
    var language = "zh-CN"
    var isDemo = false
    var appRunning = true
    var phase = DisplayPhase.idle
    var activeCount = 0
    var waitingCount = 0
    var failedCount = 0
    var totalTokens: Double?
    var costUSD: Double?
    var cachePercent: Double?
    var usageDay: TimeInterval = Calendar.current.startOfDay(for: Date()).timeIntervalSince1970
    var usageUnavailable = false
    var sessionsUnavailable = false
    var quotas: [DisplayQuota] = []

    func text(_ source: String) -> String { language == "en" ? LocalizationCatalog.english[source] ?? source : source }
    func isStale(at date: Date) -> Bool { !appRunning || date.timeIntervalSince1970 - capturedAt >= Self.staleAfter || capturedAt > date.timeIntervalSince1970 + 60 }
    func phase(at date: Date) -> DisplayPhase { isStale(at: date) || sessionsUnavailable ? .unknown : phase }
    func tokens(at date: Date) -> Double? {
        guard Calendar.current.isDate(Date(timeIntervalSince1970: usageDay), inSameDayAs: date) else { return nil }
        return totalTokens
    }
    func status(at date: Date, live: Bool) -> String {
        if !appRunning { return text("应用已退出") }
        if isStale(at: date) { return text("快照待更新") }
        if isDemo { return text("演示") }
        if usageUnavailable { return text("用量待更新") }
        return text(live ? "实时" : "快照")
    }
    static func compact(_ value: Double?) -> String {
        guard let value, value.isFinite, value >= 0 else { return "—" }
        for (threshold, suffix) in [(1_000_000_000.0, "B"), (1_000_000.0, "M"), (1_000.0, "K")] where value >= threshold {
            return String(format: value / threshold >= 100 ? "%.0f%@" : "%.1f%@", value / threshold, suffix)
        }
        return String(format: "%.0f", value)
    }
    static func sample(_ phase: DisplayPhase = .working, language: String = "zh-CN", now: Date = Date()) -> Self {
        var value = Self()
        value.capturedAt = now.timeIntervalSince1970
        value.usageDay = Calendar.current.startOfDay(for: now).timeIntervalSince1970
        value.language = language; value.isDemo = true; value.phase = phase
        value.activeCount = phase.isAnimated ? 3 : 0
        value.waitingCount = phase == .needsInput ? 1 : 0
        value.failedCount = phase == .failed ? 1 : 0
        value.sessionsUnavailable = phase == .unknown
        value.totalTokens = 1_284_600; value.costUSD = 8.42; value.cachePercent = 73
        value.quotas = [
            DisplayQuota(provider: "Codex", title: "5h", remaining: 72, resetsAt: now.addingTimeInterval(2_640).timeIntervalSince1970),
            DisplayQuota(provider: "Codex", title: "7d", remaining: 43, resetsAt: now.addingTimeInterval(172_800).timeIntervalSince1970)
        ]
        return value
    }
    static var empty: Self {
        var value = Self(); value.appRunning = false; value.phase = .unknown
        return value
    }
}

enum DisplaySnapshotFile {
    // Local development builds use a read-only sandbox exception for this single
    // directory. A signed distribution may supply an app-group container instead.
    static func url(bundle: Bundle = .main) -> URL? {
        guard let path = bundle.object(forInfoDictionaryKey: "AieyesWidgetSnapshotPath") as? String, !path.isEmpty else { return nil }
        return URL(fileURLWithPath: path)
    }
    static func write(_ snapshot: DisplaySnapshot, to url: URL) throws {
        let directory = url.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let data = try JSONEncoder().encode(snapshot)
        try data.write(to: url, options: [.atomic])
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }
    static func read(from url: URL?) -> DisplaySnapshot {
        guard let url, let data = try? Data(contentsOf: url), data.count <= 65_536,
              let value = try? JSONDecoder().decode(DisplaySnapshot.self, from: data),
              value.version == DisplaySnapshot.schemaVersion, value.capturedAt.isFinite,
              value.activeCount >= 0, value.waitingCount >= 0, value.failedCount >= 0,
              value.quotas.count <= 2, value.quotas.allSatisfy({ $0.remaining.isFinite }) else { return .empty }
        return value
    }
}
