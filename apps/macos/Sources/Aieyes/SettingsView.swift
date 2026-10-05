import SwiftUI

struct SettingsView: View {
    @ObservedObject var model: AppModel
    private var draft: Settings {
        get { model.settingsDraft }
        nonmutating set { model.settingsDraft = newValue }
    }
    @State private var sourceEditor: AgentSource?
    @State private var accountEditor: AgentAccount?
    @State private var hostEditor: Host?
    @State private var priceEditor: ModelPrice?
    @State private var search = ""
    @State private var mappingModel = ""
    @State private var mappingID = ""
    @MainActor init(model: AppModel) {
        self.model = model
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("设置").font(AppFont.title)
                    Text(!model.settingsLoaded ? "正在读取配置…" : model.settingsDirty ? "有未保存的配置更改" : "配置编辑完成后，在此保存全部更改").font(AppFont.secondary).foregroundStyle(.secondary)
                }
                Spacer()
                if !model.settingsLoaded { Button("重试读取") { Task { await model.bootstrap() } } }
                if model.settingsSaving { ProgressView().controlSize(.small) }
                Button("放弃更改") { model.discardSettingsDraft() }.disabled(!model.settingsDirty || model.settingsSaving)
                Button(model.settingsSaving ? "保存中…" : "保存") { Task { _ = await model.saveSettingsDraft() } }.buttonStyle(.borderedProminent).keyboardShortcut("s").disabled(!model.settingsDirty || model.settingsSaving)
            }.padding(22)
            if let message = model.settingsMessage ?? model.message { HStack { Text(message).font(AppFont.secondary).textSelection(.enabled); Spacer(); Button { model.settingsMessage = nil; model.message = nil } label: { Image(systemName: "xmark") }.buttonStyle(.plain).accessibilityLabel("关闭提示") }.padding(.horizontal, 22).padding(.bottom, 10) }
            HStack(spacing: 6) {
                ForEach([("sources", "数据源", "tray.full"), ("servers", "服务器", "server.rack"), ("prices", "价格", "dollarsign.circle"), ("general", "通用", "slider.horizontal.3")], id: \.0) { key, title, icon in
                    Button { model.settingsTab = key } label: {
                        Label(title, systemImage: icon).font(.system(size: 15, weight: model.settingsTab == key ? .semibold : .regular)).frame(maxWidth: .infinity).padding(.vertical, 10)
                            .foregroundStyle(model.settingsTab == key ? Palette.accent : .secondary)
                            .background(model.settingsTab == key ? Palette.accent.opacity(0.1) : Color.clear, in: RoundedRectangle(cornerRadius: 9))
                    }.buttonStyle(.plain).accessibilityValue(model.settingsTab == key ? "已选中" : "")
                }
            }.padding(.horizontal, 22).padding(.bottom, 12)
            Divider().opacity(0.5)
            Group {
                switch model.settingsTab {
                case "sources", "accounts": sources
                case "servers": servers
                case "prices": prices
                case "general", "connection": general
                default: sources
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top).padding(.horizontal, 16).padding(.bottom, 16).disabled(model.settingsSaving || !model.settingsLoaded)
        }
        .font(AppFont.body).disabled(model.installingUpdate)
        .frame(minWidth: 620, idealWidth: 760, minHeight: 440, idealHeight: 600).tint(Palette.accent)
        .onAppear { if model.settingsTab == "accounts" { model.settingsTab = "sources" }; if model.settingsTab == "connection" { model.settingsTab = "general" }; Task { await model.loadPrices() }; consumeEditorRequest() }
        .onChange(of: model.requestedSourceProvider) { _, _ in consumeEditorRequest() }
        .onChange(of: model.requestHostEditor) { _, _ in consumeEditorRequest() }
        .sheet(item: $sourceEditor) { source in
            SourceEditor(source: source, hosts: draft.hosts, accounts: draft.accounts, sources: draft.sources, pendingAPIKey: model.pendingAPIKeys[source.id] ?? "") { item, account, apiKey in
                var next = draft
                if let i = next.sources.firstIndex(where: { $0.id == item.id }) { next.sources[i] = item } else { next.sources.append(item) }
                if let account {
                    if let i = next.accounts.firstIndex(where: { $0.key == account.key }) { next.accounts[i] = account } else { next.accounts.append(account) }
                }
                for i in next.accounts.indices where next.accounts[i].quotaSourceId == item.id && (next.accounts[i].id != item.accountId || next.accounts[i].provider != item.provider) { next.accounts[i].quotaSourceId = nil }
                draft = next; model.stageAPIKey(apiKey, sourceID: item.id); sourceEditor = nil
            }
        }
        .sheet(item: $accountEditor) { account in
            AccountEditor(account: account, sources: draft.sources) { item in
                var next = draft
                if let i = next.accounts.firstIndex(where: { $0.key == account.key }) { next.accounts[i] = item }
                draft = next; accountEditor = nil
            }
        }
        .sheet(item: $hostEditor) { host in HostEditor(host: host) { item in
            if let i = draft.hosts.firstIndex(where: { $0.id == item.id }) { draft.hosts[i] = item } else { draft.hosts.append(item) }
            hostEditor = nil
        } }
        .sheet(item: $priceEditor) { price in PriceEditor(price: price) { item in
            guard await model.savePrice(item) else { throw ClientError.message(model.message ?? "保存失败") }
            priceEditor = nil
        } }
    }
    private func consumeEditorRequest() {
        if let provider = model.requestedSourceProvider {
            sourceEditor = AgentSource(provider: provider, path: provider == "codex" ? "~/.codex" : provider == "claude" ? "~/.claude" : "")
            model.requestedSourceProvider = nil
        }
        if model.requestHostEditor { hostEditor = Host(); model.requestHostEditor = false }
    }
    private var sources: some View {
        VStack(spacing: 12) {
            List {
                ForEach($model.settingsDraft.sources) { $source in
                    HStack(spacing: 12) {
                        Toggle("启用", isOn: $source.enabled).labelsHidden().toggleStyle(.switch).controlSize(.small)
                        VStack(alignment: .leading, spacing: 5) { Text(source.name).font(AppFont.section); Text(Format.provider(source.provider) + " · " + (source.accountId.isEmpty ? "无账户" : draft.accounts.first { $0.id == source.accountId && $0.provider == source.provider }?.name ?? source.accountId)).font(AppFont.secondary).foregroundStyle(.secondary) }
                        if let error = model.dashboard.sources.first(where: { $0.id == source.id })?.status?.error { Text(error).font(AppFont.secondary).foregroundStyle(.orange) }
                        Spacer()
                        Button("编辑") { sourceEditor = source }
                        Button { model.stageAPIKey(nil, sourceID: source.id); draft.sources.removeAll { $0.id == source.id }; for i in draft.accounts.indices where draft.accounts[i].quotaSourceId == source.id { draft.accounts[i].quotaSourceId = nil } } label: { Image(systemName: "minus.circle") }.buttonStyle(.borderless).help("移除数据源")
                    }.padding(.vertical, 7)
                }
                if !unlinkedAccounts.isEmpty {
                    DisclosureGroup("未关联账户") { ForEach(unlinkedAccounts, id: \.key) { accountRow($0) } }
                }
                let archived = draft.accounts.filter { $0.archived == true }
                if !archived.isEmpty {
                    DisclosureGroup("已归档账户") { ForEach(archived, id: \.key) { accountRow($0) } }
                }
            }.listStyle(.inset)
            HStack { Button("添加数据源", systemImage: "plus") { sourceEditor = AgentSource() }; Spacer(); Button("保存并同步记录") { Task { if await model.saveSettingsDraft() { await model.scan() } } }.disabled(model.busy) }.padding(.horizontal, 8)
        }.padding(.vertical, 12)
    }
    private var unlinkedAccounts: [AgentAccount] {
        draft.accounts.filter { account in account.archived != true && !draft.sources.contains { $0.accountId == account.id && $0.provider == account.provider } }
    }
    private func accountRow(_ account: AgentAccount) -> some View {
        HStack {
            Text(account.name); Spacer(); Button("编辑") { accountEditor = account }
            Button(account.archived == true ? "恢复" : "归档") {
                Task {
                    var next = draft
                    if let i = next.accounts.firstIndex(where: { $0.key == account.key }) { next.accounts[i].archived = account.archived != true }
                    draft = next
                }
            }
        }.padding(.vertical, 6)
    }
    private var servers: some View {
        VStack(spacing: 12) {
            List {
                ForEach($model.settingsDraft.hosts) { $host in HStack(spacing: 12) {
                    Toggle("启用", isOn: $host.enabled).labelsHidden().toggleStyle(.switch).controlSize(.small)
                    VStack(alignment: .leading, spacing: 5) { Text(host.name.isEmpty ? host.target : host.name).font(AppFont.section); Text(host.target).font(AppFont.secondary).foregroundStyle(.secondary) }
                    Spacer(); Button("编辑") { hostEditor = host }
                    Button { draft.hosts.removeAll { $0.id == host.id }; for i in draft.sources.indices where draft.sources[i].hostId == host.id { draft.sources[i].hostId = nil; draft.sources[i].enabled = false } } label: { Image(systemName: "minus.circle") }.buttonStyle(.borderless).help("移除主机")
                }.padding(.vertical, 7) }
            }.listStyle(.inset)
            HStack { Button("添加 SSH 主机", systemImage: "plus") { hostEditor = Host() }; Spacer(); Button("保存并测试采样") { Task { if await model.saveSettingsDraft() { await model.sampleHosts(); if let e = model.hosts.first(where: { $0.error != nil })?.error { model.message = e } else { model.message = "采样完成" } } } }.disabled(model.serverBusy) }.padding(.horizontal, 8)
        }.padding(.vertical, 12)
    }
    private var filteredPrices: [ModelPrice] {
        let query = search.trimmingCharacters(in: .whitespacesAndNewlines)
        return model.prices.filter { query.isEmpty || $0.id.localizedCaseInsensitiveContains(query) || $0.name.localizedCaseInsensitiveContains(query) }
    }
    private var prices: some View {
        ScrollView { VStack(spacing: 12) {
            Text("价格条目与同步立即生效；模型映射随顶部“保存”提交。").font(AppFont.secondary).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading)
            if !model.dashboard.pricingGaps.isEmpty {
                GroupBox("所选时间范围 · 待计价模型") {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(model.dashboard.pricingGaps) { gap in
                                HStack {
                                    VStack(alignment: .leading, spacing: 4) { Text(gap.model).font(.system(size: 14, weight: .medium)); Text("缺少\(gap.missing) · \(Format.compact(gap.unpricedTokens)) Token").font(AppFont.secondary).foregroundStyle(.secondary) }
                                    Spacer()
                                    Button("映射") { mappingModel = gap.model; mappingID = gap.priceId ?? "" }
                                    Button("补充价格") { priceEditor = model.prices.first { $0.id == (gap.priceId ?? gap.model) } ?? ModelPrice(id: gap.priceId ?? gap.model, name: gap.model) }
                                }
                            }
                        }.padding(6)
                    }.frame(height: 100)
                }
            }
            HStack {
                TextField("搜索模型", text: $search).textFieldStyle(.roundedBorder)
                Button("同步 OpenRouter") { Task { await model.syncPrices() } }.disabled(model.busy)
                Button("添加价格") { priceEditor = ModelPrice() }
            }
            List(filteredPrices) { price in
                HStack { VStack(alignment: .leading, spacing: 4) { Text(price.id).font(AppFont.body); Text("输入 \(price.input.map { Format.money($0 * 1e6) } ?? "—") · 输出 \(price.output.map { Format.money($0 * 1e6) } ?? "—") / 百万 Token").font(AppFont.secondary).foregroundStyle(.secondary) }; Spacer(); Button("编辑") { priceEditor = price } }
            }.listStyle(.inset).frame(height: 200).overlay { if filteredPrices.isEmpty { Text(search.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "同步模型价格" : "无匹配模型").foregroundStyle(.secondary) } }
            GroupBox("模型映射") {
                VStack(spacing: 8) {
                    HStack { TextField("日志中的模型名称", text: $mappingModel); Image(systemName: "arrow.right"); TextField("OpenRouter 模型 ID", text: $mappingID); Button("添加映射") { draft.modelMappings[mappingModel.trimmingCharacters(in: .whitespacesAndNewlines)] = mappingID.trimmingCharacters(in: .whitespacesAndNewlines); mappingModel = ""; mappingID = "" }.disabled(mappingModel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || mappingID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
                    ScrollView { LazyVStack(spacing: 8) { ForEach(draft.modelMappings.keys.sorted(), id: \.self) { key in HStack { Text(key); Image(systemName: "arrow.right"); Text(draft.modelMappings[key] ?? ""); Spacer(); Button { draft.modelMappings.removeValue(forKey: key) } label: { Image(systemName: "minus.circle") }.buttonStyle(.plain) }.font(AppFont.secondary) } }.padding(.vertical, 4) }.frame(maxHeight: 100)
                }.padding(6)
            }
            HStack { Text("USD / 百万 Token").font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Button("保存配置并重算") { Task { if await model.saveSettingsDraft() { await model.reprice() } } } }
        }.padding(12) }
    }
    private var general: some View {
        Form {
            Section("菜单栏") {
                Picker("显示内容", selection: $model.settingsDraft.menuMetric) { Text("图标").tag("icon"); Text("当前范围 Token").tag("tokens"); Text("首个账号剩余额度").tag("quota"); Text("首台服务器 CPU").tag("cpu") }
            }
            Section("刷新") {
                TextField("Agent 间隔（秒）", value: $model.settingsDraft.refreshSeconds, format: .number)
                TextField("服务器后台间隔（秒）", value: $model.settingsDraft.serverRefreshSeconds, format: .number)
            }
            Section("连接") { ProxyFields(proxy: $model.settingsDraft.proxy) }
            Section("更新") { UpdateSettingsView() }
        }.formStyle(.grouped)
    }
}

