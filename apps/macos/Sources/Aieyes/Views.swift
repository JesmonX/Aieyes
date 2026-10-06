import SwiftUI
import Charts

struct Surface<Content: View>: View {
    var title: String?
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let title { Text(title).font(.system(size: 15, weight: .semibold)) }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading).padding(16)
        .modifier(NeutralCard())
    }
}

private struct AccessibleSurfacePreview: EnvironmentKey { static let defaultValue = false }
extension EnvironmentValues {
    var previewAccessibleSurfaces: Bool {
        get { self[AccessibleSurfacePreview.self] }
        set { self[AccessibleSurfacePreview.self] = newValue }
    }
}
struct NeutralCard: ViewModifier {
    @Environment(\.previewAccessibleSurfaces) private var previewAccessible
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast
    func body(content: Content) -> some View {
        content.background {
            if previewAccessible || reduceTransparency || contrast == .increased { RoundedRectangle(cornerRadius: Palette.radiusCard).fill(Color(nsColor: .controlBackgroundColor)) }
            else { RoundedRectangle(cornerRadius: Palette.radiusCard).fill(.regularMaterial) }
        }
        .overlay(RoundedRectangle(cornerRadius: Palette.radiusCard).strokeBorder(Palette.cardBorder, lineWidth: previewAccessible || contrast == .increased ? 2 : 1))
        .shadow(color: .black.opacity(0.06), radius: 4, y: 2)
    }
}
struct AttentionNotice: View {
    var reason: String, action: String, perform: () -> Void
    var body: some View {
        HStack(spacing: 8) { Image(systemName: "exclamationmark.triangle"); Text(reason).font(AppFont.secondary).fixedSize(horizontal: false, vertical: true); Spacer(minLength: 0); Button(action, action: perform) }
            .foregroundStyle(Palette.warn).padding(10).overlay(RoundedRectangle(cornerRadius: Palette.radiusControl).strokeBorder(Palette.warn))
    }
}
struct ActionFailureList: View {
    @ObservedObject var model: AppModel
    var body: some View {
        ScrollView { VStack(alignment: .leading, spacing: 8) {
            ForEach(model.actionFailures) { failure in
                HStack(alignment: .top, spacing: 8) { VStack(alignment: .leading, spacing: 3) { Text(failure.name).fontWeight(.semibold); Text(failure.reason).textSelection(.enabled) }; Spacer(minLength: 0); Button("重试") { Task { await model.retry(failure) } }.accessibilityLabel("重试" + failure.name).disabled(model.refreshStates[failure.action]?.busy == true) }
                if failure.id != model.actionFailures.last?.id { Divider() }
            }
        }.font(AppFont.secondary).padding(10) }.frame(maxHeight: 180).modifier(NeutralCard())
    }
}

