import SwiftUI

struct AutomaticValuationControls: View {
    @SurfaceModel var model: AppModel
    var sources: [AgentSource]
    private var eligible: [AgentSource] { sources.filter { model.settings.supports($0.provider, "valuation") } }
    var body: some View {
        if !eligible.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Text("数据源自动估值").font(AppFont.section)
                Text(eligible.contains { $0.provider == "codex" } ? "估算 5 小时 / 7 天额度及 credits 的 API 等价价值" : "估算 5 小时 / 7 天额度的 API 等价价值").font(AppFont.secondary).foregroundStyle(.secondary)
                ForEach(eligible) { source in AutomaticValuationSourceControls(model: model, source: source) }
                if let error = model.automaticError { ErrorNotice(message: error).foregroundStyle(Palette.warn) }
            }
        }
    }
}
private struct AutomaticValuationSourceControls: View {
    @SurfaceModel var model: AppModel
    let source: AgentSource
    @State private var confirming = false
    private var enabled: Bool { source.autoValuation?.enabled == true }
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(source.name)
                Spacer()
                Toggle("自动估值 · " + source.name, isOn: Binding(get: { enabled }, set: { value in
                    if value { confirming = true }
                    else { Task { await model.configureAutomatic(source, enabled: false, confirmed: false) } }
                })).labelsHidden().toggleStyle(.switch)
            }
            Text(source.path).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled)
            if enabled { Text(model.dashboard.sources.first { $0.id == source.id }?.autoValuationStatus?.note ?? "等待首个实时订阅快照").font(AppFont.secondary).foregroundStyle(.secondary) }
        }.disabled(model.automaticSaving)
            .confirmationDialog("开启自动估值？", isPresented: $confirming, titleVisibility: .visible) {
                Button("确认开启") { Task { await model.configureAutomatic(source, enabled: true, confirmed: true) } }
                Button("取消", role: .cancel) { }
            } message: { Text("我确认影响订阅额度的所有设备用量均已纳入；按日志识别 API，不确定的区间跳过。") }
    }
}

struct SamplingManagementView: View {
    @SurfaceModel var model: AppModel
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("后台采样").font(AppFont.title)
                Text("关闭窗口后继续采样，点击“结束采样”停止。").font(AppFont.secondary).foregroundStyle(.secondary)
                if model.runningEstimates.isEmpty { Text("当前没有进行中的采样").foregroundStyle(.secondary) }
                ForEach(model.runningEstimates, id: \.recordKey) { estimate in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Text(model.settings.accounts.first { $0.key == estimate.accountKey }?.name ?? estimate.accountKey).font(AppFont.section)
                            Spacer(); Text(estimate.statusLabel).foregroundStyle(estimate.status == "pending" ? Palette.warn : Palette.ok)
                        }
                        Text(estimate.calculationNote).font(AppFont.secondary)
                        if !estimate.reason.isEmpty { Text(estimate.reason).foregroundStyle(Palette.warn) }
                        if let error = model.estimateErrors[estimate.accountKey] { ErrorNotice(message: error).foregroundStyle(Palette.warn) }
                        if model.estimateBusy.contains(estimate.accountKey) { TimelineView(.periodic(from: .now, by: 1)) { context in
                            Text((model.estimateStages[estimate.accountKey] ?? "准备同步") + " · " + String(Int(context.date.timeIntervalSince(model.estimateStartedAt[estimate.accountKey] ?? context.date))) + " 秒").font(AppFont.secondary)
                        } }
                        HStack {
                            Button("查看与管理") { Task { await model.openEstimate(estimate) } }
                            Spacer()
                            Button("结束采样") { Task { _ = await model.performEstimate("stop", accountKey: estimate.accountKey, params: ["id": estimate.id], credits: estimate.kind == "credits") } }
                                .disabled(model.estimateBusy.contains(estimate.accountKey))
                        }
                    }.padding(14).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
                }
            }.padding(DesignTokens.dialogPadding).frame(maxWidth: .infinity, alignment: .leading)
        }.aieyesAccent().font(AppFont.body).disabled(model.installingUpdate)
    }
}

