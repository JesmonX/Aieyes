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
    @Published var settings = Settings()
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
    var panelVisible = false
    var serverTabVisible = false
    private var timer: Timer?
    private var lastScan = Date.distantPast
    private var lastMetrics = Date.distantPast
    private var lastQuota = Date.distantPast
    var showSettings: (() -> Void)?
    var showDetail: (() -> Void)?

    init(autostart: Bool = true) {
        guard autostart else { return }
        Task { await bootstrap() }
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
    }
    func bootstrap() async {
        do { settings = try await engine.call("settings.get"); await reload(); await scan() }
        catch { message = error.localizedDescription }
    }
    func openPricing() { settingsTab = "prices"; showSettings?() }
    func reload() async {
        var params: [String: Any] = ["days":range]
        if provider != "all" { params["provider"] = provider }
        if selectedAccount == "none" { params["accountId"] = "" }
        else if let account = settings.accounts.first(where: { $0.key == selectedAccount }) { params["accountId"] = account.id; params["provider"] = account.provider }
        if selectedSource != "all" { params["sourceId"] = selectedSource }
        if selectedModel != "all" { params["model"] = selectedModel }
        do { dashboard = try await engine.call("dashboard", params: params) } catch { message = error.localizedDescription }
    }
    func scan() async {
        guard !busy else { return }; busy = true; activity = "同步记录"; defer { busy = false; activity = "" }
        do { let _: [Acknowledgement] = try await engine.call("sources.scan"); lastScan = Date(); await reload() }
        catch { message = error.localizedDescription }
    }
    func refreshQuotas() async {
        guard !busy else { return }; busy = true; activity = "读取限额"; defer { busy = false; activity = "" }
        do { let _: [Quota] = try await engine.call("quotas.refresh"); lastQuota = Date(); await reload() }
        catch { message = error.localizedDescription }
    }
    func sampleHosts() async {
        guard !serverBusy, !settings.hosts.isEmpty else { return }; serverBusy = true; defer { serverBusy = false }
        do {
            let results: [HostResult] = try await metricsEngine.call("hosts.sample")
            hosts = results.map { result in
                if result.error != nil, let old = hosts.first(where: { $0.id == result.id }) {
                    return HostResult(id: result.id, name: result.name, sample: old.sample, error: result.error)
                }
                return result
            }
            lastMetrics = Date()
        } catch { message = error.localizedDescription }
    }
    func save(_ draft: Settings) async -> Bool {
        do {
            let data = try JSONEncoder().encode(draft)
            let params = try JSONSerialization.jsonObject(with: data) as! [String: Any]
            let _: Acknowledgement = try await engine.call("settings.save", params: params)
            settings = draft
            if selectedAccount != "all" && selectedAccount != "none" && !settings.accounts.contains(where: { $0.key == selectedAccount }) { selectedAccount = "all" }
            if selectedSource != "all" && !settings.sources.contains(where: { $0.id == selectedSource }) { selectedSource = "all" }
            message = "已保存"; await reload(); return true
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
        if !sessionBusy, Date().timeIntervalSince(lastSessionRead) >= 5 { Task { await refreshSessions() } }
        
        if !serverBusy, !settings.hosts.isEmpty, Date().timeIntervalSince(lastMetrics) > Double(panelVisible && serverTabVisible ? 2 : settings.serverRefreshSeconds) {
            Task { await sampleHosts() }
        }
        guard !busy else { return }
        if Date().timeIntervalSince(lastScan) > Double(settings.refreshSeconds) { Task { await scan() }; return }
        // Live quota polling starts after the user first requests it.
        if lastQuota != .distantPast, Date().timeIntervalSince(lastQuota) > Double(settings.refreshSeconds) { Task { await refreshQuotas() } }
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