struct RootView: View {
    @ObservedObject var model: AppModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var compact = true
    @State private var tab = "agent"
    @State private var costMode = false
    @State private var orderingQuotas = false
    var body: some View {
        VStack(spacing: 0) {
            header
            if !model.runningEstimates.isEmpty {
                Button { model.showSampling?() } label: {
                    HStack { Label(model.samplingSummary, systemImage: model.samplingNeedsAttention ? "pause.circle" : "record.circle"); Spacer(); Text("管理采样"); Image(systemName: "chevron.right") }
                        .font(AppFont.secondary).padding(.horizontal, compact ? 18 : 28).padding(.bottom, 10)
                }.buttonStyle(.plain).foregroundStyle(model.samplingNeedsAttention ? .orange : Palette.accent)
            }
            Divider().opacity(0.55)
            ScrollView {
                VStack(alignment: .leading, spacing: compact ? 14 : 22) {
                    if !model.actionFailures.isEmpty { ActionFailureList(model: model) }
                    if let message = model.message, !model.actionFailures.contains(where: { $0.reason == message }) {
                        HStack(spacing: 8) {
                            Text(message).font(AppFont.secondary).textSelection(.enabled)
                            Spacer(); Button { model.message = nil } label: { Image(systemName: "xmark") }.buttonStyle(.plain)
                        }.padding(10).background(.quaternary, in: RoundedRectangle(cornerRadius: 9))
                    }
                    if tab == "agent" { agentContent } else { serverContent }
                }.padding(compact ? 18 : 28).background(OverlayScrollStyle())
            }
            Divider().opacity(0.55)
            footer
        }
        .frame(width: compact ? 450 : nil, height: compact ? model.panelHeight : nil)
        .frame(minWidth: compact ? nil : 760, minHeight: compact ? nil : 480)
        .background {
            ZStack {
                Rectangle().fill(.ultraThinMaterial)
                Color(nsColor: .windowBackgroundColor).opacity(0.5)
            }
        }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: tab)
        .font(AppFont.body).disabled(model.installingUpdate)
        .tint(Palette.accent)
        .sheet(isPresented: $orderingQuotas) { QuotaOrderView(model: model) }
        .onChange(of: model.provider) { _, _ in model.selectedModel = "all"; model.selectedAccount = "all"; model.selectedSource = "all"; Task { await model.reload() } }
        .onChange(of: model.selectedAccount) { _, _ in model.selectedSource = "all"; Task { await model.reload() } }
        .onChange(of: model.selectedSource) { _, _ in Task { await model.reload() } }
        .onChange(of: model.selectedModel) { _, _ in Task { await model.reload() } }
        .onChange(of: model.range) { _, _ in Task { await model.reload() } }
        .onChange(of: tab) { _, value in model.setServerVisible(value == "servers", window: compact ? "panel" : "detail"); if value == "servers" { Task { await model.sampleHosts() } } }
        .onAppear { model.setServerVisible(tab == "servers", window: compact ? "panel" : "detail") }
    }
    private var header: some View {
        HStack(spacing: 12) {
            BrandMark().frame(width: 34, height: 34)
            if !compact { VStack(alignment: .leading, spacing: 1) { Text("Aieyes").font(AppFont.section); EmptyView() } }
            Picker("页面", selection: $tab) { Text("Agent").tag("agent"); Text("服务器").tag("servers") }
                .pickerStyle(.segmented).labelsHidden().frame(maxWidth: compact ? .infinity : 230)
            if !compact { Spacer() }
            Menu {
                Button(model.refreshLabel("scan")) { Task { await model.scan() } }.disabled(model.busy || model.quotaBusy)
                Button(model.refreshLabel("quotas")) { Task { await model.refreshQuotas() } }.disabled(model.quotaBusy || model.busy)
                Button(model.refreshLabel("prices")) { Task { await model.syncPrices() } }.disabled(model.busy || model.quotaBusy)
                Button(model.refreshLabel("hosts")) { Task { await model.sampleHosts() } }.disabled(model.serverBusy)
            } label: {
                Image(systemName: "arrow.clockwise").font(AppFont.secondary)
            }.menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().help("刷新").accessibilityLabel("刷新选项")
        }.padding(.horizontal, compact ? 18 : 28).padding(.vertical, 16)
    }
    private var footer: some View {
        HStack(spacing: 8) {
            Button { Task { if tab == "servers" { await model.sampleHosts() } else { await model.scan() } } } label: {
                HStack(spacing: 6) {
                    if model.busy || model.quotaBusy || model.serverBusy { ProgressView().controlSize(.mini).accessibilityLabel("进行中") }
                    else { Circle().fill(model.actionFailures.isEmpty ? Palette.ok : Palette.danger).frame(width: 7, height: 7) }
                    Text(tab == "servers" ? model.serverStatusText : model.statusText).font(AppFont.secondary).foregroundStyle(.secondary)
                }
            }.help(tab == "servers" ? "服务器采样时间；点击刷新服务器" : "数据生成时间；点击同步记录")
            Spacer()
            if compact {
                Button { model.isPinned.toggle() } label: { Image(systemName: model.isPinned ? "pin.fill" : "pin").frame(width: 30, height: 30).contentShape(Rectangle()) }.help(model.isPinned ? "取消固定" : "固定面板").accessibilityLabel("固定面板").accessibilityValue(model.isPinned ? "已固定" : "未固定")
                Button { model.showDetail?() } label: { Image(systemName: "arrow.up.left.and.arrow.down.right").frame(width: 30, height: 30).contentShape(Rectangle()) }.help("打开详情").accessibilityLabel("打开详情")
            }
            Button { model.showSettings?() } label: { Image(systemName: "gearshape").frame(width: 30, height: 30).contentShape(Rectangle()) }.help("设置").accessibilityLabel("设置").keyboardShortcut(",")
            Menu { Text("关闭窗口后继续在菜单栏运行"); Divider(); Button("退出 Aieyes") { NSApplication.shared.terminate(nil) }.keyboardShortcut("q") } label: { Image(systemName: "ellipsis").frame(width: 30, height: 30).contentShape(Rectangle()) }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().accessibilityLabel("更多操作").help("关闭窗口后继续后台；在此退出应用")
        }.buttonStyle(.plain).padding(.horizontal, compact ? 18 : 28).padding(.vertical, 12)
    }
    @ViewBuilder private var agentContent: some View {
        sessionStrip
        if model.sessionsUnavailable || model.sessions.contains(where: { $0.phase == .unknown }) { AttentionNotice(reason: "待确认：会话状态可能不完整", action: "重试读取") { Task { await model.refreshSessions() } } }
        if model.settings.sources.isEmpty { connectionGuide }
        if hasUsageData || hasActiveFilters {
        HStack(spacing: 8) {
            Picker("Agent", selection: $model.provider) {
                Text("全部 Agent").tag("all")
                ForEach(["codex", "claude", "antigravity", "agy", "deepseek", "custom"], id: \.self) { Text(Format.provider($0)).tag($0) }
            }.labelsHidden()
            Picker("账户", selection: $model.selectedAccount) {
                Text("全部账户").tag("all"); Text("未关联账户").tag("none")
                ForEach(model.settings.accounts.filter { model.provider == "all" || $0.provider == model.provider }, id: \.key) { Text($0.name).tag($0.key) }
            }.labelsHidden()
            if !compact {
                Picker("数据源", selection: $model.selectedSource) {
                    Text("全部数据源").tag("all")
                    ForEach(selectableSources) { Text($0.name).tag($0.id) }
                }.labelsHidden()
                Picker("模型", selection: $model.selectedModel) {
                    Text("全部模型").tag("all")
                    ForEach(Array(Set(model.modelOptions + (model.selectedModel == "all" ? [] : [model.selectedModel]))).sorted(), id: \.self) { Text($0).tag($0) }
                }.labelsHidden()
            }
        }.controlSize(.regular)
        activeFilters
        }
        if !model.dashboard.quotas.isEmpty || !displayedQuotaAccounts.isEmpty { quotaSection }
        if model.samplingNeedsAttention { AttentionNotice(reason: "待确认：采样需要确认用量来源", action: "管理采样") { model.showSampling?() } }
        if hasUsageData {
        if model.dashboard.summary.total > model.dashboard.summary.pricedTokens { AttentionNotice(reason: "计价未完成：部分 Token 缺少价格", action: "补充价格") { model.openPricing() } }
        HStack(alignment: .firstTextBaseline) {
            Text(model.range == 1 ? "今日概览" : "使用概览").font(.system(size: compact ? 20 : 22, weight: .semibold))
            Spacer()
            Picker("时间范围", selection: $model.range) {
                Text("今日").tag(1); Text("最近 7 天").tag(7); Text("最近 30 天").tag(30); Text("最近 90 天").tag(90); Text("最近一年").tag(365)
            }.labelsHidden().fixedSize().controlSize(.regular)
        }
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: compact ? 2 : 3), spacing: 10) {
            StatCard(title: "总 Token", value: Format.compact(model.dashboard.summary.total), detail: "", icon: "sparkle", accent: Palette.accent)
            StatCard(title: "API 等价成本", value: model.dashboard.summary.pricedTokens > 0 ? Format.money(model.dashboard.summary.cost) : "—", detail: "", icon: "dollarsign.circle", accent: .teal, pricingIncomplete: model.dashboard.summary.total > 0 && model.dashboard.summary.pricedTokens < model.dashboard.summary.total, onPricing: { model.openPricing() })
            if !compact { CacheSummaryCard(tokens: model.dashboard.summary.tokens) }
        }
        if compact { CacheSummaryCard(tokens: model.dashboard.summary.tokens) }
        if compact {
            Surface {
                DisclosureGroup("近 \(max(7, model.range)) 天用量") {
                UsageChart(days: recentDays, rows: recentRows, cost: false).frame(height: 85)
                ModelKey(rows: recentRows)
                DailyUsage(days: recentDays, rows: recentRows, compact: true)
                }
            }
            Button { model.showDetail?() } label: { HStack { Text("用量详情"); Spacer(); Image(systemName: "arrow.up.right") }.font(AppFont.secondary) }.buttonStyle(.plain)
        } else {
            HStack {
                Text(model.range == 1 ? "近 7 天趋势" : "使用趋势").font(AppFont.section); Spacer()
                Picker("统计指标", selection: $costMode) { Text("Token").tag(false); Text("API 等价成本").tag(true) }.pickerStyle(.segmented).frame(width: 230)
            }
            HStack(alignment: .top, spacing: 18) {
                Surface(title: "每日用量 · 按模型") { UsageChart(days: model.dashboard.trendDays, rows: model.dashboard.dayModels, cost: costMode).frame(height: 230); ModelKey(rows: model.dashboard.dayModels) }
                Surface(title: "所选范围 · 模型分布") { ModelChart(models: model.dashboard.models, cost: costMode).frame(height: 230) }.frame(width: 340)
            }
            Surface(title: "每日明细") { DailyUsage(days: model.dashboard.trendDays, rows: model.dashboard.dayModels, compact: false, cost: costMode) }
            Surface(title: "过去 365 天") { Heatmap(days: model.dashboard.heatmap, cost: costMode) }
        }
        } else if !model.settings.sources.isEmpty {
            Surface(title: "暂无用量记录") { Text("当前筛选范围没有记录，可清除筛选或同步记录后再查看。").font(AppFont.secondary).foregroundStyle(.secondary); Button("同步记录") { Task { await model.scan() } }.disabled(model.busy || model.quotaBusy) }
        }
    }
    private var hasActiveFilters: Bool { model.provider != "all" || model.selectedAccount != "all" || model.selectedSource != "all" || model.selectedModel != "all" || model.range != 1 }
    private var hasUsageData: Bool {
        model.dashboard.summary.total > 0 || model.dashboard.trendDays.contains { $0.total > 0 } || model.dashboard.heatmap.contains { $0.total > 0 }
    }
    private var connectionGuide: some View {
        Surface(title: "连接你的第一个数据源") {
            Text("接入日志查看用量，或连接账户查看限额与余额。").font(AppFont.secondary).foregroundStyle(.secondary)
            ViewThatFits(in: .horizontal) {
                HStack { connectionButtons }
                VStack(alignment: .leading, spacing: 8) { connectionButtons }
            }
        }
    }
    @ViewBuilder private var connectionButtons: some View {
        Button("本机日志", systemImage: "folder") { model.settingsTab = "sources"; model.requestedSourceProvider = "codex"; model.showSettings?() }
        Button("账户限额", systemImage: "person.crop.circle") { model.settingsTab = "sources"; model.requestedSourceProvider = "deepseek"; model.showSettings?() }
        Button("SSH 主机", systemImage: "server.rack") { model.settingsTab = "servers"; model.requestHostEditor = true; model.showSettings?() }
    }
    @ViewBuilder private var activeFilters: some View {
        if hasActiveFilters {
            VStack(alignment: .leading, spacing: 6) {
                if model.provider != "all" { filterChip("Agent：" + Format.provider(model.provider)) { model.provider = "all" } }
                if model.selectedAccount != "all" { filterChip("账户：" + (model.selectedAccount == "none" ? "未关联账户" : model.settings.accounts.first { $0.key == model.selectedAccount }?.name ?? model.selectedAccount)) { model.selectedAccount = "all" } }
                if model.range != 1 { filterChip("最近 \(model.range) 天") { model.range = 1 } }
                Button("清除全部") { model.provider = "all"; model.selectedAccount = "all"; model.selectedSource = "all"; model.selectedModel = "all"; model.range = 1 }
                if model.selectedSource != "all" {
                    filterChip("数据源：" + (model.settings.sources.first { $0.id == model.selectedSource }?.name ?? model.selectedSource)) { model.selectedSource = "all" }
                }
                if model.selectedModel != "all" { filterChip("模型：" + model.selectedModel) { model.selectedModel = "all" } }
            }
        }
    }
    private func filterChip(_ title: String, clear: @escaping () -> Void) -> some View {
        HStack(spacing: 8) {
            Text(title).font(AppFont.secondary).lineLimit(2).help(title)
            Spacer(minLength: 4)
            Button(action: clear) { Image(systemName: "xmark.circle.fill").frame(width: 28, height: 28) }.buttonStyle(.plain).accessibilityLabel("清除" + title)
        }.padding(.leading, 10).background(Palette.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
    }
    private var recentDays: [Aggregate] { Array(model.dashboard.trendDays.suffix(max(7, model.range))) }
    private var selectableSources: [AgentSource] {
        let account = model.settings.accounts.first { $0.key == model.selectedAccount }
        return model.settings.sources.filter { source in
            guard model.provider == "all" || source.provider == model.provider else { return false }
            if model.selectedAccount == "all" { return true }
            let ids = model.dashboard.sources.first { $0.id == source.id }?.accountIds ?? [source.accountId]
            if model.selectedAccount == "none" { return ids.contains("") }
            return source.provider == account?.provider && ids.contains(account?.id ?? "")
        }
    }
    private var recentRows: [DayModel] {
        let dates = Set(recentDays.map(\.key))
        return model.dashboard.dayModels.filter { dates.contains($0.day) }
    }
    private var sessionStrip: some View {
        DisclosureGroup {
            VStack(spacing: 12) {
                ForEach(model.sessions) { session in
                    HStack(spacing: 10) {
                        ActivityIndicator(phase: session.phase)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(session.source).lineLimit(1)
                            TimelineView(.periodic(from: .now, by: 60)) { context in
                                Text("会话 " + String(URL(fileURLWithPath: session.id).deletingPathExtension().lastPathComponent.prefix(8)) + " · " + Format.relative(session.updatedAt, now: context.date)).font(AppFont.secondary).foregroundStyle(.secondary)
                            }
                        }
                        Spacer()
                        Text(session.phase.rawValue).foregroundStyle(session.phase.color)
                    }
                }
                if model.sessions.isEmpty { Text(model.sessionsUnavailable ? "读取失败" : "暂无会话动态").foregroundStyle(.secondary) }
            }.padding(.top, 12)
        } label: {
            HStack(spacing: 9) {
                ActivityIndicator(phase: model.sessionPhase)
                Text(model.sessionSummary).font(.system(size: 15, weight: .medium)).help("本机 Codex · 日志状态")
                Spacer()
                if let phase = model.sessionPhase { Text(phase.rawValue).font(AppFont.secondary).foregroundStyle(phase.color) }
            }
        }
        .padding(14).modifier(NeutralCard())
    }
    private var displayedQuotaAccounts: [AgentAccount] {
        model.quotaAccounts.filter { account in
            (model.provider == "all" || model.provider == account.provider) && (model.selectedAccount == "all" || model.selectedAccount == account.key)
        }
    }
    private var quotaSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("账户限额").font(.system(size: compact ? 20 : 22, weight: .semibold)); if !displayedQuotaAccounts.isEmpty { Text("\(displayedQuotaAccounts.count)").font(AppFont.secondary).foregroundStyle(.secondary) }; Spacer(); Button { orderingQuotas = true } label: { Image(systemName: "arrow.up.arrow.down") }.help("调整账户顺序").accessibilityLabel("调整账户顺序"); Button(model.quotaBusy ? "读取中…" : "刷新限额") { Task { await model.refreshQuotas() } }.font(AppFont.secondary).disabled(model.quotaBusy) }
            ForEach(displayedQuotaAccounts.filter { account in
                !model.dashboard.quotas.contains { $0.provider == account.provider && $0.accountId == account.id }
            }, id: \.key) { account in
                Surface {
                    HStack { Text(account.name).font(AppFont.section); Spacer(); if model.quotaBusy { ProgressView().controlSize(.small) } }
                    Text(model.quotaBusy ? "正在读取账户限额…" : model.quotaError == nil ? "等待首次限额查询" : "暂时无法读取限额").font(AppFont.secondary).foregroundStyle(.secondary)
                }
            }
            if let error = model.quotaError {
                VStack(alignment: .leading, spacing: 4) {
                    Text(error).font(AppFont.secondary).foregroundStyle(Palette.warn).textSelection(.enabled)
                    if let retry = model.quotaNextAttempt { Text("下次自动重试：" + retry.formatted(date: .omitted, time: .shortened)).font(AppFont.secondary).foregroundStyle(.secondary) }
                }
            }
            if compact { ForEach(model.dashboard.quotas) { quota in QuotaCard(quota: quota, compact: true, estimate: model.dashboard.quotaEstimates?.first { $0.accountKey == quota.id }, onEstimate: { model.creditEstimateMode = false; model.showEstimate?(quota) }, onCredits: { model.creditEstimateMode = true; model.showEstimate?(quota) }, creditEstimate: model.dashboard.creditEstimates?.first { $0.accountKey == quota.id }) } }
            else { LazyVGrid(columns: [GridItem(.adaptive(minimum: 340), spacing: 16, alignment: .top)], alignment: .leading, spacing: 16) { ForEach(model.dashboard.quotas) { quota in QuotaCard(quota: quota, estimate: model.dashboard.quotaEstimates?.first { $0.accountKey == quota.id }, onEstimate: { model.creditEstimateMode = false; model.showEstimate?(quota) }, onCredits: { model.creditEstimateMode = true; model.showEstimate?(quota) }, creditEstimate: model.dashboard.creditEstimates?.first { $0.accountKey == quota.id }) } } }
        }.animation(reduceMotion ? nil : .spring(response: 0.35, dampingFraction: 0.86), value: model.dashboard.quotaOrder)
    }
    @ViewBuilder private var serverContent: some View {
        HStack { VStack(alignment: .leading, spacing: 4) { Text("服务器").font(.system(size: compact ? 20 : 22, weight: .semibold, design: .default)); Text("\(model.settings.hosts.filter(\.enabled).count) 台主机").font(AppFont.secondary).foregroundStyle(.secondary) }; Spacer(); Button { model.settingsTab = "servers"; model.showSettings?() } label: { Image(systemName: "plus") }.help("添加主机") }
        if model.settings.hosts.isEmpty { EmptyCard(icon: "server.rack", title: "添加服务器", subtitle: "", action: { model.settingsTab = "servers"; model.showSettings?() }) }
        ForEach(model.settings.hosts) { host in
            ServerCard(host: host, result: model.hosts.first(where: { $0.id == host.id }), compact: compact, onRefresh: { Task { await model.sampleHosts(hostID: host.id) } })
        }
    }
}

