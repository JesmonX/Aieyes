import Foundation

final class EngineClient: @unchecked Sendable {
    private let queue = DispatchQueue(label: "app.aieyes.engine", qos: .utility)
    private var process: Process?
    private var input: FileHandle?
    private var output: FileHandle?
    private var buffer = Data()
    private var nextID = 0

    private func start() throws {
        if process?.isRunning == true { return }
        let executable = ProcessInfo.processInfo.environment["AIEYES_CORE_PATH"]
            ?? Bundle.main.url(forResource: "aieyes-core", withExtension: nil)?.path
            ?? FileManager.default.currentDirectoryPath + "/target/debug/aieyes-core"
        guard FileManager.default.isExecutableFile(atPath: executable) else { throw ClientError.message("未找到 Aieyes 核心") }
        let p = Process(), stdin = Pipe(), stdout = Pipe()
        p.executableURL = URL(fileURLWithPath: executable)
        p.standardInput = stdin; p.standardOutput = stdout; p.standardError = FileHandle.nullDevice
        var environment = ProcessInfo.processInfo.environment
        if environment["AIEYES_DATA_DIR"] == nil, let developmentPath = Bundle.main.object(forInfoDictionaryKey: "AieyesDevelopmentDataDirectory") as? String {
            environment["AIEYES_DATA_DIR"] = developmentPath
        }
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        environment["PATH"] = home + "/.local/bin:/opt/homebrew/bin:/opt/local/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:" + (environment["PATH"] ?? "")
        p.environment = environment
        try p.run()
        process = p; input = stdin.fileHandleForWriting; output = stdout.fileHandleForReading; buffer.removeAll()
    }

    func call<T: Decodable>(_ method: String, params: [String: Any] = [:], as type: T.Type = T.self, onProgress: (@Sendable (String) -> Void)? = nil) async throws -> T {
        let payload = try JSONSerialization.data(withJSONObject: params)
        return try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do {
                    try self.start(); self.nextID += 1
                    let parameters = try JSONSerialization.jsonObject(with: payload)
                    let request = try JSONSerialization.data(withJSONObject: ["jsonrpc":"2.0", "id":self.nextID, "method":method, "params":parameters])
                    try self.input?.write(contentsOf: request + Data([10]))
                    while true {
                        if let end = self.buffer.firstIndex(of: 10) {
                            let line = self.buffer[..<end]; self.buffer.removeSubrange(...end)
                            guard let response = try JSONSerialization.jsonObject(with: line) as? [String: Any] else { continue }
                            if response["method"] as? String == "operations.progress", let progress = response["params"] as? [String: Any], let stage = progress["stage"] as? String { onProgress?(stage); continue }
                            guard response["id"] as? Int == self.nextID else { continue }
                            if let error = response["error"] as? [String: Any] { throw ClientError.message(error["message"] as? String ?? "查询失败") }
                            let result = try JSONSerialization.data(withJSONObject: response["result"] ?? [:], options: .fragmentsAllowed)
                            continuation.resume(returning: try JSONDecoder().decode(T.self, from: result)); return
                        }
                        guard let data = self.output?.availableData, !data.isEmpty else { throw ClientError.message("核心连接已断开") }
                        self.buffer.append(data)
                    }
                } catch { continuation.resume(throwing: error) }
            }
        }
    }
    func stop() { process?.terminate() }
    deinit { process?.terminate() }
}
enum ClientError: LocalizedError { case message(String); var errorDescription: String? { if case .message(let text) = self { return text }; return nil } }
struct Acknowledgement: Decodable { var id: String?, name: String?, error: String? }
struct RefreshStatus { var busy = false; var succeededAt: Date?; var error: String? }
struct ActionFailure: Identifiable { var action: String; var itemID: String? = nil; var name: String, reason: String; var id: String { action + ":" + (itemID ?? name) } }
struct RemovalRequest: Identifiable { var kind: String, itemID: String, title: String, explanation: String, affected: [String]; var id: String { kind + ":" + itemID } }


@MainActor final class AppModel: ObservableObject {
    let engine = EngineClient()
    let metricsEngine = EngineClient()
    let networkEngine = EngineClient()
    @Published var networkTest: NetworkTest?
    @Published var networkBusy = false
    private var lastNetworkAttempt = Date.distantPast
    private var networkRevision = 0
    func testNetwork(force: Bool = true) async {
        guard !networkBusy else { return }; networkBusy = true; lastNetworkAttempt = Date()
        let revision = networkRevision
        defer { networkBusy = false }
        do {
            let measured: NetworkTest = try await networkEngine.call("network.test", params: ["force":force])
            if revision == networkRevision { networkTest = measured }
        } catch { if revision == networkRevision { message = error.localizedDescription } }
    }

