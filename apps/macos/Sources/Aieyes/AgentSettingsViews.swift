import SwiftUI

struct AgentDirectorySelection: Identifiable {
    let provider: String, machine: String
    var id: String { provider + ":" + machine }
}
struct AgentsSettingsView: View {
    @ObservedObject var model: AppModel
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            SettingsSidebar(title: "选择 Agent", items: model.agentConfigurations.map { ($0.provider, Format.provider($0.provider)) }, selection: $model.settingsAgent)
            AccountsSettingsView(model: model, provider: model.settingsAgent) { model.requestedAccountKey = $0.key }
                .id(model.settingsAgent)
        }.padding(.top, 12)
    }
}

struct AgentMachinesSection: View {
    @ObservedObject var model: AppModel
    let agent: AgentConfiguration
    @State private var directories: AgentDirectorySelection?
    @State private var hooks: AgentDirectorySelection?
    @State private var sourceToManage: AgentSource?
    @State private var agySource: AgentSource?
    @State private var authIntent = "login"
    private var unboundMachines: [String] { Array(Set(model.settings.sources.filter { $0.provider == agent.provider && $0.unbound }.compactMap(\.hostId))).filter { id in !model.settings.hosts.contains { $0.id == id } }.sorted() }
    var body: some View {
        Section("机器与目录 · 可多选") {
            machine("local", name: "本机", available: model.settings.localDeviceEnabled != false)
            ForEach(model.settings.hosts) { host in machine(host.id, name: host.name.isEmpty ? host.target : host.name, available: host.enabled) }
            ForEach(unboundMachines, id: \.self) { id in
                machine(id, name: (model.settings.sources.first { $0.hostId == id }?.lastMachineName ?? id) + " · 机器未绑定", available: false, unbound: true)
            }
            if model.settings.localDeviceEnabled == false && !model.settings.hosts.contains(where: \.enabled) {
                Text("暂无可采集机器，请在服务器设置开启 Agent 采集。").foregroundStyle(.secondary)
            } else if !agent.machineIds.contains(where: { id in id == "local" ? model.settings.localDeviceEnabled != false : model.settings.hosts.contains { $0.id == id && $0.enabled } }) {
                Text("未选择机器，暂不采集。").foregroundStyle(.secondary)
            }
            Button("添加 SSH 主机") { model.openSettings(.init(category: "servers", section: "add-host")) }
        }.id("machines")
        .sheet(item: $directories) { selection in AgentDirectoriesView(model: model, provider: selection.provider, machine: selection.machine) }
        .sheet(item: $hooks) { selection in MachineSessionHooksView(model: model, provider: selection.provider, machine: selection.machine) }
        .sheet(item: $sourceToManage) { source in AccountConnectionView(model: model, initialSourceID: source.id, intent: authIntent) }
        .sheet(item: $agySource) { source in AntigravityLoginView(model: model, source: source) }
    }
    private func machine(_ id: String, name: String, available: Bool, unbound: Bool = false) -> some View {
        let sources = model.settings.sources.filter { $0.provider == agent.provider && ($0.hostId ?? "local") == id }
        let enabled = sources.filter { $0.autoValuation?.enabled == true }.count
        return HStack(spacing: 10) {
            Toggle(name + (available || unbound ? "" : " · 采集已暂停"), isOn: Binding(get: { !unbound && agent.machineIds.contains(id) }, set: { selected in
                var ids = agent.machineIds.filter { $0 != id }; if selected { ids.append(id) }
                Task { await model.configureAgent(agent.provider, machines: ids) }
            })).toggleStyle(.checkbox).disabled(!agent.enabled || unbound || !available).lineLimit(2).help(name)
            Spacer(minLength: 4)
            Button("目录 \(sources.count)") { directories = .init(provider: agent.provider, machine: id) }
            if !unbound { MachineSessionStatus(live: model.live, provider: agent.provider, sources: sources, collecting: sources.contains { model.settings.sourceEnabled($0) }) { hooks = .init(provider: agent.provider, machine: id) } }
            Text(unbound || !available || !agent.enabled || !agent.machineIds.contains(id) || !sources.contains(where: { model.settings.sourceEnabled($0) }) ? "估值暂停" : enabled == 0 ? "估值已关闭" : "估值 \(enabled)/\(sources.count)").font(AppFont.secondary).foregroundStyle(.secondary).fixedSize()
            if !unbound {
                Menu {
                    ForEach(sources.filter { model.settings.sourceEnabled($0) }) { source in
                        if agent.provider == "codex" {
                            Button("登录 · " + source.displayLocation) { authIntent = "login"; sourceToManage = source }.help(source.path)
                            Button("切换账户 · " + source.displayLocation) { authIntent = "switch"; sourceToManage = source }.help(source.path)
                        }
                        if agent.provider == "antigravity" { Button("登录 / 检查 · " + source.displayLocation) { agySource = source }.help(source.path) }
                    }
                    if !sources.contains(where: { model.settings.sourceEnabled($0) }) { Text("登录与切换需要启用 Agent、选择机器并开启目录采集") }
                    Button("机器与采集设置") { model.openSettings(.init(category: "servers")) }
                    Button(available ? "暂停 Agent 采集" : "恢复 Agent 采集") { Task { await model.setDeviceEnabled(id, enabled: !available) } }
                } label: { Image(systemName: "ellipsis") }.menuStyle(.borderlessButton).fixedSize().accessibilityLabel(name + "操作")
            }
        }.padding(.vertical, 3)
    }
}
private struct MachineSessionStatus: View {
    @ObservedObject var live: LiveSessionState
    let provider: String, sources: [AgentSource]
    let collecting: Bool
    var open: () -> Void
    private var label: String {
        if sources.isEmpty { return "会话未接入" }
        if !collecting { return "采集已暂停" }
        let rows = live.sources.filter { row in sources.contains { $0.id == row.sourceId } }
        if provider == "codex" { return rows.count == sources.count && rows.allSatisfy { $0.state == "healthy" } ? "日志接入" : "日志待确认" }
        if rows.contains(where: { $0.hooks == "error" }) { return "接入失败" }
        if rows.contains(where: { $0.hooks == "disabled" }) { return "接入已停用" }
        return rows.count == sources.count && rows.allSatisfy { $0.hooks == "installed" } ? "会话已接入" : "等待自动接入"
    }
    var body: some View {
        if ["codex", "claude", "antigravity"].contains(provider) { Button(label, action: open).disabled(sources.isEmpty).font(AppFont.secondary) }
    }
}
private struct MachineSessionHooksView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    let provider: String, machine: String
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack { Text("会话状态接入 · " + model.machineName(machine)).font(AppFont.title); Spacer(); Button("完成") { dismiss() } }
            if provider == "codex" {
                Text("通过 Codex 会话日志读取状态，无需部署 Hook。").foregroundStyle(.secondary)
            } else {
                Text(machine == "local" ? "采集时自动接入观察 Hook；停用后继续读取日志。" : "通过 SSH 自动接入观察 Hook，需要远端 Python 3；停用后继续读取日志。").foregroundStyle(.secondary)
                ScrollView { LazyVStack(alignment: .leading, spacing: 16) {
                    ForEach(model.settings.sources.filter { $0.provider == provider && ($0.hostId ?? "local") == machine && !$0.unbound }) { source in SessionHookControls(model: model, source: source) }
                } }
            }
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogHooksWidth, height: provider == "codex" ? 180 : 400).aieyesAccent()
    }
}