struct TokenBreakdown: View {
    var tokens: Tokens
    var inline = false
    private var values: [(String, Double)] { [("输入", tokens.input), ("输出", tokens.output), ("缓存", tokens.cacheRead + tokens.cacheWrite)] }
    var body: some View {
        if inline {
            HStack(alignment: .top, spacing: 12) {
                ForEach(values, id: \.0) { label, value in
                    VStack(alignment: .leading, spacing: 5) {
                        Text(label == "缓存" ? "缓存（命中率 " + Format.percent(tokens.cacheRate.map { $0 * 100 }) + "）" : label)
                            .font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        Text(Format.compact(value)).font(.system(size: 18, weight: .semibold)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.9)
                    }.frame(minWidth: label == "缓存" ? 170 : nil, maxWidth: .infinity, alignment: .leading)
                }
            }
        } else {
            VStack(spacing: 5) {
                ForEach(values, id: \.0) { label, value in
                    HStack { Text(label).font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Text(Format.compact(value)).font(.system(size: 16, weight: .semibold)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.9) }
                }
            }
        }
    }
}
struct CacheSummaryCard: View {
    var tokens: Tokens
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("缓存命中率").font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Text(Format.percent(tokens.cacheRate.map { $0 * 100 })).font(AppFont.section).monospacedDigit() }
            HStack(alignment: .top, spacing: 10) {
                ForEach([("输入", tokens.input), ("输出", tokens.output), ("缓存", tokens.cacheRead + tokens.cacheWrite)], id: \.0) { title, value in
                    VStack(alignment: .leading, spacing: 4) { Text(title).font(AppFont.secondary).foregroundStyle(.secondary); Text(Format.compact(value)).font(AppFont.section).monospacedDigit().lineLimit(1).minimumScaleFactor(0.8) }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }.padding(14).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).modifier(NeutralCard())
    }
}

