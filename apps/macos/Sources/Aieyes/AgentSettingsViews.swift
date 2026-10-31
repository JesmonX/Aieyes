import SwiftUI

struct AgentDirectorySelection: Identifiable {
    let provider: String, machine: String
    var id: String { provider + ":" + machine }
}
struct AgentsSettingsView: View {
    @SurfaceModel var model: AppModel
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 12) {
            AgentEntrySelector(model: model)
            if model.settingsAgentIDs.isEmpty && model.requestedAccountKey == nil {
                Text(L("添加要接入的 Agent")).foregroundStyle(.secondary)
            } else {
                AccountsSettingsView(model: model, provider: model.settingsAgent) { model.requestedAccountKey = $0.key }.id(model.settingsAgent)
            }
        }.padding(.top, 12)
    }
}

private struct AgentEntrySelector: View {
    @SurfaceModel var model: AppModel
    @State private var selecting = false
    @State private var managing = false
    @State private var pickerSearch = ""
    @State private var managementSearch = ""
    private var entries: [AgentConfiguration] { model.settingsAgentIDs.compactMap { id in model.agentConfigurations.first { $0.provider == id } } }
    private func matches(_ agent: AgentConfiguration, _ query: String) -> Bool {
        let text = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty || Format.provider(agent.provider).localizedCaseInsensitiveContains(text) || agent.provider.localizedCaseInsensitiveContains(text)
    }
    private func move(_ provider: String, by offset: Int) {
        var ids = model.settingsAgentIDs
        guard let index = ids.firstIndex(of: provider), ids.indices.contains(index + offset) else { return }
        ids.swapAt(index, index + offset)
        Task { await model.orderAgents(ids) }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            FlowLayout(spacing: 6) {
                ForEach(entries) { agent in
                    Button { model.settingsAgent = agent.provider } label: { Label { Text(Format.provider(agent.provider)) } icon: { ProviderMark(provider: agent.provider, size: 20) } }
                        .buttonStyle(.plain).padding(6).background(model.settingsAgent == agent.provider ? Palette.accent.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: DesignTokens.radiusControl))
                }
            }
            HStack { Spacer(); Button(L("更多 Agent")) { selecting = true }; Button(L("管理 Agent")) { managing = true } }
        }
        .popover(isPresented: $selecting) {
            VStack(alignment: .leading) {
                TextField(L("搜索 Agent"), text: $pickerSearch)
                ScrollView {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach([true, false], id: \.self) { joined in
                            Text(L(joined ? "已加入" : "未加入")).foregroundStyle(.secondary)
                            ForEach((joined ? entries : model.agentConfigurations.filter { !model.settingsAgentIDs.contains($0.provider) }).filter { matches($0, pickerSearch) }) { agent in
                                Button { Task { if !joined { await model.manageAgent(agent.provider, visible: true) }; if model.settingsAgentIDs.contains(agent.provider) { model.settingsAgent = agent.provider; selecting = false } } } label: {
                                    HStack { Text(Format.provider(agent.provider)); Spacer(); if !joined { Text(L("添加")).foregroundStyle(.secondary) } }
                                }.buttonStyle(.plain).padding(6).disabled(model.settingsSaving)
                            }
                        }
                        if !model.agentConfigurations.contains(where: { matches($0, pickerSearch) }) { Text(L("没有匹配项")).foregroundStyle(.secondary) }
                    }
                }
                if let message = model.settingsMessage { Text(message).foregroundStyle(Palette.danger) }
            }.padding().frame(width: 280, height: 320)
        }
        .sheet(isPresented: $managing) {
            VStack(alignment: .leading, spacing: 12) {
                Text(L("管理 Agent")).font(AppFont.title)
                Text(L("移出会暂停采集，保留目录、账户和历史。定时唤醒由任务开关独立控制。")).font(AppFont.secondary).foregroundStyle(.secondary)
                TextField(L("搜索 Agent"), text: $managementSearch)
                List {
                    Section(L("已加入")) {
                        ForEach(entries.filter { matches($0, managementSearch) }) { agent in
                            HStack {
                                Text(Format.provider(agent.provider)); Spacer()
                                if managementSearch.isEmpty {
                                    Button { move(agent.provider, by: -1) } label: { Image(systemName: "chevron.up") }.accessibilityLabel(L("上移 Agent")).disabled(model.settingsAgentIDs.first == agent.provider)
                                    Button { move(agent.provider, by: 1) } label: { Image(systemName: "chevron.down") }.accessibilityLabel(L("下移 Agent")).disabled(model.settingsAgentIDs.last == agent.provider)
                                }
                                Button(L("移出并暂停采集")) { Task { await model.manageAgent(agent.provider, visible: false) } }
                            }
                        }.onMove { source, destination in
                            guard managementSearch.isEmpty else { return }
                            var ids = model.settingsAgentIDs; ids.move(fromOffsets: source, toOffset: destination)
                            Task { await model.orderAgents(ids) }
                        }.moveDisabled(!managementSearch.isEmpty || model.settingsSaving)
                    }
                    Section(L("未加入")) {
                        ForEach(model.agentConfigurations.filter { !model.settingsAgentIDs.contains($0.provider) && matches($0, managementSearch) }) { agent in
                            HStack { Text(Format.provider(agent.provider)); Spacer(); Button(L("添加")) { Task { await model.manageAgent(agent.provider, visible: true) } } }
                        }
                    }
                }.disabled(model.settingsSaving)
                if let message = model.settingsMessage { Text(message).foregroundStyle(Palette.danger) }
                HStack { Spacer(); Button(L("完成")) { managing = false }.keyboardShortcut(.defaultAction) }
            }.padding(20).frame(width: 560, height: 480)
        }
    }
}

