import SwiftUI

struct SettingsView: View {
    @ObservedObject var model: AppModel
    @State private var draft = Settings()
    @State private var sourceEditor: AgentSource?
    @State private var accountEditor: AgentAccount?
    @State private var hostEditor: Host?
    @State private var priceEditor: ModelPrice?
    @State private var saving = false
    @State private var search = ""
    @State private var mappingModel = ""
    @State private var mappingID = ""
    @MainActor init(model: AppModel) {
        self.model = model
        _draft = State(initialValue: model.settings)
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("设置").font(.title2.weight(.semibold)); Spacer(); if saving { ProgressView().controlSize(.small) }; Button("保存") { Task { saving = true; _ = await model.save(draft); saving = false } }.buttonStyle(.borderedProminent).keyboardShortcut("s").disabled(saving) }.padding(22)
            if let message = model.message { HStack { Text(message).font(.caption); Spacer(); Button { model.message = nil } label: { Image(systemName: "xmark") }.buttonStyle(.plain) }.padding(.horizontal, 22).padding(.bottom, 10) }
            HStack(spacing: 6) {
                ForEach([("accounts", "账户", "person.crop.circle"), ("sources", "数据源", "tray.full"), ("servers", "服务器", "server.rack"), ("connection", "连接", "network"), ("prices", "价格", "dollarsign.circle"), ("general", "通用", "slider.horizontal.3")], id: \.0) { key, title, icon in
                    Button { model.settingsTab = key } label: {
                        Label(title, systemImage: icon).font(.system(size: 13, weight: model.settingsTab == key ? .semibold : .regular)).frame(maxWidth: .infinity).padding(.vertical, 10)
                            .foregroundStyle(model.settingsTab == key ? Palette.accent : .secondary)
                            .background(model.settingsTab == key ? Palette.accent.opacity(0.1) : Color.clear, in: RoundedRectangle(cornerRadius: 9))
                    }.buttonStyle(.plain).accessibilityValue(model.settingsTab == key ? "已选中" : "")
                }
            }.padding(.horizontal, 22).padding(.bottom, 12)
            Divider().opacity(0.5)
            Group {
                switch model.settingsTab {
                case "sources": sources
                case "servers": servers
                case "connection": connection
                case "prices": prices
                case "general": general
                default: accounts
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top).padding(.horizontal, 16).padding(.bottom, 16)
        }
        .frame(width: 760, height: 600).tint(Palette.accent)
        .onAppear { Task { await model.loadPrices() } }
        .sheet(item: $sourceEditor) { source in SourceEditor(source: source, hosts: draft.hosts, accounts: draft.accounts) { item in
            if let i = draft.sources.firstIndex(where: { $0.id == item.id }) { draft.sources[i] = item } else { draft.sources.append(item) }
            for i in draft.accounts.indices where draft.accounts[i].quotaSourceId == item.id && (draft.accounts[i].id != item.accountId || draft.accounts[i].provider != item.provider) { draft.accounts[i].quotaSourceId = nil }
            sourceEditor = nil
        } }
        .sheet(item: $accountEditor) { account in AccountEditor(account: account, sources: draft.sources) { item in
            if let i = draft.accounts.firstIndex(where: { $0.key == account.key }) { draft.accounts[i] = item } else { draft.accounts.append(item) }
            accountEditor = nil
        } }
        .sheet(item: $hostEditor) { host in HostEditor(host: host) { item in
            if let i = draft.hosts.firstIndex(where: { $0.id == item.id }) { draft.hosts[i] = item } else { draft.hosts.append(item) }
            hostEditor = nil
        } }
        .sheet(item: $priceEditor) { price in PriceEditor(price: price) { item in Task { if await model.savePrice(item) { priceEditor = nil } } } }
    }
    private var accounts: some View {
        VStack(alignment: .leading, spacing: 12) {
            List {
                ForEach(draft.accounts, id: \.key) { account in
                    HStack(spacing: 12) {
                        Image(systemName: "person.crop.circle").font(.title2).foregroundStyle(Palette.accent)
                        VStack(alignment: .leading, spacing: 5) {
                            Text(account.name).font(.headline)
                            Text("\(Format.provider(account.provider)) · \(draft.sources.filter { $0.provider == account.provider && $0.accountId == account.id }.count) 个数据源 · \(account.quotaEnabled ? "显示限额" : "仅统计用量")").font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer(); Button("编辑") { accountEditor = account }
                        Button { draft.accounts.removeAll { $0.key == account.key }; for i in draft.sources.indices where draft.sources[i].accountId == account.id && draft.sources[i].provider == account.provider { draft.sources[i].accountId = "" } } label: { Image(systemName: "minus.circle") }.buttonStyle(.borderless).help("移除账户并解除数据源关联")
                    }.padding(.vertical, 8)
                }
            }.listStyle(.inset)
            HStack { Button("添加账户", systemImage: "plus") { accountEditor = AgentAccount() }; Spacer(); Button("接入 agy") { addQuery("agy") }; Button("接入 DeepSeek") { addQuery("deepseek") } }.padding(.horizontal, 8)
        }.padding(.vertical, 12)
    }
    private func addQuery(_ provider: String) {
        let account = AgentAccount(name: Format.provider(provider), provider: provider)
        draft.accounts.append(account)
        sourceEditor = AgentSource(name: Format.provider(provider) + " · 本机", provider: provider, accountId: account.id, path: "")
    }
    private var sources: some View {
        VStack(spacing: 12) {
            List {
                ForEach($draft.sources) { $source in
                    HStack(spacing: 12) {
                        Toggle("启用", isOn: $source.enabled).labelsHidden().toggleStyle(.switch).controlSize(.small)
                        VStack(alignment: .leading, spacing: 5) { Text(source.name).font(.headline); Text(source.accountId.isEmpty ? "无账户 / 外接 API" : "账户 · " + (draft.accounts.first { $0.id == source.accountId && $0.provider == source.provider }?.name ?? source.accountId)).font(.caption).foregroundStyle(Palette.accent); Text("\(Format.provider(source.provider)) · \(source.hostId.flatMap { id in draft.hosts.first { $0.id == id }?.name } ?? "本机") · \(source.path)").font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle) }
                        Spacer()
                        Button("编辑") { sourceEditor = source }
                        Button { draft.sources.removeAll { $0.id == source.id }; for i in draft.accounts.indices where draft.accounts[i].quotaSourceId == source.id { draft.accounts[i].quotaSourceId = nil } } label: { Image(systemName: "minus.circle") }.buttonStyle(.borderless).help("移除数据源")
                    }.padding(.vertical, 7)
                }
            }.listStyle(.inset)
            HStack { Button("添加数据源", systemImage: "plus") { sourceEditor = AgentSource() }; Spacer(); Button("同步记录") { Task { if await model.save(draft) { await model.scan() } } }.disabled(model.busy) }.padding(.horizontal, 8)
        }.padding(.vertical, 12)
    }
    private var servers: some View {
        VStack(spacing: 12) {
            List {
                ForEach($draft.hosts) { $host in HStack(spacing: 12) {
                    Toggle("启用", isOn: $host.enabled).labelsHidden().toggleStyle(.switch).controlSize(.small)
                    VStack(alignment: .leading, spacing: 5) { Text(host.name.isEmpty ? host.target : host.name).font(.headline); Text(host.target).font(.caption).foregroundStyle(.secondary) }
                    Spacer(); Button("编辑") { hostEditor = host }
                    Button { draft.hosts.removeAll { $0.id == host.id }; for i in draft.sources.indices where draft.sources[i].hostId == host.id { draft.sources[i].hostId = nil; draft.sources[i].enabled = false } } label: { Image(systemName: "minus.circle") }.buttonStyle(.borderless).help("移除主机")
                }.padding(.vertical, 7) }
            }.listStyle(.inset)
            HStack { Button("添加 SSH 主机", systemImage: "plus") { hostEditor = Host() }; Spacer(); Button("测试采样") { Task { if await model.save(draft) { await model.sampleHosts(); if let e = model.hosts.first(where: { $0.error != nil })?.error { model.message = e } else { model.message = "采样完成" } } } }.disabled(model.serverBusy) }.padding(.horizontal, 8)
        }.padding(.vertical, 12)
    }
    private var connection: some View {
        Form {
            Section("应用代理") {
                Picker("连接方式", selection: $draft.proxy.mode) { Text("系统代理").tag("system"); Text("直连").tag("direct"); Text("自定义代理").tag("custom") }
                if draft.proxy.mode == "custom" { TextField("代理地址", text: $draft.proxy.url, prompt: Text("http://127.0.0.1:7890")) }
                LabeledContent("应用范围", value: "限额查询、模型价格、GitHub 更新")
            }
            Section("远程连接") { Text("SSH 使用已有的密钥、ssh-agent 与跳板配置。远程代理在数据源的「限额查询前置命令」中设置。").font(.callout).foregroundStyle(.secondary) }
        }.formStyle(.grouped)
    }
    private var prices: some View {
        VStack(spacing: 12) {
            if !model.dashboard.pricingGaps.isEmpty {
                GroupBox("所选时间范围 · 待计价模型") {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(model.dashboard.pricingGaps) { gap in
                                HStack {
                                    VStack(alignment: .leading, spacing: 4) { Text(gap.model).font(.system(size: 12, weight: .medium)); Text("缺少\(gap.missing) · \(Format.compact(gap.unpricedTokens)) Token").font(.caption).foregroundStyle(.secondary) }
                                    Spacer()
                                    Button("映射") { mappingModel = gap.model; mappingID = gap.priceId ?? "" }
                                    Button("补充价格") { priceEditor = model.prices.first { $0.id == (gap.priceId ?? gap.model) } ?? ModelPrice(id: gap.priceId ?? gap.model, name: gap.model) }
                                }
                            }
                        }.padding(6)
                    }.frame(maxHeight: 125)
                }
            }
            HStack {
                TextField("搜索模型", text: $search).textFieldStyle(.roundedBorder)
                Button("同步 OpenRouter") { Task { if await model.save(draft) { await model.syncPrices() } } }.disabled(model.busy)
                Button("添加价格") { priceEditor = ModelPrice() }
            }
            List(model.prices.filter { search.isEmpty || $0.id.localizedCaseInsensitiveContains(search) }) { price in
                HStack { VStack(alignment: .leading, spacing: 4) { Text(price.id).font(.callout); Text("输入 \(price.input.map { Format.money($0 * 1e6) } ?? "—") · 输出 \(price.output.map { Format.money($0 * 1e6) } ?? "—") / 百万 Token").font(.caption).foregroundStyle(.secondary) }; Spacer(); Button("编辑") { priceEditor = price } }
            }.listStyle(.inset)
            GroupBox("模型映射") {
                VStack(spacing: 8) {
                    HStack { TextField("日志中的模型名称", text: $mappingModel); Image(systemName: "arrow.right"); TextField("OpenRouter 模型 ID", text: $mappingID); Button("保存映射") { draft.modelMappings[mappingModel] = mappingID; mappingModel = ""; mappingID = ""; Task { _ = await model.save(draft) } }.disabled(mappingModel.isEmpty || mappingID.isEmpty) }
                    ForEach(draft.modelMappings.keys.sorted(), id: \.self) { key in HStack { Text(key); Image(systemName: "arrow.right"); Text(draft.modelMappings[key] ?? ""); Spacer(); Button { draft.modelMappings.removeValue(forKey: key) } label: { Image(systemName: "minus.circle") }.buttonStyle(.plain) }.font(.caption) }
                }.padding(6)
            }
            HStack { Text("USD / 百万 Token").font(.caption).foregroundStyle(.secondary); Spacer(); Button("按当前价格重算") { Task { if await model.save(draft) { await model.reprice() } } } }
        }.padding(12)
    }
    private var general: some View {
        Form {
            Section("菜单栏") {
                Picker("显示内容", selection: $draft.menuMetric) { Text("图标").tag("icon"); Text("当前范围 Token").tag("tokens"); Text("首个账号剩余额度").tag("quota"); Text("首台服务器 CPU").tag("cpu") }
            }
            Section("刷新") {
                TextField("Agent 间隔（秒）", value: $draft.refreshSeconds, format: .number)
                TextField("服务器后台间隔（秒）", value: $draft.serverRefreshSeconds, format: .number)
                LabeledContent("服务器面板展开时", value: "每 3 秒")
            }
            Section("更新") {
                TextField("GitHub 仓库", text: $draft.githubRepository, prompt: Text("owner/repo"))
                HStack { Text("Aieyes \(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "—")").foregroundStyle(.secondary); Spacer(); Button("检查更新") { Task {
                    if await model.save(draft) {
                        do { let update: UpdateInfo = try await model.engine.call("updates.check"); model.message = "最新版本 \(update.version)"; if let url = URL(string: update.url), url.host == "github.com" { NSWorkspace.shared.open(url) } }
                        catch { model.message = error.localizedDescription }
                    }
                } } }
            }
        }.formStyle(.grouped)
    }
}
struct UpdateInfo: Decodable { var version: String, url: String }