struct TokenBreakdownCard: View {
    var tokens: Tokens
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Token 明细").font(.system(size: 14, weight: .medium)).foregroundStyle(.secondary)
            TokenBreakdown(tokens: tokens)
        }.padding(14).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .modifier(NeutralCard())
    }
}

struct StatCard: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var title: String, value: String, detail: String, icon: String, accent: Color
    var pricingIncomplete = false
    var onPricing: () -> Void = {}
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Text(title).font(.system(size: 14, weight: .medium)).foregroundStyle(.secondary)
                if pricingIncomplete {
                    Button(action: onPricing) { Image(systemName: "exclamationmark.circle").foregroundStyle(Palette.warn) }
                        .buttonStyle(.plain)
                        .help("计价覆盖未达 100%，部分模型缺少价格。请设置模型价格以补齐成本。")
                        .accessibilityLabel("计价未完成，设置模型价格")
                }
                Spacer(); Image(systemName: icon).font(AppFont.secondary).foregroundStyle(accent) }
            Text(value).contentTransition(.numericText()).font(.system(size: 24, weight: .semibold, design: .default)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.9)
            if !detail.isEmpty { Text(detail).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1) }
        }.padding(14).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .modifier(NeutralCard())
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.3), value: value)
    }
}
struct EmptyCard: View {
    var icon: String, title: String, subtitle: String, action: () -> Void
    var body: some View {
        VStack(spacing: 12) { Image(systemName: icon).font(.system(size: 32, weight: .light)).foregroundStyle(Palette.accent); if !subtitle.isEmpty { Text(subtitle).font(AppFont.body).foregroundStyle(.secondary) }; Button(title, action: action).buttonStyle(.borderedProminent) }
            .frame(maxWidth: .infinity).padding(.vertical, 36)
    }
}

struct UsageChart: View {
    var days: [Aggregate], rows: [DayModel], cost: Bool
    private var models: [String] { Array(Set(rows.map(\.model))).sorted() }
    var body: some View {
        Chart(rows) { row in
            BarMark(x: .value("日期", row.day), y: .value(cost ? "USD" : "Token", cost ? row.usage.cost : row.usage.total))
                .foregroundStyle(by: .value("模型", row.model))
        }
        .chartForegroundStyleScale(domain: models, range: models.map { Palette.model($0) })
        .chartXScale(domain: days.map(\.key))
        .chartLegend(.hidden)
        .chartXAxis { AxisMarks(values: days.enumerated().filter { $0.offset % max(1, days.count / 7) == 0 }.map { $0.element.key }) { value in
            AxisValueLabel { if let label = value.as(String.self) { Text(String(label.suffix(5))).font(AppFont.secondary) } }
        } }
        .chartYAxis { AxisMarks(position: .leading, values: .automatic(desiredCount: 3)) { value in AxisGridLine().foregroundStyle(.primary.opacity(0.06)); AxisValueLabel { if let n = value.as(Double.self) { Text(cost ? Format.money(n) : Format.compact(n)).font(AppFont.secondary) } } } }
        .accessibilityLabel("按模型分组的每日\(cost ? "成本" : "Token")趋势；下方可展开逐日数据")
    }
}

