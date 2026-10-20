import Foundation

/// Presentation only: unknown errors keep their diagnostic text without guessing a cause.
struct UserFacingError {
    let summary: String
    let details: String?
    init(_ message: String, context: String = "操作未完成") {
        var text = message.trimmingCharacters(in: .whitespacesAndNewlines)
        let rules = [
            (#"(?i)([\"']?cookie[\"']?\s*[:=]\s*[\"']?)[^\r\n]+"#, "$1[已隐藏]"),
            (#"(?i)(bearer\s+)[a-z0-9._~+/=-]+"#, "$1[已隐藏]"),
            (#"(?i)((?:[\"']?)(?:api[_ -]?key|access[_-]?token|refresh[_-]?token|token|authorization|password)(?:[\"']?)\s*[:=]\s*[\"']?)[^\s\"';,}&]+"#, "$1[已隐藏]"),
            (#"(https?://)[^\s/@]+:[^\s/@]+@"#, "$1[已隐藏]@"),
            (#"\bsk-(?:proj-|ant-)?[A-Za-z0-9_-]{8,}"#, "[已隐藏]")
        ]
        for (pattern, replacement) in rules {
            if let expression = try? NSRegularExpression(pattern: pattern) {
                text = expression.stringByReplacingMatches(in: text, range: NSRange(text.startIndex..., in: text), withTemplate: replacement)
            }
        }
        let first = text.components(separatedBy: .newlines).first ?? ""
        let sourceFirst = message.trimmingCharacters(in: .whitespacesAndNewlines).components(separatedBy: .newlines).first ?? ""
        let readable = sourceFirst.range(of: #"\p{Han}"#, options: .regularExpression) != nil && !["{", "["].contains(String(sourceFirst.prefix(1)))
        summary = first.count <= 72 && readable ? first : context
        details = !text.isEmpty && text != summary ? text : nil
    }
}

enum ProviderBoundary {
    static func normalize(_ value: Any, aliases: [String: String] = [:]) -> Any {
        if let rows = value as? [Any] { return rows.map { normalize($0, aliases: aliases) } }
        guard let map = value as? [String: Any] else { return value }
        let aliases = map["accountAliases"] as? [String: String] ?? aliases
        func canonicalKey(_ key: String) -> String { aliases[key] ?? (key.hasPrefix("agy:") ? "antigravity:" + key.dropFirst(4) : key) }
        var next = map.mapValues { normalize($0, aliases: aliases) }
        if next["provider"] as? String == "agy" {
            let field = next["accountId"] != nil ? "accountId" : next["quotaEnabled"] != nil ? "id" : nil
            if let field, let id = next[field] as? String { next[field] = String(canonicalKey("agy:" + id).dropFirst("antigravity:".count)) }
            next["provider"] = "antigravity"
        }
        if let key = next["accountKey"] as? String { next["accountKey"] = canonicalKey(key) }
        if let keys = next["quotaOrder"] as? [String] { next["quotaOrder"] = keys.map(canonicalKey) }
        return next
    }
}

enum NativePanelKey: String, CaseIterable {
    case accountsLegacy = "panel.accounts.v1", accounts = "panel.accounts.v2"
    case aliasMigration = "antigravity.preferences.v1", filters = "dashboard.filters.v1"
    case panelPage = "panel.page", detailPage = "detail.page"
    case panelServerFilter = "panel.serverFilter", detailServerFilter = "detail.serverFilter"
    case panelTrend = "panel.trendExpanded", detailTrend = "detail.trendExpanded"
    case panelSessions = "panel.sessionsExpanded", detailSessions = "detail.sessionsExpanded"
    case panelCost = "panel.costMode", detailCost = "detail.costMode"
    case showCount = "menu.showCount"
    enum Prefix: String, CaseIterable {
        case quota = "quota.expanded.v2.", server = "server.expanded.", metric = "server.metric.expanded."
    }
    static func scoped(_ panel: Self, _ detail: Self, compact: Bool) -> String { (compact ? panel : detail).rawValue }
    static func expansion(_ prefix: Prefix, id: String, compact: Bool) -> String { prefix.rawValue + (compact ? "panel." : "detail.") + id }
    static func snapshotOverrides(defaults: UserDefaults = .standard, panelQuotaExpanded: Bool = true) -> [String: Any] {
        var result: [String: Any] = [:]
        for key in allCases {
            switch key {
            case .accountsLegacy, .filters: result[key.rawValue] = "{}"
            case .accounts: result[key.rawValue] = "{\"version\":2,\"providers\":{}}"
            case .aliasMigration: result[key.rawValue] = ""
            case .panelPage, .detailPage: result[key.rawValue] = "agent"
            case .panelServerFilter, .detailServerFilter: result[key.rawValue] = "all"
            default: result[key.rawValue] = false
            }
        }
        for name in defaults.dictionaryRepresentation().keys where Prefix.allCases.contains(where: { name.hasPrefix($0.rawValue) }) { result[name] = name.hasPrefix(Prefix.quota.rawValue) && (!name.hasPrefix(Prefix.quota.rawValue + "panel.") || panelQuotaExpanded) }
        return result
    }
}

struct SettingsDestination {
    var category = "integrations"
    var provider: String?
    var accountKey: String?
    var hostID: String?
    var section: String?
    var machineID: String?
    var normalizedCategory: String {
        switch category {
        case "sources", "accounts", "codexAuth": return "integrations"
        case "hosts": return "servers"
        case "connection": return "general"
        default: return category
        }
    }
}

struct DashboardDateRange: Codable, Equatable {
    var kind: String
    var start: String?
    var end: String?
    static func day(_ date: Date) -> String { formatter.string(from: date) }
    static func date(_ day: String?) -> Date { day.flatMap { formatter.date(from: $0) } ?? Date() }
    private static var formatter: DateFormatter {
        let value = DateFormatter(); value.locale = Locale(identifier: "en_US_POSIX"); value.calendar = Calendar(identifier: .gregorian); value.timeZone = .autoupdatingCurrent; value.dateFormat = "yyyy-MM-dd"; return value
    }
    var parameters: [String: Any] { var result: [String: Any] = ["kind":kind]; if let start { result["start"] = start }; if let end { result["end"] = end }; return result }
}

struct DashboardFilterPreference: Codable, Equatable {
    var provider = "all", source = "all", account = "all", model = "all"
    var days = 1
    var dateRange: DashboardDateRange?
    static func read(scope: String? = nil, defaults: UserDefaults = .standard) -> Self {
        guard let data = scope.flatMap({ defaults.data(forKey: "dashboard.filters.v2." + $0) }) ?? defaults.data(forKey: NativePanelKey.filters.rawValue), var value = try? JSONDecoder().decode(Self.self, from: data) else { return Self() }
        if value.provider == "agy" { value.provider = "antigravity" }
        if !CoreContract.providers.contains(value.provider) && value.provider != "all" { value.provider = "all" }
        if let range = value.dateRange, !["all", "custom"].contains(range.kind) { value.dateRange = nil }
        if ![1, 7, 30, 90, 365].contains(value.days) { value.days = 1 }
        return value.normalized(for: scope)
    }
    func normalized(for scope: String?) -> Self {
        var value = self
        value.account = "all"
        value.provider = CoreContract.agentProvider(value.provider)
        if scope == "overview" || scope == "panel" { value.days = 1; value.dateRange = nil; value.source = "all"; value.model = "all" }
        return value
    }
    func save(scope: String? = nil, defaults: UserDefaults = .standard) { defaults.set(try? JSONEncoder().encode(normalized(for: scope)), forKey: scope.map { "dashboard.filters.v2." + $0 } ?? NativePanelKey.filters.rawValue) }
    var isToday: Bool { dateRange == nil && days == 1 }
    var rangeTitle: String { dateRange.map { $0.kind == "all" ? "所有" : ($0.start ?? "") + " 至 " + ($0.end ?? "") } ?? (days == 1 ? "今日" : "最近 \(days) 天") }
    var rangeChoice: String {
        get { dateRange?.kind ?? String(days) }
        set {
            if newValue == "all" { dateRange = DashboardDateRange(kind: "all") }
            else if newValue == "custom" { dateRange = DashboardDateRange(kind: "custom", start: DashboardDateRange.day(Calendar.current.date(byAdding: .day, value: -6, to: Date())!), end: DashboardDateRange.day(Date())) }
            else { dateRange = nil; days = Int(newValue) ?? 1 }
        }
    }
    var customStart: Date {
        get { DashboardDateRange.date(dateRange?.start) }
        set { dateRange = DashboardDateRange(kind: "custom", start: DashboardDateRange.day(newValue), end: dateRange?.end ?? DashboardDateRange.day(Date())) }
    }
    var customEnd: Date {
        get { DashboardDateRange.date(dateRange?.end) }
        set { dateRange = DashboardDateRange(kind: "custom", start: dateRange?.start ?? DashboardDateRange.day(Date()), end: DashboardDateRange.day(newValue)) }
    }
    func parameters(settings: Settings) -> [String: Any] {
        var params: [String: Any] = ["days": days]
        if let dateRange { params["dateRange"] = dateRange.parameters }
        if provider != "all" { params["provider"] = provider }
        if source != "all" { params["sourceId"] = source }
        if model != "all" { params["model"] = model }
        return params
    }
    mutating func reconcile(settings: Settings) {
        provider = CoreContract.agentProvider(provider)
        account = settings.accountAliases?[account] ?? (account.hasPrefix("agy:") ? "antigravity:" + account.dropFirst(4) : account)
        if account != "all", account != "none", !settings.filterAccounts.contains(where: { $0.key == account }) { account = "all" }
        if source != "all", !settings.sources.contains(where: { $0.id == source }) { source = "all" }
    }
}

enum PanelAccountPreference {
    struct Choice: Codable { var mode = "auto"; var keys: [String] = [] }
    struct Preference: Codable { var version = 2; var providers: [String: Choice] = [:] }
    static func decode(_ json: String) -> Preference {
        guard let value = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] else { return Preference() }
        if value["version"] != nil || value["providers"] != nil {
            guard value["version"] as? Int == 2, let providers = value["providers"] as? [String: Any] else { return Preference() }
            return Preference(providers: providers.mapValues { row in
                guard let choice = row as? [String: Any], choice["mode"] as? String == "custom", let keys = choice["keys"] as? [String] else { return Choice() }
                return Choice(mode: "custom", keys: keys)
            })
        }
        return Preference(providers: value.compactMapValues { ($0 as? [String]).map { Choice(mode: "custom", keys: $0) } })
    }
    static func migrated(_ json: String) -> String {
        String(data: (try? JSONEncoder().encode(decode(json))) ?? Data(), encoding: .utf8) ?? "{}"
    }
    static func migrateAntigravity(_ aliases: [String: String], defaults: UserDefaults = .standard) {
        let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
        let marker = (try? encoder.encode(aliases)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
        guard defaults.string(forKey: NativePanelKey.aliasMigration.rawValue) != marker else { return }
        func key(_ value: String) -> String { aliases[value] ?? (value.hasPrefix("agy:") ? "antigravity:" + value.dropFirst(4) : value) }
        var pref = decode(defaults.string(forKey: NativePanelKey.accounts.rawValue) ?? defaults.string(forKey: NativePanelKey.accountsLegacy.rawValue) ?? "{}")
        if let old = pref.providers.removeValue(forKey: "agy") {
            let current = pref.providers["antigravity"]
            let values = ((current?.keys ?? []) + old.keys).map(key)
            var seen = Set<String>()
            pref.providers["antigravity"] = Choice(mode: current?.mode == "custom" || old.mode == "custom" ? "custom" : "auto", keys: values.filter { seen.insert($0).inserted })
            defaults.set(String(data: (try? JSONEncoder().encode(pref)) ?? Data(), encoding: .utf8), forKey: NativePanelKey.accounts.rawValue)
        }
        for (name, value) in defaults.dictionaryRepresentation() where name.hasPrefix(NativePanelKey.Prefix.quota.rawValue) {
            if let range = name.range(of: ".agy:") {
                let start = name.index(after: range.lowerBound), next = String(name[..<start]) + key(String(name[start...]))
                if defaults.object(forKey: next) == nil { defaults.set(value, forKey: next) }
            }
        }
        defaults.set(marker, forKey: NativePanelKey.aliasMigration.rawValue)
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
            let automatic = sorted.filter { CoreContract.sameAgent($0.provider, provider) && preference.providers[$0.provider]?.mode != "custom" }.prefix(5).filter { $0.provider == provider }.map(\.key)
            let selected = choice.mode == "custom" ? choice.keys.filter { keys.contains($0) } : automatic
            result[provider] = selected.reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }
        }
        return result
    }
    static func includeNew(_ account: AgentAccount, accounts: [AgentAccount], defaults: UserDefaults = .standard) {
        // Automatic choices are derived at display time. A manual choice is explicit,
        // including an empty choice, and never gains accounts as a side effect.
    }
    static func encode(_ selections: [String: [String]], automatic: Set<String> = []) -> String {
        var value = Preference(providers: selections.mapValues { Choice(mode: "custom", keys: $0) })
        for provider in automatic { value.providers[provider] = Choice() }
        return String(data: (try? JSONEncoder().encode(value)) ?? Data(), encoding: .utf8) ?? "{}"
    }
}

struct Tokens: Codable, Equatable {
    var input: Double = 0, output: Double = 0, cacheRead: Double = 0, cacheWrite: Double = 0
    var allInput: Double { input + cacheRead + cacheWrite }
    var cacheRate: Double? { allInput > 0 ? cacheRead / allInput : nil }
}
struct Aggregate: Codable, Identifiable, Equatable {
    var key = "", tokens = Tokens(), total: Double = 0, cost: Double = 0, pricedTokens: Double = 0
    var nativeCredits: [String: NativeCreditsTotal]?
    var hasNativeCredits: Bool { nativeCredits?.values.contains { $0.requests > 0 } == true }
    var costAvailable: Bool { pricedTokens > 0 || (total == 0 && !hasNativeCredits) }
    var nativeCreditText: String {
        (nativeCredits ?? [:]).keys.sorted().compactMap { provider in
            guard let row = nativeCredits?[provider], row.requests > 0 else { return nil }
            return Format.provider(provider) + " 原生记录 " + row.amount(row.credits) + " Credits · " + String(row.requests) + (provider == "codebuddy" ? " 条计量" : " 次请求")
        }.joined(separator: "；")
    }
    var id: String { key }
}
struct NativeCreditsTotal: Codable, Equatable {
    var credits: Double, billable: Double, nonBillable: Double, unknownBillability: Double
    var requests: Int
    var originalCredits: Double?, originalCreditsRequests: Int?
    var billableRequests: Int?, nonBillableRequests: Int?, unknownBillabilityRequests: Int?
    func amount(_ value: Double) -> String { value.formatted(.number.precision(.fractionLength(0...6))) }
    var detail: String {
        var parts: [String] = []
        if (billableRequests ?? 0) > 0 { parts.append("标记可计费 " + amount(billable) + "（\(billableRequests!) 次）") }
        if (nonBillableRequests ?? 0) > 0 { parts.append("标记不计费 " + amount(nonBillable) + "（\(nonBillableRequests!) 次）") }
        if (unknownBillabilityRequests ?? 0) > 0 { parts.append("计费状态未提供 " + amount(unknownBillability) + "（\(unknownBillabilityRequests!) 次）") }
        if let originalCredits, let count = originalCreditsRequests, count > 0 { parts.append("折扣前已记录 " + amount(originalCredits) + "（\(count)/\(requests) 次请求）") }
        return parts.joined(separator: " · ")
    }
}
struct QuotaWindow: Codable, Equatable, Identifiable {
    var name: String, usedPercent: Double, windowMinutes: Int?, resetsAt: Double?
    var stableId: String?, groupId: String?, groupName: String?
    var id: String { stableId.flatMap { $0.isEmpty ? nil : $0 } ?? name }
    var estimateGroup: String { groupId.flatMap { $0.isEmpty ? nil : $0 } ?? groupLabel }
    var groupLabel: String { groupName.flatMap { $0.isEmpty ? nil : $0 } ?? name.components(separatedBy: " · ").dropLast().joined(separator: " · ") }
    enum CodingKeys: String, CodingKey { case name, usedPercent, windowMinutes, resetsAt, groupId, groupName; case stableId = "id" }
}
struct ResetCredit: Codable, Equatable, Identifiable {
    var id: String, status: String, expiresAt: Double?, title: String?
}
struct BankReset: Codable, Equatable { var availableCount: Int, credits: [ResetCredit]? }
struct Balance: Codable, Equatable, Identifiable {
    var currency: String, total: String, granted: String, toppedUp: String
    var id: String { currency }
}
struct ExtraUsageWallet: Codable, Equatable {
    var currency: String, total: String
    var balance: String?, monthlyLimitEnabled: Bool?, monthlyLimit: String?, monthlyUsed: String?
    func amount(_ value: String?) -> String { value.map { currency + " " + $0 } ?? "未提供" }
    var monthlyLimitLabel: String {
        if monthlyLimitEnabled == true { return "月度上限 " + amount(monthlyLimit) }
        return monthlyLimitEnabled == false ? "月度上限未启用" : "月度上限状态未提供"
    }
}
struct CreditsBalance: Codable, Equatable { var hasCredits: Bool, unlimited: Bool, balance: String? }
struct AccountIdentity: Codable, Equatable {
    var key: String, email: String
    var subscription: String?, checkedAt: Double?, stale: Bool?
    var summary: String { email + (subscription.map { " · " + $0 } ?? " · 订阅未知") + (stale == true ? " · 订阅待更新" : "") }
}
struct Quota: Codable, Equatable, Identifiable {
    var identity: AccountIdentity? = nil
    var metadataError: String? = nil
    var extraUsage: ExtraUsageWallet? = nil
    var credits: CreditsBalance?, creditsUpdatedAt: Double?
    var sourceId: String, accountId: String, provider: String, name: String, updatedAt: Double, origin: String
    var balances: [Balance]?, isAvailable: Bool?
    var windows: [QuotaWindow], bankReset: BankReset?, bankUpdatedAt: Double?, plan: String?, error: String?
    var id: String { provider + ":" + accountId }
    var queryFailure: String? {
        guard let reason = error?.trimmingCharacters(in: .whitespacesAndNewlines), !reason.isEmpty else { return nil }
        return reason
    }
}
struct SyncIssue: Codable { var path: String?; var message: String; var code: String?; var step: Int? }
func syncFailureReason(error: String?, partial: Bool?, issues: [SyncIssue]?) -> String? {
    if let error { return error }
    guard partial == true else { return nil }
    let visible = (issues ?? []).filter { $0.code != "unknownModel" && $0.code != "missingHistoryBase" }
    if !(issues ?? []).isEmpty && visible.isEmpty { return nil }
    return visible.isEmpty ? "部分记录需要处理" : visible.map { ($0.path.map { $0 + " · " } ?? "") + $0.message }.joined(separator: "；")
}
struct SourceStatus: Codable {
    var partial: Bool?, issues: [SyncIssue]?, forkHistoryIssues: [SyncIssue]?
    var updatedAt: Double?, files: Int?, readBytes: Int?, error: String?
    var failureReason: String? { syncFailureReason(error: error, partial: partial, issues: issues) }
}
struct AutomaticValuationStatus: Codable { var state: String?, note: String?, accountKey: String?, updatedAt: Double? }
struct AutomaticValuationConfig: Codable, Equatable { var enabled: Bool, consentVersion: Int; var mode: String? }
struct SourceSummary: Codable, Identifiable { var id: String, name: String, provider: String, enabled: Bool, status: SourceStatus?, accountIds: [String]?; var autoValuationStatus: AutomaticValuationStatus? }
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
struct AccountDeviceSettings: Codable, Equatable { var machineId: String; var preCommand: String; var preCommandResetRequired: Bool? = nil }
struct AntigravityLoginState: Decodable {
    var id: String, phase: String, message: String, authUrl: String?
    var authenticated: Bool, identityConfirmed: Bool
    var identity: AccountIdentity?, current: Bool?, metadataError: String?
    var accountStatus: AccountDeviceStatus?
    var workspaceConfirmationRequired: Bool?
}
struct AccountTarget: Decodable, Identifiable {
    var accountKey: String, accountName: String, provider: String, sourceId: String, sourceName: String, path: String, machineId: String, machineName: String
    var profileId: String?, current: Bool?, credential: Bool?, checkedAt: Double?, reason: String?
    var canWake: Bool, canSwitch: Bool
    var canDraft: Bool? = nil, available: Bool? = nil, draftReason: String? = nil
    var bound: Bool? = nil
    var id: String { accountKey + ":" + sourceId + ":" + (profileId ?? "") }
    var wakeupLocationLabel: String {
        let location = path.count > 55 ? "…" + path.suffix(54) : path
        let state = available == false ? " · 已暂停" : canWake ? "" : " · 执行待确认"
        return [machineName, location].joined(separator: " · ") + state
    }
}
struct AccountTargetRefresh: Decodable {
    struct Issue: Decodable { var message: String }
    var issues: [Issue]
}
struct AccountDeviceStatus: Codable, Equatable, Identifiable {
    var identity: AccountIdentity? = nil
    var metadataError: String? = nil
    var authenticated: Bool?, identityConfirmed: Bool?
    var accountKey: String, sourceId: String, machineId: String
    var current: Bool?, credential: Bool?, checkedAt: Double?, error: String?, note: String?
    var id: String { accountKey + ":" + sourceId }
    var identitySummary: String? {
        guard let identity else { return authenticated == true ? "登录有效 · 账户身份待确认" : nil }
        return (current == false ? "身份不一致 · 当前登录 " : "正在使用 ") + identity.summary + (error != nil ? " · 上次检查" : "")
    }
}
struct AccountDeviceCounts {
    let total: Int, logged: Int, using: Int
    let pending: Bool
    init(accountKey: String, machines: [String], rows: [AccountDeviceStatus]) {
        let ids = Set(machines)
        let matching = rows.filter { $0.accountKey == accountKey }
        total = ids.count
        logged = ids.filter { id in matching.contains { $0.machineId == id && ($0.current == true || $0.credential == true) } }.count
        using = ids.filter { id in matching.contains { $0.machineId == id && $0.current == true } }.count
        pending = ids.contains { id in let rows = matching.filter { $0.machineId == id }; return rows.isEmpty || rows.contains { $0.current == nil || $0.credential == nil || $0.error != nil } }
    }
    var loginText: String { "已登录设备 \(logged)/\(total)" }
    var usageText: String { "使用中设备 \(using)/\(total)" }
    var inlineSummary: String { loginText + " · " + usageText }
    var summary: String { inlineSummary + (pending ? " · 有设备待确认" : "") }
}
struct QuotaResetNotificationSettings: Codable, Equatable {
    var unexpected: Bool?
    var natural: Bool?
}

struct AgentAccount: Codable, Equatable, Identifiable {
    var quotaResetNotifications: QuotaResetNotificationSettings?
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
    var billingSummary: CodexBillingSummary?
    var generatedAt: Double = 0, summary = Aggregate(), heatmap: [Aggregate] = [], models: [Aggregate] = []
    var trendDays: [Aggregate] = [], dayModels: [DayModel] = [], pricingGaps: [PricingGap] = []
    var modelOptions: [String]? = nil
    var quotaOrder: [String]? = nil, quotaEstimates: [QuotaEstimate]? = nil, creditEstimates: [QuotaEstimate]? = nil
    var automaticEstimateSummaries: [QuotaEstimate]?
    var quotas: [Quota] = [], sources: [SourceSummary] = []
}
struct DashboardStatus: Decodable {
    var quotas: [Quota], sources: [SourceSummary]
    var quotaOrder: [String]?, quotaEstimates: [QuotaEstimate]?, creditEstimates: [QuotaEstimate]?
    var automaticEstimateSummaries: [QuotaEstimate]?
}
extension Dashboard {
    func presentationEstimates(account: String, credits: Bool = false) -> [QuotaEstimate] {
        ((credits ? creditEstimates : quotaEstimates) ?? []).filter { $0.accountKey == account }
        + (automaticEstimateSummaries ?? []).filter { $0.accountKey == account && ($0.kind == "credits") == credits }
    }
    mutating func applyStatus(_ status: DashboardStatus) {
        quotas = status.quotas; sources = status.sources; quotaOrder = status.quotaOrder
        quotaEstimates = status.quotaEstimates; creditEstimates = status.creditEstimates
        automaticEstimateSummaries = status.automaticEstimateSummaries
    }
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
struct SubscriptionConfig: Codable, Equatable {
    var mode = "", credentialRef = "", workspaceId = ""
    var nativeConfigDir: String?
}
struct AgentSource: Codable, Equatable, Identifiable {
    var displayLocation: String {
        guard !path.isEmpty else { return name }
        let shortened = path.count > 60 ? "…" + path.suffix(59) : path
        return name + " · " + shortened
    }
    var machineBinding: String?
    var lastMachineName: String?
    var unbound: Bool { machineBinding == "unbound" }
    var autoValuation: AutomaticValuationConfig?
    var subscription: SubscriptionConfig?
    var codexHomeId: String?
    var id = UUID().uuidString, name = "", provider = "codex", accountId = ""
    var path = "", hostId: String?, enabled = true, quotaCommand = "", quotaPreCommand = "", codexBinary = CoreContract.binaries["codex"]!, agyBinary: String? = CoreContract.binaries["antigravity"], proxy: ProxySettings?
}
struct Host: Codable, Equatable, Identifiable {
    var monitoringEnabled: Bool?
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
    var theme = "system", accent = "indigo", chartPalette = "classic", chartContrast = "high"
    init() {}
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        theme = try container.decodeIfPresent(String.self, forKey: .theme) ?? "system"
        accent = try container.decodeIfPresent(String.self, forKey: .accent) ?? "indigo"
        chartPalette = try container.decodeIfPresent(String.self, forKey: .chartPalette) ?? "classic"
        chartContrast = try container.decodeIfPresent(String.self, forKey: .chartContrast) ?? "high"
    }
}
struct LocalMonitor: Codable, Equatable {
    var enabled = true
    var metrics = Host().metrics, devices: [String] = [], details: [String]? = Host().details
    var host: Host { var h = Host(); h.id = "local"; h.name = "本机"; h.target = "本机"; h.enabled = enabled; h.metrics = metrics; h.devices = devices; h.details = details; return h }
    init() {}
    init(_ h: Host) { enabled = h.enabled; metrics = h.metrics; devices = h.devices; details = h.details }
}
struct Settings: Codable, Equatable {
    var quotaResetNotifications: QuotaResetNotificationSettings?
    var codexApis: [CodexAPIConnection]?
    var deletedAccounts: [AgentAccount]?
    var historicalAccounts: [AgentAccount] { accounts + (deletedAccounts ?? []) }
    var historyShowDeletedAccounts: Bool?
    var codexPreciseCounting: Bool?
    var filterAccounts: [AgentAccount] { accounts + (historyShowDeletedAccounts == true ? deletedAccounts ?? [] : []) }
    func accountFilterName(_ account: AgentAccount) -> String {
        account.name + (accounts.contains { $0.key == account.key } ? (account.archived == true ? "（已归档）" : "") : "（已删除）")
    }
    var agents: [AgentConfiguration]?
    var accountAliases: [String: String]?

    var appearance: AppearanceSettings?

    var configurationIssues: [String]?
    var localDeviceEnabled: Bool?
    var version = 10, sources: [AgentSource] = [], accounts: [AgentAccount] = [], hosts: [Host] = [], proxy = ProxySettings()
    var refreshSeconds = 300, serverRefreshSeconds = 10, menuMetric = "icon"
    var historyRefreshSeconds: Int?, serverForegroundRefreshSeconds: Int?, localMonitor: LocalMonitor?
    var historyInterval: Int { get { historyRefreshSeconds ?? max(10, refreshSeconds) } set { historyRefreshSeconds = newValue } }
    var foregroundInterval: Int { get { serverForegroundRefreshSeconds ?? 2 } set { serverForegroundRefreshSeconds = newValue } }
    var monitoredHosts: [Host] {
        let local = (localMonitor ?? LocalMonitor()).host
        return [local] + hosts.map { var host = $0; host.enabled = host.monitoringEnabled != false; return host }
    }
    func sourceEnabled(_ source: AgentSource) -> Bool {
        let configured = (agents ?? []).filter { $0.provider == source.provider }
        return source.enabled && !source.unbound && (source.hostId != nil || localDeviceEnabled != false) && configured.count <= 1
            && (configured.first.map { $0.enabled && $0.machineIds.contains(source.hostId ?? "local") } ?? true)
            && (source.hostId == nil || hosts.contains { $0.id == source.hostId && $0.enabled })
    }
    var usageSources: [AgentSource] { sources.filter { $0.provider != "deepseek" && sourceEnabled($0) } }
    var modelMappings: [String: String] = [:]
    var proxyTestUrls: [String]?
}
struct ModelPrice: Codable, Equatable, Identifiable {
    var id = "", name = "", input: Double?, output: Double?, cacheRead: Double?, cacheWrite: Double?, fetchedAt: Double = 0
}
struct CodexAPIConnection: Codable, Equatable, Identifiable {
    var id = "", sourceId = "", name = "OpenAI API", baseUrl = "https://api.openai.com/v1", revision = ""
}
struct CodexSelectedConnection: Decodable { var kind: String; var provider: String?, connectionId: String?, revision: String?, name: String? }
struct CodexBillingSummary: Codable {
    var apiCost: Double?, apiTokens: Double?, apiPricedTokens: Double?, apiHistoryTokens: Double?, apiUnmeteredTokens: Double?
    var subscriptionEquivalentCost: Double?, subscriptionTokens: Double?, subscriptionPricedTokens: Double?, unknownTokens: Double?, incompleteRequests: Int?
    var visible: Bool {
        let accountedTokens: Double = (apiTokens ?? 0) + (subscriptionTokens ?? 0)
        let otherTokens: Double = (unknownTokens ?? 0) + (apiUnmeteredTokens ?? 0)
        return accountedTokens + otherTokens > 0 || (incompleteRequests ?? 0) > 0
    }
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
struct HostMonitorState: Decodable {
    var rows: [HostResult], inFlight: [String], pending: [String]
    var sampleSession: String, version: UInt64, paused: Bool
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
            if window.windowMinutes == CoreContract.fiveHourMinutes { return "5h" }
            if window.windowMinutes == CoreContract.weekMinutes { return "7d" }
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
    static func counterTotal(_ received: Double?, _ sent: Double?) -> String {
        guard let received, let sent, received.isFinite, sent.isFinite, received >= 0, sent >= 0, (received + sent).isFinite else { return "—" }
        return compact(received + sent)
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
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = .autoupdatingCurrent
        formatter.dateFormat = "MM/dd HH:mm"
        return formatter.string(from: Date(timeIntervalSince1970: stamp))
    }
    static func relative(_ date: Date, now: Date = Date()) -> String {
        let formatter = RelativeDateTimeFormatter(); formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: now)
    }
    static func time(_ stamp: Double) -> String { Date(timeIntervalSince1970: stamp).formatted(date: .omitted, time: .shortened) }
    static func provider(_ key: String) -> String { CoreContract.providerNames[key] ?? key }
}

struct QuotaEstimate: Codable, Identifiable {
    var samplingMode: String?
    var sampleCount: Int?, lookbackDays: Int?, halfLifeDays: Int?
    var quotaPlan: String?
    var checkpointWindows: [QuotaWindow]?
    var recordKey: String { (kind == "credits" ? "credits:" : "quota:") + id }
    var groupId: String?

    var calculationStatus: String?, originalEstimateId: String?, repairedAt: Double?
    var rollingExcludedReason: String?
    var hasValue: Bool { [fiveHourValue, weeklyValue, valuePer1000].compactMap { $0 }.contains { $0.isFinite } }
    var issueLabel: String {
        if let reason = rollingExcludedReason, !reason.isEmpty { return reason }
        if status == "pending" { return reason.isEmpty ? "待确认" : reason }
        if let code = calculationStatus, let label = ["boundary":"用量边界待确认", "noUsage":"暂无有效用量", "unpriced":"模型价格不完整", "insufficientUsage":"等待可确认的消耗", "invalidPrice":"价格数据无效"][code] { return label }
        return calculationNote.isEmpty ? "尚无估值" : calculationNote
    }

    var kind: String?, consumedCredits: Double?, valuePer1000: Double?
    var id: String, accountKey: String, windowId: String, windowName: String
    var sourceIds: [String], sourceNames: [String], status: String, reason: String
    var startedAt: Double, checkpointAt: Double, endedAt: Double?
    var consumedPercent: Double?
    var cost: Double, totalTokens: Double, pricedTokens: Double
    var weeklyValue: Double?, calculationNote: String
    var prices: [ModelPrice]?
    var valuationMode: String?, fiveHourValue: Double?, weeklyDirectValue: Double?, weeklyRatioValue: Double?
    var capacity: CapacityInfo?, segments: [EstimateSegment]?
    var statusLabel: String { ["active":"采样中", "pending":"需要确认", "completed":"已结束"][status] ?? status }
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
    struct Visibility {
        var five: Bool, direct: Bool, ratio: Bool
        var rolling = false
        var periods: String { [five ? "5h" : "", direct || ratio ? "7d" : ""].filter { !$0.isEmpty }.joined(separator: " / ") }
        var help: String {
            if rolling { return "滚动估值：汇总最近 90 天的有效片段，权重每 7 天减半，按额度消耗量加权。" + (five ? "5h 按对应额度片段计算。" : "") + (direct ? "7d 按对应周额度片段计算。" : "") + (ratio ? "7d 容量推算为滚动 5h 价值乘近期容量倍率。" : "") + "金额为 API 等价价值，不可兑换。" }
            if ratio && direct { return "7d 左值：5h 等价值 × 近期容量倍率；右值：同一采样时段的 API 等价成本 ÷ 周额度消耗比例。旧记录显示整周估值；这些金额为 API 等价价值。" }
            if ratio { return "7d：5h 等价值 × 近期容量倍率；这些金额为 API 等价价值。" }
            if direct { return "7d：同一采样时段的 API 等价成本 ÷ 周额度消耗比例。旧记录显示整周估值；这些金额为 API 等价价值。" }
            return "按采样时段的 API 等价成本与对应额度消耗比例计算。"
        }
    }
    static func visibility(_ quota: Quota?, records: [QuotaEstimate], group: String? = nil) -> Visibility {
        let latest = canonical(records).first
        let scope = group.flatMap { $0.isEmpty ? nil : $0 } ?? latest?.groupId ?? ""
        let windows = quota.flatMap { $0.windows.isEmpty ? nil : $0.windows } ?? latest?.checkpointWindows ?? []
        let rows = scope.isEmpty ? windows : windows.filter { ($0.groupId.flatMap { $0.isEmpty ? nil : $0 } ?? $0.groupName) == scope }
        let five = windows.isEmpty ? latest?.valuationMode == "fiveHour" : rows.contains { $0.windowMinutes == CoreContract.fiveHourMinutes }
        let provider = quota?.provider ?? latest?.accountKey.components(separatedBy: ":").first
        let plan = (quota?.plan.flatMap { $0.isEmpty ? nil : $0 } ?? latest?.quotaPlan ?? "").trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return Visibility(five: five, direct: !(provider == "codex" && plan == "plus"), ratio: five, rolling: latest?.samplingMode == "rolling")
    }
    static func windows(_ quota: Quota) -> [QuotaWindow] {
        if quota.provider == "antigravity" {
            func group(_ window: QuotaWindow) -> String {
                window.groupId.flatMap { $0.isEmpty ? nil : $0 } ?? window.groupName ?? ""
            }
            var groups = Set<String>()
            return quota.windows.compactMap { window in
                let key = group(window)
                guard groups.insert(key).inserted,
                      ["gemini", "gemini models", "claude-gpt", "claude and gpt models"].contains(key.lowercased()) else { return nil }
                let rows = quota.windows.filter { group($0) == key }
                let five = rows.filter { $0.windowMinutes == CoreContract.fiveHourMinutes }
                let week = rows.filter { $0.windowMinutes == CoreContract.weekMinutes }
                return five.count == 1 ? five.first : five.isEmpty && week.count == 1 ? week.first : nil
            }
        }
        let five = quota.windows.filter { $0.windowMinutes == CoreContract.fiveHourMinutes }
        if five.count == 1 { return five }
        if quota.provider == "claude", let overall = five.first(where: { $0.stableId == "five_hour" }) { return [overall] }
        let week = quota.windows.filter { $0.windowMinutes == CoreContract.weekMinutes }
        return week.count == 1 ? week : week.filter { quota.provider == "claude" && $0.stableId == "seven_day" }
    }
    struct Entry: Identifiable {
        var id: String, label: String, value: Double, record: QuotaEstimate
        var historical: Bool
    }
    static func canonical(_ records: [QuotaEstimate]) -> [QuotaEstimate] {
        let replaced = Set(records.compactMap(\.originalEstimateId))
        return records.filter { !replaced.contains($0.id) && ($0.rollingExcludedReason ?? "").isEmpty }.sorted {
            if ($0.samplingMode == "rolling") != ($1.samplingMode == "rolling") { return $0.samplingMode == "rolling" }
            return $0.startedAt == $1.startedAt ? ($0.repairedAt ?? 0) > ($1.repairedAt ?? 0) : $0.startedAt > $1.startedAt
        }
    }
    static func entries(_ records: [QuotaEstimate], credits: Bool = false, quota: Quota? = nil, group: String? = nil) -> [Entry] {
        let canonical = canonical(records)
        let rolling = canonical.filter { $0.samplingMode == "rolling" }
        let all = rolling.isEmpty ? canonical : rolling
        let visible = visibility(quota, records: records, group: group)
        let fields: [(String, String, KeyPath<QuotaEstimate, Double?>)] = credits
            ? [("credits", "1000 credits", \.valuePer1000)]
            : [("five", "5h", \.fiveHourValue), ("week", "7d 同期 / 整周", \.weeklyValue), ("ratio", "7d 容量倍率", \.weeklyRatioValue)]
        return fields.compactMap { id, label, path in
            guard credits || (id == "five" ? visible.five : id == "week" ? visible.direct : visible.ratio) else { return nil }
            guard let record = all.first(where: { $0.status != "pending" && $0[keyPath: path]?.isFinite == true }), let value = record[keyPath: path] else { return nil }
            let title = id == "week" ? (record.valuationMode == "fiveHour" ? "7d 同期" : "7d 整周") : label
            return Entry(id: id, label: title, value: value, record: record, historical: record.id != all.first?.id || record.status == "completed")
        }
    }
    static func summary(_ records: [QuotaEstimate], quota: Quota? = nil, group: String? = nil) -> String {
        let visible = visibility(quota, records: records, group: group)
        let values = entries(records, quota: quota, group: group)
        func value(_ id: String) -> String { Format.compactMoney(values.first { $0.id == id }?.value) }
        let week = [visible.ratio ? value("ratio") : nil, visible.direct ? value("week") : nil].compactMap { $0 }
        let parts = [visible.five ? "5h ≈ " + value("five") : "", week.isEmpty ? "" : "7d ≈ " + week.joined(separator: "/")].filter { !$0.isEmpty }
        return parts.isEmpty ? "额度估值" : parts.joined(separator: "  ")
    }
    static func credit(_ records: [QuotaEstimate]) -> QuotaEstimate? { entries(records, credits: true).first?.record }
}

struct AccountCleanupTask: Decodable, Identifiable { var id: String, name: String, machine: String; var root: String?, error: String? }
struct AccountDeletionPreview: Decodable { var tasks: [AccountCleanupTask] }
struct AccountCleanupResult: Decodable { var deleted: Bool?, settings: Settings?, failedTasks: [AccountCleanupTask]? }
struct AccountCleanupRecord: Decodable, Identifiable { var accountKey: String, name: String, tasks: [AccountCleanupTask]; var id: String { accountKey } }

// BEGIN GENERATED CORE CONTRACT
// Generated by scripts/sync-core-contracts.py; edit docs/core-contracts.json.
enum CoreContract {
    static let contractHash = "cde493bff081e56afcfa2ebb506848f129864a2a530673ad89938d731f157733"
    static let providers = ["codex", "claude", "antigravity", "deepseek", "zcode", "qoder", "grok", "cursor", "opencode", "kimi", "deepseek-harness", "codebuddy", "pi", "omp", "custom"]
    static let wakeupProviders = ["codex", "claude", "antigravity"]
    static let agentProviders = ["codex", "claude", "antigravity", "deepseek", "zcode", "qoder", "grok", "cursor", "opencode", "kimi", "codebuddy", "pi", "omp", "custom"]
    static let agentAliases: [String: String] = ["deepseek-harness": "deepseek"]
    static let providerCapabilities: [String: Set<String>] = ["codex": ["accounts", "api", "machines", "quota", "quotaReset", "valuation"], "claude": ["accounts", "api", "machines", "quota", "quotaReset", "valuation"], "antigravity": ["accounts", "machines", "quota", "quotaReset", "valuation"], "deepseek": ["accounts", "quota"], "zcode": ["accounts", "api", "machines", "quota", "quotaReset", "valuation"], "qoder": ["accounts", "api", "machines", "quota", "quotaReset"], "grok": ["accounts", "api", "machines", "quota", "quotaReset"], "cursor": ["accounts", "machines", "quota", "quotaReset"], "opencode": ["accounts", "api", "machines", "quota", "quotaReset", "valuation"], "kimi": ["accounts", "api", "machines", "quota", "quotaReset", "valuation"], "deepseek-harness": ["accounts", "api", "machines", "quota"], "codebuddy": ["accounts", "api", "machines"], "pi": ["accounts", "api", "machines"], "omp": ["accounts", "api", "machines"], "custom": ["accounts", "machines"]]
    static let providerIcons: [String: String] = ["codex": "codex", "claude": "claude", "antigravity": "antigravity", "deepseek": "deepseek", "zcode": "zcode", "qoder": "qoder", "grok": "grok", "cursor": "cursor", "opencode": "opencode", "kimi": "kimi", "deepseek-harness": "deepseek", "codebuddy": "codebuddy", "pi": "pi", "omp": "omp"]
    static let providerDarkIcons: [String: String] = ["opencode": "opencode-dark", "kimi": "kimi-dark", "omp": "omp-dark"]
    static let monochromeIcons: Set<String> = ["codex", "zcode", "qoder", "grok", "cursor"]
    static func agentProvider(_ provider: String) -> String { agentAliases[provider] ?? provider }
    static func sameAgent(_ lhs: String, _ rhs: String) -> Bool { agentProvider(lhs) == agentProvider(rhs) }
    static func supports(_ provider: String, _ feature: String) -> Bool { providerCapabilities[provider]?.contains(feature) == true }
    static func featureProvider(_ provider: String, _ feature: String) -> String? { providers.first { sameAgent($0, provider) && supports($0, feature) } }
    static let providerNames: [String: String] = ["codex": "Codex", "claude": "Claude Code", "antigravity": "Antigravity", "deepseek": "DeepSeek", "zcode": "ZCode", "qoder": "Qoder", "grok": "Grok", "cursor": "Cursor", "opencode": "OpenCode", "kimi": "Kimi Code", "deepseek-harness": "DeepSeek", "codebuddy": "CodeBuddy", "pi": "Pi", "omp": "Oh My Pi", "custom": "自定义"]
    static let defaultPaths: [String: String] = ["codex": "~/.codex", "claude": "~/.claude", "antigravity": "~/.gemini/antigravity-cli", "deepseek": "", "zcode": "~/.zcode", "qoder": "~/.qoder", "grok": "~/.grok", "cursor": "~/.cursor", "opencode": "~/.local/share/opencode", "kimi": "~/.kimi-code", "deepseek-harness": "~/.dsh", "codebuddy": "~/.codebuddy", "pi": "~/.pi/agent", "omp": "~/.omp/agent", "custom": "~/.aieyes"]
    static let binaries: [String: String] = ["codex": "codex", "claude": "claude", "antigravity": "agy", "deepseek": "", "zcode": "zcode", "qoder": "qodercli", "grok": "grok", "cursor": "agent", "opencode": "", "kimi": "kimi", "deepseek-harness": "dsh", "codebuddy": "codebuddy", "pi": "pi", "omp": "omp", "custom": ""]
    static let subscriptionHints: [String: String] = ["deepseek": "输入 DeepSeek 官方 API Key 查询账户余额。第三方模型的额度请在对应服务商管理。", "zcode": "从对应地区的 Z.ai / 智谱账户获取 Coding Plan API Key。", "qoder": "CLI 登录直接查询当前账户 Credits，无需复制令牌；请先运行 qodercli login。也可使用对应站点 Cookie，旧中国站与 qoder.cn 新账户不互通。", "grok": "订阅：运行 grok login，从 ~/.grok/auth.json 复制账户 key。API 余额：填写 Management API Key 和 Team ID。", "cursor": "从 Cursor 用量页复制含 WorkosCursorSessionToken 的请求 Cookie。", "opencode": "Go 使用 API Key，或 Console Cookie 与 Workspace ID 查询 Go / Zen 余额。外部订阅请连接对应服务商。", "kimi": "使用 Kimi Code 订阅的 API Key 或有效访问令牌；Moonshot 开放平台 Key 与自建模型不提供此订阅限额。请选择凭据对应地区。", "deepseek-harness": "仅用于 Harness 的 DeepSeek 官方 API 后端：输入该后端的 API Key 查询余额。自建及其他第三方模型不共用此余额。"]
    static let subscriptionLoginURLs: [String: String] = ["deepseek": "https://platform.deepseek.com/usage", "zcode": "https://zcode.z.ai/en/docs/configuration", "qoder": "https://qoder.com/account/usage", "grok": "https://grok.com", "cursor": "https://cursor.com/dashboard", "opencode": "https://opencode.ai/console", "kimi": "https://www.kimi.com/code/console", "deepseek-harness": "https://platform.deepseek.com/usage"]
    static let subscriptionModes: [String: [String: String]] = ["deepseek": ["api": "DeepSeek 官方 API 余额"], "zcode": ["global": "Z.ai 国际版 API Key", "cn": "智谱中国版 API Key"], "qoder": ["cli": "本机 Qoder CLI 登录", "global": "qoder.com Cookie", "cn-legacy": "qoder.com.cn 旧站点 Cookie"], "grok": ["subscription": "Grok 订阅登录令牌", "api": "xAI API 余额"], "cursor": ["session": "Cursor 会话 Cookie"], "opencode": ["go": "Go API Key", "console": "Console Cookie / Workspace"], "kimi": ["cn": "Kimi Code 中国版 API Key / 访问令牌", "global": "Kimi Code 国际版 API Key / 访问令牌"], "deepseek-harness": ["api": "DeepSeek 官方 API 余额"]]
    static let apiGuidance: [String: [String: String]] = ["antigravity": ["hint": "agy 自定义端点使用 Gemini 协议：在 CLI settings.json 设置 modelProvider=gemini，并在启动环境提供 GEMINI_API_KEY 和 GOOGLE_GEMINI_BASE_URL。OpenAI 兼容服务可通过官方 Python SDK 的 LocalOpenAIAgentConfig 使用。", "url": "https://www.antigravity.google/docs/install?tab=cli", "sdkUrl": "https://www.antigravity.google/docs/sdk/local-models"], "cursor": ["hint": "Cursor CLI 的 CURSOR_API_KEY 使用 Cursor 账户生成的用户 API Key。--endpoint / CURSOR_API_ENDPOINT 指向兼容 Cursor 服务协议的端点，包含账户密钥交换；普通 OpenAI 兼容模型地址无法直接替代。模型请通过 /model 选择。", "label": "CLI 登录与 API Key 说明", "url": "https://cursor.com/docs/cli/reference/authentication", "configurationUrl": "https://cursor.com/docs/cli/reference/configuration"], "pi": ["url": "https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/models.md", "hint": "配置 Pi 原生 API 模型。Pi 不提供统一订阅余额；额度由所选模型服务商提供。", "label": "Pi 模型与额度"], "omp": ["url": "https://github.com/can1357/oh-my-pi/blob/main/docs/models.md", "hint": "配置 Oh My Pi 原生 API 模型。额度属于所选模型服务商，不能由本地 Token 或预估费用推算余额。", "label": "Oh My Pi 模型与额度"]]
    static let fiveHourMinutes = 300
    static let weekMinutes = 10080
    static let excludedFilesystemTypes: Set<String> = ["tmpfs", "devtmpfs", "squashfs", "overlay", "proc", "procfs", "sysfs", "devfs", "autofs", "cgroup", "cgroup2"]
    static let excludedFilesystemPaths = ["/snap", "/var/lib/snapd/snap", "/run", "/efi", "/boot/efi", "/dev", "/proc", "/sys", "/system/volumes/preboot", "/system/volumes/vm", "/system/volumes/update", "/system/volumes/xarts", "/system/volumes/iscpreboot", "/system/volumes/hardware"]
    static let petFrameDurations: [[Int]] = [[280, 110, 110, 140, 140, 320], [120, 120, 120, 120, 120, 120, 120, 220], [120, 120, 120, 120, 120, 120, 120, 220], [140, 140, 140, 280], [140, 140, 140, 140, 280], [140, 140, 140, 140, 140, 140, 140, 240], [150, 150, 150, 150, 150, 260], [120, 120, 120, 120, 120, 220], [150, 150, 150, 150, 150, 280]]
    static let methodLanes: [String: String] = [
        "accounts.cleanup.list": "configuration",
        "accounts.connect": "configuration",
        "accounts.create": "configuration",
        "accounts.delete": "accounts",
        "accounts.deletion.preview": "configuration",
        "accounts.deployments.sync": "accounts",
        "accounts.status.get": "configuration",
        "accounts.status.refresh": "accounts",
        "accounts.status.refreshBatch": "accounts",
        "accounts.targets": "configuration",
        "agentApi.inspect": "configuration",
        "agentApi.save": "main",
        "agents.set": "configuration",
        "agyAuth.cancel": "accounts",
        "agyAuth.confirmWorkspace": "accounts",
        "agyAuth.inspect": "accounts",
        "agyAuth.openUrl": "accounts",
        "agyAuth.start": "accounts",
        "agyAuth.status": "accounts",
        "agyAuth.submitCode": "accounts",
        "agyAuth.verify": "accounts",
        "autoEstimates.configure": "configuration",
        "autoEstimates.tick": "main",
        "autoEstimates.rebuild": "main",
        "codexApi.list": "accounts",
        "codexApi.probe": "accounts",
        "codexApi.remove": "accounts",
        "codexApi.save": "accounts",
        "codexApi.status": "accounts",
        "codexAuth.adopt": "main",
        "codexAuth.enable": "main",
        "codexAuth.inspect": "main",
        "codexAuth.list": "main",
        "codexAuth.login.cancel": "main",
        "codexAuth.login.start": "main",
        "codexAuth.login.status": "main",
        "codexAuth.model.list": "main",
        "codexAuth.openUrl": "main",
        "codexAuth.profiles.bind": "main",
        "codexAuth.profiles.list": "main",
        "codexAuth.profiles.remove": "main",
        "codexAuth.quota": "quotas",
        "codexAuth.reconcile": "accounts",
        "codexAuth.status": "main",
        "codexAuth.switch.commit": "main",
        "codexAuth.switch.prepare": "main",
        "credentials.save": "main",
        "creditEstimates.get": "main",
        "creditEstimates.list": "main",
        "creditEstimates.repair": "main",
        "creditEstimates.restart": "main",
        "creditEstimates.start": "main",
        "creditEstimates.stop": "main",
        "dashboard": "dashboard",
        "dashboard.summary": "dashboard",
        "hello": "main",
        "hosts.credentials.delete": "hosts",
        "hosts.credentials.save": "hosts",
        "hosts.discover": "hosts",
        "hosts.sample": "hosts",
        "hosts.sshConfig.read": "hosts",
        "methods.manifest": "main",
        "network.status": "network",
        "network.test": "network",
        "operations.prepareUpdate": "configuration",
        "operations.resume": "configuration",
        "operations.status": "configuration",
        "pets.asset": "configuration",
        "pets.current": "configuration",
        "pets.import": "network",
        "pets.list": "configuration",
        "pets.select": "configuration",
        "prices.list": "main",
        "prices.recalculate": "main",
        "prices.save": "main",
        "prices.sync": "main",
        "quotaEstimates.get": "main",
        "quotaEstimates.list": "main",
        "quotaEstimates.repair": "main",
        "quotaEstimates.restart": "main",
        "quotaEstimates.start": "main",
        "quotaEstimates.stop": "main",
        "quotaResetNotifications.take": "configuration",
        "quotas.order.set": "main",
        "quotas.refresh": "quotas",
        "quotas.schedule": "dashboard",
        "runtime.revisions": "configuration",
        "sessions.hooks.remove": "configuration",
        "sessions.hooks.repair": "configuration",
        "sessions.hooks.status": "configuration",
        "sessions.list": "main",
        "settings.get": "configuration",
        "settings.patch": "configuration",
        "settings.save": "configuration",
        "sources.defaultPath": "configuration",
        "sources.configure": "configuration",
        "sources.rebind": "configuration",
        "sources.remove": "configuration",
        "sources.rescan": "main",
        "sources.scan": "main",
        "subscriptions.connect": "accounts",
        "subscriptions.openLogin": "accounts",
        "wakeups.delete": "wakeups",
        "wakeups.deploy": "wakeups",
        "wakeups.disable": "wakeups",
        "wakeups.enable": "wakeups",
        "wakeups.history": "wakeups",
        "wakeups.list": "wakeups",
        "wakeups.probe": "wakeups",
        "wakeups.remove": "wakeups",
        "wakeups.run": "wakeups",
        "wakeups.save": "wakeups",
        "wakeups.status": "wakeups",
        "dashboard.status": "dashboard",
        "hosts.monitor.configure": "hosts",
        "hosts.monitor.refresh": "hosts",
    ]
    static let updateBehaviors: [String: String] = ["accounts.cleanup.list": "observe", "accounts.connect": "finish", "accounts.create": "finish", "accounts.delete": "finish", "accounts.deletion.preview": "finish", "accounts.deployments.sync": "finish", "accounts.status.get": "observe", "accounts.status.refresh": "cancel", "accounts.status.refreshBatch": "cancel", "accounts.targets": "observe", "agentApi.inspect": "observe", "agentApi.save": "finish", "agents.set": "finish", "agyAuth.cancel": "complete", "agyAuth.confirmWorkspace": "complete", "agyAuth.inspect": "finish", "agyAuth.openUrl": "complete", "agyAuth.start": "finish", "agyAuth.status": "complete", "agyAuth.submitCode": "complete", "agyAuth.verify": "complete", "autoEstimates.configure": "finish", "autoEstimates.tick": "cancel", "autoEstimates.rebuild": "cancel", "codexApi.list": "observe", "codexApi.probe": "cancel", "codexApi.remove": "finish", "codexApi.save": "finish", "codexApi.status": "observe", "codexAuth.adopt": "finish", "codexAuth.enable": "finish", "codexAuth.inspect": "finish", "codexAuth.list": "observe", "codexAuth.login.cancel": "complete", "codexAuth.login.start": "finish", "codexAuth.login.status": "complete", "codexAuth.model.list": "cancel", "codexAuth.openUrl": "complete", "codexAuth.profiles.bind": "finish", "codexAuth.profiles.list": "observe", "codexAuth.profiles.remove": "finish", "codexAuth.quota": "cancel", "codexAuth.reconcile": "cancel", "codexAuth.status": "cancel", "codexAuth.switch.commit": "complete", "codexAuth.switch.prepare": "finish", "credentials.save": "finish", "creditEstimates.get": "cancel", "creditEstimates.list": "observe", "creditEstimates.repair": "finish", "creditEstimates.restart": "finish", "creditEstimates.start": "finish", "creditEstimates.stop": "finish", "dashboard": "cancel", "dashboard.summary": "cancel", "hello": "observe", "hosts.credentials.delete": "finish", "hosts.credentials.save": "finish", "hosts.discover": "cancel", "hosts.sample": "cancel", "hosts.sshConfig.read": "cancel", "methods.manifest": "observe", "network.status": "finish", "network.test": "cancel", "operations.prepareUpdate": "observe", "operations.resume": "observe", "operations.status": "observe", "pets.asset": "observe", "pets.current": "observe", "pets.import": "cancel", "pets.list": "observe", "pets.select": "finish", "prices.list": "observe", "prices.recalculate": "finish", "prices.save": "finish", "prices.sync": "cancel", "quotaEstimates.get": "cancel", "quotaEstimates.list": "observe", "quotaEstimates.repair": "finish", "quotaEstimates.restart": "finish", "quotaEstimates.start": "finish", "quotaEstimates.stop": "finish", "quotaResetNotifications.take": "complete", "quotas.order.set": "finish", "quotas.refresh": "cancel", "quotas.schedule": "cancel", "runtime.revisions": "observe", "sessions.hooks.remove": "finish", "sessions.hooks.repair": "finish", "sessions.hooks.status": "observe", "sessions.list": "cancel", "settings.get": "observe", "settings.patch": "finish", "settings.save": "finish", "sources.defaultPath": "complete", "sources.configure": "finish", "sources.rebind": "finish", "sources.remove": "finish", "sources.rescan": "cancel", "sources.scan": "cancel", "subscriptions.connect": "finish", "subscriptions.openLogin": "finish", "wakeups.delete": "finish", "wakeups.deploy": "finish", "wakeups.disable": "finish", "wakeups.enable": "finish", "wakeups.history": "cancel", "wakeups.list": "observe", "wakeups.probe": "cancel", "wakeups.remove": "finish", "wakeups.run": "finish", "wakeups.save": "finish", "wakeups.status": "cancel", "dashboard.status": "cancel", "hosts.monitor.configure": "observe", "hosts.monitor.refresh": "cancel"]
    static let configurationMethods = Set(methodLanes.filter { $0.value == "configuration" }.keys)
}
// END GENERATED CORE CONTRACT