struct SourceEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State var source: AgentSource
    @State private var apiKey = ""
    @State private var savingKey = false
    @State private var error: String?
    private let credentialEngine = EngineClient()
    var hosts: [Host], accounts: [AgentAccount], onSave: (AgentSource) -> Void
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("数据源").font(.title2.weight(.semibold)); Spacer() }.padding(22)
            Form {
                TextField("名称", text: $source.name)
                Picker("Agent", selection: $source.provider) { Text("Codex").tag("codex"); Text("Claude Code").tag("claude"); Text("Antigravity · 导入记录").tag("antigravity"); Text("agy · 限额查询").tag("agy"); Text("DeepSeek · 余额").tag("deepseek"); Text("自定义").tag("custom") }
                Picker("关联账户", selection: $source.accountId) {
                    Text("无账户 / 外接 API").tag("")
                    ForEach(accounts.filter { $0.provider == source.provider }, id: \.key) { Text($0.name).tag($0.id) }
                }
                Text(["agy", "deepseek"].contains(source.provider) ? "选择要查询的账户。也可从「账户」页使用快捷接入按钮。" : "先在「账户」中添加身份。同一账户的本机和远程目录选择同一项；两个本机账户分别选择各自数据目录。修改关联后需同步记录。").font(.caption).foregroundStyle(.secondary)
                if source.provider != "deepseek" { Picker("位置", selection: Binding(get: { source.hostId ?? "local" }, set: { source.hostId = $0 == "local" ? nil : $0 })) { Text("本机").tag("local"); ForEach(hosts) { Text($0.name.isEmpty ? $0.target : $0.name).tag($0.id) } } }
                if source.provider == "deepseek" {
                    SecureField("API Key（留空保留）", text: $apiKey)
                    Text("查询可用余额、赠送余额和充值余额。已有 Key 不会回显，留空保留。").font(.caption).foregroundStyle(.secondary)
                    DisclosureGroup("高级设置") {
                        TextField("API Key 文件（可选）", text: $source.path)
                        Text("可选择包含 API Key 的文本文件。未指定时使用 DEEPSEEK_API_KEY 环境变量。").font(.caption).foregroundStyle(.secondary)
                    }
                } else if source.provider != "agy" { HStack { TextField("数据目录", text: $source.path); if source.hostId == nil { Button("选择") { let panel = NSOpenPanel(); panel.canChooseDirectories = true; panel.canChooseFiles = true; panel.showsHiddenFiles = true; if panel.runModal() == .OK, let url = panel.url { source.path = url.path } } } } }
                if source.provider == "agy" {
                    TextField("agy 程序", text: Binding(get: { source.agyBinary ?? "agy" }, set: { source.agyBinary = $0 }))
                    Text("读取此位置 agy 当前登录账户的 /usage；请先在终端登录 agy。此入口只查询限额。").font(.caption).foregroundStyle(.secondary)
                }
                if let error { Text(error).foregroundStyle(.orange) }
                if source.provider == "codex" { TextField("Codex 程序", text: $source.codexBinary) }
                if source.hostId != nil && !source.accountId.isEmpty {
                    Section("限额查询前置命令") {
                        TextEditor(text: $source.quotaPreCommand).font(.system(.callout, design: .monospaced)).frame(height: 75)
                        Text("例如：export HTTPS_PROXY=http://127.0.0.1:7890\n在远程限额查询的同一 shell 中执行，用于加载代理环境；留空继承主机前置命令。限额由应用自动读取。").font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                    }
                }
                if source.hostId == nil {
                    Picker("查询代理", selection: Binding(get: { source.proxy?.mode ?? "inherit" }, set: { source.proxy = $0 == "inherit" ? nil : ProxySettings(mode: $0, url: source.proxy?.url ?? "") })) { Text("跟随应用").tag("inherit"); Text("系统代理").tag("system"); Text("直连").tag("direct"); Text("自定义").tag("custom") }
                    if source.proxy?.mode == "custom" { TextField("代理地址", text: Binding(get: { source.proxy?.url ?? "" }, set: { source.proxy?.url = $0 })) }
                }
            }.formStyle(.grouped)
            HStack { Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Spacer(); Button(savingKey ? "保存中…" : "完成") { Task {
                savingKey = true; defer { savingKey = false }
                do {
                    if source.provider == "deepseek", !apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        let result: [String: String] = try await credentialEngine.call("credentials.save", params: ["sourceId": source.id, "apiKey": apiKey])
                        source.path = result["path"] ?? source.path; apiKey = ""
                    }
                    if source.name.isEmpty { source.name = Format.provider(source.provider) }; onSave(source)
                } catch { self.error = error.localizedDescription }
            } }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(savingKey || (["agy", "deepseek"].contains(source.provider) && source.accountId.isEmpty) || (source.path.isEmpty && !["agy", "deepseek"].contains(source.provider))) }.padding(20)
        }.frame(width: 570, height: 660)
        .onChange(of: source.provider) { _, p in source.accountId = ""; apiKey = ""; if p == "deepseek" { source.hostId = nil; source.path = "" }; if ["~/.codex", "~/.claude", ""].contains(source.path) { source.path = p == "codex" ? "~/.codex" : p == "claude" ? "~/.claude" : "" } }
    }
}

