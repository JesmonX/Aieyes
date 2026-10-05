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
                            Spacer(); Text(estimate.statusLabel).foregroundStyle(estimate.status == "pending" ? .orange : .teal)
                        }
                        Text(estimate.calculationNote).font(AppFont.secondary)
                        if !estimate.reason.isEmpty { Text(estimate.reason).foregroundStyle(.orange) }
                        if let error = model.estimateErrors[estimate.accountKey] { Text(error).foregroundStyle(.orange) }
                        HStack {
                            Button("查看与管理") { Task { await model.openEstimate(estimate) } }
                            Spacer()
                            Button("结束采样") { Task { _ = await model.performEstimate("stop", accountKey: estimate.accountKey, params: ["id": estimate.id]) } }
                                .disabled(model.estimateBusy.contains(estimate.accountKey))
                        }
                    }.padding(14).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
                }
            }.padding(24).frame(maxWidth: .infinity, alignment: .leading)
        }.font(AppFont.body).disabled(model.installingUpdate)
    }
}

struct QuotaOrderView: View {
    @ObservedObject var model: AppModel
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
                        Text(model.settings.accounts.first { $0.key == key }?.name ?? key)
                        Spacer()
                        Button { move(key, -1) } label: { Image(systemName: "arrow.up") }.disabled(keys.first == key).accessibilityLabel("上移账户")
                        Button { move(key, 1) } label: { Image(systemName: "arrow.down") }.disabled(keys.last == key).accessibilityLabel("下移账户")
                    }.buttonStyle(.borderless).padding(.vertical, 4)
                }.onMove { from, to in withAnimation(reduceMotion ? nil : .smooth(duration: 0.25)) { keys.move(fromOffsets: from, toOffset: to) } }
            }.frame(minHeight: 150, maxHeight: 300)
            if let error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button(saving ? "保存中…" : "保存顺序") {
                saving = true
                Task { do { try await model.saveQuotaOrder(keys); dismiss() } catch { self.error = error.localizedDescription }; saving = false }
            }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction) }.disabled(saving)
        }.font(AppFont.body).padding(24).frame(width: 440).onAppear { keys = model.dashboard.quotaOrder ?? model.quotaAccounts.map(\.key) }
    }
    private func move(_ key: String, _ offset: Int) {
        guard let i = keys.firstIndex(of: key), keys.indices.contains(i + offset) else { return }
        withAnimation(reduceMotion ? nil : .spring(response: 0.3)) { keys.swapAt(i, i + offset) }
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
    private var busy: Bool { model.estimateBusy.contains(quota.id) }
    private var error: String? { model.estimateErrors[quota.id] }
    private var sources: [AgentSource] { model.settings.sources.filter { $0.enabled && $0.provider == quota.provider && $0.accountId == quota.accountId && !["agy", "deepseek"].contains($0.provider) } }
    private var windows: [QuotaWindow] { quota.windows.filter { $0.windowMinutes == 10080 } }
    private var records: [QuotaEstimate] { (model.dashboard.quotaEstimates ?? []).filter { $0.accountKey == quota.id } }
    private var current: QuotaEstimate? { records.first { $0.status != "completed" } }
    private var eligible: Bool { quota.provider != "agy" && (windows.count == 1 || (quota.provider == "claude" && windows.contains { $0.id == "seven_day" || $0.name == "7d" })) }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { VStack(alignment: .leading, spacing: 4) { Text("7d 整周价值").font(AppFont.title); Text(quota.name).foregroundStyle(.secondary) }; Spacer(); Button("关闭") { close() }.keyboardShortcut(.cancelAction) }
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text("按采样期间的 API 等价成本与额度消耗比例，估算一整周额度的价值。结果取决于模型组合，并非可兑换余额。").foregroundStyle(.secondary)
                    if let current {
                        result(current)
                        if current.status == "pending" {
                            Text(current.reason).foregroundStyle(.orange)
                            confirmation
                            Button("确认并开始新一段") { perform("restart", ["id": current.id, "confirmed": confirmed]) }.disabled(!confirmed || busy)
                        }
                        Button(current.status == "pending" ? "结束并保留有效段" : "结束采样并计算") { perform("stop", ["id": current.id]) }.buttonStyle(.borderedProminent).disabled(busy)
                    } else if eligible && !sources.isEmpty {
                        Picker("7d 额度池", selection: $window) {
                            ForEach(windows.filter { windows.count == 1 || $0.id == "seven_day" || $0.name == "7d" }) { Text($0.name).tag($0.id) }
                        }
                        Text("纳入的用量来源").font(AppFont.section)
                        ForEach(sources) { source in
                            Toggle(source.name, isOn: Binding(get: { selected.contains(source.id) }, set: { if $0 { selected.insert(source.id) } else { selected.remove(source.id) } })).disabled(busy)
                        }
                        confirmation
                        Text("建议开始后新建会话。跨起始边界的累计用量会使本次结果不可用；至少消耗 5 个百分点后输出估值。").font(AppFont.secondary).foregroundStyle(.secondary)
                        Button("开始采样") { perform("start", ["accountKey": quota.id, "windowId": window, "sourceIds": Array(selected), "confirmed": confirmed]) }.buttonStyle(.borderedProminent).disabled(!confirmed || selected.isEmpty || window.isEmpty || busy)
                    } else {
                        Text(sources.isEmpty ? "需要可采集 Token 的关联数据源。agy 限额查询本身不提供用量历史。" : "此额度池缺少可靠的模型映射，暂不支持估值。").foregroundStyle(.secondary)
                    }
                    if busy { HStack { ProgressView().controlSize(.small); Text("正在同步用量与限额…") }.font(AppFont.secondary) }
                    if let error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
                    if records.contains(where: { $0.status == "completed" }) {
                        Divider(); Text("采样历史").font(AppFont.section)
                        ForEach(records.filter { $0.status == "completed" }) { result($0) }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxHeight: 480)
        }.font(AppFont.body).disabled(model.installingUpdate).padding(24).frame(minWidth: 470, idealWidth: 520)
        .onAppear { selected = Set(sources.map(\.id)); window = windows.first(where: { $0.id == "seven_day" || $0.name == "7d" })?.id ?? windows.first?.id ?? "" }
    }
    private var confirmation: some View {
        Toggle("我确认采样期间只使用目标订阅，所有设备的用量均已纳入所选来源；不混用 API / 中转。", isOn: $confirmed).fixedSize(horizontal: false, vertical: true)
    }
    private func result(_ e: QuotaEstimate) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack { Text(e.statusLabel).foregroundStyle(e.status == "pending" ? .orange : Palette.accent); Spacer(); Text(e.weeklyValue.map { Format.money($0) + " USD" } ?? "—").font(AppFont.section).monospacedDigit() }
            Text(e.calculationNote).font(AppFont.secondary).foregroundStyle(.secondary)
            DisclosureGroup("计算依据") {
                VStack(alignment: .leading, spacing: 5) {
                    Text("\(Format.date(e.startedAt)) → \(Format.date(e.checkpointAt))")
                    Text("\(e.windowName) · 消耗 \(Format.percent(e.consumedPercent)) · 样本成本 \(Format.money(e.cost)) USD")
                    Text("计价 Token：\(Format.compact(e.pricedTokens)) / \(Format.compact(e.totalTokens))")
                    Text(e.sourceNames.joined(separator: "、"))
                    Text("整周估值 = 样本 API 等价成本 × 100 ÷ 消耗百分点；结束后价格依据固定。")
                    ForEach(e.prices ?? []) { price in
                        Text("价格依据：" + price.id + " · " + Format.date(price.fetchedAt))
                        Text("每百万 Token USD：输入 \(price.input.map { Format.money($0 * 1_000_000) } ?? "—") · 输出 \(price.output.map { Format.money($0 * 1_000_000) } ?? "—")")
                    }
                    if !e.reason.isEmpty { Text(e.reason).foregroundStyle(.orange) }
                }.font(AppFont.secondary).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading)
            }
        }.padding(14).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
    }
    private func close() { if let onClose { onClose() } else { dismiss() } }
    private func perform(_ action: String, _ params: [String: Any]) {
        Task { @MainActor in
            if await model.performEstimate(action, accountKey: quota.id, params: params) {
                confirmed = false
                if action == "start" || action == "restart" { close() }
            }
        }
    }
}
