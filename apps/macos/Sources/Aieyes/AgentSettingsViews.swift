import SwiftUI

extension AppModel {
    var agentConfigurations: [AgentConfiguration] {
        settings.agents ?? ["codex", "claude", "antigravity", "deepseek", "custom"].map { provider in
            let sources = settings.sources.filter { $0.provider == provider && $0.enabled }
            return AgentConfiguration(provider: provider, enabled: !sources.isEmpty, machineIds: Array(Set(sources.map { $0.hostId ?? "local" })).sorted())
        }
    }
    func acceptConfiguration(_ saved: Settings) {
        for account in saved.accounts where !settings.accounts.contains(where: { $0.key == account.key }) { PanelAccountPreference.includeNew(account, accounts: saved.accounts) }
        settings = saved; settingsDraft = saved; settingsMessage = "已保存"
        scheduleConfigurationRefresh()
    }
    func scheduleConfigurationRefresh() {
        settingsRefreshTask?.cancel()
        settingsRefreshTask = Task {
            do { try await Task.sleep(for: .milliseconds(300)) } catch { return }
            guard !Task.isCancelled, !installingUpdate else { return }
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
                guard settings.sources.contains(where: { $0.id == sourceID && $0.provider == "codex" && $0.enabled }) else { return false }
                for account in settings.accounts.filter({ $0.provider == "codex" }) {
                    do {
                        let row: AccountDeviceStatus = try await reader.call("accounts.status.refresh", params: ["accountKey":account.key,"sourceId":sourceID])
                        rows.append(row)
                        if row.error != nil { succeeded = false }
                    } catch { succeeded = false }
                }
                if requested != accountSourceStatusRequests[sourceID] { continue }
                for row in rows where settings.accounts.contains(where: { $0.key == row.accountKey }) {
                    accountStatuses.removeAll { $0.id == row.id }; accountStatuses.append(row)
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
        let active = (config?.enabled == true ? config?.machineIds ?? [] : []).filter { id in id == "local" || settings.hosts.contains { $0.id == id && $0.enabled } }
        if !includePaused { return active.sorted { $0 == "local" && $1 != "local" || ($0 != "local" && $1 != "local" && $0 < $1) } }
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
    func accountCounts(_ account: AgentAccount) -> String {
        let machines = accountMachineIDs(account.provider)
        let logged = machines.filter { machine in accountStatuses.contains { $0.accountKey == account.key && $0.machineId == machine && $0.current == true } }.count
        let stored = machines.filter { machine in accountStatuses.contains { $0.accountKey == account.key && $0.machineId == machine && ($0.current == true || $0.credential == true) } }.count
        let pending = machines.contains { machine in let rows = accountStatuses.filter { $0.accountKey == account.key && $0.machineId == machine }; return rows.isEmpty || rows.contains { $0.current == nil || $0.credential == nil || $0.error != nil } }
        return "已登录 \(stored)/\(machines.count) · 使用中 \(logged)/\(machines.count)" + (pending ? " · 有设备待确认" : "")
    }
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
        var next = settings
        next.appearance = settingsDraft.appearance; next.proxy = settingsDraft.proxy
        next.proxyTestUrls = settingsDraft.proxyTestUrls; next.menuMetric = settingsDraft.menuMetric
        next.historyRefreshSeconds = settingsDraft.historyRefreshSeconds; next.serverForegroundRefreshSeconds = settingsDraft.serverForegroundRefreshSeconds; next.localMonitor = settingsDraft.localMonitor; next.refreshSeconds = settingsDraft.refreshSeconds; next.serverRefreshSeconds = settingsDraft.serverRefreshSeconds
        guard next != settings else { return }
        do { try await persistConfiguration(next) } catch { settingsMessage = error.localizedDescription }
    }
    func persistConfiguration(_ next: Settings) async throws {
        guard !settingsSaving else { throw ClientError.message("正在保存，请稍后重试") }
        let previous = settingsDraft
        settingsDraft = next
        guard await saveSettingsDraft(includeMappings: false) else {
            settingsDraft = previous
            throw ClientError.message(settingsMessage ?? "保存失败，输入已保留")
        }
        synchronizeAccountDeployments()
        Task { await loadAccountStatuses() }
    }
    func removeConfiguration(_ request: RemovalRequest) async {
        var next = settings
        let id = request.itemID
        switch request.kind {
        case "host":
            next.hosts.removeAll { $0.id == id }
            for i in next.sources.indices where next.sources[i].hostId == id { next.sources[i].hostId = nil; next.sources[i].enabled = false }
            if var agents = next.agents {
                for i in agents.indices { agents[i].machineIds.removeAll { $0 == id } }
                next.agents = agents
            }
        case "source":
            next.sources.removeAll { $0.id == id }
            for i in next.accounts.indices where next.accounts[i].quotaSourceId == id { next.accounts[i].quotaSourceId = nil }
        case "account":
            if let i = next.accounts.firstIndex(where: { $0.key == id }) { next.accounts[i].archived = true }
        default: return
        }
        do { try await persistConfiguration(next) } catch { settingsMessage = error.localizedDescription }
    }
}

struct AgentDirectorySelection: Identifiable {
    let provider: String, machine: String
    var id: String { provider + ":" + machine }
}
struct AgentsSettingsView: View {
    @ObservedObject var model: AppModel
    var editSource: (AgentSource) -> Void
    @State private var directories: AgentDirectorySelection?
    var body: some View {
        Form {
            ForEach(model.agentConfigurations) { agent in
                Section {
                    HStack {
                        ProviderMark(provider: agent.provider)
                        Toggle(Format.provider(agent.provider), isOn: Binding(get: { agent.enabled }, set: { enabled in
                            Task { await model.configureAgent(agent.provider, enabled: enabled, machines: enabled && agent.machineIds.isEmpty ? ["local"] : nil) }
                        })).toggleStyle(.switch)
                    }
                    if agent.provider == "deepseek" { Text("在账户中添加 API Key，通过本机查询余额。").foregroundStyle(.secondary) }
                    else {
                        Text("所在机器 · 可多选").font(AppFont.secondary).foregroundStyle(.secondary)
                        machine("local", name: "本机", agent: agent)
                        ForEach(model.settings.hosts) { host in machine(host.id, name: (host.name.isEmpty ? host.target : host.name) + (host.enabled ? "" : " · 服务器已停用"), agent: agent) }
                        if agent.machineIds.isEmpty { Text("未选择机器，暂不采集。").foregroundStyle(.secondary) }
                        if model.settings.hosts.isEmpty { Button("添加服务器") { model.settingsTab = "servers"; model.requestHostEditor = true } }
                    }
                }
            }
        }.formStyle(.grouped)
        .sheet(item: $directories) { selection in AgentDirectoriesView(model: model, provider: selection.provider, machine: selection.machine) }
    }
    private func machine(_ id: String, name: String, agent: AgentConfiguration) -> some View {
        HStack {
            Toggle(name, isOn: Binding(get: { agent.machineIds.contains(id) }, set: { selected in
                var ids = agent.machineIds.filter { $0 != id }; if selected { ids.append(id) }
                Task { await model.configureAgent(agent.provider, machines: ids) }
            })).disabled(!agent.enabled)
            Button("目录设置") { directories = .init(provider: agent.provider, machine: id) }
        }
    }
}

struct AgentDirectoriesView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    let provider: String, machine: String
    @State private var editing: AgentSource?
    @State private var removing: AgentSource?
    @State private var error: String?
    @State private var busy = false
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == provider && ($0.hostId ?? "local") == machine } }
    var body: some View {
        VStack {
            HStack { Text(Format.provider(provider) + " · " + model.machineName(machine)).font(AppFont.title); Spacer(); Button("完成") { dismiss() } }.padding()
            Form {
                ForEach(sources) { source in
                    Section {
                        Text(source.path).textSelection(.enabled)
                        HStack { Button("编辑目录") { editing = source }; Button("移除目录配置", role: .destructive) { removing = source } }
                    }
                }
                Button("添加目录") {
                    var source = AgentSource(provider: provider, path: provider == "codex" ? "~/.codex" : provider == "claude" ? "~/.claude" : provider == "antigravity" ? "~/.gemini/antigravity-cli" : "~/.aieyes")
                    source.hostId = machine == "local" ? nil : machine; editing = source
                }
                Text("只配置采集位置，不移动文件。远程 Shell 位于服务器设置，前置命令位于账户设置。").font(AppFont.secondary).foregroundStyle(.secondary)
                if let error { Text(error).foregroundStyle(Palette.danger).textSelection(.enabled) }
            }.formStyle(.grouped).disabled(busy)
        }.frame(width: 600, height: 480).aieyesAccent().interactiveDismissDisabled(busy)
        .updateOperation("Agent 目录配置", active: busy)
        .sheet(item: $editing) { source in
            SourceEditor(source: source, hosts: model.settings.hosts, appProxy: model.settings.proxy, testURLs: model.settings.proxyTestUrls) { item in
                var params: [String: Any] = ["source": try JSONSerialization.jsonObject(with: JSONEncoder().encode(item))]
                if sources.contains(where: { $0.id == source.id }) { params["baseSource"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(source)) }
                let saved: Settings = try await model.engine.call("sources.configure", params: params)
                model.settings = saved; model.settingsDraft = saved; editing = nil
                Task { await model.reload() }
            }
        }
        .sheet(item: $removing) { source in
            DangerConfirmation(title: "移除目录配置？", explanation: "仅解除此目录的采集和账户连接；文件、登录凭证及历史记录保留。", affected: [source.path], confirmLabel: "移除配置", cancel: { removing = nil }, confirm: {
                removing = nil; busy = true; error = nil
                Task { defer { busy = false }; do {
                    let saved: Settings = try await model.engine.call("sources.remove", params: ["sourceId": source.id, "baseSource": try JSONSerialization.jsonObject(with: JSONEncoder().encode(source))])
                    model.settings = saved; model.settingsDraft = saved; Task { await model.reload() }
                } catch { self.error = error.localizedDescription } }
            })
        }
    }
}

