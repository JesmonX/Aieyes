import Foundation

enum PanelAccountPreference {
    struct Choice: Codable { var mode = "auto"; var keys: [String] = [] }
    struct Preference: Codable { var version = 2; var providers: [String: Choice] = [:] }
    static func decode(_ json: String) -> Preference {
        let data = Data(json.utf8)
        if let value = try? JSONDecoder().decode(Preference.self, from: data) { return value }
        let legacy = (try? JSONDecoder().decode([String: [String]].self, from: data)) ?? [:]
        return Preference(providers: legacy.mapValues { Choice(mode: "custom", keys: $0) })
    }
    static func migrated(_ json: String) -> String {
        String(data: (try? JSONEncoder().encode(decode(json))) ?? Data(), encoding: .utf8) ?? "{}"
    }
    static func migrateAntigravity(_ aliases: [String: String], defaults: UserDefaults = .standard) {
        let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
        let marker = (try? encoder.encode(aliases)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
        guard defaults.string(forKey: "antigravity.preferences.v1") != marker else { return }
        func key(_ value: String) -> String { aliases[value] ?? (value.hasPrefix("agy:") ? "antigravity:" + value.dropFirst(4) : value) }
        var pref = decode(defaults.string(forKey: "panel.accounts.v2") ?? defaults.string(forKey: "panel.accounts.v1") ?? "{}")
        if let old = pref.providers.removeValue(forKey: "agy") {
            let current = pref.providers["antigravity"]
            let values = ((current?.keys ?? []) + old.keys).map(key)
            var seen = Set<String>()
            pref.providers["antigravity"] = Choice(mode: current?.mode == "custom" || old.mode == "custom" ? "custom" : "auto", keys: Array(values.filter { seen.insert($0).inserted }.prefix(5)))
            defaults.set(String(data: (try? JSONEncoder().encode(pref)) ?? Data(), encoding: .utf8), forKey: "panel.accounts.v2")
        }
        for (name, value) in defaults.dictionaryRepresentation() where name.hasPrefix("quota.expanded.v2.") {
            if let range = name.range(of: ".agy:") {
                let start = name.index(after: range.lowerBound), next = String(name[..<start]) + key(String(name[start...]))
                if defaults.object(forKey: next) == nil { defaults.set(value, forKey: next) }
            }
        }
        for name in ["panel.provider", "detail.provider"] where defaults.string(forKey: name) == "agy" { defaults.set("antigravity", forKey: name) }
        defaults.set(marker, forKey: "antigravity.preferences.v1")
    }
    static func automatic(_ json: String, provider: String) -> Bool { decode(json).providers[provider]?.mode != "custom" }
    static func selections(_ json: String, accounts: [AgentAccount], order: [String]) -> [String: [String]] {
        let preference = decode(json)
        let eligible = accounts.filter { $0.archived != true && $0.quotaEnabled }
        let sorted = eligible.sorted { (order.firstIndex(of: $0.key) ?? Int.max) < (order.firstIndex(of: $1.key) ?? Int.max) }
        var result: [String: [String]] = [:]
        for provider in Set(eligible.map(\.provider)).union(preference.providers.keys) {
            let keys = sorted.filter { $0.provider == provider }.map(\.key)
            let choice = preference.providers[provider] ?? Choice()
            var selected = choice.mode == "auto" ? keys : choice.keys.filter { keys.contains($0) }
            if choice.mode == "custom" && !choice.keys.isEmpty && selected.isEmpty { selected = keys }
            result[provider] = Array(selected.reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }.prefix(5))
        }
        return result
    }
    static func includeNew(_ account: AgentAccount, accounts: [AgentAccount]) {
        let defaults = UserDefaults.standard
        let raw = defaults.string(forKey: "panel.accounts.v2") ?? migrated(defaults.string(forKey: "panel.accounts.v1") ?? "{}")
        var pref = decode(raw)
        guard pref.providers[account.provider]?.mode == "custom" else { return }
        var keys = selections(raw, accounts: accounts, order: [])[account.provider] ?? []
        guard keys.count < 5, !keys.contains(account.key), account.archived != true else { return }
        keys.append(account.key); pref.providers[account.provider] = Choice(mode: "custom", keys: keys)
        defaults.set(String(data: (try? JSONEncoder().encode(pref)) ?? Data(), encoding: .utf8), forKey: "panel.accounts.v2")
    }
    static func encode(_ selections: [String: [String]], automatic: Set<String> = []) -> String {
        var value = Preference(providers: selections.mapValues { Choice(mode: "custom", keys: $0) })
        for provider in automatic { value.providers[provider] = Choice() }
        return String(data: (try? JSONEncoder().encode(value)) ?? Data(), encoding: .utf8) ?? "{}"
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
    var estimateGroup: String { groupId.flatMap { $0.isEmpty ? nil : $0 } ?? groupLabel }
    var supportsEstimate: Bool { ["gemini", "gemini models", "claude-gpt", "claude and gpt models"].contains(estimateGroup.lowercased()) }
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
struct AccountIdentity: Codable {
    var key: String, email: String
    var subscription: String?, checkedAt: Double?, subscriptionCheckedAt: Double?, stale: Bool?
    var summary: String { email + (subscription.map { " · " + $0 } ?? " · 订阅未知") + (stale == true ? " · 订阅待更新" : "") }
}
struct Quota: Codable, Identifiable {
    var identity: AccountIdentity? = nil
    var metadataError: String? = nil
    var credits: CreditsBalance?, creditsUpdatedAt: Double?
    var sourceId: String, accountId: String, provider: String, name: String, updatedAt: Double, origin: String
    var balances: [Balance]?, isAvailable: Bool?
    var windows: [QuotaWindow], bankReset: BankReset?, bankUpdatedAt: Double?, plan: String?, error: String?
    var id: String { provider + ":" + accountId }
}
struct SyncIssue: Codable { var path: String?; var message: String; var code: String?; var step: Int? }
struct SourceStatus: Codable { var partial: Bool?, issues: [SyncIssue]?; var updatedAt: Double?, newEvents: Int?, files: Int?, readBytes: Int?, malformedLines: Int?, error: String? }
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
struct AccountConnection: Codable, Equatable { var sourceId: String; var profileId: String? }
struct AgentConfiguration: Codable, Equatable, Identifiable {
    var provider: String, enabled: Bool, machineIds: [String]
    var id: String { provider }
}
struct AccountDeviceSettings: Codable, Equatable { var machineId: String; var preCommand: String }
struct AntigravityLoginState: Decodable {
    var id: String, phase: String, message: String, authUrl: String?
    var authenticated: Bool, identityConfirmed: Bool
    var identity: AccountIdentity?, current: Bool?, metadataError: String?
    var accountStatus: AccountDeviceStatus?
    var workspaceConfirmationRequired: Bool?
}
struct AccountDeviceStatus: Codable, Identifiable {
    var identity: AccountIdentity? = nil
    var metadataError: String? = nil
    var authenticated: Bool?, identityConfirmed: Bool?
    var accountKey: String, sourceId: String, machineId: String
    var current: Bool?, credential: Bool?, checkedAt: Double?, attemptedAt: Double?, error: String?, note: String?
    var id: String { accountKey + ":" + sourceId }
    var identitySummary: String? {
        guard let identity else { return authenticated == true ? "登录有效 · 账户身份待确认" : nil }
        return (current == false ? "身份不一致 · 当前登录 " : "正在使用 ") + identity.summary + (error != nil ? " · 上次检查" : "")
    }
}
struct AgentAccount: Codable, Equatable, Identifiable {
    var pendingName: Bool?
    var quotaRefreshSeconds: Int?
    var deviceSettings: [AccountDeviceSettings]?
    var connections: [AccountConnection]?
    var identityKey: String?
    var profileRefs: [String] { Array(Set(([quotaProfileId].compactMap { $0 }) + (connections ?? []).compactMap(\.profileId))).sorted() }
    func uses(_ source: AgentSource) -> Bool { provider == source.provider && (source.accountId == id || source.codexHomeId.map { home in profileRefs.contains { $0.hasPrefix(home + ":") } } == true) }
    var quotaProfileId: String?
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
    var codexHomeId: String?
    var id = UUID().uuidString, name = "", provider = "codex", accountId = ""
    var path = "~/.codex", hostId: String?, enabled = true, quotaCommand = "", quotaPreCommand = "", codexBinary = "codex", agyBinary: String? = "agy", proxy: ProxySettings?
}
struct Host: Codable, Equatable, Identifiable {
    var id = UUID().uuidString, name = "", target = "", port: Int?, identityFile = "", shell = "/bin/bash", preCommand = "", enabled = true
    var metrics = ["cpu", "memory", "gpu", "filesystems", "disk", "network"], devices: [String] = []
    var details: [String]? = Host.detailOptions.map { $0.0 }
    var authMode: String?, username: String?, passwordRef: String?
    static let detailOptions = [("uptime", "连续运行时间"), ("cpuTimes", "CPU 时间分布"), ("memoryCache", "内存缓存 / Buffer"), ("swap", "Swap"), ("fsAvailable", "文件系统可用空间"), ("fsType", "文件系统类型 / 设备"), ("inodes", "inode"), ("diskIops", "磁盘 IOPS"), ("diskBusy", "磁盘忙碌率"), ("networkTotals", "累计流量"), ("networkErrors", "网络错误 / 丢包"), ("gpuMemory", "GPU 显存"), ("gpuThermals", "GPU 温度 / 功耗")]
    var connectionIdentity: String { [target, port.map(String.init) ?? "", identityFile, shell, preCommand, authMode ?? "ssh", username ?? "", passwordRef ?? ""].joined(separator: "\u{0}") }
    func shows(_ key: String) -> Bool { details?.contains(key) ?? true }
    // Also filter retained samples immediately when the device selection changes.
    func selectedDevices(_ group: String, _ rows: [DeviceMetric]?) -> [DeviceMetric] {
        guard metrics.contains(group) else { return [] }
        let selected = Set(devices.filter { $0.hasPrefix(group + ":") })
        return (rows ?? []).filter { (selected.isEmpty && (group != "filesystems" || MonitorSelection.recommendedFilesystem($0.id, type: $0.type))) || (group == "cpu" && $0.id == "cpu") || selected.contains(group + ":__all__") || selected.contains(group + ":" + $0.id) }
    }
}
struct AppearanceSettings: Codable, Equatable {
    var theme = "system", accent = "indigo"
}
struct LocalMonitor: Codable, Equatable {
    var enabled = true
    var metrics = Host().metrics, devices: [String] = [], details: [String]? = Host().details
    var host: Host { var h = Host(); h.id = "local"; h.name = "本机"; h.target = "本机"; h.enabled = enabled; h.metrics = metrics; h.devices = devices; h.details = details; return h }
    init() {}
    init(_ h: Host) { enabled = h.enabled; metrics = h.metrics; devices = h.devices; details = h.details }
}
struct Settings: Codable, Equatable {
    var deletedAccounts: [AgentAccount]?
    var historicalAccounts: [AgentAccount] { accounts + (deletedAccounts ?? []) }
    var agents: [AgentConfiguration]?
    var accountAliases: [String: String]?

    var appearance: AppearanceSettings?

    var version = 2, sources: [AgentSource] = [], accounts: [AgentAccount] = [], hosts: [Host] = [], proxy = ProxySettings()
    var refreshSeconds = 300, serverRefreshSeconds = 10, menuMetric = "icon", githubRepository = ""
    var historyRefreshSeconds: Int?, serverForegroundRefreshSeconds: Int?, localMonitor: LocalMonitor?
    var historyInterval: Int { get { historyRefreshSeconds ?? refreshSeconds } set { historyRefreshSeconds = newValue } }
    var foregroundInterval: Int { get { serverForegroundRefreshSeconds ?? 2 } set { serverForegroundRefreshSeconds = newValue } }
    var monitoredHosts: [Host] { (localMonitor.map { [$0.host] } ?? []) + hosts }
    var modelMappings: [String: String] = [:]
    var proxyTestUrls: [String]?
}
struct ModelPrice: Codable, Equatable, Identifiable {
    var id = "", name = "", input: Double?, output: Double?, cacheRead: Double?, cacheWrite: Double?, fetchedAt: Double = 0
}
struct DeviceMetric: Codable, Identifiable, Equatable {
    var id: String, name: String?, device: String?, type: String?
    var utilization: Double?, userPercent: Double?, systemPercent: Double?, iowaitPercent: Double?, stealPercent: Double?
    var total: Double?, available: Double?, used: Double?, inodes: Double?, inodesFree: Double?
    var memoryUsedMiB: Double?, memoryTotalMiB: Double?, temperature: Double?, powerWatts: Double?
    var rxBytes: Double?, txBytes: Double?, rxBytesPerSecond: Double?, txBytesPerSecond: Double?, rxErrors: Double?, txErrors: Double?, rxDrops: Double?, txDrops: Double?
    var readBytesPerSecond: Double?, writeBytesPerSecond: Double?, readIops: Double?, writeIops: Double?, busyMsPerSecond: Double?
}
struct MemoryMetric: Codable, Equatable {
    var total: Double?, available: Double?, cached: Double?, buffers: Double?, swapTotal: Double?, swapFree: Double?
    var used: Double? { Format.usedCapacity(total, available) }
    var swapUsed: Double? { Format.usedCapacity(swapTotal, swapFree) }
}
struct MetricSample: Codable, Equatable {
    var timestamp: Double, uptime: Double?, load: [Double], errors: [String: String]
    var cpu: [DeviceMetric]?, memory: MemoryMetric?, gpu: [DeviceMetric]?, filesystems: [DeviceMetric]?, disk: [DeviceMetric]?, network: [DeviceMetric]?
}
struct HostResult: Codable, Identifiable, Equatable {
    var id: String, name: String, sample: MetricSample?, error: String?
    var sampleSession: String? = nil, sampleVersion: UInt64? = nil
}
struct HostSampleBatch: Decodable {
    var rows: [HostResult]
    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if let legacy = try? value.decode([HostResult].self) { rows = legacy }
        else { rows = try value.decode(Envelope.self).rows }
    }
    private struct Envelope: Decodable { var rows: [HostResult] }
}

enum Format {
    static func usedCapacity(_ total: Double?, _ available: Double?) -> Double? {
        guard let total, let available, total.isFinite, available.isFinite else { return nil }
        return total - available
    }
    static func capacityPercent(_ used: Double?, _ total: Double?) -> Double? {
        guard let used, let total, used.isFinite, total.isFinite, total > 0 else { return nil }
        return used / total * 100
    }
    static func uptime(_ seconds: Double?) -> String {
        guard let seconds, seconds.isFinite, seconds >= 0, seconds / 60 < Double(Int.max) else { return "—" }
        let minutes = Int(seconds / 60)
        var parts: [String] = []
        if minutes >= 1440 { parts.append("\(minutes / 1440) 天") }
        if minutes >= 60 { parts.append("\(minutes % 1440 / 60) 小时") }
        parts.append("\(minutes % 60) 分")
        return parts.joined(separator: " ")
    }
    // A zero usage percentage does not establish whether a window has started.
    // Only the source timestamp can establish its next reset.
    static func resetDisplayTime(_ stamp: Double?, now: Date = Date()) -> Double? {
        guard let stamp, stamp.isFinite, stamp > 0, stamp > now.timeIntervalSince1970 else { return nil }
        return stamp
    }
    static func resetCountdown(_ stamp: Double?, now: Date = Date()) -> String {
        guard let target = resetDisplayTime(stamp, now: now) else {
            return stamp.map { $0.isFinite && $0 > 0 ? "确认重置中" : "重置时间未知" } ?? "重置时间未知"
        }
        let rawMinutes = ceil((target - now.timeIntervalSince1970) / 60)
        guard rawMinutes.isFinite, rawMinutes > 0, rawMinutes < Double(Int.max) else { return "重置时间未知" }
        let minutes = Int(rawMinutes)
        var parts: [String] = []
        if minutes >= 1440 { parts.append("\(minutes / 1440) 天") }
        if minutes % 1440 >= 60 { parts.append("\(minutes % 1440 / 60) 小时") }
        if minutes < 1440 && minutes % 60 > 0 { parts.append("\(minutes % 60) 分") }
        return parts.joined(separator: " ") + "后重置"
    }
    static func quotaReset(_ window: QuotaWindow, now: Date = Date()) -> String {
        if !(window.resetsAt.map { $0.isFinite && $0 > 0 } ?? false), window.usedPercent == 0 {
            if window.windowMinutes == 300 { return "5h" }
            if window.windowMinutes == 10080 { return "7d" }
        }
        let countdown = resetCountdown(window.resetsAt, now: now)
        guard let stamp = resetDisplayTime(window.resetsAt, now: now) else { return countdown }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = .autoupdatingCurrent
        formatter.dateFormat = "MM/dd HH:mm"
        return countdown + " · " + formatter.string(from: Date(timeIntervalSince1970: stamp))
    }
    static func subscription(_ plan: String) -> String {
        let name = plan.trimmingCharacters(in: .whitespacesAndNewlines)
        return ["plus":"Plus", "pro":"Pro", "free":"Free", "max":"Max", "team":"Team", "business":"Business", "enterprise":"Enterprise", "api":"API"][name.lowercased()] ?? (name.prefix(1).uppercased() + name.dropFirst())
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
        guard let value=estimate.valuePer1000, value.isFinite else { return estimate.issueLabel }
        return "1000 credit ≈ " + money(value) + " USD"
    }
    static func compactMoney(_ value: Double?) -> String {
        guard let value, value.isFinite else { return "—" }
        for (size, suffix) in [(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")] where abs(value) >= size * 0.999995 { return String(format: "$%.2f", value / size) + suffix }
        return money(value)
    }
    static func capacityPair(_ used: Double?, _ total: Double?) -> String {
        let maximum = max(used.flatMap { $0.isFinite ? $0 : nil } ?? 0, total.flatMap { $0.isFinite ? $0 : nil } ?? 0)
        let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"]
        let power = maximum > 0 ? min(units.count - 1, max(0, Int(log(maximum) / log(1024)))) : 0
        func number(_ value: Double?) -> String {
            guard let value, value.isFinite else { return "—" }
            let text = String(format: "%.1f", value / pow(1024, Double(power)))
            return text.hasSuffix(".0") ? String(text.dropLast(2)) : text
        }
        return number(used) + "/" + number(total) + " " + units[power]
    }
    static func money(_ value: Double) -> String { String(format: "$%.2f", value) }
    static func percent(_ value: Double?) -> String { value.map { String(format: "%.1f%%", $0) } ?? "—" }
    static func bytes(_ value: Double?) -> String {
        guard let value, value.isFinite, value >= 0, value < Double(Int64.max) else { return "—" }
        if value == 0 { return "0 B" }
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
    static func provider(_ key: String) -> String { ["codex":"Codex", "claude":"Claude Code", "antigravity":"Antigravity", "agy":"Antigravity", "deepseek":"DeepSeek", "custom":"自定义" ][key] ?? key }
}

struct QuotaEstimate: Codable, Identifiable {
    var groupId: String?

    var calculationStatus: String?, calculationVersion: Int?, originalEstimateId: String?, repairedAt: Double?
    var hasValue: Bool { [fiveHourValue, weeklyValue, valuePer1000].compactMap { $0 }.contains { $0.isFinite } }
    var issueLabel: String {
        if status == "pending" { return "待确认" }
        if let code = calculationStatus, let label = ["boundary":"用量边界待确认", "noUsage":"暂无有效用量", "unpriced":"模型价格不完整", "insufficientUsage":"尚未达到采样阈值", "invalidPrice":"价格数据无效"][code] { return label }
        return calculationNote.isEmpty ? "尚无估值" : calculationNote
    }

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

enum EstimatePresentation {
    struct Entry: Identifiable {
        var id: String, label: String, value: Double, record: QuotaEstimate
        var historical: Bool
    }
    static func canonical(_ records: [QuotaEstimate]) -> [QuotaEstimate] {
        let replaced = Set(records.compactMap(\.originalEstimateId))
        return records.filter { !replaced.contains($0.id) }.sorted {
            $0.startedAt == $1.startedAt ? ($0.repairedAt ?? 0) > ($1.repairedAt ?? 0) : $0.startedAt > $1.startedAt
        }
    }
    static func entries(_ records: [QuotaEstimate], credits: Bool = false) -> [Entry] {
        let all = canonical(records)
        let fields: [(String, String, KeyPath<QuotaEstimate, Double?>)] = credits
            ? [("credits", "1000 credits", \.valuePer1000)]
            : [("five", "5h", \.fiveHourValue), ("week", "7d 同期 / 整周", \.weeklyValue), ("ratio", "7d 容量倍率", \.weeklyRatioValue)]
        return fields.compactMap { id, label, path in
            guard let record = all.first(where: { $0.status != "pending" && $0[keyPath: path]?.isFinite == true }), let value = record[keyPath: path] else { return nil }
            let title = id == "week" ? (record.valuationMode == "fiveHour" ? "7d 同期" : "7d 整周") : label
            return Entry(id: id, label: title, value: value, record: record, historical: record.id != all.first?.id || record.status == "completed")
        }
    }
    static func summary(_ records: [QuotaEstimate]) -> String {
        let values = entries(records)
        func value(_ id: String) -> String { Format.compactMoney(values.first { $0.id == id }?.value) }
        return "5h ≈ " + value("five") + "  7d ≈ " + value("ratio") + "/" + value("week")
    }
    static func credit(_ records: [QuotaEstimate]) -> QuotaEstimate? { entries(records, credits: true).first?.record }
}

struct AccountCleanupTask: Decodable, Identifiable { var id: String, name: String, machine: String; var root: String?, error: String? }
struct AccountDeletionPreview: Decodable { var tasks: [AccountCleanupTask] }
struct AccountCleanupResult: Decodable { var deleted: Bool?, settings: Settings?, failedTasks: [AccountCleanupTask]? }
struct AccountCleanupRecord: Decodable, Identifiable { var accountKey: String, name: String, tasks: [AccountCleanupTask]; var id: String { accountKey } }