struct AgentAutomaticValuationView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    let provider: String, machine: String
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == provider && ($0.hostId ?? "local") == machine } }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text(Format.provider(provider) + " · " + model.machineName(machine)).font(AppFont.title)
                Spacer()
                Button("完成") { dismiss() }.disabled(model.automaticSaving)
            }
            ScrollView { AutomaticValuationControls(model: model, sources: sources).frame(maxWidth: .infinity, alignment: .leading) }
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogAutomaticWidth, height: 440).aieyesAccent()
            .interactiveDismissDisabled(model.automaticSaving)
            .updateOperation("自动估值设置", active: model.automaticSaving)
    }
}

struct AgentDirectoriesView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    let provider: String, machine: String
    @State private var editing: AgentSource?
    @State private var removing: AgentSource?
    @State private var rebinding: AgentSource?
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
                        HStack {
                            if source.unbound { Text("机器未绑定").foregroundStyle(Palette.warn); Button("重新绑定机器") { rebinding = source } }
                            else { Button("编辑目录") { editing = source } }
                            Button("移除目录配置", role: .destructive) { removing = source }
                        }
                    }
                }
                Button("添加目录") {
                    var source = AgentSource(provider: provider, path: CoreContract.defaultPaths[provider] ?? "")
                    source.hostId = machine == "local" ? nil : machine; editing = source
                }.disabled(machine != "local" && !model.settings.hosts.contains { $0.id == machine })
                Text("只配置采集位置，不移动文件。远程 Shell 位于服务器设置，前置命令位于账户设置。").font(AppFont.secondary).foregroundStyle(.secondary)
                if let error { Text(error).foregroundStyle(Palette.danger).textSelection(.enabled) }
            }.formStyle(.grouped).disabled(busy)
        }.frame(width: 600, height: 480).aieyesAccent().interactiveDismissDisabled(busy)
        .updateOperation("Agent 目录配置", active: busy)
        .sheet(item: $rebinding) { source in RebindSourceView(model: model, source: source) }
        .sheet(item: $editing) { source in
            SourceEditor(source: source, hosts: model.settings.hosts, appProxy: model.settings.proxy, testURLs: model.settings.proxyTestUrls) { item in
                var params: [String: Any] = ["source": try JSONSerialization.jsonObject(with: JSONEncoder().encode(item))]
                if sources.contains(where: { $0.id == source.id }) { params["baseSource"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(source)) }
                let saved: Settings = try await model.engine.call("sources.configure", params: params)
                model.acceptConfiguration(saved); editing = nil
                Task { await model.reload() }
            }
        }
        .sheet(item: $removing) { source in
            DangerConfirmation(title: "移除目录配置？", explanation: "仅解除此目录的采集和账户连接；文件、登录凭证及历史记录保留。", affected: [source.path], confirmLabel: "移除配置", cancel: { removing = nil }, confirm: {
                removing = nil; busy = true; error = nil
                Task { defer { busy = false }; do {
                    let saved: Settings = try await model.engine.call("sources.remove", params: ["sourceId": source.id, "baseSource": try JSONSerialization.jsonObject(with: JSONEncoder().encode(source))])
                    model.acceptConfiguration(saved); Task { await model.reload() }
                } catch { self.error = error.localizedDescription } }
            })
        }
    }
}

