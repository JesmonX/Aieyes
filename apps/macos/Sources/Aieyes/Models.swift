import Foundation

enum PanelAccountPreference {
    static func selections(_ json: String, accounts: [AgentAccount], order: [String]) -> [String: [String]] {
        var result = (try? JSONDecoder().decode([String: [String]].self, from: Data(json.utf8))) ?? [:]
        let eligible = accounts.filter { $0.archived != true && $0.quotaEnabled }
        let sorted = eligible.sorted { (order.firstIndex(of: $0.key) ?? Int.max) < (order.firstIndex(of: $1.key) ?? Int.max) }
        for provider in Set(eligible.map(\.provider)).union(result.keys) {
            let keys = sorted.filter { $0.provider == provider }.map(\.key)
            // A missing preference takes the first five; an explicit [] hides this Agent.
            result[provider] = Array((result[provider] ?? keys).filter { keys.contains($0) }.reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }.prefix(5))
        }
        return result
    }
    static func encode(_ selections: [String: [String]]) -> String {
        String(data: (try? JSONEncoder().encode(selections)) ?? Data("{}".utf8), encoding: .utf8) ?? "{}"
    }
}

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
    var id = UUID().uuidString, name = "", target = "", port: Int?, identityFile = "", shell = "/bin/bash", preCommand = "", enabled = true
    var metrics = ["cpu", "memory", "gpu", "filesystems", "disk", "network"], devices: [String] = []
    var details: [String]? = Host.detailOptions.map { $0.0 }
    var authMode: String?, username: String?, passwordRef: String?
    static let detailOptions = [("cpuTimes", "CPU 时间分布"), ("memoryCache", "内存缓存 / Buffer"), ("swap", "Swap"), ("fsAvailable", "文件系统可用空间"), ("fsType", "文件系统类型 / 设备"), ("inodes", "inode"), ("diskIops", "磁盘 IOPS"), ("diskBusy", "磁盘忙碌率"), ("networkTotals", "累计流量"), ("networkErrors", "网络错误 / 丢包"), ("gpuMemory", "GPU 显存"), ("gpuThermals", "GPU 温度 / 功耗")]
    var connectionIdentity: String { [target, port.map(String.init) ?? "", identityFile, shell, preCommand, authMode ?? "ssh", username ?? "", passwordRef ?? ""].joined(separator: "\u{0}") }
    func shows(_ key: String) -> Bool { details?.contains(key) ?? true }
}
struct Settings: Codable, Equatable {
    var version = 2, sources: [AgentSource] = [], accounts: [AgentAccount] = [], hosts: [Host] = [], proxy = ProxySettings()
    var refreshSeconds = 300, serverRefreshSeconds = 10, menuMetric = "icon", githubRepository = ""
    var modelMappings: [String: String] = [:]
    var proxyTestUrls: [String]?
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
    // A dormant window does not start another countdown until a new reset is reported.
    // The derived date is display-only; refresh and sampling retain the original timestamp.
    static func resetDisplayTime(_ stamp: Double?, windowMinutes: Int? = nil, now: Date = Date()) -> Double? {
        if let stamp, stamp.isFinite, stamp > now.timeIntervalSince1970 { return stamp }
        guard let windowMinutes, windowMinutes > 0 else { return nil }
        return now.timeIntervalSince1970 + Double(windowMinutes) * 60
    }
    static func resetCountdown(_ stamp: Double?, windowMinutes: Int? = nil, now: Date = Date()) -> String {
        guard let target = resetDisplayTime(stamp, windowMinutes: windowMinutes, now: now) else {
            return stamp.map { $0.isFinite && $0 > 0 ? "确认重置中" : "重置时间未知" } ?? "重置时间未知"
        }
        let rawMinutes = ceil((target - now.timeIntervalSince1970) / 60)
        guard rawMinutes.isFinite, rawMinutes > 0, rawMinutes < Double(Int.max) else { return "重置时间未知" }
        let minutes = Int(rawMinutes)
        var parts: [String] = []
        if minutes >= 1440 { parts.append("\(minutes / 1440) 天") }
        if minutes % 1440 >= 60 { parts.append("\(minutes % 1440 / 60) 小时") }
        if minutes % 60 > 0 { parts.append("\(minutes % 60) 分") }
        return parts.joined(separator: " ") + "后重置"
    }
    static func quotaReset(_ window: QuotaWindow, now: Date = Date()) -> String {
        let countdown = resetCountdown(window.resetsAt, windowMinutes: window.windowMinutes, now: now)
        guard let stamp = resetDisplayTime(window.resetsAt, windowMinutes: window.windowMinutes, now: now) else { return countdown }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = .autoupdatingCurrent
        formatter.dateFormat = "MM/dd HH:mm"
        return countdown + " · " + formatter.string(from: Date(timeIntervalSince1970: stamp))
    }
    static func subscription(_ plan: String) -> String {
        let name = plan.trimmingCharacters(in: .whitespacesAndNewlines)
        return ["plus":"Plus", "pro":"Pro", "free":"Free", "max":"Max", "team":"Team", "business":"Business", "enterprise":"Enterprise", "api":"API"][name.lowercased()] ?? name
    }
    static func creditPrimary(_ balance: CreditsBalance?) -> String {
        if balance?.unlimited == true { return "无限" }
        guard let raw = balance?.balance else { return balance?.hasCredits == true ? "数量未知" : "—" }
        return credits(raw)
    }

