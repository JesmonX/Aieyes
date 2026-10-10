import Foundation

final class EngineClient: @unchecked Sendable {
    private let queue = DispatchQueue(label: "app.aieyes.engine", qos: .utility)
    private var process: Process?
    private var input: FileHandle?
    private var output: FileHandle?
    private var buffer = Data()
    private var nextID = 0
    private let configuration: EngineClient?
    private let queries: EngineClient?
    private let lifecycle = NSLock()
    private var stopped = false
    init(configurationOnly: Bool = false) {
        configuration = configurationOnly ? nil : EngineClient(configurationOnly: true)
        queries = configurationOnly ? nil : EngineClient(configurationOnly: true)
    }
    private static let configurationMethods: Set<String> = ["settings.get", "settings.patch", "settings.save", "agents.set", "sources.configure", "sources.remove", "accounts.connect", "accounts.create", "accounts.status.get", "accounts.deletion.preview", "accounts.cleanup.list"]

    private func start() throws {
        lifecycle.lock(); defer { lifecycle.unlock() }
        guard !stopped else { throw ClientError.message("核心连接已关闭") }
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

    func call<T: Decodable>(_ method: String, params: [String: Any] = [:], as type: T.Type = T.self, onProgress: (@Sendable (String) -> Void)? = nil, onHostSample: (@Sendable (HostResult) -> Void)? = nil) async throws -> T {
        if ["dashboard", "dashboard.summary", "quotas.schedule"].contains(method), let queries {
            return try await queries.call(method, params: params, as: type, onProgress: onProgress)
        }
        if Self.configurationMethods.contains(method), let configuration {
            return try await configuration.call(method, params: params, as: type, onProgress: onProgress)
        }
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
                            if response["method"] as? String == "operations.progress", let progress = response["params"] as? [String: Any] {
                                if progress["kind"] as? String == "hosts.sample", let row = progress["row"], let data = try? JSONSerialization.data(withJSONObject: row), let result = try? JSONDecoder().decode(HostResult.self, from: data) { onHostSample?(result) }
                                else if let stage = progress["stage"] as? String { onProgress?(stage) }
                                continue
                            }
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
    func stop() {
        configuration?.stop(); queries?.stop()
        lifecycle.lock(); defer { lifecycle.unlock() }
        stopped = true; try? input?.close(); if process?.isRunning == true { process?.terminate() }
    }
    deinit { stop() }
}
enum ClientError: LocalizedError { case message(String); var errorDescription: String? { if case .message(let text) = self { return text }; return nil } }
struct Acknowledgement: Decodable { var partial: Bool?, issues: [SyncIssue]?; var id: String?, name: String?, error: String? }
struct QuotaSchedule: Decodable { var nextDueAt: Double? }
struct RefreshStatus { var busy = false; var succeededAt: Date?; var error: String? }
struct ActionFailure: Identifiable { var action: String; var itemID: String? = nil; var name: String, reason: String; var id: String { action + ":" + (itemID ?? name) } }
struct RemovalRequest: Identifiable { var kind: String, itemID: String, title: String, explanation: String, affected: [String]; var id: String { kind + ":" + itemID } }

/// Local catalogue reads must not queue behind scans or remote quota requests.
@MainActor final class PriceCatalog: ObservableObject {
    @Published private(set) var prices: [ModelPrice] = []
    @Published private(set) var filtered: [ModelPrice] = []
    @Published private(set) var loading = false
    @Published private(set) var loaded = false
    @Published private(set) var error: String?
    @Published var query = "" { didSet { if query != oldValue { filter() } } }
    private let fetch: () async throws -> [ModelPrice]
    private let reader: EngineClient
    private var pending: Task<Void, Never>?
    private var revision = 0

    init(fetch: (() async throws -> [ModelPrice])? = nil) {
        let reader = EngineClient(configurationOnly: true)
        self.reader = reader
        self.fetch = fetch ?? { try await reader.call("prices.list") }
    }
    func stop() { pending?.cancel(); reader.stop() }
    func replace(_ values: [ModelPrice]) {
        revision += 1
        if prices != values { prices = values; filter() }
        loaded = true; error = nil
    }
    private func filter() {
        let term = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let next = term.isEmpty ? prices : prices.filter { $0.id.localizedCaseInsensitiveContains(term) || $0.name.localizedCaseInsensitiveContains(term) }
        if filtered != next { filtered = next }
    }
    func load(force: Bool = false) async {
        if force { revision += 1; loaded = false }
        if let pending { await pending.value; return }
        guard !loaded else { return }
        loading = true; error = nil
        let work = Task { @MainActor in
            defer { pending = nil; loading = false }
            while !Task.isCancelled {
                let requested = revision
                do {
                    let next = try await fetch()
                    // A save/sync during an outstanding read requires a fresh snapshot.
                    guard requested == revision else { continue }
                    replace(next); break
                } catch {
                    guard requested == revision else { continue }
                    self.error = error.localizedDescription; break
                }
            }
        }
        pending = work
        await work.value
    }
}


@MainActor final class AppModel: ObservableObject {
    let engine = EngineClient()
    let accountEngine = EngineClient()
    @Published var accountStatuses: [AccountDeviceStatus] = []
    @Published var accountStatusBusy = false
    var accountStatusRevision = 0
    var accountSourceStatusRequests: [String: Int] = [:]
    var accountSourceStatusTasks: [String: Task<Bool, Never>] = [:]
    @Published var agyLoginActive = false
    var antigravityRefreshTasks: [String: Task<Void, Never>] = [:]
    @Published var deploymentSyncMessage: String?
    var deploymentSyncTask: Task<Void, Never>?
    var settingsRefreshTask: Task<Void, Never>?
    let metricsEngine = EngineClient()
    let networkEngine = EngineClient()
    let priceCatalog = PriceCatalog()
    @Published var networkTest: NetworkTest?
    @Published var networkBusy = false { didSet { scheduleRefreshes() } }
    private var lastNetworkAttempt = Date.distantPast
    private var networkRevision = 0
    func loadAccountStatuses(refresh: Bool = false) async {
        guard !accountStatusBusy else { return }
        accountStatusBusy = true; defer { accountStatusBusy = false }
        let revision = accountStatusRevision
        do {
            let rows: [AccountDeviceStatus] = try await engine.call("accounts.status.get")
            if revision == accountStatusRevision { accountStatuses = rows }
        }
        catch { settingsMessage = error.localizedDescription; return }
        guard refresh else { return }
        let accounts = settings.accounts.filter { $0.archived != true }
        for account in accounts {
            for source in settings.sources.filter({ $0.provider == account.provider && $0.enabled }) {
                if let host = source.hostId, !settings.hosts.contains(where: { $0.id == host && $0.enabled }) { continue }
                if Task.isCancelled || installingUpdate { return }
                do {
                    let row: AccountDeviceStatus = try await accountEngine.call("accounts.status.refresh", params: ["accountKey":account.key,"sourceId":source.id])
                    if revision == accountStatusRevision { accountStatuses.removeAll { $0.id == row.id }; accountStatuses.append(row) }
                } catch { /* The per-device result is retained; a concurrent settings edit invalidates it. */ }
            }
        }
        await refreshIdentitySettings()
    }
    func refreshIdentitySettings(reportFailure: Bool = false) async {
        let before = settings
        let saved: Settings
        do { saved = try await accountEngine.call("settings.get") }
        catch { if reportFailure { settingsMessage = "登录已确认，账户设置刷新失败：" + error.localizedDescription }; return }
        guard settings == before else { return }
        let cleanDraft = settingsDraft == before
        settings = saved
        if cleanDraft { settingsDraft = saved }
    }
    func refreshAntigravityAccount(sourceID: String, status: AccountDeviceStatus? = nil) async throws {
        guard let source = settings.sources.first(where: { $0.id == sourceID }),
              let account = settings.accounts.first(where: { $0.provider == "antigravity" && $0.id == source.accountId }) else { throw ClientError.message("账户连接已变化，请重新检查") }
        accountStatusRevision += 1
        let revision = accountStatusRevision
        let row: AccountDeviceStatus
        if let status { row = status }
        else { row = try await accountEngine.call("accounts.status.refresh", params: ["accountKey":account.key,"sourceId":sourceID]) }
        if status == nil, let error = row.error { throw ClientError.message("设备状态更新失败：" + error) }
        guard revision == accountStatusRevision, row.sourceId == sourceID, row.accountKey == account.key, row.machineId == (source.hostId ?? "local"),
              settings.sources.contains(where: { $0.id == sourceID && $0.accountId == account.id && $0.enabled }) else {
            throw ClientError.message("账户连接已变化，请重新检查")
        }
        accountStatuses.removeAll { $0.id == row.id }; accountStatuses.append(row)
        // Owned by the model, so closing the login sheet does not cancel quota/settings reads.
        if antigravityRefreshTasks[sourceID] == nil {
            antigravityRefreshTasks[sourceID] = Task {
                defer { antigravityRefreshTasks[sourceID] = nil }
                await refreshIdentitySettings(reportFailure: true)
                guard !Task.isCancelled else { return }
                await refreshQuotas(accountID: account.id)
            }
        }
    }
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
    private var sessionBusy = false { didSet { scheduleRefreshes() } }
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
    @Published var settings = Settings() { didSet { invalidateQuotaSchedule() } }
    @Published var settingsDraft = Settings()
    @Published var settingsLoaded = false
    @Published var settingsSaving = false { didSet { scheduleRefreshes() } }
    @Published var settingsStage = ""
    @Published var settingsStartedAt: Date?
    @Published private(set) var repricing = false { didSet { scheduleRefreshes() } }
    @Published var settingsMessage: String?
    @Published var mappingModel = ""
    @Published var mappingID = ""
    @Published private(set) var pendingAPIKeys: [String: String] = [:]
    @Published private(set) var pendingHostPasswords: [String: String] = [:]
    private var preparedHostPasswords: [String: (password: String, reference: String)] = [:]
    func stageHostPassword(_ password: String, hostID: String) { if !password.isEmpty { pendingHostPasswords[hostID] = password } }
    private var preparedCredentials: [String: (key: String, path: String)] = [:]
    var settingsChangeCount: Int {
        var rows = Set<String>()
        for source in settingsDraft.sources where settings.sources.first(where: { $0.id == source.id }) != source { rows.insert("source:" + source.id) }
        for source in settings.sources where !settingsDraft.sources.contains(where: { $0.id == source.id }) { rows.insert("source:" + source.id) }
        for host in settingsDraft.hosts where settings.hosts.first(where: { $0.id == host.id }) != host { rows.insert("host:" + host.id) }
        for host in settings.hosts where !settingsDraft.hosts.contains(where: { $0.id == host.id }) { rows.insert("host:" + host.id) }
        for account in settingsDraft.accounts where settings.accounts.first(where: { $0.key == account.key }) != account { rows.insert("account:" + account.key) }
        for account in settings.accounts where !settingsDraft.accounts.contains(where: { $0.key == account.key }) { rows.insert("account:" + account.key) }
        for id in pendingAPIKeys.keys { rows.insert("source:" + id) }; for id in pendingHostPasswords.keys { rows.insert("host:" + id) }
        var general = settingsDraft
        general.sources = settings.sources; general.hosts = settings.hosts; general.accounts = settings.accounts; general.modelMappings = settings.modelMappings
        if general != settings { rows.insert("general") }
        if settingsDraft.modelMappings != settings.modelMappings || !mappingModel.isEmpty || !mappingID.isEmpty { rows.insert("mappings") }
        return rows.count
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
    var prices: [ModelPrice] { get { priceCatalog.prices } set { priceCatalog.replace(newValue) } }
    @Published var provider = "all"
    @Published var selectedSource = "all"
    @Published var selectedAccount = "all"
    @Published var settingsTab = "sources"
    @Published var selectedModel = "all"
    @Published var range = 1
    @Published var busy = false { didSet { scheduleRefreshes() } }
    @Published var serverBusy = false { didSet { scheduleRefreshes() } }
    @Published var message: String?
    @Published var activity = ""
    @Published var isPinned = false
    @Published var installingUpdate = false { didSet { scheduleRefreshes() } }
    @Published private(set) var estimateBusy = Set<String>() { didSet { scheduleRefreshes() } }
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
    @Published private(set) var visibleWindows = Set<String>()
    func isWindowVisible(_ window: String) -> Bool { visibleWindows.contains(window) }
    var panelVisible: Bool { !visibleWindows.isEmpty }
    func setWindowVisible(_ visible: Bool, window: String) {
        guard visibleWindows.contains(window) != visible else { return }
        if visible { visibleWindows.insert(window) } else { visibleWindows.remove(window) }; scheduleRefreshes()
    }
    private var visibleServerWindows = Set<String>()
    var serverTabVisible: Bool { !visibleServerWindows.intersection(visibleWindows).isEmpty }
    func setServerVisible(_ visible: Bool, window: String) {
        if visible { visibleServerWindows.insert(window) } else { visibleServerWindows.remove(window) }; scheduleRefreshes()
    }
    @Published var quotaBusy = false { didSet { scheduleRefreshes() } }
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
    var showEstimate: ((Quota, Bool, String?) -> Void)?
    var showSampling: (() -> Void)?
    var showQuotaOrder: (() -> Void)?
    var showPanelAccounts: (() -> Void)?
    func openEstimate(_ estimate: QuotaEstimate) async {
        do {
            // Use an unfiltered read so changing the usage filter cannot hide sampling controls.
            let snapshot: Dashboard = try await engine.call("dashboard.summary")
            guard let quota = snapshot.quotas.first(where: { $0.id == estimate.accountKey }) else {
                estimateErrors[estimate.accountKey] = "此账户已归档或移除；仍可结束采样并保留有效段。"; return
            }
            showEstimate?(quota, estimate.kind == "credits", estimate.groupId)
        } catch { estimateErrors[estimate.accountKey] = error.localizedDescription }
    }

    init(autostart: Bool = true) {
        guard autostart else { return }
        schedulingEnabled = true
        Task { await bootstrap(); scheduleRefreshes() }
    }
    func bootstrap() async {
        do {
            settings = try await engine.call("settings.get"); PanelAccountPreference.migrateAntigravity(settings.accountAliases ?? [:]); settingsDraft = settings; settingsLoaded = true
            await reload(); await scan(); await refreshQuotas()
        }
        catch { message = error.localizedDescription }
    }
    func openPricing() { settingsTab = "prices"; showSettings?() }
    @Published private(set) var dashboardPending = false
    @Published private(set) var dashboardError: String?
    @Published private(set) var appliedScope = "今日 · 全部 Agent · 全部账户 · 全部来源 · 全部模型"
    private var dashboardRequest = 0
    private var reloadTask: Task<Bool, Never>?
    private var reloadAgain = false
    private var filterRefresh: Task<Void, Never>?
    func requestDashboardReload() {
        filterRefresh?.cancel()
        dashboardRequest += 1
        filterRefresh = Task { do { try await Task.sleep(for: .milliseconds(120)) } catch { return }; filterRefresh = nil; _ = await reload() }
    }
    @discardableResult func reload() async -> Bool {
        guard !installingUpdate, !stopped else { return false }
        dashboardRequest += 1; reloadAgain = true
        if let reloadTask { return await reloadTask.value }
        let work = Task { @MainActor in
            defer { reloadTask = nil }
            var result = true
            repeat { reloadAgain = false; result = await reloadOnce(request: dashboardRequest) } while reloadAgain && !Task.isCancelled
            return result
        }
        reloadTask = work
        return await work.value
    }
    private func reloadOnce(request: Int) async -> Bool {
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
            let unfiltered: Dashboard = unfilteredSelected ? next : try await engine.call("dashboard.summary")
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
    var usageSources: [AgentSource] { settings.sources.filter { source in source.enabled && source.provider != "deepseek" && (source.hostId == nil || settings.hosts.contains { $0.id == source.hostId && $0.enabled }) } }
    var dataTime: Double {
        let stamps = usageSources.map { source in dashboard.sources.first { $0.id == source.id }?.status?.updatedAt ?? 0 }
        return stamps.min() ?? 0
    }
    var statusText: String {
        let delayed = usageSources.filter { source in
            guard let stamp = dashboard.sources.first(where: { $0.id == source.id })?.status?.updatedAt else { return true }
            return Date().timeIntervalSince1970 - stamp > Double(max(60, settings.historyInterval * 2))
        }.count
        let failed = dashboard.sources.contains { $0.enabled && ($0.status?.error != nil || $0.status?.partial == true) }
        let prefix = dataTime > 0 ? "记录同步于 " + Format.time(dataTime) : usageSources.isEmpty ? "未接入用量来源" : "记录尚未全部同步"
        return prefix + " · " + (busy ? "同步中…" : failed || !actionFailures.isEmpty ? "部分失败" : delayed > 0 ? "\(delayed) 个来源有延迟" : usageSources.isEmpty ? "等待接入" : "记录已同步")
    }
    var serverStatusText: String {
        let enabled = settings.monitoredHosts.filter(\.enabled)
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
        var state = refreshStates[key] ?? RefreshStatus(); state.busy = true; refreshStates[key] = state
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
        guard !installingUpdate, !busy, !quotaBusy, estimateBusy.isEmpty else { return }; busy = true; activity = "同步记录"; beginRefresh("scan")
        defer { lastScan = Date(); busy = false; activity = "" }
        do {
            let rows: [Acknowledgement] = try await engine.call("sources.scan", params: sourceID.map { ["sourceId": $0] } ?? [:])
            let failures = rows.compactMap { row -> ActionFailure? in
                guard let error = row.error ?? (row.partial == true ? row.issues?.map { ($0.path.map { $0 + " · " } ?? "") + $0.message }.joined(separator: "；") ?? "部分记录需要处理" : nil) else { return nil }
                return ActionFailure(action: "scan", itemID: row.id, name: row.name ?? settings.sources.first { $0.id == row.id }?.name ?? "数据源", reason: error)
            }
            let loaded = await reload()
            finishRefresh("scan", failures: failures + (loaded ? [] : [ActionFailure(action: "scan", name: "概览", reason: message ?? "读取失败")]), itemID: sourceID)
        } catch { message = error.localizedDescription; finishRefresh("scan", failures: [ActionFailure(action: "scan", itemID: sourceID, name: "同步记录", reason: error.localizedDescription)], itemID: sourceID) }
    }
    func refreshQuotas(accountID: String? = nil, dueOnly: Bool = false) async {
        guard !installingUpdate, !quotaBusy, !busy, estimateBusy.isEmpty, hasQuotaAccounts else { return }
        quotaBusy = true; quotaError = nil; beginRefresh("quotas")
        defer { quotaBusy = false; invalidateQuotaSchedule() }
        do {
            var params: [String: Any] = ["dueOnly": dueOnly]
            if let accountID { params["accountId"] = accountID }
            let result: [Quota] = try await engine.call("quotas.refresh", params: params)
            if dueOnly && result.isEmpty { refreshStates["quotas"]?.busy = false; return }
            if result.contains(where: { q in q.identity.map { id in settings.accounts.contains { $0.id == q.accountId && $0.provider == q.provider && $0.identityKey != id.key } } ?? false }) {
                let before = settings
                if let saved: Settings = try? await engine.call("settings.get"), settings == before { let clean = settingsDraft == before; settings = saved; if clean { settingsDraft = saved } }
            }
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
    private var hostVersions: [String: (String, UInt64)] = [:]
    func acceptHostSamples(_ rows: [HostResult]) {
        var next = Dictionary(uniqueKeysWithValues: hosts.map { ($0.id, $0) })
        for var row in rows {
            if let session = row.sampleSession, let version = row.sampleVersion {
                if let old = hostVersions[row.id], old.0 == session, old.1 >= version { continue }
                hostVersions[row.id] = (session, version)
            }
            if row.error != nil, row.sample == nil { row.sample = next[row.id]?.sample }
            next[row.id] = row
        }
        let ids = Set(settings.monitoredHosts.map(\.id))
        hostVersions = hostVersions.filter { ids.contains($0.key) }
        let ordered = settings.monitoredHosts.compactMap { next[$0.id] }
        if hosts != ordered { hosts = ordered }
    }
    func sampleHosts(hostID: String? = nil) async {
        guard !installingUpdate, !serverBusy, settings.monitoredHosts.contains(where: \.enabled) else { return }; serverBusy = true; beginRefresh("hosts")
        defer { lastMetrics = Date(); serverBusy = false }
        do {
            let batch: HostSampleBatch = try await metricsEngine.call("hosts.sample", params: ["stream": true].merging(hostID.map { ["hostId": $0] } ?? [:]) { _, new in new }, onHostSample: { [weak self] row in Task { @MainActor in self?.acceptHostSamples([row]) } })
            let results = batch.rows
            acceptHostSamples(results)
            if !results.contains(where: { $0.error != nil }), let lastHostError {
                if message == lastHostError { message = nil }; self.lastHostError = nil
            }
            finishRefresh("hosts", failures: results.compactMap { row in row.error.map { ActionFailure(action: "hosts", itemID: row.id, name: row.name, reason: $0) } }, itemID: hostID)
        } catch {
            let failure = error.localizedDescription; lastHostError = failure; message = failure
            hosts = settings.monitoredHosts.map { host in
                let previous = hosts.first { $0.id == host.id }
                return HostResult(id: host.id, name: host.name.isEmpty ? host.target : host.name, sample: previous?.sample, error: host.enabled && (hostID == nil || hostID == host.id) ? failure : previous?.error)
            }
            finishRefresh("hosts", failures: [ActionFailure(action: "hosts", itemID: hostID, name: "服务器", reason: failure)], itemID: hostID)
        }
    }
    func removalRequest(_ kind: String, id: String) -> RemovalRequest? {
        if kind == "host", let host = settingsDraft.hosts.first(where: { $0.id == id }) {
            let linked = settingsDraft.sources.filter { $0.hostId == id }.map(\.name)
            return RemovalRequest(kind: kind, itemID: id, title: "移除服务器「" + (host.name.isEmpty ? host.target : host.name) + "」？", explanation: linked.isEmpty ? "历史用量会保留。确认后立即生效。" : "以下 \(linked.count) 个数据源将暂停，并需要重新选择服务器。确认后立即生效。", affected: linked)
        }
        if kind == "source", let source = settingsDraft.sources.first(where: { $0.id == id }) {
            return RemovalRequest(kind: kind, itemID: id, title: "移除数据源「" + source.name + "」？", explanation: "已导入的历史用量会保留。以此来源查询限额的账户需要重新选择来源；确认后立即生效。", affected: settingsDraft.accounts.filter { $0.quotaSourceId == id }.map(\.name))
        }
        if kind == "account", let account = settingsDraft.accounts.first(where: { $0.key == id }) {
            return RemovalRequest(kind: kind, itemID: id, title: "归档账户「" + account.name + "」？", explanation: "停止展示此账户的实时限额，保留历史用量；确认后立即生效，可从已归档账户恢复。", affected: [])
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
    func saveSettingsDraft(refreshDashboard: Bool = true, includeMappings: Bool = true) async -> Bool {
        guard !settingsSaving, !repricing else { return false }
        settingsSaving = true; settingsMessage = nil; settingsStartedAt = Date(); settingsStage = "准备保存"
        defer { settingsSaving = false; settingsStage = ""; settingsStartedAt = nil }
        var next = settingsDraft
        do {
            let from = mappingModel.trimmingCharacters(in: .whitespacesAndNewlines), to = mappingID.trimmingCharacters(in: .whitespacesAndNewlines)
            if includeMappings && (!from.isEmpty || !to.isEmpty) {
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
            if success { if includeMappings { mappingModel = ""; mappingID = "" }; settingsDraft = settings; pendingAPIKeys.removeAll(); preparedCredentials.removeAll(); pendingHostPasswords.removeAll(); preparedHostPasswords.removeAll() }
            settingsMessage = success ? "已保存配置 · " + Format.time(Date().timeIntervalSince1970) : message ?? "保存失败"
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
    func toggleTheme(currentlyDark: Bool) async {
        guard settingsLoaded, !settingsSaving else { return }
        settingsSaving = true
        defer { settingsSaving = false }
        var next = settings
        var appearance = next.appearance ?? AppearanceSettings()
        appearance.theme = currentlyDark ? "light" : "dark"
        next.appearance = appearance
        guard await save(next, refreshDashboard: false) else { return }
        // Preserve unrelated edits in the settings window.
        var draftAppearance = settingsDraft.appearance ?? AppearanceSettings()
        draftAppearance.theme = settings.appearance?.theme ?? appearance.theme
        settingsDraft.appearance = draftAppearance
    }
    func save(_ draft: Settings, refreshDashboard: Bool = true) async -> Bool {
        do {
            let data = try JSONEncoder().encode(draft)
            let params = try JSONSerialization.jsonObject(with: data) as! [String: Any]
            settingsStage = busy || quotaBusy ? "等待当前刷新完成" : "保存配置"
            let baseline = try JSONSerialization.jsonObject(with: JSONEncoder().encode(settings))
            let saved: Settings = try await engine.call("settings.patch", params: ["base": baseline, "settings": params], onProgress: { [weak self] stage in Task { @MainActor in guard let self, self.settingsSaving else { return }; self.settingsStage = stage } })
            let historyConfigurationChanged = settings.accounts != draft.accounts || settings.sources != draft.sources || settings.hosts != draft.hosts || settings.modelMappings != draft.modelMappings
            let quotaConfigurationChanged = settings.accounts != draft.accounts || settings.sources != draft.sources || settings.hosts != draft.hosts || settings.proxy != draft.proxy
            if settings.proxy != draft.proxy || settings.proxyTestUrls != draft.proxyTestUrls { networkRevision += 1; networkTest = nil; lastNetworkAttempt = .distantPast }
            for account in saved.accounts where !settings.accounts.contains(where: { $0.key == account.key }) { PanelAccountPreference.includeNew(account, accounts: saved.accounts) }
            settings = saved
            if quotaConfigurationChanged { quotaNextAttempt = nil }
            if selectedAccount != "all" && selectedAccount != "none" && !settings.historicalAccounts.contains(where: { $0.key == selectedAccount }) { selectedAccount = "all" }
            if selectedSource != "all" && !settings.sources.contains(where: { $0.id == selectedSource }) { selectedSource = "all" }
            message = "已保存"
            if refreshDashboard { Task {
                if historyConfigurationChanged { await reload() }
                if quotaConfigurationChanged && hasQuotaAccounts { await refreshQuotas(dueOnly: true) }
            } }
            return true
        } catch { message = error.localizedDescription; return false }
    }
    func syncPrices() async {
        guard !busy, !quotaBusy else { return }; busy = true; activity = "同步价格"; beginRefresh("prices"); defer { busy = false; activity = "" }
        do { let _: Acknowledgement = try await engine.call("prices.sync"); await loadPrices(force: true); if let error = priceCatalog.error { throw ClientError.message(error) }; let loaded = await reload(); message = loaded ? "价格已更新" : message
            finishRefresh("prices", failures: loaded ? [] : [ActionFailure(action: "prices", name: "概览", reason: message ?? "读取失败")]) }
        catch { message = error.localizedDescription; finishRefresh("prices", failures: [ActionFailure(action: "prices", name: "价格", reason: error.localizedDescription)]) }
    }
    func loadPrices(force: Bool = false) async { await priceCatalog.load(force: force) }
    func savePrice(_ price: ModelPrice) async -> Bool {
        do {
            let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(price))
            let _: Acknowledgement = try await engine.call("prices.save", params: ["prices":[object]])
            await loadPrices(force: true); if let error = priceCatalog.error { throw ClientError.message("价格已保存，但刷新失败：" + error) }; await reload(); message = "已保存价格 · " + Format.time(Date().timeIntervalSince1970); return true
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
    var updateBlockers: [String] {
        [(reloadTask != nil || filterRefresh != nil, "概览查询"), (quotaScheduleTask != nil, "刷新调度读取"), (busy, activity.isEmpty ? "记录同步 / 价格操作" : activity), (quotaBusy, "Agent 限额查询"), (serverBusy, "服务器采样"), (settingsSaving, "保存配置"), (repricing, "重算价格"), (!estimateBusy.isEmpty, "采样任务"), (networkBusy, "网络检测"), (sessionBusy, "会话状态读取"), (accountStatusBusy, "账户状态检查"), (deploymentSyncTask != nil, "唤醒配置同步"), (!antigravityRefreshTasks.isEmpty, "Antigravity 登录后刷新"), (agyLoginActive, "Antigravity 登录授权"), (UserDefaults.standard.string(forKey: "codexLogin.operation") != nil, "Codex 登录授权（可在账户页继续或取消）")].filter { $0.0 }.map { $0.1 }
    }
    private var resetConfirmations = Set<String>()
    private var schedulingEnabled = false
    private var stopped = false
    private var quotaScheduleKnown = false
    private var quotaScheduleRevision = 0
    private var quotaScheduleTask: Task<Void, Never>?
    private func invalidateQuotaSchedule() {
        quotaScheduleRevision += 1; quotaScheduleKnown = false; scheduleRefreshes()
    }
    private func readQuotaSchedule() {
        guard quotaScheduleTask == nil, !stopped else { return }
        let revision = quotaScheduleRevision
        quotaScheduleTask = Task { @MainActor in
            defer { quotaScheduleTask = nil; scheduleRefreshes() }
            do {
                let schedule: QuotaSchedule = try await engine.call("quotas.schedule")
                guard revision == quotaScheduleRevision, !Task.isCancelled else { return }
                quotaNextAttempt = schedule.nextDueAt.map { max(Date().addingTimeInterval(1), Date(timeIntervalSince1970: $0)) }
                quotaScheduleKnown = true
            } catch {
                if revision == quotaScheduleRevision { quotaNextAttempt = Date().addingTimeInterval(30); quotaScheduleKnown = true }
            }
        }
    }
    private var historyBlocked: Bool { busy || quotaBusy || !estimateBusy.isEmpty }
    private var metricsDeadline: Date { lastMetrics.addingTimeInterval(Double(serverTabVisible ? settings.foregroundInterval : settings.serverRefreshSeconds)) }
    private var pendingResets: [(Quota, Double, String)] {
        dashboard.quotas.flatMap { quota in quota.windows.compactMap { window in
            guard let reset = window.resetsAt else { return nil }
            let key = quota.id + ":" + String(reset)
            return resetConfirmations.contains(key) ? nil : (quota, reset, key)
        } }
    }
    private func scheduleRefreshes() {
        timer?.invalidate(); timer = nil
        guard schedulingEnabled, settingsLoaded, !stopped, !installingUpdate, !settingsSaving, !repricing else { return }
        let now = Date()
        if lastScan > now { lastScan = .distantPast }
        if lastMetrics > now { lastMetrics = .distantPast }
        if lastNetworkAttempt > now { lastNetworkAttempt = .distantPast }
        if lastSessionRead > now { lastSessionRead = .distantPast }
        if !quotaScheduleKnown { readQuotaSchedule() }
        var deadlines: [Date] = []
        if panelVisible, !networkBusy { deadlines.append(lastNetworkAttempt.addingTimeInterval(300)) }
        if !sessionBusy { deadlines.append(lastSessionRead.addingTimeInterval(5)) }
        if !serverBusy, settings.monitoredHosts.contains(where: \.enabled) { deadlines.append(metricsDeadline) }
        if !historyBlocked {
            deadlines.append(lastScan.addingTimeInterval(Double(settings.historyInterval)))
            if quotaScheduleKnown, let quotaNextAttempt { deadlines.append(quotaNextAttempt) }
            deadlines += pendingResets.map { Date(timeIntervalSince1970: $0.1) }
        }
        guard let next = deadlines.min() else { return }
        timer = Timer.scheduledTimer(withTimeInterval: max(0.01, next.timeIntervalSinceNow), repeats: false) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
    }
    func resumeAfterWake() { invalidateQuotaSchedule(); tick() }
    func tick() {
        guard !stopped, !installingUpdate, !settingsSaving, !repricing else { return }
        defer { scheduleRefreshes() }
        let now = Date()
        if panelVisible, !networkBusy, now.timeIntervalSince(lastNetworkAttempt) >= 300 { Task { await testNetwork(force: false) } }
        if !sessionBusy, now.timeIntervalSince(lastSessionRead) >= 5 { Task { await refreshSessions() } }
        if !serverBusy, settings.monitoredHosts.contains(where: \.enabled), now >= metricsDeadline { Task { await sampleHosts() } }
        guard !historyBlocked else { return }
        if let (quota, _, key) = pendingResets.first(where: { $0.1 <= now.timeIntervalSince1970 }) {
            resetConfirmations.insert(key); Task { await refreshQuotas(accountID: quota.accountId) }; return
        }
        if now.timeIntervalSince(lastScan) >= Double(settings.historyInterval) { Task { await scan() }; return }
        if quotaScheduleKnown, let next = quotaNextAttempt, now >= next {
            quotaNextAttempt = nil; Task { await refreshQuotas(dueOnly: true) }
        }
    }
    func stop() {
        stopped = true; schedulingEnabled = false; timer?.invalidate(); timer = nil
        filterRefresh?.cancel(); reloadTask?.cancel(); quotaScheduleTask?.cancel()
        deploymentSyncTask?.cancel(); settingsRefreshTask?.cancel()
        for task in antigravityRefreshTasks.values { task.cancel() }
        priceCatalog.stop(); accountEngine.stop(); engine.stop(); metricsEngine.stop(); networkEngine.stop()
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
