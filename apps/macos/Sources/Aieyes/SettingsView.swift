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
    @AppStorage("menu.showCount") private var showMenuCount = false
    @State private var removal: RemovalRequest?
    @State private var confirmDiscardConfig = false
    private var mappingModel: String { get { model.mappingModel } nonmutating set { model.mappingModel = newValue } }
    private var mappingID: String { get { model.mappingID } nonmutating set { model.mappingID = newValue } }
    @FocusState private var mappingFocused: Bool
    @MainActor init(model: AppModel) {
        self.model = model
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    if model.settingsSaving { TimelineView(.periodic(from: .now, by: 1)) { context in Text(model.settingsStage + " · " + String(Int(context.date.timeIntervalSince(model.settingsStartedAt ?? context.date))) + " 秒").font(AppFont.secondary).foregroundStyle(.secondary) } }
                    else { Text(!model.settingsLoaded ? "正在读取配置…" : model.settingsDirty ? "应用配置 · 未保存 \(model.settingsChangeCount) 项" : "应用配置 · 已保存").font(AppFont.secondary).foregroundStyle(.secondary) }
                    Text("编辑器更新草稿，顶部保存全部；价格与定时唤醒独立保存。").font(.system(size: 12)).foregroundStyle(.secondary)
                }
                Spacer()
                if !model.settingsLoaded { Button("重试读取") { Task { await model.bootstrap() } } }
                if model.settingsSaving { ProgressView().controlSize(.small) }
                Button("放弃更改") { confirmDiscardConfig = true }.disabled(!model.settingsDirty || model.settingsSaving || model.repricing)
                Button(model.settingsSaving ? "保存中…" : "保存应用配置") { Task { _ = await model.saveSettingsDraft() } }.buttonStyle(.borderedProminent).keyboardShortcut("s").disabled(!model.settingsDirty || model.settingsSaving || model.repricing)
            }.padding(.horizontal, 22).padding(.vertical, 12)
            if let message = model.settingsMessage ?? model.message { HStack { Text(message).font(AppFont.secondary).textSelection(.enabled); Spacer(); Button { model.settingsMessage = nil; model.message = nil } label: { Image(systemName: "xmark") }.buttonStyle(.plain).accessibilityLabel("关闭提示") }.padding(.horizontal, 22).padding(.bottom, 10) }
            Picker("设置分类", selection: $model.settingsTab) {
                Text("数据源").tag("sources"); Text("服务器").tag("servers"); Text("价格").tag("prices"); Text("定时唤醒").tag("wakeups"); Text("通用").tag("general")
            }.pickerStyle(.segmented).labelsHidden().padding(.horizontal, 22).padding(.bottom, 12)
            Divider().opacity(0.5)
            Group {
                switch model.settingsTab {
                case "wakeups": WakeupsView(model: model)
                case "sources", "accounts": sources
                case "servers": servers
                case "prices": prices
                case "general", "connection": general
                default: sources
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top).padding(.horizontal, 16).padding(.bottom, 16).disabled(model.settingsSaving || model.repricing || !model.settingsLoaded)
        }
        .font(AppFont.body).disabled(model.installingUpdate)
        .discardDraftConfirmation($confirmDiscardConfig) { model.discardSettingsDraft() }
        .frame(minWidth: 620, idealWidth: 760, minHeight: 440, idealHeight: 600).tint(Palette.accent)
        .onAppear { if model.settingsTab == "accounts" { model.settingsTab = "sources" }; if model.settingsTab == "connection" { model.settingsTab = "general" }; Task { await model.loadPrices() }; consumeEditorRequest() }
        .onChange(of: model.requestedSourceProvider) { _, _ in consumeEditorRequest() }
        .onChange(of: model.requestedAccountKey) { _, _ in consumeEditorRequest() }
        .onChange(of: model.requestHostEditor) { _, _ in consumeEditorRequest() }
        .sheet(item: $removal) { request in
            DangerConfirmation(title: request.title, explanation: request.explanation, affected: request.affected, confirmLabel: request.kind == "account" ? "归档并保留历史" : "确认移除", cancel: { removal = nil }, confirm: { model.applyRemoval(request); removal = nil })
        }
        .sheet(item: $sourceEditor) { source in
            SourceEditor(source: source, hosts: draft.hosts, accounts: draft.accounts, sources: draft.sources, pendingAPIKey: model.pendingAPIKeys[source.id] ?? "", appProxy: model.settingsDraft.proxy, testURLs: model.settingsDraft.proxyTestUrls) { item, account, apiKey in
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
        .sheet(item: $hostEditor) { host in HostEditor(host: host, password: model.pendingHostPasswords[host.id] ?? "") { item, password in
            if let i = draft.hosts.firstIndex(where: { $0.id == item.id }) { draft.hosts[i] = item } else { draft.hosts.append(item) }
            model.stageHostPassword(password, hostID: item.id); hostEditor = nil
        } }
        .sheet(item: $priceEditor) { price in PriceEditor(price: price) { item in
            guard await model.savePrice(item) else { throw ClientError.message(model.message ?? "保存失败") }
            priceEditor = nil
        } }
    }
    private func consumeEditorRequest() {
        if let key = model.requestedAccountKey { accountEditor = draft.accounts.first { $0.key == key }; model.requestedAccountKey = nil }
        if let provider = model.requestedSourceProvider {
            sourceEditor = AgentSource(provider: provider, path: provider == "codex" ? "~/.codex" : provider == "claude" ? "~/.claude" : "")
            model.requestedSourceProvider = nil
        }
        if model.requestHostEditor { hostEditor = Host(); model.requestHostEditor = false }
    }
    private var sources: some View {
        VStack(spacing: 12) {
            Form {
                ForEach(draft.accounts.filter { $0.archived != true }, id: \.key) { account in
                    Section {
                        accountHeader(account)
                        ForEach($model.settingsDraft.sources) { $source in if source.accountId == account.id && source.provider == account.provider { sourceRow($source) } }
                    }
                }
                Section("未关联账户的来源") { ForEach($model.settingsDraft.sources) { $source in if source.accountId.isEmpty || !draft.accounts.contains(where: { $0.id == source.accountId && $0.provider == source.provider && $0.archived != true }) { sourceRow($source) } } }
                if !unlinkedAccounts.isEmpty {
                    DisclosureGroup("未关联账户") { ForEach(unlinkedAccounts, id: \.key) { accountRow($0) } }
                }
                let archived = draft.accounts.filter { $0.archived == true }
                if !archived.isEmpty {
                    DisclosureGroup("已归档账户") { ForEach(archived, id: \.key) { accountRow($0) } }
                }
            }.formStyle(.grouped)
            HStack { Button("添加数据源", systemImage: "plus") { sourceEditor = AgentSource() }; Spacer(); Button("保存并同步记录") { Task { if await model.saveSettingsDraft() { await model.scan() } } }.disabled(model.busy) }.padding(.horizontal, 8)
        }.padding(.vertical, 12)
    }
    private func accountHeader(_ account: AgentAccount) -> some View {
        let sourceCount = draft.sources.filter { $0.accountId == account.id && $0.provider == account.provider }.count
        return HStack {
            ProviderMark(provider: account.provider)
            VStack(alignment: .leading) {
                Text(account.name).font(AppFont.section)
                Text(Format.provider(account.provider)).font(AppFont.secondary).foregroundStyle(.secondary)
                Text("共用 \(sourceCount) 个来源 · 账户名称与限额设置共同生效").font(AppFont.secondary).foregroundStyle(.secondary)
            }
            Spacer()
            Button("编辑账户") { accountEditor = account }.accessibilityLabel("编辑账户 " + account.name)
        }
    }
    private func sourceRow(_ binding: Binding<AgentSource>) -> some View {
        let source = binding.wrappedValue, saved = model.settings.sources.first { $0.id == source.id }
        return HStack(spacing: 12) {
            Toggle("启用 " + source.name, isOn: binding.enabled).labelsHidden().toggleStyle(.switch).controlSize(.small)
            ProviderMark(provider: source.provider)
            VStack(alignment: .leading, spacing: 4) {
                Text(source.name).font(AppFont.body)
                Text(Format.provider(source.provider)).font(AppFont.secondary).foregroundStyle(.secondary)
                Text(source.hostId.flatMap { id in draft.hosts.first { $0.id == id }?.name } ?? "本机").font(AppFont.secondary).foregroundStyle(.secondary)
                if saved != source { Text("待保存：" + (saved?.enabled == source.enabled ? "配置修改" : source.enabled ? "启用" : "停用") + " · " + (saved?.enabled == true ? "当前仍运行" : "当前未运行")).font(AppFont.secondary).foregroundStyle(Palette.warn) }
                if let status = model.dashboard.sources.first(where: { $0.id == source.id })?.status { Text(status.error ?? ("记录同步于 " + Format.date(status.updatedAt))).font(AppFont.secondary).foregroundStyle(status.error == nil ? Color.secondary : Palette.warn) }
            }
            Spacer(); Button("编辑") { sourceEditor = source }.accessibilityLabel("编辑来源 " + source.name)
            Button { removal = model.removalRequest("source", id: source.id) } label: { Image(systemName: "minus.circle").frame(width: 32, height: 32) }.buttonStyle(.borderless).accessibilityLabel("移除来源 " + source.name)
        }.padding(.vertical, 6)
    }
    private var unlinkedAccounts: [AgentAccount] {
        draft.accounts.filter { account in account.archived != true && !draft.sources.contains { $0.accountId == account.id && $0.provider == account.provider } }
    }
    private func accountRow(_ account: AgentAccount) -> some View {
        HStack {
            ProviderMark(provider: account.provider); VStack(alignment: .leading) { Text(account.name); Text(Format.provider(account.provider)).font(AppFont.secondary).foregroundStyle(.secondary) }; Spacer(); Button("编辑") { accountEditor = account }
            Button(account.archived == true ? "恢复" : "归档") {
                if account.archived == true {
                    if let i = draft.accounts.firstIndex(where: { $0.key == account.key }) { draft.accounts[i].archived = false }
                } else { removal = model.removalRequest("account", id: account.key) }
            }
        }.padding(.vertical, 6)
    }
    private var servers: some View {
        VStack(spacing: 12) {
            Form {
                ForEach($model.settingsDraft.hosts) { $host in HStack(spacing: 12) {
                    Toggle("启用 " + (host.name.isEmpty ? host.target : host.name), isOn: $host.enabled).labelsHidden().toggleStyle(.switch).controlSize(.small)
                    VStack(alignment: .leading, spacing: 5) { Text(host.name.isEmpty ? host.target : host.name).font(AppFont.section); Text(host.target).font(AppFont.secondary).foregroundStyle(.secondary); if model.settings.hosts.first(where: { $0.id == host.id }) != host { Text("待保存：" + (host.enabled ? "启用" : "停用") + " · " + (model.settings.hosts.first(where: { $0.id == host.id })?.enabled == true ? "当前仍运行" : "当前未运行")).font(AppFont.secondary).foregroundStyle(Palette.warn) } }
                    Spacer(); Button("编辑") { hostEditor = host }.accessibilityLabel("编辑服务器 " + (host.name.isEmpty ? host.target : host.name))
                    Button { removal = model.removalRequest("host", id: host.id) } label: { Image(systemName: "minus.circle") }.buttonStyle(.borderless).help("移除主机").accessibilityLabel("移除服务器 " + (host.name.isEmpty ? host.target : host.name))
                }.padding(.vertical, 7) }
            }.formStyle(.grouped)
            HStack { Button("添加 SSH 主机", systemImage: "plus") { hostEditor = Host() }; Spacer(); Button("保存并测试采样") { Task { if await model.saveSettingsDraft() { await model.sampleHosts(); if let e = model.hosts.first(where: { $0.error != nil })?.error { model.message = e } else { model.message = "采样完成" } } } }.disabled(model.serverBusy) }.padding(.horizontal, 8)
        }.padding(.vertical, 12)
    }
    private func applyMappingDraft() -> Bool {
        let from = mappingModel.trimmingCharacters(in: .whitespacesAndNewlines), to = mappingID.trimmingCharacters(in: .whitespacesAndNewlines)
        if from.isEmpty && to.isEmpty { return true }
        guard !from.isEmpty && !to.isEmpty else { model.settingsMessage = "请补全模型映射两端，输入已保留"; mappingFocused = true; return false }
        guard draft.modelMappings[from] == nil || draft.modelMappings[from] == to else { model.settingsMessage = "该模型已有映射，请先移除原映射再添加新值"; return false }
        draft.modelMappings[from] = to
        return true
    }
    private var filteredPrices: [ModelPrice] {
        let query = search.trimmingCharacters(in: .whitespacesAndNewlines)
        return model.prices.filter { query.isEmpty || $0.id.localizedCaseInsensitiveContains(query) || $0.name.localizedCaseInsensitiveContains(query) }
    }
    private var prices: some View {
        ScrollViewReader { reader in Form {
            Section {
                Text("价格条目立即生效；映射属于应用配置。“保存并重算”包含当前映射输入。唤醒任务独立保存。").font(AppFont.secondary).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading)
                HStack { Text("USD / 百万 Token").font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Button(model.repricing ? "重算中…" : "保存并重算") { Task { await model.saveAndReprice() } }.disabled(model.settingsSaving || model.repricing).accessibilityIdentifier("save-and-reprice") }
            }
            if !model.dashboard.pricingGaps.isEmpty {
                Section("所选时间范围 · 待计价模型") {
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
            Section("模型映射") {
                VStack(spacing: 8) {
                    HStack { TextField("日志中的模型名称", text: $model.mappingModel); Image(systemName: "arrow.right"); TextField("OpenRouter 模型 ID", text: $model.mappingID).focused($mappingFocused); Button("添加映射") { if applyMappingDraft() { mappingModel = ""; mappingID = "" } }.disabled(mappingModel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || mappingID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
                    ScrollView { LazyVStack(spacing: 8) { ForEach(draft.modelMappings.keys.sorted(), id: \.self) { key in HStack { Text(key); Image(systemName: "arrow.right"); Text(draft.modelMappings[key] ?? ""); Spacer(); Button { draft.modelMappings.removeValue(forKey: key) } label: { Image(systemName: "minus.circle") }.buttonStyle(.plain) }.font(AppFont.secondary) } }.padding(.vertical, 4) }.frame(maxHeight: 100)
                }.padding(6)
            }.id("model-mapping")
            HStack {
                TextField("搜索模型", text: $search).textFieldStyle(.roundedBorder)
                Button("同步 OpenRouter") { Task { await model.syncPrices() } }.disabled(model.busy)
                Button("添加价格") { priceEditor = ModelPrice() }
            }
            Section("模型价格") { ForEach(filteredPrices) { price in
                HStack { VStack(alignment: .leading, spacing: 4) { Text(price.id).font(AppFont.body); Text("输入 \(price.input.map { Format.money($0 * 1e6) } ?? "—") · 输出 \(price.output.map { Format.money($0 * 1e6) } ?? "—") / 百万 Token").font(AppFont.secondary).foregroundStyle(.secondary) }; Spacer(); Button("一键映射") { mappingID = price.id; reader.scrollTo("model-mapping", anchor: .top); mappingFocused = true }; Button("编辑") { priceEditor = price } }
            } }.overlay { if filteredPrices.isEmpty { Text(search.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "同步模型价格" : "无匹配模型").foregroundStyle(.secondary) } }
        }.formStyle(.grouped).textFieldStyle(.roundedBorder) }
    }
    private var general: some View {
        Form {
            Section("菜单栏") {
                Toggle("显示活跃会话计数（本机，立即生效）", isOn: $showMenuCount)
                Text("菜单栏口径：今日全部数据，不受面板和详情筛选影响。").font(AppFont.secondary).foregroundStyle(.secondary)
                Picker("显示内容", selection: $model.settingsDraft.menuMetric) { Text("图标").tag("icon"); Text("今日全部 Token").tag("tokens"); Text("首个账户剩余额度").tag("quota"); Text("首台服务器 CPU").tag("cpu") }
            }
            Section("刷新") {
                TextField("Agent 间隔（秒）", value: $model.settingsDraft.refreshSeconds, format: .number)
                TextField("服务器后台间隔（秒）", value: $model.settingsDraft.serverRefreshSeconds, format: .number)
            }
            Section("连接") {
                ProxyFields(proxy: $model.settingsDraft.proxy, testURLs: model.settingsDraft.proxyTestUrls)
                TextField("测试地址（逗号分隔）", text: Binding(get: { (model.settingsDraft.proxyTestUrls ?? ["https://api.github.com/rate_limit", "https://openrouter.ai"]).joined(separator: ", ") }, set: { model.settingsDraft.proxyTestUrls = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty } }))
            }
            Section("更新") { UpdateSettingsView() }
        }.formStyle(.grouped)
    }
}

struct ProxyFields: View {
    @Binding var proxy: ProxySettings
    var inherit = false
    @State private var address: ProxyAddressDraft
    var inheritedProxy = ProxySettings()
    var testURLs: [String]?
    @State private var result: NetworkTest?
    @State private var testing = false
    @State private var testError: String?
    @State private var testEngine = EngineClient()
    init(proxy: Binding<ProxySettings>, inherit: Bool = false, inheritedProxy: ProxySettings = ProxySettings(), testURLs: [String]? = nil) {
        _proxy = proxy; self.inherit = inherit; self.inheritedProxy = inheritedProxy; self.testURLs = testURLs
        _address = State(initialValue: ProxyAddressDraft(url: proxy.wrappedValue.url))
    }
    private func testConnection() async {
        testing = true; testError = nil
        defer { testing = false }
        do {
            let effective = proxy.mode == "inherit" ? inheritedProxy : proxy
            let data = try JSONEncoder().encode(effective)
            var params: [String: Any] = ["proxy": try JSONSerialization.jsonObject(with: data), "force":true]
            if let testURLs { params["urls"] = testURLs }
            result = try await testEngine.call("network.test", params: params)
        } catch { testError = error.localizedDescription }
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
            HStack {
                Button(testing ? "测试中…" : "测试连接") { Task { await testConnection() } }.disabled(testing)
                if let result { Text(result.label).help(result.detail).foregroundStyle(result.status == "ok" ? Color.secondary : Color.orange) }
            }
            if let testError { Text(testError).foregroundStyle(.orange) }
        }
        .disabled(testing)
        .onChange(of: address) { _, _ in result = nil; testError = nil }
        .onChange(of: proxy.mode) { _, _ in result = nil; testError = nil }
        .onChange(of: testURLs) { _, _ in result = nil; testError = nil }
        .onChange(of: address.scheme) { old, scheme in
            if scheme == "url", old != "url" { var prior = address; prior.scheme = old; address.customURL = prior.url }
        }
        .onChange(of: address) { _, value in proxy.url = value.url }
        .onChange(of: proxy.mode) { _, mode in if mode == "custom" { proxy.url = address.url } }
    }
}

struct SourceEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var initialDraft = ""
    @State private var confirmDiscard = false
    private var draftValue: String { draftSnapshot(source) + draftSnapshot(account) + accountChoice + String(asAccount) + apiKey }
    private func cancel() { if !initialDraft.isEmpty && initialDraft != draftValue { confirmDiscard = true } else { dismiss() } }
    @State var source: AgentSource
    @State private var asAccount: Bool
    @State private var accountChoice: String
    @State private var account: AgentAccount
    @State private var apiKey = ""
    @State private var saving = false
    @State private var error: String?
    var hosts: [Host], accounts: [AgentAccount], sources: [AgentSource]
    var appProxy = ProxySettings(), testURLs: [String]?
    var onSave: (AgentSource, AgentAccount?, String?) async throws -> Void
    init(source: AgentSource, hosts: [Host], accounts: [AgentAccount], sources: [AgentSource] = [], pendingAPIKey: String = "", appProxy: ProxySettings = ProxySettings(), testURLs: [String]? = nil, onSave: @escaping (AgentSource, AgentAccount?, String?) async throws -> Void) {
        _apiKey = State(initialValue: pendingAPIKey)
        _source = State(initialValue: source); self.hosts = hosts; self.accounts = accounts; self.sources = sources; self.onSave = onSave; self.appProxy = appProxy; self.testURLs = testURLs
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
                DisclosureGroup("高级连接选项 · CLI / 代理 / 前置命令") {
                if source.provider == "agy" { TextField("agy 程序", text: Binding(get: { source.agyBinary ?? "agy" }, set: { source.agyBinary = $0 })) }
                if source.provider == "codex" { TextField("Codex 程序", text: $source.codexBinary) }
                if source.hostId != nil && asAccount {
                    Section("限额查询前置命令") { TextEditor(text: $source.quotaPreCommand).font(.system(size: 14, design: .monospaced)).frame(height: 75).help("留空继承主机前置命令") }
                }
                if source.hostId == nil {
                    ProxyFields(proxy: Binding(get: { source.proxy ?? ProxySettings(mode: "inherit") }, set: { source.proxy = $0.mode == "inherit" ? nil : $0 }), inherit: true, inheritedProxy: appProxy, testURLs: testURLs)
                }
                }
                if let error { Text(error).foregroundStyle(Palette.warn) }
            }.formStyle(.grouped).textFieldStyle(.roundedBorder).disabled(saving)
            HStack {
                Button("取消") { cancel() }.keyboardShortcut(.cancelAction).disabled(saving); Spacer(); Text("应用后仍需保存应用配置").font(AppFont.secondary).foregroundStyle(.secondary); Spacer()
                Button(saving ? "处理中…" : "应用到草稿") { Task { await save() } }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(saving)
            }.padding(20)
        }.font(AppFont.body).frame(width: 570, height: EditorLayout.height(660))
        .interactiveDismissDisabled(saving || initialDraft != draftValue)
        .onAppear { if initialDraft.isEmpty { initialDraft = draftValue } }
        .discardDraftConfirmation($confirmDiscard) { dismiss() }
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
    @State private var initialDraft = ""
    @State private var confirmDiscard = false
    private var draftValue: String { draftSnapshot(account) }
    private func cancel() { if !initialDraft.isEmpty && initialDraft != draftValue { confirmDiscard = true } else { dismiss() } }
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
                if let error { Text(error).foregroundStyle(Palette.warn) }
            }.formStyle(.grouped).textFieldStyle(.roundedBorder).disabled(saving)
            HStack {
                Button("取消") { cancel() }.keyboardShortcut(.cancelAction).disabled(saving); Spacer(); Text("应用后仍需保存应用配置").font(AppFont.secondary).foregroundStyle(.secondary); Spacer()
                Button("应用到草稿") { Task {
                    saving = true; error = nil; defer { saving = false }
                    do { try await onSave(account) } catch { self.error = error.localizedDescription }
                } }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(saving || account.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }.padding(20)
        }.font(AppFont.body).frame(width: 560, height: EditorLayout.height(420)).interactiveDismissDisabled(saving || initialDraft != draftValue)
        .onAppear { if initialDraft.isEmpty { initialDraft = draftValue } }
        .discardDraftConfirmation($confirmDiscard) { dismiss() }
    }
}