struct ProxyFields: View {
    @Binding var proxy: ProxySettings
    var inherit = false
    @State private var address: ProxyAddressDraft
    init(proxy: Binding<ProxySettings>, inherit: Bool = false) {
        _proxy = proxy; self.inherit = inherit
        _address = State(initialValue: ProxyAddressDraft(url: proxy.wrappedValue.url))
    }
    var body: some View {
        Group {
            Picker("连接方式", selection: $proxy.mode) {
                if inherit { Text("跟随应用").tag("inherit") }
                Text("系统代理").tag("system"); Text("直连").tag("direct"); Text("指定代理").tag("custom")
            }
            if proxy.mode == "custom" {
                Picker("协议", selection: $address.scheme) {
                    Text("HTTP").tag("http"); Text("HTTPS").tag("https"); Text("SOCKS5").tag("socks5"); Text("SOCKS5H").tag("socks5h"); Text("自定义 URL").tag("url")
                }
                if address.scheme == "url" { TextField("代理 URL", text: $address.customURL) }
                else { TextField("Host", text: $address.host); TextField("端口", text: $address.port) }
            }
        }
        .onChange(of: address.scheme) { old, scheme in
            if scheme == "url", old != "url" { var prior = address; prior.scheme = old; address.customURL = prior.url }
        }
        .onChange(of: address) { _, value in proxy.url = value.url }
        .onChange(of: proxy.mode) { _, mode in if mode == "custom" { proxy.url = address.url } }
    }
}

