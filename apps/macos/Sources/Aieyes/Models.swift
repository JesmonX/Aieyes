import Foundation

struct Tokens: Codable {
    var input: Double = 0, output: Double = 0, cacheRead: Double = 0, cacheWrite: Double = 0, reasoning: Double = 0
    var allInput: Double { input + cacheRead + cacheWrite }
    var cacheRate: Double? { allInput > 0 ? cacheRead / allInput : nil }
}
struct Aggregate: Codable, Identifiable {
    var key = "", tokens = Tokens(), total: Double = 0, cost: Double = 0, pricedTokens: Double = 0, events: Int = 0
    var id: String { key }
}
struct QuotaWindow: Codable, Identifiable {
    var name: String, usedPercent: Double, windowMinutes: Int?, resetsAt: Double?
    var stableId: String?, groupId: String?, groupName: String?
    var id: String { stableId.flatMap { $0.isEmpty ? nil : $0 } ?? name }
    var groupLabel: String { groupName.flatMap { $0.isEmpty ? nil : $0 } ?? name.components(separatedBy: " · ").dropLast().joined(separator: " · ") }
    enum CodingKeys: String, CodingKey { case name, usedPercent, windowMinutes, resetsAt, groupId, groupName; case stableId = "id" }
}
struct ResetCredit: Codable, Identifiable {
    var id: String, status: String, expiresAt: Double?, grantedAt: Double?, title: String?
}
struct BankReset: Codable { var availableCount: Int, credits: [ResetCredit]? }
struct Balance: Codable, Identifiable {
    var currency: String, total: String, granted: String, toppedUp: String
    var id: String { currency }
}
struct CreditsBalance: Codable { var hasCredits: Bool, unlimited: Bool, balance: String? }
struct Quota: Codable, Identifiable {
    var credits: CreditsBalance?, creditsUpdatedAt: Double?
    var sourceId: String, accountId: String, provider: String, name: String, updatedAt: Double, origin: String
    var balances: [Balance]?, isAvailable: Bool?
    var windows: [QuotaWindow], bankReset: BankReset?, bankUpdatedAt: Double?, plan: String?, error: String?
    var id: String { provider + ":" + accountId }
}
struct SourceStatus: Codable { var updatedAt: Double?, newEvents: Int?, files: Int?, readBytes: Int?, malformedLines: Int?, error: String? }
struct SourceSummary: Codable, Identifiable { var id: String, name: String, provider: String, enabled: Bool, status: SourceStatus?, accountIds: [String]? }
struct DayModel: Codable, Identifiable {
    var day: String, model: String, usage: Aggregate
    var id: String { day + ":" + model }
}
struct PricingGap: Codable, Identifiable {
    var model: String, priceId: String?, tokens: Tokens, unpricedTokens: Double
    var id: String { model }
    var missing: String { [("输入", tokens.input), ("输出", tokens.output), ("缓存读取", tokens.cacheRead), ("缓存写入", tokens.cacheWrite)].filter { $0.1 > 0 }.map { $0.0 }.joined(separator: "、") }
}
struct AgentAccount: Codable, Equatable, Identifiable {
    var id = UUID().uuidString, name = "", provider = "codex", quotaEnabled = true
    var quotaSourceId: String?
    var archived: Bool?
    var key: String { provider + ":" + id }
}
struct Dashboard: Codable {
    var generatedAt: Double = 0, summary = Aggregate(), days: [Aggregate] = [], heatmap: [Aggregate] = [], models: [Aggregate] = []
    var trendDays: [Aggregate] = [], dayModels: [DayModel] = [], pricingGaps: [PricingGap] = []
    var modelOptions: [String]? = nil
    var quotaOrder: [String]? = nil, quotaEstimates: [QuotaEstimate]? = nil, creditEstimates: [QuotaEstimate]? = nil
    var quotas: [Quota] = [], sources: [SourceSummary] = [], priceUpdatedAt: Double?
}
struct ProxySettings: Codable, Equatable { var mode = "system", url = "" }
struct ProxyAddressDraft: Equatable {
    var scheme = "http", host = "127.0.0.1", port = "7890", customURL = ""
    init(url: String) {
        customURL = url
        guard !url.isEmpty else { return }
        guard let parts = URLComponents(string: url), let scheme = parts.scheme, ["http", "https", "socks5", "socks5h"].contains(scheme), let host = parts.host, !host.isEmpty,
              parts.user == nil, parts.password == nil, parts.query == nil, parts.fragment == nil, ["", "/"].contains(parts.path) else { self.scheme = "url"; return }
        self.scheme = scheme; self.host = host.trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
        self.port = parts.port.map(String.init) ?? (scheme == "http" ? "80" : scheme == "https" ? "443" : "1080")
    }
    var url: String {
        if scheme == "url" { return customURL }
        let hostname = host.trimmingCharacters(in: .whitespacesAndNewlines).trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
        guard !hostname.isEmpty, !hostname.contains(where: { $0.isWhitespace || "/@?#".contains($0) }), let number = Int(port.trimmingCharacters(in: .whitespacesAndNewlines)), (1...65535).contains(number) else { return "invalid-proxy-address" }
        return scheme + "://" + (hostname.contains(":") ? "[" + hostname + "]" : hostname) + ":" + port.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
struct AgentSource: Codable, Equatable, Identifiable {
    var id = UUID().uuidString, name = "", provider = "codex", accountId = ""
    var path = "~/.codex", hostId: String?, enabled = true, quotaCommand = "", quotaPreCommand = "", codexBinary = "codex", agyBinary: String? = "agy", proxy: ProxySettings?
}
struct Host: Codable, Equatable, Identifiable {
    var id = UUID().uuidString, name = "", target = "", port: Int?, identityFile = "", shell = "/bin/sh", preCommand = "", enabled = true
    var metrics = ["cpu", "memory", "gpu", "filesystems", "disk", "network"], devices: [String] = []
    var details: [String]? = Host.detailOptions.map { $0.0 }
    static let detailOptions = [("cpuTimes", "CPU 时间分布"), ("memoryCache", "内存缓存 / Buffer"), ("swap", "Swap"), ("fsAvailable", "文件系统可用空间"), ("fsType", "文件系统类型 / 设备"), ("inodes", "inode"), ("diskIops", "磁盘 IOPS"), ("diskBusy", "磁盘忙碌率"), ("networkTotals", "累计流量"), ("networkErrors", "网络错误 / 丢包"), ("gpuMemory", "GPU 显存"), ("gpuThermals", "GPU 温度 / 功耗")]
    func shows(_ key: String) -> Bool { details?.contains(key) ?? true }
}
struct Settings: Codable, Equatable {
    var version = 2, sources: [AgentSource] = [], accounts: [AgentAccount] = [], hosts: [Host] = [], proxy = ProxySettings()
    var refreshSeconds = 300, serverRefreshSeconds = 10, menuMetric = "icon", githubRepository = ""
    var modelMappings: [String: String] = [:]
}
struct ModelPrice: Codable, Identifiable {
    var id = "", name = "", input: Double?, output: Double?, cacheRead: Double?, cacheWrite: Double?, fetchedAt: Double = 0
}
struct DeviceMetric: Codable, Identifiable {
    var id: String, name: String?, device: String?, type: String?
    var utilization: Double?, userPercent: Double?, systemPercent: Double?, iowaitPercent: Double?, stealPercent: Double?
    var total: Double?, available: Double?, used: Double?, inodes: Double?, inodesFree: Double?
    var memoryUsedMiB: Double?, memoryTotalMiB: Double?, temperature: Double?, powerWatts: Double?
    var rxBytes: Double?, txBytes: Double?, rxBytesPerSecond: Double?, txBytesPerSecond: Double?, rxErrors: Double?, txErrors: Double?, rxDrops: Double?, txDrops: Double?
    var readBytesPerSecond: Double?, writeBytesPerSecond: Double?, readIops: Double?, writeIops: Double?, busyMsPerSecond: Double?
}
struct MemoryMetric: Codable { var total: Double, available: Double, cached: Double, buffers: Double, swapTotal: Double, swapFree: Double }
struct MetricSample: Codable {
    var timestamp: Double, uptime: Double, load: [Double], errors: [String: String]
    var cpu: [DeviceMetric]?, memory: MemoryMetric?, gpu: [DeviceMetric]?, filesystems: [DeviceMetric]?, disk: [DeviceMetric]?, network: [DeviceMetric]?
}
struct HostResult: Codable, Identifiable { var id: String, name: String, sample: MetricSample?, error: String? }

enum Format {
    static func compact(_ value: Double) -> String {
        if value >= 1_000_000_000 { return String(format: "%.2fB", value / 1_000_000_000) }
        if value >= 1_000_000 { return String(format: "%.2fM", value / 1_000_000) }
        if value >= 1_000 { return String(format: "%.1fK", value / 1_000) }
        return String(format: "%.0f", value)
    }
    static func money(_ value: Double) -> String { String(format: "$%.2f", value) }
    static func percent(_ value: Double?) -> String { value.map { String(format: "%.1f%%", $0) } ?? "—" }
    static func bytes(_ value: Double?) -> String {
        guard let value else { return "—" }
        return ByteCountFormatter.string(fromByteCount: Int64(value), countStyle: .binary)
    }
    static func speed(_ value: Double?) -> String { value.map { bytes($0) + "/s" } ?? "—" }
    static func date(_ stamp: Double?) -> String {
        guard let stamp, stamp > 0 else { return "—" }
        return Date(timeIntervalSince1970: stamp).formatted(.dateTime.month(.twoDigits).day(.twoDigits).hour().minute())
    }
    static func time(_ stamp: Double) -> String { Date(timeIntervalSince1970: stamp).formatted(date: .omitted, time: .shortened) }
    static func provider(_ key: String) -> String { ["codex":"Codex", "claude":"Claude Code", "antigravity":"Antigravity", "agy":"agy", "deepseek":"DeepSeek", "custom":"自定义" ][key] ?? key }
}

struct QuotaEstimate: Codable, Identifiable {
    var kind: String?, consumedCredits: Double?, valuePer500: Double?, valuePer1000: Double?
    var id: String, accountKey: String, windowId: String, windowName: String
    var sourceIds: [String], sourceNames: [String], status: String, reason: String
    var startedAt: Double, checkpointAt: Double, endedAt: Double?
    var consumedPercent: Double, cost: Double, totalTokens: Double, pricedTokens: Double
    var weeklyValue: Double?, calculationNote: String
    var prices: [ModelPrice]?
    var statusLabel: String { ["active":"采样中", "pending":"待确认", "completed":"已结束"][status] ?? status }
}