struct HostEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var initialDraft = ""
    @State private var confirmDiscard = false
    private var draftValue: String { draftSnapshot(host) + password + devices }
    private func cancel() { if !initialDraft.isEmpty && initialDraft != draftValue { confirmDiscard = true } else { dismiss() } }
    @State var host: Host
    @State var password = ""
    var onSave: (Host, String) -> Void
    @State private var devices = ""
    @State private var discovered: [String: [DeviceMetric]] = [:]
    @State private var discovering = false
    @State private var discoveryRequest = UUID()
    @State private var discoveryInfo: String?
    private var connectionIdentity: String { host.connectionIdentity + password }
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
        discovering = true; defer { discovering = false }; discoveryError = nil; discoveryInfo = nil
        let request = UUID(), identity = connectionIdentity; discoveryRequest = request
        do {
            var testingHost = host
            var temporaryReference: String?
            if host.authMode == "password", !password.isEmpty {
                let saved: [String: String] = try await discoveryEngine.call("hosts.credentials.save", params: ["password": password])
                temporaryReference = saved["passwordRef"]; testingHost.passwordRef = temporaryReference
            }
            defer { if let reference = temporaryReference { Task { let _: Acknowledgement? = try? await discoveryEngine.call("hosts.credentials.delete", params: ["passwordRef": reference]) } } }
            let data = try JSONEncoder().encode(testingHost)
            let object = try JSONSerialization.jsonObject(with: data)
            let sample: MetricSample = try await discoveryEngine.call("hosts.discover", params: ["host": object])
            guard request == discoveryRequest, identity == connectionIdentity else { return }
            discoveryInfo = "从 " + testingHost.target + " 读取 · " + Format.time(Date().timeIntervalSince1970)
            for (group, rows) in [("cpu",sample.cpu),("gpu",sample.gpu),("filesystems",sample.filesystems),("disk",sample.disk),("network",sample.network)] {
                if let rows { discovered[group] = rows }
            }
            let failed = sample.errors.keys.sorted().map { key in (groups.first { $0.0 == key }?.1 ?? key) + "读取失败" }
            discoveryError = failed.isEmpty ? nil : failed.joined(separator: " · ")
        } catch { if request == discoveryRequest, identity == connectionIdentity { discoveryError = error.localizedDescription } }
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
                Picker("登录方式", selection: Binding(get: { host.authMode ?? "ssh" }, set: { host.authMode = $0 })) {
                    Text("SSH 配置 / 密钥").tag("ssh"); Text("账号密码").tag("password")
                }
                TextField("用户名", text: Binding(get: { host.username ?? "" }, set: { host.username = $0 }), prompt: Text("跟随地址或 SSH 配置"))
                if host.authMode == "password" { SecureField("密码（留空保留）", text: $password) }
                else { HStack { TextField("密钥路径", text: $host.identityFile, prompt: Text("跟随 SSH 配置")); Button("选择…") { let panel = NSOpenPanel(); panel.showsHiddenFiles = true; if panel.runModal() == .OK { host.identityFile = panel.url?.path ?? host.identityFile } } } }
                Button(discovering ? "连接中…" : "连接并读取设备") { Task { await discover() } }.disabled(discovering || host.target.isEmpty)
                DisclosureGroup("监控指标与设备 · 按需选择") {
                Section {
                    MultiSelectPicker(title: "采集项目", options: groups.map { SelectionOption(id: $0.0, label: $0.1) }, selected: Set(host.metrics)) { selected, _ in
                        host.metrics = selected.sorted()
                    }
                }
                Section("设备") {
                    if let discoveryInfo { Text(discoveryInfo).font(AppFont.secondary).foregroundStyle(.secondary) }
                    if let discoveryError { Text(discoveryError).font(AppFont.secondary).foregroundStyle(Palette.warn) }
                    ForEach(groups.filter { $0.0 != "memory" }, id: \.0) { group, label in devicePicker(group, label) }
                }
                Section {
                    MultiSelectPicker(title: "显示细分项", options: Host.detailOptions.filter { host.metrics.contains(detailGroups[$0.0] ?? "") }.map { SelectionOption(id: $0.0, label: $0.1) }, selected: Set(host.details ?? Host.detailOptions.map { $0.0 })) { selected, _ in
                        host.details = selected.sorted()
                    }.disabled(host.metrics.isEmpty)
                }
                }
                DisclosureGroup("高级设置") {
                    TextField("远程 shell", text: $host.shell)
                    Text("远程前置命令").foregroundStyle(.secondary)
                    TextEditor(text: $host.preCommand).font(.system(size: 14, design: .monospaced)).frame(height: 65)
                    TextField("设备表达式", text: $devices, prompt: Text("network:eth0, gpu:0, filesystems:/")).help("留空显示全部设备")
                }
            }.formStyle(.grouped).textFieldStyle(.roundedBorder)
            HStack { Button("取消") { cancel() }.keyboardShortcut(.cancelAction); Spacer(); Text("应用后仍需保存应用配置").font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Button("应用到草稿") { host.devices = MonitorSelection.parse(devices); if host.name.isEmpty { host.name = host.target }; onSave(host, password) }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(host.target.isEmpty) }.padding(20)
        }.font(AppFont.body).frame(width: 620, height: EditorLayout.height(650)).onChange(of: connectionIdentity) { _, _ in discoveryRequest = UUID(); discovered = [:]; discoveryInfo = nil; discoveryError = "连接配置已更改，请重新读取设备" }
        .onAppear { devices = host.devices.joined(separator: ", "); initialDraft = draftValue }
        .interactiveDismissDisabled(initialDraft != draftValue)
        .discardDraftConfirmation($confirmDiscard) { dismiss() }
    }
}

