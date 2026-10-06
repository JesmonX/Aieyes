import SwiftUI
import Charts

struct Surface<Content: View>: View {
    var title: String?
    var spacing: CGFloat = 14
    var padding: CGFloat = 16
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: spacing) {
            if let title { Text(title).font(.system(size: 15, weight: .semibold)) }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading).padding(padding)
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
        .overlay(RoundedRectangle(cornerRadius: Palette.radiusCard).strokeBorder(Palette.cardBorder.opacity(previewAccessible || contrast == .increased ? 1 : 0.22), lineWidth: previewAccessible || contrast == .increased ? 2 : 1))
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
    @ObservedObject private var updater = AppUpdater.shared
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var compact = true
    @State private var tab = "agent"
    @State private var costMode = false
    @State private var orderingQuotas = false
    @State private var showingAccounts = false
    @State private var serverFilter = "all"
    @AppStorage("quota.pinned") private var pinnedAccount = ""
    private var usageDashboard: Dashboard { compact ? model.panelDashboard : model.dashboard }
    init(model: AppModel, compact: Bool = true, page: String = "agent") {
        self.model = model; self.compact = compact; _tab = State(initialValue: page)
    }
    var body: some View {
        VStack(spacing: 0) {
            if compact { header }
            Divider().opacity(0.55)
            ScrollView {
                VStack(alignment: .leading, spacing: compact ? 8 : tab == "servers" ? 12 : 22) {
                    if !model.actionFailures.isEmpty { DisclosureGroup("\(model.actionFailures.count) 项操作需要处理") { ActionFailureList(model: model) }.font(AppFont.secondary) }
                    if let message = model.message, !model.actionFailures.contains(where: { $0.reason == message }) {
                        HStack(spacing: 8) {
                            Text(message).font(AppFont.secondary).textSelection(.enabled)
                            Spacer(); Button { model.message = nil } label: { Image(systemName: "xmark") }.buttonStyle(.plain)
                        }.padding(10).background(.quaternary, in: RoundedRectangle(cornerRadius: 9))
                    }
                    if tab == "agent" { agentContent } else { serverContent }
                }.padding(compact ? 12 : 24).background(OverlayScrollStyle())
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
        .sheet(isPresented: $showingAccounts) { allAccounts }
        .onChange(of: model.provider) { _, _ in model.selectedModel = "all"; model.selectedAccount = "all"; model.selectedSource = "all"; Task { await model.reload() } }
        .onChange(of: model.selectedAccount) { _, _ in model.selectedSource = "all"; Task { await model.reload() } }
        .onChange(of: model.selectedSource) { _, _ in Task { await model.reload() } }
        .onChange(of: model.selectedModel) { _, _ in Task { await model.reload() } }
        .onChange(of: model.range) { _, _ in Task { await model.reload() } }
        .onChange(of: tab) { _, value in if !compact { model.detailPage = value }; model.setServerVisible(value == "servers", window: compact ? "panel" : "detail"); if value == "servers" { Task { await model.sampleHosts() } } }
        .onChange(of: model.requestedDetailPage) { _, page in if !compact, let page { tab = page; model.requestedDetailPage = nil } }
        .onAppear { if !compact, let page = model.requestedDetailPage { tab = page; model.requestedDetailPage = nil }; model.setServerVisible(tab == "servers", window: compact ? "panel" : "detail") }
    }
    private var allAccounts: some View {
        ScrollView {
            VStack(spacing: 14) {
                HStack { Text("实时账户限额").font(AppFont.section); Spacer(); Button("关闭") { showingAccounts = false }.keyboardShortcut(.cancelAction) }
                ForEach(model.dashboard.quotas) { quota in
                    QuotaCard(quota: quota, compact: true, estimate: model.dashboard.quotaEstimates?.first { $0.accountKey == quota.id }, onEstimate: { model.showEstimate?(quota, false) }, onCredits: { model.showEstimate?(quota, true) }, creditEstimate: model.dashboard.creditEstimates?.first { $0.accountKey == quota.id })
                    HStack { Button("置顶 " + quota.name) { pinnedAccount = quota.id; showingAccounts = false }; Button("编辑账户") { editAccount(quota.id) } }
                }
                ForEach(displayedQuotaAccounts.filter { account in !model.dashboard.quotas.contains { $0.id == account.key } }, id: \.key) { account in
                    Surface(title: account.name) { Text(model.quotaBusy ? "正在读取账户限额…" : model.quotaError == nil ? "等待首次限额查询" : "暂时无法读取限额，可重试").foregroundStyle(.secondary); Button("编辑账户") { editAccount(account.key) } }
                }
                Button("刷新限额") { Task { await model.refreshQuotas() } }.disabled(model.quotaBusy)
            }.padding(20)
        }.frame(width: 460, height: 520)
    }
    private func editAccount(_ key: String) { showingAccounts = false; model.requestedAccountKey = key; model.settingsTab = "sources"; model.showSettings?() }
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
        }.padding(.horizontal, compact ? 18 : 28).padding(.vertical, compact ? 8 : 16)
    }
    private var footer: some View {
        HStack(spacing: 8) {
            Button { Task { if tab == "servers" { await model.sampleHosts() } else { await model.scan() } } } label: {
                HStack(spacing: 6) {
                    if model.busy || model.quotaBusy || model.serverBusy { ProgressView().controlSize(.mini).accessibilityLabel("进行中") }
                    else { Circle().fill(!model.actionFailures.isEmpty ? Palette.danger : (tab == "servers" ? model.serverStatusText.contains("延迟") : model.statusText.contains("延迟") || model.dataTime == 0) ? Palette.warn : Palette.ok).frame(width: 7, height: 7) }
                    Text(tab == "servers" ? model.serverStatusText : model.statusText).font(AppFont.secondary).foregroundStyle(.secondary)
                }
            }.help(tab == "servers" ? "服务器采样时间；点击刷新服务器" : "记录最后成功同步时间；点击同步记录")
            Spacer()
            Button { Task { await model.testNetwork() } } label: { Text(model.networkBusy ? "出站测试中…" : "出站 · " + (model.networkTest?.label ?? "连接未测试")).font(.system(size: 12)).lineLimit(1) }.help("应用出站连接测试，不代表 SSH 或所有账户可用。\n" + (model.networkTest?.detail ?? "测试当前应用连接方式")).disabled(model.networkBusy)
            if compact {
                Button { updater.check() } label: {
                    Image(systemName: "arrow.down.circle").frame(width: 30, height: 30).overlay(alignment: .topTrailing) { if updater.hasUpdate { Text("!").font(.system(size: 11, weight: .bold)).foregroundStyle(.orange) } }
                }.help(updater.hasUpdate ? "发现新版本 · 查看更新" : "检查更新").accessibilityLabel(updater.hasUpdate ? "发现新版本，查看更新" : "检查更新").disabled(updater.phase == "checking")
                Button { model.showDetailPage?(tab) } label: { Image(systemName: "arrow.up.left.and.arrow.down.right").frame(width: 30, height: 30).contentShape(Rectangle()) }.help("打开详情").accessibilityLabel("打开详情")
            }
            Button { model.showSettings?() } label: { Image(systemName: "gearshape").frame(width: 30, height: 30).contentShape(Rectangle()) }.help("设置").accessibilityLabel("设置").keyboardShortcut(",")
            Menu { if compact { Button(model.isPinned ? "取消固定面板" : "固定面板") { model.isPinned.toggle() }; Divider() }; Text("关闭窗口后继续在菜单栏运行"); Divider(); Button("退出 Aieyes") { NSApplication.shared.terminate(nil) }.keyboardShortcut("q") } label: { Image(systemName: "ellipsis").frame(width: 30, height: 30).contentShape(Rectangle()) }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().accessibilityLabel("更多操作").help("关闭窗口后继续后台；在此退出应用")
        }.buttonStyle(.plain).padding(.horizontal, compact ? 18 : 28).padding(.vertical, 12)
    }
    @ViewBuilder private var agentContent: some View {
        sessionStrip
        if model.settings.sources.isEmpty { connectionGuide }
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

        if compact {
            if let quota = model.dashboard.quotas.first(where: { $0.id == (model.selectedAccount == "all" ? pinnedAccount : model.selectedAccount) }) ?? model.dashboard.quotas.first {
                PanelQuotaSummary(quota: quota) { showingAccounts = true }
            }
            if !displayedQuotaAccounts.isEmpty || !model.dashboard.quotas.isEmpty {
                Button("查看全部 \(max(displayedQuotaAccounts.count, model.dashboard.quotas.count)) 个账户") { showingAccounts = true }.buttonStyle(.plain).foregroundStyle(Palette.accent).font(.system(size: 13))
            }
        }
        if model.dashboardPending || model.dashboardError != nil {
            VStack(alignment: .leading, spacing: 4) {
                Text((model.dashboardPending ? "正在更新，仍显示上一结果：" : "更新失败，仍显示上一结果：") + model.appliedScope).font(AppFont.secondary)
                if let error = model.dashboardError { Text(error).font(AppFont.secondary); Button("重试查询") { Task { await model.reload() } } }
            }.foregroundStyle(.secondary)
        }
        HStack(alignment: .firstTextBaseline) {
            Text(compact || model.range == 1 ? "今日概览" : "用量分析").font(.system(size: compact ? 17 : 22, weight: .semibold))
            Spacer()
            if !compact { Picker("时间范围", selection: $model.range) { Text("今日").tag(1); Text("最近 7 天").tag(7); Text("最近 30 天").tag(30); Text("最近 90 天").tag(90); Text("最近一年").tag(365) }.labelsHidden().fixedSize() }
        }
        if hasUsageData {
            HStack(spacing: 10) {
                StatCard(title: "总 Token", value: Format.compact(usageDashboard.summary.total), detail: "", icon: "sparkle", accent: Palette.accent)
                StatCard(title: "API 等价成本", value: usageDashboard.summary.total == 0 || usageDashboard.summary.pricedTokens > 0 ? Format.money(usageDashboard.summary.cost) : "—", detail: "", icon: "dollarsign.circle", accent: Palette.accent, pricingIncomplete: usageDashboard.summary.total > usageDashboard.summary.pricedTokens, onPricing: { model.openPricing() })
            }
            if compact {
                HStack { Text("缓存 Token"); Text(Format.compact(usageDashboard.summary.tokens.cacheRead + usageDashboard.summary.tokens.cacheWrite)).monospacedDigit(); Spacer(); Text("读取命中率"); Text(Format.percent(usageDashboard.summary.tokens.cacheRate.map { $0 * 100 })).monospacedDigit() }.font(.system(size: 13)).padding(8).modifier(NeutralCard()).help("缓存 Token = 读取 + 写入；读取命中率 = 缓存读取 ÷ 全部输入")
            } else { CacheSummaryCard(tokens: usageDashboard.summary.tokens) }
        } else if !model.settings.sources.isEmpty { usageEmptyState }
        if !model.runningEstimates.isEmpty {
            Button { model.showSampling?() } label: { HStack { Text(model.samplingSummary); Spacer(); Text("管理采样"); Image(systemName: "chevron.right") }.font(AppFont.secondary) }.buttonStyle(.plain).foregroundStyle(model.samplingNeedsAttention ? Palette.warn : Palette.accent)
        }
        if hasUsageData {
            if compact {
                DisclosureGroup("近 7 天趋势与每日明细") { UsageChart(days: recentDays, rows: recentRows, cost: false).frame(minHeight: 220); DailyUsage(days: recentDays, rows: recentRows, compact: true) }.font(AppFont.secondary)
                Button("用量详情") { model.showDetailPage?("agent") }.buttonStyle(.plain).foregroundStyle(Palette.accent)
            } else {
                Text("日期、来源、模型筛选作用于用量分析；今日视图的趋势仍为近 7 天，热力图为过去 365 天。").font(AppFont.secondary).foregroundStyle(.secondary)
                HStack { Text(model.range == 1 ? "近 7 天趋势" : "使用趋势").font(AppFont.section); Spacer(); Picker("统计指标", selection: $costMode) { Text("Token").tag(false); Text("API 等价成本").tag(true) }.pickerStyle(.segmented).frame(width: 230) }
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .top, spacing: 18) { trendSurface.frame(minWidth: 450); modelSurface.frame(minWidth: 350) }
                    VStack(spacing: 18) { trendSurface; modelSurface }
                }
                quotaSection
                Surface(title: "每日明细 · 可选择与复制精确数值") { DailyUsage(days: model.dashboard.trendDays, rows: model.dashboard.dayModels, compact: false, cost: costMode) }
                Surface(title: "过去 365 天") { Heatmap(days: model.dashboard.heatmap, cost: costMode) }
            }
        } else if !compact { quotaSection }
    }
    private var trendSurface: some View { Surface(title: "每日用量 · 按模型") { UsageChart(days: model.dashboard.trendDays, rows: model.dashboard.dayModels, cost: costMode).frame(minHeight: 230) } }
    private var modelSurface: some View { Surface(title: "所选范围 · 模型分布") { ModelChart(models: model.dashboard.models, cost: costMode).frame(minHeight: 230) } }
    private var usageEmptyState: some View {
        let onlyQuota = model.settings.sources.allSatisfy { ["agy", "deepseek"].contains($0.provider) }
        let failed = model.dashboard.sources.contains { $0.enabled && $0.status?.error != nil }
        return Surface(title: onlyQuota ? "此来源提供账户限额" : failed ? "记录同步失败" : hasActiveFilters ? "没有匹配的用量记录" : model.dataTime == 0 ? "尚未同步用量记录" : "此时间范围暂无用量") {
            Text(onlyQuota ? "查看实时余额与限额；接入日志后即可分析用量。" : failed ? "请检查对应来源的错误后重试。" : "保留筛选与日期入口，可同步记录或扩大范围。").font(AppFont.secondary).foregroundStyle(.secondary)
            HStack { if onlyQuota { Button("添加日志来源") { model.requestedSourceProvider = "codex"; model.showSettings?() } } else { Button("同步记录") { Task { await model.scan() } }; Button("扩大到最近一年") { model.range = 365; if compact { model.showDetailPage?("agent") } }; if hasActiveFilters { Button("清除筛选") { model.provider = "all"; model.selectedAccount = "all"; model.selectedSource = "all"; model.selectedModel = "all" } } } }
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
            if compact { DisclosureGroup("已筛选 \(filterCount) 项") { filterChips } } else { filterChips }
        }
    }
    private var filterCount: Int { [model.provider != "all", model.selectedAccount != "all", model.range != 1, model.selectedSource != "all", model.selectedModel != "all"].filter { $0 }.count }
    private var filterChips: some View {
            FlowLayout {
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
    private func filterChip(_ title: String, clear: @escaping () -> Void) -> some View {
        HStack(spacing: 8) {
            Text(title).font(AppFont.secondary).lineLimit(2).help(title)
            Button(action: clear) { Image(systemName: "xmark.circle.fill").frame(width: 28, height: 28) }.buttonStyle(.plain).accessibilityLabel("清除" + title)
        }.padding(.leading, 10).background(Palette.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
    }
    private var recentDays: [Aggregate] { Array(usageDashboard.trendDays.suffix(compact ? 7 : max(7, model.range))) }
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
        return usageDashboard.dayModels.filter { dates.contains($0.day) }
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
        .padding(compact ? 4 : 14).modifier(NeutralCard())
    }
    private var displayedQuotaAccounts: [AgentAccount] {
        model.quotaAccounts.filter { account in
            (model.provider == "all" || model.provider == account.provider) && (model.selectedAccount == "all" || model.selectedAccount == account.key)
        }
    }
    private var quotaSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("实时账户限额").font(.system(size: compact ? 20 : 22, weight: .semibold)); if !displayedQuotaAccounts.isEmpty { Text("\(displayedQuotaAccounts.count)").font(AppFont.secondary).foregroundStyle(.secondary) }; Spacer(); Button { orderingQuotas = true } label: { Image(systemName: "arrow.up.arrow.down") }.help("调整账户顺序").accessibilityLabel("调整账户顺序"); Button(model.quotaBusy ? "读取中…" : "刷新限额") { Task { await model.refreshQuotas() } }.font(AppFont.secondary).disabled(model.quotaBusy) }
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
            if compact { ForEach(model.dashboard.quotas) { quota in QuotaCard(quota: quota, compact: true, estimate: model.dashboard.quotaEstimates?.first { $0.accountKey == quota.id }, onEstimate: { model.showEstimate?(quota, false) }, onCredits: { model.showEstimate?(quota, true) }, creditEstimate: model.dashboard.creditEstimates?.first { $0.accountKey == quota.id }) } }
            else { LazyVGrid(columns: [GridItem(.adaptive(minimum: 340), spacing: 16, alignment: .top)], alignment: .leading, spacing: 16) { ForEach(model.dashboard.quotas) { quota in QuotaCard(quota: quota, estimate: model.dashboard.quotaEstimates?.first { $0.accountKey == quota.id }, onEstimate: { model.showEstimate?(quota, false) }, onCredits: { model.showEstimate?(quota, true) }, creditEstimate: model.dashboard.creditEstimates?.first { $0.accountKey == quota.id }) } } }
        }.animation(reduceMotion ? nil : .spring(response: 0.35, dampingFraction: 0.86), value: model.dashboard.quotaOrder)
    }
    private var filteredHosts: [Host] {
        model.settings.hosts.filter { host in
            if serverFilter == "all" { return true }
            if serverFilter == "paused" { return !host.enabled }
            guard host.enabled else { return false }
            guard let result = model.hosts.first(where: { $0.id == host.id }), let sample = result.sample else { return true }
            return result.error != nil || !sample.errors.isEmpty || Date().timeIntervalSince1970 - sample.timestamp > 10
        }
    }
    @ViewBuilder private var serverContent: some View {
        HStack { VStack(alignment: .leading, spacing: 4) { Text("服务器").font(.system(size: compact ? 20 : 22, weight: .semibold, design: .default)); Text("\(model.settings.hosts.filter(\.enabled).count) 台主机").font(AppFont.secondary).foregroundStyle(.secondary) }; Spacer(); Button { model.settingsTab = "servers"; model.requestHostEditor = true; model.showSettings?() } label: { Image(systemName: "plus") }.help("添加主机") }
        if model.settings.hosts.isEmpty { EmptyCard(icon: "server.rack", title: "添加服务器", subtitle: "", action: { model.settingsTab = "servers"; model.requestHostEditor = true; model.showSettings?() }) }
        Picker("服务器筛选", selection: $serverFilter) { Text("全部").tag("all"); Text("异常").tag("errors"); Text("已暂停").tag("paused") }.pickerStyle(.segmented)
        ForEach(filteredHosts) { host in
            ServerCard(host: host, result: model.hosts.first(where: { $0.id == host.id }), compact: compact, onRefresh: { Task { await model.sampleHosts(hostID: host.id) } })
        }
    }
}