struct AccountsSettingsView: View {
    @ObservedObject var model: AppModel
    var editAccount: (AgentAccount) -> Void
    @State private var adding: AgentConfiguration?
    @State private var selectedAccount: AgentAccount?
    @State private var deleting: AgentAccount?
    @State private var cleanup: [AccountCleanupRecord] = []
    @State private var sourceToManage: AgentSource?
    @State private var authIntent = "login"
    @State private var agySource: AgentSource?
    private var visibleAgents: [AgentConfiguration] {
        model.agentConfigurations.filter { config in
            config.enabled || model.settings.accounts.contains { $0.provider == config.provider && $0.archived != true }
        }
    }
    private var archivedAccounts: [AgentAccount] { model.settings.accounts.filter { $0.archived == true } }
    var body: some View {
        Form {
            statusSection
            ForEach(visibleAgents) { agent in agentSection(agent) }
            archivedSection
            cleanupSection
        }.formStyle(.grouped)
        .task { cleanup = (try? await model.engine.call("accounts.cleanup.list")) ?? []; await model.loadAccountStatuses(refresh: true) }
        .sheet(item: $adding, onDismiss: { Task { await model.loadAccountStatuses(refresh: true) } }) { agent in AccountConnectionView(model: model, initialProvider: agent.provider) }
        .sheet(item: $selectedAccount) { account in AccountDevicesView(model: model, account: account) }
        .sheet(item: $agySource) { source in AntigravityLoginView(model: model, source: source) }
        .sheet(item: $sourceToManage, onDismiss: { Task { await model.loadAccountStatuses(refresh: true) } }) { source in AccountConnectionView(model: model, initialSourceID: source.id, intent: authIntent) }
        .sheet(item: $deleting, onDismiss: { Task { cleanup = (try? await model.engine.call("accounts.cleanup.list")) ?? [] } }) { account in DeleteArchivedAccountView(model: model, account: account) }
    }
    private var statusSection: some View {
        Section {
            HStack {
                Spacer()
                Button(model.accountStatusBusy ? "检查中…" : "刷新设备状态") { Task { await model.loadAccountStatuses(refresh: true) } }.disabled(model.accountStatusBusy)
            }
            Text("一个账户就是一个实际账号，可在多台机器登录。“已登录”表示机器保存了该账号的登录；“使用中”表示当前选用该账号。离线状态以上次检查为准。").font(AppFont.secondary).foregroundStyle(.secondary)
            if let message = model.deploymentSyncMessage { HStack { Text(message); Button("重试同步") { model.synchronizeAccountDeployments() } } }
        }
    }
    private func agentSection(_ agent: AgentConfiguration) -> some View {
        let accounts = model.settings.accounts.filter { $0.provider == agent.provider && $0.archived != true }
        return Section {
            HStack {
                Label { Text(Format.provider(agent.provider)).font(AppFont.section) } icon: { ProviderMark(provider: agent.provider) }
                Spacer()
                Button("添加账户", systemImage: "plus") { adding = agent }.disabled(!agent.enabled)
            }
            ForEach(accounts, id: \.key) { account in
                HStack {
                    VStack(alignment: .leading) { Text(account.name); Text(model.accountCounts(account)).font(AppFont.secondary).foregroundStyle(.secondary)
                        if let identity = model.accountStatuses.first(where: { $0.accountKey == account.key && $0.current == true && $0.identity != nil })?.identity { Text(identity.summary).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled) } }
                    Spacer(); Button("设置") { selectedAccount = account }
                }
            }
            if accounts.isEmpty { Text("尚未添加账户").foregroundStyle(.secondary) }
            Divider()
            ForEach(model.accountMachineIDs(agent.provider), id: \.self) { machine in
                machineRow(agent, machine: machine)
            }
        }
    }
    private func machineRow(_ agent: AgentConfiguration, machine: String) -> some View {
        HStack {
            Text(model.machineName(machine)); Spacer()
            Text(currentAccount(agent.provider, machine: machine)).font(AppFont.secondary).foregroundStyle(.secondary)
            if agent.provider == "antigravity", let source = managementSource(agent.provider, machine: machine) { Button("登录 / 检查") { agySource = source } }
            if agent.provider == "codex", let source = managementSource(agent.provider, machine: machine) {
                let loggedIn = model.accountStatuses.contains { $0.accountKey.hasPrefix("codex:") && $0.sourceId == source.id && ($0.current == true || $0.credential == true) }
                if loggedIn { Button("切换账户") { authIntent = "switch"; sourceToManage = source } }
                else { Button("登录") { authIntent = "login"; sourceToManage = source } }
            }
        }
    }
    @ViewBuilder private var archivedSection: some View {
        if !archivedAccounts.isEmpty {
            Section("已归档账户") {
                ForEach(archivedAccounts, id: \.key) { account in
                    HStack {
                        Text(Format.provider(account.provider) + " · " + account.name); Spacer()
                        Button("恢复") { restore(account) }
                        Button("删除账户", role: .destructive) { deleting = account }
                    }
                }
            }
        }
    }
    @ViewBuilder private var cleanupSection: some View {
        if !cleanup.isEmpty {
            Section("已解除管理的远端任务") {
                ForEach(cleanup) { record in cleanupRecord(record) }
            }
        }
    }
    private func cleanupRecord(_ record: AccountCleanupRecord) -> some View {
        DisclosureGroup {
            ForEach(record.tasks) { task in cleanupTask(task) }
        } label: {
            Text("\(record.name) · 远端可能仍在运行")
        }
    }
    private func cleanupTask(_ task: AccountCleanupTask) -> some View {
        let description: String = "\(task.name) · \(task.machine) · \(task.root ?? "")"
        return Text(description).textSelection(.enabled)
    }
    private func restore(_ account: AgentAccount) {
        Task {
            var next = model.settings
            if let index = next.accounts.firstIndex(where: { $0.key == account.key }) { next.accounts[index].archived = false }
            do { try await model.persistConfiguration(next) }
            catch { model.settingsMessage = error.localizedDescription }
        }
    }
    private func managementSource(_ provider: String, machine: String) -> AgentSource? {
        let sources = model.settings.sources.filter { $0.provider == provider && ($0.hostId ?? "local") == machine && $0.enabled }
        return sources.first { source in model.accountStatuses.contains { $0.sourceId == source.id && ($0.current == true || $0.credential == true) } } ?? sources.first
    }
    private func currentAccount(_ provider: String, machine: String) -> String {
        let names = model.settings.accounts.filter { account in account.provider == provider && model.accountStatuses.contains { $0.accountKey == account.key && $0.machineId == machine && $0.current == true } }.map(\.name)
        if !names.isEmpty { return "正在使用 " + names.joined(separator: "、") }
        let rows = model.accountStatuses.filter { $0.accountKey.hasPrefix(provider + ":") && $0.machineId == machine }
        if let summary = rows.first(where: { $0.identity != nil })?.identitySummary ?? rows.compactMap(\.identitySummary).first { return summary }
        if rows.contains(where: { $0.credential == true }) { return "已登录，尚未选用" }
        return rows.isEmpty || rows.contains(where: { $0.current == nil || $0.credential == nil || $0.error != nil }) ? "登录状态待确认" : "未登录"
    }
}

