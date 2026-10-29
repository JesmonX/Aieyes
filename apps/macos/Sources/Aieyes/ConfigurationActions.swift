import Foundation

extension AppModel {
    var rawAgentConfigurations: [AgentConfiguration] {
        let rows = settings.agents ?? CoreContract.providers.map { provider in
            let sources = settings.sources.filter { $0.provider == provider && $0.enabled }
            return AgentConfiguration(provider: provider, enabled: !sources.isEmpty, machineIds: Array(Set(sources.map { $0.hostId ?? "local" })).sorted())
        }
        let order = CoreContract.providers.filter { $0 != "custom" }
        func rank(_ provider: String) -> Int { provider == "custom" ? order.count + 1 : order.firstIndex(of: provider) ?? order.count }
        return rows.sorted { rank($0.provider) < rank($1.provider) }
    }
    var agentConfigurations: [AgentConfiguration] {
        var groups: [AgentConfiguration] = []
        for row in rawAgentConfigurations {
            let provider = CoreContract.agentProvider(row.provider)
            if !groups.contains(where: { $0.provider == provider }) {
                groups.append(AgentConfiguration(provider: provider, enabled: false, machineIds: []))
            }
            let index = groups.firstIndex { $0.provider == provider }!
            groups[index].enabled = groups[index].enabled || row.enabled
            if CoreContract.supports(row.provider, "machines") {
                groups[index].machineIds = Array(Set(groups[index].machineIds + row.machineIds)).sorted()
            }
        }
        return groups
    }
    var settingsAgentIDs: [String] {
        settings.appearance?.settingsAgentIds ?? agentConfigurations.filter { agent in
            agent.enabled || !agent.machineIds.isEmpty || settings.sources.contains { CoreContract.sameAgent($0.provider, agent.provider) } || settings.accounts.contains { CoreContract.sameAgent($0.provider, agent.provider) }
        }.map(\.provider)
    }
    func orderAgents(_ ids: [String]) async {
        guard !settingsSaving else { return }
        settingsSaving = true; settingsMessage = nil
        defer { settingsSaving = false }
        do { let saved: Settings = try await engine.call("agents.manage", params: ["order": ids]); acceptConfiguration(saved) }
        catch { settingsMessage = error.localizedDescription }
    }
    func manageAgent(_ provider: String, visible: Bool) async {
        guard !settingsSaving else { return }
        settingsSaving = true; settingsMessage = nil
        defer { settingsSaving = false }
        do {
            let saved: Settings = try await engine.call("agents.manage", params: ["provider":provider, "visible":visible])
            acceptConfiguration(saved)
            if visible { settingsAgent = provider }
            else if settingsAgent == provider { settingsAgent = settingsAgentIDs.first ?? "" }
        } catch { settingsMessage = error.localizedDescription }
    }
    var overviewAgentProviders: [String] {
        CoreContract.agentProviders.filter { provider in
            agentConfigurations.contains { CoreContract.sameAgent($0.provider, provider) && $0.enabled }
            || (settings.agents == nil && settings.accounts.contains { CoreContract.sameAgent($0.provider, provider) && $0.archived != true })
        }
    }
    func machineConfiguration(_ provider: String) -> AgentConfiguration? {
        guard let key = CoreContract.featureProvider(provider, "machines"), var config = rawAgentConfigurations.first(where: { $0.provider == key }) else { return nil }
        config.enabled = agentConfigurations.first { CoreContract.sameAgent($0.provider, provider) }?.enabled ?? config.enabled
        return config
    }
    func accountStatusConfiguration(_ value: Settings) -> Data {
        let object = (try? JSONSerialization.jsonObject(with: JSONEncoder().encode(value))) as? [String: Any] ?? [:]
        func pick(_ rows: Any?, _ keys: [String]) -> [[String: Any]] { (rows as? [[String: Any]] ?? []).map { row in row.filter { keys.contains($0.key) } } }
        let sources = pick(object["sources"], ["id","provider","accountId","path","hostId","enabled","machineBinding","codexHomeId","codexBinary","agyBinary","quotaPreCommand","subscription","proxy"])
        let next: [String: Any] = ["accounts":pick(object["accounts"], ["id","provider","identityKey","quotaProfileId","connections","deviceSettings"]), "sources":sources, "hosts":pick(object["hosts"], ["id","target","port","username","identityFile","authMode","passwordRef","shell","preCommand","enabled"]), "agents":object["agents"] ?? [], "localDeviceEnabled":value.localDeviceEnabled ?? true, "nativeConnections":pick(object["nativeConnections"], ["id","provider","machineId","sourceId","sourcePath","root","configPath","selector","kind","accountKey","identityKey","status","hasCredential"]), "proxy": sources.contains { ($0["provider"] as? String) != "codex" } ? (object["proxy"] ?? [:]) : [:]]
        return (try? JSONSerialization.data(withJSONObject: next, options: .sortedKeys)) ?? Data()
    }
    func acceptConfiguration(_ saved: Settings) {
        if accountStatusConfiguration(settings) != accountStatusConfiguration(saved) { invalidateAccountStatuses() }
        mergeConfiguration(saved)
        scheduleConfigurationRefresh()
    }
    func mergeConfiguration(_ saved: Settings, submitted: Settings? = nil, draftAtSubmission: Settings? = nil) {
        let draft = SettingsDraftMerge.rebase(base: settings, draft: settingsDraft, saved: saved, submitted: submitted, draftAtSubmission: draftAtSubmission)
        if settings != saved { settings = saved }
        if settingsDraft != draft { settingsDraft = draft }
        reconcileDashboardFilters()
    }
    func scheduleConfigurationRefresh() {
        settingsRefreshTask?.cancel()
        settingsRefreshTask = Task {
            do { try await Task.sleep(for: .milliseconds(300)) } catch { return }
            guard !Task.isCancelled, !installingUpdate else { return }
            Task { await loadAccountStatuses(refresh: true, dueOnly: true) }
            await reload()
            guard !Task.isCancelled, !installingUpdate else { return }
            await refreshQuotas(dueOnly: true)
        }
        synchronizeAccountDeployments()
    }
    func synchronizeAccountDeployments() {
        deploymentSyncRequested = true
        guard deploymentSyncTask == nil else { return }
        deploymentSyncTask = Task {
            defer { deploymentSyncTask = nil }
            repeat {
                deploymentSyncRequested = false
                do {
                    let result: AccountCleanupResult = try await accountEngine.call("accounts.deployments.sync")
                    deploymentSyncMessage = result.failedTasks?.isEmpty == false ? L("配置已保存，部分唤醒部署待同步，可重试。") : nil
                } catch { deploymentSyncMessage = L("配置已保存，唤醒部署待同步：") + L(error.localizedDescription) }
            } while deploymentSyncRequested && !Task.isCancelled
        }
    }
    /// Refresh the switched home without waiting for unrelated SSH checks. Repeated requests
    /// coalesce, and a switch invalidates observations that started before it.
    func refreshCodexAccountStatuses(sourceID: String, using reader: EngineClient) async -> Bool {
        accountStatusRevision += 1
        accountSourceStatusRequests[sourceID] = accountStatusRevision
        if let pending = accountSourceStatusTasks[sourceID] { return await pending.value }
        let work = Task { @MainActor in
            defer { accountSourceStatusTasks[sourceID] = nil }
            while !Task.isCancelled {
                let requested = accountSourceStatusRequests[sourceID]
                var rows: [AccountDeviceStatus] = [], succeeded = true
                guard settings.sources.contains(where: { $0.id == sourceID && $0.provider == "codex" && settings.sourceEnabled($0) }) else { return false }
                do {
                    rows = try await reader.call("accounts.status.refreshBatch", params: ["sourceIds":[sourceID], "accountKeys":settings.accounts.filter { $0.provider == "codex" }.map(\.key)])
                    succeeded = rows.allSatisfy { $0.error == nil }
                } catch { succeeded = false }
                if requested != accountSourceStatusRequests[sourceID] { continue }
                for row in rows where settings.accounts.contains(where: { $0.key == row.accountKey }) {
                    accountStatusStore.upsert(row)
                }
                return succeeded
            }
            return false
        }
        accountSourceStatusTasks[sourceID] = work
        return await work.value
    }
    func accountMachineIDs(_ provider: String, includePaused: Bool = false) -> [String] {
        let config = rawAgentConfigurations.first { $0.provider == provider }
        let active = (config?.enabled == true ? config?.machineIds ?? [] : []).filter { id in
            settings.sources.contains { $0.provider == provider && ($0.hostId ?? "local") == id && settings.sourceEnabled($0) }
        }
        if !includePaused { return Array(Set(active)).sorted { $0 == "local" && $1 != "local" || ($0 != "local" && $1 != "local" && $0 < $1) } }
        return Array(Set(active + settings.sources.filter { $0.provider == provider }.map { $0.hostId ?? "local" })).sorted { $0 == "local" && $1 != "local" || ($0 != "local" && $1 != "local" && $0 < $1) }
    }
    func machineName(_ id: String) -> String { id == "local" ? L("本机") : settings.hosts.first { $0.id == id }.map { $0.name.isEmpty ? $0.target : $0.name } ?? L("已移除设备") }
    func accountDeviceSummary(_ account: AgentAccount, machine: String) -> String {
        let rows = accountStatuses.filter { $0.accountKey == account.key && $0.machineId == machine }
        if let summary = rows.first(where: { $0.identity != nil })?.identitySummary ?? rows.compactMap(\.identitySummary).first { return summary }
        let current = rows.contains { $0.current == true }, credential = rows.contains { $0.credential == true }
        let uncertain = rows.isEmpty || rows.contains { $0.error != nil || $0.current == nil || $0.credential == nil }
        let loginText = current || credential ? L("已登录") : (!rows.isEmpty && rows.allSatisfy { $0.current == false && $0.credential == false } ? L("未登录") : L("登录待确认"))
        let usageText = current ? L("正在使用") : (!rows.isEmpty && rows.allSatisfy { $0.current == false } ? L("未使用") : L("使用状态待确认"))
        return loginText + " · " + usageText + (uncertain ? L(" · 待确认") : "")
    }
    func accountDeviceCounts(_ key: String) -> AccountDeviceCounts {
        let provider = settings.accounts.first { $0.key == key }?.provider ?? "codex"
        return AccountDeviceCounts(accountKey: key, machines: accountMachineIDs(provider), rows: accountStatuses)
    }
    func accountCounts(_ account: AgentAccount) -> String { accountDeviceCounts(account.key).summary }
    func configureAgent(_ provider: String, enabled: Bool? = nil, machines: [String]? = nil, group: Bool = false) async {
        guard !settingsSaving else { return }
        settingsSaving = true; settingsMessage = nil
        defer { settingsSaving = false }
        do {
            var params: [String: Any] = ["provider": provider, "group": group]
            if let enabled { params["enabled"] = enabled }
            if let machines { params["machineIds"] = machines }
            let saved: Settings = try await engine.call("agents.set", params: params)
            acceptConfiguration(saved)
        } catch { settingsMessage = error.localizedDescription }
    }
    func saveGeneralConfiguration() async {
        if let pending = generalSaveTask { await pending.value; return }
        let work = Task { @MainActor in
            defer { generalSaveTask = nil }
            while !Task.isCancelled {
                let intervals = [("quota", settingsDraft.refreshSeconds, 5, L("额度查询")), ("history", settingsDraft.historyInterval, 10, L("日志同步")), ("foreground", settingsDraft.foregroundInterval, 2, L("服务器前台监控")), ("background", settingsDraft.serverRefreshSeconds, 2, L("服务器后台监控"))]
                if let (field, _, minimum, label) = intervals.first(where: { !(($0.2)...86400).contains($0.1) }) {
                    settingsValidationField = field
                    settingsMessage = label + L("间隔须为 {0}–86400 秒，输入已保留" , minimum)
                    return
                }
                settingsValidationField = nil
                var next = settings
                next.appearance = settingsDraft.appearance; next.proxy = settingsDraft.proxy
                next.proxyTestUrls = settingsDraft.proxyTestUrls; next.menuMetric = settingsDraft.menuMetric
                next.quotaResetNotifications = settingsDraft.quotaResetNotifications
                next.historyShowDeletedAccounts = settingsDraft.historyShowDeletedAccounts
                next.preciseCounting = settingsDraft.preciseCounting
                next.historyRefreshSeconds = settingsDraft.historyRefreshSeconds
                next.serverForegroundRefreshSeconds = settingsDraft.serverForegroundRefreshSeconds
                next.localMonitor = settingsDraft.localMonitor
                next.refreshSeconds = settingsDraft.refreshSeconds; next.serverRefreshSeconds = settingsDraft.serverRefreshSeconds
                guard next != settings else { return }
                do { try await persistConfiguration(next) }
                catch { settingsMessage = error.localizedDescription; return }
            }
        }
        generalSaveTask = work
        await work.value
    }
    func persistConfiguration(_ next: Settings, base: Settings? = nil) async throws {
        guard await save(next, base: base) else {
            settingsMessage = message ?? L("保存失败，输入已保留")
            throw ClientError.message(settingsMessage ?? L("保存失败，输入已保留"))
        }
        settingsMessage = L("已保存 · ") + Format.time(Date().timeIntervalSince1970)
        synchronizeAccountDeployments()
        Task { await loadAccountStatuses() }
    }
    func saveHostConfiguration(_ host: Host, password: String, base: Settings) async throws {
        var next = base
        var created: String?
        if host.id == "local" {
            next.localMonitor = LocalMonitor(host)
        } else {
            var item = host
            if host.authMode == "password" && !password.isEmpty {
                let result: [String: String] = try await engine.call("hosts.credentials.save", params: ["password": password])
                guard let reference = result["passwordRef"], !reference.isEmpty else { throw ClientError.message(L("密码保存失败")) }
                created = reference
                item.passwordRef = reference
            }
            if let index = next.hosts.firstIndex(where: { $0.id == item.id }) { next.hosts[index] = item }
            else { next.hosts.append(item) }
        }
        do { try await persistConfiguration(next, base: base) }
        catch {
            if let created { let _: Acknowledgement? = try? await engine.call("hosts.credentials.delete", params: ["passwordRef": created]) }
            throw error
        }
    }
    func setDeviceEnabled(_ id: String, enabled: Bool) async {
        var next = settings
        if id == "local" { next.localDeviceEnabled = enabled }
        else if let index = next.hosts.firstIndex(where: { $0.id == id }) { next.hosts[index].enabled = enabled }
        do { try await persistConfiguration(next) } catch { settingsMessage = error.localizedDescription }
    }
    func removeConfiguration(_ request: RemovalRequest) async {
        guard request.kind == "host" else { return }
        var next = settings
        let id = request.itemID
        next.hosts.removeAll { $0.id == id }
        if var agents = next.agents {
            for i in agents.indices { agents[i].machineIds.removeAll { $0 == id } }
            next.agents = agents
        }
        do { try await persistConfiguration(next) } catch { settingsMessage = error.localizedDescription }
    }
}