struct SourceEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State var source: AgentSource
    @State private var asAccount: Bool
    @State private var accountChoice: String
    @State private var account: AgentAccount
    @State private var apiKey = ""
    @State private var saving = false
    @State private var error: String?
    var hosts: [Host], accounts: [AgentAccount], sources: [AgentSource]
    var onSave: (AgentSource, AgentAccount?, String?) async throws -> Void
    init(source: AgentSource, hosts: [Host], accounts: [AgentAccount], sources: [AgentSource] = [], pendingAPIKey: String = "", onSave: @escaping (AgentSource, AgentAccount?, String?) async throws -> Void) {
        _apiKey = State(initialValue: pendingAPIKey)
        _source = State(initialValue: source); self.hosts = hosts; self.accounts = accounts; self.sources = sources; self.onSave = onSave
        _asAccount = State(initialValue: !source.accountId.isEmpty || (!sources.contains { $0.id == source.id } && ["agy", "deepseek"].contains(source.provider)))
        _accountChoice = State(initialValue: source.accountId.isEmpty ? "new" : source.accountId)
        _account = State(initialValue: accounts.first { $0.id == source.accountId && $0.provider == source.provider } ?? AgentAccount(provider: source.provider, quotaEnabled: ["codex", "claude", "agy", "deepseek"].contains(source.provider)))
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("数据源").font(AppFont.title); Spacer() }.padding(22)
            Form {
                TextField("名称", text: $source.name)
                Picker("Agent", selection: $source.provider) {
                    Text("Codex").tag("codex"); Text("Claude Code").tag("claude"); Text("Antigravity").tag("antigravity"); Text("agy").tag("agy"); Text("DeepSeek").tag("deepseek"); Text("自定义").tag("custom")
                }
                Toggle("关联账户与限额", isOn: $asAccount)
                Text("多个数据源可以关联同一账户，共用账户身份和限额。").font(AppFont.secondary).foregroundStyle(.secondary)
                if asAccount {
                    Picker("账户", selection: $accountChoice) {
                        Text("新建账户").tag("new")
                        ForEach(accounts.filter { $0.provider == source.provider && ($0.archived != true || $0.id == source.accountId) }, id: \.key) { Text($0.name + ($0.archived == true ? "（已归档）" : "")).tag($0.id) }
                    }
                    TextField("账户名称", text: $account.name)
                    Toggle("显示并查询账户限额", isOn: $account.quotaEnabled)
                    if account.quotaEnabled {
                        Picker("优先查询位置", selection: Binding(get: { account.quotaSourceId ?? "" }, set: { account.quotaSourceId = $0.isEmpty ? nil : $0 })) {
                            Text("自动 · 优先本机").tag("")
                            ForEach(sources.filter { $0.provider == source.provider && $0.accountId == account.id && $0.id != source.id }) { Text($0.name).tag($0.id) }
                            Text(source.name.isEmpty ? "此数据源" : source.name).tag(source.id)
                        }
                    }
                    Toggle("归档账户", isOn: Binding(get: { account.archived == true }, set: { account.archived = $0 }))
                }
                if source.provider != "deepseek" {
                    Picker("位置", selection: Binding(get: { source.hostId ?? "local" }, set: { source.hostId = $0 == "local" ? nil : $0 })) {
                        Text("本机").tag("local"); ForEach(hosts) { Text($0.name.isEmpty ? $0.target : $0.name).tag($0.id) }
                    }
                }
                if source.provider == "deepseek" {
                    SecureField("API Key（留空保留）", text: $apiKey)
                    Text("新 Key 随设置顶部的保存提交；留空保留已保存 Key。").font(AppFont.secondary).foregroundStyle(.secondary)
                    DisclosureGroup("高级设置") { TextField("API Key 文件（可选）", text: $source.path) }
                } else if source.provider != "agy" {
                    HStack {
                        TextField("数据目录", text: $source.path)
                        if source.hostId == nil {
                            Button("选择") { let panel = NSOpenPanel(); panel.canChooseDirectories = true; panel.canChooseFiles = true; panel.showsHiddenFiles = true; if panel.runModal() == .OK, let url = panel.url { source.path = url.path } }
                        }
                    }
                }
                if source.provider == "agy" { TextField("agy 程序", text: Binding(get: { source.agyBinary ?? "agy" }, set: { source.agyBinary = $0 })) }
                if source.provider == "codex" { TextField("Codex 程序", text: $source.codexBinary) }
                if source.hostId != nil && asAccount {
                    Section("限额查询前置命令") { TextEditor(text: $source.quotaPreCommand).font(.system(size: 14, design: .monospaced)).frame(height: 75).help("留空继承主机前置命令") }
                }
                if source.hostId == nil {
                    ProxyFields(proxy: Binding(get: { source.proxy ?? ProxySettings(mode: "inherit") }, set: { source.proxy = $0.mode == "inherit" ? nil : $0 }), inherit: true)
                }
                if let error { Text(error).foregroundStyle(.orange) }
            }.formStyle(.grouped).disabled(saving)
            HStack {
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction).disabled(saving); Spacer(); Text("完成后，在设置中保存").font(AppFont.secondary).foregroundStyle(.secondary); Spacer()
                Button(saving ? "处理中…" : "完成") { Task { await save() } }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(saving)
            }.padding(20)
        }.font(AppFont.body).frame(width: 570, height: EditorLayout.height(660))
        .interactiveDismissDisabled(saving)
        .onChange(of: accountChoice) { _, id in
            account = accounts.first { $0.id == id && $0.provider == source.provider } ?? AgentAccount(provider: source.provider, quotaEnabled: ["codex", "claude", "agy", "deepseek"].contains(source.provider))
        }
        .onChange(of: source.provider) { _, provider in
            accountChoice = "new"; account = AgentAccount(provider: provider, quotaEnabled: ["codex", "claude", "agy", "deepseek"].contains(provider)); source.accountId = ""; apiKey = ""
            if ["deepseek", "agy"].contains(provider) { asAccount = true }
            if provider == "deepseek" { source.hostId = nil; source.path = "" }
            if ["~/.codex", "~/.claude", ""].contains(source.path) { source.path = provider == "codex" ? "~/.codex" : provider == "claude" ? "~/.claude" : "" }
        }
    }
    private func save() async {
        saving = true; error = nil; defer { saving = false }
        do {
            var next = source
            if !["agy", "deepseek"].contains(next.provider), next.path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { throw ClientError.message("请输入数据目录") }
            if next.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { next.name = Format.provider(next.provider) }
            if asAccount {
                account.provider = next.provider
                if account.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { account.name = next.name }
                next.accountId = account.id
            } else { next.accountId = "" }
            if next.provider == "deepseek" { next.hostId = nil }
            let pendingKey = next.provider == "deepseek" && !apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? apiKey : nil
            try await onSave(next, asAccount ? account : nil, pendingKey)
        } catch { self.error = error.localizedDescription }
    }
}