struct TokenBreakdown: View {
    var tokens: Tokens
    var inline = false
    private var values: [(String, String)] { [("普通输入", Format.compact(tokens.input)), ("输出", Format.compact(tokens.output)), ("缓存读取", Format.compact(tokens.cacheRead)), ("缓存写入", Format.compact(tokens.cacheWrite)), ("读取命中率", Format.percent(tokens.cacheRate.map { $0 * 100 }))] }
    var body: some View {
        if inline {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 110), alignment: .leading)], alignment: .leading, spacing: 10) {
                ForEach(values, id: \.0) { label, value in VStack(alignment: .leading, spacing: 4) { Text(label).font(AppFont.secondary).foregroundStyle(.secondary); Text(value).font(AppFont.section).monospacedDigit().textSelection(.enabled) } }
            }
        } else {
            VStack(spacing: 6) { ForEach(values, id: \.0) { label, value in HStack { Text(label).foregroundStyle(.secondary); Spacer(); Text(value).monospacedDigit().textSelection(.enabled) }.font(AppFont.secondary) } }
        }
    }
}
struct CacheSummaryCard: View {
    var tokens: Tokens
    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 120), alignment: .leading)], alignment: .leading, spacing: 12) {
            ForEach([("普通输入", Format.compact(tokens.input)), ("输出", Format.compact(tokens.output)), ("缓存读取", Format.compact(tokens.cacheRead)), ("缓存写入", Format.compact(tokens.cacheWrite)), ("读取命中率", Format.percent(tokens.cacheRate.map { $0 * 100 }))], id: \.0) { label, value in
                VStack(alignment: .leading, spacing: 6) { Text(label).font(AppFont.secondary).foregroundStyle(.secondary); Text(value).font(AppFont.section).monospacedDigit().textSelection(.enabled) }
            }
        }.padding(14).modifier(NeutralCard()).help("读取命中率 = 缓存读取 ÷（普通输入 + 缓存读取 + 缓存写入）")
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
    @State private var selected: String?
    @State private var hidden = Set<String>()
    private var models: [String] { Array(Set(rows.map(\.model))).sorted() }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Chart {
                ForEach(rows.filter { !hidden.contains($0.model) }) { row in
                    BarMark(x: .value("日期", row.day), y: .value(cost ? "USD" : "Token", cost ? row.usage.cost : row.usage.total)).foregroundStyle(by: .value("模型", row.model))
                }
                if let selected { RuleMark(x: .value("所选日期", selected)).foregroundStyle(Palette.accent).lineStyle(StrokeStyle(lineWidth: 1, dash: [3])) }
            }
            .chartForegroundStyleScale(domain: models, range: models.map { Palette.model($0) })
            .chartXScale(domain: days.map(\.key)).chartLegend(.hidden)
            .chartXSelection(value: $selected)
            .chartXAxis { AxisMarks(values: days.enumerated().filter { $0.offset % max(1, days.count / 7) == 0 }.map { $0.element.key }) { value in AxisValueLabel { if let label = value.as(String.self) { Text(String(label.suffix(5))).font(AppFont.secondary) } } } }
            .chartYAxis { AxisMarks(position: .leading, values: .automatic(desiredCount: 3)) { value in AxisGridLine().foregroundStyle(.primary.opacity(0.06)); AxisValueLabel { if let n = value.as(Double.self) { Text(cost ? Format.money(n) : Format.compact(n)).font(AppFont.secondary) } } } }
            .chartOverlay { proxy in GeometryReader { geometry in Color.clear.contentShape(Rectangle()).onContinuousHover { phase in if case .active(let point) = phase, let frame = proxy.plotFrame { selected = proxy.value(atX: point.x - geometry[frame].minX, as: String.self) } } } }
            .frame(height: 180).focusable().onKeyPress(.leftArrow) { move(-1); return .handled }.onKeyPress(.rightArrow) { move(1); return .handled }
            .accessibilityLabel("每日用量，左右方向键选择日期，下方提供精确读数")
            if let day = days.first(where: { $0.key == selected }) {
                Text(day.key + " · " + (cost ? String(format: "%.6f USD", day.cost) : String(format: "%.0f Token", day.total))).font(AppFont.secondary).monospacedDigit().textSelection(.enabled)
            }
            DisclosureGroup("模型图例 · 点击显示或隐藏") {
                FlowLayout { ForEach(models, id: \.self) { name in Button { if hidden.contains(name) { hidden.remove(name) } else { hidden.insert(name) } } label: { Label(name, systemImage: hidden.contains(name) ? "eye.slash" : "eye").font(AppFont.secondary).strikethrough(hidden.contains(name)) }.buttonStyle(.plain).help(name).accessibilityLabel((hidden.contains(name) ? "显示模型 " : "隐藏模型 ") + name) } }
            }.font(AppFont.secondary)
        }
    }
    private func move(_ offset: Int) { guard !days.isEmpty else { return }; let index = selected.flatMap { key in days.firstIndex { $0.key == key } } ?? 0; selected = days[min(days.count - 1, max(0, index + offset))].key }
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
    private func exact(_ usage: Aggregate) -> String { String(format: "%.0f Token · %.6f USD", usage.total, usage.cost) + " · 读取命中率 " + Format.percent(usage.tokens.cacheRate.map { $0 * 100 }) }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(days.reversed()) { day in
                DisclosureGroup {
                    Text(exact(day)).font(AppFont.secondary).monospacedDigit().textSelection(.enabled)
                    ForEach(rows.filter { $0.day == day.key }) { row in
                        VStack(alignment: .leading, spacing: 4) { Text(row.model).fontWeight(.medium).textSelection(.enabled); Text(exact(row.usage)).monospacedDigit().textSelection(.enabled) }.font(AppFont.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 5)
                            .accessibilityElement(children: .combine).accessibilityLabel(day.key + " · " + row.model + " · " + exact(row.usage))
                            .contextMenu { Button("复制模型与精确数值") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(day.key + "\t" + row.model + "\t" + exact(row.usage), forType: .string) } }
                    }
                    if day.total == 0 { Text("当日用量为零").font(AppFont.secondary).foregroundStyle(.secondary) }
                } label: { HStack { Text(day.key); Spacer(); Text(cost ? Format.money(day.cost) + " USD" : Format.compact(day.total) + " Token").monospacedDigit() }.font(AppFont.secondary) }
            }
        }
    }
}