struct AccountDevicesView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    @State var account: AgentAccount
    @State private var commands: [String: String] = [:]
    @State private var busy = false
    @State private var error: String?
    @State private var loginSource: AgentSource?
    @State private var authIntent = "login"
    @State private var agySource: AgentSource?
    @State private var saveTask: Task<Void, Never>?
    @State private var pendingSaveCount = 0
    @State private var intervalText = ""
    @FocusState private var focusedField: String?
    @AppStorage("panel.accounts.v2") private var panelPreference = "{}"
    private var current: AgentAccount { model.settings.accounts.first { $0.key == account.key } ?? account }
    var body: some View {
        VStack {
            HStack { Text(account.name).font(AppFont.title); Spacer(); Button("完成") { focusedField = nil; guard saveEditedText(force: error != nil) else { return }; Task { await saveTask?.value; if error == nil { dismiss() } } }.disabled(busy) }.padding(20)
            Form {
                Section {
                    Toggle("显示并查询额度", isOn: Binding(get: { account.quotaEnabled }, set: { value in account.quotaEnabled = value; save { $0.quotaEnabled = value } }))
                    if let message = model.dashboard.quotas.first(where: { $0.id == account.key })?.error { Text("额度查询：" + message).font(AppFont.secondary).foregroundStyle(Palette.warn).textSelection(.enabled) }
                    Picker("优先查询位置", selection: Binding(get: { account.quotaSourceId ?? "" }, set: { value in account.quotaSourceId = value.isEmpty ? nil : value; save { $0.quotaSourceId = value.isEmpty ? nil : value } })) {
                        Text("自动 · 优先本机").tag("")
                        ForEach(model.settings.sources.filter { current.uses($0) }) { source in Text(model.machineName(source.hostId ?? "local") + " · " + source.name + (source.enabled ? "" : " · 已暂停")).tag(source.id) }
                    }
                    TextField("查询间隔（秒）", text: $intervalText, prompt: Text("沿用全局 · \(model.settings.refreshSeconds) 秒")).focused($focusedField, equals: "interval")
                    Text("留空沿用全局间隔；自定义范围 30–86400 秒。").font(AppFont.secondary).foregroundStyle(.secondary)
                    TextField("账户名称", text: $account.name).focused($focusedField, equals: "name")
                    Toggle("归档账户", isOn: Binding(get: { account.archived == true }, set: { value in account.archived = value; save { $0.archived = value } }))
                    Toggle("在面板显示", isOn: panelBinding).disabled(!current.quotaEnabled)
                }
                ForEach(model.accountMachineIDs(account.provider, includePaused: true), id: \.self) { machine in
                    Section(model.machineName(machine) + (model.accountMachineIDs(account.provider).contains(machine) ? "" : " · 已暂停")) {
                        Text(model.accountDeviceSummary(current, machine: machine)).font(AppFont.secondary)
                        ForEach(model.settings.sources.filter { $0.provider == account.provider && ($0.hostId ?? "local") == machine }) { source in
                            let status = model.accountStatuses.first { $0.accountKey == account.key && $0.sourceId == source.id }
                            HStack {
                                VStack(alignment: .leading) { Text(source.name); Text(source.path).font(AppFont.secondary).foregroundStyle(.secondary) }
                                Spacer()
                                if account.provider == "antigravity", source.accountId == account.id { Button("登录 / 检查") { agySource = source }.disabled(!source.enabled) }
                                if account.provider == "codex" {
                                    if status?.current == true { Text("正在使用").foregroundStyle(.secondary) }
                                    else if status?.credential == true { Button("切换到此账户") { openLogin(source, intent: "switch") }.disabled(!source.enabled || !model.accountMachineIDs(account.provider).contains(machine)) }
                                    else { Button("登录") { openLogin(source, intent: "login") }.disabled(!source.enabled || !model.accountMachineIDs(account.provider).contains(machine)) }
                                }
                            }
                            if let message = status?.error ?? status?.note, !message.isEmpty { Text(message).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled) }
                            if let time = status?.checkedAt { Text("检查于 " + Format.time(time)).font(AppFont.secondary).foregroundStyle(.secondary) }
                        }
                        if machine != "local" {
                            DisclosureGroup("前置命令") {
                                TextEditor(text: Binding(get: { commands[machine] ?? inheritedCommand(machine) }, set: { commands[machine] = $0 })).font(.system(size: 13, design: .monospaced)).frame(height: 85).focused($focusedField, equals: machine)
                                Text("用于此账户的授权、切换、额度查询及定时唤醒；不用于同步历史或监控。清空后不执行。").font(AppFont.secondary).foregroundStyle(.secondary)
                                if legacyCommands(machine).count > 1 && current.deviceSettings?.contains(where: { $0.machineId == machine }) != true {
                                    Text("旧目录的命令不同，暂时分别保留。编辑完成后统一使用上方命令。").foregroundStyle(Palette.warn)
                                    ForEach(model.settings.sources.filter { $0.provider == account.provider && $0.hostId == machine }) { source in Text(source.name + ": " + legacyCommand(source)).font(.system(size: 12, design: .monospaced)).textSelection(.enabled) }
                                }

                            }
                        }
                    }
                }
                if let error { HStack { Text(error).foregroundStyle(Palette.danger).textSelection(.enabled); Button("重试保存") { saveEditedText(force: true) } } }
                if let message = model.deploymentSyncMessage { HStack { Text(message); Button("重试同步") { model.synchronizeAccountDeployments() } } }
            }.formStyle(.grouped)
        }.frame(width: 680, height: 650).aieyesAccent().interactiveDismissDisabled(busy || error != nil || focusedField != nil)
        .updateDraftGuard("账户设置", snapshot: draftSnapshot(account) + intervalText + draftSnapshot(commands), dirty: account != current || !commands.isEmpty || intervalText != (current.quotaRefreshSeconds.map(String.init) ?? ""), saving: busy, save: { focusedField = nil; guard saveEditedText(force: true) else { return false }; await saveTask?.value; if error == nil { dismiss(); return true }; return false }, discard: { dismiss() })
        .onAppear { intervalText = account.quotaRefreshSeconds.map(String.init) ?? "" }
        .onChange(of: current.name) { old, value in if focusedField != "name" && account.name == old { account.name = value } }
        .onChange(of: focusedField) { old, _ in if old != nil { saveEditedText() } }
        .sheet(item: $agySource) { source in AntigravityLoginView(model: model, source: source) }
        .sheet(item: $loginSource, onDismiss: { Task { await model.loadAccountStatuses(refresh: true) } }) { source in AccountConnectionView(model: model, account: current, initialSourceID: source.id, intent: authIntent) }
    }
    private func openLogin(_ source: AgentSource, intent: String) {
        focusedField = nil
        guard saveEditedText(force: error != nil) else { return }
        Task { await saveTask?.value; if error == nil { authIntent = intent; loginSource = source } }
    }
    @discardableResult
    private func saveEditedText(force: Bool = false) -> Bool {
        let name = account.name.trimmingCharacters(in: .whitespacesAndNewlines)
        let text = intervalText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { error = "请输入账户名称"; return false }
        guard text.isEmpty || Int(text).map({ (30...86400).contains($0) }) == true else { error = "查询间隔范围为 30–86400 秒"; return false }
        let interval = Int(text), edits = commands
        let flagsChanged = account.quotaEnabled != current.quotaEnabled || (account.archived == true) != (current.archived == true) || account.quotaSourceId != current.quotaSourceId
        if !force && !flagsChanged && name == current.name && interval == current.quotaRefreshSeconds && edits.allSatisfy({ machine, command in current.deviceSettings?.first(where: { $0.machineId == machine })?.preCommand == command }) { return true }
        account.quotaRefreshSeconds = interval
        let quotaEnabled = account.quotaEnabled, archived = account.archived, priority = account.quotaSourceId
        save { next in
            next.name = name; next.quotaRefreshSeconds = interval
            if force || flagsChanged { next.quotaEnabled = quotaEnabled; next.archived = archived; next.quotaSourceId = priority }
            for (machine, command) in edits { var values = next.deviceSettings ?? []; values.removeAll { $0.machineId == machine }; values.append(.init(machineId: machine, preCommand: command)); next.deviceSettings = values }
        }
        return true
    }
    private func save(_ change: @escaping (inout AgentAccount) -> Void) {
        let previous = saveTask
        pendingSaveCount += 1; busy = true
        saveTask = Task {
            await previous?.value
            do {
                var next = model.settings
                guard let i = next.accounts.firstIndex(where: { $0.key == account.key }) else { throw ClientError.message("账户已移除") }
                change(&next.accounts[i]); try await model.persistConfiguration(next)
                if account.name.trimmingCharacters(in: .whitespacesAndNewlines) == current.name && Int(intervalText.trimmingCharacters(in: .whitespacesAndNewlines)) == current.quotaRefreshSeconds && account.quotaEnabled == current.quotaEnabled && (account.archived == true) == (current.archived == true) && account.quotaSourceId == current.quotaSourceId && commands.allSatisfy({ machine, command in current.deviceSettings?.first(where: { $0.machineId == machine })?.preCommand == command }) { error = nil }
            } catch { self.error = error.localizedDescription }
            pendingSaveCount -= 1; busy = pendingSaveCount > 0
        }
    }
    private func legacyCommand(_ source: AgentSource) -> String { source.quotaPreCommand.isEmpty ? model.settings.hosts.first { $0.id == source.hostId }?.preCommand ?? "" : source.quotaPreCommand }
    private func legacyCommands(_ machine: String) -> Set<String> { Set(model.settings.sources.filter { $0.provider == account.provider && $0.hostId == machine }.map(legacyCommand)) }
    private func inheritedCommand(_ machine: String) -> String { current.deviceSettings?.first { $0.machineId == machine }?.preCommand ?? (legacyCommands(machine).count == 1 ? legacyCommands(machine).first ?? "" : "") }
    private var panelBinding: Binding<Bool> {
        Binding(get: { (PanelAccountPreference.selections(panelPreference, accounts: model.settings.accounts, order: model.dashboard.quotaOrder ?? [])[account.provider] ?? []).contains(account.key) }, set: { enabled in
            var pref = PanelAccountPreference.decode(panelPreference)
            var keys = (PanelAccountPreference.selections(panelPreference, accounts: model.settings.accounts, order: model.dashboard.quotaOrder ?? [])[account.provider] ?? []).filter { $0 != account.key }
            if enabled { guard keys.count < 5 else { error = "面板最多显示 5 个同类账户，请先调整面板选择。"; return }; keys.append(account.key) }
            pref.providers[account.provider] = .init(mode: "custom", keys: keys)
            panelPreference = String(data: (try? JSONEncoder().encode(pref)) ?? Data(), encoding: .utf8) ?? "{}"
        })
    }
}