struct AccountsSettingsView: View {
    @ObservedObject var model: AppModel
    var provider: String? = nil
    var editAccount: (AgentAccount) -> Void
    @State private var showingValuation = false
    @State private var adding: AgentConfiguration?
    @State private var selectedAccount: AgentAccount?
    @State private var deleting: AgentAccount?
    @State private var cleanup: [AccountCleanupRecord] = []
    @State private var sourceToManage: AgentSource?
    @State private var authIntent = "login"
    @State private var agySource: AgentSource?
    private var visibleAgents: [AgentConfiguration] {
        model.agentConfigurations.filter { config in
            provider.map { config.provider == $0 } ?? (config.enabled || model.settings.accounts.contains { $0.provider == config.provider && $0.archived != true })
        }
    }
    private var archivedAccounts: [AgentAccount] { model.settings.accounts.filter { $0.archived == true && (provider == nil || $0.provider == provider) } }
    var body: some View {
        ScrollViewReader { reader in
            Form {
                if let provider, let agent = visibleAgents.first(where: { $0.provider == provider }) {
                    Section {
                        HStack {
                            Label { Text(Format.provider(provider)).font(AppFont.section) } icon: { ProviderMark(provider: provider) }.fixedSize()
                            Spacer()
                            Text("启用").fixedSize()
                            Toggle("启用 " + Format.provider(provider), isOn: Binding(get: { agent.enabled }, set: { value in Task { await model.configureAgent(provider, enabled: value, machines: value && agent.machineIds.isEmpty ? ["local"] : nil) } })).labelsHidden().toggleStyle(.switch).fixedSize()
                        }
                        if provider == "deepseek" { Text("添加 API Key，通过本机查询账户余额。").font(AppFont.secondary).foregroundStyle(.secondary) }
                        else { Button("估值与采样") { showingValuation = true } }
                    }
                }
                if let provider, let agent = visibleAgents.first(where: { $0.provider == provider }), provider != "deepseek" { AgentMachinesSection(model: model, agent: agent) }
                if let message = model.deploymentSyncMessage { HStack { Text(message); Button("重试同步") { model.synchronizeAccountDeployments() } } }
                ForEach(visibleAgents) { agent in agentSection(agent) }
                if provider == nil || provider == "codex" { CodexAPISettingsSection(model: model) }
                archivedSection
                cleanupSection
            }.formStyle(.grouped)
            .onAppear { if let section = model.settingsSection { reader.scrollTo(section, anchor: .top) } }
            .onChange(of: model.settingsSection) { _, section in if let section { reader.scrollTo(section, anchor: .top) } }
        }
        .sheet(isPresented: $showingValuation) { if let provider { AgentValuationView(model: model, provider: provider) } }
        .task { cleanup = (try? await model.engine.call("accounts.cleanup.list")) ?? []; await model.loadAccountStatuses(refresh: true) }
        .sheet(item: $adding, onDismiss: { Task { await model.loadAccountStatuses(refresh: true) } }) { agent in AccountConnectionView(model: model, initialProvider: agent.provider) }
        .sheet(item: $selectedAccount) { account in AccountDevicesView(model: model, account: account) }
        .sheet(item: $agySource) { source in AntigravityLoginView(model: model, source: source) }
        .sheet(item: $sourceToManage, onDismiss: { Task { await model.loadAccountStatuses(refresh: true) } }) { source in AccountConnectionView(model: model, initialSourceID: source.id, intent: authIntent) }
        .sheet(item: $deleting, onDismiss: { Task { cleanup = (try? await model.engine.call("accounts.cleanup.list")) ?? [] } }) { account in DeleteArchivedAccountView(model: model, account: account) }
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
                    AccountStatusSummary(model: model, account: account)
                    Spacer(); Button("设置") { selectedAccount = account }
                }
            }
            if accounts.isEmpty { Text("尚未添加账户").foregroundStyle(.secondary) }
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
        let sources = model.settings.sources.filter { $0.provider == provider && ($0.hostId ?? "local") == machine && model.settings.sourceEnabled($0) }
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

