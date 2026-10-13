import Foundation

extension AppModel {
    var agentConfigurations: [AgentConfiguration] {
        settings.agents ?? CoreContract.providers.map { provider in
            let sources = settings.sources.filter { $0.provider == provider && $0.enabled }
            return AgentConfiguration(provider: provider, enabled: !sources.isEmpty, machineIds: Array(Set(sources.map { $0.hostId ?? "local" })).sorted())
        }
    }
    func acceptConfiguration(_ saved: Settings) {
        for account in saved.accounts where !settings.accounts.contains(where: { $0.key == account.key }) { PanelAccountPreference.includeNew(account, accounts: saved.accounts) }
        invalidateAccountStatuses()
        settings = saved; settingsDraft = saved; settingsMessage = "已保存"
        if selectedAccount != "all" && selectedAccount != "none" && !settings.filterAccounts.contains(where: { $0.key == selectedAccount }) { selectedAccount = "all" }
        scheduleConfigurationRefresh()
    }
    func scheduleConfigurationRefresh() {
        settingsRefreshTask?.cancel()
        settingsRefreshTask = Task {
            do { try await Task.sleep(for: .milliseconds(300)) } catch { return }
            guard !Task.isCancelled, !installingUpdate else { return }
            Task { await loadAccountStatuses(refresh: true, provider: "codex") }
            await reload()
            guard !Task.isCancelled, !installingUpdate else { return }
            await refreshQuotas()
        }
        synchronizeAccountDeployments()
    }
    func synchronizeAccountDeployments() {
        guard deploymentSyncTask == nil else { return }
        deploymentSyncTask = Task {
            defer { deploymentSyncTask = nil }
            do {
                let result: AccountCleanupResult = try await accountEngine.call("accounts.deployments.sync")
                deploymentSyncMessage = result.failedTasks?.isEmpty == false ? "配置已保存，部分唤醒部署待同步，可重试。" : nil
            } catch { deploymentSyncMessage = "配置已保存，唤醒部署待同步：" + error.localizedDescription }
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
                for account in settings.accounts.filter({ $0.provider == "codex" }) {
                    do {
                        let row: AccountDeviceStatus = try await reader.call("accounts.status.refresh", params: ["accountKey":account.key,"sourceId":sourceID])
                        rows.append(row)
                        if row.error != nil { succeeded = false }
                    } catch { succeeded = false }
                }
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
        let config = agentConfigurations.first { $0.provider == provider }
        let active = (config?.enabled == true ? config?.machineIds ?? [] : []).filter { id in id == "local" ? settings.localDeviceEnabled != false : settings.hosts.contains { $0.id == id && $0.enabled } }
        if !includePaused { return Array(Set(active)).sorted { $0 == "local" && $1 != "local" || ($0 != "local" && $1 != "local" && $0 < $1) } }
        return Array(Set(active + settings.sources.filter { $0.provider == provider }.map { $0.hostId ?? "local" })).sorted { $0 == "local" && $1 != "local" || ($0 != "local" && $1 != "local" && $0 < $1) }
    }
    func machineName(_ id: String) -> String { id == "local" ? "本机" : settings.hosts.first { $0.id == id }.map { $0.name.isEmpty ? $0.target : $0.name } ?? "已移除设备" }
    func accountDeviceSummary(_ account: AgentAccount, machine: String) -> String {
        let rows = accountStatuses.filter { $0.accountKey == account.key && $0.machineId == machine }
        if let summary = rows.first(where: { $0.identity != nil })?.identitySummary ?? rows.compactMap(\.identitySummary).first { return summary }
        let current = rows.contains { $0.current == true }, credential = rows.contains { $0.credential == true }
        let uncertain = rows.isEmpty || rows.contains { $0.error != nil || $0.current == nil || $0.credential == nil }
        let loginText = current || credential ? "已登录" : (!rows.isEmpty && rows.allSatisfy { $0.current == false && $0.credential == false } ? "未登录" : "登录待确认")
        let usageText = current ? "正在使用" : (!rows.isEmpty && rows.allSatisfy { $0.current == false } ? "未使用" : "使用状态待确认")
        return loginText + " · " + usageText + (uncertain ? " · 待确认" : "")
    }
    func accountDeviceCounts(_ key: String) -> AccountDeviceCounts {
        let provider = settings.accounts.first { $0.key == key }?.provider ?? "codex"
        return AccountDeviceCounts(accountKey: key, machines: accountMachineIDs(provider), rows: accountStatuses)
    }
    func accountCounts(_ account: AgentAccount) -> String { accountDeviceCounts(account.key).summary }
    func configureAgent(_ provider: String, enabled: Bool? = nil, machines: [String]? = nil) async {
        guard !settingsSaving else { return }
        settingsSaving = true; settingsMessage = nil
        defer { settingsSaving = false }
        do {
            var params: [String: Any] = ["provider": provider]
            if let enabled { params["enabled"] = enabled }
            if let machines { params["machineIds"] = machines }
            let saved: Settings = try await engine.call("agents.set", params: params)
            acceptConfiguration(saved)
        } catch { settingsMessage = error.localizedDescription }
    }
    func saveGeneralConfiguration() async {
        guard (5...86400).contains(settingsDraft.refreshSeconds),
              (10...86400).contains(settingsDraft.historyInterval),
              (2...86400).contains(settingsDraft.foregroundInterval),
              (2...86400).contains(settingsDraft.serverRefreshSeconds) else {
            settingsMessage = "刷新间隔范围：额度 5–86400 秒，记录 10–86400 秒，服务器 2–86400 秒"
            return
        }
        var next = settings
        next.appearance = settingsDraft.appearance; next.proxy = settingsDraft.proxy
        next.proxyTestUrls = settingsDraft.proxyTestUrls; next.menuMetric = settingsDraft.menuMetric
        next.historyShowDeletedAccounts = settingsDraft.historyShowDeletedAccounts
        next.historyRefreshSeconds = settingsDraft.historyRefreshSeconds; next.serverForegroundRefreshSeconds = settingsDraft.serverForegroundRefreshSeconds; next.localMonitor = settingsDraft.localMonitor; next.refreshSeconds = settingsDraft.refreshSeconds; next.serverRefreshSeconds = settingsDraft.serverRefreshSeconds
        guard next != settings else { return }
        do { try await persistConfiguration(next) } catch { settingsMessage = error.localizedDescription }
    }
    func persistConfiguration(_ next: Settings, base: Settings? = nil) async throws {
        guard !settingsSaving else { throw ClientError.message("正在保存，请稍后重试") }
        let previous = settingsDraft
        settingsDraft = next
        guard await saveSettingsDraft(includeMappings: false, base: base) else {
            settingsDraft = previous
            throw ClientError.message(settingsMessage ?? "保存失败，输入已保留")
        }
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
                guard let reference = result["passwordRef"], !reference.isEmpty else { throw ClientError.message("密码保存失败") }
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