struct DeleteArchivedAccountView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    let account: AgentAccount
    @State private var tasks: [AccountCleanupTask] = []
    @State private var failed: [AccountCleanupTask] = []
    @State private var busy = false
    @State private var loaded = false
    @State private var force = false
    @State private var error: String?
    private var visibleTasks: [AccountCleanupTask] { failed.isEmpty ? tasks : failed }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("删除归档账户「" + account.name + "」").font(AppFont.title)
            Text("账户配置与关联任务将移除；历史记录和机器上的凭证保留，不退出 Agent。")
            taskList
            deletionStatus
            deletionControls
        }.padding(24).frame(width: 600, height: 430).interactiveDismissDisabled(busy)
        .updateDraftGuard("删除归档账户", snapshot: "", dirty: false, saving: busy, save: { false }, discard: { dismiss() })
        .task { await loadPreview() }
    }
    private var taskList: some View {
        ScrollView {
            VStack(alignment: .leading) {
                ForEach(visibleTasks) { task in Text(taskDescription(task)) }
            }
        }
    }
    private func taskDescription(_ task: AccountCleanupTask) -> String {
        let location = task.name + " · " + task.machine
        guard let error = task.error else { return location }
        return location + " · " + error
    }
    @ViewBuilder private var deletionStatus: some View {
        if !tasks.isEmpty { Toggle("强制删除（设备已弃用或无法连接）", isOn: $force).disabled(busy) }
        if force { Text("无法保证远端任务停止。强制删除后将解除本地管理，不再自动重试；未清理设备和部署位置会保留供查看。").foregroundStyle(Palette.warn) }
        if let error { Text(error).foregroundStyle(Palette.warn) }
        if busy { ProgressView("正在清理关联任务…") }
    }
    private var deletionControls: some View {
        HStack {
            Button("取消") { dismiss() }.disabled(busy)
            Spacer()
            Button(force ? "确认强制删除" : "停止任务并删除账户", role: .destructive) { remove() }.disabled(busy || !loaded)
        }
    }
    private func loadPreview() async {
        do {
            let result: AccountDeletionPreview = try await model.engine.call("accounts.deletion.preview", params: ["accountKey": account.key])
            tasks = result.tasks
            loaded = true
        } catch { self.error = error.localizedDescription }
    }
    private func remove() {
        busy = true; error = nil
        Task { defer { busy = false }; do {
            let result: AccountCleanupResult = try await model.accountEngine.call("accounts.delete", params: ["accountKey":account.key,"force":force])
            if result.deleted == true, let saved = result.settings { model.acceptConfiguration(saved); dismiss() }
            else { failed = result.failedTasks ?? []; error = "部分任务未清理，账户仍保留。可重试，或确认强制删除。" }
        } catch { self.error = error.localizedDescription } }
    }
}