struct ModelChart: View {
    var models: [Aggregate], cost: Bool
    @State private var showAll = false
    private var ranked: [Aggregate] { models.sorted { value($0) == value($1) ? $0.key < $1.key : value($0) > value($1) } }
    private func value(_ item: Aggregate) -> Double { cost ? item.cost : item.total }
    private var total: Double { models.reduce(0) { $0 + value($1) } }
    private var slices: [Aggregate] {
        let remainder = Array(ranked.dropFirst(5))
        guard !remainder.isEmpty else { return ranked }
        return Array(ranked.prefix(5)) + [Aggregate(key: "其他", total: remainder.reduce(0) { $0 + $1.total }, cost: remainder.reduce(0) { $0 + $1.cost })]
    }
    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            Chart(slices) { item in SectorMark(angle: .value(cost ? "USD" : "Token", value(item)), innerRadius: .ratio(0.72), angularInset: 2).foregroundStyle(Palette.model(item.key)).cornerRadius(3) }.frame(width: 132, height: 160)
                .chartBackground { _ in VStack { Text("\(models.count)").font(.title2); Text("模型").font(AppFont.secondary) } }.accessibilityLabel("前五模型与其他；完整数值见列表")
            VStack(alignment: .leading, spacing: 10) {
                ForEach(showAll ? ranked : Array(ranked.prefix(5))) { item in
                    VStack(alignment: .leading, spacing: 3) { Text(item.key).font(AppFont.secondary).textSelection(.enabled).help(item.key); Text((cost ? Format.money(item.cost) + " USD" : Format.compact(item.total) + " Token") + " · " + Format.percent(total > 0 ? value(item) / total * 100 : 0)).font(AppFont.secondary).monospacedDigit().foregroundStyle(.secondary) }
                        .contextMenu { Button("复制模型与精确数值") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(item.key + "\t" + String(item.total) + " Token\t" + String(item.cost) + " USD", forType: .string) } }
                }
                if ranked.count > 5 { Button(showAll ? "收起其他模型" : "展开其他 \(ranked.count - 5) 个模型") { showAll.toggle() }.font(AppFont.secondary) }
            }.frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

