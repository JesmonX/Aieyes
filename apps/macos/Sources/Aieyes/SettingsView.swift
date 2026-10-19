import SwiftUI

private struct HostEditorRequest: Identifiable {
    let id = UUID()
    var host: Host
    var baseline: Settings
}

struct SettingsView: View {
    @SurfaceModel var model: AppModel
    @Environment(\.surfaceActive) private var parentActive
    @ObservedObject private var catalog: PriceCatalog
    private var draft: Settings {
        get { model.settingsDraft }
        nonmutating set { model.settingsDraft = newValue }
    }
    @State private var autoSaveTask: Task<Void, Never>?
    @State private var accountEditor: AgentAccount?
    @State private var accountMachineID: String?
    @State private var accountSection: String?
    @State private var hostEditor: HostEditorRequest?
    @State private var priceEditor: ModelPrice?
    @AppStorage("aieyes.pet.enabled") private var petEnabled = true
    @AppStorage(NativePanelKey.showCount.rawValue) private var showMenuCount = false
    @State private var removal: RemovalRequest?
    @State private var confirmRescan = false
    @State private var importSSH = false
    @State private var mappingsExpanded = false
    @State private var hasVisitedPrices = false
    @State private var savingServerSwitches: Set<String> = []
    private var mappingModel: String { get { model.mappingModel } nonmutating set { model.mappingModel = newValue } }
    private var mappingID: String { get { model.mappingID } nonmutating set { model.mappingID = newValue } }
    @FocusState private var mappingFocused: Bool
    @FocusState private var generalField: String?
    @MainActor init(model: AppModel) {
        _model = SurfaceModel(wrappedValue: model)
        self.catalog = model.priceCatalog
    }
    var body: some View {
        VStack(spacing: 0) {
            if model.settingsSaving || model.settingsMessage != nil || !model.settingsLoaded {
            HStack {
                ErrorNotice(message: model.settingsSaving ? "正在保存…" : model.settingsMessage ?? "正在读取配置…", context: "设置操作未完成", tint: .secondary)
                Spacer()
                if model.settingsSaving { ProgressView().controlSize(.small) }
                if !model.settingsLoaded { Button("重试读取") { Task { await model.bootstrap() } } }
                if model.settingsTab == "general" && model.settingsDraft != model.settings && model.settingsMessage != nil && !model.settingsSaving {
                    Button("重试保存") { Task { await model.saveGeneralConfiguration() } }
                    Button("恢复已保存配置") { model.discardSettingsDraft() }
                }
            }.padding(.horizontal, 22).padding(.vertical, 12)
            }
            if let message = model.message, model.settingsMessage == nil, !message.hasPrefix("已保存") { HStack { Text(message).font(AppFont.secondary).textSelection(.enabled); Spacer(); Button { model.message = nil } label: { Image(systemName: "xmark") }.buttonStyle(.plain).accessibilityLabel("关闭提示") }.padding(.horizontal, 22).padding(.bottom, 10) }
            ForEach(model.settings.configurationIssues ?? [], id: \.self) { Text($0).font(AppFont.secondary).foregroundStyle(Palette.warn).textSelection(.enabled).padding(.horizontal, 22) }
            Divider().opacity(0.5)
            ZStack(alignment: .top) {
                // Keep the lazily created catalog and its native controls across
                // tab switches, preserving scroll/drafts without recreating rows.
                if hasVisitedPrices || model.settingsTab == "prices" {
                    prices.opacity(model.settingsTab == "prices" ? 1 : 0)
                        .allowsHitTesting(model.settingsTab == "prices")
                        .disabled(model.settingsTab != "prices")
                        .accessibilityHidden(model.settingsTab != "prices")
                }
                if model.settingsTab != "prices" {
                switch model.settingsTab {
                case "integrations", "accounts", "codexAuth": AgentsSettingsView(model: model)
                case "wakeups": WakeupsView(model: model)
                case "sources": AgentsSettingsView(model: model)
                case "servers": servers
                case "general", "connection": general
                default: AgentsSettingsView(model: model)
                }
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top).padding(.horizontal, 16).padding(.bottom, 16).disabled(model.repricing || !model.settingsLoaded)
        }
        .font(AppFont.body).disabled(model.installingUpdate)
        .environment(\.surfaceActive, parentActive || CommandLine.arguments.contains("--render"))
        .alert("全量重扫历史记录", isPresented: $confirmRescan) {
            Button("取消", role: .cancel) { }
            Button("开始重扫") { Task { await model.rescan() } }
        } message: {
            Text("将从头读取所有已启用本机及服务器数据源的完整日志，修复可确认的历史重复。已有历史和价格快照会保留；日志较多时可能需要较长时间。")
        }
        .frame(minWidth: 0, maxWidth: .infinity, maxHeight: .infinity).aieyesAccent()
        .onAppear { normalizeCategory(); consumeEditorRequest() }
        .onChange(of: model.settingsTab, initial: true) { _, tab in normalizeCategory(); if tab == "prices" { hasVisitedPrices = true } }
        .onChange(of: model.settingsDraft) { _, _ in
            guard model.settingsTab == "general", model.settingsDraft != model.settings else { return }
            autoSaveTask?.cancel()
            autoSaveTask = Task { do { try await Task.sleep(for: .milliseconds(600)) } catch { return }; await model.saveGeneralConfiguration() }
        }
        .onChange(of: model.settingsValidationField) { _, field in
            if let field { model.settingsTab = "general"; model.generalPage = "refresh"; generalField = field }
        }
        .task(id: model.settingsTab) { if model.settingsTab == "prices" { await catalog.load() } }
        .onChange(of: model.requestedSourceProvider) { _, _ in consumeEditorRequest() }
        .onChange(of: model.requestedAccountKey) { _, _ in consumeEditorRequest() }
        .onChange(of: model.requestedHostID) { _, _ in consumeEditorRequest() }
        .onChange(of: model.requestHostEditor) { _, _ in consumeEditorRequest() }
        .sheet(item: $removal) { request in
            DangerConfirmation(title: request.title, explanation: request.explanation, affected: request.affected, confirmLabel: request.kind == "account" ? "归档并保留历史" : "确认移除", cancel: { removal = nil }, confirm: { removal = nil; Task { await model.removeConfiguration(request) } })
        }
        .sheet(item: $accountEditor) { account in AccountDevicesView(model: model, account: account, initialMachineID: accountMachineID, initialSection: accountSection) }
        .sheet(item: $hostEditor) { request in HostEditor(host: request.host, model: model) { item, password in
            try await model.saveHostConfiguration(item, password: password, base: request.baseline)
            hostEditor = nil
        } }
        .sheet(isPresented: $importSSH) { SSHConfigImportView(model: model) }
        .sheet(item: $priceEditor) { price in PriceEditor(price: price) { item in
            guard await model.savePrice(item) else { throw ClientError.message(model.message ?? "保存失败") }
            priceEditor = nil
        } }
    }
    private func normalizeCategory() {
        if model.settingsTab == "connection" { model.generalPage = "network" }
        model.settingsTab = SettingsDestination(category: model.settingsTab).normalizedCategory
    }
    private func consumeEditorRequest() {
        if let key = model.requestedAccountKey {
            accountMachineID = model.requestedAccountMachineID
            accountSection = model.settingsSection
            model.requestedAccountMachineID = nil
            accountEditor = draft.accounts.first { $0.key == key }
            if let accountEditor { model.settingsAgent = accountEditor.provider; model.settingsTab = "integrations" }
            model.requestedAccountKey = nil
        }
        if let provider = model.requestedSourceProvider {
            model.settingsTab = "integrations"; model.settingsAgent = provider; model.requestedSourceProvider = nil
        }
        if let id = model.requestedHostID {
            if id == "local" { editHost((model.settings.localMonitor ?? LocalMonitor()).host) }
            else if let host = model.settings.hosts.first(where: { $0.id == id }) { editHost(host) }
            else { model.settingsMessage = "此服务器已移除，请重新选择。" }
            model.requestedHostID = nil
        }
        if model.requestHostEditor { editHost(Host()); model.requestHostEditor = false }
    }
    private func editHost(_ host: Host) { hostEditor = HostEditorRequest(host: host, baseline: model.settings) }
    private func serverSwitch(_ id: String, resource: Bool) -> some View {
        let key = id + (resource ? ":resources" : ":agents")
        let label = resource ? "资源监控" : "Agent 采集"
        return HStack(spacing: 8) {
        Text(label).fixedSize(horizontal: true, vertical: false)
        Toggle(label, isOn: Binding(get: {
            if id == "local" { return resource ? model.settings.localMonitor?.enabled ?? true : model.settings.localDeviceEnabled != false }
            guard let host = model.settings.hosts.first(where: { $0.id == id }) else { return false }
            return resource ? host.monitoringEnabled != false : host.enabled
        }, set: { value in
            guard savingServerSwitches.insert(key).inserted else { return }
            Task {
                defer { savingServerSwitches.remove(key) }
                var next = model.settings
                if id == "local" {
                    if resource { var local = next.localMonitor ?? LocalMonitor(); local.enabled = value; next.localMonitor = local }
                    else { next.localDeviceEnabled = value }
                } else if let index = next.hosts.firstIndex(where: { $0.id == id }) {
                    if resource { next.hosts[index].monitoringEnabled = value }
                    else { next.hosts[index].enabled = value }
                }
                do { try await model.persistConfiguration(next) }
                catch { model.settingsMessage = error.localizedDescription }
            }
        })).labelsHidden().toggleStyle(.switch).controlSize(.small).fixedSize()
        }.disabled(savingServerSwitches.contains(key))
    }
    private var servers: some View {
        GeometryReader { geometry in VStack(spacing: 12) {
            Form {
                serverRow((model.settings.localMonitor ?? LocalMonitor()).host, narrow: geometry.size.width < 650)
                ForEach(model.settings.hosts) { host in serverRow(host, narrow: geometry.size.width < 650) }
            }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle())
            HStack { Button("从 SSH 配置导入") { importSSH = true }; Button("添加 SSH 主机", systemImage: "plus") { editHost(Host()) }; Spacer(); Button("测试采样") { Task { await model.sampleHosts() } }.disabled(model.serverBusy) }.padding(.horizontal, 8)
        }.padding(.vertical, 12) }
    }
    private func serverRow(_ host: Host, narrow: Bool) -> some View {
        let local = host.id == "local", name = host.id == "local" ? "本机" : host.name.isEmpty ? host.target : host.name
        let information = VStack(alignment: .leading, spacing: 5) {
            Text(name).font(AppFont.section).lineLimit(2).help(name)
            Text(local ? "当前设备" : host.target).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle).help(host.target)
        }.frame(maxWidth: .infinity, alignment: .leading)
        let controls = HStack(spacing: 12) {
            serverSwitch(host.id, resource: true).frame(width: 112, alignment: .leading)
            serverSwitch(host.id, resource: false).frame(width: 112, alignment: .leading)
            if narrow { Spacer(minLength: 0) }
            HStack(spacing: 8) {
                Button(local ? "监控设置" : "编辑") { editHost(host) }.frame(width: 78).accessibilityLabel("编辑" + name)
                if local { Color.clear.frame(width: 20, height: 20).accessibilityHidden(true) }
                else { Button { removal = model.removalRequest("host", id: host.id) } label: { Image(systemName: "minus.circle").frame(width: 20, height: 20) }.buttonStyle(.borderless).help("移除主机").accessibilityLabel("移除服务器 " + name) }
            }.frame(width: 106, alignment: .trailing)
        }
        return Group {
            if narrow { VStack(alignment: .leading, spacing: 12) { information; controls } }
            else { HStack(spacing: 12) { information; controls } }
        }.padding(.vertical, 7)
    }
    private func applyMappingDraft() -> Bool {
        let from = mappingModel.trimmingCharacters(in: .whitespacesAndNewlines), to = mappingID.trimmingCharacters(in: .whitespacesAndNewlines)
        if from.isEmpty && to.isEmpty { return true }
        guard !from.isEmpty && !to.isEmpty else { model.settingsMessage = "请补全模型映射两端，输入已保留"; mappingFocused = true; return false }
        guard draft.modelMappings[from] == nil || draft.modelMappings[from] == to else { model.settingsMessage = "该模型已有映射，请在已有映射中编辑"; return false }
        draft.modelMappings[from] = to
        return true
    }
    private func openGapPrice(_ gap: PricingGap) {
        let id = gap.priceId ?? gap.model
        priceEditor = model.prices.first(where: { $0.id == id }) ?? ModelPrice(id: id, name: gap.model)
    }
    private var prices: some View {
        ScrollViewReader { reader in ScrollView { LazyVStack(alignment: .leading, spacing: 14) {
            Section {
                Text("价格和映射保存后生效；“保存并重算”同时更新历史成本。").font(AppFont.secondary).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading)
                HStack { Text("USD / 百万 Token").font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Button(model.repricing ? "重算中…" : "保存并重算") { Task { await model.saveAndReprice() } }.disabled(model.settingsSaving || model.repricing).accessibilityIdentifier("save-and-reprice") }
            }
            if !model.dashboard.pricingGaps.isEmpty {
                Section("待计价模型") {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(model.dashboard.pricingGaps) { gap in
                                HStack {
                                    VStack(alignment: .leading, spacing: 4) { Text(gap.model).font(.system(size: DesignTokens.fontSecondary, weight: DesignTokens.weightMedium)); Text("缺少\(gap.missing) · \(Format.compact(gap.unpricedTokens)) Token").font(AppFont.secondary).foregroundStyle(.secondary) }
                                    Spacer()
                                    Button("映射") { mappingModel = gap.model; mappingID = gap.priceId ?? "" }
                                    Button("补充价格") { openGapPrice(gap) }
                                }
                            }
                        }.padding(6)
                    }.frame(height: 100)
                }
            }
            Section("模型映射") {
                VStack(spacing: 8) {
                    HStack { TextField("日志中的模型名称", text: $model.mappingModel); Image(systemName: "arrow.right"); TextField("OpenRouter 模型 ID", text: $model.mappingID).focused($mappingFocused); Button("添加映射") { if applyMappingDraft() { Task { _ = await model.saveSettingsDraft() } } }.disabled(mappingModel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || mappingID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
                    DisclosureGroup("已有映射（\(draft.modelMappings.count)）", isExpanded: $mappingsExpanded) {
                        if mappingsExpanded {
                            ScrollView { LazyVStack(spacing: 8) { ForEach(draft.modelMappings.keys.sorted(), id: \.self) { key in ExistingMappingRow(model: model, name: key) } }.padding(.vertical, 4) }.frame(height: min(180, CGFloat(draft.modelMappings.count) * 48))
                        }
                    }
                }.padding(6)
            }.id("model-mapping")
            HStack {
                TextField("搜索模型", text: $catalog.query).textFieldStyle(.aieyes)
                Button("同步 OpenRouter") { Task { await model.syncPrices() } }.disabled(model.busy || model.quotaBusy)
                Button("添加价格") { priceEditor = ModelPrice() }
            }
            HStack {
                Text("模型价格 · \(catalog.filtered.count) 项").font(AppFont.section)
                Spacer()
                if catalog.loading { ProgressView().controlSize(.small); Text("读取本地价格…").font(AppFont.secondary) }
            }
            if let error = catalog.error {
                HStack { ErrorNotice(message: error).foregroundStyle(Palette.warn); Spacer(); Button("重试读取") { Task { await catalog.load(force: true) } } }.font(AppFont.secondary)
            }
            ScrollView { LazyVStack(spacing: 8) {
                ForEach(catalog.filtered) { price in
                    PriceCatalogRow(price: price, map: { mappingID = price.id; reader.scrollTo("model-mapping", anchor: .top); mappingFocused = true }, edit: { priceEditor = price }).equatable()
                }
            } }.frame(height: 372)
            if !catalog.loading && catalog.error == nil && catalog.filtered.isEmpty {
                Text(catalog.query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "同步模型价格" : "无匹配模型").foregroundStyle(.secondary).frame(maxWidth: .infinity).padding()
            }
        }.padding(16) }.textFieldStyle(.aieyes) }
    }
    private var general: some View {
        HStack(alignment: .top, spacing: 12) {

            GeometryReader { viewport in Form {
            if model.generalPage == "appearance" {
            Section("外观") {
                Picker("主题", selection: Binding(get: { model.settingsDraft.appearance?.theme ?? "system" }, set: { value in var appearance = model.settingsDraft.appearance ?? AppearanceSettings(); appearance.theme = value; model.settingsDraft.appearance = appearance })) {
                    Text("跟随系统").tag("system"); Text("浅色").tag("light"); Text("深色").tag("dark")
                }
                Picker("强调色", selection: Binding(get: { model.settingsDraft.appearance?.accent ?? "indigo" }, set: { value in var appearance = model.settingsDraft.appearance ?? AppearanceSettings(); appearance.accent = value; model.settingsDraft.appearance = appearance })) {
                    Text("靛蓝").tag("indigo"); Text("蓝").tag("blue"); Text("青绿").tag("teal"); Text("紫").tag("purple")
                }
                Picker("图表配色", selection: Binding(get: { model.settingsDraft.appearance?.chartPalette ?? "classic" }, set: { value in var appearance = model.settingsDraft.appearance ?? AppearanceSettings(); appearance.chartPalette = value; model.settingsDraft.appearance = appearance })) {
                    ForEach(Palette.chartPresets, id: \.id) { Text($0.name).tag($0.id) }
                }
                Picker("图表对比度", selection: Binding(get: { model.settingsDraft.appearance?.chartContrast ?? "high" }, set: { value in var appearance = model.settingsDraft.appearance ?? AppearanceSettings(); appearance.chartContrast = value; model.settingsDraft.appearance = appearance })) {
                    Text("高对比度").tag("high"); Text("低对比度 · 柔和").tag("low")
                }
                ChartPalettePreview(preset: model.settingsDraft.appearance?.chartPalette ?? "classic", contrast: model.settingsDraft.appearance?.chartContrast ?? "high")
                Text("图表配色随明暗主题调整。高对比度适合小色块，低对比度更柔和。").font(AppFont.secondary).foregroundStyle(.secondary)
            }
            }
            if model.generalPage == "desktop" {
            Section("桌面宠物") {
                Toggle("显示桌面宠物（本机，立即生效）", isOn: $petEnabled)
                PetScaleControl()
                PetLibraryView(model: model)
                Text("桌宠展示本机和 SSH 的 Codex、Claude Code、Antigravity 会话。单击打开面板，拖动移动。").font(AppFont.secondary).foregroundStyle(.secondary)
            }
            Section("菜单栏") {
                Toggle("显示活跃会话数（本机与 SSH）", isOn: $showMenuCount)
                Text("菜单栏显示今日全部数据，独立于页面筛选。").font(AppFont.secondary).foregroundStyle(.secondary)
                Picker("显示内容", selection: $model.settingsDraft.menuMetric) { Text("图标").tag("icon"); Text("今日全部 Token").tag("tokens"); Text("首个账户剩余额度").tag("quota"); Text("首台服务器 CPU").tag("cpu") }
            }
            }
            if model.generalPage == "refresh" {
            Section("刷新") {
                LabeledContent("额度查询间隔（秒）") { TextField("额度查询间隔（秒）", value: $model.settingsDraft.refreshSeconds, format: .number).labelsHidden().frame(width: DesignTokens.fieldShortWidth).focused($generalField, equals: "quota") }
                LabeledContent("日志同步间隔（秒）") { TextField("日志同步间隔（秒）", value: $model.settingsDraft.historyInterval, format: .number).labelsHidden().frame(width: DesignTokens.fieldShortWidth).focused($generalField, equals: "history") }
                LabeledContent("服务器前台间隔（秒）") { TextField("服务器前台间隔（秒）", value: $model.settingsDraft.foregroundInterval, format: .number).labelsHidden().frame(width: DesignTokens.fieldShortWidth).focused($generalField, equals: "foreground") }
                LabeledContent("服务器后台间隔（秒）") { TextField("服务器后台间隔（秒）", value: $model.settingsDraft.serverRefreshSeconds, format: .number).labelsHidden().frame(width: DesignTokens.fieldShortWidth).focused($generalField, equals: "background") }
            }
            Section("历史记录") {
                Toggle("精确计数", isOn: Binding(get: { model.settingsDraft.codexPreciseCounting == true }, set: { model.settingsDraft.codexPreciseCounting = $0 }))
                Text("开启后扣除 Codex fork 会话继承的父历史用量；关闭时允许重复计入。修改后在下次同步自动重算已有日志。").font(AppFont.secondary).foregroundStyle(.secondary)
                if model.settings.codexPreciseCounting == true {
                    ForEach(model.dashboard.sources.filter { $0.enabled && !($0.status?.forkHistoryIssues ?? []).isEmpty }) { source in
                        DisclosureGroup(source.name + " · 父历史待确认") {
                            ForEach(Array((source.status?.forkHistoryIssues ?? []).enumerated()), id: \.offset) { _, issue in
                                Text((issue.path.map { $0 + " · " } ?? "") + issue.message).font(AppFont.secondary).textSelection(.enabled)
                            }
                            Text("首段不确定累计暂不计入，后续有效增量继续统计；父历史恢复后自动补全。").font(AppFont.secondary).foregroundStyle(.secondary)
                        }
                    }
                }
                Toggle("在筛选中显示已删除账户", isOn: Binding(get: { model.settingsDraft.historyShowDeletedAccounts == true }, set: { model.settingsDraft.historyShowDeletedAccounts = $0 }))
                Text("在筛选中显示有历史记录的已删除账户；其用量始终计入总统计。").font(AppFont.secondary).foregroundStyle(.secondary)
                Text("重新读取已启用来源的完整日志，修正可确认的重复记录。已有历史和价格快照保留。").font(AppFont.secondary).foregroundStyle(.secondary)
                HStack {
                    Button(model.rescanning ? "全量重扫中…" : "全量重扫") { confirmRescan = true }
                        .disabled(model.busy || model.quotaBusy || model.settingsSaving || !model.estimateBusy.isEmpty)
                    RefreshIndicator(model: model, key: "rescan")
                }
                if model.rescanning { ProgressView("正在重新读取完整日志…").controlSize(.small) }
                if let result = model.rescanMessage { Text(result).font(AppFont.secondary).foregroundStyle(model.rescanFailed ? Palette.warn : Color.secondary).textSelection(.enabled) }
            }
            }
            if model.generalPage == "network" {
            Section("网络代理") {
                ProxyFields(proxy: $model.settingsDraft.proxy, testURLs: model.settingsDraft.proxyTestUrls)
                VStack(alignment: .leading, spacing: 8) {
                    Text("测试地址（逗号分隔）")
                    TextField("测试地址（逗号分隔）", text: Binding(get: { (model.settingsDraft.proxyTestUrls ?? ["https://api.github.com/rate_limit", "https://openrouter.ai"]).joined(separator: ", ") }, set: { model.settingsDraft.proxyTestUrls = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty } }), axis: .vertical).labelsHidden().textFieldStyle(.aieyes).lineLimit(2...4)
                }
            }
            }
            if model.generalPage == "notifications" { QuotaNotificationSettingsView(model: model, service: model.resetNotifications) }
            if model.generalPage == "updates" { Section("更新") { UpdateSettingsView() } }
            }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle()).environment(\.petPreviewViewport, viewport.frame(in: .global)) }
        }.padding(.top, 12)
    }
}

