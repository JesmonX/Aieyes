import Foundation

struct UpdateOperations: Decodable { var preparing: Bool; var protected: [String]; var active: Int }
final class EngineClient: @unchecked Sendable {
    static let runtimeChanged = Notification.Name("AieyesRuntimeChanged")
    static let hostsChanged = Notification.Name("AieyesHostsChanged")
    private struct Pending {
        var complete: (Result<Data, Error>) -> Void
        var progress: (@Sendable (String) -> Void)?
        var sample: (@Sendable (HostResult) -> Void)?
        var quota: (@Sendable (Quota) -> Void)?
        var account: (@Sendable (AccountDeviceStatus) -> Void)?
    }
    private static let registryLock = NSLock()
    private static let clients = NSHashTable<EngineClient>.weakObjects()
    private static var preparingUpdate = false
    private static func setPreparing(_ value: Bool) { registryLock.lock(); preparingUpdate = value; registryLock.unlock() }
    private static func allowedDuringUpdate(_ method: String) -> Bool {
        registryLock.lock(); defer { registryLock.unlock() }
        return !preparingUpdate || method.hasPrefix("operations.") || ["observe", "complete"].contains(CoreContract.updateBehaviors[method] ?? "finish")
    }
    private let queue = DispatchQueue(label: "app.aieyes.engine", qos: .utility)
    private let lifecycle = NSLock()
    private var process: Process?
    private var input: FileHandle?
    private var pending: [Int: Pending] = [:]
    private var nextID = 0
    private var stopped = false
    private let configuration: EngineClient?
    private let queries: EngineClient?
    private let quotas: EngineClient?
    init(configurationOnly: Bool = false) {
        configuration = configurationOnly ? nil : EngineClient(configurationOnly: true)
        queries = configurationOnly ? nil : EngineClient(configurationOnly: true)
        quotas = configurationOnly ? nil : EngineClient(configurationOnly: true)
        Self.registryLock.lock(); Self.clients.add(self); Self.registryLock.unlock()
    }
    private func startLocked() throws {
        guard !stopped else { throw ClientError.message(L("核心连接已关闭")) }
        if process?.isRunning == true { return }
        let executable = ProcessInfo.processInfo.environment["AIEYES_CORE_PATH"]
            ?? Bundle.main.url(forResource: "aieyes-core", withExtension: nil)?.path
            ?? FileManager.default.currentDirectoryPath + "/target/debug/aieyes-core"
        guard FileManager.default.isExecutableFile(atPath: executable) else { throw ClientError.message(L("未找到 Aieyes 核心")) }
        let p = Process(), stdin = Pipe(), stdout = Pipe()
        p.executableURL = URL(fileURLWithPath: executable)
        p.standardInput = stdin; p.standardOutput = stdout; p.standardError = FileHandle.nullDevice
        var environment = ProcessInfo.processInfo.environment
        if environment["AIEYES_DATA_DIR"] == nil, let developmentPath = Bundle.main.object(forInfoDictionaryKey: "AieyesDevelopmentDataDirectory") as? String { environment["AIEYES_DATA_DIR"] = developmentPath }
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        environment["PATH"] = home + "/.local/bin:/opt/homebrew/bin:/opt/local/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:" + (environment["PATH"] ?? "")
        p.environment = environment
        try p.run(); process = p; input = stdin.fileHandleForWriting
        let output = stdout.fileHandleForReading
        DispatchQueue(label: "app.aieyes.engine.read", qos: .utility).async { [weak self] in
            var buffer = Data()
            // This dispatch work item lives for the lifetime of the connection.
            // Foundation's file/JSON temporaries must drain before the next read,
            // rather than waiting for the work item (and core process) to exit.
            while autoreleasepool(invoking: { () -> Bool in
                let data = output.availableData
                guard !data.isEmpty else { return false }
                buffer.append(data)
                while let end = buffer.firstIndex(of: 10) {
                    let line = Data(buffer[..<end]); buffer.removeSubrange(...end)
                    self?.receive(line)
                }
                return true
            }) {}
            self?.disconnected(p)
        }
    }
    private func receive(_ line: Data) {
        guard let response = (try? JSONSerialization.jsonObject(with: line)) as? [String: Any] else { return }
        if response["method"] as? String == "hosts.changed", let value = response["params"],
           let data = try? JSONSerialization.data(withJSONObject: value), let state = try? JSONDecoder().decode(HostMonitorState.self, from: data) {
            NotificationCenter.default.post(name: Self.hostsChanged, object: self, userInfo: ["state":state]); return
        }
        if response["method"] as? String == "runtime.changed", let value = response["params"] as? [String: Any] {
            NotificationCenter.default.post(name: Self.runtimeChanged, object: nil, userInfo: value); return
        }
        if response["method"] as? String == "operations.progress", let value = response["params"] as? [String: Any], value["kind"] as? String == "runtime.changed" {
            NotificationCenter.default.post(name: Self.runtimeChanged, object: nil, userInfo: value); return
        }
        lifecycle.lock()
        if response["method"] as? String == "operations.progress", let value = response["params"] as? [String: Any] {
            let request = (value["requestId"] as? Int).flatMap { pending[$0] } ?? pending.values.first
            lifecycle.unlock()
            if value["kind"] as? String == "hosts.sample", let row = value["row"], let data = try? JSONSerialization.data(withJSONObject: row), let sample = try? JSONDecoder().decode(HostResult.self, from: data) { request?.sample?(sample) }
            else if value["kind"] as? String == "quotas.refresh", let row = value["row"], let data = try? JSONSerialization.data(withJSONObject: row), let sample = try? JSONDecoder().decode(Quota.self, from: data) { request?.quota?(sample) }
            else if value["kind"] as? String == "accounts.status.refresh", let row = value["row"], let data = try? JSONSerialization.data(withJSONObject: row), let sample = try? JSONDecoder().decode(AccountDeviceStatus.self, from: data) { request?.account?(sample) }
            else if let stage = value["stage"] as? String { request?.progress?(stage) }
            return
        }
        let request = (response["id"] as? Int).flatMap { pending.removeValue(forKey: $0) }
        lifecycle.unlock()
        guard let request else { return }
        if let error = response["error"] as? [String: Any] { request.complete(.failure(ClientError.message(error["message"] as? String ?? L("查询失败")))); return }
        do { request.complete(.success(try JSONSerialization.data(withJSONObject: ProviderBoundary.normalize(response["result"] ?? [:]), options: .fragmentsAllowed))) }
        catch { request.complete(.failure(error)) }
    }
    private func disconnected(_ old: Process) {
        lifecycle.lock()
        guard process === old else { lifecycle.unlock(); return }
        process = nil; input = nil
        let requests = Array(pending.values); pending.removeAll(); lifecycle.unlock()
        for request in requests { request.complete(.failure(ClientError.message(L("核心连接已断开")))) }
    }
    private func send<T: Decodable>(_ method: String, params: [String: Any], onProgress: (@Sendable (String) -> Void)? = nil, onHostSample: (@Sendable (HostResult) -> Void)? = nil, onQuotaSample: (@Sendable (Quota) -> Void)? = nil, onAccountSample: (@Sendable (AccountDeviceStatus) -> Void)? = nil) async throws -> T {
        let payload = try JSONSerialization.data(withJSONObject: params)
        let data: Data = try await withCheckedThrowingContinuation { continuation in
            queue.async {
                self.lifecycle.lock()
                var requestID: Int?
                do {
                    guard Self.allowedDuringUpdate(method) else { throw ClientError.message(L("正在准备更新，请稍候")) }
                    try self.startLocked(); self.nextID += 1; let id = self.nextID; requestID = id
                    let parameters = try JSONSerialization.jsonObject(with: payload)
                    let request = try JSONSerialization.data(withJSONObject: ["jsonrpc":"2.0", "id":id, "method":method, "params":parameters])
                    self.pending[id] = Pending(complete: { continuation.resume(with: $0) }, progress: onProgress, sample: onHostSample, quota: onQuotaSample, account: onAccountSample)
                    try self.input?.write(contentsOf: request + Data([10])); self.lifecycle.unlock()
                } catch {
                    if let id = requestID { self.pending.removeValue(forKey: id) }
                    self.lifecycle.unlock(); continuation.resume(throwing: error)
                }
            }
        }
        return try JSONDecoder().decode(T.self, from: data)
    }
    func call<T: Decodable>(_ method: String, params: [String: Any] = [:], as type: T.Type = T.self, onProgress: (@Sendable (String) -> Void)? = nil, onHostSample: (@Sendable (HostResult) -> Void)? = nil, onQuotaSample: (@Sendable (Quota) -> Void)? = nil, onAccountSample: (@Sendable (AccountDeviceStatus) -> Void)? = nil) async throws -> T {
        if CoreContract.methodLanes[method] == "quotas", let quotas { return try await quotas.call(method, params: params, as: type, onProgress: onProgress, onQuotaSample: onQuotaSample) }
        if CoreContract.methodLanes[method] == "dashboard", let queries { return try await queries.call(method, params: params, as: type, onProgress: onProgress) }
        if CoreContract.configurationMethods.contains(method), let configuration { return try await configuration.call(method, params: params, as: type, onProgress: onProgress) }
        return try await send(method, params: params, onProgress: onProgress, onHostSample: onHostSample, onQuotaSample: onQuotaSample, onAccountSample: onAccountSample)
    }
    static func coordinateUpdate(_ method: String) async -> [String] {
        if method == "operations.prepareUpdate" { setPreparing(true) }
        if method == "operations.resume" { setPreparing(false) }
        let active: [EngineClient] = snapshot()
        var blockers: [String] = []
        for client in active {
            do { let state: UpdateOperations = try await client.send(method, params: [:]); blockers += state.protected }
            catch { if snapshot().contains(where: { $0 === client }) { blockers.append(L("核心更新协调失败：") + L(error.localizedDescription)) } }
        }
        return blockers
    }
    private static func snapshot() -> [EngineClient] {
        registryLock.lock(); let all = clients.allObjects; registryLock.unlock()
        return all.filter { client in client.lifecycle.lock(); defer { client.lifecycle.unlock() }; return client.process?.isRunning == true }
    }
    func stop() {
        configuration?.stop(); queries?.stop(); quotas?.stop()
        lifecycle.lock(); stopped = true; try? input?.close(); let old = process; lifecycle.unlock()
        if old?.isRunning == true { old?.terminate() }
    }
    deinit { stop() }
}
enum ClientError: LocalizedError { case message(String); var errorDescription: String? { if case .message(let text) = self { return text }; return nil } }
struct Acknowledgement: Decodable { var partial: Bool?, issues: [SyncIssue]?; var id: String?, name: String?, error: String? }
struct QuotaSchedule: Decodable { var nextDueAt: Double? }
struct RefreshStatus: Equatable { var busy = false; var succeededAt: Date?; var error: String? }
struct ActionFailure: Identifiable, Equatable { var action: String; var itemID: String? = nil; var name: String, reason: String; var dismissed = false; var id: String { action + ":" + (itemID ?? name) } }
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