    private let sessionMonitor = SessionMonitor()
    @Published var sessions: [LiveSession] = []
    @Published var sessionsUnavailable = false
    private var sessionBusy = false
    private var lastSessionRead = Date.distantPast
    var activeSessions: [LiveSession] { sessions.filter { $0.phase.active } }
    var sessionPhase: SessionPhase? { sessionsUnavailable ? .unknown : activeSessions.first?.phase ?? sessions.first?.phase }
    var sessionSummary: String {
        if sessionsUnavailable { return "会话状态不可用" }
        if !settings.sources.contains(where: { $0.enabled && $0.hostId == nil && $0.provider == "codex" }) { return "未接入会话状态" }
        if !activeSessions.isEmpty { return "\(activeSessions.count) 个活跃会话" }
        if sessions.contains(where: { $0.phase == .unknown }) { return "会话状态待确认" }
        return "无活跃会话"
    }
    func refreshSessions() async {
        guard !sessionBusy else { return }
        sessionBusy = true; lastSessionRead = Date()
        let result = await sessionMonitor.read(sources: settings.sources)
        sessions = result.sessions; sessionsUnavailable = result.unavailable
        sessionBusy = false
    }
    @Published var dashboard = Dashboard()
    @Published private(set) var menuDashboard = Dashboard()
    @Published private(set) var panelDashboard = Dashboard()
    @Published private(set) var refreshStates: [String: RefreshStatus] = [:]
    @Published private(set) var actionFailures: [ActionFailure] = []
    @Published private(set) var fallbackModelOptions: [String] = []
    var modelOptions: [String] { dashboard.modelOptions ?? fallbackModelOptions }
    @Published var settings = Settings()
    @Published var settingsDraft = Settings()
    @Published var settingsLoaded = false
    @Published var settingsSaving = false
    @Published var settingsStage = ""
    @Published var settingsStartedAt: Date?
    @Published private(set) var repricing = false
    @Published var settingsMessage: String?
    @Published var mappingModel = ""
    @Published var mappingID = ""
    @Published private(set) var pendingAPIKeys: [String: String] = [:]
    @Published private(set) var pendingHostPasswords: [String: String] = [:]
    private var preparedHostPasswords: [String: (password: String, reference: String)] = [:]
    func stageHostPassword(_ password: String, hostID: String) { if !password.isEmpty { pendingHostPasswords[hostID] = password } }
    private var preparedCredentials: [String: (key: String, path: String)] = [:]
    var settingsChangeCount: Int {
        let rows = settingsDraft.sources.filter { source in settings.sources.first { $0.id == source.id } != source }.count + settings.sources.filter { source in !settingsDraft.sources.contains { $0.id == source.id } }.count
        let hosts = settingsDraft.hosts.filter { host in settings.hosts.first { $0.id == host.id } != host }.count + settings.hosts.filter { host in !settingsDraft.hosts.contains { $0.id == host.id } }.count
        let accounts = settingsDraft.accounts.filter { account in settings.accounts.first { $0.key == account.key } != account }.count
        return max(settingsDirty ? 1 : 0, rows + hosts + accounts + (settingsDraft.modelMappings != settings.modelMappings ? 1 : 0))
    }
    var settingsDirty: Bool { settingsDraft != settings || !pendingAPIKeys.isEmpty || !pendingHostPasswords.isEmpty || !mappingModel.isEmpty || !mappingID.isEmpty }
    func stageAPIKey(_ key: String?, sourceID: String) {
        let trimmed = key?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if trimmed.isEmpty { pendingAPIKeys.removeValue(forKey: sourceID) }
        else { pendingAPIKeys[sourceID] = trimmed }
        if preparedCredentials[sourceID]?.key != trimmed { preparedCredentials.removeValue(forKey: sourceID) }
    }
    @Published var requestedSourceProvider: String?
    @Published var requestedAccountKey: String?
    @Published var requestedDetailPage: String?
    @Published var detailPage = "agent"
    var showDetailPage: ((String) -> Void)?
    @Published var requestHostEditor = false
    @Published var hosts: [HostResult] = []
    @Published var prices: [ModelPrice] = []
    @Published var provider = "all"
    @Published var selectedSource = "all"
    @Published var selectedAccount = "all"
    @Published var settingsTab = "sources"
    @Published var selectedModel = "all"
    @Published var range = 1
    @Published var busy = false
    @Published var serverBusy = false
    @Published var message: String?
    @Published var activity = ""
    @Published var isPinned = false
    @Published var installingUpdate = false
    @Published private(set) var estimateBusy = Set<String>()
    @Published var estimateStages: [String: String] = [:]
    @Published var estimateStartedAt: [String: Date] = [:]
    @Published private(set) var estimateErrors: [String: String] = [:]
    // Dashboard estimates are global, even when usage is filtered by account/model.
    var runningEstimates: [QuotaEstimate] { ((dashboard.quotaEstimates ?? []) + (dashboard.creditEstimates ?? [])).filter { $0.status == "active" || $0.status == "pending" } }
    var samplingNeedsAttention: Bool { runningEstimates.contains { $0.status == "pending" } }
    var samplingSummary: String {
        let active = runningEstimates.filter { $0.status == "active" }.count
        let pending = runningEstimates.count - active
        return [(active > 0 ? "\(active) 项采样中" : nil), (pending > 0 ? "\(pending) 项待确认" : nil)].compactMap { $0 }.joined(separator: " · ")
    }
    @Published var panelHeight: CGFloat = 720
    private var visibleWindows = Set<String>()
    var panelVisible: Bool { !visibleWindows.isEmpty }
    func setWindowVisible(_ visible: Bool, window: String) {
        if visible { visibleWindows.insert(window) } else { visibleWindows.remove(window) }
    }
    private var visibleServerWindows = Set<String>()
    var serverTabVisible: Bool { !visibleServerWindows.intersection(visibleWindows).isEmpty }
    func setServerVisible(_ visible: Bool, window: String) {
        if visible { visibleServerWindows.insert(window) } else { visibleServerWindows.remove(window) }
    }
    @Published var quotaBusy = false
    @Published var quotaError: String?
    @Published var quotaNextAttempt: Date?
    var quotaAccounts: [AgentAccount] { settings.accounts.filter { $0.quotaEnabled && $0.archived != true } }
    var hasQuotaAccounts: Bool { !quotaAccounts.isEmpty }
    private var timer: Timer?
    private var lastScan = Date.distantPast
    private var lastMetrics = Date.distantPast
    private var lastHostError: String?
    var showSettings: (() -> Void)?
    var showDetail: (() -> Void)?
    var showEstimate: ((Quota, Bool) -> Void)?
    var showSampling: (() -> Void)?
    func openEstimate(_ estimate: QuotaEstimate) async {
        do {
            // Use an unfiltered read so changing the usage filter cannot hide sampling controls.
            let snapshot: Dashboard = try await engine.call("dashboard", params: ["days": 1])
            guard let quota = snapshot.quotas.first(where: { $0.id == estimate.accountKey }) else {
                estimateErrors[estimate.accountKey] = "此账户已归档或移除；仍可结束采样并保留有效段。"; return
            }
            showEstimate?(quota, estimate.kind == "credits")
        } catch { estimateErrors[estimate.accountKey] = error.localizedDescription }
    }