struct AccountEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State var account: AgentAccount
    @State private var saving = false
    @State private var error: String?
    var sources: [AgentSource], onSave: (AgentAccount) async throws -> Void
    private var linked: [AgentSource] { sources.filter { $0.provider == account.provider && $0.accountId == account.id } }
    var body: some View {
        VStack(spacing: 0) {
            Text("账户").font(AppFont.title).padding(22)
            Form {
                TextField("账户名称", text: $account.name)
                Toggle("显示并查询账户限额", isOn: $account.quotaEnabled)
                Toggle("归档账户", isOn: Binding(get: { account.archived == true }, set: { account.archived = $0 }))
                if account.quotaEnabled {
                    Picker("优先查询位置", selection: Binding(get: { account.quotaSourceId ?? "" }, set: { account.quotaSourceId = $0.isEmpty ? nil : $0 })) {
                        Text("自动 · 优先本机").tag(""); ForEach(linked) { Text($0.name).tag($0.id) }
                    }
                }
                if let error { Text(error).foregroundStyle(.orange) }
            }.formStyle(.grouped).disabled(saving)
            HStack {
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction).disabled(saving); Spacer(); Text("完成后，在设置中保存").font(AppFont.secondary).foregroundStyle(.secondary); Spacer()
                Button("完成") { Task {
                    saving = true; error = nil; defer { saving = false }
                    do { try await onSave(account) } catch { self.error = error.localizedDescription }
                } }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(saving || account.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }.padding(20)
        }.font(AppFont.body).frame(width: 560, height: EditorLayout.height(420)).interactiveDismissDisabled(saving)
    }
}