struct SettingsSidebar: View {
    var title: String
    var items: [(String, String)]
    @Binding var selection: String
    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            ForEach(items, id: \.0) { key, label in
                Button { selection = key } label: {
                    Text(label).frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 10).padding(.vertical, 9)
                        .foregroundStyle(selection == key ? Palette.accent : Color.primary)
                        .background(selection == key ? Palette.accent.opacity(0.12) : Color.clear, in: RoundedRectangle(cornerRadius: 8))
                        .contentShape(Rectangle())
                }.buttonStyle(.plain).accessibilityAddTraits(selection == key ? .isSelected : [])
            }
        }.padding(7).frame(width: DesignTokens.settingsSidebarWidth).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 12)).accessibilityLabel(title)
    }
}

private struct ExistingMappingRow: View {
    @SurfaceModel var model: AppModel
    let name: String
    @State private var editing = false
    @State private var from = ""
    @State private var to = ""
    @State private var baseline: Settings?
    @State private var busy = false
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if editing {
                HStack {
                    TextField("日志模型名称", text: $from)
                    Image(systemName: "arrow.right")
                    TextField("OpenRouter 模型 ID", text: $to)
                    Button("保存") { Task { await save() } }
                    Button("取消") { editing = false; error = nil }
                }.disabled(busy)
                if let error { ErrorNotice(message: error).foregroundStyle(Palette.danger) }
            } else {
                HStack {
                    Text(name); Image(systemName: "arrow.right"); Text(model.settingsDraft.modelMappings[name] ?? ""); Spacer()
                    Button("编辑") { baseline = model.settings; from = name; to = model.settingsDraft.modelMappings[name] ?? ""; editing = true }
                    Button { model.settingsDraft.modelMappings.removeValue(forKey: name); Task { _ = await model.saveSettingsDraft() } } label: { Image(systemName: "minus.circle") }.buttonStyle(.plain).accessibilityLabel("移除映射 " + name)
                }
            }
        }.font(AppFont.secondary)
    }
    private func save() async {
        let from = from.trimmingCharacters(in: .whitespacesAndNewlines), to = to.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !from.isEmpty, !to.isEmpty, let baseline else { error = "请补全映射两端"; return }
        guard from == name || baseline.modelMappings[from] == nil else { error = "日志模型名称已存在"; return }
        busy = true; error = nil; defer { busy = false }
        var next = baseline; next.modelMappings.removeValue(forKey: name); next.modelMappings[from] = to
        if await model.save(next, base: baseline) { editing = false }
        else { error = model.message ?? "保存失败，输入已保留" }
    }
}