struct ModelKey: View {
    var rows: [DayModel]
    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 14) { entries }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), alignment: .leading)], alignment: .leading, spacing: 8) { entries }
        }
    }
    private var entries: some View {
        ForEach(Array(Set(rows.map(\.model))).sorted(), id: \.self) { name in
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Circle().fill(Palette.model(name)).frame(width: 7, height: 7)
                Text(name).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }.help(name)
        }
    }
}

struct DailyUsage: View {
    var days: [Aggregate], rows: [DayModel], compact: Bool
    var cost = false
    private func value(_ aggregate: Aggregate) -> String { cost ? (aggregate.pricedTokens > 0 ? Format.money(aggregate.cost) + " USD" : "— USD") : Format.compact(aggregate.total) + " Token" }
    @ViewBuilder var body: some View {
        if compact {
            DisclosureGroup("每日明细") { dailyRows.padding(.top, 8) }.font(AppFont.secondary)
        } else { dailyRows }
    }
    private var dailyRows: some View {
        VStack(spacing: 10) {
            ForEach(days.reversed()) { day in
                DisclosureGroup {
                    ForEach(rows.filter { $0.day == day.key }.sorted { $0.usage.total > $1.usage.total }) { row in
                        HStack(spacing: 7) {
                            Circle().fill(Palette.model(row.model)).frame(width: 6, height: 6)
                            Text(row.model).lineLimit(1).truncationMode(.middle); Spacer()
                            Text(value(row.usage)).monospacedDigit()
                            if !compact && !cost { Text("缓存 " + Format.percent(row.usage.tokens.cacheRate.map { $0 * 100 })).foregroundStyle(.secondary).frame(width: 100, alignment: .trailing) }
                        }.font(AppFont.secondary).padding(.vertical, 3)
                    }
                    if day.total == 0 { Text("当日暂无记录").font(AppFont.secondary).foregroundStyle(.secondary) }
                } label: {
                    HStack { Text(String(day.key.suffix(5))); Spacer(); Text(value(day)).monospacedDigit(); if !cost { Text("缓存 " + Format.percent(day.tokens.cacheRate.map { $0 * 100 })).foregroundStyle(.secondary).frame(width: 100, alignment: .trailing) } }.font(AppFont.secondary)
                }
            }
        }
    }
}

struct ModelChart: View {
    var models: [Aggregate], cost: Bool
    var body: some View {
        HStack(spacing: 18) {
            Chart(models) { model in
                SectorMark(angle: .value("用量", cost ? model.cost : model.total), innerRadius: .ratio(0.72), angularInset: 2)
                    .foregroundStyle(Palette.model(model.key)).cornerRadius(3)
            }.frame(width: 132, height: 150).chartBackground { _ in
                VStack(spacing: 3) { Text("\(models.count)").font(.system(size: 25, weight: .semibold, design: .default)); Text("模型").font(AppFont.secondary).foregroundStyle(.secondary) }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) { ForEach(models) { item in
                    HStack(alignment: .top, spacing: 6) {
                        Circle().fill(Palette.model(item.key)).frame(width: 6, height: 6).padding(.top, 4)
                        VStack(alignment: .leading, spacing: 4) { Text(item.key).font(.system(size: 14, weight: .medium)).lineLimit(2); Text(cost ? Format.money(item.cost) : Format.compact(item.total)).font(AppFont.secondary).foregroundStyle(.secondary) }
                    }
                } }.frame(maxWidth: .infinity, alignment: .leading).background(OverlayScrollStyle())
            }
        }
    }
}

struct Heatmap: View {
    var days: [Aggregate], cost: Bool
    private var leading: Int {
        guard let first = days.first, let date = ISO8601DateFormatter().date(from: first.key + "T12:00:00Z") else { return 0 }
        return (Calendar.current.component(.weekday, from: date) + 5) % 7
    }
    private var maxValue: Double { max(1, days.map { cost ? $0.cost : $0.total }.max() ?? 1) }
    var body: some View {
        VStack(spacing: 12) {
            GeometryReader { proxy in
                let columns = (days.count + leading + 6) / 7
                let size = min(13.0, (proxy.size.width - 22) / CGFloat(max(1, columns)) - 3)
                HStack(alignment: .top, spacing: 3) {
                    VStack(spacing: 3) { ForEach(0..<7) { row in Text(["一", "", "三", "", "五", "", "日"][row]).font(AppFont.secondary).foregroundStyle(.secondary).frame(width: 16, height: size) } }
                    ForEach(0..<columns, id: \.self) { col in
                        VStack(spacing: 3) {
                            ForEach(0..<7) { row in
                                let index = col * 7 + row - leading
                                if index >= 0 && index < days.count {
                                    let day = days[index], value = cost ? day.cost : day.total
                                    RoundedRectangle(cornerRadius: 2)
                                        .fill(value == 0 ? Color.primary.opacity(0.055) : Palette.accent.opacity(0.2 + 0.8 * sqrt(value / maxValue)))
                                        .frame(width: size, height: size).help("\(day.key) · \(cost ? Format.money(value) : Format.compact(value) + " Token")")
                                        .accessibilityLabel("\(day.key) \(Format.compact(value))")
                                } else { Color.clear.frame(width: size, height: size) }
                            }
                        }
                    }
                }
            }.frame(height: 112)
            HStack { Text(days.first?.key ?? ""); Spacer(); Text("少"); ForEach(0..<5) { n in RoundedRectangle(cornerRadius: 2).fill(Palette.accent.opacity(Double(n + 1) / 5)).frame(width: 10, height: 10) }; Text("多") }
                .font(AppFont.secondary).foregroundStyle(.secondary)
        }
    }
}