struct HostEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State var host: Host
    var onSave: (Host) -> Void
    @State private var devices = ""
    @State private var discovered: [String: [DeviceMetric]] = [:]
    @State private var discovering = false
    @State private var discoveryError: String?
    private let discoveryEngine = EngineClient()
    private let groups = [("cpu","CPU"),("memory","内存"),("gpu","GPU"),("filesystems","文件系统"),("disk","磁盘 I/O"),("network","网络")]
    private let detailGroups = ["cpuTimes":"cpu", "memoryCache":"memory", "swap":"memory", "fsAvailable":"filesystems", "fsType":"filesystems", "inodes":"filesystems", "diskIops":"disk", "diskBusy":"disk", "networkTotals":"network", "networkErrors":"network", "gpuMemory":"gpu", "gpuThermals":"gpu"]
    private var tokens: [String] { MonitorSelection.parse(devices) }
    private func deviceOptions(_ group: String) -> [SelectionOption] {
        let rows = (discovered[group] ?? []).filter { group != "cpu" || $0.id != "cpu" }
        let known = Set(rows.map(\.id))
        let missing = MonitorSelection.deviceIDs(tokens, group: group).subtracting(known).sorted()
        return rows.map { device in SelectionOption(id: device.id, label: device.id + (device.type.map { " · " + $0 } ?? device.name.map { " · " + $0 } ?? "")) }
            + missing.map { SelectionOption(id: $0, label: $0, unavailable: true) }
    }
    private func discover() async {
        discovering = true; defer { discovering = false }; discoveryError = nil
        do {
            let data = try JSONEncoder().encode(host)
            let object = try JSONSerialization.jsonObject(with: data)
            let sample: MetricSample = try await discoveryEngine.call("hosts.discover", params: ["host": object])
            for (group, rows) in [("cpu",sample.cpu),("gpu",sample.gpu),("filesystems",sample.filesystems),("disk",sample.disk),("network",sample.network)] {
                if let rows { discovered[group] = rows }
            }
            let failed = sample.errors.keys.sorted().map { key in (groups.first { $0.0 == key }?.1 ?? key) + "读取失败" }
            discoveryError = failed.isEmpty ? nil : failed.joined(separator: " · ")
        } catch { discoveryError = error.localizedDescription }
    }
    private func devicePicker(_ group: String, _ label: String) -> some View {
        let options = deviceOptions(group)
        return MultiSelectPicker(title: label, options: options,
            selected: MonitorSelection.selected(tokens, group: group, available: Set(options.map(\.id))),
            all: !tokens.contains { $0.hasPrefix(group + ":") }) { selected, all in
                devices = MonitorSelection.write(tokens, group: group, selected: selected, all: all).joined(separator: ", ")
            }.disabled(!host.metrics.contains(group))
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("SSH 主机").font(AppFont.title); Spacer() }.padding(22)
            Form {
                TextField("名称", text: $host.name)
                TextField("SSH 别名或地址", text: $host.target, prompt: Text("my-server 或 user@host"))
                TextField("端口", text: Binding(get: { host.port.map(String.init) ?? "" }, set: { host.port = Int($0) }), prompt: Text("跟随 SSH 配置"))
                TextField("密钥路径", text: $host.identityFile, prompt: Text("跟随 SSH 配置"))
                Section {
                    MultiSelectPicker(title: "采集项目", options: groups.map { SelectionOption(id: $0.0, label: $0.1) }, selected: Set(host.metrics)) { selected, _ in
                        host.metrics = selected.sorted()
                    }
                }
                Section("设备") {
                    Button(discovering ? "读取中…" : "读取设备") { Task { await discover() } }.disabled(discovering || host.target.isEmpty)
                    if let discoveryError { Text(discoveryError).font(AppFont.secondary).foregroundStyle(.orange) }
                    ForEach(groups.filter { $0.0 != "memory" }, id: \.0) { group, label in devicePicker(group, label) }
                }
                Section {
                    MultiSelectPicker(title: "显示细分项", options: Host.detailOptions.filter { host.metrics.contains(detailGroups[$0.0] ?? "") }.map { SelectionOption(id: $0.0, label: $0.1) }, selected: Set(host.details ?? Host.detailOptions.map { $0.0 })) { selected, _ in
                        host.details = selected.sorted()
                    }.disabled(host.metrics.isEmpty)
                }
                DisclosureGroup("高级设置") {
                    TextField("远程 shell", text: $host.shell)
                    Text("远程前置命令").foregroundStyle(.secondary)
                    TextEditor(text: $host.preCommand).font(.system(size: 14, design: .monospaced)).frame(height: 65)
                    TextField("设备表达式", text: $devices, prompt: Text("network:eth0, gpu:0, filesystems:/")).help("留空显示全部设备")
                }
            }.formStyle(.grouped)
            HStack { Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Spacer(); Text("完成后，在设置中保存").font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Button("完成") { host.devices = MonitorSelection.parse(devices); if host.name.isEmpty { host.name = host.target }; onSave(host) }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(host.target.isEmpty) }.padding(20)
        }.font(AppFont.body).frame(width: 620, height: EditorLayout.height(650)).onAppear { devices = host.devices.joined(separator: ", ") }
    }
}