struct Heatmap: View {
    var days: [Aggregate], cost: Bool
    @State private var selected = 0
    @State private var showDay = false
    private var leading: Int {
        guard let first = days.first, let date = ISO8601DateFormatter().date(from: first.key + "T12:00:00Z") else { return 0 }
        return (Calendar.current.component(.weekday, from: date) + 5) % 7
    }
    private var maximum: Double { max(1, days.map { cost ? $0.cost : $0.total }.max() ?? 1) }
    private func label(_ day: Aggregate) -> String { day.key + " · " + (cost ? String(format: "%.6f USD", day.cost) : String(format: "%.0f Token", day.total)) }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            GeometryReader { proxy in
                let columns = max(1, (days.count + leading + 6) / 7), step = (proxy.size.width - 22) / CGFloat(columns), size = max(3, min(13, step - 3))
                VStack(alignment: .leading, spacing: 6) {
                    ZStack(alignment: .topLeading) { ForEach(Array(days.enumerated().filter { $0.offset == 0 || $0.element.key.hasSuffix("-01") }), id: \.element.key) { index, day in Text(String(day.key.dropFirst(5).prefix(2)) + "月").font(.system(size: 11)).offset(x: 22 + CGFloat((index + leading) / 7) * step) } }.frame(height: 15)
                    HStack(alignment: .top, spacing: 3) {
                        VStack(spacing: 3) { ForEach(0..<7) { row in Text(["一", "", "三", "", "五", "", "日"][row]).font(.system(size: 11)).frame(width: 16, height: size) } }
                        ForEach(0..<columns, id: \.self) { column in VStack(spacing: 3) {
                            ForEach(0..<7) { row in
                                let index = column * 7 + row - leading
                                if days.indices.contains(index) {
                                    let day = days[index], value = cost ? day.cost : day.total
                                    RoundedRectangle(cornerRadius: 2).fill(value == 0 ? Color.primary.opacity(0.055) : Palette.accent.opacity(0.25 + 0.75 * sqrt(value / maximum))).frame(width: size, height: size)
                                        .overlay { if selected == index { RoundedRectangle(cornerRadius: 2).strokeBorder(Color.primary, lineWidth: 1.5) } }
                                        .help(label(day)).accessibilityLabel(label(day)).onTapGesture { selected = index; showDay = true }
                                } else { Color.clear.frame(width: size, height: size) }
                            }
                        }.frame(width: step - 3) }
                    }
                }
            }.frame(height: 130).focusable()
                .onKeyPress(.leftArrow) { move(-7); return .handled }.onKeyPress(.rightArrow) { move(7); return .handled }
                .onKeyPress(.upArrow) { move(-1); return .handled }.onKeyPress(.downArrow) { move(1); return .handled }
                .onKeyPress(.return) { showDay = true; return .handled }
                .accessibilityLabel("年度用量，方向键选择日期，回车查看当天详情")
            if days.indices.contains(selected) { Text(label(days[selected])).font(AppFont.secondary).textSelection(.enabled) }
            HStack { Text("零"); ForEach(0..<5) { n in RoundedRectangle(cornerRadius: 2).fill(n == 0 ? Color.primary.opacity(0.055) : Palette.accent.opacity(0.25 + Double(n) * 0.1875)).frame(width: 12, height: 12) }; Text("多") }.font(AppFont.secondary).foregroundStyle(.secondary)
            DisclosureGroup("年度数据表 · 精确值") {
                Table(days) {
                    TableColumn("日期", value: \.key)
                    TableColumn("Token") { day in Text(String(format: "%.0f", day.total)).textSelection(.enabled) }
                    TableColumn("API 等价成本 USD") { day in Text(String(format: "%.6f", day.cost)).textSelection(.enabled) }
                }.frame(height: 260)
            }.font(AppFont.secondary)
        }.sheet(isPresented: $showDay) { if days.indices.contains(selected) { let day = days[selected]; VStack(alignment: .leading, spacing: 14) { Text(day.key).font(AppFont.title); Text(label(day)).textSelection(.enabled); CacheSummaryCard(tokens: day.tokens); Button("关闭") { showDay = false }.keyboardShortcut(.cancelAction) }.padding(24).frame(width: 520) } }
    }
    private func move(_ offset: Int) { selected = min(max(0, days.count - 1), max(0, selected + offset)) }
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
        _expanded = AppStorage(wrappedValue: !compact && quota.provider != "agy", "quota.expanded." + (compact ? "panel." : "detail.") + quota.id)
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
            ForEach(expanded ? agyGroups : Array(agyGroups.prefix(1)), id: \.self) { group in
                VStack(alignment: .leading, spacing: 7) {
                    Text(group).font(.system(size: 14, weight: .medium)).fixedSize(horizontal: false, vertical: true)
                    HStack(alignment: .top, spacing: 14) {
                        ForEach(quota.windows.filter { $0.groupLabel == group }.sorted { ($0.windowMinutes ?? 0) < ($1.windowMinutes ?? 0) }) { window in
                            VStack(alignment: .leading, spacing: 5) {
                                HStack { Text(window.windowMinutes == 10080 ? "7d" : window.windowMinutes == 300 ? "5h" : window.name); Spacer(minLength: 2); Text(Format.percent(max(0, 100-window.usedPercent))).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent)) }.font(AppFont.secondary)
                                ResourceBar(percent: 100-window.usedPercent, tint: Palette.quota(window.usedPercent))
                                TimelineView(.periodic(from: .now, by: 60)) { context in Text(Format.resetCountdown(window.resetsAt, now: context.date)).font(AppFont.secondary).foregroundStyle(.secondary).help(Format.date(window.resetsAt)) }
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
                    if let reset = window.resetsAt {
                        TimelineView(.periodic(from: .now, by: 60)) { context in
                            HStack {
                                Text(Format.resetCountdown(reset, now: context.date)).help(Format.date(reset))
                                Spacer()

                            }.font(AppFont.secondary).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            }
            }
            if let onEstimate, (!compact || expanded || estimate != nil), (quota.provider != "agy" && quota.windows.contains(where: { $0.windowMinutes == 300 || $0.windowMinutes == 10080 })) || estimate != nil {
                Button(action: onEstimate) {
                    HStack { Label(estimate == nil ? "估算额度价值" : estimate?.valuationMode == "fiveHour" ? "5h / 7d 估值" : "7d 整周估值", systemImage: "chart.line.uptrend.xyaxis"); Spacer();
                        if let value = estimate?.valuationMode == "fiveHour" ? estimate?.fiveHourValue : estimate?.weeklyValue { Text((estimate?.valuationMode == "fiveHour" ? "5h ≈ " : "7d 整周 ≈ ") + Format.money(value) + " USD").monospacedDigit(); if estimate?.status == "pending" { Text("待确认").foregroundStyle(Palette.warn) } }
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
        _expanded = AppStorage(wrappedValue: false, "server.expanded." + (compact ? "panel." : "detail.") + host.id)
    }
    var body: some View {
        Surface(spacing: 8, padding: 12) {
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
            if !expanded {
                HStack(spacing: 14) {
                    if host.metrics.contains("cpu") { compactMetric("CPU", percent: result?.sample?.cpu?.first(where: { $0.id == "cpu" })?.utilization) }
                    if host.metrics.contains("memory") { compactMetric("内存", percent: result?.sample?.memory.flatMap { $0.total > 0 ? ($0.total - $0.available) / $0.total * 100 : nil }) }
                    if host.metrics.contains("gpu") { compactMetric("GPU / \(result?.sample?.gpu?.count ?? 0) 卡", percent: result?.sample?.gpu?.compactMap(\.utilization).max()) }
                }.opacity(host.enabled ? 1 : 0.65)
                HStack {
                    Text("采样于 " + Format.time(result?.sample?.timestamp ?? 0) + (host.enabled ? "" : " · 已暂停，保留旧读数"))
                    Spacer()
                    if host.enabled { Button("刷新", action: onRefresh).accessibilityLabel("刷新服务器 " + (host.name.isEmpty ? host.target : host.name)) }
                }.font(AppFont.secondary).foregroundStyle(.secondary)
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
        HStack(spacing: 6) { Text(title).foregroundStyle(.secondary); Text(Format.percent(percent)).monospacedDigit() }
            .font(AppFont.secondary).frame(maxWidth: .infinity, alignment: .leading)
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