struct PriceCatalogRow: View, Equatable {
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.price == rhs.price }
    let price: ModelPrice
    var map: () -> Void
    var edit: () -> Void
    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text(price.id).font(AppFont.body).lineLimit(1).help(price.id)
                Text("输入 \(price.input.map { Format.money($0 * 1e6) } ?? "—") · 输出 \(price.output.map { Format.money($0 * 1e6) } ?? "—") / 百万 Token").font(AppFont.secondary).foregroundStyle(.secondary)
            }
            Spacer()
            Button("一键映射", action: map)
            Button("编辑", action: edit).accessibilityLabel("编辑价格 " + price.id)
        }.padding(12).frame(height: 68).background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 10))
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
                else { TextField("Host", text: $address.host); LabeledContent("端口") { TextField("端口", text: $address.port).labelsHidden().frame(width: DesignTokens.fieldShortWidth) } }
            }
            HStack {
                Button(testing ? "测试中…" : "测试连接") { Task { await testConnection() } }.disabled(testing)
                if let result { Text(result.label).help(result.detail).foregroundStyle(result.status == "ok" ? Color.secondary : Palette.warn) }
            }
            if let testError { ErrorNotice(message: testError).foregroundStyle(Palette.warn) }
        }
        .disabled(testing)
        .updateOperation("编辑器代理检测", active: false)
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
    private var draftValue: String { draftSnapshot(source) }
    private func cancel() { if !initialDraft.isEmpty && initialDraft != draftValue { confirmDiscard = true } else { dismiss() } }
    @State var source: AgentSource
    @State private var saving = false
    @State private var error: String?
    var hosts: [Host]
    var appProxy = ProxySettings(), testURLs: [String]?
    var onSave: (AgentSource) async throws -> Void
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("目录设置").font(AppFont.title); Spacer() }.padding(22)
            Form {
                Picker("Agent", selection: $source.provider) {
                    ForEach(CoreContract.providers, id: \.self) { provider in Text(Format.provider(provider)).tag(provider) }
                }.disabled(true)
                if source.provider != "deepseek" {
                    Picker("位置", selection: Binding(get: { source.hostId ?? "local" }, set: { source.hostId = $0 == "local" ? nil : $0 })) {
                        Text("本机").tag("local"); ForEach(hosts) { Text($0.name.isEmpty ? $0.target : $0.name).tag($0.id) }
                    }.disabled(true)
                }
                if source.subscription != nil {
                    Text("此来源只查询订阅；历史请在机器与采集中添加独立目录。").font(AppFont.secondary).foregroundStyle(.secondary)
                } else if source.provider == "deepseek" {
                    Text("API Key 请在账户页添加或更新。").font(AppFont.secondary).foregroundStyle(.secondary)
                    DisclosureGroup("高级设置") { TextField("API Key 文件（可选）", text: $source.path) }
                } else {
                    HStack {
                        TextField("数据目录", text: $source.path)
                        if source.hostId == nil {
                            Button("选择") { let panel = NSOpenPanel(); panel.canChooseDirectories = true; panel.canChooseFiles = true; panel.showsHiddenFiles = true; if panel.runModal() == .OK, let url = panel.url { source.path = url.path } }
                        }
                    }
                }
                if source.codexHomeId != nil { Text("更换目录会解除旧账户连接，保留原文件和凭据。之后请读取新目录的登录。").font(AppFont.secondary).foregroundStyle(.secondary) }
                DisclosureGroup(source.hostId == nil ? "高级连接选项 · CLI / 代理" : "高级连接选项 · CLI") {
                if source.provider == "antigravity" { TextField("agy 程序", text: Binding(get: { source.agyBinary ?? "agy" }, set: { source.agyBinary = $0 })) }
                if source.provider == "codex" { TextField("Codex 程序", text: $source.codexBinary) }
                if ["custom", "kimi", "deepseek-harness", "codebuddy"].contains(source.provider) || CoreContract.subscriptionModes[source.provider] != nil { TextField("额度查询命令", text: $source.quotaCommand) }
                if source.hostId == nil {
                    ProxyFields(proxy: Binding(get: { source.proxy ?? ProxySettings(mode: "inherit") }, set: { source.proxy = $0.mode == "inherit" ? nil : $0 }), inherit: true, inheritedProxy: appProxy, testURLs: testURLs)
                }
                }
                if let error { ErrorNotice(message: error).foregroundStyle(Palette.warn) }
            }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle()).textFieldStyle(.aieyes).disabled(saving)
            HStack {
                Button("取消") { cancel() }.keyboardShortcut(.cancelAction).disabled(saving); Spacer(); Text("保存后立即生效").font(AppFont.secondary).foregroundStyle(.secondary); Spacer()
                Button(saving ? "处理中…" : "保存") { Task { await save() } }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction).disabled(saving)
            }.padding(20)
        }.font(AppFont.body).frame(width: DesignTokens.dialogSourceWidth, height: EditorLayout.height(660))
        .interactiveDismissDisabled(saving || initialDraft != draftValue)
        .updateDraftGuard("数据源", snapshot: draftValue, dirty: !initialDraft.isEmpty && initialDraft != draftValue, saving: saving, save: { await save(); return error == nil }, discard: { dismiss() })
        .onAppear { if initialDraft.isEmpty { initialDraft = draftValue } }
        .discardDraftConfirmation($confirmDiscard) { dismiss() }
    }
    private func save() async {
        saving = true; error = nil; defer { saving = false }
        do {
            var next = source
            if !["antigravity", "deepseek"].contains(next.provider), CoreContract.subscriptionModes[next.provider] == nil, next.path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { throw ClientError.message("请输入数据目录") }
            if next.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { next.name = Format.provider(next.provider) }
            if next.provider == "deepseek" { next.hostId = nil }
            try await onSave(next)
        } catch { self.error = error.localizedDescription }
    }
}