@MainActor private final class EstimateHistoryReader: ObservableObject {
    let engine = EngineClient(configurationOnly: true)
    deinit { engine.stop() }
}
struct AgentValuationView: View {
    @StateObject private var historyReader = EstimateHistoryReader()
    @Environment(\.dismiss) private var dismiss
    @SurfaceModel var model: AppModel
    let provider: String
    @State private var snapshot: Dashboard?
    @State private var error: String?
    @State private var loading = false
    @State private var manualExpanded = false
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == provider } }
    private var accounts: [AgentAccount] { model.settings.accounts.filter { $0.provider == provider && $0.archived != true } }
    private var records: [QuotaEstimate] {
        let data = snapshot ?? model.dashboard
        return ((data.quotaEstimates ?? []) + (data.creditEstimates ?? [])).filter { $0.accountKey.hasPrefix(provider + ":") }.sorted { $0.checkpointAt > $1.checkpointAt }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(Format.provider(provider) + " · 估值与采样").font(AppFont.title); Spacer(); Button("完成") { dismiss() }.disabled(model.automaticSaving) }
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    AutomaticValuationControls(model: model, sources: sources)
                    if sources.isEmpty { Text("添加数据目录后可配置自动估值。").foregroundStyle(.secondary) }
                    DisclosureGroup("手动采样（进行中 \(records.filter { $0.status != "completed" && $0.samplingMode != "automatic" }.count)）", isExpanded: $manualExpanded) {
                        if manualExpanded {
                    ForEach(accounts, id: \.key) { account in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack {
                                Text(account.name); Spacer()
                                Button("额度采样") { Task { await model.openEstimate(accountKey: account.key) } }
                                if provider == "codex" { Button("credits 采样") { Task { await model.openEstimate(accountKey: account.key, credits: true) } } }
                            }
                            if let error = model.estimateErrors[account.key] { ErrorNotice(message: error).font(AppFont.secondary).foregroundStyle(Palette.warn) }
                        }
                    }
                    if accounts.isEmpty { Text("添加账户后可开始手动采样。").foregroundStyle(.secondary) }
                    recordSection("进行中的手动采样", records.filter { $0.status != "completed" && $0.samplingMode != "automatic" })
                    recordSection("手动采样历史", records.filter { $0.status == "completed" && $0.samplingMode != "automatic" })
                        }
                    }
                    recordSection("自动采样历史", records.filter { $0.samplingMode == "automatic" })
                    if loading { ProgressView("正在读取采样记录…") }
                    if let error { HStack { ErrorNotice(message: error).foregroundStyle(Palette.warn); Button("重试读取") { Task { await load() } } } }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogValuationWidth, height: 650).aieyesAccent()
            .interactiveDismissDisabled(model.automaticSaving)
            .updateOperation("估值与采样", active: model.automaticSaving)
            .task(id: recordsRevision) { await load() }
    }
    private var recordsRevision: String {
        ((model.dashboard.quotaEstimates ?? []) + (model.dashboard.creditEstimates ?? [])).filter { $0.accountKey.hasPrefix(provider + ":") }.map { $0.recordKey + ":" + $0.status + ":" + String($0.checkpointAt) }.joined(separator: "|")
    }
    private func recordSection(_ title: String, _ values: [QuotaEstimate]) -> some View {
        LazyVStack(alignment: .leading, spacing: 12) {
            Text(title).font(AppFont.section)
            if values.isEmpty { Text("暂无记录").font(AppFont.secondary).foregroundStyle(.secondary) }
            ForEach(values, id: \.recordKey) { record in
                VStack(alignment: .leading, spacing: 6) {
                    HStack { Text(model.settings.historicalAccounts.first { $0.key == record.accountKey }?.name ?? record.accountKey); Spacer(); Button("查看与管理") { Task { await model.openEstimate(record) } } }
                    Text((record.kind == "credits" ? "credits 估值" : (EstimatePresentation.visibility((snapshot ?? model.dashboard).quotas.first { $0.id == record.accountKey }, records: [record], group: record.groupId).periods + " 额度估值").trimmingCharacters(in: .whitespaces)) + " · " + record.statusLabel).font(AppFont.secondary).foregroundStyle(.secondary)
                    Text(Format.date(record.startedAt) + " → " + Format.date(record.checkpointAt)).font(AppFont.secondary).foregroundStyle(.secondary)
                    if let reason = record.rollingExcludedReason { Text(reason).font(AppFont.secondary).foregroundStyle(Palette.warn) }
                    if let error = model.estimateErrors[record.accountKey] { ErrorNotice(message: error).font(AppFont.secondary).foregroundStyle(Palette.warn) }
                }.padding(12).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 10))
            }
        }
    }
    private func load() async {
        loading = true; error = nil; defer { loading = false }
        do {
            var value: Dashboard = try await model.engine.call("dashboard.summary", params: ["estimateHistory":"display"])
            async let quotaRows: [QuotaEstimate] = historyReader.engine.call("quotaEstimates.list", params: ["provider":provider])
            async let creditRows: [QuotaEstimate] = historyReader.engine.call("creditEstimates.list", params: ["provider":provider])
            let rows = try await (quotaRows, creditRows); value.quotaEstimates = rows.0; value.creditEstimates = rows.1; snapshot = value
        }
        catch { self.error = error.localizedDescription }
    }
}