struct QuotaCard: View {
    var quota: Quota
    var compact = false
    var sourceName = ""
    var estimate: QuotaEstimate?
    var onEstimate: (() -> Void)?
    var onCredits: (() -> Void)?
    var creditEstimate: QuotaEstimate?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @AppStorage private var expanded: Bool
    init(quota: Quota, compact: Bool = false, sourceName: String = "", estimate: QuotaEstimate? = nil, onEstimate: (() -> Void)? = nil, onCredits: (() -> Void)? = nil, creditEstimate: QuotaEstimate? = nil) {
        self.quota = quota; self.compact = compact; self.sourceName = sourceName; self.estimate = estimate; self.onEstimate = onEstimate; self.onCredits = onCredits; self.creditEstimate = creditEstimate
        _expanded = AppStorage(wrappedValue: !compact && quota.provider != "agy", "quota.expanded." + quota.id)
    }
    var body: some View {
        fullCard
    }
    @ViewBuilder private var balanceContent: some View {
        if let balances = quota.balances { ForEach(balances) { balance in
            VStack(alignment: .leading, spacing: 8) {
                HStack { Text("可用余额").font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Text(balance.currency + " " + balance.total).font(AppFont.section).monospacedDigit() }
                if expanded { HStack { Text("赠送 " + balance.granted); Spacer(); Text("充值 " + balance.toppedUp) }.font(AppFont.secondary).foregroundStyle(.secondary) }
            }
        } }
        if quota.isAvailable == false { Text("余额不足").font(AppFont.secondary).foregroundStyle(Palette.warn) }
    }
    private var agyGroups: [String] {
        quota.windows.reduce(into: [String]()) { result, w in
            let group = w.groupLabel
            if !result.contains(group) { result.append(group) }
        }
    }
    private var agyWindows: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(agyGroups, id: \.self) { group in
                VStack(alignment: .leading, spacing: 7) {
                    Text(group).font(.system(size: 14, weight: .medium)).fixedSize(horizontal: false, vertical: true)
                    HStack(alignment: .top, spacing: 14) {
                        ForEach(quota.windows.filter { $0.groupLabel == group }.sorted { ($0.windowMinutes ?? 0) < ($1.windowMinutes ?? 0) }) { window in
                            VStack(alignment: .leading, spacing: 5) {
                                HStack { Text(window.windowMinutes == 10080 ? "7d" : window.windowMinutes == 300 ? "5h" : window.name); Spacer(minLength: 2); Text(Format.percent(max(0, 100-window.usedPercent))).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent)) }.font(AppFont.secondary)
                                ResourceBar(percent: 100-window.usedPercent, tint: Palette.quota(window.usedPercent))
                                if expanded { Text(window.resetsAt.map { "重置 " + Format.date($0) } ?? "重置时间未知").font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
                            }.frame(maxWidth: .infinity).accessibilityElement(children: .combine).accessibilityLabel(window.name + "，剩余 " + Format.percent(max(0, 100-window.usedPercent)))
                        }
                    }
                }
            }
        }
    }
    private var fullCard: some View {
        Surface {
            Button { withAnimation(reduceMotion ? nil : .spring(response: 0.32, dampingFraction: 0.86)) { expanded.toggle() } } label: {
            HStack {
                VStack(alignment: .leading, spacing: 3) { Text(quota.name).font(.system(size: 14, weight: .semibold)); if expanded { Text(quota.plan.map { "\(Format.provider(quota.provider)) · \($0.capitalized)" } ?? Format.provider(quota.provider)).font(AppFont.secondary).foregroundStyle(.secondary) } }
                Spacer()
                if quota.provider == "agy" { Text("剩余额度").font(AppFont.secondary).foregroundStyle(.secondary) }
                if expanded && quota.updatedAt > 0 { Text("\(quota.origin == "log" ? "记录" : "更新") \(Format.date(quota.updatedAt))").font(AppFont.secondary).foregroundStyle(.secondary) }
                Image(systemName: expanded ? "chevron.up" : "chevron.down").font(.system(size: 14, weight: .semibold)).foregroundStyle(.secondary)
            }.contentShape(Rectangle())
            }.buttonStyle(.plain).help(expanded ? "折叠限额" : "展开限额")
                .accessibilityLabel(quota.name + (expanded ? "，折叠限额" : "，展开限额"))
            balanceContent
            if quota.provider == "agy" { agyWindows }
            else if !quota.windows.isEmpty { LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .leading), count: expanded ? 1 : 2), alignment: .leading, spacing: expanded ? 14 : 8) {
            ForEach(quota.windows) { window in
                VStack(spacing: 6) {
                    ViewThatFits(in: .horizontal) {
                        HStack { Text(window.name); Spacer(minLength: 6); Text("剩余 " + Format.percent(max(0, 100 - window.usedPercent))).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent)) }
                        VStack(alignment: .leading, spacing: 3) { Text(window.name); Text("剩余 " + Format.percent(max(0, 100 - window.usedPercent))).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent)) }.frame(maxWidth: .infinity, alignment: .leading)
                    }.font(AppFont.secondary)
                    ResourceBar(percent: 100 - window.usedPercent, tint: Palette.quota(window.usedPercent))
                    if expanded, let reset = window.resetsAt {
                        TimelineView(.periodic(from: .now, by: 60)) { context in
                            HStack {
                                Text("重置 \(Format.date(reset))")
                                Spacer()
                                if reset <= context.date.timeIntervalSince1970 { Text("等待同步") }
                            }.font(AppFont.secondary).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            }
            }
            if let onEstimate, (!compact || expanded || estimate != nil), (quota.provider != "agy" && quota.windows.contains(where: { $0.windowMinutes == 10080 })) || estimate != nil {
                Button(action: onEstimate) {
                    HStack { Label(estimate == nil ? "估算整周价值" : "7d 整周估值", systemImage: "chart.line.uptrend.xyaxis"); Spacer();
                        if let value = estimate?.weeklyValue { Text(Format.money(value) + " USD").monospacedDigit(); if estimate?.status == "pending" { Text("待确认").foregroundStyle(Palette.warn) } }
                        else if let estimate { Text(estimate.statusLabel).foregroundStyle(.secondary) }
                        Image(systemName: "chevron.right").font(AppFont.secondary)
                    }.font(AppFont.secondary).contentShape(Rectangle())
                }.buttonStyle(.plain).foregroundStyle(Palette.accent)
            }
            if quota.provider == "agy" && expanded { Text("整周估值需匹配的 Token 用量与模型组归属。").font(AppFont.secondary).foregroundStyle(.secondary) }
            if expanded && !sourceName.isEmpty { Text(sourceName).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1) }
            if quota.provider == "codex" {
                HStack { Text("Codex credits"); Spacer(); Text(quota.credits?.unlimited == true ? "无限" : quota.credits?.balance.map { Format.credits($0) } ?? (quota.credits?.hasCredits == true ? "有余额 · 数量未知" : "—")).monospacedDigit() }.font(AppFont.secondary)
                if expanded, let stamp = quota.creditsUpdatedAt { Text("更新于 \(Format.date(stamp))").font(AppFont.secondary).foregroundStyle(.secondary) }
                if let onCredits, !compact || expanded || creditEstimate != nil { Button("估算 credit 价值", action: onCredits).buttonStyle(.plain).foregroundStyle(Palette.accent).font(AppFont.secondary) }
            }
            if let creditEstimate {
                VStack(alignment: .leading, spacing: 4) {
                    Text(Format.creditValue(creditEstimate)).monospacedDigit()
                    Text(creditEstimate.statusLabel + " · API 等价价值").foregroundStyle(.secondary)
                }.font(AppFont.secondary)
            }
            if expanded, let bank = quota.bankReset {
                Divider().opacity(0.5)
                DisclosureGroup {
                    if let credits = bank.credits { ForEach(credits) { credit in
                        HStack { VStack(alignment: .leading, spacing: 3) { Text(credit.title ?? "Reset Credit").font(AppFont.secondary); Text(["available":"可用", "redeeming":"处理中", "redeemed":"已使用", "unknown":"未知状态"][credit.status] ?? credit.status).font(AppFont.secondary).foregroundStyle(.secondary) }; Spacer(); Text(credit.expiresAt.map { "到期 \(Format.date($0))" } ?? "无到期时间").font(AppFont.secondary).foregroundStyle(.secondary) }
                    } }
                    if let updated = quota.bankUpdatedAt { Text("\(Format.date(updated))").font(AppFont.secondary).foregroundStyle(.secondary) }
                } label: {
                    HStack { Image(systemName: "rectangle.stack").foregroundStyle(Palette.accent); Text("Bank Reset"); Spacer(); Text("\(bank.availableCount) 次可用").foregroundStyle(Palette.accent) }.font(.system(size: 14, weight: .medium))
                }
            }
            if let error = quota.error {
                Text(expanded ? error : "限额更新失败 · 展开查看").font(AppFont.secondary).foregroundStyle(Palette.warn).help(error)
            }
            if quota.windows.isEmpty && (quota.balances ?? []).isEmpty && quota.error == nil && quota.credits == nil { Text("暂无限额数据").font(AppFont.secondary).foregroundStyle(.secondary) }
        }
    }
}