struct PriceEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State var price: ModelPrice
    var onSave: (ModelPrice) async throws -> Void
    @State private var input = ""
    @State private var output = ""
    @State private var cacheRead = ""
    @State private var cacheWrite = ""
    @State private var saving = false
    @State private var error: String?
    private var valid: Bool { [input, output, cacheRead, cacheWrite].allSatisfy { $0.isEmpty || Double($0).map { $0.isFinite && $0 >= 0 } == true } }
    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                HStack { Text("模型价格").font(AppFont.title); Spacer(); Text("USD / 百万 Token").font(AppFont.secondary).foregroundStyle(.secondary) }
                Text("保存此价格后立即生效，不需要再保存设置。").font(AppFont.secondary).foregroundStyle(.secondary)
            }.padding(22)
            Form {
                TextField("模型 ID", text: $price.id, prompt: Text("openai/model-name"))
                TextField("显示名称", text: $price.name)
                TextField("输入", text: $input, prompt: Text("待定价")); TextField("输出", text: $output, prompt: Text("待定价"))
                TextField("缓存读取", text: $cacheRead, prompt: Text("待定价")); TextField("缓存写入", text: $cacheWrite, prompt: Text("待定价"))
                if !valid { Text("价格须为大于或等于 0 的数字；留空表示待定价。").foregroundStyle(.orange) }
                if let error { Text(error).foregroundStyle(.orange).textSelection(.enabled).accessibilityLabel("保存失败，" + error) }
            }.formStyle(.grouped).disabled(saving)
            HStack {
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction).disabled(saving)
                Spacer()
                Button(saving ? "保存中…" : "保存价格") { Task { await save() } }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(saving || !valid || price.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }.padding(20)
        }.font(AppFont.body).frame(width: 520, height: EditorLayout.height(470)).interactiveDismissDisabled(saving)
            .onAppear { input = price.input.map { String($0 * 1e6) } ?? ""; output = price.output.map { String($0 * 1e6) } ?? ""; cacheRead = price.cacheRead.map { String($0 * 1e6) } ?? ""; cacheWrite = price.cacheWrite.map { String($0 * 1e6) } ?? "" }
    }
    private func save() async {
        guard !saving else { return }
        saving = true; error = nil; defer { saving = false }
        var next = price
        next.id = next.id.trimmingCharacters(in: .whitespacesAndNewlines)
        next.input = Double(input).map { $0 / 1e6 }; next.output = Double(output).map { $0 / 1e6 }
        next.cacheRead = Double(cacheRead).map { $0 / 1e6 }; next.cacheWrite = Double(cacheWrite).map { $0 / 1e6 }
        do { try await onSave(next) } catch { self.error = error.localizedDescription }
    }
}

enum EditorLayout {
    static func height(_ preferred: CGFloat) -> CGFloat { min(preferred, max(350, ((NSApp.keyWindow?.screen ?? NSScreen.main)?.visibleFrame.height ?? 800) - 110)) }
}