struct HostEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State private var initialDraft = ""
    @State private var confirmDiscard = false
    private var draftValue: String { draftSnapshot(host) + password + devices }
    private func cancel() { if !initialDraft.isEmpty && initialDraft != draftValue { confirmDiscard = true } else { dismiss() } }
    @State var host: Host
    var model: AppModel? = nil
    var initialSection: String? = nil
    @State private var advancedExpanded = false
    @FocusState private var shellFocused: Bool
    @State private var importSSH = false
    @State var password = ""
    var onSave: (Host, String) async throws -> Void
    @State private var saving = false
    @State private var saveError: String?
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
            selected: MonitorSelection.selected(tokens, group: group, available: Set(options.map(\.id)), recommended: group == "filesystems" ? Set((discovered[group] ?? []).filter { MonitorSelection.recommendedFilesystem($0.id,type: $0.type) }.map(\.id)) : nil),
            all: tokens.contains(group + ":__all__") || (group != "filesystems" && !tokens.contains { $0.hasPrefix(group + ":") })) { selected, all in
                devices = MonitorSelection.write(tokens, group: group, selected: selected, all: all).joined(separator: ", ")
            }.disabled(!host.metrics.contains(group))
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text(host.id == "local" ? "本机监测" : "SSH 主机").font(AppFont.title); Spacer() }.padding(22)
            Form {
                if host.id != "local" {
                if let model { Button("从 SSH 配置导入") { importSSH = true }.sheet(isPresented: $importSSH) { SSHConfigImportView(model: model) { if initialDraft == draftValue { dismiss() } } } }
                TextField("名称", text: $host.name)
                TextField("SSH 别名或地址", text: $host.target, prompt: Text("my-server 或 user@host"))
                LabeledContent("端口") { TextField("端口", text: Binding(get: { host.port.map(String.init) ?? "" }, set: { host.port = Int($0) }), prompt: Text("跟随配置")).labelsHidden().frame(width: DesignTokens.fieldHintWidth) }
                Picker("登录方式", selection: Binding(get: { host.authMode ?? "ssh" }, set: { host.authMode = $0 })) {
                    Text("SSH 配置 / 密钥").tag("ssh"); Text("用户名和密码").tag("password")
                }
                TextField("用户名", text: Binding(get: { host.username ?? "" }, set: { host.username = $0 }), prompt: Text("跟随地址或 SSH 配置"))
                if host.authMode == "password" { SecureField("密码（留空保留）", text: $password) }
                else { HStack { TextField("密钥路径", text: $host.identityFile, prompt: Text("跟随 SSH 配置")); Button("选择…") { let panel = NSOpenPanel(); panel.showsHiddenFiles = true; if panel.runModal() == .OK { host.identityFile = panel.url?.path ?? host.identityFile } } } }
                }
                Button(discovering ? "读取中…" : "连接并读取设备") { Task { await discover() } }.disabled(discovering || host.target.isEmpty)
                DisclosureGroup("监控指标与设备 · 按需选择") {
                Section {
                    MultiSelectPicker(title: "采集项目", options: groups.map { SelectionOption(id: $0.0, label: $0.1) }, selected: Set(host.metrics)) { selected, _ in
                        host.metrics = selected.sorted()
                    }
                }
                Section("设备") {
                    if let discoveryInfo { Text(discoveryInfo).font(AppFont.secondary).foregroundStyle(.secondary) }
                    if let discoveryError { ErrorNotice(message: discoveryError).font(AppFont.secondary).foregroundStyle(Palette.warn) }
                    ForEach(groups.filter { $0.0 != "memory" }, id: \.0) { group, label in devicePicker(group, label) }
                }
                Section {
                    MultiSelectPicker(title: "显示细分项", options: Host.detailOptions.filter { $0.0 == "uptime" || host.metrics.contains(detailGroups[$0.0] ?? "") }.map { SelectionOption(id: $0.0, label: $0.1) }, selected: Set(host.details ?? Host.detailOptions.map { $0.0 })) { selected, _ in
                        host.details = selected.sorted()
                    }
                }
                }
                DisclosureGroup("高级设置", isExpanded: $advancedExpanded) {
                    if host.id != "local" { TextField("远程 Shell", text: $host.shell).focused($shellFocused) }
                    TextField("设备表达式", text: $devices, prompt: Text("network:eth0, gpu:0, filesystems:/")).help("留空使用推荐设备；文件系统默认隐藏虚拟挂载。全选可显示全部")
                }
            }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle()).textFieldStyle(.aieyes)
            if let saveError { ErrorNotice(message: saveError) }
            HStack { Button("取消") { cancel() }.keyboardShortcut(.cancelAction); Spacer(); Text("保存后立即生效").font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Button(saving ? "保存中…" : "保存") { Task { saving = true; saveError = nil; defer { saving = false }; host.devices = MonitorSelection.parse(devices); if host.name.isEmpty { host.name = host.target }; do { try await onSave(host, password) } catch { saveError = error.localizedDescription } } }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction).disabled(host.target.isEmpty || saving) }.padding(20)
        }.font(AppFont.body).frame(width: DesignTokens.dialogHostWidth, height: EditorLayout.height(650)).onChange(of: connectionIdentity) { _, _ in discoveryRequest = UUID(); discovered = [:]; discoveryInfo = nil; discoveryError = "连接配置已更改，请重新读取设备" }
        .updateDraftGuard(host.id == "local" ? "本机监测" : "SSH 主机", snapshot: draftValue, dirty: !initialDraft.isEmpty && initialDraft != draftValue, saving: saving || discovering, save: {
            saving = true; defer { saving = false }; saveError = nil
            host.devices = MonitorSelection.parse(devices); if host.name.isEmpty { host.name = host.target }
            do { try await onSave(host,password); return true } catch { saveError = error.localizedDescription; return false }
        }, discard: { dismiss() })
        .onAppear { devices = host.devices.joined(separator: ", "); initialDraft = draftValue; if initialSection == "shell" { advancedExpanded = true; shellFocused = true } }
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
                LabeledContent("输入") { TextField("输入", text: $input, prompt: Text("待定价")).labelsHidden().frame(width: DesignTokens.fieldPriceWidth) }; LabeledContent("输出") { TextField("输出", text: $output, prompt: Text("待定价")).labelsHidden().frame(width: DesignTokens.fieldPriceWidth) }
                LabeledContent("缓存读取") { TextField("缓存读取", text: $cacheRead, prompt: Text("待定价")).labelsHidden().frame(width: DesignTokens.fieldPriceWidth) }; LabeledContent("缓存写入") { TextField("缓存写入", text: $cacheWrite, prompt: Text("待定价")).labelsHidden().frame(width: DesignTokens.fieldPriceWidth) }
                if !valid { Text("价格须为大于或等于 0 的数字；留空表示待定价。").foregroundStyle(Palette.warn) }
                if let error { ErrorNotice(message: error).accessibilityLabel("保存失败，" + error) }
            }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle()).textFieldStyle(.aieyes).disabled(saving)
            HStack {
                Button("取消") { cancel() }.keyboardShortcut(.cancelAction).disabled(saving)
                Spacer()
                Button(saving ? "保存中…" : "保存价格") { Task { await save() } }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction).disabled(saving || !valid || price.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }.padding(20)
        }.font(AppFont.body).frame(width: DesignTokens.dialogPriceWidth, height: EditorLayout.height(470)).interactiveDismissDisabled(saving || initialDraft != draftValue)
            .updateDraftGuard("模型价格", snapshot: draftValue, dirty: !initialDraft.isEmpty && initialDraft != draftValue, saving: saving, save: { guard valid, !price.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { error = "请填写模型 ID 和有效的非负价格"; return false }; await save(); return error == nil }, discard: { dismiss() })
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
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogConfirmationWidth).onAppear { cancelFocused = true }
        .updateDraftGuard(title, snapshot: "", dirty: false, saving: false, save: { false }, discard: cancel)
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