struct ServerCard: View {
    var host: Host, result: HostResult?, compact: Bool
    var onRefresh: () -> Void = {}
    @AppStorage private var expanded: Bool
    init(host: Host, result: HostResult?, compact: Bool, onRefresh: @escaping () -> Void = {}) {
        self.host = host; self.result = result; self.compact = compact; self.onRefresh = onRefresh
        _expanded = AppStorage(wrappedValue: true, "server.expanded." + host.id)
    }
    var body: some View {
        Surface {
            DisclosureGroup(isExpanded: $expanded) {
                if let sample = result?.sample {
                    VStack(alignment: .leading, spacing: 16) {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 130), spacing: 14)], spacing: 14) {
                            if sample.cpu != nil { ResourceRing(title: "CPU", percent: sample.cpu?.first(where: { $0.id == "cpu" })?.utilization) }
                            if let memory = sample.memory { ResourceRing(title: "内存", percent: memory.total > 0 ? (memory.total - memory.available) / memory.total * 100 : nil) }
                            ForEach(sample.gpu ?? []) { gpu in ResourceRing(title: "GPU " + gpu.id, percent: gpu.utilization) }
                        }

                        if let cpus = sample.cpu {
                            DisclosureGroup {
                                ForEach(cpus.filter { $0.id != "cpu" }) { cpu in
                                    VStack(spacing: 4) {
                                        metricRow(cpu.id, Format.percent(cpu.utilization))
                                        ResourceBar(percent: cpu.utilization)
                                    }.padding(.vertical, 3)
                                }
                                if host.shows("cpuTimes"), let cpu = cpus.first(where: { $0.id == "cpu" }) { metricRow("user / system", "\(Format.percent(cpu.userPercent)) / \(Format.percent(cpu.systemPercent))"); metricRow("iowait / steal", "\(Format.percent(cpu.iowaitPercent)) / \(Format.percent(cpu.stealPercent))") }
                            } label: { MetricHeading(icon: "cpu", title: "CPU", value: Format.percent(cpus.first(where: { $0.id == "cpu" })?.utilization), percent: cpus.first(where: { $0.id == "cpu" })?.utilization) }
                        }
                        if let m = sample.memory {
                            DisclosureGroup {
                                metricRow("可用", Format.bytes(m.available)); if host.shows("memoryCache") { metricRow("缓存 / Buffer", "\(Format.bytes(m.cached)) / \(Format.bytes(m.buffers))") }
                                if host.shows("swap") { metricRow("Swap", "\(Format.bytes(m.swapTotal - m.swapFree)) / \(Format.bytes(m.swapTotal))"); ResourceBar(percent: m.swapTotal > 0 ? (m.swapTotal - m.swapFree) / m.swapTotal * 100 : nil) }
                            } label: { MetricHeading(icon: "memorychip", title: "内存", value: "\(Format.bytes(m.total - m.available)) / \(Format.bytes(m.total))", percent: m.total > 0 ? (m.total - m.available) / m.total * 100 : nil) }
                        }
                        if let gpus = sample.gpu {
                            ForEach(gpus) { gpu in
                                DisclosureGroup {
                                    metricRow(gpu.name ?? "GPU \(gpu.id)", "")
                                    if host.shows("gpuMemory") {
                                        metricRow("显存", "\(Format.bytes(gpu.memoryUsedMiB.map { $0 * 1048576 })) / \(Format.bytes(gpu.memoryTotalMiB.map { $0 * 1048576 }))")
                                        if let used = gpu.memoryUsedMiB, let total = gpu.memoryTotalMiB, total > 0 { ResourceBar(percent: used / total * 100) }
                                    }
                                    if host.shows("gpuThermals") { metricRow("温度 / 功耗", "\(gpu.temperature.map { String(format: "%.0f°C", $0) } ?? "—") / \(gpu.powerWatts.map { String(format: "%.1f W", $0) } ?? "—")") }
                                } label: { MetricHeading(icon: "rectangle.3.group", title: "GPU \(gpu.id)", value: Format.percent(gpu.utilization), percent: gpu.utilization) }
                            }
                        }
                        if let filesystems = sample.filesystems {
                            DisclosureGroup {
                                ForEach(filesystems) { fs in VStack(alignment: .leading, spacing: 4) {
                                    metricRow(fs.id, "\(Format.bytes(fs.used)) / \(Format.bytes(fs.total))")
                                    if let total = fs.total, let used = fs.used, total > 0 { ResourceBar(percent: used / total * 100) }
                                    if host.shows("fsAvailable") { metricRow("可用", Format.bytes(fs.available)) }
                                    if host.shows("fsType") { metricRow(fs.device ?? "", fs.type ?? "") }
                                    if host.shows("inodes"), let total = fs.inodes, let free = fs.inodesFree, total > 0 { metricRow("inode", Format.percent((total - free) / total * 100)) }
                                }.padding(.vertical, 4) }
                            } label: { MetricHeading(icon: "internaldrive", title: "文件系统", value: "\(filesystems.count) 个挂载点") }
                        }
                        if let disks = sample.disk {
                            DisclosureGroup {
                                ForEach(disks) { disk in VStack(spacing: 4) { metricRow(disk.id, "读 \(Format.speed(disk.readBytesPerSecond))"); metricRow("", "写 \(Format.speed(disk.writeBytesPerSecond))"); if host.shows("diskIops") { metricRow("IOPS 读 / 写", "\(disk.readIops.map(Format.compact) ?? "—") / \(disk.writeIops.map(Format.compact) ?? "—")") }; if host.shows("diskBusy") { metricRow("忙碌率", Format.percent(disk.busyMsPerSecond.map { min(100, max(0, $0 / 10)) })); ResourceBar(percent: disk.busyMsPerSecond.map { $0 / 10 }) } }.padding(.vertical, 3) }
                            } label: { MetricHeading(icon: "arrow.left.arrow.right", title: "磁盘 I/O", value: "\(disks.count) 个设备") }
                        }
                        if let networks = sample.network {
                            DisclosureGroup {
                                ForEach(networks) { net in VStack(spacing: 4) {
                                    metricRow(net.id, "↓ \(Format.speed(net.rxBytesPerSecond))")
                                    metricRow("", "↑ \(Format.speed(net.txBytesPerSecond))")
                                    if host.shows("networkTotals") { metricRow("累计接收 / 发送", "\(Format.bytes(net.rxBytes)) / \(Format.bytes(net.txBytes))") }
                                    if host.shows("networkErrors") { metricRow("错误 / 丢包", "\(Format.compact((net.rxErrors ?? 0) + (net.txErrors ?? 0))) / \(Format.compact((net.rxDrops ?? 0) + (net.txDrops ?? 0)))") }
                                }.padding(.vertical, 3) }
                            } label: { MetricHeading(icon: "network", title: "网络", value: "\(networks.count) 个网卡") }
                        }
                        ForEach(sample.errors.keys.sorted(), id: \.self) { key in metricRow(["gpu":"GPU", "cpu":"CPU", "memory":"内存", "filesystems":"文件系统", "disk":"磁盘 I/O", "network":"网络"][key] ?? key, "采集失败") }
                        HStack { Text("负载 " + sample.load.map { String(format: "%.2f", $0) }.joined(separator: " / ")); Spacer(); TimelineView(.periodic(from: .now, by: 1)) { context in Text("" + Date(timeIntervalSince1970: sample.timestamp).formatted(.dateTime.hour().minute().second()) + (context.date.timeIntervalSince1970 - sample.timestamp > 10 ? " · 数据已延迟" : "")) } }.font(AppFont.secondary).foregroundStyle(.secondary)
                    }.padding(.top, 14)
                } else if host.enabled { Text(result?.error == nil ? "等待首次采样" : "连接失败").font(AppFont.secondary).foregroundStyle(.secondary).padding(.top, 10) }
                if let error = result?.error { Text(error).font(AppFont.secondary).foregroundStyle(Palette.warn).padding(.top, 6) }
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: "server.rack").font(.system(size: 20)).foregroundStyle(Palette.accent)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(host.name.isEmpty ? host.target : host.name).font(AppFont.section).lineLimit(1).help(host.name.isEmpty ? host.target : host.name)
                        if expanded { Text(host.target).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1).help(host.target) }
                    }
                    Spacer()
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        let label = status(at: context.date)
                        HStack(spacing: 6) { Circle().fill(label == "正常" ? Palette.ok : label == "已暂停" ? Color.gray : label == "连接失败" ? Palette.danger : Palette.warn).frame(width: 6, height: 6); Text(label).font(AppFont.secondary).foregroundStyle(.secondary) }
                    }

                }
            }
            if !expanded, let sample = result?.sample {
                HStack(alignment: .top, spacing: 18) {
                    if sample.cpu != nil {
                        compactMetric("CPU", percent: sample.cpu?.first(where: { $0.id == "cpu" })?.utilization)
                    }
                    if let memory = sample.memory {
                        compactMetric("内存", percent: memory.total > 0 ? (memory.total - memory.available) / memory.total * 100 : nil)
                    }
                }
                if !sample.errors.isEmpty { Text("部分指标采集失败").font(AppFont.secondary).foregroundStyle(Palette.warn) }
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    if context.date.timeIntervalSince1970 - sample.timestamp > 10 { AttentionNotice(reason: "数据已延迟", action: "刷新服务器", perform: onRefresh) }
                }
            }
        }
    }
    private func status(at date: Date) -> String {
        if !host.enabled { return "已暂停" }
        if result?.error != nil { return "连接失败" }
        guard let sample = result?.sample else { return "等待采样" }
        if date.timeIntervalSince1970 - sample.timestamp > 10 { return "数据延迟" }
        return sample.errors.isEmpty ? "正常" : "部分采集失败"
    }
    private func compactMetric(_ title: String, percent: Double?) -> some View {
        VStack(spacing: 6) {
            HStack { Text(title).foregroundStyle(.secondary); Spacer(); Text(Format.percent(percent)).monospacedDigit() }.font(AppFont.secondary)
            ResourceBar(percent: percent)
        }.frame(maxWidth: .infinity)
    }
    private func metricRow(_ key: String, _ value: String) -> some View { HStack { Text(key).lineLimit(1).truncationMode(.middle); Spacer(minLength: 8); Text(value).monospacedDigit() }.font(AppFont.secondary).foregroundStyle(.secondary).padding(.vertical, 2) }
}
struct MetricHeading: View {
    var icon: String, title: String, value: String
    var percent: Double? = nil
    var body: some View {
        VStack(spacing: 7) {
            HStack(spacing: 8) { Image(systemName: icon).frame(width: 15).foregroundStyle(.secondary); Text(title); Spacer(); Text(value).monospacedDigit().foregroundStyle(.secondary) }.font(.system(size: 14, weight: .medium))
            if let percent { ResourceBar(percent: percent) }
        }
    }
}

