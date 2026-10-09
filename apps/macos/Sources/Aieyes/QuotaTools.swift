import SwiftUI

struct SamplingManagementView: View {
    @ObservedObject var model: AppModel
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("后台采样").font(AppFont.title)
                Text("关闭窗口后采样继续。只有结束采样才会停止记录。").font(AppFont.secondary).foregroundStyle(.secondary)
                if model.runningEstimates.isEmpty { Text("当前没有进行中的采样").foregroundStyle(.secondary) }
                ForEach(model.runningEstimates) { estimate in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Text(model.settings.accounts.first { $0.key == estimate.accountKey }?.name ?? estimate.accountKey).font(AppFont.section)
                            Spacer(); Text(estimate.statusLabel).foregroundStyle(estimate.status == "pending" ? Palette.warn : Palette.ok)
                        }
                        Text(estimate.calculationNote).font(AppFont.secondary)
                        if !estimate.reason.isEmpty { Text(estimate.reason).foregroundStyle(Palette.warn) }
                        if let error = model.estimateErrors[estimate.accountKey] { Text(error).foregroundStyle(Palette.warn) }
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
            }.padding(24).frame(maxWidth: .infinity, alignment: .leading)
        }.aieyesAccent().font(AppFont.body).disabled(model.installingUpdate)
    }
}

struct QuotaOrderView: View {
    @ObservedObject var model: AppModel
    var onClose: (() -> Void)? = nil
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var keys: [String] = []
    @State private var saving = false
    @State private var error: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("调整账户顺序").font(AppFont.title)
            Text("拖动账户，或使用上下按钮。顺序应用于所有限额面板。").foregroundStyle(.secondary)
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
            if let error { Text(error).foregroundStyle(Palette.warn).textSelection(.enabled) }
            HStack { Spacer(); Button("取消") { close() }.keyboardShortcut(.cancelAction); Button(saving ? "保存中…" : "保存顺序") {
                saving = true
                Task { do { try await model.saveQuotaOrder(keys); close() } catch { self.error = error.localizedDescription }; saving = false }
            }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction) }.disabled(saving)
        }.aieyesAccent().font(AppFont.body).padding(24).frame(width: 440).onAppear { keys = model.dashboard.quotaOrder ?? model.quotaAccounts.map(\.key) }
    }
    private func close() { if let onClose { onClose() } else { dismiss() } }
    private func move(_ key: String, _ offset: Int) {
        guard let i = keys.firstIndex(of: key), keys.indices.contains(i + offset) else { return }
        withAnimation(reduceMotion ? nil : .spring(response: 0.3)) { keys.swapAt(i, i + offset) }
    }
}

struct PanelAccountsView: View {
    @ObservedObject var model: AppModel
    var onClose: () -> Void
    @AppStorage("panel.accounts.v2") private var stored = "{}"
    @State private var selected: [String: [String]] = [:]
    @State private var automatic = Set<String>()
    @FocusState private var cancelFocused: Bool
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("面板显示账户").font(AppFont.title)
            Text("每个 Agent 可选择 0～5 个账户。仅影响本机面板，详情与采集保留全部账户。").font(AppFont.secondary).foregroundStyle(.secondary)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    ForEach(Array(Set(model.quotaAccounts.map(\.provider))).sorted(), id: \.self) { provider in
                        VStack(alignment: .leading, spacing: 8) {
                            HStack { ProviderIdentity(provider: provider); Text("· \(selected[provider]?.count ?? 0)/5"); Spacer(); Button(automatic.contains(provider) ? "自动选择" : "恢复默认显示") { automatic.insert(provider); selected[provider] = PanelAccountPreference.selections("{}", accounts: model.settings.accounts, order: model.dashboard.quotaOrder ?? [])[provider] }.font(AppFont.secondary) }.font(AppFont.section)
                            ForEach(model.quotaAccounts.filter { $0.provider == provider }, id: \.key) { account in
                                Toggle(account.name, isOn: Binding(get: { selected[provider]?.contains(account.key) == true }, set: { enabled in
                                    automatic.remove(provider)
                                    var keys = selected[provider] ?? []
                                    if enabled && keys.count < 5 { keys.append(account.key) } else if !enabled { keys.removeAll { $0 == account.key } }
                                    selected[provider] = keys
                                })).disabled(selected[provider]?.contains(account.key) != true && (selected[provider]?.count ?? 0) >= 5)
                            }
                        }
                    }
                    if model.quotaAccounts.isEmpty { Text("尚无可显示限额的账户").foregroundStyle(.secondary) }
                }.padding(4)
            }
            HStack { Spacer(); Button("取消", action: onClose).keyboardShortcut(.cancelAction).focused($cancelFocused); Button("保存面板选择") { stored = PanelAccountPreference.encode(selected, automatic: automatic); onClose() }.buttonStyle(.borderedProminent) }
        }.padding(24).font(AppFont.body).frame(width: 440, height: 480).aieyesAccent()
            .onAppear { if stored == "{}" { stored = PanelAccountPreference.migrated(UserDefaults.standard.string(forKey: "panel.accounts.v1") ?? "{}") }; automatic = Set(model.quotaAccounts.map(\.provider).filter { PanelAccountPreference.automatic(stored, provider: $0) }); selected = PanelAccountPreference.selections(stored, accounts: model.settings.accounts, order: model.dashboard.quotaOrder ?? []); cancelFocused = true }
    }
}

