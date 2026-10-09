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
private struct TranslucentPanel: EnvironmentKey { static let defaultValue = false }
extension EnvironmentValues {
    var translucentPanel: Bool {
        get { self[TranslucentPanel.self] }
        set { self[TranslucentPanel.self] = newValue }
    }
    var previewAccessibleSurfaces: Bool {
        get { self[AccessibleSurfacePreview.self] }
        set { self[AccessibleSurfacePreview.self] = newValue }
    }
}
struct NeutralCard: ViewModifier {
    @Environment(\.translucentPanel) private var translucentPanel
    @Environment(\.previewAccessibleSurfaces) private var previewAccessible
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast
    func body(content: Content) -> some View {
        content.background {
            if previewAccessible || reduceTransparency || contrast == .increased { RoundedRectangle(cornerRadius: Palette.radiusCard).fill(Color(nsColor: .controlBackgroundColor)) }
            else if translucentPanel { RoundedRectangle(cornerRadius: Palette.radiusCard).fill(Color(nsColor: .controlBackgroundColor).opacity(0.35)) }
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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.previewAccessibleSurfaces) private var previewAccessible
    var compact = true
    var nativePanelBackground = false
    @AppStorage private var tab: String
    @State private var costMode = false
    @AppStorage private var serverFilter: String
    @AppStorage private var trendExpanded: Bool
    @AppStorage private var sessionsExpanded: Bool
    @AppStorage("panel.accounts.v2") private var panelAccounts = "{}"
    private var usageDashboard: Dashboard { compact ? model.panelDashboard : model.dashboard }
    init(model: AppModel, compact: Bool = true, page: String = "agent", nativePanelBackground: Bool = false) {
        self.model = model; self.compact = compact; self.nativePanelBackground = nativePanelBackground
        let scope = compact ? "panel." : "detail."
        _tab = AppStorage(wrappedValue: page, scope + "page")
        _serverFilter = AppStorage(wrappedValue: "all", scope + "serverFilter")
        _trendExpanded = AppStorage(wrappedValue: false, scope + "trendExpanded")
        _sessionsExpanded = AppStorage(wrappedValue: false, scope + "sessionsExpanded")
    }
    var body: some View {
        usageObservedContent
            .onChange(of: tab) { _, value in if !compact { model.detailPage = value }; model.setServerVisible(value == "servers", window: compact ? "panel" : "detail"); if value == "servers" { Task { await model.sampleHosts() } } }
            .onChange(of: model.requestedDetailPage) { _, page in if !compact, let page { tab = page; model.requestedDetailPage = nil } }
            .onAppear { if !compact, let page = model.requestedDetailPage { tab = page; model.requestedDetailPage = nil }; model.setServerVisible(tab == "servers", window: compact ? "panel" : "detail") }
    }
    // Keep each modifier chain small enough for release-build type checking.
    private var rootLayout: some View {
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
    }
    private var styledContent: some View {
        rootLayout
        .frame(width: compact ? 450 : nil, height: compact ? model.panelHeight : nil)
        .frame(minWidth: compact ? nil : 760, minHeight: compact ? nil : 480)
        .background { rootBackground }
        .environment(\.translucentPanel, compact)
        .environment(\.surfaceActive, model.isWindowVisible(compact ? "panel" : "detail"))
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: tab)
        .font(AppFont.body).disabled(model.installingUpdate)
        .aieyesAccent()
    }
    private var accountObservedContent: some View {
        styledContent
        .onAppear { initializePanelAccounts() }
        .onChange(of: model.settings) { _, _ in initializePanelAccounts() }
        .onChange(of: model.dashboard.generatedAt) { _, _ in initializePanelAccounts() }
        .onChange(of: model.dashboard.quotaOrder) { _, _ in initializePanelAccounts() }
    }
    private var usageObservedContent: some View {
        accountObservedContent
        .onChange(of: model.provider) { _, _ in model.selectedModel = "all"; model.selectedAccount = "all"; model.selectedSource = "all"; Task { await model.reload() } }
        .onChange(of: model.selectedAccount) { _, _ in model.selectedSource = "all"; Task { await model.reload() } }
        .onChange(of: model.selectedSource) { _, _ in Task { await model.reload() } }
        .onChange(of: model.selectedModel) { _, _ in Task { await model.reload() } }
        .onChange(of: model.range) { _, _ in Task { await model.reload() } }
    }
    @ViewBuilder private var rootBackground: some View {
        if compact {
            // NSPopover already supplies behind-window vibrancy. Avoid stacking blur layers.
            if !nativePanelBackground { Color(nsColor: .windowBackgroundColor).opacity(reduceTransparency || contrast == .increased || previewAccessible ? 1 : 0.12) }
        } else {
            ZStack {
                Rectangle().fill(.ultraThinMaterial)
                Color(nsColor: .windowBackgroundColor).opacity(0.5)
            }
        }
    }
    private func initializePanelAccounts() {
        guard compact && model.settingsLoaded && model.dashboard.generatedAt > 0 && !CommandLine.arguments.contains("--render") else { return }
        if panelAccounts == "{}" { panelAccounts = PanelAccountPreference.migrated(UserDefaults.standard.string(forKey: "panel.accounts.v1") ?? "{}") }
    }
    private var header: some View {
        HStack(spacing: 12) {
            BrandMark().frame(width: 34, height: 34)
            Picker("页面", selection: $tab) { Text("Agent").tag("agent"); Text("服务器").tag("servers") }
                .pickerStyle(.segmented).labelsHidden().frame(maxWidth: .infinity)
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
    private var footerStatus: String { (tab == "servers" ? model.serverStatusText : model.statusText) + " · 出站 · " + (model.networkBusy ? "测试中…" : model.networkTest?.label ?? "连接未测试") }
    private var footer: some View {
        VStack(alignment: .leading, spacing: compact ? 3 : 8) {
            Button { Task { if tab == "servers" { await model.sampleHosts() } else { await model.scan() } } } label: {
                HStack(spacing: 6) {
                    Circle().fill(model.busy || model.quotaBusy || model.serverBusy ? Palette.accent : !model.actionFailures.isEmpty ? Palette.danger : (tab == "servers" ? model.serverStatusText.contains("延迟") : model.statusText.contains("延迟") || model.dataTime == 0) ? Palette.warn : Palette.ok).frame(width: 7, height: 7)
                    Text(footerStatus).font(.system(size: compact ? 12 : 14)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.tail)
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.accessibilityLabel(footerStatus + (tab == "servers" ? "，点击刷新服务器" : "，点击同步记录"))
                .help(footerStatus + "\n应用出站测试不代表 SSH 或所有账户可用。\n" + (model.networkTest?.detail ?? "可从更多菜单测试出站连接"))
            HStack(spacing: 8) {
                Spacer(minLength: 0)
                ThemeToggleButton(model: model)
                if compact {
                    UpdateMenuButton()
                    Button { model.showDetailPage?(tab) } label: { Image(systemName: "arrow.up.left.and.arrow.down.right").frame(width: 28, height: 28).contentShape(Rectangle()) }.help("打开详情").accessibilityLabel("打开详情")
                }
                Button { model.showSettings?() } label: { Image(systemName: "gearshape").frame(width: 28, height: 28).contentShape(Rectangle()) }.help("设置").accessibilityLabel("设置").keyboardShortcut(",")
                Menu {
                    if compact { Button("面板显示账户…") { model.showPanelAccounts?() }; Button(model.isPinned ? "取消固定面板" : "固定面板") { model.isPinned.toggle() }; Divider() }
                    Button(model.networkBusy ? "出站测试中…" : "测试出站连接") { Task { await model.testNetwork() } }.disabled(model.networkBusy)
                    Text("关闭窗口后继续在菜单栏运行"); Divider(); Button("退出 Aieyes") { NSApplication.shared.terminate(nil) }.keyboardShortcut("q")
                } label: { Image(systemName: "ellipsis").frame(width: 28, height: 28).contentShape(Rectangle()) }.menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().accessibilityLabel("更多操作").help("面板账户、固定、出站测试与退出")
            }
        }.buttonStyle(.plain).padding(.horizontal, compact ? 18 : 28).padding(.vertical, compact ? 5 : 10)
    }
    @ViewBuilder private var agentContent: some View {
        sessionStrip
        if model.settings.sources.isEmpty { connectionGuide }
        HStack(spacing: 8) {
            Picker("Agent", selection: $model.provider) {
                Text("全部 Agent").tag("all")
                ForEach(["codex", "claude", "antigravity", "deepseek", "custom"], id: \.self) { Text(Format.provider($0)).tag($0) }
            }.labelsHidden()
            Picker("账户", selection: $model.selectedAccount) {
                Text("全部账户").tag("all"); Text("未关联账户").tag("none")
                ForEach(model.settings.historicalAccounts.filter { model.provider == "all" || $0.provider == model.provider }, id: \.key) { Text($0.name).tag($0.key) }
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

        if model.dashboardPending || model.dashboardError != nil {
            VStack(alignment: .leading, spacing: 4) {
                Text((model.dashboardPending ? "正在更新，仍显示上一结果：" : "更新失败，仍显示上一结果：") + model.appliedScope).font(AppFont.secondary)
                if let error = model.dashboardError { Text(error).font(AppFont.secondary); Button("重试查询") { Task { await model.reload() } } }
            }.foregroundStyle(.secondary)
        }
        HStack(alignment: .firstTextBaseline) {
            Text(model.range == 1 ? "今日概览" : "使用概览").font(.system(size: compact ? 17 : 22, weight: .semibold))
            Spacer()
            Group { Picker("时间范围", selection: $model.range) { Text("今日").tag(1); Text("最近 7 天").tag(7); Text("最近 30 天").tag(30); Text("最近 90 天").tag(90); Text("最近一年").tag(365) }.labelsHidden().fixedSize() }
        }
        if hasUsageData {
            HStack(spacing: 10) {
                if compact { FloatingTokenCard(total: usageDashboard.summary.total, tokens: usageDashboard.summary.tokens) } else { StatCard(title: "总 Token", value: Format.compact(usageDashboard.summary.total), detail: "", icon: "sparkle", accent: Palette.accent) }
                StatCard(title: "API 等价成本", value: usageDashboard.summary.total == 0 || usageDashboard.summary.pricedTokens > 0 ? Format.money(usageDashboard.summary.cost) : "—", detail: "", icon: "dollarsign.circle", accent: Palette.accent, pricingIncomplete: usageDashboard.summary.total > usageDashboard.summary.pricedTokens, onPricing: { model.openPricing() })
            }
            if !compact { CacheSummaryCard(tokens: usageDashboard.summary.tokens) }
        } else if !model.settings.sources.isEmpty { usageEmptyState }
        quotaSection
        if !model.runningEstimates.isEmpty {
            Button { model.showSampling?() } label: { HStack { Text(model.samplingSummary); Spacer(); Text("管理采样"); Image(systemName: "chevron.right") }.font(AppFont.secondary) }.buttonStyle(.plain).foregroundStyle(model.samplingNeedsAttention ? Palette.warn : Palette.accent)
        }
        if hasUsageData {
            if compact {
                DisclosureGroup("近 \(max(7, model.range)) 天趋势与每日明细", isExpanded: $trendExpanded) { UsageChart(days: recentDays, rows: recentRows, cost: false).frame(minHeight: 220); DailyUsage(days: recentDays, rows: recentRows, compact: true) }.font(AppFont.secondary)
                Button { model.showDetailPage?("agent") } label: { Text("用量详情").frame(maxWidth: .infinity) }.buttonStyle(.bordered).accessibilityLabel("打开用量详情")
            } else {
                Text("日期、来源、模型筛选作用于用量分析；今日视图的趋势仍为近 7 天，热力图为过去 365 天。").font(AppFont.secondary).foregroundStyle(.secondary)
                HStack { Text(model.range == 1 ? "近 7 天趋势" : "使用趋势").font(AppFont.section); Spacer(); Picker("统计指标", selection: $costMode) { Text("Token").tag(false); Text("API 等价成本").tag(true) }.pickerStyle(.segmented).frame(width: 230) }
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .top, spacing: 18) { trendSurface.frame(minWidth: 450); modelSurface.frame(minWidth: 350) }
                    VStack(spacing: 18) { trendSurface; modelSurface }
                }
                Surface(title: "每日明细 · 可选择与复制精确数值") { DailyUsage(days: model.dashboard.trendDays, rows: model.dashboard.dayModels, compact: false, cost: costMode) }
                Surface(title: "过去 365 天") { Heatmap(days: model.dashboard.heatmap, cost: costMode) }
            }
        }
    }
    private var trendSurface: some View { Surface(title: "每日用量 · 按模型") { UsageChart(days: model.dashboard.trendDays, rows: model.dashboard.dayModels, cost: costMode).frame(minHeight: 230) } }
    private var modelSurface: some View { Surface(title: "所选范围 · 模型分布") { ModelChart(models: model.dashboard.models, cost: costMode).frame(minHeight: 230) } }
    private var usageEmptyState: some View {
        let onlyQuota = model.settings.sources.allSatisfy { $0.provider == "deepseek" }
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
        Surface(title: "启用你的第一个 Agent") {
            Text("接入日志查看用量，或连接账户查看限额与余额。").font(AppFont.secondary).foregroundStyle(.secondary)
            ViewThatFits(in: .horizontal) {
                HStack { connectionButtons }
                VStack(alignment: .leading, spacing: 8) { connectionButtons }
            }
        }
    }
    @ViewBuilder private var connectionButtons: some View {
        Button("设置 Agents", systemImage: "folder") { model.settingsTab = "sources"; model.showSettings?() }
        Button("账户限额", systemImage: "person.crop.circle") { model.settingsTab = "accounts"; model.showSettings?() }
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
                if model.selectedAccount != "all" { filterChip("账户：" + (model.selectedAccount == "none" ? "未关联账户" : model.settings.historicalAccounts.first { $0.key == model.selectedAccount }?.name ?? model.selectedAccount)) { model.selectedAccount = "all" } }
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
    private var recentDays: [Aggregate] { Array(usageDashboard.trendDays.suffix(max(7, model.range))) }
    private var selectableSources: [AgentSource] {
        let account = model.settings.historicalAccounts.first { $0.key == model.selectedAccount }
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
        DisclosureGroup(isExpanded: $sessionsExpanded) {
            VStack(spacing: 12) {
                ForEach(model.sessions) { session in
                    HStack(spacing: 10) {
                        ActivityIndicator(phase: session.phase)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(session.source).lineLimit(1)
                            SurfaceTimeline(interval: 60) { context in
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
        let selections = PanelAccountPreference.selections(panelAccounts, accounts: model.settings.accounts, order: model.dashboard.quotaOrder ?? [])
        return model.quotaAccounts.filter { account in
            (model.provider == "all" || model.provider == account.provider) && (model.selectedAccount == "all" || model.selectedAccount == account.key) && (!compact || selections[account.provider]?.contains(account.key) == true)
        }
    }
    private var displayedQuotas: [Quota] { model.dashboard.quotas.filter { quota in !compact || displayedQuotaAccounts.contains { $0.key == quota.id } } }
    private var quotaSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("实时账户限额").font(.system(size: compact ? 20 : 22, weight: .semibold)); if !displayedQuotaAccounts.isEmpty { Text("\(displayedQuotaAccounts.count)").font(AppFont.secondary).foregroundStyle(.secondary) }; Spacer(); Button { model.showQuotaOrder?() } label: { Image(systemName: "arrow.up.arrow.down") }.help("调整账户顺序").accessibilityLabel("调整账户顺序"); Button(model.quotaBusy ? "读取中…" : "刷新限额") { Task { await model.refreshQuotas() } }.font(AppFont.secondary).disabled(model.quotaBusy) }
            ForEach(displayedQuotaAccounts.filter { account in
                !model.dashboard.quotas.contains { $0.provider == account.provider && $0.accountId == account.id }
            }, id: \.key) { account in
                Surface {
                    HStack { Text(account.name).font(AppFont.section); Spacer(); if model.quotaBusy { ProgressView().controlSize(.small) } }
                    Text(model.quotaBusy ? "正在读取账户限额…" : model.quotaError == nil ? "等待首次限额查询" : "暂时无法读取限额").font(AppFont.secondary).foregroundStyle(.secondary)
                }
            }
            if compact && displayedQuotaAccounts.isEmpty { Text("当前面板未显示账户，可在更多中选择；详情保留全部账户。").font(AppFont.secondary).foregroundStyle(.secondary) }
            if let error = model.quotaError {
                VStack(alignment: .leading, spacing: 4) {
                    Text(error).font(AppFont.secondary).foregroundStyle(Palette.warn).textSelection(.enabled)
                    if let retry = model.quotaNextAttempt { Text("下次自动重试：" + retry.formatted(date: .omitted, time: .shortened)).font(AppFont.secondary).foregroundStyle(.secondary) }
                }
            }
            if compact { ForEach(displayedQuotas) { quota in QuotaCard(quota: quota, compact: true, collapsible: true, estimate: model.dashboard.quotaEstimates?.first { $0.accountKey == quota.id }, onEstimate: { group in model.showEstimate?(quota, false, group) }, onCredits: { model.showEstimate?(quota, true, nil) }, creditEstimate: model.dashboard.creditEstimates?.first { $0.accountKey == quota.id }, quotaHistory: (model.dashboard.quotaEstimates ?? []).filter { $0.accountKey == quota.id }, creditHistory: (model.dashboard.creditEstimates ?? []).filter { $0.accountKey == quota.id }) } }
            else { LazyVGrid(columns: [GridItem(.adaptive(minimum: 340), spacing: 16, alignment: .top)], alignment: .leading, spacing: 16) { ForEach(model.dashboard.quotas) { quota in QuotaCard(quota: quota, collapsible: true, estimate: model.dashboard.quotaEstimates?.first { $0.accountKey == quota.id }, onEstimate: { group in model.showEstimate?(quota, false, group) }, onCredits: { model.showEstimate?(quota, true, nil) }, creditEstimate: model.dashboard.creditEstimates?.first { $0.accountKey == quota.id }, quotaHistory: (model.dashboard.quotaEstimates ?? []).filter { $0.accountKey == quota.id }, creditHistory: (model.dashboard.creditEstimates ?? []).filter { $0.accountKey == quota.id }) } } }
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

struct ThemeToggleButton: View {
    @ObservedObject var model: AppModel
    @Environment(\.colorScheme) private var colorScheme
    private var dark: Bool { colorScheme == .dark }
    private var label: String { dark ? "切换到浅色模式" : "切换到深色模式" }
    var body: some View {
        Button { Task { await model.toggleTheme(currentlyDark: dark) } } label: {
            Image(systemName: dark ? "sun.max" : "moon")
                .frame(width: 28, height: 28).contentShape(Rectangle())
        }.help(label).accessibilityLabel(label)
            .disabled(!model.settingsLoaded || model.settingsSaving)
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
                Text(day.key + " · " + (cost ? String(format: "%.6f USD", day.cost) : Format.compact(day.total) + " Token")).font(AppFont.secondary).monospacedDigit().textSelection(.enabled)
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
    private func display(_ usage: Aggregate) -> String { Format.compact(usage.total) + " Token · " + String(format: "%.6f USD", usage.cost) + " · 读取命中率 " + Format.percent(usage.tokens.cacheRate.map { $0 * 100 }) }
    private func modelRow(_ row: DayModel, day: String) -> some View {
        let value = exact(row.usage)
        let label = day + " · " + row.model + " · " + value
        return VStack(alignment: .leading, spacing: 4) {
            Text(row.model).fontWeight(.medium).textSelection(.enabled)
            Text(display(row.usage)).help(value).monospacedDigit().textSelection(.enabled)
        }.font(AppFont.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 5)
            .accessibilityElement(children: .combine).accessibilityLabel(label)
            .contextMenu { Button("复制模型与精确数值") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(day + "\t" + row.model + "\t" + value, forType: .string)
            } }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(days.reversed()) { day in
                DisclosureGroup {
                    Text(display(day)).help(exact(day)).font(AppFont.secondary).monospacedDigit().textSelection(.enabled)
                    ForEach(rows.filter { $0.day == day.key }) { row in
                        modelRow(row, day: day.key)
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
    private func label(_ day: Aggregate) -> String { day.key + " · " + (cost ? String(format: "%.6f USD", day.cost) : Format.compact(day.total) + " Token") }
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
                    TableColumn("Token") { day in Text(Format.compact(day.total)).help(String(format: "%.0f Token", day.total)).textSelection(.enabled) }
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
    var collapsible = false
    var sourceName = ""
    var estimate: QuotaEstimate?
    var onEstimate: ((String?) -> Void)?
    var onCredits: (() -> Void)?
    var creditEstimate: QuotaEstimate?
    var quotaHistory: [QuotaEstimate] = []
    var creditHistory: [QuotaEstimate] = []
    private var bestCredit: QuotaEstimate? { EstimatePresentation.credit(creditHistory) ?? creditEstimate }
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @AppStorage private var storedExpanded: Bool
    private var expanded: Bool { !collapsible || storedExpanded }
    init(quota: Quota, compact: Bool = false, collapsible: Bool = false, sourceName: String = "", estimate: QuotaEstimate? = nil, onEstimate: ((String?) -> Void)? = nil, onCredits: (() -> Void)? = nil, creditEstimate: QuotaEstimate? = nil, quotaHistory: [QuotaEstimate] = [], creditHistory: [QuotaEstimate] = []) {
        self.quotaHistory = quotaHistory; self.creditHistory = creditHistory
        self.collapsible = collapsible; self.quota = quota; self.compact = compact; self.sourceName = sourceName; self.estimate = estimate; self.onEstimate = onEstimate; self.onCredits = onCredits; self.creditEstimate = creditEstimate
        _storedExpanded = AppStorage(wrappedValue: true, "quota.expanded.v2." + (compact ? "panel." : "detail.") + quota.id)
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
                                QuotaResetLabel(window: window)
                            }.frame(maxWidth: .infinity).accessibilityElement(children: .combine).accessibilityLabel(window.name + "，剩余 " + Format.percent(max(0, 100-window.usedPercent)))
                        }
                    }
                    if let onEstimate {
                        let groupId = quota.windows.first { $0.groupLabel == group }?.groupId ?? group
                        EstimateEntry(records: quotaHistory.filter { $0.groupId == groupId }) { onEstimate(groupId) }
                    }
                }
            }
        }
    }
    private var headerAccessibility: String {
        var text = Format.provider(quota.provider) + "，" + quota.name
        if let plan = quota.plan { text += "，订阅 " + Format.subscription(plan) }
        if !expanded && quota.provider == "codex" { text += "，" + Format.creditBalance(quota.credits, estimate: creditEstimate) }
        text += expanded ? "，折叠限额" : "，展开限额"
        return text
    }
    @ViewBuilder private var cardHeader: some View {
            Button { withAnimation(reduceMotion ? nil : .spring(response: 0.32, dampingFraction: 0.86)) { if collapsible { storedExpanded.toggle() } } } label: {
            HStack(spacing: 8) {
                ProviderMark(provider: quota.provider)
                Text(quota.name).font(.system(size: 14, weight: .semibold)).lineLimit(1).help(quota.name)
                if let plan = quota.plan, !Format.subscription(plan).isEmpty { SubscriptionBadge(plan: plan).frame(maxWidth: 120) }
                Spacer(minLength: 0)
                if !expanded && quota.provider == "codex" { CreditBalanceLabel(balance: quota.credits, estimate: bestCredit, compact: true).layoutPriority(1) }
                if collapsible { Image(systemName: expanded ? "chevron.up" : "chevron.down").font(.system(size: 14, weight: .semibold)).foregroundStyle(.secondary) }
            }.contentShape(Rectangle())
            }.buttonStyle(.plain).help(collapsible ? (expanded ? "折叠限额" : "展开限额") : quota.name)
                .accessibilityLabel(headerAccessibility)
            if expanded {
                HStack { Text(Format.provider(quota.provider)); Spacer(); if quota.updatedAt > 0 { Text("\(quota.origin == "log" ? "记录" : "更新") \(Format.date(quota.updatedAt))") } }.font(AppFont.secondary).foregroundStyle(.secondary)
            }
    }
    private var fullCard: some View {
        Surface {
            cardHeader
            if !expanded {
                balanceContent
                HStack(alignment: .top, spacing: 12) {
                    ForEach(Array(quota.windows.prefix(2))) { window in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(window.windowMinutes == 300 ? "5h" : window.windowMinutes == 10080 ? "7d" : window.name).font(AppFont.secondary).lineLimit(2)
                            Text("剩余 " + Format.percent(max(0, 100 - window.usedPercent))).font(AppFont.secondary).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent))
                            ResourceBar(percent: 100 - window.usedPercent, tint: Palette.quota(window.usedPercent))
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                if quota.error != nil { Text("限额更新失败 · 展开查看").font(AppFont.secondary).foregroundStyle(Palette.warn) }
            }
            if expanded {
            balanceContent
            if ["antigravity", "agy"].contains(quota.provider) { agyWindows }
            else if !quota.windows.isEmpty { LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .leading), count: expanded ? 1 : 2), alignment: .leading, spacing: expanded ? 14 : 8) {
            ForEach(quota.windows) { window in
                VStack(spacing: 6) {
                    ViewThatFits(in: .horizontal) {
                        HStack { Text(window.name); Spacer(minLength: 6); Text("剩余 " + Format.percent(max(0, 100 - window.usedPercent))).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent)) }
                        VStack(alignment: .leading, spacing: 3) { Text(window.name); Text("剩余 " + Format.percent(max(0, 100 - window.usedPercent))).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent)) }.frame(maxWidth: .infinity, alignment: .leading)
                    }.font(AppFont.secondary)
                    ResourceBar(percent: 100 - window.usedPercent, tint: Palette.quota(window.usedPercent))
                    QuotaResetLabel(window: window).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            }
            }
            if let onEstimate, !["antigravity", "agy"].contains(quota.provider), quota.windows.contains(where: { $0.windowMinutes == 300 || $0.windowMinutes == 10080 }) || estimate != nil {
                EstimateEntry(records: quotaHistory.isEmpty ? [estimate].compactMap { $0 } : quotaHistory) { onEstimate(nil) }
            }
            if expanded && !sourceName.isEmpty { Text(sourceName).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1) }
            if quota.provider == "codex" {
                CreditBalanceLabel(balance: quota.credits, estimate: bestCredit).fixedSize(horizontal: false, vertical: true)
                    .help(quota.creditsUpdatedAt.map { "更新于 " + Format.date($0) } ?? "尚未取得 credits 信息")
                if let onCredits { EstimateEntry(records: creditHistory.isEmpty ? [creditEstimate].compactMap { $0 } : creditHistory, credits: true, action: onCredits) }
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
                EmptyView()
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: "server.rack").font(.system(size: 20)).foregroundStyle(Palette.accent)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(host.name.isEmpty ? host.target : host.name).font(AppFont.section).lineLimit(1).help(host.name.isEmpty ? host.target : host.name)
                        if expanded { Text(host.target).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1).help(host.target) }
                    }
                    Spacer()
                    SurfaceTimeline(interval: 1) { context in
                        let label = status(at: context.date)
                        HStack(spacing: 6) { Circle().fill(label == "正常" ? Palette.ok : label == "已暂停" ? Color.gray : label == "连接失败" ? Palette.danger : Palette.warn).frame(width: 6, height: 6); Text(label).font(AppFont.secondary).foregroundStyle(.secondary) }
                    }

                }
            }
            ServerResourceSummary(host: host, sample: result?.sample)
                .opacity(host.enabled ? 1 : 0.65)
            if expanded {
                if let sample = result?.sample {
                    VStack(alignment: .leading, spacing: 16) {
                        if host.metrics.contains("cpu"), let rows = sample.cpu {
                            let cpus = host.selectedDevices("cpu", rows)
                            DisclosureGroup {
                                ForEach(cpus.filter { $0.id != "cpu" }) { cpu in
                                    VStack(spacing: 4) {
                                        metricRow(cpu.id, Format.percent(cpu.utilization))
                                        ResourceBar(percent: cpu.utilization)
                                    }.padding(.vertical, 3)
                                }
                                if host.shows("cpuTimes"), let cpu = cpus.first(where: { $0.id == "cpu" }) { metricRow("user / system", "\(Format.percent(cpu.userPercent)) / \(Format.percent(cpu.systemPercent))"); metricRow("iowait / steal", "\(Format.percent(cpu.iowaitPercent)) / \(Format.percent(cpu.stealPercent))") }
                            } label: { MetricHeading(icon: "cpu", title: "CPU 明细", value: "") }
                        }
                        if host.metrics.contains("memory"), let m = sample.memory {
                            DisclosureGroup {
                                metricRow("可用", Format.bytes(m.available)); if host.shows("memoryCache") { metricRow("缓存 / Buffer", "\(Format.bytes(m.cached)) / \(Format.bytes(m.buffers))") }
                                if host.shows("swap") { metricRow("Swap", "\(Format.bytes(m.swapUsed)) / \(Format.bytes(m.swapTotal))"); ResourceBar(percent: Format.capacityPercent(m.swapUsed, m.swapTotal)) }
                            } label: { MetricHeading(icon: "memorychip", title: "内存明细", value: "") }
                        }
                        if host.shows("gpuThermals") {
                            ForEach(host.selectedDevices("gpu", sample.gpu)) { gpu in
                                DisclosureGroup {
                                    metricRow("温度 / 功耗", "\(gpu.temperature.map { String(format: "%.0f°C", $0) } ?? "—") / \(gpu.powerWatts.map { String(format: "%.1f W", $0) } ?? "—")")
                                } label: { MetricHeading(icon: "rectangle.3.group", title: "GPU \(gpu.id) 明细", value: "") }
                            }
                        }
                        if host.metrics.contains("filesystems"), let rows = sample.filesystems {
                            let filesystems = host.selectedDevices("filesystems", rows)
                            DisclosureGroup {
                                ForEach(filesystems) { fs in VStack(alignment: .leading, spacing: 4) {
                                    Text(fs.id).font(AppFont.secondary).lineLimit(1).truncationMode(.middle).help(fs.id)
                                    if host.shows("fsAvailable") { metricRow("可用", Format.bytes(fs.available)) }
                                    if host.shows("fsType") { metricRow(fs.device ?? "", fs.type ?? "") }
                                    if host.shows("inodes"), let total = fs.inodes, let free = fs.inodesFree, total > 0 { metricRow("inode", Format.percent((total - free) / total * 100)) }
                                }.padding(.vertical, 4) }
                            } label: { MetricHeading(icon: "internaldrive", title: "文件系统", value: "\(filesystems.count) 个挂载点") }
                        }
                        if host.metrics.contains("disk"), let rows = sample.disk {
                            let disks = host.selectedDevices("disk", rows)
                            DisclosureGroup {
                                ForEach(disks) { disk in VStack(spacing: 4) { metricRow(disk.id, "读 \(Format.speed(disk.readBytesPerSecond))"); metricRow("", "写 \(Format.speed(disk.writeBytesPerSecond))"); if host.shows("diskIops") { metricRow("IOPS 读 / 写", "\(disk.readIops.map(Format.compact) ?? "—") / \(disk.writeIops.map(Format.compact) ?? "—")") }; if host.shows("diskBusy") { metricRow("忙碌率", Format.percent(disk.busyMsPerSecond.map { min(100, max(0, $0 / 10)) })); ResourceBar(percent: disk.busyMsPerSecond.map { $0 / 10 }) } }.padding(.vertical, 3) }
                            } label: { MetricHeading(icon: "arrow.left.arrow.right", title: "磁盘 I/O", value: "\(disks.count) 个设备") }
                        }
                        if host.metrics.contains("network"), let rows = sample.network {
                            let networks = host.selectedDevices("network", rows)
                            DisclosureGroup {
                                ForEach(networks) { net in VStack(spacing: 4) {
                                    metricRow(net.id, "↓ \(Format.speed(net.rxBytesPerSecond))")
                                    metricRow("", "↑ \(Format.speed(net.txBytesPerSecond))")
                                    if host.shows("networkErrors") { metricRow("错误 / 丢包", "\(Format.compact((net.rxErrors ?? 0) + (net.txErrors ?? 0))) / \(Format.compact((net.rxDrops ?? 0) + (net.txDrops ?? 0)))") }
                                }.padding(.vertical, 3) }
                            } label: { MetricHeading(icon: "network", title: "网络", value: "\(networks.count) 个网卡") }
                        }
                        ForEach(sample.errors.keys.filter { host.metrics.contains($0) }.sorted(), id: \.self) { key in metricRow(["gpu":"GPU", "cpu":"CPU", "memory":"内存", "filesystems":"文件系统", "disk":"磁盘 I/O", "network":"网络"][key] ?? key, "采集失败") }
                        HStack { Text("负载 " + sample.load.map { String(format: "%.2f", $0) }.joined(separator: " / ")); Spacer(); SurfaceTimeline(interval: 1) { context in Text("" + Date(timeIntervalSince1970: sample.timestamp).formatted(.dateTime.hour().minute().second()) + (context.date.timeIntervalSince1970 - sample.timestamp > 10 ? " · 数据已延迟" : "")) } }.font(AppFont.secondary).foregroundStyle(.secondary)
                    }.padding(.top, 14)
                } else if host.enabled { Text(result?.error == nil ? "等待首次采样" : "连接失败").font(AppFont.secondary).foregroundStyle(.secondary).padding(.top, 10) }
                if let error = result?.error { Text(error).font(AppFont.secondary).foregroundStyle(Palette.warn).padding(.top, 6) }
            }
            HStack {
                Text("采样于 " + Format.time(result?.sample?.timestamp ?? 0) + (host.enabled ? "" : " · 已暂停，保留旧读数"))
                Spacer()
                if host.enabled { Button("刷新", action: onRefresh).accessibilityLabel("刷新服务器 " + (host.name.isEmpty ? host.target : host.name)) }
            }.font(AppFont.secondary).foregroundStyle(.secondary)
        }
    }
    private func status(at date: Date) -> String {
        if !host.enabled { return "已暂停" }
        if result?.error != nil { return "连接失败" }
        guard let sample = result?.sample else { return "等待采样" }
        if date.timeIntervalSince1970 - sample.timestamp > 10 { return "数据延迟" }
        return sample.errors.isEmpty ? "正常" : "部分采集失败"
    }
    private func metricRow(_ key: String, _ value: String) -> some View { HStack { Text(key).lineLimit(1).truncationMode(.middle); Spacer(minLength: 8); Text(value).monospacedDigit() }.font(AppFont.secondary).foregroundStyle(.secondary).padding(.vertical, 2) }
}
struct MetricHeading: View {
    var icon: String, title: String, value: String
    var body: some View {
        HStack(spacing: 8) { Image(systemName: icon).frame(width: 15).foregroundStyle(.secondary); Text(title); Spacer(); Text(value).monospacedDigit().foregroundStyle(.secondary) }.font(.system(size: 14, weight: .medium))
    }
}