struct QuotaOrderView: View {
    @SurfaceModel var model: AppModel
    var onClose: (() -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var keys: [String] = []
    @State private var initialKeys: [String] = []
    @State private var saving = false
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("调整账户顺序").font(AppFont.title)
            Text("拖动账户或使用上下按钮排序，应用于所有额度面板。").foregroundStyle(.secondary)
            List {
                ForEach(keys, id: \.self) { key in
                    HStack {
                        Image(systemName: "line.3.horizontal").foregroundStyle(.secondary)
                        ProviderMark(provider: key.components(separatedBy: ":").first ?? "custom")
                        Text(model.settings.accounts.first { $0.key == key }?.name ?? key)
                        Spacer()
                        Button { move(key, -1) } label: { Image(systemName: "arrow.up") }.disabled(keys.first == key).accessibilityLabel("上移账户")
                        Button { move(key, 1) } label: { Image(systemName: "arrow.down") }.disabled(keys.last == key).accessibilityLabel("下移账户")
                    }.buttonStyle(.borderless).padding(.vertical, 4)
                }.onMove { from, to in withAnimation(reduceMotion ? nil : .smooth(duration: 0.25)) { keys.move(fromOffsets: from, toOffset: to) } }
            }.frame(minHeight: 150, maxHeight: 300)
            if let error { ErrorNotice(message: error) }
            HStack { Spacer(); Button("取消") { close() }.keyboardShortcut(.cancelAction); Button(saving ? "保存中…" : "保存顺序") {
                Task { await saveOrder() }
            }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction) }.disabled(saving)
        }.aieyesAccent().font(AppFont.body).padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogSelectionWidth).onAppear { keys = model.dashboard.quotaOrder ?? model.quotaAccounts.map(\.key); initialKeys = keys }
        .updateDraftGuard("账户顺序", snapshot: draftSnapshot(keys), dirty: keys != initialKeys, saving: saving, save: { await saveOrder() }, discard: close)
    }
    @discardableResult private func saveOrder() async -> Bool {
        saving = true; error = nil; defer { saving = false }
        do { try await model.saveQuotaOrder(keys); close(); return true } catch { self.error = error.localizedDescription; return false }
    }
    private func close() { if let onClose { onClose() } else { dismiss() } }
    private func move(_ key: String, _ offset: Int) {
        guard let i = keys.firstIndex(of: key), keys.indices.contains(i + offset) else { return }
        withAnimation(reduceMotion ? nil : .spring(response: 0.3)) { keys.swapAt(i, i + offset) }
    }
}