struct AgentMachinesSection: View {
    @SurfaceModel var model: AppModel
    let agent: AgentConfiguration
    @State private var directories: AgentDirectorySelection?
    @State private var hooks: AgentDirectorySelection?
    @State private var sourceToManage: AgentSource?
    @State private var agySource: AgentSource?
    @State private var copilotAuth = false
    @State private var authIntent = "login"
    private var unboundMachines: [String] { Array(Set(model.settings.sources.filter { $0.provider == agent.provider && $0.unbound }.compactMap(\.hostId))).filter { id in !model.settings.hosts.contains { $0.id == id } }.sorted() }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Section(L("机器与目录 · 可多选")) {
            machine("local", name: L("本机"), available: model.settings.localDeviceEnabled != false)
            ForEach(model.settings.hosts) { host in machine(host.id, name: host.name.isEmpty ? host.target : host.name, available: host.enabled) }
            ForEach(unboundMachines, id: \.self) { id in
                machine(id, name: (model.settings.sources.first { $0.hostId == id }?.lastMachineName ?? id) + L(" · 机器未绑定"), available: false, unbound: true)
            }
            if agent.provider == "qoder" {
                Button(model.accountStatusBusy ? L("检查中…") : L("刷新账户状态")) { Task { await model.loadAccountStatuses(refresh: true, provider: agent.provider) } }.disabled(model.accountStatusBusy || !agent.enabled)
            }
            if model.settings.localDeviceEnabled == false && !model.settings.hosts.contains(where: \.enabled) {
                Text(L("暂无可采集机器，请在服务器设置开启 Agent 采集。")).foregroundStyle(.secondary)
            } else if !agent.machineIds.contains(where: { id in id == "local" ? model.settings.localDeviceEnabled != false : model.settings.hosts.contains { $0.id == id && $0.enabled } }) {
                Text(L("未选择机器，暂不采集。")).foregroundStyle(.secondary)
            }
            Button(L("添加 SSH 主机")) { model.openSettings(.init(category: "servers", section: "add-host")) }
        }.id("machines")
        .sheet(item: $directories) { selection in AgentDirectoriesView(model: model, provider: selection.provider, machine: selection.machine) }
        .sheet(item: $hooks) { selection in MachineSessionHooksView(model: model, provider: selection.provider, machine: selection.machine) }
        .sheet(item: $sourceToManage) { source in AccountConnectionView(model: model, initialSourceID: source.id, intent: authIntent) }
        .sheet(item: $agySource) { source in AntigravityLoginView(model: model, source: source) }
    }
    private func machine(_ id: String, name: String, available: Bool, unbound: Bool = false) -> some View {
        let sources = model.settings.sources.filter { $0.provider == agent.provider && ($0.hostId ?? "local") == id && $0.isDirectorySource }
        let enabled = sources.filter { $0.autoValuation?.enabled == true }.count
        return HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Toggle(name + (available || unbound ? "" : L(" · 采集已暂停")), isOn: Binding(get: { !unbound && agent.machineIds.contains(id) }, set: { selected in
                    var ids = agent.machineIds.filter { $0 != id }; if selected { ids.append(id) }
                    Task { await model.configureAgent(agent.provider, enabled: true, machines: ids) }
                })).toggleStyle(.checkbox).disabled(!agent.enabled || unbound || !available).lineLimit(2).help(name)
                if agent.provider == "qoder" && !unbound {
                    MachineAccountStatus(model: model, provider: agent.provider, machine: id)
                }
            }
            Spacer(minLength: 4)
            Button(L("目录 {0}" , sources.count)) { directories = .init(provider: agent.provider, machine: id) }
            if !unbound { MachineSessionStatus(live: model.live, provider: agent.provider, sources: sources.filter { $0.subscription == nil }, collecting: sources.contains { $0.subscription == nil && model.settings.sourceEnabled($0) }) { hooks = .init(provider: agent.provider, machine: id) } }
            if model.settings.supports(agent.provider, "valuation") {
                Text(unbound || !available || !agent.enabled || !agent.machineIds.contains(id) || !sources.contains(where: { model.settings.sourceEnabled($0) }) ? L("估值暂停") : enabled == 0 ? L("估值已关闭") : L("估值 {0}/{1}" , enabled, sources.count)).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize()
            }
            if !unbound {
                Menu {
                    ForEach(sources.filter { model.settings.sourceEnabled($0) }) { source in
                        if agent.provider == "codex" {
                            Button(L("登录 · ") + source.displayLocation) { authIntent = "login"; sourceToManage = source }.help(source.path)
                            Button(L("切换账户 · ") + source.displayLocation) { authIntent = "switch"; sourceToManage = source }.help(source.path)
                        }
                        if agent.provider == "antigravity" { Button(L("登录 / 检查 · ") + source.displayLocation) { agySource = source }.help(source.path) }
                    }
                    if !sources.contains(where: { model.settings.sourceEnabled($0) }) { Text(L("登录与切换需要启用 Agent、选择机器并开启目录采集")) }
                    Button(L("机器与采集设置")) { model.openSettings(.init(category: "servers")) }
                    Button(available ? L("暂停 Agent 采集") : L("恢复 Agent 采集")) { Task { await model.setDeviceEnabled(id, enabled: !available) } }
                } label: { Image(systemName: "ellipsis") }.menuStyle(.borderlessButton).menuIndicator(.hidden).tint(Color.primary).fixedSize().accessibilityLabel(name + L("操作"))
            }
        }.padding(.vertical, 3)
    }
}
private struct MachineAccountStatus: View {
    @ObservedObject var status: AccountStatusProjection
    let sourceIDs: Set<String>
    init(model: AppModel, provider: String, machine: String) {
        status = model.accountStatusStore.machineProjection(provider, machine)
        sourceIDs = Set(model.settings.sources.filter { $0.provider == provider && ($0.hostId ?? "local") == machine && model.settings.sourceEnabled($0) }.map(\.id))
    }
    private var summary: String {
        let rows = status.rows.filter { sourceIDs.contains($0.sourceId) }
        let identified = rows.filter { $0.identity != nil }.sorted {
            if ($0.error != nil) != ($1.error != nil) { return $0.error == nil }
            return ($0.checkedAt ?? 0) > ($1.checkedAt ?? 0)
        }
        var seen = Set<String>()
        let summaries = identified.filter { seen.insert($0.identity!.key).inserted }.compactMap(\.machineIdentitySummary)
        return summaries.isEmpty ? L("登录状态待确认") : summaries.joined(separator: "；")
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage; Text(summary).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(2).help(summary).textSelection(.enabled) }
}
private struct MachineSessionStatus: View {
    @ObservedObject var live: LiveSessionState
    let provider: String, sources: [AgentSource]
    let collecting: Bool
    var open: () -> Void
    private var label: String {
        if sources.isEmpty { return L("会话监测未启用") }
        if !collecting { return L("采集已暂停") }
        let rows = live.sources.filter { row in sources.contains { $0.id == row.sourceId } }
        if ["codex", "kimi", "deepseek-harness", "grok", "qoder", "opencode", "pi", "omp", "workbuddy"].contains(provider) { return rows.count == sources.count && rows.allSatisfy { $0.state == "healthy" } ? L("日志接入") : L("日志待确认") }
        if rows.contains(where: { $0.hooks == "error" }) { return L("接入失败") }
        if rows.contains(where: { $0.hooks == "disabled" }) { return L("实时监测已停用") }
        return rows.count == sources.count && rows.allSatisfy { $0.hooks == "installed" } ? L("会话监测已启用") : L("等待监测安装")
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        if ["codex", "claude", "antigravity", "cursor", "kimi", "codebuddy", "deepseek-harness", "grok", "zcode", "qoder", "opencode", "pi", "omp", "workbuddy"].contains(provider) { Button(label, action: open).disabled(sources.isEmpty).font(AppFont.secondary).lineLimit(1).help(label) }
    }
}
private struct MachineSessionHooksView: View {
    @Environment(\.dismiss) private var dismiss
    @SurfaceModel var model: AppModel
    let provider: String, machine: String
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 14) {
            Text(L("会话监测 · ") + model.machineName(machine)).font(AppFont.title)
            if ["codex", "kimi", "deepseek-harness", "grok", "qoder", "opencode", "pi", "omp", "workbuddy"].contains(provider) {
                Text(L("从 Agent 原生记录读取会话状态。")).foregroundStyle(.secondary)
            } else {
                Text(provider == "zcode" ? L("实时状态由 ZCode 原生监测提供；停用后仅显示已结算的状态。SSH 监测需要远端 Python 3。") : provider == "cursor" ? L("从用户或项目的 .cursor/hooks.json 接收状态；停用后不再接收新状态。原生会话库的 Token 暂未接入，SSH 监测需要远端 Python 3。") : machine == "local" ? L("采集时自动监测会话；停用实时监测后仍读取日志。") : L("通过 SSH 监测会话，需要远端 Python 3。停用实时监测后仍读取日志。")).foregroundStyle(.secondary)
                ScrollView { LazyVStack(alignment: .leading, spacing: 16) {
                    ForEach(model.settings.sources.filter { $0.provider == provider && $0.subscription == nil && ($0.hostId ?? "local") == machine && !$0.unbound }) { source in SessionHookControls(model: model, source: source) }
                } }
            }
            // Changes apply immediately, so the sheet ends with Done (§5.2).
            HStack { Spacer(); Button(L("完成")) { dismiss() }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction) }
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogHooksWidth, height: provider == "codex" ? 180 : 400).aieyesAccent()
    }
}