/// Rebase only local edits over a refreshed configuration. Array records are merged
/// by their persisted identity, so login can add an account without replacing a draft.
enum SettingsDraftMerge {
    static func rebase(base: Settings, draft: Settings, saved: Settings, submitted: Settings? = nil, draftAtSubmission: Settings? = nil) -> Settings {
        do {
            let encoder = JSONEncoder()
            let values = try [base, draft, saved].map { try JSONSerialization.jsonObject(with: encoder.encode($0)) }
            let submittedValue = try submitted.map { try JSONSerialization.jsonObject(with: encoder.encode($0)) }
            let originalDraft = try (draftAtSubmission ?? submitted).map { try JSONSerialization.jsonObject(with: encoder.encode($0)) }
            let merged = merge(base: values[0], draft: values[1], saved: values[2], submitted: submittedValue, originalDraft: originalDraft, acknowledging: submitted != nil) ?? [:]
            return try JSONDecoder().decode(Settings.self, from: JSONSerialization.data(withJSONObject: merged))
        } catch { return draft }
    }

    private static func equal(_ lhs: Any?, _ rhs: Any?) -> Bool {
        switch (lhs, rhs) {
        case (nil, nil): return true
        case let (left?, right?): return (left as AnyObject).isEqual(right)
        default: return false
        }
    }
    private static func identity(_ value: Any) -> String? {
        guard let row = value as? [String: Any] else { return nil }
        if let id = row["id"] as? String { return (row["provider"] as? String ?? "") + ":" + id }
        for field in ["machineId", "sourceId", "provider"] { if let id = row[field] as? String { return field + ":" + id } }
        return nil
    }
    private static func merge(base: Any?, draft: Any?, saved: Any?, submitted: Any?, originalDraft: Any?, acknowledging: Bool) -> Any? {
        if equal(draft, saved) { return saved }
        let unchangedDuringSave = !acknowledging || equal(draft, originalDraft)
        if equal(base, draft), unchangedDuringSave { return saved }
        // A successful save acknowledges only the submitted value. Core may trim or
        // normalize it; subsequent input and unrelated drafts must remain intact.
        if acknowledging, unchangedDuringSave, equal(draft, submitted) { return saved }
        guard let draft else { return nil }
        if let draft = draft as? [String: Any] {
            let base = base as? [String: Any] ?? [:], saved = saved as? [String: Any] ?? [:]
            let submitted = submitted as? [String: Any] ?? [:]
            let originalDraft = originalDraft as? [String: Any] ?? [:]
            var result = saved
            for key in Set(base.keys).union(draft.keys).union(originalDraft.keys) {
                result[key] = merge(base: base[key], draft: draft[key], saved: saved[key], submitted: submitted[key], originalDraft: originalDraft[key], acknowledging: acknowledging)
            }
            return result
        }
        if let draft = draft as? [Any], let base = base as? [Any], let saved = saved as? [Any],
           (base + draft + saved).allSatisfy({ identity($0) != nil }) {
            func records(_ rows: [Any]) -> [String: Any] { rows.reduce(into: [:]) { $0[identity($1)!] = $1 } }
            let before = records(base), edited = records(draft), current = records(saved)
            let submitted = records(submitted as? [Any] ?? [])
            let originalDraft = records(originalDraft as? [Any] ?? [])
            var seen = Set<String>()
            let order = (saved + draft).compactMap(identity).filter { seen.insert($0).inserted }
            return order.compactMap { merge(base: before[$0], draft: edited[$0], saved: current[$0], submitted: submitted[$0], originalDraft: originalDraft[$0], acknowledging: acknowledging) }
        }
        return draft
    }
}