    static func compact(_ value: Double) -> String {
        if value >= 999_995_000 { return String(format: "%.2fB", value / 1_000_000_000) }
        if value >= 999_950 { return String(format: "%.2fM", value / 1_000_000) }
        if value >= 999.5 { return String(format: "%.1fK", value / 1_000) }
        return String(format: "%.0f", value)
    }
    static func credits(_ value: String?) -> String {
        guard let value else { return "—" }
        let input=value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard input.range(of: #"^[+-]?[0-9]+(?:\.[0-9]*)?(?:[eE][+-]?[0-9]+)?$"#, options:.regularExpression) != nil,
              var decimal = Decimal(string: input, locale: Locale(identifier: "en_US_POSIX")), !decimal.isNaN else { return "—" }
        var rounded = Decimal()
        NSDecimalRound(&rounded, &decimal, 2, .plain)
        let formatter = NumberFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.numberStyle = .decimal
        formatter.usesGroupingSeparator = false
        formatter.minimumFractionDigits = 2; formatter.maximumFractionDigits = 2
        return formatter.string(from: NSDecimalNumber(decimal: rounded)) ?? "—"
    }
    static func creditBalance(_ balance: CreditsBalance?, estimate: QuotaEstimate?) -> String {
        if balance?.unlimited == true { return "余额 无限" }
        guard let raw = balance?.balance else { return balance?.hasCredits == true ? "余额 数量未知" : "余额 —" }
        let formatted = credits(raw)
        var text = "余额 " + formatted + (formatted == "—" ? "" : " credits")
        if formatted != "—", let amount = Double(raw), amount.isFinite, let estimate, estimate.status != "pending", let unit = estimate.valuePer1000, unit.isFinite, (amount * unit / 1000).isFinite {
            if let decimal = Decimal(string: raw), let rate = Decimal(string: String(unit)) {
                var value = decimal * rate / 1000, rounded = Decimal()
                NSDecimalRound(&rounded, &value, 2, .plain)
                text += " ≈ $" + credits(NSDecimalNumber(decimal: rounded).stringValue) + " USD"
            }
        }
        return text
    }
    static func creditValue(_ estimate: QuotaEstimate) -> String {
        if estimate.status == "pending" { return "待确认" }
        guard let value=estimate.valuePer1000, value.isFinite else { return "样本不足" }
        return "1000 credit ≈ " + money(value) + " USD"
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
    static func relative(_ date: Date, now: Date = Date()) -> String {
        let formatter = RelativeDateTimeFormatter(); formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: now)
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
    var valuationMode: String?, fiveHourValue: Double?, weeklyDirectValue: Double?, weeklyRatioValue: Double?
    var capacity: CapacityInfo?, segments: [EstimateSegment]?
    var statusLabel: String { ["active":"采样中", "pending":"待确认", "completed":"已结束"][status] ?? status }
}

struct CapacityInfo: Codable {
    var ratio: Double?
    var samples = 0
    var weeklyPercent: Double = 0
    var updatedAt: Double = 0
    var note: String { updatedAt > 0 && Date().timeIntervalSince1970 - updatedAt > 14 * 86400 ? "历史倍率" : weeklyPercent < 5 ? "倍率样本较少" : "近期倍率" }
}
struct EstimateSegment: Codable {
    var startedAt: Double, endedAt: Double, fivePercent: Double, weeklyPercent: Double?, cost: Double
}
struct NetworkSite: Codable { var url: String, latencyMs: Double?, error: String? }
struct NetworkTest: Codable {
    var testedAt: Double, mode: String, averageMs: Double?, status: String, sites: [NetworkSite]
    var label: String { status == "unstable" ? "连接不稳定" : status == "failed" ? "连接失败" : (mode == "direct" ? "直连 " : mode == "system" ? "系统 " : "代理 ") + String(Int(averageMs ?? 0)) + " ms" }
    var detail: String { sites.map { $0.url + " · " + ($0.latencyMs.map { String(Int($0)) + " ms" } ?? $0.error ?? "失败") }.joined(separator: "\n") + "\n测试于 " + Format.date(testedAt) }
}