struct AgentAutomaticValuationView: View {
    @Environment(\.dismiss) private var dismiss
    @SurfaceModel var model: AppModel
    let provider: String, machine: String
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == provider && ($0.hostId ?? "local") == machine && $0.isDirectorySource } }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 16) {
            Text(Format.provider(provider) + " · " + model.machineName(machine)).font(AppFont.title)
            ScrollView { AutomaticValuationControls(model: model, sources: sources).frame(maxWidth: .infinity, alignment: .leading) }
            HStack { Spacer(); Button(L("完成")) { dismiss() }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction).disabled(model.automaticSaving) }
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogAutomaticWidth, height: 440).aieyesAccent()
            .interactiveDismissDisabled(model.automaticSaving)
            .updateOperation(L("自动估值设置"), active: model.automaticSaving)
    }
}

struct AgentDirectoriesView: View {
    @Environment(\.dismiss) private var dismiss
    @SurfaceModel var model: AppModel
    let provider: String, machine: String
    @State private var editing: AgentSource?
    @State private var newDirectoryError: String?
    @State private var removing: AgentSource?
    @State private var rebinding: AgentSource?
    @State private var error: String?
    @State private var busy = false
    @State private var shellHost: Host?
    @State private var shellBaseline: Settings?
    @State private var commandAccount: AgentAccount?
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == provider && ($0.hostId ?? "local") == machine && $0.isDirectorySource } }
    private var linkedAccounts: [AgentAccount] { model.settings.accounts.filter { a in a.archived != true && a.provider == provider && sources.contains { a.uses($0) } } }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack {
            Text(Format.provider(provider) + " · " + model.machineName(machine)).font(AppFont.title).frame(maxWidth: .infinity, alignment: .leading).padding()
            Form {
                ForEach(sources) { source in
                    Section {
                        Text(source.path).textSelection(.enabled)
                        HStack {
                            if source.unbound { Text(L("机器未绑定")).foregroundStyle(Palette.warn); Button(L("重新绑定机器")) { rebinding = source } }
                            else { Button(L("编辑目录")) { newDirectoryError = nil; editing = source } }
                            Button(L("移除目录配置"), role: .destructive) { removing = source }
                        }
                    }
                }
                Button(L("添加目录")) {
                    Task {
                        busy = true; error = nil; newDirectoryError = nil; defer { busy = false }
                        var source = AgentSource(provider: provider, path: "")
                        source.hostId = machine == "local" ? nil : machine
                        do {
                            struct DefaultPath: Decodable { var path: String }
                            let result: DefaultPath = try await model.engine.call("sources.defaultPath", params: ["provider": provider, "hostId": source.hostId ?? ""])
                            source.path = result.path
                        } catch { newDirectoryError = error.localizedDescription }
                        editing = source
                    }
                }.disabled(machine != "local" && !model.settings.hosts.contains { $0.id == machine })
                Text(L("配置采集位置，原文件保留。")).font(AppFont.secondary).foregroundStyle(.secondary)
                if machine != "local", let host = model.settings.hosts.first(where: { $0.id == machine }) {
                    Button(L("远程 Shell 设置")) { shellBaseline = model.settings; shellHost = host }
                    if linkedAccounts.count == 1, let account = linkedAccounts.first {
                        Button(L("账户前置命令")) { commandAccount = account }
                    } else if !linkedAccounts.isEmpty {
                        Menu(L("账户前置命令")) { ForEach(linkedAccounts, id: \.key) { a in Button(a.name) { commandAccount = a } } }
                    } else {
                        Button(L("添加账户")) { model.openSettings(.init(provider: provider, section: "accounts")); dismiss() }
                    }
                }
                if let error { ErrorNotice(message: error) }
            }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle()).disabled(busy)
            HStack { Spacer(); Button(L("完成")) { dismiss() }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction) }.padding()
        }.frame(width: 600, height: 480).aieyesAccent().interactiveDismissDisabled(busy)
        .updateOperation(L("Agent 目录配置"), active: busy)
        .sheet(item: $shellHost) { host in
            HostEditor(host: host, model: model, initialSection: "shell") { value, password in
                try await model.saveHostConfiguration(value, password: password, base: shellBaseline ?? model.settings)
                shellHost = nil
            }
        }
        .sheet(item: $commandAccount) { account in
            AccountDevicesView(model: model, account: account, initialMachineID: machine, initialSection: "command")
        }
        .sheet(item: $rebinding) { source in RebindSourceView(model: model, source: source) }
        .sheet(item: $editing) { source in
            SourceEditor(source: source, hosts: model.settings.hosts, appProxy: model.settings.proxy, testURLs: model.settings.proxyTestUrls) { item in
                if newDirectoryError != nil && item.path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { throw ClientError.message(L("请选择实际的原生数据目录后保存")) }
                var params: [String: Any] = ["source": try JSONSerialization.jsonObject(with: JSONEncoder().encode(item))]
                if sources.contains(where: { $0.id == source.id }) { params["baseSource"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(source)) }
                let saved: Settings = try await model.engine.call("sources.configure", params: params)
                model.acceptConfiguration(saved); editing = nil
                Task { await model.reload() }
            }.safeAreaInset(edge: .top) { if let newDirectoryError { ErrorNotice(message: newDirectoryError).padding() } }
        }
        .sheet(item: $removing) { source in
            DangerConfirmation(title: L("移除目录配置？"), explanation: L("仅解除此目录的采集和账户连接；文件、登录凭证及历史记录保留。"), affected: [source.path], confirmLabel: L("移除配置"), cancel: { removing = nil }, confirm: {
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
    @SurfaceModel var model: AppModel
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
    @State private var copilotAuth = false
    @State private var copilotDeviceFlow = false
    @State private var qoderCLI: QoderCLIStatus?
    @State private var qoderInstall = false

    @State private var buddyAuth = false
    @State private var buddyProvider = "workbuddy"
    private var visibleAgents: [AgentConfiguration] {
        model.agentConfigurations.filter { config in
            provider.map { CoreContract.sameAgent(config.provider, $0) } ?? (config.enabled || model.settings.accounts.contains { CoreContract.sameAgent($0.provider, config.provider) && $0.archived != true })
        }
    }
    private var archivedAccounts: [AgentAccount] { model.settings.accounts.filter { $0.archived == true && (provider == nil || CoreContract.sameAgent($0.provider, provider!)) } }
    var chatgptWorkInstallationNotice: String? {
        guard provider == "codex", model.chatgptWorkInstalled else { return nil }
        return L("本机已安装 ChatGPT Work，会话与用量单独显示，账户和额度与 Codex 共用。")
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        ScrollViewReader { reader in
            Form {
                if let provider, let agent = visibleAgents.first(where: { $0.provider == provider }) {
                    Section {
                        HStack {
                            Label { Text(Format.provider(provider)).font(AppFont.section) } icon: { ProviderMark(provider: provider) }.fixedSize()
                            Spacer()
                            Text(L("启用")).fixedSize()
                            Toggle(L("启用 ") + Format.provider(provider), isOn: Binding(get: { agent.enabled }, set: { value in Task { await model.configureAgent(provider, enabled: value, machines: value && agent.machineIds.isEmpty ? ["local"] : nil, group: true) } })).labelsHidden().toggleStyle(.switch).fixedSize()
                        }
                        if let notice = chatgptWorkInstallationNotice {
                            Text(notice).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
                PersistentSection(key: sectionKey("accounts"), title: L("账户"), detail: L("{0} 个账户" , model.settings.accounts.filter { $0.archived != true && (provider == nil || CoreContract.sameAgent($0.provider, provider!)) }.count), tint: Palette.category(provider ?? "custom")) {
                    if let message = model.deploymentSyncMessage { HStack { Text(L(message)); Button(L("重试同步")) { model.synchronizeAccountDeployments() } } }
                    ForEach(visibleAgents) { agent in agentSection(agent) }
                    archivedSection
                    cleanupSection
                }.id("accounts").background(ScrollMemory(key: "settings.agent." + (provider ?? "all")))
                if let apiProvider = CoreContract.featureProvider(provider ?? "codex", "api") {
                    PersistentSection(key: sectionKey("api"), title: L("API 连接"), initiallyExpanded: false) {
                        if apiProvider == "codex" { CodexAPISettingsSection(model: model) }
                        else { NativeAPISettingsSection(model: model, provider: apiProvider) }
                    }.id("api")
                }
                if let provider, (model.settings.nativeConnections ?? []).contains(where: { CoreContract.sameAgent($0.provider, provider) && $0.kind == "api" }) {
                    PersistentSection(key: sectionKey("native-api"), title: L("原生 API 连接"), initiallyExpanded: false) { NativeConnectionsSection(model: model, provider: provider, kind: "api") }
                }
                if let provider, let agent = model.machineConfiguration(provider) {
                    PersistentSection(key: sectionKey("machines"), title: L("机器与目录"), detail: L("{0} 台机器", agent.machineIds.count), initiallyExpanded: false) {
                        AgentMachinesSection(model: model, agent: agent)
                    }.id("machines")
                }
                if let provider, model.settings.agentSupports(provider, "valuation") {
                    PersistentSection(key: sectionKey("valuation"), title: L("估值与采样"), initiallyExpanded: false) {
                        Button(L("管理估值与采样")) { showingValuation = true }
                    }.id("valuation")
                }
            }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle())
            .onAppear { if let section = model.settingsSection { UserDefaults.standard.set(true, forKey: sectionKey(section)); reader.scrollTo(section, anchor: .top) } }
            .onChange(of: model.settingsSection) { _, section in if let section { UserDefaults.standard.set(true, forKey: sectionKey(section)); reader.scrollTo(section, anchor: .top) } }
        }
        .sheet(isPresented: $showingValuation) { if let provider { AgentValuationView(model: model, provider: provider) } }
        .task { cleanup = (try? await model.engine.call("accounts.cleanup.list")) ?? []; await model.loadAccountStatuses() }
        .task { if visibleAgents.contains(where: { $0.provider == "copilot" }) { let status = try? await model.engine.call("copilotAuth.status", as: CopilotAuthState.self); copilotDeviceFlow = status?.deviceFlow == true } }
        .task { if visibleAgents.contains(where: { $0.provider == "qoder" }) { qoderCLI = try? await model.engine.call("qoderCli.status", as: QoderCLIStatus.self) } }
        .sheet(item: $adding, onDismiss: { Task { await model.loadAccountStatuses(refresh: true, dueOnly: true) } }) { agent in AccountConnectionView(model: model, initialProvider: agent.provider) }
        .sheet(item: $selectedAccount) { account in AccountDevicesView(model: model, account: account) }
        .sheet(item: $agySource) { source in AntigravityLoginView(model: model, source: source) }
        .sheet(isPresented: $copilotAuth) { CopilotAuthView(model: model) }
        .sheet(isPresented: $qoderInstall) { QoderCLIInstallView(model: model) { qoderCLI = $0 } }

        .sheet(isPresented: $buddyAuth) { BuddyAuthView(model: model, provider: buddyProvider) }
        .sheet(item: $sourceToManage, onDismiss: { Task { await model.loadAccountStatuses(refresh: true, dueOnly: true) } }) { source in AccountConnectionView(model: model, initialSourceID: source.id, intent: authIntent) }
        .sheet(item: $deleting, onDismiss: { Task { cleanup = (try? await model.engine.call("accounts.cleanup.list")) ?? [] } }) { account in DeleteArchivedAccountView(model: model, account: account) }
    }
    private func sectionKey(_ section: String) -> String { "settings.section." + (provider ?? "all") + "." + section }
    private func agentSection(_ agent: AgentConfiguration) -> some View {
        let accounts = model.settings.accounts.filter { CoreContract.sameAgent($0.provider, agent.provider) && $0.archived != true }
        return Section {
            HStack {
                if provider == nil { Label { Text(Format.provider(agent.provider)).font(AppFont.section) } icon: { ProviderMark(provider: agent.provider) } }
                Spacer()
                if CoreContract.subscriptionModes[agent.provider]?.isEmpty != true {
                    Button(L("添加账户"), systemImage: "plus") { adding = agent }.disabled(!agent.enabled)
                }
            }
            ForEach(accounts, id: \.key) { account in
                HStack {
                    AccountStatusSummary(model: model, account: account)
                    Spacer(); Button(L("设置")) { selectedAccount = account }
                }
            }
            if accounts.isEmpty { Text(L("尚未添加账户")).foregroundStyle(.secondary) }
            if agent.provider == "claude" { claudeKeychainRow }
            if agent.provider == "copilot" { copilotAuthRow(enabled: agent.enabled) }
            if agent.provider == "qoder", let qoderCLI { QoderCLIRow(model: model, status: qoderCLI, enabled: agent.enabled) { qoderInstall = true } }
            ForEach(CoreContract.subscriptionModes.keys.sorted().filter { CoreContract.sameAgent($0, agent.provider) && CoreContract.subscriptionModes[$0]?.isEmpty == true }, id: \.self) { edition in
                if let raw = CoreContract.subscriptionLoginURLs[edition], let url = URL(string: raw) {
                    Text(L(CoreContract.subscriptionHints[edition] ?? "")).font(AppFont.secondary).foregroundStyle(.secondary)
                    Link(L("{0} 官方用量页", Format.provider(edition)), destination: url)
                }
            }
            if ["workbuddy", "codebuddy"].contains(agent.provider) {
                DisclosureGroup(L("登录查看余额（可选）")) {
                    Text(L("可选登录，使用非公开接口；可随时退出登录。仅用于查看余额。")).foregroundStyle(.secondary)
                    Button(L("登录查看余额")) { buddyProvider = agent.provider; buddyAuth = true }.disabled(!agent.enabled)
                }.font(AppFont.secondary)
            }
            NativeConnectionsSection(model: model, provider: agent.provider, kind: "login")
        }
    }
    /// Offered until a GitHub token from a file, the environment or a grant is attributed to an account.
    @ViewBuilder private func copilotAuthRow(enabled: Bool) -> some View {
        if !(model.settings.nativeConnections ?? []).contains(where: { $0.provider == "copilot" && $0.machineId == "local" && $0.hasCredential && $0.status == "observed" && $0.accountKey != nil }) {
            HStack {
                Text(copilotDeviceFlow ? L("未找到 GitHub 登录，授权后显示 Copilot 额度") : L("在 Copilot 中登录后自动显示额度")).font(AppFont.secondary).foregroundStyle(.secondary)
                Spacer()
                Button(copilotDeviceFlow ? L("GitHub 授权") : L("GitHub 登录说明")) { copilotAuth = true }.disabled(!enabled)
            }
        }
    }
    /// The one-time Keychain answer recorded by core; the token stays in core memory.
    @ViewBuilder private var claudeKeychainRow: some View {
        if let consent = model.settings.claudeKeychain, consent.askedAt > 0 {
            HStack {
                Text(consent.granted ? L("钥匙串已获授权，显示官方额度") : consent.state == "missing" ? L("未登录或使用 API key，仍可查看本地用量") : L("已拒绝钥匙串授权，仍可查看本地用量"))
                    .font(AppFont.secondary).foregroundStyle(.secondary)
                Spacer()
                if !consent.granted { Button(L("重新授权")) { Task { await model.requestClaudeKeychain() } } }
            }
            if let error = model.claudeKeychainError { Text(error).font(AppFont.secondary).foregroundStyle(.red) }
        }
    }
    private func machineRow(_ agent: AgentConfiguration, machine: String) -> some View {
        HStack {
            Text(model.machineName(machine)); Spacer()
            Text(currentAccount(agent.provider, machine: machine)).font(AppFont.secondary).foregroundStyle(.secondary)
            if agent.provider == "antigravity", let source = managementSource(agent.provider, machine: machine) { Button(L("登录 / 检查")) { agySource = source } }
            if agent.provider == "codex", let source = managementSource(agent.provider, machine: machine) {
                let loggedIn = model.accountStatuses.contains { $0.accountKey.hasPrefix("codex:") && $0.sourceId == source.id && ($0.current == true || $0.credential == true) }
                if loggedIn { Button(L("切换账户")) { authIntent = "switch"; sourceToManage = source } }
                else { Button(L("登录")) { authIntent = "login"; sourceToManage = source } }
            }
        }
    }
    @ViewBuilder private var archivedSection: some View {
        if !archivedAccounts.isEmpty {
            Section(L("已归档账户")) {
                ForEach(archivedAccounts, id: \.key) { account in
                    HStack {
                        Text(Format.provider(account.provider) + " · " + account.name); Spacer()
                        Button(L("恢复")) { restore(account) }
                        Button(L("删除账户"), role: .destructive) { deleting = account }
                    }
                }
            }
        }
    }
    @ViewBuilder private var cleanupSection: some View {
        if !cleanup.isEmpty {
            Section(L("已解除管理的远端任务")) {
                ForEach(cleanup) { record in cleanupRecord(record) }
            }
        }
    }
    private func cleanupRecord(_ record: AccountCleanupRecord) -> some View {
        DisclosureGroup {
            ForEach(record.tasks) { task in cleanupTask(task) }
        } label: {
            Text(L("{0} · 远端可能仍在运行" , record.name))
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
        if !names.isEmpty { return L("正在使用 ") + names.joined(separator: "、") }
        let rows = model.accountStatuses.filter { $0.accountKey.hasPrefix(provider + ":") && $0.machineId == machine }
        if let summary = rows.first(where: { $0.identity != nil })?.identitySummary ?? rows.compactMap(\.identitySummary).first { return summary }
        if rows.contains(where: { $0.credential == true }) { return L("已登录，尚未选用") }
        return rows.isEmpty || rows.contains(where: { $0.current == nil || $0.credential == nil || $0.error != nil }) ? L("登录状态待确认") : L("未登录")
    }
}

private struct AccountStatusSummary: View {
    @SurfaceModel var model: AppModel
    @ObservedObject var status: AccountStatusProjection
    let account: AgentAccount
    init(model: AppModel, account: AgentAccount) {
        _model = SurfaceModel(wrappedValue: model); self.account = account; status = model.accountStatusStore.projection(account.key)
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
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
    @SurfaceModel var model: AppModel
    @ObservedObject private var statusProjection: AccountStatusProjection
    @State var account: AgentAccount
    let initialMachineID: String?
    init(model: AppModel, account: AgentAccount, initialMachineID: String? = nil, initialSection: String? = nil) {
        _model = SurfaceModel(wrappedValue: model); _account = State(initialValue: account); statusProjection = model.accountStatusStore.projection(account.key)
        self.initialMachineID = initialMachineID
        _expandedCommands = State(initialValue: initialSection == "command" ? Set([initialMachineID].compactMap { $0 }) : [])
    }
    @State private var expandedCommands: Set<String>
    @State private var commands: [String: String] = [:]
    @State private var busy = false
    @State private var error: String?
    @State private var loginSource: AgentSource?
    @State private var authIntent = "login"
    @State private var agySource: AgentSource?
    @State private var saveTask: Task<Void, Never>?
    @State private var pendingSaveCount = 0
    @State private var intervalText = ""
    @State private var queryExpanded = false
    @State private var showingConnection = false
    @FocusState private var focusedField: String?
    @AppStorage(NativePanelKey.accounts.rawValue) private var panelPreference = "{}"
    private var current: AgentAccount { model.settings.accounts.first { $0.key == account.key } ?? account }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack {
            Text(account.name).font(AppFont.title).frame(maxWidth: .infinity, alignment: .leading).padding(20)
            ScrollViewReader { scroll in
            Form {
                Section {
                    if model.settings.supports(account.provider, "quota") {
                    Toggle(L("显示并查询额度"), isOn: Binding(get: { account.quotaEnabled }, set: { value in account.quotaEnabled = value; save { $0.quotaEnabled = value } }))
                    if let message = model.dashboard.quotas.first(where: { $0.id == account.key })?.error { ErrorNotice(message: message, context: L("额度查询失败")) }
                    DisclosureGroup(L("查询与提醒"), isExpanded: $queryExpanded) {
                    Picker(L("优先查询位置"), selection: Binding(get: { account.quotaSourceId ?? "" }, set: { value in account.quotaSourceId = value.isEmpty ? nil : value; save { $0.quotaSourceId = value.isEmpty ? nil : value } })) {
                        Text(L("自动 · 优先本机")).tag("")
                        ForEach(model.settings.sources.filter { current.uses($0) }) { source in Text(model.machineName(source.hostId ?? "local") + " · " + source.displayLocation + (model.settings.sourceEnabled(source) ? "" : L(" · 已暂停"))).tag(source.id) }
                    }
                    if model.settings.supports(account.provider, "quotaReset") && !(account.provider == "grok" && model.settings.sources.contains { current.uses($0) && $0.subscription?.mode == "api" }) {
                        AccountResetNotificationPicker(title: L("提前重置提醒"), value: account.quotaResetNotifications?.unexpected, inherited: model.settings.quotaResetNotifications?.unexpected ?? true) { value in
                            var preferences = account.quotaResetNotifications ?? QuotaResetNotificationSettings(); preferences.unexpected = value; account.quotaResetNotifications = preferences
                            save { next in var p = next.quotaResetNotifications ?? QuotaResetNotificationSettings(); p.unexpected = value; next.quotaResetNotifications = p }
                        }
                        AccountResetNotificationPicker(title: L("到期重置提醒"), value: account.quotaResetNotifications?.natural, inherited: model.settings.quotaResetNotifications?.natural ?? false) { value in
                            var preferences = account.quotaResetNotifications ?? QuotaResetNotificationSettings(); preferences.natural = value; account.quotaResetNotifications = preferences
                            save { next in var p = next.quotaResetNotifications ?? QuotaResetNotificationSettings(); p.natural = value; next.quotaResetNotifications = p }
                        }
                    }
                    LabeledContent { TextField(L("查询间隔（秒）"), text: $intervalText, prompt: Text(L("沿用全局 · {0} 秒", model.settings.refreshSeconds))).labelsHidden().frame(width: DesignTokens.fieldHintWidth).focused($focusedField, equals: "interval") } label: {
                        HStack(spacing: 4) { Text(L("查询间隔（秒）")); HelpNote(title: L("查询间隔"), text: L("留空沿用全局间隔；自定义范围 5–86400 秒。")) }
                    }.help(L("留空沿用全局 · {0} 秒" , model.settings.refreshSeconds))
                    if account.provider == "qoder" { Text(L("Qoder 额度查询会启动 qodercli，最少每 10 分钟一次")).font(AppFont.secondary).foregroundStyle(.secondary) }
                    }
                    }
                    LabeledContent(L("账户名称")) { TextField(L("账户名称"), text: $account.name).labelsHidden().focused($focusedField, equals: "name") }
                    Toggle(L("归档账户"), isOn: Binding(get: { account.archived == true }, set: { value in account.archived = value; save { $0.archived = value } }))
                    if model.settings.supports(account.provider, "quota") {
                    Toggle(L("在面板显示"), isOn: panelBinding).disabled(!current.quotaEnabled)
                    HStack { Text(L("已选 {0} 个账户", panelSelectionCount)).font(AppFont.secondary).foregroundStyle(.secondary); HelpNote(title: L("面板账户"), text: L("自动模式显示每类前 5 个账户。手动模式可留空；新增账户默认加入面板，可随时取消。")) }
                    if !PanelAccountPreference.automatic(panelPreference, provider: account.provider) {
                        Button(L("恢复自动选择")) { var pref = PanelAccountPreference.decode(panelPreference); pref.providers[account.provider] = .init(); panelPreference = String(data: (try? JSONEncoder().encode(pref)) ?? Data(), encoding: .utf8) ?? "{}" }
                    }
                    }
                    if account.provider != "codex" {
                        Button(account.provider == "deepseek" ? L("更新 API Key") : CoreContract.subscriptionModes[account.provider] != nil ? L("更新额度凭据") : L("连接机器")) {
                            focusedField = nil
                            guard saveEditedText(force: error != nil) else { return }
                            Task { await saveTask?.value; if error == nil { showingConnection = true } }
                        }
                    }
                }
                ForEach(model.accountMachineIDs(account.provider, includePaused: true), id: \.self) { machine in
                    Section(model.machineName(machine) + (model.accountMachineIDs(account.provider).contains(machine) ? "" : L(" · 已暂停"))) {
                        Text(model.accountDeviceSummary(current, machine: machine)).font(AppFont.secondary)
                        ForEach(model.settings.sources.filter { $0.provider == account.provider && ($0.hostId ?? "local") == machine }) { source in
                            let status = model.accountStatuses.first { $0.accountKey == account.key && $0.sourceId == source.id }
                            HStack {
                                VStack(alignment: .leading) { Text(source.name); Text(source.path.isEmpty ? (source.subscription?.mode == "cli" ? L("CLI 额度连接") : L("订阅额度连接")) : source.path).font(AppFont.secondary).foregroundStyle(.secondary) }
                                Spacer()
                                if account.provider == "antigravity", source.accountId == account.id { Button(L("登录 / 检查")) { agySource = source }.disabled(!model.settings.sourceEnabled(source)) }
                                if account.provider == "codex" {
                                    if status?.current == true { Text(L("正在使用")).foregroundStyle(.secondary) }
                                    else if status?.credential == true { Button(L("切换账户")) { openLogin(source, intent: "switch") }.disabled(!model.settings.sourceEnabled(source)) }
                                    else { Button(L("登录")) { openLogin(source, intent: "login") }.disabled(!model.settings.sourceEnabled(source)) }
                                }
                            }
                            if let message = status?.error ?? status?.note, !message.isEmpty { Text(L(message)).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled) }
                            if let time = status?.checkedAt { Text(L("检查于 ") + Format.time(time)).font(AppFont.secondary).foregroundStyle(.secondary) }
                        }
                        if machine != "local" {
                            DisclosureGroup(isExpanded: Binding(get: { expandedCommands.contains(machine) }, set: { expanded in
                                if expanded { expandedCommands.insert(machine) } else { expandedCommands.remove(machine) }
                            })) {
                                TextEditor(text: Binding(get: { commands[machine] ?? inheritedCommand(machine) }, set: { commands[machine] = $0 })).font(.system(size: DesignTokens.fontEntry, design: .monospaced)).frame(height: 85).focused($focusedField, equals: machine)
                                if current.deviceSettings?.first(where: { $0.machineId == machine })?.preCommandResetRequired == true {
                                    Text(L("此机器的旧目录前置命令不一致，已清空，请重新设置。")).foregroundStyle(Palette.warn)
                                    Button(L("保存此机器的前置命令")) { commands[machine] = commands[machine] ?? ""; saveEditedText(force: true) }
                                }

                            } label: {
                                HStack(spacing: 4) { Text(L("前置命令")); HelpNote(title: L("前置命令"), text: L("用于此账户的授权、切换、额度查询及定时唤醒；不用于同步历史或监控。清空后不执行。")) }
                            }.id("command:" + machine)
                        }
                    }
                }
                if let error { HStack { ErrorNotice(message: error); Button(L("重试保存")) { saveEditedText(force: true) } } }
                if let message = model.deploymentSyncMessage { HStack { Text(L(message)); Button(L("重试同步")) { model.synchronizeAccountDeployments() } } }
            }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle())
            .task {
                if let machine = initialMachineID, expandedCommands.contains(machine) {
                    await Task.yield()
                    scroll.scrollTo("command:" + machine, anchor: .top)
                }
            }
            }
            // Toggles apply at once; Done also commits the text fields still being edited.
            HStack { Spacer(); Button(L("完成")) { focusedField = nil; guard saveEditedText(force: error != nil) else { return }; Task { await saveTask?.value; if error == nil { dismiss() } } }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction).disabled(busy) }.padding(20)
        }.frame(width: 680, height: 650).aieyesAccent().interactiveDismissDisabled(busy || error != nil || focusedField != nil)
        .updateDraftGuard(L("账户设置"), snapshot: draftSnapshot(account) + intervalText + draftSnapshot(commands), dirty: account != current || !commands.isEmpty || intervalText != (current.quotaRefreshSeconds.map(String.init) ?? ""), saving: busy, save: { focusedField = nil; guard saveEditedText(force: true) else { return false }; await saveTask?.value; if error == nil { dismiss(); return true }; return false }, discard: { dismiss() })
        .onAppear { intervalText = account.quotaRefreshSeconds.map(String.init) ?? "" }
        .onChange(of: current.name) { old, value in if focusedField != "name" && account.name == old { account.name = value } }
        .onChange(of: focusedField) { old, _ in if old != nil { saveEditedText() } }
        .sheet(item: $agySource) { source in AntigravityLoginView(model: model, source: source) }
        .sheet(isPresented: $showingConnection, onDismiss: { Task { await model.loadAccountStatuses(refresh: true, dueOnly: true) } }) { AccountConnectionView(model: model, account: current) }
        .sheet(item: $loginSource, onDismiss: { Task { await model.loadAccountStatuses(refresh: true, dueOnly: true) } }) { source in AccountConnectionView(model: model, account: current, initialSourceID: source.id, intent: authIntent) }
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
        guard !name.isEmpty else { error = L("请输入账户名称"); return false }
        guard text.isEmpty || Int(text).map({ (5...86400).contains($0) }) == true else { queryExpanded = true; error = L("查询间隔范围为 5–86400 秒"); return false }
        let interval = Int(text), edits = commands
        let notificationChanged = account.quotaResetNotifications != current.quotaResetNotifications
        let flagsChanged = notificationChanged || account.quotaEnabled != current.quotaEnabled || (account.archived == true) != (current.archived == true) || account.quotaSourceId != current.quotaSourceId
        if !force && !flagsChanged && name == current.name && interval == current.quotaRefreshSeconds && edits.allSatisfy({ machine, command in current.deviceSettings?.first(where: { $0.machineId == machine })?.preCommand == command }) { return true }
        account.quotaRefreshSeconds = interval
        let notificationPreferences = account.quotaResetNotifications
        let quotaEnabled = account.quotaEnabled, archived = account.archived, priority = account.quotaSourceId
        save { next in
            next.name = name; next.quotaRefreshSeconds = interval
            if force || notificationChanged { next.quotaResetNotifications = notificationPreferences }
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
                guard let i = next.accounts.firstIndex(where: { $0.key == account.key }) else { throw ClientError.message(L("账户已移除")) }
                change(&next.accounts[i]); try await model.persistConfiguration(next)
                if account.name.trimmingCharacters(in: .whitespacesAndNewlines) == current.name && Int(intervalText.trimmingCharacters(in: .whitespacesAndNewlines)) == current.quotaRefreshSeconds && account.quotaEnabled == current.quotaEnabled && (account.archived == true) == (current.archived == true) && account.quotaSourceId == current.quotaSourceId && account.quotaResetNotifications == current.quotaResetNotifications && commands.allSatisfy({ machine, command in current.deviceSettings?.first(where: { $0.machineId == machine })?.preCommand == command }) { error = nil }
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
    @SurfaceModel var model: AppModel
    let account: AgentAccount
    @State private var tasks: [AccountCleanupTask] = []
    @State private var failed: [AccountCleanupTask] = []
    @State private var busy = false
    @State private var loaded = false
    @State private var force = false
    @State private var error: String?
    private var visibleTasks: [AccountCleanupTask] { failed.isEmpty ? tasks : failed }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 16) {
            Text(L("删除归档账户「") + account.name + "」").font(AppFont.title)
            Text(L("账户配置与关联任务将移除；历史记录和机器上的凭证保留，不退出 Agent。"))
            taskList
            deletionStatus
            deletionControls
        }.padding(DesignTokens.dialogPadding).frame(width: 600, height: 430).interactiveDismissDisabled(busy)
        .updateDraftGuard(L("删除归档账户"), snapshot: "", dirty: false, saving: busy, save: { false }, discard: { dismiss() })
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
        if !tasks.isEmpty { Toggle(L("强制删除（设备已弃用或无法连接）"), isOn: $force).disabled(busy) }
        if force { Text(L("无法保证远端任务停止。强制删除后将解除本地管理，不再自动重试；未清理设备和部署位置会保留供查看。")).foregroundStyle(Palette.warn) }
        if let error { ErrorNotice(message: error).foregroundStyle(Palette.warn) }
        if busy { ProgressView(L("正在清理关联任务…")) }
    }
    private var deletionControls: some View {
        HStack {
            Button(L("取消")) { dismiss() }.disabled(busy)
            Spacer()
            Button(force ? L("确认强制删除") : L("停止任务并删除账户"), role: .destructive) { remove() }.disabled(busy || !loaded)
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
            else { failed = result.failedTasks ?? []; error = L("部分任务未清理，账户仍保留。可重试，或确认强制删除。") }
        } catch { self.error = error.localizedDescription } }
    }
}

struct AccountConnectionView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var agySource: AgentSource?
    @SurfaceModel var model: AppModel
    var account: AgentAccount?
    var initialProvider: String? = nil
    var initialSourceID: String? = nil
    var intent = "login"
    @State private var provider = ""
    @State private var selection = Set<String>()
    @State private var apiKey = ""
    @State private var useLocalLogin = false
    @State private var subscriptionMode = ""
    @State private var nativeConfigDir = ""
    @State private var workspaceID = ""
    @State private var preparedKey: (key: String, path: String)?
    @State private var busy = false
    @State private var error: String?
    @State private var confirmDiscard = false
    /// Typed keys and machine choices are only kept by saving; Cancel and Esc ask before dropping them.
    private var dirty: Bool { !selection.isEmpty || !apiKey.isEmpty || useLocalLogin || !workspaceID.isEmpty || !nativeConfigDir.isEmpty }
    private var usesSubscription: Bool { CoreContract.subscriptionModes[provider] != nil && !(provider == "deepseek" && account != nil && model.settings.sources.contains { account!.uses($0) && $0.subscription == nil }) }
    private var supportsLocalLogin: Bool { (provider == "grok" && subscriptionMode == "subscription") || (provider == "opencode" && subscriptionMode == "go") }
    private var needsWorkspace: Bool { (provider == "grok" && subscriptionMode == "api") || (provider == "opencode" && subscriptionMode == "console") }
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == provider && model.settings.sourceEnabled($0) && ($0.accountId.isEmpty || $0.accountId == (account?.id ?? "")) } }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 12) {
            Text(intent == "switch" ? L("切换账户") : account == nil ? L("添加账户") : provider == "deepseek" ? L("更新 API Key") : CoreContract.subscriptionModes[provider] != nil ? L("更新额度凭据") : L("连接机器")).font(AppFont.title).padding(.horizontal, 20).padding(.top, 20)
            Text(Format.provider(provider)).font(AppFont.section).padding(.horizontal, 20)
            if provider == "codex" { CodexAccountsView(model: model, targetAccountID: account?.id, initialSourceID: initialSourceID, intent: intent, onBusyChanged: { busy = $0 }) }
            else {
                Form {
                    if usesSubscription, let modes = CoreContract.subscriptionModes[provider] {
                        Text(L(CoreContract.subscriptionHints[provider] ?? "")).foregroundStyle(.secondary)
                        if !(provider == "qoder" && subscriptionMode == "cli") {
                            Button(L("打开官方登录 / 用量页")) { Task { do { let _: [String: Bool] = try await model.engine.call("subscriptions.openLogin", params: ["provider": provider, "mode": subscriptionMode]) } catch { self.error = error.localizedDescription } } }
                        }
                        if modes.count > 1 { Picker(L("服务 / 地区"), selection: $subscriptionMode) { ForEach(modes.keys.sorted(), id: \.self) { key in Text(L(modes[key] ?? key)).tag(key) } } }
                        if provider == "qoder" && subscriptionMode == "cli" {
                            LabeledContent(L("CLI 配置目录（留空跟随客户端）")) { TextField(L("CLI 配置目录（留空跟随客户端）"), text: $nativeConfigDir).labelsHidden() }
                            Text(L("读取 CLI 当前登录并绑定账户身份，无需填写密钥。CLI 切换账号后请切回原账户或另建账户。")).foregroundStyle(.secondary)
                        } else {
                            if needsWorkspace { LabeledContent(provider == "grok" ? "Team ID" : "Workspace ID") { TextField(provider == "grok" ? "Team ID" : "Workspace ID", text: $workspaceID).labelsHidden() } }
                            if !supportsLocalLogin || !useLocalLogin { LabeledContent(L("API Key / 登录令牌 / Cookie")) { SecureField(L("API Key / 登录令牌 / Cookie"), text: $apiKey).labelsHidden() } }
                        }
                        if supportsLocalLogin { Toggle(L("读取本机已登录凭据"), isOn: $useLocalLogin) }
                        if !(provider == "qoder" && subscriptionMode == "cli") { Text(L("验证成功后保存到系统凭据库；不会切换客户端账户。")).foregroundStyle(.secondary) }
                    } else if provider == "deepseek" {
                        LabeledContent("API Key") { SecureField("API Key", text: $apiKey).labelsHidden() }
                        Text(L("Key 保存在本机，只用于查询账户余额。")).foregroundStyle(.secondary)
                    } else {
                        Section(L("已有登录的机器 · 可多选")) {
                            ForEach(sources) { source in HStack { Toggle(source.displayLocation, isOn: Binding(get: { selection.contains(source.id) }, set: { if $0 { selection.insert(source.id) } else { selection.remove(source.id) } })); if provider == "antigravity" { Button(L("登录 / 检查")) { agySource = source } } } }
                            if sources.isEmpty { Text(L("请启用 Agent、选择机器并配置采集目录。每个目录连接一个当前登录账户。")).foregroundStyle(.secondary); Button(L("机器与采集设置")) { model.openSettings(.init(provider: provider, section: "machines")); dismiss() } }
                        }
                    }
                    if account == nil { Text(L("添加后可修改账户的显示名称。")).foregroundStyle(.secondary) }
                    if let error { ErrorNotice(message: error) }
                }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle()).disabled(busy)
            }
            HStack {
                Spacer()
                // Codex manages its own sign-in steps; the sheet only closes.
                if provider == "codex" { Button(L("完成")) { dismiss() }.keyboardShortcut(.cancelAction).disabled(busy) }
                else {
                    Button(L("取消")) { if dirty { confirmDiscard = true } else { dismiss() } }.keyboardShortcut(.cancelAction).disabled(busy)
                    Button(busy ? L("保存中…") : account == nil ? L("添加账户") : provider == "deepseek" ? L("更新 API Key") : CoreContract.subscriptionModes[provider] != nil ? L("保存额度凭据") : L("保存连接")) { Task { await save() } }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction).disabled(busy)
                }
            }.padding([.horizontal, .bottom], 20)
        }.frame(width: DesignTokens.dialogAccountWidth, height: 620).interactiveDismissDisabled(busy || dirty)
        .discardDraftConfirmation($confirmDiscard) { dismiss() }
        .updateDraftGuard(L("账户连接"), snapshot: provider + draftSnapshot(selection.sorted()) + apiKey + subscriptionMode + workspaceID + nativeConfigDir + String(useLocalLogin), dirty: !selection.isEmpty || !apiKey.isEmpty || useLocalLogin || !workspaceID.isEmpty || !nativeConfigDir.isEmpty, saving: busy, save: { await save(); return error == nil }, discard: { dismiss() })
        .sheet(item: $agySource) { source in AntigravityLoginView(model: model, source: source) }
        .onAppear { provider = account?.provider ?? initialProvider ?? (initialSourceID != nil ? "codex" : model.agentConfigurations.first(where: { $0.enabled })?.provider ?? "") }
        .onChange(of: provider) { _, _ in
            selection = []; error = nil
            let existing = model.settings.sources.first { $0.provider == provider && $0.accountId == account?.id && $0.subscription != nil }?.subscription
            subscriptionMode = existing?.mode ?? (provider == "qoder" ? "cli" : CoreContract.subscriptionModes[provider]?["global"] != nil ? "global" : provider == "grok" ? "subscription" : provider == "opencode" ? "go" : CoreContract.subscriptionModes[provider]?.keys.sorted().first ?? "")
            workspaceID = existing?.workspaceId ?? ""
            nativeConfigDir = existing?.nativeConfigDir ?? ""
        }
    }
    private func save() async {
        busy = true; error = nil; defer { busy = false }
        do {
            let target = account?.id ?? ""
            let name = account?.name ?? Format.provider(provider) + L(" 账户")
            if usesSubscription {
                struct Result: Decodable { var settings: Settings }
                let result: Result = try await model.engine.call("subscriptions.connect", params: ["provider": provider, "accountId": target, "name": name, "secret": apiKey, "useLocalLogin": supportsLocalLogin && useLocalLogin, "subscription": ["mode": subscriptionMode, "workspaceId": needsWorkspace ? workspaceID : "", "nativeConfigDir": provider == "qoder" && subscriptionMode == "cli" ? nativeConfigDir : ""]])
                model.acceptConfiguration(result.settings)
            } else if provider == "deepseek" {
                guard !apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw ClientError.message(L("请输入 API Key")) }
                var next = model.settings
                let id = target.isEmpty ? UUID().uuidString : target
                if target.isEmpty { next.accounts.append(AgentAccount(id: id, name: name, provider: provider)) }
                let source = next.sources.first { $0.provider == provider && $0.accountId == id } ?? AgentSource(name: name, provider: provider, accountId: id, path: "")
                if !next.sources.contains(where: { $0.id == source.id }) { next.sources.append(source) }
                if preparedKey?.key != apiKey {
                    let result: [String: String] = try await model.engine.call("credentials.save", params: ["sourceId":source.id + "-" + UUID().uuidString, "apiKey":apiKey])
                    guard let path = result["path"], !path.isEmpty else { throw ClientError.message(L("凭据保存失败")) }
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
    @SurfaceModel var model: AppModel
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
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 6) {
            Text(source.path).font(AppFont.secondary).textSelection(.enabled)
            HStack {
                Toggle(L("实时监测"), isOn: Binding(get: { status.map { $0.state != "disabled" } ?? false }, set: { enabled in Task { await perform(enabled ? "repair" : "remove") } })).toggleStyle(.switch).disabled(busy || status == nil)
                Text(status?.label ?? L("正在读取…")).foregroundStyle(.secondary)
                Spacer()
                Button(L("修复监测")) { Task { await perform("repair") } }.disabled(busy)
            }
            if let message = failure ?? status?.error { HStack { Text(L(message)).font(AppFont.secondary).foregroundStyle(Palette.danger); Button(L("重试")) { Task { await perform("status") } } } }
        }.task(id: source.id + source.path) { await perform("status") }
    }
}
private struct RebindSourceView: View {
    @Environment(\.dismiss) private var dismiss
    @SurfaceModel var model: AppModel
    let source: AgentSource
    @State private var machine = ""
    @State private var busy = false
    @State private var error: String?
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 16) {
            Text(L("重新绑定机器")).font(AppFont.title)
            Text(source.path).textSelection(.enabled)
            Text(L("保留原目录、账户关联和历史；重新绑定后从目标机器采集。")).foregroundStyle(.secondary)
            Picker(L("目标机器"), selection: $machine) {
                Text(L("请选择机器")).tag(""); Text(L("本机")).tag("local")
                ForEach(model.settings.hosts) { host in Text(host.name.isEmpty ? host.target : host.name).tag(host.id) }
            }
            if let error { ErrorNotice(message: error).foregroundStyle(Palette.danger) }
            HStack { Spacer(); Button(L("取消")) { dismiss() }; Button(busy ? L("验证中…") : L("绑定")) { Task { await save() } }.disabled(machine.isEmpty) }.disabled(busy)
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