struct PriceEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var initialDraft = ""
    @State private var confirmDiscard = false
    private var draftValue: String { draftSnapshot(price) + [input, output, cacheRead, cacheWrite].joined(separator: "|") }
    private func cancel() { if !initialDraft.isEmpty && initialDraft != draftValue { confirmDiscard = true } else { dismiss() } }
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
            }.padding(.horizontal, 22).padding(.vertical, 12)
            Form {
                TextField("模型 ID", text: $price.id, prompt: Text("openai/model-name"))
                TextField("显示名称", text: $price.name)
                TextField("输入", text: $input, prompt: Text("待定价")); TextField("输出", text: $output, prompt: Text("待定价"))
                TextField("缓存读取", text: $cacheRead, prompt: Text("待定价")); TextField("缓存写入", text: $cacheWrite, prompt: Text("待定价"))
                if !valid { Text("价格须为大于或等于 0 的数字；留空表示待定价。").foregroundStyle(Palette.warn) }
                if let error { Text(error).foregroundStyle(Palette.warn).textSelection(.enabled).accessibilityLabel("保存失败，" + error) }
            }.formStyle(.grouped).textFieldStyle(.roundedBorder).disabled(saving)
            HStack {
                Button("取消") { cancel() }.keyboardShortcut(.cancelAction).disabled(saving)
                Spacer()
                Button(saving ? "保存中…" : "保存价格") { Task { await save() } }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(saving || !valid || price.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }.padding(20)
        }.font(AppFont.body).frame(width: 520, height: EditorLayout.height(470)).interactiveDismissDisabled(saving || initialDraft != draftValue)
            .onAppear { input = price.input.map { String($0 * 1e6) } ?? ""; output = price.output.map { String($0 * 1e6) } ?? ""; cacheRead = price.cacheRead.map { String($0 * 1e6) } ?? ""; cacheWrite = price.cacheWrite.map { String($0 * 1e6) } ?? ""; initialDraft = draftValue }
            .discardDraftConfirmation($confirmDiscard) { dismiss() }
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