struct PanelAccountsView: View {
    @SurfaceModel var model: AppModel
    var onClose: () -> Void
    @AppStorage(NativePanelKey.accounts.rawValue) private var stored = "{}"
    @State private var selected: [String: [String]] = [:]
    @State private var automatic = Set<String>()
    @State private var initialDraft = ""
    private var draft: String { PanelAccountPreference.encode(selected, automatic: automatic) }
    private var snapshot: String { draftSnapshot(selected) + draftSnapshot(automatic.sorted()) }
    @FocusState private var cancelFocused: Bool
    private func members(_ provider: String) -> [String] { Array(Set(model.quotaAccounts.map(\.provider))).filter { CoreContract.sameAgent($0, provider) } }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("面板显示账户").font(AppFont.title)
            Text("自动显示每类前 5 个账户；新增账户默认加入。手动选择可留空、不限数量，仅影响本机面板。").font(AppFont.secondary).foregroundStyle(.secondary)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    ForEach(Array(Set(model.quotaAccounts.map { CoreContract.agentProvider($0.provider) })).sorted(), id: \.self) { provider in
                        VStack(alignment: .leading, spacing: 8) {
                            HStack { ProviderIdentity(provider: provider); Text("· \(members(provider).reduce(0) { $0 + (selected[$1]?.count ?? 0) }) 个"); Spacer(); Button(members(provider).allSatisfy { automatic.contains($0) } ? "自动选择" : "恢复自动选择") {
                                let defaults = PanelAccountPreference.selections("{}", accounts: model.settings.accounts, order: model.dashboard.quotaOrder ?? [])
                                for member in members(provider) { automatic.insert(member); selected[member] = defaults[member] }
                            }.font(AppFont.secondary) }.font(AppFont.section)
                            ForEach(model.quotaAccounts.filter { CoreContract.sameAgent($0.provider, provider) }, id: \.key) { account in
                                Toggle(account.name, isOn: Binding(get: { selected[account.provider]?.contains(account.key) == true }, set: { enabled in
                                    for member in members(provider) { automatic.remove(member) }
                                    var keys = selected[account.provider] ?? []
                                    if enabled && !keys.contains(account.key) { keys.append(account.key) } else if !enabled { keys.removeAll { $0 == account.key } }
                                    selected[account.provider] = keys
                                }))
                            }
                        }
                    }
                    if model.quotaAccounts.isEmpty { Text("尚无可显示额度的账户").foregroundStyle(.secondary) }
                }.padding(4)
            }
            HStack { Spacer(); Button("取消", action: onClose).keyboardShortcut(.cancelAction).focused($cancelFocused); Button("保存面板选择") { stored = PanelAccountPreference.encode(selected, automatic: automatic); onClose() }.buttonStyle(.aieyesPrimary) }
        }.padding(DesignTokens.dialogPadding).font(AppFont.body).frame(width: DesignTokens.dialogSelectionWidth, height: 480).aieyesAccent()
            .onAppear { if stored == "{}" { stored = PanelAccountPreference.migrated(UserDefaults.standard.string(forKey: NativePanelKey.accountsLegacy.rawValue) ?? "{}") }; automatic = Set(model.quotaAccounts.map(\.provider).filter { PanelAccountPreference.automatic(stored, provider: $0) }); selected = PanelAccountPreference.selections(stored, accounts: model.settings.accounts, order: model.dashboard.quotaOrder ?? []); for (provider, choice) in PanelAccountPreference.decode(stored).providers where choice.mode == "custom" { selected[provider] = choice.keys }; initialDraft = snapshot; cancelFocused = true }
            .updateDraftGuard("面板显示账户", snapshot: snapshot, dirty: !initialDraft.isEmpty && initialDraft != snapshot, saving: false, save: { stored = draft; onClose(); return true }, discard: onClose)
    }
}