struct AccountEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State var account: AgentAccount
    var sources: [AgentSource], onSave: (AgentAccount) -> Void
    private var linked: [AgentSource] { sources.filter { $0.provider == account.provider && $0.accountId == account.id } }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("账户").font(.title2.weight(.semibold)); Spacer() }.padding(22)
            Form {
                TextField("账户名称", text: $account.name, prompt: Text("个人账户 / 工作账户"))
                Picker("Agent", selection: $account.provider) { Text("Codex").tag("codex"); Text("Claude Code").tag("claude"); Text("Antigravity").tag("antigravity"); Text("agy").tag("agy"); Text("DeepSeek").tag("deepseek"); Text("自定义").tag("custom") }.disabled(!linked.isEmpty)
                Toggle("显示并查询账户限额", isOn: $account.quotaEnabled)
                if account.quotaEnabled {
                    Picker("优先查询位置", selection: Binding(get: { account.quotaSourceId ?? "" }, set: { account.quotaSourceId = $0.isEmpty ? nil : $0 })) {
                        Text("自动 · 优先本机").tag("")
                        ForEach(linked) { Text($0.name).tag($0.id) }
                    }
                }
                if !["codex", "claude", "agy", "deepseek"].contains(account.provider) { Text("此 Agent 暂未接入自动限额查询，可先关闭限额，仅统计用量。").font(.caption).foregroundStyle(.secondary) }
                if !linked.isEmpty { Section("关联的数据源") { ForEach(linked) { Text($0.name + " · " + $0.path).font(.callout) } } }
            }.formStyle(.grouped)
            HStack { Button("取消") { dismiss() }; Spacer(); Button("完成") { onSave(account) }.buttonStyle(.borderedProminent).disabled(account.name.trimmingCharacters(in: .whitespaces).isEmpty) }.padding(20)
        }.frame(width: 560, height: 470)
        .onChange(of: account.provider) { _, provider in account.quotaSourceId = nil; account.quotaEnabled = ["codex", "claude", "agy", "deepseek"].contains(provider) }
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
            HStack { Text("SSH 主机").font(.title2.weight(.semibold)); Spacer() }.padding(22)
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
                    if let discoveryError { Text(discoveryError).font(.system(size: 13)).foregroundStyle(.orange) }
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
                    TextEditor(text: $host.preCommand).font(.system(size: 13, design: .monospaced)).frame(height: 65)
                    TextField("设备表达式", text: $devices, prompt: Text("network:eth0, gpu:0, filesystems:/"))
                    Text("设备表达式留空时显示全部。").font(.system(size: 13)).foregroundStyle(.secondary)
                }
            }.formStyle(.grouped)
            HStack { Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Spacer(); Button("完成") { host.devices = MonitorSelection.parse(devices); if host.name.isEmpty { host.name = host.target }; onSave(host) }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(host.target.isEmpty) }.padding(20)
        }.frame(width: 620, height: 650).onAppear { devices = host.devices.joined(separator: ", ") }
    }
}