    init(autostart: Bool = true) {
        guard autostart else { return }
        Task { await bootstrap() }
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
    }
    func bootstrap() async {
        do {
            settings = try await engine.call("settings.get"); settingsDraft = settings; settingsLoaded = true
            await reload(); await scan(); await refreshQuotas()
        }
        catch { message = error.localizedDescription }
    }
    func openPricing() { settingsTab = "prices"; showSettings?() }
    @Published private(set) var dashboardPending = false
    @Published private(set) var dashboardError: String?
    @Published private(set) var appliedScope = "今日 · 全部 Agent · 全部账户 · 全部来源 · 全部模型"
    private var dashboardRequest = 0
    @discardableResult func reload() async -> Bool {
        dashboardRequest += 1; let request = dashboardRequest
        dashboardPending = true; dashboardError = nil
        let scope = [range == 1 ? "今日" : "最近 \(range) 天", provider == "all" ? "全部 Agent" : Format.provider(provider), settings.accounts.first { $0.key == selectedAccount }?.name ?? (selectedAccount == "none" ? "未关联账户" : "全部账户"), settings.sources.first { $0.id == selectedSource }?.name ?? "全部来源", selectedModel == "all" ? "全部模型" : selectedModel].joined(separator: " · ")
        var params: [String: Any] = ["days":range]
        if provider != "all" { params["provider"] = provider }
        if selectedAccount == "none" { params["accountId"] = "" }
        else if let account = settings.accounts.first(where: { $0.key == selectedAccount }) { params["accountId"] = account.id; params["provider"] = account.provider }
        if selectedSource != "all" { params["sourceId"] = selectedSource }
        if selectedModel != "all" { params["model"] = selectedModel }
        let unfilteredSelected = params.count == 1 && range == 1
        let allModelsSelected = selectedModel == "all"
        do {
            let next: Dashboard = try await engine.call("dashboard", params: params)
            guard request == dashboardRequest else { return true }
            let panel = next
            guard request == dashboardRequest else { return true }
            let unfiltered: Dashboard = unfilteredSelected ? next : try await engine.call("dashboard", params: ["days": 1])
            guard request == dashboardRequest else { return true }
            // Publish all surfaces together so a failed secondary query cannot mix filter scopes.
            dashboard = next; panelDashboard = panel; menuDashboard = unfiltered
            appliedScope = scope; dashboardPending = false
            let names = next.dayModels.map(\.model)
            fallbackModelOptions = Array(Set(allModelsSelected ? names : fallbackModelOptions + names)).sorted()
            return true
        } catch { if request == dashboardRequest { dashboardPending = false; dashboardError = error.localizedDescription }; return false }
    }
    func saveQuotaOrder(_ keys: [String]) async throws {
        let _: [String] = try await engine.call("quotas.order.set", params: ["keys": keys])
        await reload()
    }
    func estimateAction(_ action: String, params: [String: Any], credits: Bool = false) async throws {
        let accountKey: String
        if let requestedKey = params["accountKey"] as? String {
            accountKey = requestedKey
        } else if let estimateID = params["id"] as? String {
            let quotaKey = dashboard.quotaEstimates?.first(where: { $0.id == estimateID })?.accountKey
            let creditKey = dashboard.creditEstimates?.first(where: { $0.id == estimateID })?.accountKey
            accountKey = quotaKey ?? creditKey ?? ""
        } else {
            accountKey = ""
        }
        var operation = params; operation["operationId"] = UUID().uuidString
        let _: QuotaEstimate = try await engine.call((credits ? "creditEstimates." : "quotaEstimates.") + action, params: operation, onProgress: { [weak self] stage in Task { @MainActor in guard let self, self.estimateBusy.contains(accountKey) else { return }; self.estimateStages[accountKey] = stage } })
        await reload()
    }
    func performEstimate(_ action: String, accountKey: String, params: [String: Any], credits: Bool = false) async -> Bool {
        guard !estimateBusy.contains(accountKey) else { return false }
        estimateBusy.insert(accountKey); estimateErrors[accountKey] = nil
        estimateStartedAt[accountKey] = Date(); estimateStages[accountKey] = busy || quotaBusy ? "等待当前刷新" : "准备同步"
        defer { estimateBusy.remove(accountKey); estimateStages.removeValue(forKey: accountKey); estimateStartedAt.removeValue(forKey: accountKey) }
        do { try await estimateAction(action, params: params, credits: credits); return true }
        catch { estimateErrors[accountKey] = error.localizedDescription; await reload(); return false }
    }
    static let refreshLabels = ["scan": "同步记录", "quotas": "刷新限额", "prices": "同步价格", "hosts": "刷新服务器"]
    var usageSources: [AgentSource] { settings.sources.filter { source in source.enabled && !["agy", "deepseek"].contains(source.provider) && (source.hostId == nil || settings.hosts.contains { $0.id == source.hostId && $0.enabled }) } }
    var dataTime: Double {
        let stamps = usageSources.map { source in dashboard.sources.first { $0.id == source.id }?.status?.updatedAt ?? 0 }
        return stamps.min() ?? 0
    }
    var statusText: String {
        let delayed = usageSources.filter { source in
            guard let stamp = dashboard.sources.first(where: { $0.id == source.id })?.status?.updatedAt else { return true }
            return Date().timeIntervalSince1970 - stamp > Double(max(60, settings.refreshSeconds * 2))
        }.count
        let failed = dashboard.sources.contains { $0.enabled && $0.status?.error != nil }
        let prefix = dataTime > 0 ? "记录同步于 " + Format.time(dataTime) : usageSources.isEmpty ? "未接入用量来源" : "记录尚未全部同步"
        return prefix + " · " + (busy ? "同步中…" : failed || !actionFailures.isEmpty ? "部分失败" : delayed > 0 ? "\(delayed) 个来源有延迟" : usageSources.isEmpty ? "等待接入" : "记录已同步")
    }
    var serverStatusText: String {
        let enabled = settings.hosts.filter(\.enabled)
        let updated = enabled.filter { host in
            guard let result = hosts.first(where: { $0.id == host.id }), result.error == nil, let sample = result.sample else { return false }
            return sample.errors.isEmpty && Date().timeIntervalSince1970 - sample.timestamp <= 10
        }.count
        return "\(updated)/\(enabled.count) 台已更新" + (serverBusy ? " · 采样中…" : updated < enabled.count ? " · 存在延迟或失败" : "")
    }
    func refreshLabel(_ key: String) -> String {
        let state = refreshStates[key] ?? RefreshStatus()
        return (Self.refreshLabels[key] ?? key) + " · " + (state.busy ? "进行中…" : state.error != nil ? "失败，可重试" : state.succeededAt.map { Format.time($0.timeIntervalSince1970) } ?? "尚未刷新")
    }
    private func beginRefresh(_ key: String) {
        var state = refreshStates[key] ?? RefreshStatus(); state.busy = true; state.error = nil; refreshStates[key] = state
    }
    private func finishRefresh(_ key: String, failures: [ActionFailure], itemID: String? = nil) {
        var state = refreshStates[key] ?? RefreshStatus(); state.busy = false; state.error = failures.first?.reason
        if failures.isEmpty { state.succeededAt = Date() }
        refreshStates[key] = state
        actionFailures.removeAll { $0.action == key && (itemID == nil || $0.itemID == itemID) }; actionFailures += failures
        state.error = actionFailures.first { $0.action == key }?.reason; refreshStates[key] = state
    }
    func retry(_ failure: ActionFailure) async {
        switch failure.action {
        case "scan": await scan(sourceID: failure.itemID)
        case "quotas": await refreshQuotas(accountID: failure.itemID)
        case "hosts": await sampleHosts(hostID: failure.itemID)
        case "prices": await syncPrices()
        default: await reload()
        }
    }
    func scan(sourceID: String? = nil) async {
        guard !busy, !quotaBusy, estimateBusy.isEmpty else { return }; busy = true; activity = "同步记录"; beginRefresh("scan")
        defer { busy = false; activity = ""; lastScan = Date() }
        do {
            let rows: [Acknowledgement] = try await engine.call("sources.scan", params: sourceID.map { ["sourceId": $0] } ?? [:])
            let failures = rows.compactMap { row -> ActionFailure? in
                guard let error = row.error else { return nil }
                return ActionFailure(action: "scan", itemID: row.id, name: row.name ?? settings.sources.first { $0.id == row.id }?.name ?? "数据源", reason: error)
            }
            let loaded = await reload()
            finishRefresh("scan", failures: failures + (loaded ? [] : [ActionFailure(action: "scan", name: "概览", reason: message ?? "读取失败")]), itemID: sourceID)
        } catch { message = error.localizedDescription; finishRefresh("scan", failures: [ActionFailure(action: "scan", itemID: sourceID, name: "同步记录", reason: error.localizedDescription)], itemID: sourceID) }
    }
    func refreshQuotas(accountID: String? = nil) async {
        guard !quotaBusy, !busy, estimateBusy.isEmpty, hasQuotaAccounts else { return }
        quotaBusy = true; quotaError = nil; beginRefresh("quotas")
        quotaNextAttempt = Date().addingTimeInterval(Double(max(30, settings.refreshSeconds)))
        defer { quotaBusy = false }
        do {
            let result: [Quota] = try await engine.call("quotas.refresh", params: accountID.map { ["accountId": $0] } ?? [:])
            var failures = result.compactMap { row -> ActionFailure? in
                guard let error = row.error else { return nil }
                return ActionFailure(action: "quotas", itemID: row.accountId, name: row.name, reason: error)
            }
            if result.isEmpty { failures = [ActionFailure(action: "quotas", name: "账户限额", reason: "暂未获得限额数据，请检查关联数据源与账户设置。")] }
            quotaError = failures.isEmpty ? nil : "部分账户读取失败，请逐项重试。"
            let loaded = await reload()
            finishRefresh("quotas", failures: failures + (loaded ? [] : [ActionFailure(action: "quotas", name: "概览", reason: message ?? "读取失败")]), itemID: accountID)
        } catch { quotaError = error.localizedDescription; finishRefresh("quotas", failures: [ActionFailure(action: "quotas", itemID: accountID, name: "账户限额", reason: error.localizedDescription)], itemID: accountID) }
    }
    func sampleHosts(hostID: String? = nil) async {
        guard !serverBusy, !settings.hosts.isEmpty else { return }; serverBusy = true; beginRefresh("hosts")
        defer { serverBusy = false; lastMetrics = Date() }
        do {
            let results: [HostResult] = try await metricsEngine.call("hosts.sample", params: hostID.map { ["hostId": $0] } ?? [:])
            let next = results.map { result in
                if result.error != nil, let old = hosts.first(where: { $0.id == result.id }) {
                    return HostResult(id: result.id, name: result.name, sample: result.sample ?? old.sample, error: result.error)
                }
                return result
            }
            hosts = hostID == nil ? next : hosts.filter { $0.id != hostID } + next
            if !results.contains(where: { $0.error != nil }), let lastHostError {
                if message == lastHostError { message = nil }; self.lastHostError = nil
            }
            finishRefresh("hosts", failures: results.compactMap { row in row.error.map { ActionFailure(action: "hosts", itemID: row.id, name: row.name, reason: $0) } }, itemID: hostID)
        } catch {
            let failure = error.localizedDescription; lastHostError = failure; message = failure
            hosts = settings.hosts.map { host in
                let previous = hosts.first { $0.id == host.id }
                return HostResult(id: host.id, name: host.name.isEmpty ? host.target : host.name, sample: previous?.sample, error: host.enabled && (hostID == nil || hostID == host.id) ? failure : previous?.error)
            }
            finishRefresh("hosts", failures: [ActionFailure(action: "hosts", itemID: hostID, name: "服务器", reason: failure)], itemID: hostID)
        }
    }
    func removalRequest(_ kind: String, id: String) -> RemovalRequest? {
        if kind == "host", let host = settingsDraft.hosts.first(where: { $0.id == id }) {
            let linked = settingsDraft.sources.filter { $0.hostId == id }.map(\.name)
            return RemovalRequest(kind: kind, itemID: id, title: "移除服务器「" + (host.name.isEmpty ? host.target : host.name) + "」？", explanation: linked.isEmpty ? "历史用量会保留。此更改随顶部保存提交。" : "以下 \(linked.count) 个数据源将暂停，并需要重新选择服务器。此更改随顶部保存提交。", affected: linked)
        }
        if kind == "source", let source = settingsDraft.sources.first(where: { $0.id == id }) {
            return RemovalRequest(kind: kind, itemID: id, title: "移除数据源「" + source.name + "」？", explanation: "已导入的历史用量会保留。以此来源查询限额的账户需要重新选择来源；此更改随顶部保存提交。", affected: settingsDraft.accounts.filter { $0.quotaSourceId == id }.map(\.name))
        }
        if kind == "account", let account = settingsDraft.accounts.first(where: { $0.key == id }) {
            return RemovalRequest(kind: kind, itemID: id, title: "归档账户「" + account.name + "」？", explanation: "停止展示此账户的实时限额，保留历史用量；此更改随顶部保存提交，可从已归档账户恢复。", affected: [])
        }
        return nil
    }
    func applyRemoval(_ request: RemovalRequest) {
        let id = request.itemID
        switch request.kind {
        case "source":
            stageAPIKey(nil, sourceID: id); settingsDraft.sources.removeAll { $0.id == id }
            for i in settingsDraft.accounts.indices where settingsDraft.accounts[i].quotaSourceId == id { settingsDraft.accounts[i].quotaSourceId = nil }
        case "host":
            pendingHostPasswords.removeValue(forKey: id)
            if let reference = preparedHostPasswords.removeValue(forKey: id)?.reference { Task { let _: Acknowledgement? = try? await engine.call("hosts.credentials.delete", params: ["passwordRef":reference]) } }
            settingsDraft.hosts.removeAll { $0.id == id }
            for i in settingsDraft.sources.indices where settingsDraft.sources[i].hostId == id { settingsDraft.sources[i].hostId = nil; settingsDraft.sources[i].enabled = false }
        case "account":
            if let i = settingsDraft.accounts.firstIndex(where: { $0.key == id }) { settingsDraft.accounts[i].archived = true }
        default: break
        }
    }
    func saveSettingsDraft(refreshDashboard: Bool = true) async -> Bool {
        guard !settingsSaving, !repricing else { return false }
        settingsSaving = true; settingsMessage = nil; settingsStartedAt = Date(); settingsStage = "准备保存"
        defer { settingsSaving = false; settingsStage = ""; settingsStartedAt = nil }
        var next = settingsDraft
        do {
            let from = mappingModel.trimmingCharacters(in: .whitespacesAndNewlines), to = mappingID.trimmingCharacters(in: .whitespacesAndNewlines)
            if !from.isEmpty || !to.isEmpty {
                guard !from.isEmpty && !to.isEmpty else { throw ClientError.message("请补全模型映射两端，输入已保留") }
                guard next.modelMappings[from] == nil || next.modelMappings[from] == to else { throw ClientError.message("该模型已有映射，请先移除原映射再添加新值") }
                next.modelMappings[from] = to
            }
            if !pendingHostPasswords.isEmpty || !pendingAPIKeys.isEmpty { settingsStage = busy || quotaBusy ? "等待当前刷新完成" : "保存凭据" }
            for (id,password) in pendingHostPasswords {
                guard let index = next.hosts.firstIndex(where: { $0.id == id }) else { continue }
                let reference: String
                if let prepared = preparedHostPasswords[id], prepared.password == password { reference = prepared.reference }
                else {
                    let result: [String: String] = try await engine.call("hosts.credentials.save", params: ["password": password])
                    guard let created = result["passwordRef"] else { throw ClientError.message("密码保存失败") }
                    if let obsolete = preparedHostPasswords[id]?.reference { let _: Acknowledgement? = try? await engine.call("hosts.credentials.delete", params: ["passwordRef":obsolete]) }
                    reference = created; preparedHostPasswords[id] = (password,reference)
                }
                next.hosts[index].passwordRef = reference
            }
            for id in pendingAPIKeys.keys.sorted() {
                guard let index = next.sources.firstIndex(where: { $0.id == id && $0.provider == "deepseek" }), let key = pendingAPIKeys[id] else { continue }
                let path: String
                if let prepared = preparedCredentials[id], prepared.key == key { path = prepared.path }
                else {
                    // A fresh storage identity also protects users of older cores that wrote one file per source ID.
                    let result: [String: String] = try await engine.call("credentials.save", params: ["sourceId": id + "-draft-" + UUID().uuidString, "apiKey": key])
                    guard let createdPath = result["path"], !createdPath.isEmpty else { throw ClientError.message("未获得凭据保存路径") }
                    path = createdPath
                    preparedCredentials[id] = (key, path)
                }
                next.sources[index].path = path
            }
            let success = await save(next, refreshDashboard: refreshDashboard)
            if success { mappingModel = ""; mappingID = ""; settingsDraft = settings; pendingAPIKeys.removeAll(); preparedCredentials.removeAll(); pendingHostPasswords.removeAll(); preparedHostPasswords.removeAll() }
            settingsMessage = success ? "已保存全部配置 · " + Format.time(Date().timeIntervalSince1970) : message ?? "保存失败"
            return success
        } catch { settingsMessage = error.localizedDescription; return false }
    }
    func discardSettingsDraft() {
        settingsDraft = settings; settingsMessage = nil; mappingModel = ""; mappingID = ""
        pendingAPIKeys.removeAll(); preparedCredentials.removeAll()
        let references = preparedHostPasswords.values.map(\.reference)
        Task { for reference in references { let _: Acknowledgement? = try? await engine.call("hosts.credentials.delete", params: ["passwordRef":reference]) } }
        pendingHostPasswords.removeAll(); preparedHostPasswords.removeAll()
    }
    func save(_ draft: Settings, refreshDashboard: Bool = true) async -> Bool {
        do {
            let data = try JSONEncoder().encode(draft)
            let params = try JSONSerialization.jsonObject(with: data) as! [String: Any]
            settingsStage = busy || quotaBusy ? "等待当前刷新完成" : "保存配置"
            let _: Acknowledgement = try await engine.call("settings.save", params: params, onProgress: { [weak self] stage in Task { @MainActor in guard let self, self.settingsSaving else { return }; self.settingsStage = stage } })
            let historyConfigurationChanged = settings.accounts != draft.accounts || settings.sources != draft.sources || settings.hosts != draft.hosts || settings.modelMappings != draft.modelMappings
            let quotaConfigurationChanged = settings.accounts != draft.accounts || settings.sources != draft.sources || settings.hosts != draft.hosts || settings.proxy != draft.proxy
            if settings.proxy != draft.proxy || settings.proxyTestUrls != draft.proxyTestUrls { networkRevision += 1; networkTest = nil; lastNetworkAttempt = .distantPast }
            settings = draft
            if quotaConfigurationChanged { quotaNextAttempt = nil }
            if selectedAccount != "all" && selectedAccount != "none" && !settings.accounts.contains(where: { $0.key == selectedAccount }) { selectedAccount = "all" }
            if selectedSource != "all" && !settings.sources.contains(where: { $0.id == selectedSource }) { selectedSource = "all" }
            message = "已保存"
            if refreshDashboard { Task {
                if historyConfigurationChanged { await reload() }
                if hasQuotaAccounts && quotaNextAttempt == nil { await refreshQuotas() }
            } }
            return true
        } catch { message = error.localizedDescription; return false }
    }
    func syncPrices() async {
        guard !busy, !quotaBusy else { return }; busy = true; activity = "同步价格"; beginRefresh("prices"); defer { busy = false; activity = "" }
        do { let _: Acknowledgement = try await engine.call("prices.sync"); prices = try await engine.call("prices.list"); let loaded = await reload(); message = loaded ? "价格已更新" : message
            finishRefresh("prices", failures: loaded ? [] : [ActionFailure(action: "prices", name: "概览", reason: message ?? "读取失败")]) }
        catch { message = error.localizedDescription; finishRefresh("prices", failures: [ActionFailure(action: "prices", name: "价格", reason: error.localizedDescription)]) }
    }
    func loadPrices() async { do { prices = try await engine.call("prices.list") } catch { message = error.localizedDescription } }
    func savePrice(_ price: ModelPrice) async -> Bool {
        do {
            let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(price))
            let _: Acknowledgement = try await engine.call("prices.save", params: ["prices":[object]])
            await loadPrices(); await reload(); message = "已保存价格 · " + Format.time(Date().timeIntervalSince1970); return true
        } catch { message = error.localizedDescription; return false }
    }
    func saveAndReprice() async {
        let mapping = (mappingModel, mappingID)
        guard await saveSettingsDraft(refreshDashboard: false) else { return }
        if await reprice() { settingsMessage = "已保存并按当前价格重算" }
        else { mappingModel = mapping.0; mappingID = mapping.1; settingsMessage = "配置已保存，重算失败：" + (message ?? "请重试") }
    }
    @discardableResult func reprice() async -> Bool {
        guard !repricing else { return false }; repricing = true
        defer { repricing = false }
        do { let _: Acknowledgement = try await engine.call("prices.recalculate"); let loaded = await reload(); message = loaded ? "已按当前价格重算" : "重算已完成，概览读取失败，请重试查询"; return true }
        catch { message = error.localizedDescription; return false }
    }
    private var resetConfirmations = Set<String>()
    func tick() {
        guard !installingUpdate, !settingsSaving, !repricing else { return }
        if panelVisible, !networkBusy, Date().timeIntervalSince(lastNetworkAttempt) >= 300 { Task { await testNetwork(force: false) } }
        if !sessionBusy, Date().timeIntervalSince(lastSessionRead) >= 5 { Task { await refreshSessions() } }
        
        if !serverBusy, !settings.hosts.isEmpty, Date().timeIntervalSince(lastMetrics) > Double(panelVisible && serverTabVisible ? 2 : settings.serverRefreshSeconds) {
            Task { await sampleHosts() }
        }
        if !busy, !quotaBusy, estimateBusy.isEmpty {
            for quota in dashboard.quotas { for window in quota.windows {
                if let reset = window.resetsAt, reset <= Date().timeIntervalSince1970 {
                    let key = quota.id + ":" + String(reset)
                    if resetConfirmations.insert(key).inserted { Task { await refreshQuotas(accountID: quota.accountId) }; return }
                }
            } }
        }
        if hasQuotaAccounts, !quotaBusy, quotaNextAttempt == nil || Date() >= quotaNextAttempt! { Task { await refreshQuotas() } }
        guard !busy else { return }
        if Date().timeIntervalSince(lastScan) > Double(settings.refreshSeconds) { Task { await scan() }; return }
    }
    var menuText: String {
        switch settings.menuMetric {
        case "tokens": return Format.compact(menuDashboard.summary.total)
        case "quota": return menuDashboard.quotas.first?.windows.first.map { String(format: "%.0f%%", max(0, 100 - $0.usedPercent)) } ?? ""
        case "cpu": return hosts.first?.sample?.cpu?.first(where: { $0.id == "cpu" })?.utilization.map { String(format: "%.0f%%", $0) } ?? ""
        default: return ""
        }
    }
}