struct DangerConfirmation: View {
    var title: String, explanation: String, affected: [String], confirmLabel: String
    var cancel: () -> Void, confirm: () -> Void
    @FocusState private var cancelFocused: Bool
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(title).font(AppFont.section)
            Text(explanation).font(AppFont.body).fixedSize(horizontal: false, vertical: true)
            if !affected.isEmpty { ScrollView { VStack(alignment: .leading, spacing: 8) { ForEach(Array(affected.enumerated()), id: \.offset) { _, name in Text(name).frame(maxWidth: .infinity, alignment: .leading) } } }.frame(maxHeight: 180) }
            HStack { Spacer(); Button("取消", action: cancel).focused($cancelFocused).keyboardShortcut(.cancelAction); Button(confirmLabel, role: .destructive, action: confirm) }
        }.padding(24).frame(width: 460).onAppear { cancelFocused = true }
    }
}

func draftSnapshot<T: Encodable>(_ value: T) -> String {
    let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
    return (try? encoder.encode(value)).flatMap { String(data: $0, encoding: .utf8) } ?? ""
}
extension View {
    func discardDraftConfirmation(_ presented: Binding<Bool>, discard: @escaping () -> Void) -> some View {
        alert("放弃未保存的编辑？", isPresented: presented) {
            Button("继续编辑", role: .cancel) { }
            Button("放弃更改", role: .destructive, action: discard)
        } message: { Text("此编辑器的输入尚未保存，放弃后无法恢复。") }
    }
}