struct PriceEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State var price: ModelPrice
    var onSave: (ModelPrice) -> Void
    @State private var input = ""
    @State private var output = ""
    @State private var cacheRead = ""
    @State private var cacheWrite = ""
    private var valid: Bool { [input, output, cacheRead, cacheWrite].allSatisfy { $0.isEmpty || Double($0).map { $0.isFinite && $0 >= 0 } == true } }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("模型价格").font(.title2.weight(.semibold)); Spacer(); Text("USD / 百万 Token").font(.caption).foregroundStyle(.secondary) }.padding(22)
            Form {
                TextField("模型 ID", text: $price.id, prompt: Text("openai/model-name"))
                TextField("显示名称", text: $price.name)
                TextField("输入", text: $input, prompt: Text("待定价")); TextField("输出", text: $output, prompt: Text("待定价"))
                TextField("缓存读取", text: $cacheRead, prompt: Text("待定价")); TextField("缓存写入", text: $cacheWrite, prompt: Text("待定价"))
            }.formStyle(.grouped)
            HStack { Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Spacer(); Button("保存") { price.input = Double(input).map { $0 / 1e6 }; price.output = Double(output).map { $0 / 1e6 }; price.cacheRead = Double(cacheRead).map { $0 / 1e6 }; price.cacheWrite = Double(cacheWrite).map { $0 / 1e6 }; onSave(price) }.buttonStyle(.borderedProminent).disabled(!valid || price.id.isEmpty) }.padding(20)
        }.frame(width: 520, height: 450).onAppear { input = price.input.map { String($0 * 1e6) } ?? ""; output = price.output.map { String($0 * 1e6) } ?? ""; cacheRead = price.cacheRead.map { String($0 * 1e6) } ?? ""; cacheWrite = price.cacheWrite.map { String($0 * 1e6) } ?? "" }
    }
}