struct ResourceRing: View {
    var title: String, percent: Double?
    private var value: Double? { percent.flatMap { $0.isFinite ? min(100, max(0, $0)) : nil } }
    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                Circle().stroke(.primary.opacity(0.07), lineWidth: 7)
                if let value, value > 0 { Circle().trim(from: 0, to: value / 100).stroke(value >= 90 ? Palette.danger : value >= 70 ? Palette.warn : Palette.accent, style: StrokeStyle(lineWidth: 7, lineCap: .round)).rotationEffect(.degrees(-90)) }
                Text(Format.percent(value)).font(.system(size: 14, weight: .semibold)).monospacedDigit()
            }.frame(width: 72, height: 72)
            Text(title).font(.system(size: 14, weight: .medium)).lineLimit(1)
        }.frame(maxWidth: .infinity, alignment: .leading).accessibilityElement(children: .ignore).accessibilityLabel(title + " " + Format.percent(value))
    }
}

struct ResourceBar: View {
    var percent: Double?
    var tint: Color? = nil
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(.primary.opacity(0.07))
                if let percent, percent.isFinite {
                    Capsule().fill(tint ?? (percent >= 90 ? .red : percent >= 70 ? .orange : Palette.accent))
                        .frame(width: proxy.size.width * min(1, max(0, percent / 100)))
                }
            }
        }.frame(height: 4)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.35), value: percent)
            .accessibilityLabel(percent.map { Format.percent($0) } ?? "暂无数据")
    }
}

// Apply native overlay scrollers only to the enclosing scroll view, without changing system preferences.
struct OverlayScrollStyle: NSViewRepresentable {
    final class Anchor: NSView {
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); configure() }
        func configure() {
            DispatchQueue.main.async { [weak self] in
                guard let scroll = self?.enclosingScrollView else { return }
                scroll.scrollerStyle = .overlay
                scroll.autohidesScrollers = true
                scroll.drawsBackground = false
            }
        }
    }
    func makeNSView(context: Context) -> Anchor { Anchor() }
    func updateNSView(_ view: Anchor, context: Context) { view.configure() }
}