private struct AccountStatusSummary: View {
    @ObservedObject var model: AppModel
    @ObservedObject var status: AccountStatusProjection
    let account: AgentAccount
    init(model: AppModel, account: AgentAccount) {
        self.model = model; self.account = account; status = model.accountStatusStore.projection(account.key)
    }
    var body: some View {
        VStack(alignment: .leading) {
            Text(account.name)
            Text(model.accountDeviceCounts(account.key).inlineSummary).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1).help(model.accountCounts(account))
            if let identity = status.rows.first(where: { $0.current == true && $0.identity != nil })?.identity {
                Text(identity.summary).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled)
            }
        }
    }
}

struct AccountDevicesView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    @ObservedObject private var statusProjection: AccountStatusProjection
    @State var account: AgentAccount
    init(model: AppModel, account: AgentAccount) {
        self.model = model; _account = State(initialValue: account); statusProjection = model.accountStatusStore.projection(account.key)
    }
    @State private var commands: [String: String] = [:]
    @State private var busy = false
    @State private var error: String?
    @State private var loginSource: AgentSource?
    @State private var authIntent = "login"
    @State private var agySource: AgentSource?
    @State private var saveTask: Task<Void, Never>?
    @State private var pendingSaveCount = 0
    @State private var intervalText = ""
    @State private var showingConnection = false
    @FocusState private var focusedField: String?
    @AppStorage(NativePanelKey.accounts.rawValue) private var panelPreference = "{}"
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
                        ForEach(model.settings.sources.filter { current.uses($0) }) { source in Text(model.machineName(source.hostId ?? "local") + " · " + source.displayLocation + (model.settings.sourceEnabled(source) ? "" : " · 已暂停")).tag(source.id) }
                    }
                    TextField("查询间隔（秒）", text: $intervalText, prompt: Text("沿用全局 · \(model.settings.refreshSeconds) 秒")).focused($focusedField, equals: "interval")
                    Text("留空沿用全局间隔；自定义范围 5–86400 秒。").font(AppFont.secondary).foregroundStyle(.secondary)
                    TextField("账户名称", text: $account.name).focused($focusedField, equals: "name")
                    Toggle("归档账户", isOn: Binding(get: { account.archived == true }, set: { value in account.archived = value; save { $0.archived = value } }))
                    Toggle("在面板显示", isOn: panelBinding).disabled(!current.quotaEnabled)
                    Text("面板已选择 \(panelSelectionCount) 个账户；手动选择不限数量。").font(AppFont.secondary).foregroundStyle(.secondary)
                    if !PanelAccountPreference.automatic(panelPreference, provider: account.provider) {
                        Button("恢复自动选择") { var pref = PanelAccountPreference.decode(panelPreference); pref.providers[account.provider] = .init(); panelPreference = String(data: (try? JSONEncoder().encode(pref)) ?? Data(), encoding: .utf8) ?? "{}" }
                    }
                    if account.provider != "codex" {
                        Button(account.provider == "deepseek" ? "更新 API Key" : "连接机器") {
                            focusedField = nil
                            guard saveEditedText(force: error != nil) else { return }
                            Task { await saveTask?.value; if error == nil { showingConnection = true } }
                        }
                    }
                }
                ForEach(model.accountMachineIDs(account.provider, includePaused: true), id: \.self) { machine in
                    Section(model.machineName(machine) + (model.accountMachineIDs(account.provider).contains(machine) ? "" : " · 已暂停")) {
                        Text(model.accountDeviceSummary(current, machine: machine)).font(AppFont.secondary)
                        ForEach(model.settings.sources.filter { $0.provider == account.provider && ($0.hostId ?? "local") == machine }) { source in
                            let status = model.accountStatuses.first { $0.accountKey == account.key && $0.sourceId == source.id }
                            HStack {
                                VStack(alignment: .leading) { Text(source.name); Text(source.path).font(AppFont.secondary).foregroundStyle(.secondary) }
                                Spacer()
                                if account.provider == "antigravity", source.accountId == account.id { Button("登录 / 检查") { agySource = source }.disabled(!model.settings.sourceEnabled(source)) }
                                if account.provider == "codex" {
                                    if status?.current == true { Text("正在使用").foregroundStyle(.secondary) }
                                    else if status?.credential == true { Button("切换到此账户") { openLogin(source, intent: "switch") }.disabled(!model.settings.sourceEnabled(source)) }
                                    else { Button("登录") { openLogin(source, intent: "login") }.disabled(!model.settings.sourceEnabled(source)) }
                                }
                            }
                            if let message = status?.error ?? status?.note, !message.isEmpty { Text(message).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled) }
                            if let time = status?.checkedAt { Text("检查于 " + Format.time(time)).font(AppFont.secondary).foregroundStyle(.secondary) }
                        }
                        if machine != "local" {
                            DisclosureGroup("前置命令") {
                                TextEditor(text: Binding(get: { commands[machine] ?? inheritedCommand(machine) }, set: { commands[machine] = $0 })).font(.system(size: DesignTokens.fontEntry, design: .monospaced)).frame(height: 85).focused($focusedField, equals: machine)
                                Text("用于此账户的授权、切换、额度查询及定时唤醒；不用于同步历史或监控。清空后不执行。").font(AppFont.secondary).foregroundStyle(.secondary)
                                if current.deviceSettings?.first(where: { $0.machineId == machine })?.preCommandResetRequired == true {
                                    Text("此机器的旧目录前置命令不一致，已清空，请重新设置。").foregroundStyle(Palette.warn)
                                    Button("保存此机器的前置命令") { commands[machine] = commands[machine] ?? ""; saveEditedText(force: true) }
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
        .sheet(isPresented: $showingConnection, onDismiss: { Task { await model.loadAccountStatuses(refresh: true) } }) { AccountConnectionView(model: model, account: current) }
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
        guard text.isEmpty || Int(text).map({ (5...86400).contains($0) }) == true else { error = "查询间隔范围为 5–86400 秒"; return false }
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
    private var panelSelectionCount: Int { PanelAccountPreference.selections(panelPreference, accounts: model.settings.accounts, order: model.dashboard.quotaOrder ?? [])[account.provider]?.count ?? 0 }
    private var panelBinding: Binding<Bool> {
        Binding(get: { (PanelAccountPreference.selections(panelPreference, accounts: model.settings.accounts, order: model.dashboard.quotaOrder ?? [])[account.provider] ?? []).contains(account.key) }, set: { enabled in
            var pref = PanelAccountPreference.decode(panelPreference)
            let selected = pref.providers[account.provider]?.mode == "custom" ? pref.providers[account.provider]?.keys ?? [] : PanelAccountPreference.selections(panelPreference, accounts: model.settings.accounts, order: model.dashboard.quotaOrder ?? [])[account.provider] ?? []
            var keys = selected.filter { $0 != account.key }
            if enabled { keys.append(account.key) }
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
        }.padding(DesignTokens.dialogPadding).frame(width: 600, height: 430).interactiveDismissDisabled(busy)
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
    @State private var useLocalLogin = false
    @State private var subscriptionMode = ""
    @State private var workspaceID = ""
    @State private var preparedKey: (key: String, path: String)?
    @State private var busy = false
    @State private var error: String?
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == provider && model.settings.sourceEnabled($0) && ($0.accountId.isEmpty || $0.accountId == (account?.id ?? "")) } }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text(intent == "switch" ? "切换账户" : account == nil ? "添加账户" : provider == "deepseek" ? "更新 API Key" : "连接机器").font(AppFont.title); Spacer(); Button("完成") { dismiss() }.disabled(busy) }.padding(.horizontal, 20).padding(.top, 20)
            Text(Format.provider(provider)).font(AppFont.section).padding(.horizontal, 20)
            if provider == "codex" { CodexAccountsView(model: model, targetAccountID: account?.id, initialSourceID: initialSourceID, intent: intent, onBusyChanged: { busy = $0 }) }
            else {
                Form {
                    if let modes = CoreContract.subscriptionModes[provider] {
                        Text(CoreContract.subscriptionHints[provider] ?? "").foregroundStyle(.secondary)
                        Button("打开官方登录 / 用量页") { Task { do { let _: [String: Bool] = try await model.engine.call("subscriptions.openLogin", params: ["provider": provider, "mode": subscriptionMode]) } catch { self.error = error.localizedDescription } } }
                        Picker("订阅 / 地区", selection: $subscriptionMode) { ForEach(modes.keys.sorted(), id: \.self) { key in Text(modes[key] ?? key).tag(key) } }
                        TextField("Workspace / Team ID（仅 Console 或 xAI API）", text: $workspaceID)
                        SecureField("API Key / 登录令牌 / Cookie", text: $apiKey)
                        if provider == "grok" || provider == "opencode" { Toggle("读取本机已登录凭据（仅 Grok 订阅 / OpenCode Go）", isOn: $useLocalLogin) }
                        Text("验证成功后保存到系统凭据库；不会切换客户端账户。").foregroundStyle(.secondary)
                    } else if provider == "deepseek" {
                        SecureField("API Key", text: $apiKey)
                        Text("Key 保存在本机，只用于查询账户余额。").foregroundStyle(.secondary)
                    } else {
                        Section("已有登录的机器 · 可多选") {
                            ForEach(sources) { source in HStack { Toggle(source.displayLocation, isOn: Binding(get: { selection.contains(source.id) }, set: { if $0 { selection.insert(source.id) } else { selection.remove(source.id) } })); if provider == "antigravity" { Button("登录 / 检查") { agySource = source } } } }
                            if sources.isEmpty { Text("没有可用位置。请启用 Agent、选择机器并开启目录采集；同一目录只能连接一个当前登录账户。").foregroundStyle(.secondary); Button("机器与采集设置") { model.openSettings(.init(provider: provider, section: "machines")); dismiss() } }
                        }
                    }
                    if account == nil { Text("添加后可修改账号的显示名称。").foregroundStyle(.secondary) }
                    if let error { Text(error).foregroundStyle(Palette.warn).textSelection(.enabled) }
                    Button(busy ? "保存中…" : account == nil ? "添加账号" : provider == "deepseek" ? "更新 API Key" : "保存连接") { Task { await save() } }.buttonStyle(.borderedProminent).disabled(busy)
                }.formStyle(.grouped).disabled(busy)
            }
        }.frame(width: DesignTokens.dialogAccountWidth, height: 620).interactiveDismissDisabled(busy)
        .updateDraftGuard("账户连接", snapshot: provider + draftSnapshot(selection.sorted()) + apiKey + subscriptionMode + workspaceID + String(useLocalLogin), dirty: !selection.isEmpty || !apiKey.isEmpty || useLocalLogin || !workspaceID.isEmpty, saving: busy, save: { await save(); return error == nil }, discard: { dismiss() })
        .sheet(item: $agySource) { source in AntigravityLoginView(model: model, source: source) }
        .onAppear { provider = account?.provider ?? initialProvider ?? (initialSourceID != nil ? "codex" : model.agentConfigurations.first(where: { $0.enabled })?.provider ?? "") }
        .onChange(of: provider) { _, _ in
            selection = []; error = nil
            let existing = model.settings.sources.first { $0.provider == provider && $0.accountId == account?.id && $0.subscription != nil }?.subscription
            subscriptionMode = existing?.mode ?? (CoreContract.subscriptionModes[provider]?["global"] != nil ? "global" : provider == "grok" ? "subscription" : provider == "opencode" ? "go" : "session")
            workspaceID = existing?.workspaceId ?? ""
        }
    }
    private func save() async {
        busy = true; error = nil; defer { busy = false }
        do {
            let target = account?.id ?? ""
            let name = account?.name ?? Format.provider(provider) + " 账户"
            if CoreContract.subscriptionModes[provider] != nil {
                struct Result: Decodable { var settings: Settings }
                let result: Result = try await model.engine.call("subscriptions.connect", params: ["provider": provider, "accountId": target, "name": name, "secret": apiKey, "useLocalLogin": useLocalLogin, "subscription": ["mode": subscriptionMode, "workspaceId": workspaceID]])
                model.acceptConfiguration(result.settings)
            } else if provider == "deepseek" {
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

private struct SessionHookControls: View {
    @ObservedObject var model: AppModel
    let source: AgentSource
    @State private var status: SessionHookStatus?
    @State private var busy = false
    @State private var failure: String?
    private func perform(_ action: String) async {
        guard !busy else { return }; busy = true
        defer { busy = false }
        do { status = try await model.manageSessionHooks(sourceID: source.id, action: action).first; failure = nil }
        catch { failure = error.localizedDescription }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(source.path).font(AppFont.secondary).textSelection(.enabled)
            HStack {
                Toggle("自动接入", isOn: Binding(get: { status.map { $0.state != "disabled" } ?? false }, set: { enabled in Task { await perform(enabled ? "repair" : "remove") } })).toggleStyle(.switch).disabled(busy || status == nil)
                Text(status?.label ?? "正在读取…").foregroundStyle(.secondary)
                Spacer()
                Button("修复接入") { Task { await perform("repair") } }.disabled(busy)
            }
            if let message = failure ?? status?.error { HStack { Text(message).font(AppFont.secondary).foregroundStyle(Palette.danger); Button("重试") { Task { await perform("status") } } } }
        }.task(id: source.id + source.path) { await perform("status") }
    }
}
private struct RebindSourceView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    let source: AgentSource
    @State private var machine = ""
    @State private var busy = false
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("重新绑定机器").font(AppFont.title)
            Text(source.path).textSelection(.enabled)
            Text("保留原目录、账户关联和历史；重新绑定后从目标机器采集。").foregroundStyle(.secondary)
            Picker("目标机器", selection: $machine) {
                Text("请选择机器").tag(""); Text("本机").tag("local")
                ForEach(model.settings.hosts) { host in Text(host.name.isEmpty ? host.target : host.name).tag(host.id) }
            }
            if let error { Text(error).foregroundStyle(Palette.danger) }
            HStack { Spacer(); Button("取消") { dismiss() }; Button(busy ? "验证中…" : "绑定") { Task { await save() } }.disabled(machine.isEmpty) }.disabled(busy)
        }.padding(DesignTokens.dialogPadding).frame(width: 540).interactiveDismissDisabled(busy).aieyesAccent()
    }
    private func save() async {
        busy = true; error = nil; defer { busy = false }
        do {
            let saved: Settings = try await model.engine.call("sources.rebind", params: ["sourceId":source.id,"machineId":machine,"baseSource":try JSONSerialization.jsonObject(with: JSONEncoder().encode(source))])
            model.acceptConfiguration(saved); dismiss(); Task { await model.reload() }
        } catch { self.error = error.localizedDescription }
    }
}