private struct ChartPalettePreview: View {
    let preset: String, contrast: String
    private let heights: [CGFloat] = [0.54, 0.82, 0.66, 0.94, 0.44, 0.74, 0.60, 0.88, 0.50, 0.70]
    var body: some View {
        HStack(spacing: 12) {
            ForEach([false, true], id: \.self) { dark in
                VStack(alignment: .leading, spacing: 8) {
                    Text(dark ? "深色" : "浅色").font(AppFont.secondary).foregroundStyle(dark ? Color.white : Color.black)
                    HStack(alignment: .bottom, spacing: 5) {
                        ForEach(Array(Palette.chartPreset(named: preset).colors(dark: dark, contrast: contrast).enumerated()), id: \.offset) { index, hex in
                            RoundedRectangle(cornerRadius: 2).fill(Palette.adaptive(hex, hex)).frame(height: heights[index] * 48).frame(maxWidth: .infinity)
                        }
                    }.frame(height: 48, alignment: .bottom)
                }.padding(12).frame(maxWidth: .infinity).background(Palette.adaptive(dark ? 0x29292e : 0xffffff, dark ? 0x29292e : 0xffffff), in: RoundedRectangle(cornerRadius: 8))
                .accessibilityElement(children: .ignore).accessibilityLabel("\(dark ? "深色" : "浅色")背景柱形图配色预览")
            }
        }
    }
}