@MainActor final class AccountStatusProjection: ObservableObject {
    @Published private(set) var rows: [AccountDeviceStatus] = []
    func replace(_ values: [AccountDeviceStatus]) {
        let sorted = values.sorted { $0.id < $1.id }
        if rows != sorted { rows = sorted }
    }
}
@MainActor final class AccountStatusStore {
    private(set) var rows: [AccountDeviceStatus] = []
    private var projections: [String: AccountStatusProjection] = [:]
    private var machineProjections: [String: AccountStatusProjection] = [:]
    private func machineKey(_ row: AccountDeviceStatus) -> String { String(row.accountKey.split(separator: ":", maxSplits: 1).first ?? "") + ":" + row.machineId }
    func machineProjection(_ provider: String, _ machine: String) -> AccountStatusProjection {
        let key = provider + ":" + machine
        if let value = machineProjections[key] { return value }
        let value = AccountStatusProjection()
        value.replace(rows.filter { machineKey($0) == key }); machineProjections[key] = value
        return value
    }
    func projection(_ key: String) -> AccountStatusProjection {
        if let value = projections[key] { return value }
        let value = AccountStatusProjection()
        value.replace(rows.filter { $0.accountKey == key }); projections[key] = value
        return value
    }
    func replace(_ values: [AccountDeviceStatus]) {
        let sorted = values.sorted { $0.id < $1.id }
        guard rows != sorted else { return }
        rows = sorted
        let groups = Dictionary(grouping: rows, by: \.accountKey)
        for (key, value) in projections { value.replace(groups[key] ?? []) }
        let machines = Dictionary(grouping: rows, by: machineKey)
        for (key, value) in machineProjections { value.replace(machines[key] ?? []) }
    }
    func upsert(_ row: AccountDeviceStatus) {
        let priorMachine = rows.first(where: { $0.id == row.id }).map(machineKey)
        if let index = rows.firstIndex(where: { $0.id == row.id }) {
            guard rows[index] != row else { return }; rows[index] = row
        } else { rows.append(row); rows.sort { $0.id < $1.id } }
        projections[row.accountKey]?.replace(rows.filter { $0.accountKey == row.accountKey })
        let machine = machineKey(row)
        for key in Set([machine, priorMachine ?? machine]) {
            machineProjections[key]?.replace(rows.filter { machineKey($0) == key })
        }
    }
}