struct ServerResourceSummary: View {
    var host: Host, sample: MetricSample?
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 170), alignment: .leading)], alignment: .leading, spacing: 10) {
                if host.metrics.contains("cpu") {
                    ResourceRing(title: "CPU", percent: sample?.cpu?.first { $0.id == "cpu" }?.utilization, compact: true)
                }
                if host.metrics.contains("memory") {
                    ResourceRing(title: "内存", percent: Format.capacityPercent(sample?.memory?.used, sample?.memory?.total), compact: true,
                                 detail: "\(Format.bytes(sample?.memory?.used)) / \(Format.bytes(sample?.memory?.total))")
                }
            }
            if host.metrics.contains("gpu") {
                let gpus = host.selectedDevices("gpu", sample?.gpu)
                if gpus.isEmpty { Text(sample?.gpu == nil ? "GPU —" : "GPU 无已选设备").font(AppFont.secondary).foregroundStyle(.secondary) }
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 290), alignment: .leading)], alignment: .leading, spacing: 10) {
                    ForEach(gpus) { gpu in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text("GPU " + gpu.id).fontWeight(.medium).fixedSize()
                                Text(gpu.name ?? "—").foregroundStyle(.secondary).lineLimit(1).help(gpu.name ?? "—")
                            }.font(AppFont.secondary)
                            HStack(alignment: .top, spacing: 12) {
                                ResourceStrip(title: "利用率", percent: gpu.utilization).frame(width: host.shows("gpuMemory") ? 86 : nil)
                                if host.shows("gpuMemory") {
                                    ResourceStrip(title: "显存", percent: Format.capacityPercent(gpu.memoryUsedMiB, gpu.memoryTotalMiB),
                                                  detail: Format.capacityPair(gpu.memoryUsedMiB.map { $0 * 1048576 }, gpu.memoryTotalMiB.map { $0 * 1048576 }))
                                }
                            }

                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
            if host.metrics.contains("filesystems") {
                let filesystems = host.selectedDevices("filesystems", sample?.filesystems)
                VStack(alignment: .leading, spacing: 6) {
                    Text("文件系统").font(AppFont.secondary).foregroundStyle(.secondary)
                    if filesystems.isEmpty { Text(sample?.filesystems == nil ? "—" : "无已选挂载点").font(AppFont.secondary).foregroundStyle(.secondary) }
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 290), alignment: .leading)], spacing: 9) {
                        ForEach(filesystems) { fs in ResourceStrip(title: fs.id, percent: Format.capacityPercent(fs.used, fs.total), detail: Format.capacityPair(fs.used, fs.total)) }
                    }
                }
            }
            if host.shows("uptime") {
                Text("连续运行 " + Format.uptime(sample?.uptime)).font(AppFont.secondary).monospacedDigit().foregroundStyle(.secondary)
            }
            if host.shows("networkTotals"), host.metrics.contains("network") {
                let networks = host.selectedDevices("network", sample?.network)
                if networks.isEmpty { Text(sample?.network == nil ? "累计流量 —" : "累计流量 无已选网卡").font(AppFont.secondary).foregroundStyle(.secondary) }
                ForEach(networks) { net in
                    ViewThatFits(in: .horizontal) {
                        HStack { Text(net.id + " 累计"); Text("↓ \(Format.bytes(net.rxBytes)) · ↑ \(Format.bytes(net.txBytes))").monospacedDigit() }
                        VStack(alignment: .leading, spacing: 3) { Text(net.id + " 累计"); Text("↓ \(Format.bytes(net.rxBytes)) · ↑ \(Format.bytes(net.txBytes))").monospacedDigit() }
                    }.font(AppFont.secondary).foregroundStyle(.secondary)
                }
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct ResourceStrip: View {
    var title: String, percent: Double?
    var detail = ""
    private var value: Double? { percent.flatMap { $0.isFinite ? min(100, max(0, $0)) : nil } }
    private var label: String { (detail.isEmpty ? "" : detail + " · ") + (value.map { String(format: "%.0f%%", $0) } ?? "—") }
    var body: some View {
        VStack(spacing: 3) {
            HStack(spacing: 5) {
                Text(title).font(AppFont.secondary).lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 0)
                Text(label).font(.system(size: 12)).monospacedDigit().lineLimit(1).fixedSize().foregroundStyle((value ?? 0) >= 90 ? Palette.danger : Color.primary)
            }.frame(height: 17)
            ResourceBar(percent: value)
        }.frame(maxWidth: .infinity, alignment: .leading).help(title + " " + label)
            .accessibilityElement(children: .ignore).accessibilityLabel(title + " " + label)
    }
}