struct QuotaEstimateView: View {
    @StateObject private var historyReader = EstimateHistoryReader()
    @SurfaceModel var model: AppModel
    let quota: Quota
    var onClose: (() -> Void)?
    @Environment(\.dismiss) private var dismiss
    @State private var fullQuotaRecords: [QuotaEstimate]?
    @State private var fullCreditRecords: [QuotaEstimate]?
    @State private var historyError: String?
    @State private var selected = Set<String>()
    @State private var window = ""
    @State private var confirmed = false
    @State var credits = false
    var initialGroup: String? = nil
    private var busy: Bool { model.estimateBusy.contains(quota.id) }
    private var error: String? { model.estimateErrors[quota.id] }
    private var sources: [AgentSource] {
        guard let account = model.settings.accounts.first(where: { $0.provider == quota.provider && $0.id == quota.accountId }) else { return [] }
        return model.settings.usageSources.filter { account.uses($0) }
    }
    private var currentQuota: Quota { model.dashboard.quotas.first { $0.id == quota.id && $0.updatedAt >= quota.updatedAt } ?? quota }
    private var windows: [QuotaWindow] { EstimatePresentation.windows(currentQuota) }
    private var visibility: EstimatePresentation.Visibility { EstimatePresentation.visibility(currentQuota, records: records, group: initialGroup) }
    private var records: [QuotaEstimate] { (credits ? (fullCreditRecords ?? model.dashboard.creditEstimates ?? []) : (fullQuotaRecords ?? model.dashboard.quotaEstimates ?? [])).filter { $0.accountKey == quota.id } }
    private var current: QuotaEstimate? { records.first { $0.status != "completed" } }
    private var eligible: Bool { if credits { return quota.credits?.balance != nil && quota.credits?.unlimited == false }; return windows.contains { initialGroup == nil || $0.estimateGroup == initialGroup } }
    private var title: String {
        credits ? "credit 价值" : (visibility.periods + " 额度价值").trimmingCharacters(in: .whitespaces)
    }
    private var historyRevision: String {
        let rows = (model.dashboard.quotaEstimates ?? []) + (model.dashboard.creditEstimates ?? [])
        return rows.filter { $0.accountKey == quota.id }
            .map { $0.recordKey + ":" + $0.status + ":" + String($0.checkpointAt) }
            .joined(separator: "|")
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header
            if quota.provider == "codex" {
                Picker("采样口径", selection: $credits) {
                    Text("额度价值").tag(false)
                    Text("credit 价值").tag(true)
                }.pickerStyle(.segmented).disabled(busy)
            }
            ScrollView { content }.frame(maxHeight: 480)
        }
        .aieyesAccent().font(AppFont.body).disabled(model.installingUpdate)
        .padding(DesignTokens.dialogPadding)
        .frame(minWidth: DesignTokens.dialogEstimateMinWidth, idealWidth: DesignTokens.dialogEstimateWidth)
        .task(id: historyRevision) { await loadHistory() }
        .onChange(of: credits) { _, _ in confirmed = false }
        .onAppear {
            selected = Set(sources.map(\.id))
            window = windows.first { ($0.groupId ?? $0.groupName) == initialGroup }?.id ?? windows.first?.id ?? ""
        }
    }
    private var header: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(AppFont.title)
                Text(quota.name).foregroundStyle(.secondary)
            }
            Spacer()
            Button("关闭") { close() }.keyboardShortcut(.cancelAction)
        }
    }
    private var content: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(credits ? "按 credits 消耗与样本成本估算 API 等价价值，金额不可兑换。" : "按样本成本与额度消耗比例估算 API 等价价值，金额不可兑换。" + visibility.help).foregroundStyle(.secondary)
            AutomaticValuationControls(model: model, sources: sources)
            sampling
            if busy { progress }
            if let error { ErrorNotice(message: error) }
            if let historyError {
                HStack {
                    ErrorNotice(message: historyError)
                    Button("重试") { Task { await loadHistory() } }
                }
            }
            ForEach((model.dashboard.automaticEstimateSummaries ?? []).filter { $0.accountKey == quota.id && ($0.kind == "credits") == credits && (initialGroup == nil || $0.groupId == initialGroup) }) { estimate in result(estimate) }
            history
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
    @ViewBuilder private var sampling: some View {
        if let current {
            activeSampling(current)
        } else if sources.contains(where: { $0.autoValuation?.enabled == true }) {
            Text("自动估值正在等待有效样本。关闭来源的自动估值后可开始手动采样。").foregroundStyle(.secondary)
        } else if eligible && !sources.isEmpty {
            newSampling
        } else {
            Text(sources.isEmpty ? "需要可采集 Token 的关联数据源，请检查数据目录和账户关联。" : credits ? "需要明确且有限的 credits 余额；请刷新账户额度。" : "此额度池缺少可靠的模型映射，暂不支持估值。").foregroundStyle(.secondary)
        }
    }
    @ViewBuilder private func activeSampling(_ current: QuotaEstimate) -> some View {
        if let group = current.groupId, let initialGroup, group != initialGroup {
            Text("此账户正在采样其他模型组，请先结束当前采样。").foregroundStyle(.secondary)
        }
        result(current)
        if current.status == "pending" {
            Text(current.reason).foregroundStyle(Palette.warn)
            confirmation
            Button("确认并开始新一段") {
                perform("restart", ["id": current.id, "confirmed": confirmed])
            }.disabled(!confirmed || busy)
        }
        Button(current.status == "pending" ? "结束并保留有效段" : "结束并保存本段结果") {
            perform("stop", ["id": current.id])
        }.buttonStyle(.aieyesPrimary).disabled(busy)
    }
    @ViewBuilder private var newSampling: some View {
        if !credits {
            Picker("主要额度池", selection: $window) {
                ForEach(windows) { Text($0.name).tag($0.id) }
            }
        }
        Text("纳入的用量来源").font(AppFont.section)
        ForEach(sources) { source in
            Toggle(source.name, isOn: Binding(get: { selected.contains(source.id) }, set: {
                if $0 { selected.insert(source.id) } else { selected.remove(source.id) }
            })).disabled(busy)
        }
        confirmation
        if sources.contains(where: { $0.codexHomeId != nil }) {
            Text("共享目录仅统计本次采样时段。切换账户前请结束采样，切换后重新开始。").font(AppFont.secondary).foregroundStyle(.secondary)
        }
        Text(credits ? "开始后请新建会话。有可确认的 credits 消耗且价格完整时显示估值；充值或订阅额度恢复后请重新采样。" : "开始后请新建会话。有可确认的额度消耗且价格完整时显示估值；跨采样边界的累计用量不计入。").font(AppFont.secondary).foregroundStyle(.secondary)
        Button("开始采样") {
            perform("start", ["accountKey": quota.id, "windowId": credits ? "credits" : window, "sourceIds": Array(selected), "confirmed": confirmed])
        }.buttonStyle(.aieyesPrimary).disabled(!confirmed || selected.isEmpty || (!credits && window.isEmpty) || busy)
    }
    private var progress: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let elapsed = Int(context.date.timeIntervalSince(model.estimateStartedAt[quota.id] ?? context.date))
            HStack {
                ProgressView().controlSize(.small)
                Text((model.estimateStages[quota.id] ?? "准备同步") + " · " + String(elapsed) + " 秒")
            }.font(AppFont.secondary)
        }
    }
    @ViewBuilder private var history: some View {
        if records.contains(where: { $0.status == "completed" }) {
            Divider()
            Text("采样历史").font(AppFont.section)
            ForEach(records.filter { $0.status == "completed" && (initialGroup == nil || $0.groupId == nil || $0.groupId == initialGroup) }) { result($0) }
        }
    }
    private func loadHistory() async {
        do {
            async let quotaRows: [QuotaEstimate] = historyReader.engine.call("quotaEstimates.list", params: ["accountKey":quota.id])
            async let creditRows: [QuotaEstimate] = historyReader.engine.call("creditEstimates.list", params: ["accountKey":quota.id])
            let values = try await (quotaRows, creditRows)
            guard !Task.isCancelled else { return }; fullQuotaRecords = values.0; fullCreditRecords = values.1; historyError = nil
        } catch { historyError = "历史读取失败：" + error.localizedDescription }
    }
    private var confirmation: some View {
        Toggle(credits ? "我确认本段仅消耗 credits，所有设备用量均已纳入；不混用包含额度、API 或中转。" : "我确认采样期间只使用目标订阅，所有设备的用量均已纳入所选来源；不混用 API / 中转。", isOn: $confirmed).fixedSize(horizontal: false, vertical: true)
    }
    private func result(_ e: QuotaEstimate) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if let group = e.groupId { Text(quota.windows.first { $0.estimateGroup == group }?.groupLabel ?? group).font(AppFont.section) }
            else if quota.provider == "antigravity" { Text("账户总体 · 旧记录").font(AppFont.section) }
            resultHeader(e)
            if e.kind != "credits" { quotaValues(e) }
            Text(e.calculationNote).font(AppFont.secondary).foregroundStyle(.secondary)
            if let reason = e.rollingExcludedReason { Text(reason).font(AppFont.secondary).foregroundStyle(Palette.warn) }
            if e.originalEstimateId != nil { Text("已修正 · 原采样记录保留").font(AppFont.secondary).foregroundStyle(.secondary) }
            else if e.samplingMode != "automatic" && e.status == "completed" && !e.hasValue { Button("按原记录修复估值") { perform("repair", ["id": e.id]) }.disabled(busy) }
            DisclosureGroup("计算依据") { calculationDetails(e) }
        }.padding(14).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
    }
    private func resultHeader(_ e: QuotaEstimate) -> some View {
        HStack {
            Text(e.samplingMode == "rolling" ? (e.status == "completed" ? "滚动估值 · 历史" : "滚动估值 · 持续更新") : e.samplingMode == "automatic" ? (e.status == "completed" ? "自动估值 · 已封存" : "自动估值 · 持续更新") : e.status == "active" ? ((e.fiveHourValue ?? e.weeklyValue ?? e.valuePer1000) == nil ? "正在积累样本" : "可输出估值 · 采样继续") : e.statusLabel).foregroundStyle(e.status == "pending" ? Palette.warn : Palette.accent)
            Spacer()
            Text(resultValue(e)).font(AppFont.section).monospacedDigit()
        }
    }
    private func resultValue(_ e: QuotaEstimate) -> String {
        if e.kind == "credits" { return Format.creditValue(e) }
        let periods = EstimatePresentation.visibility(currentQuota, records: [e], group: e.groupId).periods
        return periods.isEmpty ? "额度估值" : periods
    }
    private func amount(_ value: Double?) -> String {
        guard let value else { return "—" }
        return Format.money(value) + " USD"
    }
    private func quotaValues(_ e: QuotaEstimate) -> some View {
        let visible = EstimatePresentation.visibility(currentQuota, records: [e], group: e.groupId)
        var values: [(String, Double?)] = []
        if visible.five { values.append(("本段 5h API 等价值", e.fiveHourValue)) }
        if visible.direct { values.append((e.valuationMode == "fiveHour" ? "7d 推算 · 同期消耗比例" : "7d 直接估值", e.weeklyDirectValue ?? e.weeklyValue)) }
        if visible.ratio { values.append(("7d 推算 · 近期容量倍率", e.weeklyRatioValue)) }
        return VStack(alignment: .leading, spacing: 8) {
            ForEach(values, id: \.0) { label, value in
                HStack { Text(label); Spacer(); Text(amount(value)).monospacedDigit() }.font(label.hasPrefix("本段") ? AppFont.section : AppFont.secondary)
            }
            if visible.ratio { Text(capacityDescription(e.capacity)).font(AppFont.secondary).foregroundStyle(.secondary) }
        }
    }
    private func capacityDescription(_ capacity: CapacityInfo?) -> String {
        guard let capacity, let ratio = capacity.ratio else { return "倍率学习中" }
        let value = "容量倍率 " + String(format: "%.2f", ratio)
        let samples = String(capacity.samples) + " 个样本"
        return [value, capacity.note, samples, Format.date(capacity.updatedAt)].joined(separator: " · ")
    }
    private func calculationDetails(_ e: QuotaEstimate) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("\(Format.date(e.startedAt)) → \(Format.date(e.checkpointAt))")
            Text("\(e.windowName) · 消耗 \(consumption(e)) · 样本成本 \(Format.money(e.cost)) USD")
            Text("计价 Token：\(Format.compact(e.pricedTokens)) / \(Format.compact(e.totalTokens))")
            Text(e.sourceNames.joined(separator: "、"))
            if let segments = e.segments, !segments.isEmpty {
                Text("已保存 \(segments.count) 个有效片段")
                ForEach(Array(segments.enumerated()), id: \.offset) { _, segment in
                    Text("\(Format.date(segment.startedAt)) → \(Format.date(segment.endedAt)) · 5h 消耗 \(Format.percent(segment.fivePercent)) · \(Format.money(segment.cost))")
                }
            }
            Text(calculationRule(e))
            ForEach(e.prices ?? []) { price in priceDetails(price) }
            if !e.reason.isEmpty { Text(e.reason).foregroundStyle(Palette.warn) }
        }.font(AppFont.secondary).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading)
    }
    private func consumption(_ e: QuotaEstimate) -> String {
        if e.kind == "credits" { return Format.credits(e.consumedCredits.map { String($0) }) + " credit" }
        return Format.percent(e.consumedPercent)
    }
    private func calculationRule(_ e: QuotaEstimate) -> String {
        if e.kind == "credits" { return "1000 credit 估值 = 样本成本 × 1000 ÷ 消耗 credit；结束后价格依据固定。" }
        let visible = EstimatePresentation.visibility(currentQuota, records: [e], group: e.groupId)
        let rules = [
            visible.five ? "5h 价值 = 有效成本 × 100 ÷ 5h 消耗百分点" : "",
            visible.direct ? "7d 直接估值 = 样本 API 等价成本 × 100 ÷ 周额度消耗百分点" : "",
            visible.ratio ? "7d 倍率估算 = 5h 价值 × 近期容量倍率" : ""
        ].filter { !$0.isEmpty }
        return (rules.isEmpty ? "按采样时段的 API 等价成本与对应额度消耗比例计算" : rules.joined(separator: "；")) + "；结束后价格与倍率固定。"
    }
    private func priceDetails(_ price: ModelPrice) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("缓存读取 \(price.cacheRead.map { Format.money($0 * 1_000_000) } ?? "—") · 缓存写入 \(price.cacheWrite.map { Format.money($0 * 1_000_000) } ?? "—") / 百万 Token")
            Text("价格依据：" + price.id + " · " + Format.date(price.fetchedAt))
            Text("每百万 Token USD：输入 \(price.input.map { Format.money($0 * 1_000_000) } ?? "—") · 输出 \(price.output.map { Format.money($0 * 1_000_000) } ?? "—")")
        }
    }
    private func close() { if let onClose { onClose() } else { dismiss() } }
    private func perform(_ action: String, _ params: [String: Any]) {
        Task { @MainActor in
            if await model.performEstimate(action, accountKey: quota.id, params: params, credits: credits) {
                confirmed = false
                if action == "start" || action == "restart" { close() }
            }
        }
    }
}