struct AccountConnectionView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var agySource: AgentSource?
    @ObservedObject var model: AppModel
    var account: AgentAccount?
    var initialProvider: String? = nil
    var initialSourceID: String? = nil
    var intent = "login"
    @State private var provider = ""
    @State private var selection = Set<String>()
    @State private var apiKey = ""
    @State private var preparedKey: (key: String, path: String)?
    @State private var busy = false
    @State private var error: String?
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == provider && $0.enabled && ($0.accountId.isEmpty || $0.accountId == (account?.id ?? "")) } }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text(intent == "switch" ? "切换账户" : account == nil ? "添加账户" : "连接机器").font(AppFont.title); Spacer(); Button("完成") { dismiss() }.disabled(busy) }.padding(.horizontal, 20).padding(.top, 20)
            Text(Format.provider(provider)).font(AppFont.section).padding(.horizontal, 20)
            if provider == "codex" { CodexAccountsView(model: model, targetAccountID: account?.id, initialSourceID: initialSourceID, intent: intent, onBusyChanged: { busy = $0 }) }
            else {
                Form {
                    if provider == "deepseek" {
                        SecureField("API Key", text: $apiKey)
                        Text("Key 保存在本机，只用于查询账户余额。").foregroundStyle(.secondary)
                    } else {
                        Section("已有登录的机器 · 可多选") {
                            ForEach(sources) { source in HStack { Toggle(source.name, isOn: Binding(get: { selection.contains(source.id) }, set: { if $0 { selection.insert(source.id) } else { selection.remove(source.id) } })); if provider == "antigravity" { Button("登录 / 检查") { agySource = source } } } }
                            if sources.isEmpty { Text("没有可用位置，请先在 Agents 选择机器；同一目录只能连接一个当前登录账户。").foregroundStyle(.secondary) }
                        }
                    }
                    if account == nil { Text("添加后可修改账号的显示名称。").foregroundStyle(.secondary) }
                    if let error { Text(error).foregroundStyle(Palette.warn).textSelection(.enabled) }
                    Button(busy ? "保存中…" : account == nil ? "添加账号" : "保存连接") { Task { await save() } }.buttonStyle(.borderedProminent).disabled(busy)
                }.formStyle(.grouped).disabled(busy)
            }
        }.frame(width: 640, height: 620).interactiveDismissDisabled(busy)
        .updateDraftGuard("账户连接", snapshot: provider + draftSnapshot(selection.sorted()) + apiKey, dirty: !selection.isEmpty || !apiKey.isEmpty, saving: busy, save: { await save(); return error == nil }, discard: { dismiss() })
        .sheet(item: $agySource) { source in AntigravityLoginView(model: model, source: source) }
        .onAppear { provider = account?.provider ?? initialProvider ?? (initialSourceID != nil ? "codex" : model.agentConfigurations.first(where: { $0.enabled })?.provider ?? "") }
        .onChange(of: provider) { _, _ in selection = []; error = nil }
    }
    private func save() async {
        busy = true; error = nil; defer { busy = false }
        do {
            let target = account?.id ?? ""
            let name = account?.name ?? Format.provider(provider) + " 账户"
            if provider == "deepseek" {
                guard !apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw ClientError.message("请输入 API Key") }
                var next = model.settings
                let id = target.isEmpty ? UUID().uuidString : target
                if target.isEmpty { next.accounts.append(AgentAccount(id: id, name: name, provider: provider)) }
                let source = next.sources.first { $0.provider == provider && $0.accountId == id } ?? AgentSource(name: name, provider: provider, accountId: id, path: "")
                if !next.sources.contains(where: { $0.id == source.id }) { next.sources.append(source) }
                if preparedKey?.key != apiKey {
                    let result: [String: String] = try await model.engine.call("credentials.save", params: ["sourceId":source.id + "-" + UUID().uuidString, "apiKey":apiKey])
                    guard let path = result["path"], !path.isEmpty else { throw ClientError.message("凭据保存失败") }
                    preparedKey = (apiKey, path)
                }
                if let index = next.sources.firstIndex(where: { $0.id == source.id }) { next.sources[index].path = preparedKey?.path ?? "" }
                try await model.persistConfiguration(next)
            } else {
                struct Result: Decodable { var settings: Settings }
                let result: Result = try await model.engine.call("accounts.connect", params: ["provider": provider, "accountId": target, "name": name, "sourceIds": Array(selection)])
                model.acceptConfiguration(result.settings)
            }
            dismiss()
        } catch { self.error = error.localizedDescription }
    }
}