struct ResourceRing: View {
    var title: String, percent: Double?
    var compact = false
    var detail = ""
    private var value: Double? { percent.flatMap { $0.isFinite ? min(100, max(0, $0)) : nil } }
    private var accessibilityText: String {
        var text = title + " " + Format.percent(value)
        if !detail.isEmpty { text += "，" + detail }
        if let value, value >= 90 { text += "，高负载" }
        return text
    }
    private var ring: some View {
        ZStack {
            Circle().stroke(.primary.opacity(0.07), lineWidth: 5)
            if let value, value > 0 {
                Circle().trim(from: 0, to: value / 100)
                    .stroke(value >= 90 ? Palette.danger : value >= 70 ? Palette.warn : Palette.accent, style: StrokeStyle(lineWidth: 5, lineCap: .round))
                    .rotationEffect(.degrees(-90))
            }
            Text(Format.percent(value)).font(.system(size: compact ? 10 : 14, weight: .semibold)).monospacedDigit()
        }.frame(width: compact ? 48 : 72, height: compact ? 48 : 72)
    }
    var body: some View {
        HStack(spacing: 8) {
            ring
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(AppFont.secondary).fontWeight(.medium)
                if !detail.isEmpty { Text(detail).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
                if let value, value >= 90 { Text("高负载").font(AppFont.secondary).foregroundStyle(Palette.danger) }
            }
        }.frame(maxWidth: .infinity, alignment: .leading).accessibilityElement(children: .ignore).accessibilityLabel(accessibilityText)
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
                    Capsule().fill(tint ?? (percent >= 90 ? Palette.danger : percent >= 70 ? Palette.warn : Palette.accent))
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
        private var configurationPending = false
        func configure() {
            guard !configurationPending else { return }
            configurationPending = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.configurationPending = false
                guard let scroll = self.enclosingScrollView else { return }
                if scroll.scrollerStyle != .overlay { scroll.scrollerStyle = .overlay }
                if !scroll.autohidesScrollers { scroll.autohidesScrollers = true }
                if scroll.drawsBackground { scroll.drawsBackground = false }
            }
        }
    }
    func makeNSView(context: Context) -> Anchor { Anchor() }
    func updateNSView(_ view: Anchor, context: Context) { view.configure() }
}