struct QuotaEstimateView: View {
    @ObservedObject var model: AppModel
    let quota: Quota
    var onClose: (() -> Void)?
    @Environment(\.dismiss) private var dismiss
    @State private var selected = Set<String>()
    @State private var window = ""
    @State private var confirmed = false
    @State var credits = false
    var initialGroup: String? = nil
    private var busy: Bool { model.estimateBusy.contains(quota.id) }
    private var error: String? { model.estimateErrors[quota.id] }
    private var sources: [AgentSource] { model.settings.sources.filter { $0.enabled && $0.provider == quota.provider && $0.accountId == quota.accountId && $0.provider != "deepseek" } }
    private var windows: [QuotaWindow] {
        let five = quota.windows.filter { $0.windowMinutes == 300 }
        if ["antigravity", "agy"].contains(quota.provider) {
            return quota.windows.filter { w in w.supportsEstimate && (w.windowMinutes == 300 || (w.windowMinutes == 10080 && !quota.windows.contains { $0.estimateGroup == w.estimateGroup && $0.windowMinutes == 300 })) }
        }
        if five.count == 1 { return five }
        if let overall = five.first(where: { $0.stableId == "five_hour" }) { return [overall] }
        let weekly = quota.windows.filter { $0.windowMinutes == 10080 }
        return weekly.count == 1 ? weekly : weekly.filter { $0.stableId == "seven_day" }
    }
    private var records: [QuotaEstimate] { (credits ? (model.dashboard.creditEstimates ?? []) : (model.dashboard.quotaEstimates ?? [])).filter { $0.accountKey == quota.id } }
    private var current: QuotaEstimate? { records.first { $0.status != "completed" } }
    private var eligible: Bool { if credits { return quota.credits?.balance != nil && quota.credits?.unlimited == false }; return windows.contains { initialGroup == nil || $0.estimateGroup == initialGroup } }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { VStack(alignment: .leading, spacing: 4) { Text(credits ? "credit 价值" : "5h / 7d 额度价值").font(AppFont.title); Text(quota.name).foregroundStyle(.secondary) }; Spacer(); Button("关闭") { close() }.keyboardShortcut(.cancelAction) }
            if quota.provider == "codex" { Picker("采样口径", selection: $credits) { Text("额度价值").tag(false); Text("credit 价值").tag(true) }.pickerStyle(.segmented).disabled(busy) }
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(credits ? "按实际 credits 扣款与样本 API 等价成本，估算 credit 的 API 等价价值，不是可兑换余额。" : "主要采样 5h 额度，显示 5h 价值、7d 同期换算及近期倍率估算。正常重置后自动接续，结果取决于本次模型组合。").foregroundStyle(.secondary)
                    if let current {
                        if let group = current.groupId, let initialGroup, group != initialGroup { Text("当前账户正在采样其他模型组；每个账户同时保留一段采样。").foregroundStyle(.secondary) }
                        result(current)
                        if current.status == "pending" {
                            Text(current.reason).foregroundStyle(Palette.warn)
                            confirmation
                            Button("确认并开始新一段") { perform("restart", ["id": current.id, "confirmed": confirmed]) }.disabled(!confirmed || busy)
                        }
                        Button(current.status == "pending" ? "结束并保留有效段" : "结束并保存本段结果") { perform("stop", ["id": current.id]) }.buttonStyle(.borderedProminent).disabled(busy)
                    } else if eligible && !sources.isEmpty {
                        if !credits { Picker("主要额度池", selection: $window) {
                            ForEach(windows) { Text($0.name).tag($0.id) }
                        } }
                        Text("纳入的用量来源").font(AppFont.section)
                        ForEach(sources) { source in
                            Toggle(source.name, isOn: Binding(get: { selected.contains(source.id) }, set: { if $0 { selected.insert(source.id) } else { selected.remove(source.id) } })).disabled(busy)
                        }
                        confirmation
                        Text(credits ? "建议开始后新建会话，至少消耗 5 credits 且价格完整后输出估值。充值或包含额度恢复后需要开始新一段。" : "建议开始后新建会话。跨起始边界的累计用量会使本次结果不可用；5h 至少消耗 5 个百分点后输出估值；7d 同期消耗较少时会标注样本较少。").font(AppFont.secondary).foregroundStyle(.secondary)
                        Button("开始采样") { perform("start", ["accountKey": quota.id, "windowId": credits ? "credits" : window, "sourceIds": Array(selected), "confirmed": confirmed]) }.buttonStyle(.borderedProminent).disabled(!confirmed || selected.isEmpty || (!credits && window.isEmpty) || busy)
                    } else {
                        Text(sources.isEmpty ? "需要可采集 Token 的关联数据源，请检查数据目录和账户关联。" : credits ? "需要明确且有限的 credits 余额；请刷新账户限额。" : "此额度池缺少可靠的模型映射，暂不支持估值。").foregroundStyle(.secondary)
                    }
                    if busy { TimelineView(.periodic(from: .now, by: 1)) { context in
                        HStack { ProgressView().controlSize(.small); Text((model.estimateStages[quota.id] ?? "准备同步") + " · " + String(Int(context.date.timeIntervalSince(model.estimateStartedAt[quota.id] ?? context.date))) + " 秒") }.font(AppFont.secondary)
                    } }
                    if let error { Text(error).foregroundStyle(Palette.warn).textSelection(.enabled) }
                    if records.contains(where: { $0.status == "completed" }) {
                        Divider(); Text("采样历史").font(AppFont.section)
                        ForEach(records.filter { $0.status == "completed" && (initialGroup == nil || $0.groupId == nil || $0.groupId == initialGroup) }) { result($0) }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxHeight: 480)
        }.aieyesAccent().font(AppFont.body).disabled(model.installingUpdate).padding(24).frame(minWidth: 470, idealWidth: 520)
        .onChange(of: credits) { _, _ in confirmed = false }
        .onAppear { selected = Set(sources.map(\.id)); window = windows.first { ($0.groupId ?? $0.groupName) == initialGroup }?.id ?? windows.first?.id ?? "" }
    }
    private var confirmation: some View {
        Toggle(credits ? "我确认本段仅消耗 credits，所有设备用量均已纳入；不混用包含额度、API 或中转。" : "我确认采样期间只使用目标订阅，所有设备的用量均已纳入所选来源；不混用 API / 中转。", isOn: $confirmed).fixedSize(horizontal: false, vertical: true)
    }
    private func result(_ e: QuotaEstimate) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if let group = e.groupId { Text(quota.windows.first { $0.estimateGroup == group }?.groupLabel ?? group).font(AppFont.section) }
            else if ["antigravity", "agy"].contains(quota.provider) { Text("账户总体 · 旧记录").font(AppFont.section) }
            resultHeader(e)
            if e.valuationMode == "fiveHour" { fiveHourValues(e) }
            Text(e.calculationNote).font(AppFont.secondary).foregroundStyle(.secondary)
            if e.originalEstimateId != nil { Text("已修正 · 原采样记录保留").font(AppFont.secondary).foregroundStyle(.secondary) }
            else if e.status == "completed" && !e.hasValue { Button("按原记录修复估值") { perform("repair", ["id": e.id]) }.disabled(busy) }
            DisclosureGroup("计算依据") { calculationDetails(e) }
        }.padding(14).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
    }
    private func resultHeader(_ e: QuotaEstimate) -> some View {
        HStack {
            Text(e.status == "active" ? ((e.fiveHourValue ?? e.weeklyValue ?? e.valuePer1000) == nil ? "正在积累样本" : "可输出估值 · 采样继续") : e.statusLabel).foregroundStyle(e.status == "pending" ? Palette.warn : Palette.accent)
            Spacer()
            Text(resultValue(e)).font(AppFont.section).monospacedDigit()
        }
    }
    private func resultValue(_ e: QuotaEstimate) -> String {
        if e.kind == "credits" { return Format.creditValue(e) }
        if e.valuationMode == "fiveHour" { return "5h / 7d" }
        return amount(e.weeklyValue)
    }
    private func amount(_ value: Double?) -> String {
        guard let value else { return "—" }
        return Format.money(value) + " USD"
    }
    private func fiveHourValues(_ e: QuotaEstimate) -> some View {
        let values: [(String, Double?)] = [
            ("本段 5h API 等价值", e.fiveHourValue),
            ("7d 推算 · 同期消耗比例", e.weeklyDirectValue),
            ("7d 推算 · 近期容量倍率", e.weeklyRatioValue)
        ]
        return VStack(alignment: .leading, spacing: 8) {
            ForEach(values, id: \.0) { label, value in
                HStack { Text(label); Spacer(); Text(amount(value)).monospacedDigit() }.font(label.hasPrefix("本段") ? AppFont.section : AppFont.secondary)
            }
            Text(capacityDescription(e.capacity)).font(AppFont.secondary).foregroundStyle(.secondary)
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
        if e.valuationMode == "fiveHour" { return "5h / 7d 同期价值 = 有效成本 × 100 ÷ 对应消耗百分点；7d 倍率估算 = 5h 价值 × 近期容量倍率。结束后价格与倍率固定。" }
        return "整周估值 = 样本 API 等价成本 × 100 ÷ 消耗百分点；结束后价格依据固定。"
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
