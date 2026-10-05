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

    func call<T: Decodable>(_ method: String, params: [String: Any] = [:], as type: T.Type = T.self) async throws -> T {
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
                            guard let response = try JSONSerialization.jsonObject(with: line) as? [String: Any], response["id"] as? Int == self.nextID else { continue }
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
struct Acknowledgement: Decodable { }

@MainActor final class AppModel: ObservableObject {
    let engine = EngineClient()
    let metricsEngine = EngineClient()
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
    @Published private(set) var fallbackModelOptions: [String] = []
    var modelOptions: [String] { dashboard.modelOptions ?? fallbackModelOptions }
    @Published var settings = Settings()
    @Published var settingsDraft = Settings()
    @Published var settingsLoaded = false
    @Published var settingsSaving = false
    @Published var settingsMessage: String?
    @Published private(set) var pendingAPIKeys: [String: String] = [:]
    private var preparedCredentials: [String: (key: String, path: String)] = [:]
    var settingsDirty: Bool { settingsDraft != settings || !pendingAPIKeys.isEmpty }
    func stageAPIKey(_ key: String?, sourceID: String) {
        let trimmed = key?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if trimmed.isEmpty { pendingAPIKeys.removeValue(forKey: sourceID) }
        else { pendingAPIKeys[sourceID] = trimmed }
        if preparedCredentials[sourceID]?.key != trimmed { preparedCredentials.removeValue(forKey: sourceID) }
    }
    @Published var requestedSourceProvider: String?
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
    var showEstimate: ((Quota) -> Void)?
    var showSampling: (() -> Void)?
    @Published var creditEstimateMode = false
    func openEstimate(_ estimate: QuotaEstimate) async {
        creditEstimateMode = estimate.kind == "credits"
        do {
            // Use an unfiltered read so changing the usage filter cannot hide sampling controls.
            let snapshot: Dashboard = try await engine.call("dashboard", params: ["days": 1])
            guard let quota = snapshot.quotas.first(where: { $0.id == estimate.accountKey }) else {
                estimateErrors[estimate.accountKey] = "此账户已归档或移除；仍可结束采样并保留有效段。"; return
            }
            showEstimate?(quota)
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
    private var dashboardRequest = 0
    func reload() async {
        dashboardRequest += 1; let request = dashboardRequest
        var params: [String: Any] = ["days":range]
        if provider != "all" { params["provider"] = provider }
        if selectedAccount == "none" { params["accountId"] = "" }
        else if let account = settings.accounts.first(where: { $0.key == selectedAccount }) { params["accountId"] = account.id; params["provider"] = account.provider }
        if selectedSource != "all" { params["sourceId"] = selectedSource }
        if selectedModel != "all" { params["model"] = selectedModel }
        do {
            let next: Dashboard = try await engine.call("dashboard", params: params)
            guard request == dashboardRequest else { return }
            dashboard = next
            let names = dashboard.dayModels.map(\.model)
            fallbackModelOptions = Array(Set(selectedModel == "all" ? names : fallbackModelOptions + names)).sorted()
        } catch { if request == dashboardRequest { message = error.localizedDescription } }
    }
    func saveQuotaOrder(_ keys: [String]) async throws {
        let _: [String] = try await engine.call("quotas.order.set", params: ["keys": keys])
        await reload()
    }
    func estimateAction(_ action: String, params: [String: Any], credits: Bool = false) async throws {
        let _: QuotaEstimate = try await engine.call((credits ? "creditEstimates." : "quotaEstimates.") + action, params: params)
        await reload()
    }
    func performEstimate(_ action: String, accountKey: String, params: [String: Any], credits: Bool = false) async -> Bool {
        guard !estimateBusy.contains(accountKey) else { return false }
        estimateBusy.insert(accountKey); estimateErrors[accountKey] = nil
        defer { estimateBusy.remove(accountKey) }
        do { try await estimateAction(action, params: params, credits: credits); return true }
        catch { estimateErrors[accountKey] = error.localizedDescription; await reload(); return false }
    }
    func scan() async {
        guard !busy else { return }; busy = true; activity = "同步记录"; defer { busy = false; activity = ""; lastScan = Date() }
        do { let _: [Acknowledgement] = try await engine.call("sources.scan"); lastScan = Date(); await reload() }
        catch { message = error.localizedDescription }
    }
    func refreshQuotas() async {
        guard !quotaBusy, hasQuotaAccounts else { return }
        quotaBusy = true; quotaError = nil
        // Advance the retry deadline on every attempt, including failed requests.
        quotaNextAttempt = Date().addingTimeInterval(Double(max(30, settings.refreshSeconds)))
        defer { quotaBusy = false }
        do {
            let result: [Quota] = try await engine.call("quotas.refresh")
            let failures = result.compactMap(\.error)
            if !failures.isEmpty { quotaError = failures.joined(separator: " · ") }
            else if result.isEmpty { quotaError = "暂未获得限额数据，请检查关联数据源与账户设置。" }
            await reload()
        } catch { quotaError = error.localizedDescription }
    }
    func sampleHosts() async {
        guard !serverBusy, !settings.hosts.isEmpty else { return }; serverBusy = true; defer { serverBusy = false; lastMetrics = Date() }
        do {
            let results: [HostResult] = try await metricsEngine.call("hosts.sample")
            hosts = results.map { result in
                if result.error != nil, let old = hosts.first(where: { $0.id == result.id }) {
                    return HostResult(id: result.id, name: result.name, sample: result.sample ?? old.sample, error: result.error)
                }
                return result
            }
            if !results.contains(where: { $0.error != nil }), let lastHostError {
                if message == lastHostError { message = nil }
                self.lastHostError = nil
            }
        } catch {
            let failure = error.localizedDescription
            lastHostError = failure
            message = failure
            hosts = settings.hosts.map { host in
                let previous = hosts.first { $0.id == host.id }
                return HostResult(id: host.id, name: host.name.isEmpty ? host.target : host.name, sample: previous?.sample, error: host.enabled ? failure : nil)
            }
        }
    }
    func saveSettingsDraft() async -> Bool {
        guard !settingsSaving else { return false }
        settingsSaving = true; settingsMessage = nil
        defer { settingsSaving = false }
        var next = settingsDraft
        do {
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
            let success = await save(next)
            if success { settingsDraft = settings; pendingAPIKeys.removeAll(); preparedCredentials.removeAll() }
            settingsMessage = success ? "已保存全部配置" : message ?? "保存失败"
            return success
        } catch { settingsMessage = error.localizedDescription; return false }
    }
    func discardSettingsDraft() {
        settingsDraft = settings; settingsMessage = nil
        pendingAPIKeys.removeAll(); preparedCredentials.removeAll()
    }
    func save(_ draft: Settings) async -> Bool {
        do {
            let data = try JSONEncoder().encode(draft)
            let params = try JSONSerialization.jsonObject(with: data) as! [String: Any]
            let _: Acknowledgement = try await engine.call("settings.save", params: params)
            let quotaConfigurationChanged = settings.accounts != draft.accounts || settings.sources != draft.sources || settings.proxy != draft.proxy
            settings = draft
            if quotaConfigurationChanged { quotaNextAttempt = nil }
            if selectedAccount != "all" && selectedAccount != "none" && !settings.accounts.contains(where: { $0.key == selectedAccount }) { selectedAccount = "all" }
            if selectedSource != "all" && !settings.sources.contains(where: { $0.id == selectedSource }) { selectedSource = "all" }
            message = "已保存"; await reload()
            if hasQuotaAccounts && quotaNextAttempt == nil { Task { await refreshQuotas() } }
            return true
        } catch { message = error.localizedDescription; return false }
    }
    func syncPrices() async {
        guard !busy else { return }; busy = true; activity = "同步价格"; defer { busy = false; activity = "" }
        do { let _: Acknowledgement = try await engine.call("prices.sync"); prices = try await engine.call("prices.list"); await reload(); message = "价格已更新" }
        catch { message = error.localizedDescription }
    }
    func loadPrices() async { do { prices = try await engine.call("prices.list") } catch { message = error.localizedDescription } }
    func savePrice(_ price: ModelPrice) async -> Bool {
        do {
            let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(price))
            let _: Acknowledgement = try await engine.call("prices.save", params: ["prices":[object]])
            await loadPrices(); await reload(); return true
        } catch { message = error.localizedDescription; return false }
    }
    func reprice() async {
        do { let _: Acknowledgement = try await engine.call("prices.recalculate"); await reload(); message = "已按当前价格重算" }
        catch { message = error.localizedDescription }
    }
    func tick() {
        guard !installingUpdate else { return }
        if !sessionBusy, Date().timeIntervalSince(lastSessionRead) >= 5 { Task { await refreshSessions() } }
        
        if !serverBusy, !settings.hosts.isEmpty, Date().timeIntervalSince(lastMetrics) > Double(panelVisible && serverTabVisible ? 2 : settings.serverRefreshSeconds) {
            Task { await sampleHosts() }
        }
        if hasQuotaAccounts, !quotaBusy, quotaNextAttempt == nil || Date() >= quotaNextAttempt! { Task { await refreshQuotas() } }
        guard !busy else { return }
        if Date().timeIntervalSince(lastScan) > Double(settings.refreshSeconds) { Task { await scan() }; return }
    }
    var menuText: String {
        switch settings.menuMetric {
        case "tokens": return Format.compact(dashboard.summary.total)
        case "quota": return dashboard.quotas.first?.windows.first.map { String(format: "%.0f%%", max(0, 100 - $0.usedPercent)) } ?? ""
        case "cpu": return hosts.first?.sample?.cpu?.first(where: { $0.id == "cpu" })?.utilization.map { String(format: "%.0f%%", $0) } ?? ""
        default: return ""
        }
    }
}