@MainActor final class AppModel: ObservableObject {
    let resetNotifications = QuotaNotificationService()
    private let automaticEngine = EngineClient(configurationOnly: true)
    private var automaticTask: Task<Void, Never>?
    @Published var automaticSaving = false
    @Published var automaticError: String?
    func configureAutomatic(_ source: AgentSource, enabled: Bool, confirmed: Bool) async {
        guard !automaticSaving else { return }
        automaticSaving = true; automaticError = nil
        defer { automaticSaving = false }
        do {
            struct Saved: Decodable { var settings: Settings }
            let baseline: Any = try source.autoValuation.map { try JSONSerialization.jsonObject(with: JSONEncoder().encode($0)) } ?? NSNull()
            let result: Saved = try await engine.call("autoEstimates.configure", params: ["sourceId":source.id,"enabled":enabled,"confirmed":confirmed,"baseConfig":baseline])
            acceptConfiguration(result.settings); automaticTask?.cancel(); startAutomatic(); await reload()
        } catch { automaticError = error.localizedDescription }
    }
    private func startAutomatic() {
        automaticTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, !self.stopped else { return }
                if self.settingsLoaded && !self.installingUpdate {
                    struct Changed: Decodable { var changed: Bool; var enabled: Bool? }
                    if let value: Changed = try? await self.automaticEngine.call("autoEstimates.tick") { if value.changed { await self.reload() }; if value.enabled == false { do { try await Task.sleep(for: .seconds(55)) } catch { return } } }
                }
                do { try await Task.sleep(nanoseconds: 5_000_000_000) } catch { return }
            }
        }
    }
    let engine = EngineClient()
    let accountEngine = EngineClient()
    let accountStatusStore = AccountStatusStore()
    var accountStatuses: [AccountDeviceStatus] {
        get { accountStatusStore.rows }
        set { accountStatusStore.replace(newValue) }
    }
    @Published var accountStatusBusy = false
    var accountStatusNeedsRefresh = false
    var accountStatusRevision = 0
    var accountSourceStatusRequests: [String: Int] = [:]
    var accountSourceStatusTasks: [String: Task<Bool, Never>] = [:]
    @Published var agyLoginActive = false
    var antigravityRefreshTasks: [String: Task<Void, Never>] = [:]
    @Published var deploymentSyncMessage: String?
    var deploymentSyncTask: Task<Void, Never>?
    var deploymentSyncRequested = false
    var settingsRefreshTask: Task<Void, Never>?
    let metricsEngine = EngineClient()
    let networkEngine = EngineClient()
    let priceCatalog = PriceCatalog()
    @Published var networkTest: NetworkTest?
    @Published var networkBusy = false { didSet { scheduleRefreshes() } }
    private var lastNetworkAttempt = Date.distantPast
    private var networkRevision = 0
    func invalidateAccountStatuses() {
        accountStatusRevision += 1
        for id in accountSourceStatusRequests.keys { accountSourceStatusRequests[id] = accountStatusRevision }
        if accountStatusBusy { accountStatusNeedsRefresh = true }
    }
    func loadAccountStatuses(refresh: Bool = false, provider: String? = nil, dueOnly: Bool = false) async {
        guard !accountStatusBusy else { return }
        accountStatusBusy = true
        defer {
            accountStatusBusy = false
            if accountStatusNeedsRefresh {
                accountStatusNeedsRefresh = false
                Task { await loadAccountStatuses(refresh: true, dueOnly: true) }
            }
        }
        let revision = accountStatusRevision
        do {
            let rows: [AccountDeviceStatus] = try await engine.call("accounts.status.get")
            if revision == accountStatusRevision { accountStatuses = rows }
        }
        catch { settingsMessage = error.localizedDescription; return }
        guard refresh else { return }
        let accounts = settings.accounts.filter { $0.archived != true && (provider == nil || $0.provider == provider) }
        guard !accounts.isEmpty else { return }
        do {
            let rows: [AccountDeviceStatus] = try await accountEngine.call("accounts.status.refreshBatch", params: ["accountKeys": accounts.map(\.key), "dueOnly": dueOnly], onAccountSample: { [weak self] row in Task { @MainActor in guard let self, self.accountStatusRevision == revision else { return }; self.accountStatusStore.upsert(row) } })
            if revision == accountStatusRevision { for row in rows { accountStatusStore.upsert(row) } }
        } catch { if !dueOnly { settingsMessage = L("设备状态检查失败：") + L(error.localizedDescription) } }
        await refreshIdentitySettings()
    }
    func refreshIdentitySettings(reportFailure: Bool = false) async {
        let before = settings
        let saved: Settings
        do { saved = try await accountEngine.call("settings.get") }
        catch { if reportFailure { settingsMessage = L("登录已确认，账户设置刷新失败：") + L(error.localizedDescription) }; return }
        guard settings == before else { return }
        mergeConfiguration(saved)
    }
    func refreshAntigravityAccount(sourceID: String, status: AccountDeviceStatus? = nil) async throws {
        guard let source = settings.sources.first(where: { $0.id == sourceID }),
              let account = settings.accounts.first(where: { $0.provider == "antigravity" && $0.id == source.accountId }) else { throw ClientError.message(L("账户连接已变化，请重新检查")) }
        accountStatusRevision += 1
        let revision = accountStatusRevision
        let row: AccountDeviceStatus
        if let status { row = status }
        else { row = try await accountEngine.call("accounts.status.refresh", params: ["accountKey":account.key,"sourceId":sourceID]) }
        if status == nil, let error = row.error { throw ClientError.message(L("设备状态更新失败：") + error) }
        guard revision == accountStatusRevision, row.sourceId == sourceID, row.accountKey == account.key, row.machineId == (source.hostId ?? "local"),
              settings.sources.contains(where: { $0.id == sourceID && $0.accountId == account.id && $0.enabled }) else {
            throw ClientError.message(L("账户连接已变化，请重新检查"))
        }
        accountStatusStore.upsert(row)
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

    private let summaryEngine = EngineClient(configurationOnly: true)
    private var runtimeObserver: NSObjectProtocol?
    private var runtimeTask: Task<Void, Never>?
    private var runtimeRefresh: Task<Void, Never>?
    private var runtimeRevisions: [String: Int] = [:]
    private var pendingRuntimeDomains = Set<String>()
    private var completedDashboardRevision: [String: Int] = [:]
    private var scopeCache: [String: Dashboard] = [:]
    private var scopeCacheRevision: [String: Int] = [:]
    private var scopeStatusRevisions: [String: [String: Int]] = [:]
    private var menuCache: (usage: [String: Int], status: [String: Int], value: Dashboard)?
    private var usageRevisions: [String: Int] { dashboardRevisions.filter { ["settings", "usage", "prices", "calendarDay", "timeZone"].contains($0.key) } }
    var dashboardCacheNow: () -> Date = Date.init
    private var dashboardRevisions: [String: Int] {
        var values = runtimeRevisions.filter { ["settings","usage","quotas","estimates","prices","sources"].contains($0.key) }
        values["calendarDay"] = Int(Calendar.current.startOfDay(for: dashboardCacheNow()).timeIntervalSince1970)
        values["timeZone"] = TimeZone.current.identifier.hashValue
        return values
    }
    private var selectionLoaded = false
    private var lastAccountCheck = Date.distantPast
    private func startRuntimeMonitoring() {
        runtimeObserver = NotificationCenter.default.addObserver(forName: EngineClient.runtimeChanged, object: nil, queue: .main) { [weak self] note in
            let revisions = note.userInfo?["revisions"] as? [String: Int] ?? [:]
            Task { @MainActor in self?.acceptRuntimeRevisions(revisions) }
        }
        runtimeTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, !self.stopped else { return }
                if let revisions: [String: Int] = try? await self.summaryEngine.call("runtime.revisions") { self.acceptRuntimeRevisions(revisions) }
                if !self.pendingRuntimeDomains.isEmpty || self.completedDashboardRevision != self.dashboardRevisions { self.scheduleRuntimeRefresh() }
                if !self.selectionLoaded { await self.loadSelectedPet() }
                if Date() < self.lastAccountCheck || Date().timeIntervalSince(self.lastAccountCheck) >= 300 {
                    self.lastAccountCheck = Date()
                    Task { await self.loadAccountStatuses(refresh: true, dueOnly: true) }
                }
                do { try await Task.sleep(for: .seconds(5)) } catch { return }
            }
        }
    }
    private func acceptRuntimeRevisions(_ values: [String: Int]) {
        resetNotifications.observe(values)
        // Replies from independent lanes can arrive out of order. Counters only advance.
        let changed = Set(values.keys.filter { values[$0, default: 0] > runtimeRevisions[$0, default: -1] })
        guard !changed.isEmpty else { return }
        for key in changed { runtimeRevisions[key] = values[key] }
        pendingRuntimeDomains.formUnion(changed)
        if !changed.isDisjoint(with: ["settings","usage","prices"]) { scopeCache.removeAll(); scopeStatusRevisions.removeAll() }
        scheduleRuntimeRefresh()
    }
    private func scheduleRuntimeRefresh() {
        guard runtimeRefresh == nil else { return }
        runtimeRefresh = Task { [weak self] in
            do { try await Task.sleep(for: .milliseconds(120)) } catch { return }
            guard let self else { return }
            defer { self.runtimeRefresh = nil }
            guard !self.stopped, !self.installingUpdate else { return }
            let changed = self.pendingRuntimeDomains
            self.pendingRuntimeDomains.removeAll()
            if changed.contains("pets") { await self.loadSelectedPet() }
            if changed.contains("settings") {
                if self.settingsSaving { self.pendingRuntimeDomains.insert("settings") }
                else {
                    let before = self.settings
                    if let saved: Settings = try? await self.summaryEngine.call("settings.get"), self.settings == before {
                        let automaticBefore = before.sources.map { $0.autoValuation?.enabled == true }
                        let deviceChanged = self.accountStatusConfiguration(before) != self.accountStatusConfiguration(saved)
                        self.mergeConfiguration(saved)
                        if deviceChanged { self.invalidateAccountStatuses(); Task { await self.loadAccountStatuses(refresh: true, dueOnly: true) } }
                        self.invalidateQuotaSchedule()
                        if automaticBefore != saved.sources.map({ $0.autoValuation?.enabled == true }) { self.automaticTask?.cancel(); self.startAutomatic() }
                    } else { self.pendingRuntimeDomains.insert("settings") }
                }
            }
            if changed.contains("accounts") { await self.loadAccountStatuses() }
            if changed.contains("notifications") { await self.resetNotifications.drain() }
            if !self.busy, !self.quotaBusy, self.estimateBusy.isEmpty, self.completedDashboardRevision != self.dashboardRevisions { await self.reload() }
        }
    }
    private func loadSelectedPet() async {
        let revision = petSelectionRevision
        do {
            let selection: PetSelection = try await petEngine.call("pets.current")
            guard !stopped, revision == petSelectionRevision, !petImportBusy, petSelectionBusy == nil else { return }
            if let error = selection.error {
                if petImportError != error { petImportError = error }
                if importedPet != nil { selectionLoaded = true; return }
            }
            if let pet = selection.pet { if importedPet != pet { importedPet = pet }; selectionLoaded = true }
        } catch { /* Keep the last decoded resource and retry on the lightweight observer. */ }
    }
    private let sessionEngine = EngineClient(configurationOnly: true)
    private var sessionTask: Task<Void, Never>?
    let live = LiveSessionState()
    lazy var menuStatus = MenuStatusStore(model: self)
    lazy var sessionStripStatus = SessionStripStore(model: self)
    var sessions: [LiveSession] { get { live.sessions } set { if live.sessions != newValue { live.sessions = newValue } } }
    var sessionSources: [SessionSourceHealth] { get { live.sources } set { if live.sources != newValue { live.sources = newValue } } }
    var sessionsUnavailable: Bool { get { live.unavailable } set { if live.unavailable != newValue { live.unavailable = newValue } } }
    var petState: PetState { get { live.pet } set { if live.pet != newValue { live.pet = newValue } } }
    @Published var importedPet: ImportedPet? { didSet { if live.asset != importedPet { live.asset = importedPet } } }
    @Published var petImportError: String?
    @Published var petImportBusy = false
    @Published var petLibrary: [ImportedPet] = []
    @Published var petLibraryWarnings: [PetLibraryWarning] = []
    @Published var petLibraryError: String?
    @Published var petLibraryBusy = false
    @Published var petSelectionBusy: String?
    private let petEngine = EngineClient(configurationOnly: true)
    private var petSelectionRevision = 0
    private var petLibraryRevision = 0
    private var petLibraryTask: Task<Void, Never>?
    func importPet(path: String) async {
        await importPet(input: path)
    }
    func importPet(input: String) async {
        guard !petImportBusy, petSelectionBusy == nil else { return }
        petImportBusy = true; petImportError = nil; petSelectionRevision += 1
        defer { petImportBusy = false }
        do {
            let result: PetSelection = try await petEngine.call("pets.import", params: ["input": input])
            if let error = result.error { throw ClientError.message(error) }
            if let pet = result.pet { importedPet = pet }
            await loadPetLibrary()
        } catch { petImportError = error.localizedDescription }
    }
    func loadPetLibrary() async {
        petLibraryRevision += 1
        if let pending = petLibraryTask { await pending.value; return }
        petLibraryBusy = true
        let work = Task { @MainActor in
            defer { petLibraryBusy = false; petLibraryTask = nil }
            while !Task.isCancelled {
                let revision = petLibraryRevision
                do {
                    let library: PetLibrary = try await petEngine.call("pets.list")
                    guard revision == petLibraryRevision else { continue }
                    petLibrary = library.pets; petLibraryWarnings = library.warnings; petLibraryError = nil
                    return
                } catch {
                    guard revision == petLibraryRevision else { continue }
                    petLibraryError = error.localizedDescription
                    return
                }
            }
        }
        petLibraryTask = work
        await work.value
    }
    func selectPet(_ pet: ImportedPet) async {
        guard !petImportBusy, petSelectionBusy == nil else { return }
        petSelectionBusy = pet.libraryKey; petImportError = nil; petSelectionRevision += 1
        defer { petSelectionBusy = nil }
        do {
            let result: PetSelection = try await petEngine.call("pets.select", params: ["libraryId": pet.libraryKey])
            if let error = result.error { throw ClientError.message(error) }
            if let pet = result.pet { importedPet = pet }
            await loadPetLibrary()
        } catch { petImportError = error.localizedDescription }
    }
    private var sessionBusy = false
    // Live state has its own clock and IPC stream, independent of history, quotas and configuration work.
    private func startSessionMonitoring() {
        guard schedulingEnabled, !stopped, sessionTask == nil else { return }
        sessionTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, !self.stopped else { return }
                let next = ContinuousClock.now.advanced(by: .seconds(1))
                if self.settingsLoaded && !self.installingUpdate { await self.refreshSessions() }
                do { try await Task.sleep(until: next, clock: .continuous) } catch { return }
            }
        }
    }
    var activeSessions: [LiveSession] { sessions.filter { $0.phase.active } }
    private var coreSessionSummary: String? { get { live.summary } set { if live.summary != newValue { live.summary = newValue } } }
    var sessionPhase: SessionPhase? { sessions.first?.phase ?? (sessionsUnavailable ? .unknown : nil) }
    var sessionSummary: String {
        if let summary = coreSessionSummary { return L(summary) }
        if sessionsUnavailable { return L("会话状态不可用") }
        if !settings.sources.contains(where: { settings.sourceEnabled($0) && ["codex", "claude", "antigravity", "kimi", "codebuddy", "deepseek-harness", "grok", "zcode", "qoder", "opencode"].contains($0.provider) }) { return L("未接入会话状态") }
        return activeSessions.isEmpty ? L("无活跃会话") : L("{0} 个活跃会话" , activeSessions.count)
    }
    func manageSessionHooks(sourceID: String, action: String) async throws -> [SessionHookStatus] {
        let rows: [SessionHookStatus] = try await engine.call("sessions.hooks." + action, params: ["sourceId": sourceID])
        for row in rows { if let index = sessionSources.firstIndex(where: { $0.sourceId == row.sourceId }) { sessionSources[index].hooks = row.state } }
        return rows
    }
    let sessionSounds = SessionSoundService()
    func refreshSessions() async {
        guard !sessionBusy, !stopped, !installingUpdate else { return }
        sessionBusy = true
        defer { sessionBusy = false }
        do {
            let result: SessionSnapshot = try await sessionEngine.call("sessions.list")
            guard !stopped, !installingUpdate, !Task.isCancelled else { return }
            sessionSounds.consume(result, preferences: settings.sessionSounds)
            sessions = result.sessions; sessionSources = result.sources ?? []; sessionsUnavailable = result.unavailable; petState = result.pet ?? .unknown; coreSessionSummary = result.summary

        } catch {
            guard !stopped, !installingUpdate, !Task.isCancelled else { return }
            sessionsUnavailable = true; petState = .unknown; coreSessionSummary = nil
        }
    }
    @Published var dashboard = Dashboard()
    @Published private(set) var menuDashboard = Dashboard() { didSet { scheduleRefreshes() } }
    @Published private(set) var panelDashboard = Dashboard()
    @Published private(set) var overviewDashboard = Dashboard()
    @Published var panelFilters = DashboardFilterPreference()
    @Published var overviewFilters = DashboardFilterPreference()
    @Published private(set) var refreshStates: [String: RefreshStatus] = [:]
    @Published private(set) var actionFailures: [ActionFailure] = []
    @Published private(set) var fallbackModelOptions: [String] = []
    var modelOptions: [String] { dashboard.modelOptions ?? fallbackModelOptions }
    @Published var settings = Settings() { didSet {
        AieyesLocale.apply(settings.appearance?.language)
        if !CommandLine.arguments.contains("--render") {
            PanelAccountPreference.migrateAntigravity(settings.accountAliases ?? [:])
            PanelAccountPreference.observeAccounts(settings.accounts, previous: settingsLoaded ? oldValue.accounts : nil, order: dashboard.quotaOrder ?? [], aliases: settings.accountAliases ?? [:])
        }
        invalidateQuotaSchedule()
    } }
    @Published var settingsDraft = Settings()
    @Published var settingsLoaded = false
    @Published var settingsWindowVisible = false
    @Published var settingsSaving = false { didSet { scheduleRefreshes() } }
    private var configurationSaveTask: Task<Bool, Never>?
    private var pendingConfigurationSaves = 0
    private var configurationSaveError: String?
    var generalSaveTask: Task<Void, Never>?
    @Published private(set) var repricing = false { didSet { scheduleRefreshes() } }
    @Published var settingsMessage: String?
    @Published var settingsValidationField: String?
    @Published var mappingModel = ""
    @Published var mappingID = ""
    var settingsDirty: Bool { settingsDraft != settings || !mappingModel.isEmpty || !mappingID.isEmpty }
    @Published var requestedSourceProvider: String?
    @Published var requestedAccountKey: String?
    var requestedAccountMachineID: String?
    @Published var requestedHostID: String?
    @Published var requestedDetailPage: String?
    @Published var detailPage = "agent" { didSet { if detailPage != oldValue { requestDashboardReload() } } }
    var showDetailPage: ((String) -> Void)?
    @Published var requestHostEditor = false
    @Published var hosts: [HostResult] = []
    var prices: [ModelPrice] { get { priceCatalog.prices } set { priceCatalog.replace(newValue) } }
    @Published private(set) var analysisFilters = DashboardFilterPreference() { didSet { persistDashboardFilters() } }
    private func updateAnalysis<Value: Equatable>(_ key: WritableKeyPath<DashboardFilterPreference, Value>, _ value: Value) {
        guard analysisFilters[keyPath: key] != value else { return }
        analysisFilters[keyPath: key] = value
    }
    var provider: String { get { analysisFilters.provider } set { updateAnalysis(\.provider, newValue) } }
    var selectedSource: String { get { analysisFilters.source } set { updateAnalysis(\.source, newValue) } }
    var selectedAccount: String { get { analysisFilters.account } set { updateAnalysis(\.account, "all") } }
    @Published var settingsTab = "integrations"
    @Published var settingsAgent = "codex"
    @Published var generalPage = "appearance"
    @Published var settingsSection: String?
    var selectedModel: String { get { analysisFilters.model } set { updateAnalysis(\.model, newValue) } }
    var range: Int { get { analysisFilters.days } set { updateAnalysis(\.days, newValue) } }
    var dateRange: DashboardDateRange? { get { analysisFilters.dateRange } set { updateAnalysis(\.dateRange, newValue) } }
    var isToday: Bool { dateRange == nil && range == 1 }
    var rangeTitle: String { dateRange.map { $0.kind == "all" ? L("所有") : ($0.start ?? "") + L(" 至 ") + ($0.end ?? "") } ?? (range == 1 ? L("今日") : L("最近 {0} 天" , range)) }
    var rangeChoice: String {
        get { dateRange?.kind ?? String(range) }
        set {
            var next = analysisFilters; next.rangeChoice = newValue
            if next != analysisFilters { analysisFilters = next }
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
    private var filterPersistenceEnabled = false
    private func persistDashboardFilters() {
        guard filterPersistenceEnabled else { return }
        analysisFilters.save(scope: "analysis")
    }
    func filters(for scope: String) -> DashboardFilterPreference { (scope == "panel" ? panelFilters : scope == "overview" ? overviewFilters : analysisFilters).normalized(for: scope) }
    func updateFilters(_ value: DashboardFilterPreference, scope: String, resetDependents: Bool = true) {
        var next = value.normalized(for: scope)
        let previous = filters(for: scope)
        if resetDependents && previous.provider != next.provider { next.account = "all"; next.source = "all"; next.model = "all" }
        else if resetDependents && previous.account != next.account { next.source = "all" }
        guard next != previous else { return }
        switch scope {
        case "panel": panelFilters = next
        case "overview": overviewFilters = next
        default: analysisFilters = next
        }
        if filterPersistenceEnabled && ["panel", "overview"].contains(scope) { next.save(scope: scope) }
        requestDashboardReload()
    }
    func openAnalysis(from scope: String, account: String? = nil, provider: String? = nil) {
        let source = filters(for: scope)
        var next = analysisFilters
        next.provider = provider ?? account.flatMap { $0.split(separator: ":").first.map(String.init) } ?? source.provider; next.account = "all"
        next.source = "all"; next.model = "all"
        updateFilters(next, scope: "analysis", resetDependents: false)
        showDetailPage?("analysis")
    }
    func reconcileDashboardFilters() {
        for scope in ["panel", "overview", "analysis"] {
            var next = filters(for: scope)
            next.reconcile(settings: settings)
            if scope != "analysis", next.provider != "all", !overviewAgentProviders.contains(next.provider) {
                next.provider = "all"; next.account = "all"; next.source = "all"; next.model = "all"
            }
            updateFilters(next, scope: scope, resetDependents: false)
        }
    }
    @Published var busy = false { didSet { scheduleRefreshes() } }
    @Published private(set) var rescanning = false
    @Published private(set) var rescanMessage: String?
    @Published private(set) var rescanFailed = false
    @Published var serverBusy = false { didSet { scheduleRefreshes() } }
    @Published var message: String?
    @Published var activity = ""
    @Published var isPinned = false
    @Published var installingUpdate = false { didSet { live.suspended = installingUpdate; scheduleRefreshes() } }
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
        return [(active > 0 ? L("{0} 项采样中" , active) : nil), (pending > 0 ? L("{0} 项待确认" , pending) : nil)].compactMap { $0 }.joined(separator: " · ")
    }
    @Published var panelWidth: CGFloat = 450
    @Published var panelHeight: CGFloat = 720
    @Published private(set) var visibleWindows = Set<String>()
    func isWindowVisible(_ window: String) -> Bool { visibleWindows.contains(window) }
    var panelVisible: Bool { !visibleWindows.isEmpty }
    func setWindowVisible(_ visible: Bool, window: String) {
        guard visibleWindows.contains(window) != visible else { return }
        if visible { visibleWindows.insert(window); requestDashboardReload() } else { visibleWindows.remove(window) }; scheduleRefreshes()
    }
    private var visibleServerWindows = Set<String>()
    var serverTabVisible: Bool { !visibleServerWindows.intersection(visibleWindows).isEmpty }
    func setServerVisible(_ visible: Bool, window: String) {
        if visible { visibleServerWindows.insert(window) } else { visibleServerWindows.remove(window) }; scheduleRefreshes()
    }
    @Published var quotaBusy = false { didSet { scheduleRefreshes() } }
    @Published var quotaError: String?
    @Published var quotaNextAttempt: Date?
    var quotaAccounts: [AgentAccount] { settings.accounts.filter { $0.quotaEnabled && $0.archived != true && settings.supports($0.provider, "quota") } }
    var hasQuotaAccounts: Bool { !quotaAccounts.isEmpty }
    private var timer: Timer?
    private var lastScan = Date.distantPast
    private var lastMetrics = Date.distantPast
    private var lastHostError: String?
    var showSettings: (() -> Void)?
    var showEstimate: ((Quota, Bool, String?) -> Void)?
    var showAccountSwitch: ((String) -> Void)?
    var showSampling: (() -> Void)?
    var showQuotaOrder: (() -> Void)?
    var showPanelAccounts: (() -> Void)?
    func openSettings(_ destination: SettingsDestination = .init()) {
        settingsTab = destination.normalizedCategory
        if let account = settings.accounts.first(where: { $0.key == destination.accountKey }) { settingsAgent = CoreContract.agentProvider(account.provider) }
        else if let provider = destination.provider { settingsAgent = CoreContract.agentProvider(provider) }
        if settingsTab == "general", let section = destination.section { generalPage = section }
        settingsSection = destination.section
        requestedAccountMachineID = destination.machineID
        requestedAccountKey = destination.accountKey
        requestedHostID = destination.hostID
        requestHostEditor = destination.section == "add-host"
        showSettings?()
    }
    func openEstimate(_ estimate: QuotaEstimate) async {
        await openEstimate(accountKey: estimate.accountKey, credits: estimate.kind == "credits", group: estimate.groupId)
    }
    func openEstimate(accountKey: String, credits: Bool = false, group: String? = nil) async {
        do {
            // Use an unfiltered read so changing the usage filter cannot hide sampling controls.
            let snapshot: Dashboard = try await engine.call("dashboard.summary", params: ["estimateHistory":"display"])
            let account = settings.historicalAccounts.first { $0.key == accountKey }
            let parts = accountKey.split(separator: ":", maxSplits: 1).map(String.init)
            let quota = snapshot.quotas.first(where: { $0.id == accountKey }) ?? Quota(sourceId: "", accountId: account?.id ?? parts.last ?? accountKey, provider: account?.provider ?? parts.first ?? "custom", name: account?.name ?? accountKey, updatedAt: 0, origin: "", windows: [], bankReset: nil, bankUpdatedAt: nil, plan: nil, error: nil)
            var next = dashboard
            next.quotaEstimates = snapshot.quotaEstimates; next.creditEstimates = snapshot.creditEstimates
            next.automaticEstimateSummaries = snapshot.automaticEstimateSummaries
            if !dashboard.hasSameContent(as: next) { dashboard = next }
            estimateErrors[accountKey] = nil
            showEstimate?(quota, credits, group)
        } catch { estimateErrors[accountKey] = error.localizedDescription }
    }

    init(autostart: Bool = true) {
        guard autostart else { return }
        panelFilters = DashboardFilterPreference.read(scope: "panel"); overviewFilters = DashboardFilterPreference.read(scope: "overview")
        let saved = DashboardFilterPreference.read(scope: "analysis")
        analysisFilters = saved
        filterPersistenceEnabled = true
        schedulingEnabled = true
        startAutomatic()
        Task { await bootstrap(); scheduleRefreshes() }
    }
    func bootstrap() async {
        do {
            struct CoreInfo: Decodable { var contractHash: String?; var hostMonitoring: Bool? }
            let info: CoreInfo = try await engine.call("hello")
            if let hash = info.contractHash, hash != CoreContract.contractHash { throw ClientError.message(L("应用资源与核心版本不一致，请重新安装同一版本")) }
            hostMonitoringSupported = info.hostMonitoring == true
            if hostMonitoringSupported {
                hostMonitorObserver = NotificationCenter.default.addObserver(forName: EngineClient.hostsChanged, object: metricsEngine, queue: .main) { [weak self] note in
                    guard let snapshot = note.userInfo?["state"] as? HostMonitorState else { return }
                    Task { @MainActor in self?.acceptHostMonitor(snapshot) }
                }
            }
            settings = try await engine.call("settings.get"); PanelAccountPreference.migrateAntigravity(settings.accountAliases ?? [:]); settingsDraft = settings; settingsLoaded = true
            resetNotifications.start()
            startSessionMonitoring()
            startRuntimeMonitoring()
            synchronizeAccountDeployments()
            reconcileDashboardFilters()
            Task { await loadAccountStatuses(refresh: true, dueOnly: true) }
            await reload(); await scan(); await refreshQuotas()
        }
        catch { message = error.localizedDescription }
    }
    func openPricing() { openSettings(.init(category: "prices")) }
    @Published private(set) var dashboardPending = false
    @Published private(set) var dashboardError: String?
    @Published private(set) var appliedScope = L("今日 · 全部 Agent · 全部来源 · 全部模型")
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
            // Superseded/cancelled requests may exit before reloadOnce's apply step.
            // A finished worker must never leave an orphaned loading indicator.
            defer { reloadTask = nil; if dashboardPending { dashboardPending = false } }
            var result = true
            repeat { reloadAgain = false; result = await reloadOnce(request: dashboardRequest) } while reloadAgain && !Task.isCancelled
            return result
        }
        reloadTask = work
        return await work.value
    }
    private func reloadOnce(request: Int) async -> Bool {
        if !dashboardPending { dashboardPending = true }
        if dashboardError != nil { dashboardError = nil }
        if let current: [String: Int] = try? await summaryEngine.call("runtime.revisions") { acceptRuntimeRevisions(current) }
        let revision = dashboardRevisions, usage = usageRevisions
        if scopeCacheRevision != usage { scopeCache.removeAll(); scopeStatusRevisions.removeAll(); scopeCacheRevision = usage }
        var statusRequests: [String: Task<DashboardStatus, Error>] = [:]
        func status(_ params: [String: Any]) async throws -> DashboardStatus {
            let scope = params.filter { ["provider", "accountId", "sourceId"].contains($0.key) }
            let key = String(data: try JSONSerialization.data(withJSONObject: scope, options: .sortedKeys), encoding: .utf8)!
            if let request = statusRequests[key] { return try await request.value }
            let request = Task { @MainActor in try await self.summaryEngine.call("dashboard.status", params: scope, as: DashboardStatus.self) }
            statusRequests[key] = request
            return try await request.value
        }
        let summary = Task<Dashboard?, Never> { @MainActor [weak self] in
            guard let self else { return nil }
            do {
                var menu: Dashboard
                if !self.runtimeRevisions.isEmpty, let cached = self.menuCache, cached.usage == usage {
                    menu = cached.value
                    if cached.status != revision { menu.applyStatus(try await status([:])) }
                } else { menu = try await self.summaryEngine.call("dashboard.summary", params: ["estimateHistory":"display"]) }
                if revision == self.dashboardRevisions { self.menuCache = (usage, revision, menu) }
                return menu
            } catch { if request == self.dashboardRequest { self.dashboardError = L("菜单摘要读取失败：") + L(error.localizedDescription) }; return nil }
        }
        let allSurfaces = !schedulingEnabled
        var requested: [(String, DashboardFilterPreference)] = []
        if allSurfaces || isWindowVisible("detail") && detailPage == "analysis" { requested.append(("analysis", analysisFilters)) }
        if allSurfaces || isWindowVisible("panel") { requested.append(("panel", filters(for: "panel"))) }
        if allSurfaces || isWindowVisible("detail") && detailPage == "agent" { requested.append(("overview", filters(for: "overview"))) }
        var results: [String: Dashboard] = [:]
        var queried: [String: Dashboard] = [:]
        do {
            for (surface, filter) in requested {
                var params = filter.parameters(settings: settings); params["estimateHistory"] = "display"
                let key = String(data: try JSONSerialization.data(withJSONObject: params, options: .sortedKeys), encoding: .utf8)!
                var next: Dashboard
                var cacheChanged = false
                if let shared = queried[key] { next = shared }
                else if !runtimeRevisions.isEmpty, let cached = scopeCache[key] {
                    next = cached
                    if scopeStatusRevisions[key] != revision { next.applyStatus(try await status(params)); cacheChanged = true }
                } else { next = try await engine.call("dashboard", params: params); cacheChanged = true }
                if cacheChanged {
                    let candidate = next
                    let bytes = await Task.detached(priority: .utility) { (try? JSONEncoder().encode(candidate).count) ?? Int.max }.value
                    if request == dashboardRequest, revision == dashboardRevisions, bytes <= 2 * 1024 * 1024 {
                        if scopeCache.count >= 4 && scopeCache[key] == nil { scopeCache.removeAll(); scopeStatusRevisions.removeAll() }
                        scopeCache[key] = next; scopeStatusRevisions[key] = revision
                    }
                }
                guard request == dashboardRequest else { _ = await summary.value; return true }
                queried[key] = next; results[surface] = next
            }
            let menu = await summary.value
            guard request == dashboardRequest else { return true }
            if let menu {
                if !menuDashboard.hasSameContent(as: menu) { menuDashboard = menu }
                if results["analysis"] == nil {
                    var next = dashboard
                    next.quotaEstimates = menu.quotaEstimates; next.creditEstimates = menu.creditEstimates
                    next.automaticEstimateSummaries = menu.automaticEstimateSummaries
                    if !dashboard.hasSameContent(as: next) { dashboard = next }
                }
            }
            for (surface, value) in results {
                switch surface {
                case "panel": if !panelDashboard.hasSameContent(as: value) { panelDashboard = value }
                case "overview": if !overviewDashboard.hasSameContent(as: value) { overviewDashboard = value }
                default: if !dashboard.hasSameContent(as: value) { dashboard = value }
                }
            }
            if let value = results["analysis"] {
                let scope = [rangeTitle, provider == "all" ? L("全部 Agent") : Format.provider(provider), settings.sources.first { $0.id == selectedSource }?.name ?? L("全部来源"), selectedModel == "all" ? L("全部模型") : selectedModel].joined(separator: " · ")
                if appliedScope != scope { appliedScope = scope }
                let names = value.dayModels.map(\.model)
                let options = Array(Set(selectedModel == "all" ? names : fallbackModelOptions + names)).sorted()
                if fallbackModelOptions != options { fallbackModelOptions = options }
            }
            Palette.registerModels(Array(results.values).flatMap { ($0.modelOptions ?? []) + $0.models.map(\.key) + $0.dayModels.map(\.model) })
            completedDashboardRevision = revision; if dashboardPending { dashboardPending = false }; return menu != nil
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
        estimateStartedAt[accountKey] = Date(); estimateStages[accountKey] = busy || quotaBusy ? L("等待当前刷新") : L("准备同步")
        defer { estimateBusy.remove(accountKey); estimateStages.removeValue(forKey: accountKey); estimateStartedAt.removeValue(forKey: accountKey) }
        do { try await estimateAction(action, params: params, credits: credits); return true }
        catch { estimateErrors[accountKey] = error.localizedDescription; await reload(); return false }
    }
    static var refreshLabels: [String: String] { ["scan": L("同步记录"), "rescan": L("全量重扫"), "quotas": L("刷新额度"), "prices": L("同步价格"), "hosts": L("刷新服务器")] }
    var usageSources: [AgentSource] { settings.usageSources }
    var dataTime: Double {
        let stamps = usageSources.map { source in dashboard.sources.first { $0.id == source.id }?.status?.updatedAt ?? 0 }
        return stamps.min() ?? 0
    }
    var delayedUsageCount: Int {
        usageSources.filter { source in
            guard let stamp = dashboard.sources.first(where: { $0.id == source.id })?.status?.updatedAt else { return true }
            return Date().timeIntervalSince1970 - stamp > Double(max(60, settings.historyInterval * 2))
        }.count
    }
    var hasStaleUsage: Bool { delayedUsageCount > 0 || dataTime == 0 }
    var statusText: String {
        let delayed = delayedUsageCount
        let failed = usageSources.contains { source in
            let status = dashboard.sources.first { $0.id == source.id }?.status
            return status?.failureReason != nil
        }
        let prefix = dataTime > 0 ? L("记录同步于 {0}", Format.time(dataTime)) : usageSources.isEmpty ? L("未接入用量来源") : L("记录尚未全部同步")
        // The normal state is the time alone; a second part appears only for work or problems.
        let detail = busy ? L("同步中…") : failed || !actionFailures.isEmpty ? L("部分失败") : delayed > 0 ? L("{0} 个来源有延迟" , delayed) : ""
        return detail.isEmpty ? prefix : prefix + " · " + detail
    }
    var updatedHostCount: Int {
        settings.monitoredHosts.filter(\.enabled).filter { host in
            guard let result = hosts.first(where: { $0.id == host.id }), result.error == nil, let sample = result.sample else { return false }
            return sample.errors.isEmpty && Date().timeIntervalSince1970 - sample.timestamp <= 10
        }.count
    }
    var hasStaleHosts: Bool { updatedHostCount < settings.monitoredHosts.filter(\.enabled).count }
    var serverStatusText: String {
        let enabled = settings.monitoredHosts.filter(\.enabled)
        let updated = updatedHostCount
        return L("{0}/{1} 台已更新" , updated, enabled.count) + (serverBusy ? L(" · 采样中…") : updated < enabled.count ? L(" · 存在延迟或失败") : "")
    }
    func refreshLabel(_ key: String) -> String {
        let state = refreshStates[key] ?? RefreshStatus()
        return (Self.refreshLabels[key] ?? key) + " · " + (state.busy ? L("进行中…") : state.error != nil ? L("失败，可重试") : state.succeededAt.map { Format.time($0.timeIntervalSince1970) } ?? L("尚未刷新"))
    }
    private func beginRefresh(_ key: String) {
        var state = refreshStates[key] ?? RefreshStatus(); state.busy = true; refreshStates[key] = state
    }
    private func finishRefresh(_ key: String, failures: [ActionFailure], itemID: String? = nil) {
        var state = refreshStates[key] ?? RefreshStatus(); state.busy = false; state.error = failures.first?.reason
        if failures.isEmpty { state.succeededAt = Date() }
        var nextFailures = actionFailures
        nextFailures.removeAll { $0.action == key && (itemID == nil || $0.itemID == itemID) }; nextFailures += failures
        if actionFailures != nextFailures { actionFailures = nextFailures }
        state.error = nextFailures.first { $0.action == key }?.reason
        if refreshStates[key] != state { refreshStates[key] = state }
    }
    func retry(_ failure: ActionFailure) async {
        switch failure.action {
        case "scan": await scan(sourceID: failure.itemID)
        case "rescan": await rescan(sourceID: failure.itemID)
        case "quotas": await refreshQuotas(accountID: failure.itemID)
        case "hosts": await sampleHosts(hostID: failure.itemID)
        case "prices": await syncPrices()
        default: await reload()
        }
    }
    func dismissFailure(_ failure: ActionFailure) {
        guard let index = actionFailures.firstIndex(where: { $0.id == failure.id }) else { return }
        actionFailures[index].dismissed = true
        if message == failure.reason { message = nil }
    }
    func scan(sourceID: String? = nil) async {
        await collectHistory(sourceID: sourceID, full: false)
    }
    func rescan(sourceID: String? = nil) async {
        await collectHistory(sourceID: sourceID, full: true)
    }
    private func collectHistory(sourceID: String?, full: Bool) async {
        guard !installingUpdate, !busy, estimateBusy.isEmpty, !full || !settingsSaving else { return }
        let key = full ? "rescan" : "scan", label = full ? L("全量重扫") : L("同步记录")
        busy = true; activity = label; beginRefresh(key)
        if full { rescanning = true; rescanMessage = nil; rescanFailed = false }
        defer { lastScan = Date(); busy = false; activity = ""; if full { rescanning = false } }
        do {
            let rows: [Acknowledgement] = try await engine.call(full ? "sources.rescan" : "sources.scan", params: sourceID.map { ["sourceId": $0] } ?? [:])
            let failures = rows.compactMap { row -> ActionFailure? in
                guard let error = syncFailureReason(error: row.error, partial: row.partial, issues: row.issues) else { return nil }
                return ActionFailure(action: key, itemID: row.id, name: row.name ?? settings.sources.first { $0.id == row.id }?.name ?? L("数据源"), reason: error)
            }
            let loaded = await reload()
            let allFailures = failures + (loaded ? [] : [ActionFailure(action: key, name: L("概览"), reason: message ?? L("读取失败"))])
            finishRefresh(key, failures: allFailures, itemID: sourceID)
            if full {
                rescanFailed = !allFailures.isEmpty
                let unknown = rows.reduce(0) { $0 + ($1.issues?.filter { $0.code == "unknownModel" }.count ?? 0) }
                let warning = unknown > 0 ? L(" {0} 条调用的模型身份待确认，Token 已保留、暂不计价。" , unknown) : ""
                rescanMessage = allFailures.isEmpty ? (rows.isEmpty ? L("没有可重扫的已启用数据源。") : L("全量重扫完成，已更新历史统计（{0} 个数据源）。" , rows.count) + warning) : allFailures.map { $0.name + " · " + $0.reason }.joined(separator: "；")
            }
        } catch {
            message = error.localizedDescription
            finishRefresh(key, failures: [ActionFailure(action: key, itemID: sourceID, name: label, reason: error.localizedDescription)], itemID: sourceID)
            if full { rescanMessage = error.localizedDescription; rescanFailed = true }
        }
    }
    func refreshQuotas(accountID: String? = nil, dueOnly: Bool = false) async {
        guard !installingUpdate, !quotaBusy, hasQuotaAccounts else { return }
        quotaBusy = true; quotaError = nil; beginRefresh("quotas")
        defer { quotaBusy = false; invalidateQuotaSchedule() }
        do {
            var params: [String: Any] = ["dueOnly": dueOnly]
            if let accountID { params["accountId"] = accountID }
            let result: [Quota] = try await engine.call("quotas.refresh", params: params, onQuotaSample: { [weak self] row in Task { @MainActor in self?.acceptQuotaSamples([row]) } })
            acceptQuotaSamples(result)
            if dueOnly && result.isEmpty { refreshStates["quotas"]?.busy = false; return }
            if result.contains(where: { q in q.identity.map { id in settings.accounts.contains { $0.id == q.accountId && $0.provider == q.provider && $0.identityKey != id.key } } ?? false }) {
                let before = settings
                if let saved: Settings = try? await engine.call("settings.get"), settings == before { mergeConfiguration(saved) }
            }
            var failures = result.compactMap { row -> ActionFailure? in
                guard let error = row.error else { return nil }
                return ActionFailure(action: "quotas", itemID: row.accountId, name: row.name, reason: error)
            }
            if result.isEmpty { failures = [ActionFailure(action: "quotas", name: L("账户额度"), reason: L("暂未获得额度数据，请检查关联数据源与账户设置。"))] }
            quotaError = failures.isEmpty ? nil : L("部分账户读取失败，请逐项重试。")
            let loaded = await reload()
            finishRefresh("quotas", failures: failures + (loaded ? [] : [ActionFailure(action: "quotas", name: L("概览"), reason: message ?? L("读取失败"))]), itemID: accountID)
        } catch { quotaError = error.localizedDescription; finishRefresh("quotas", failures: [ActionFailure(action: "quotas", itemID: accountID, name: L("账户额度"), reason: error.localizedDescription)], itemID: accountID) }
    }
    func acceptQuotaSamples(_ rows: [Quota]) {
        func merge(_ old: [Quota]) -> [Quota] {
            var result = old
            for var row in rows {
                guard settings.accounts.contains(where: { $0.key == row.id && $0.archived != true }) else { continue }
                if let index = result.firstIndex(where: { $0.id == row.id }) {
                    if let error = row.error { row = result[index]; row.error = error }
                    else if result[index].updatedAt > row.updatedAt { continue }
                    if result[index] != row { result[index] = row }
                } else { result.append(row) }
            }
            return result
        }
        let main = merge(dashboard.quotas), menu = merge(menuDashboard.quotas), panel = merge(panelDashboard.quotas)
        if dashboard.quotas != main { dashboard.quotas = main }
        if menuDashboard.quotas != menu { menuDashboard.quotas = menu }
        if panelDashboard.quotas != panel { panelDashboard.quotas = panel }
        let overview = merge(overviewDashboard.quotas)
        if overviewDashboard.quotas != overview { overviewDashboard.quotas = overview }
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
    private var hostMonitoringSupported = false
    private var hostMonitorObserver: NSObjectProtocol?
    private var hostMonitorTask: Task<Void, Never>?
    private var hostMonitorConfiguration = ""
    private var hostMonitorRetryAt = Date.distantPast
    private var hostMonitorVersion: (String, UInt64)?
    @Published private var hostMonitorBusy = Set<String>()
    @Published private var hostMonitorRequestBusy = false
    func hostRefreshBusy(_ id: String? = nil) -> Bool {
        guard hostMonitoringSupported else { return serverBusy }
        return id.map { hostMonitorBusy.contains($0) } ?? hostMonitorRequestBusy
    }
    private func acceptHostMonitor(_ snapshot: HostMonitorState) {
        guard !stopped else { return }
        if let known = hostMonitorVersion, known.0 == snapshot.sampleSession, known.1 >= snapshot.version { return }
        hostMonitorVersion = (snapshot.sampleSession, snapshot.version)
        acceptHostSamples(snapshot.rows)
        let busy = Set(snapshot.inFlight + snapshot.pending)
        if hostMonitorBusy != busy { hostMonitorBusy = busy }
        if serverBusy != !busy.isEmpty { serverBusy = !busy.isEmpty }
        if busy.isEmpty {
            finishRefresh("hosts", failures: snapshot.rows.compactMap { row in row.error.map { ActionFailure(action: "hosts", itemID: row.id, name: row.name, reason: $0) } })
        } else if refreshStates["hosts"]?.busy != true { beginRefresh("hosts") }
    }
    private func configureHostMonitor() {
        guard hostMonitoringSupported, settingsLoaded, !stopped, hostMonitorTask == nil, Date() >= hostMonitorRetryAt else { return }
        let interval = serverTabVisible ? settings.foregroundInterval : settings.serverRefreshSeconds
        let paused = installingUpdate
        let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
        let key = (try? String(data: encoder.encode(settings.monitoredHosts), encoding: .utf8)) ?? ""
        let configuration = key + ":\(interval):\(paused)"
        guard configuration != hostMonitorConfiguration else { return }
        hostMonitorTask = Task { @MainActor in
            defer { hostMonitorTask = nil; scheduleRefreshes() }
            do {
                let snapshot: HostMonitorState = try await metricsEngine.call("hosts.monitor.configure", params: ["intervalSeconds":interval,"paused":paused])
                hostMonitorConfiguration = configuration; hostMonitorRetryAt = .distantPast
                acceptHostMonitor(snapshot)
            } catch { hostMonitorRetryAt = Date().addingTimeInterval(5); message = error.localizedDescription }
        }
    }
    func sampleHosts(hostID: String? = nil) async {
        if hostMonitoringSupported {
            guard !installingUpdate, !hostMonitorRequestBusy else { return }
            hostMonitorRequestBusy = true; defer { hostMonitorRequestBusy = false }
            hostMonitorRetryAt = .distantPast; configureHostMonitor()
            if let task = hostMonitorTask { await task.value }
            do {
                let snapshot: HostMonitorState = try await metricsEngine.call("hosts.monitor.refresh", params: hostID.map { ["hostId":$0] } ?? [:])
                acceptHostMonitor(snapshot)
            } catch { finishRefresh("hosts", failures: [ActionFailure(action: "hosts", itemID: hostID, name: L("服务器"), reason: error.localizedDescription)]) }
            return
        }
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
            finishRefresh("hosts", failures: [ActionFailure(action: "hosts", itemID: hostID, name: L("服务器"), reason: failure)], itemID: hostID)
        }
    }
    func removalRequest(_ kind: String, id: String) -> RemovalRequest? {
        if kind == "host", let host = settings.hosts.first(where: { $0.id == id }) {
            let linked = settings.sources.filter { $0.hostId == id }.map(\.name)
            return RemovalRequest(kind: kind, itemID: id, title: L("移除服务器「") + (host.name.isEmpty ? host.target : host.name) + "」？", explanation: linked.isEmpty ? L("历史用量会保留。确认后立即生效。") : L("以下 {0} 个数据源将暂停，并需要重新选择服务器。确认后立即生效。" , linked.count), affected: linked)
        }
        if kind == "source", let source = settings.sources.first(where: { $0.id == id }) {
            return RemovalRequest(kind: kind, itemID: id, title: L("移除数据源「") + source.name + "」？", explanation: L("已导入的历史用量会保留。以此来源查询额度的账户需要重新选择来源；确认后立即生效。"), affected: settings.accounts.filter { $0.quotaSourceId == id }.map(\.name))
        }
        if kind == "account", let account = settings.accounts.first(where: { $0.key == id }) {
            return RemovalRequest(kind: kind, itemID: id, title: L("归档账户「") + account.name + "」？", explanation: L("停止展示此账户的实时额度，保留历史用量；确认后立即生效，可从已归档账户恢复。"), affected: [])
        }
        return nil
    }
    func saveSettingsDraft(refreshDashboard: Bool = true, includeMappings: Bool = true, base: Settings? = nil) async -> Bool {
        guard !repricing else { return false }
        settingsMessage = nil
        var next = settingsDraft
        let enteredMapping = (mappingModel, mappingID)
        do {
            let from = mappingModel.trimmingCharacters(in: .whitespacesAndNewlines), to = mappingID.trimmingCharacters(in: .whitespacesAndNewlines)
            if includeMappings && (!from.isEmpty || !to.isEmpty) {
                guard !from.isEmpty && !to.isEmpty else { throw ClientError.message(L("请补全模型映射两端，输入已保留")) }
                guard next.modelMappings[from] == nil || next.modelMappings[from] == to else { throw ClientError.message(L("该模型已有映射，请在已有映射中编辑")) }
                next.modelMappings[from] = to
            }
            let success = await save(next, base: base, refreshDashboard: refreshDashboard)
            if success && includeMappings && mappingModel == enteredMapping.0 && mappingID == enteredMapping.1 { mappingModel = ""; mappingID = "" }
            settingsMessage = success ? L("已保存配置 · ") + Format.time(Date().timeIntervalSince1970) : configurationSaveError ?? message ?? L("保存失败")
            return success
        } catch { settingsMessage = error.localizedDescription; return false }
    }
    func discardSettingsDraft() {
        settingsDraft = settings; settingsMessage = nil; settingsValidationField = nil; mappingModel = ""; mappingID = ""
    }
    func setLanguage(_ language: String) async {
        guard settingsLoaded else { return }
        let draftLanguage = settingsDraft.appearance?.language ?? "zh-CN"
        var next = settings
        var appearance = next.appearance ?? AppearanceSettings()
        appearance.language = AieyesLocale.normalize(language)
        next.appearance = appearance
        guard await save(next, refreshDashboard: false) else { return }
        var draft = settingsDraft.appearance ?? AppearanceSettings()
        if draft.language == draftLanguage {
            draft.language = appearance.language
            settingsDraft.appearance = draft
        }
    }
    func setTheme(_ theme: String) async {
        guard settingsLoaded, ["system", "light", "dark"].contains(theme), (settings.appearance?.theme ?? "system") != theme else { return }
        let draftTheme = settingsDraft.appearance?.theme ?? "system"
        var next = settings
        var appearance = next.appearance ?? AppearanceSettings()
        appearance.theme = theme
        next.appearance = appearance
        guard await save(next, refreshDashboard: false) else { return }
        // Preserve unrelated edits in the settings window.
        var draftAppearance = settingsDraft.appearance ?? AppearanceSettings()
        guard draftAppearance.theme == draftTheme else { return }
        draftAppearance.theme = settings.appearance?.theme ?? appearance.theme
        settingsDraft.appearance = draftAppearance
    }
    func save(_ draft: Settings, base: Settings? = nil, refreshDashboard: Bool = true) async -> Bool {
        let previous = configurationSaveTask, baseline = base ?? settings
        let draftAtSubmission = settingsDraft
        pendingConfigurationSaves += 1; settingsSaving = true
        let work = Task { @MainActor in
            _ = await previous?.value
            let result = await performConfigurationSave(draft, base: baseline, draftAtSubmission: draftAtSubmission, refreshDashboard: refreshDashboard)
            pendingConfigurationSaves -= 1; settingsSaving = pendingConfigurationSaves > 0
            if pendingConfigurationSaves == 0 { configurationSaveTask = nil }
            return result
        }
        configurationSaveTask = work
        return await work.value
    }
    private func performConfigurationSave(_ draft: Settings, base: Settings, draftAtSubmission: Settings, refreshDashboard: Bool) async -> Bool {
        do {
            let data = try JSONEncoder().encode(draft)
            let params = try JSONSerialization.jsonObject(with: data) as! [String: Any]
            let baseline = try JSONSerialization.jsonObject(with: JSONEncoder().encode(base))
            let saved: Settings = try await engine.call("settings.patch", params: ["base": baseline, "settings": params])
            let historyConfigurationChanged = settings.preciseCounting != saved.preciseCounting || settings.historyShowDeletedAccounts != saved.historyShowDeletedAccounts || settings.accounts != saved.accounts || settings.sources != saved.sources || settings.hosts != saved.hosts || settings.agents != saved.agents || settings.modelMappings != saved.modelMappings
            let quotaConfigurationChanged = settings.accounts != saved.accounts || settings.sources != saved.sources || settings.hosts != saved.hosts || settings.agents != saved.agents || settings.proxy != saved.proxy
            if settings.proxy != saved.proxy || settings.proxyTestUrls != saved.proxyTestUrls { networkRevision += 1; networkTest = nil; lastNetworkAttempt = .distantPast }
            mergeConfiguration(saved, submitted: draft, draftAtSubmission: draftAtSubmission)
            if quotaConfigurationChanged { quotaNextAttempt = nil }
            if accountStatusConfiguration(base) != accountStatusConfiguration(saved) {
                invalidateAccountStatuses()
                Task { await loadAccountStatuses(refresh: true, dueOnly: true) }
            }
            if message == configurationSaveError { message = nil }
            configurationSaveError = nil
            if refreshDashboard { Task {
                if historyConfigurationChanged { await reload() }
                if quotaConfigurationChanged && hasQuotaAccounts { await refreshQuotas(dueOnly: true) }
            } }
            return true
        } catch { configurationSaveError = error.localizedDescription; message = configurationSaveError; return false }
    }
    func syncPrices() async {
        guard !busy else { return }; busy = true; activity = L("同步价格"); beginRefresh("prices"); defer { busy = false; activity = "" }
        do { let _: Acknowledgement = try await engine.call("prices.sync"); await loadPrices(force: true); if let error = priceCatalog.error { throw ClientError.message(error) }; let loaded = await reload(); message = loaded ? L("价格已更新") : message
            finishRefresh("prices", failures: loaded ? [] : [ActionFailure(action: "prices", name: L("概览"), reason: message ?? L("读取失败"))]) }
        catch { message = error.localizedDescription; finishRefresh("prices", failures: [ActionFailure(action: "prices", name: L("价格"), reason: error.localizedDescription)]) }
    }
    func loadPrices(force: Bool = false) async { await priceCatalog.load(force: force) }
    func referencePrices(model: String? = nil, name: String = "") async {
        guard !busy, !repricing, await saveSettingsDraft(refreshDashboard: false) else { return }
        busy = true; activity = L("智能引用价格")
        defer { busy = false; activity = "" }
        do {
            var params: [String: Any] = [:]
            if let model { params = ["model":model, "name":name] }
            let result: PriceReferenceResult = try await engine.call("prices.reference", params: params)
            acceptConfiguration(result.settings)
            await reload()
            settingsMessage = L("已智能引用 {0} 个模型；{1} 个未匹配；{2} 个已有价格或映射。仅供估值，历史成本可点“保存并重算”更新。" , result.matched, result.unmatched.count, result.skipped)
        } catch { settingsMessage = error.localizedDescription }
    }
    func savePrice(_ price: ModelPrice) async -> Bool {
        do {
            let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(price))
            let _: Acknowledgement = try await engine.call("prices.save", params: ["prices":[object]])
            await loadPrices(force: true); if let error = priceCatalog.error { throw ClientError.message(L("价格已保存，但刷新失败：") + error) }; await reload(); message = L("已保存价格 · ") + Format.time(Date().timeIntervalSince1970); return true
        } catch { message = error.localizedDescription; return false }
    }
    func saveAndReprice() async {
        let mapping = (mappingModel, mappingID)
        guard await saveSettingsDraft(refreshDashboard: false) else { return }
        if await reprice() { settingsMessage = L("已保存并按当前价格重算") }
        else { mappingModel = mapping.0; mappingID = mapping.1; settingsMessage = L("配置已保存，重算失败：") + (message ?? L("请重试")) }
    }
    @discardableResult func reprice() async -> Bool {
        guard !repricing else { return false }; repricing = true
        defer { repricing = false }
        do { let _: Acknowledgement = try await engine.call("prices.recalculate"); let loaded = await reload(); message = loaded ? L("已按当前价格重算") : L("重算已完成，概览读取失败，请重试查询"); return true }
        catch { message = error.localizedDescription; return false }
    }
    var updateBlockers: [String] {
        [(settingsSaving, L("保存配置")), (repricing, L("重算价格")), (agyLoginActive, L("Antigravity 登录授权")), (UserDefaults.standard.string(forKey: "codexLogin.operation") != nil, L("Codex 登录授权（可在账户页继续或取消）"))].filter { $0.0 }.map { $0.1 }
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
    private var historyBlocked: Bool { busy || !estimateBusy.isEmpty }
    private var metricsDeadline: Date { lastMetrics.addingTimeInterval(Double(serverTabVisible ? settings.foregroundInterval : settings.serverRefreshSeconds)) }
    private var pendingResets: [(Quota, Double, String)] {
        menuDashboard.quotas.flatMap { quota in quota.windows.compactMap { window in
            guard let reset = window.resetsAt else { return nil }
            let key = quota.id + ":" + String(reset)
            return resetConfirmations.contains(key) ? nil : (quota, reset, key)
        } }
    }
    private func scheduleRefreshes() {
        configureHostMonitor()
        timer?.invalidate(); timer = nil
        guard schedulingEnabled, settingsLoaded, !stopped, !installingUpdate, !settingsSaving, !repricing else { return }
        let now = Date()
        if lastScan > now { lastScan = .distantPast }
        if lastMetrics > now { lastMetrics = .distantPast }
        if lastNetworkAttempt > now { lastNetworkAttempt = .distantPast }
        if !quotaScheduleKnown { readQuotaSchedule() }
        var deadlines: [Date] = []
        if panelVisible, !networkBusy { deadlines.append(lastNetworkAttempt.addingTimeInterval(300)) }
        if !hostMonitoringSupported, !serverBusy, settings.monitoredHosts.contains(where: \.enabled) { deadlines.append(metricsDeadline) }
        if hostMonitoringSupported, hostMonitorRetryAt > now { deadlines.append(hostMonitorRetryAt) }
        if !historyBlocked {
            deadlines.append(lastScan.addingTimeInterval(Double(settings.historyInterval)))
        }
        if !quotaBusy {
            if quotaScheduleKnown, let quotaNextAttempt { deadlines.append(quotaNextAttempt) }
            deadlines += pendingResets.map { Date(timeIntervalSince1970: $0.1) }
        }
        guard let next = deadlines.min() else { return }
        timer = Timer.scheduledTimer(withTimeInterval: max(0.01, next.timeIntervalSinceNow), repeats: false) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
    }
    func resumeAfterWake() {
        invalidateQuotaSchedule(); tick()
        if settingsLoaded { Task { await refreshSessions() } }
    }
    func tick() {
        guard !stopped, !installingUpdate, !settingsSaving, !repricing else { return }
        defer { scheduleRefreshes() }
        let now = Date()
        if panelVisible, !networkBusy, now.timeIntervalSince(lastNetworkAttempt) >= 300 { Task { await testNetwork(force: false) } }
        if !hostMonitoringSupported, !serverBusy, settings.monitoredHosts.contains(where: \.enabled), now >= metricsDeadline { Task { await sampleHosts() } }
        if !historyBlocked, now.timeIntervalSince(lastScan) >= Double(settings.historyInterval) { Task { await scan() } }
        guard !quotaBusy else { return }
        if let (quota, _, key) = pendingResets.first(where: { $0.1 <= now.timeIntervalSince1970 }) {
            resetConfirmations.insert(key); Task { await refreshQuotas(accountID: quota.accountId) }; return
        }
        if quotaScheduleKnown, let next = quotaNextAttempt, now >= next {
            quotaNextAttempt = nil; Task { await refreshQuotas(dueOnly: true) }
        }
    }
    func stop() {
        stopped = true; schedulingEnabled = false; timer?.invalidate(); timer = nil
        hostMonitorTask?.cancel(); hostMonitorTask = nil
        if let hostMonitorObserver { NotificationCenter.default.removeObserver(hostMonitorObserver) }
        sessionTask?.cancel(); sessionTask = nil
        runtimeTask?.cancel(); runtimeRefresh?.cancel(); summaryEngine.stop()
        if let runtimeObserver { NotificationCenter.default.removeObserver(runtimeObserver) }
        automaticTask?.cancel(); automaticEngine.stop(); resetNotifications.stop()
        filterRefresh?.cancel(); reloadTask?.cancel(); quotaScheduleTask?.cancel()
        deploymentSyncTask?.cancel(); settingsRefreshTask?.cancel(); generalSaveTask?.cancel(); petLibraryTask?.cancel()
        for task in antigravityRefreshTasks.values { task.cancel() }
        priceCatalog.stop(); petEngine.stop(); sessionEngine.stop(); accountEngine.stop(); engine.stop(); metricsEngine.stop(); networkEngine.stop()
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
