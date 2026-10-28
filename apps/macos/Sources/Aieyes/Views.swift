import SwiftUI
import Charts

/// Fill the available width without reserving empty adaptive columns.
struct BalancedGridLayout: Layout {
    var minimumWidth: CGFloat
    var spacing: CGFloat = 10
    var maximumColumns: Int = .max
    struct Cache {
        struct Key: Equatable { let width: CGFloat, minimum: CGFloat, spacing: CGFloat; let maximum: Int, count: Int }
        struct Measurement { let key: Key, size: CGSize, points: [CGPoint], sizes: [CGSize] }
        var measurements: [Measurement] = []
    }
    func makeCache(subviews: Subviews) -> Cache { Cache() }
    func updateCache(_ cache: inout Cache, subviews: Subviews) { cache.measurements.removeAll(keepingCapacity: true) }
    private func columns(_ width: CGFloat, _ count: Int) -> Int {
        let available = Int(min(CGFloat(max(1, count)), max(1, (width + spacing) / max(1, minimumWidth + spacing))))
        let fit = max(1, min(count, maximumColumns, available))
        return max(1, Int(ceil(Double(count) / ceil(Double(max(1, count)) / Double(fit)))))
    }
    private func rows(_ width: CGFloat, _ subviews: Subviews) -> [[Int]] {
        let count = columns(width, subviews.count)
        return stride(from: 0, to: subviews.count, by: count).map { Array($0..<min($0 + count, subviews.count)) }
    }
    private func measure(_ proposedWidth: CGFloat?, subviews: Subviews, cache: inout Cache) -> Cache.Measurement {
        let width = max(0, proposedWidth.flatMap { $0.isFinite ? $0 : nil } ?? minimumWidth)
        let key = Cache.Key(width: width, minimum: minimumWidth, spacing: spacing, maximum: maximumColumns, count: subviews.count)
        if let known = cache.measurements.first(where: { $0.key == key }) { return known }
        let columns = columns(width, subviews.count)
        var points: [CGPoint] = [], sizes: [CGSize] = [], y: CGFloat = 0
        for row in rows(width, subviews) {
            let cell = max(0, row.count == 1 ? width : (width - CGFloat(columns - 1) * spacing) / CGFloat(columns))
            var height: CGFloat = 0
            for (column, index) in row.enumerated() {
                let size = subviews[index].sizeThatFits(.init(width: cell, height: nil))
                points.append(CGPoint(x: CGFloat(column) * (cell + spacing), y: y)); sizes.append(CGSize(width: cell, height: size.height))
                height = max(height, size.height)
            }
            y += height + spacing
        }
        let result = Cache.Measurement(key: key, size: CGSize(width: width, height: max(0, y - (subviews.isEmpty ? 0 : spacing))), points: points, sizes: sizes)
        if cache.measurements.count >= 8 { cache.measurements.removeFirst() }
        cache.measurements.append(result); return result
    }
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) -> CGSize { measure(proposal.width, subviews: subviews, cache: &cache).size }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) {
        let measured = measure(bounds.width, subviews: subviews, cache: &cache)
        for index in subviews.indices {
            subviews[index].place(at: CGPoint(x: bounds.minX + measured.points[index].x, y: bounds.minY + measured.points[index].y), anchor: .topLeading, proposal: ProposedViewSize(measured.sizes[index]))
        }
    }
}

/// Measure each chart once. ViewThatFits would build and probe both a horizontal
/// and a vertical copy of the same expensive Chart/legend trees on every update.
struct ChartPairLayout: Layout {
    struct Cache { var width: CGFloat?; var sizes: [CGSize] = []; var proposals: [CGFloat] = []; var horizontal = false }
    func makeCache(subviews: Subviews) -> Cache { Cache() }
    func updateCache(_ cache: inout Cache, subviews: Subviews) { cache = Cache() }
    private func measure(_ proposed: CGFloat?, subviews: Subviews, cache: inout Cache) -> CGSize {
        let width = max(0, proposed.flatMap { $0.isFinite ? $0 : nil } ?? 818)
        if cache.width != width || cache.sizes.count != subviews.count {
            cache.width = width;cache.horizontal = subviews.count == 2 && width >= 818
            let first = max(450, (width - 18) / 2)
            cache.proposals = cache.horizontal ? [first, width - 18 - first] : Array(repeating:width,count:subviews.count)
            cache.sizes = subviews.indices.map { subviews[$0].sizeThatFits(.init(width:cache.proposals[$0],height:nil)) }
        }
        let height = cache.horizontal ? cache.sizes.map(\.height).max() ?? 0 : cache.sizes.map(\.height).reduce(0,+) + CGFloat(max(0,subviews.count-1))*18
        return CGSize(width:width,height:height)
    }
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) -> CGSize { measure(proposal.width,subviews:subviews,cache:&cache) }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) {
        _ = measure(bounds.width,subviews:subviews,cache:&cache)
        var point = bounds.origin
        for index in subviews.indices {
            subviews[index].place(at:point,anchor:.topLeading,proposal:.init(width:cache.proposals[index],height:cache.sizes[index].height))
            if cache.horizontal { point.x += cache.proposals[index]+18 } else { point.y += cache.sizes[index].height+18 }
        }
    }
}

private struct WorkspaceCardColumns: EnvironmentKey { static let defaultValue = 2 }
extension EnvironmentValues {
    var workspaceCardColumns: Int { get { self[WorkspaceCardColumns.self] } set { self[WorkspaceCardColumns.self] = newValue } }
}

struct Surface<Content: View>: View {
    var title: String?
    var spacing: CGFloat = DesignTokens.spaceCardContent
    var padding: CGFloat = DesignTokens.spaceCard
    var tint: Color = .clear
    var minimumHeight: CGFloat? = nil
    @ViewBuilder var content: Content
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: spacing) {
            if let title { Text(title).font(.system(size: DesignTokens.fontCardTitle, weight: DesignTokens.weightStrong)) }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading).padding(padding)
        .frame(minHeight: minimumHeight, alignment: .topLeading)
        .modifier(NeutralCard(tint: tint))
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
    var tint: Color = .clear
    @Environment(\.translucentPanel) private var translucentPanel
    @Environment(\.previewAccessibleSurfaces) private var previewAccessible
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast
    func body(content: Content) -> some View {
        content.background {
            if previewAccessible || reduceTransparency || contrast == .increased { RoundedRectangle(cornerRadius: Palette.radiusCard).fill(Color(nsColor: .controlBackgroundColor)) }
            else if translucentPanel { RoundedRectangle(cornerRadius: Palette.radiusCard).fill(Color(nsColor: .controlBackgroundColor).opacity(DesignTokens.panelCardOpacity)) }
            else { RoundedRectangle(cornerRadius: Palette.radiusCard).fill(.regularMaterial) }
            RoundedRectangle(cornerRadius: Palette.radiusCard).fill(LinearGradient(colors: [tint.opacity(0.18), tint.opacity(0.04)], startPoint: .topLeading, endPoint: .bottomTrailing))
        }
        .shadow(color: .black.opacity(previewAccessible || reduceTransparency || contrast == .increased ? 0 : 0.06), radius: 9, y: 3)
        .overlay(RoundedRectangle(cornerRadius: Palette.radiusCard).strokeBorder(Palette.cardBorder.opacity(previewAccessible || contrast == .increased ? 1 : DesignTokens.cardBorderOpacity), lineWidth: previewAccessible || contrast == .increased ? 2 : DesignTokens.borderWidth))
    }
}
struct RootView: View, Equatable {
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.model === rhs.model && lhs.compact == rhs.compact && lhs.page == rhs.page && lhs.nativePanelBackground == rhs.nativePanelBackground }
    @Environment(\.workspaceCardColumns) private var workspaceCardColumns
    let model: AppModel
    @StateObject private var observation: SurfaceObservation
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.previewAccessibleSurfaces) private var previewAccessible
    var compact = true
    var nativePanelBackground = false
    private let snapshotPage: String?
    private var activePage: String { compact ? (snapshotPage ?? tab) : page }
    private let page: String
    private var scope: String { compact ? "panel" : activePage == "analysis" ? "analysis" : "overview" }
    private var filters: DashboardFilterPreference {
        get { model.filters(for: scope) }
        nonmutating set { model.updateFilters(newValue, scope: scope) }
    }
    private func filterBinding<Value>(_ key: WritableKeyPath<DashboardFilterPreference, Value>) -> Binding<Value> { Binding(get: { filters[keyPath: key] }, set: { filters[keyPath: key] = $0 }) }
    @AppStorage private var tab: String
    @AppStorage private var costMode: Bool
    @AppStorage private var serverFilter: String
    @AppStorage private var serverSelection: String
    @AppStorage private var trendExpanded: Bool
    @AppStorage private var sessionsExpanded: Bool
    @AppStorage(NativePanelKey.accounts.rawValue) private var panelAccounts = "{}"
    private var usageDashboard: Dashboard { compact ? model.panelDashboard : activePage == "analysis" ? model.dashboard : model.overviewDashboard }
    init(model: AppModel, compact: Bool = true, page: String = "agent", nativePanelBackground: Bool = false) {
        self.model = model; self.compact = compact; self.page = page;
        _observation = StateObject(wrappedValue: SurfaceObservation(model: model, compact: compact, page: page)); self.nativePanelBackground = nativePanelBackground
        self.snapshotPage = CommandLine.arguments.contains("--render") ? page : nil
        _tab = AppStorage(wrappedValue: page, NativePanelKey.scoped(.panelPage, .detailPage, compact: compact))
        _serverFilter = AppStorage(wrappedValue: "all", NativePanelKey.scoped(.panelServerFilter, .detailServerFilter, compact: compact))
        _serverSelection = AppStorage(wrappedValue: "all", NativePanelKey.scoped(.panelServerSelection, .detailServerSelection, compact: compact))
        _trendExpanded = AppStorage(wrappedValue: false, NativePanelKey.scoped(.panelTrend, .detailTrend, compact: compact))
        _sessionsExpanded = AppStorage(wrappedValue: false, NativePanelKey.scoped(.panelSessions, .detailSessions, compact: compact))
        _costMode = AppStorage(wrappedValue: false, NativePanelKey.scoped(.panelCost, .detailCost, compact: compact))
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        accountObservedContent
            .onChange(of: tab) { _, value in if compact { model.setServerVisible(value == "servers", window: "panel"); if value == "servers" { Task { await model.sampleHosts() } } } }
            .onAppear { if compact { model.setServerVisible(activePage == "servers", window: "panel") } }
    }
    // Keep each modifier chain small enough for release-build type checking.
    private var rootLayout: some View {
        VStack(spacing: 0) {
            if compact { header }
            Divider().opacity(0.55)
            ScrollView {
                Group {
                    if activePage == "servers" { LazyVStack(alignment: .leading, spacing: compact ? 8 : 12) { serverContent } }
                    else { VStack(alignment: .leading, spacing: compact ? DesignTokens.panelSectionGap : DesignTokens.spaceSection) { if activePage == "analysis" { analysisContent } else { agentContent } } }
                }.padding(compact ? DesignTokens.panelPadding : DesignTokens.contentPadding).background(OverlayScrollStyle()).background(ScrollMemory(key: scope + "." + activePage, ready: usageDashboard.generatedAt > 0))
            }
            Divider().opacity(0.55)
            footer
        }
    }
    private var styledContent: some View {
        rootLayout
        .accessibilityIdentifier(activePage == "servers" ? "server-overview" : "agent-overview")
        .frame(height: compact ? model.panelHeight : nil)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background { rootBackground }
        .environment(\.translucentPanel, compact)
        .environment(\.surfaceActive, model.isWindowVisible(compact ? "panel" : "detail") && (compact || model.detailPage == activePage))
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: activePage)
        .font(AppFont.body).disabled(model.installingUpdate)
        .aieyesAccent()
    }
    private var accountObservedContent: some View {
        styledContent
        .onAppear { initializePanelAccounts() }
        .onChange(of: model.settings) { _, _ in initializePanelAccounts() }
        .onChange(of: usageDashboard.generatedAt) { _, _ in initializePanelAccounts() }
        .onChange(of: usageDashboard.quotaOrder) { _, _ in initializePanelAccounts() }
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
        guard compact && model.settingsLoaded && usageDashboard.generatedAt > 0 && !CommandLine.arguments.contains("--render") else { return }
        if panelAccounts == "{}" { panelAccounts = PanelAccountPreference.migrated(UserDefaults.standard.string(forKey: NativePanelKey.accountsLegacy.rawValue) ?? "{}") }
    }
    private var header: some View { SurfaceHeader(model: model, tab: $tab, compact: compact, activePage: activePage) }
    private var footer: some View { SurfaceFooter(model: model, compact: compact, activePage: activePage) }
    @ViewBuilder private var agentContent: some View {
        if model.settings.sources.isEmpty && model.settings.accounts.isEmpty { connectionGuide }
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 4) { agentButtons }
            ScrollView(.horizontal) { HStack(spacing: 4) { agentButtons } }.scrollIndicators(.hidden)
        }
        sessionStrip
        Surface(spacing: 6, padding: 12) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing:24) { todayHeading; todayMetrics; Spacer(minLength:8); analysisAction }
                VStack(alignment:.leading,spacing:8) { HStack { todayHeading; Spacer(); analysisAction }; todayMetrics }
            }
        }
        if usageDashboard.summary.hasDisplayCredits { NativeCreditsView(usage: usageDashboard.summary, compact: true) }
        quotaSection
        if !model.runningEstimates.isEmpty { Button(model.samplingSummary + L(" · 管理采样")) { model.showSampling?() }.font(AppFont.caption) }
    }
    private var todayHeading: some View { HStack(spacing:6) { Text(L("今日用量")).font(AppFont.caption).foregroundStyle(.secondary); RefreshIndicator(model:model,key:"scan") }.fixedSize() }
    private var todayMetrics: some View {
        HStack(spacing:20) {
            FloatingTokenCard(total:usageDashboard.summary.total,tokens:usageDashboard.summary.tokens,inline:true,valueSize:compact ? DesignTokens.fontPanelOverview : DesignTokens.fontNumber)
            HStack(spacing:6) { Text(L("API 等价成本")).foregroundStyle(.secondary); Text(usageDashboard.summary.costAvailable ? Format.money(usageDashboard.summary.cost) : "—").font(.system(size:compact ? DesignTokens.fontPanelOverview : DesignTokens.fontNumber,weight:.semibold)) }
        }.font(AppFont.secondary).monospacedDigit().fixedSize(horizontal:true,vertical:false)
    }
    private var analysisAction: some View { HStack { Button(L("查看分析"),systemImage:"arrow.up.right") { model.openAnalysis(from:scope) }.font(AppFont.caption).tint(.primary); if usageDashboard.summary.pricedTokens < usageDashboard.summary.total { PricingWarning { model.openPricing() } } }.fixedSize() }
    @ViewBuilder private var agentButtons: some View {
        agentButton("all", title: L("总览"))
        ForEach(model.overviewAgentProviders, id: \.self) { provider in agentButton(provider, title: Format.provider(provider)) }
    }
    private func agentButton(_ provider: String, title: String) -> some View {
        Button { filters.provider = provider } label: { Text(title).font(AppFont.secondary).padding(.horizontal, 10).padding(.vertical, 7).foregroundStyle(filters.provider == provider ? Palette.category(provider) : Color.secondary).background(filters.provider == provider ? Palette.category(provider).opacity(0.16) : Color.clear, in: RoundedRectangle(cornerRadius: 8)) }.buttonStyle(.plain).accessibilityAddTraits(filters.provider == provider ? .isSelected : [])
    }
    @ViewBuilder private var analysisContent: some View {
        if model.settings.sources.isEmpty { connectionGuide }
        FlowLayout(spacing: 8) {
            Picker("Agent", selection: filterBinding(\.provider)) {
                Text(L("全部 Agent")).tag("all")
                ForEach(CoreContract.agentProviders, id: \.self) { Text(Format.provider($0)).tag($0) }
            }.labelsHidden()

            if !compact {
                Picker(L("数据源"), selection: filterBinding(\.source)) {
                    Text(L("全部数据源")).tag("all")
                    ForEach(selectableSources) { Text($0.name).tag($0.id) }
                }.labelsHidden()
                Picker(L("模型"), selection: filterBinding(\.model)) {
                    Text(L("全部模型")).tag("all")
                    ForEach(Array(Set(model.modelOptions + (filters.model == "all" ? [] : [filters.model]))).sorted(), id: \.self) { Text(usageDashboard.modelName($0)).tag($0) }
                }.labelsHidden()
            }
            Group { Picker(L("时间范围"), selection: Binding(get: { filters.rangeChoice }, set: { filters.rangeChoice = $0 })) { Text(L("今日")).tag("1"); Text(L("最近 7 天")).tag("7"); Text(L("最近 30 天")).tag("30"); Text(L("最近 90 天")).tag("90"); Text(L("最近一年")).tag("365"); Text(L("所有")).tag("all"); Text(L("自定义日期")).tag("custom") }.labelsHidden().fixedSize() }
        }.controlSize(.regular)
        activeFilters

        HStack(alignment: .firstTextBaseline) {
            Text(filters.isToday ? L("今日概览") : L("使用概览")).font(.system(size: compact ? DesignTokens.fontPanelOverview : DesignTokens.fontTitle, weight: DesignTokens.weightStrong)); RefreshIndicator(model: model, key: "scan")
            Spacer()
        }
        if filters.dateRange?.kind == "custom" {
            VStack(alignment: .leading, spacing: 8) {
                DatePicker(L("开始日期"), selection: Binding(get: { filters.customStart }, set: { filters.customStart = $0 }), in: ...filters.customEnd, displayedComponents: .date).environment(\.locale, AieyesLocale.locale)
                DatePicker(L("结束日期"), selection: Binding(get: { filters.customEnd }, set: { filters.customEnd = $0 }), in: filters.customStart..., displayedComponents: .date).environment(\.locale, AieyesLocale.locale)
            }.font(AppFont.secondary)
        }
        if hasUsageData {
            HStack(spacing: 10) {
                if compact { FloatingTokenCard(total: usageDashboard.summary.total, tokens: usageDashboard.summary.tokens) } else { StatCard(title: L("总 Token"), value: Format.compact(usageDashboard.summary.total), detail: "", icon: "sparkle", accent: Palette.accent) }
                StatCard(title: L("API 等价成本"), value: usageDashboard.summary.costAvailable ? Format.money(usageDashboard.summary.cost) : "—", detail: "", icon: "dollarsign.circle", accent: Palette.accent, pricingIncomplete: usageDashboard.summary.total > usageDashboard.summary.pricedTokens, onPricing: { model.openPricing() })
            }
            if !compact { CacheSummaryCard(tokens: usageDashboard.summary.tokens) }
            if usageDashboard.summary.hasDisplayCredits { Surface { NativeCreditsView(usage: usageDashboard.summary, models: usageDashboard.models, compact: compact, modelNames: usageDashboard.modelDisplayNames ?? [:]) } }
        } else if !model.settings.sources.isEmpty { usageEmptyState }
        if !model.runningEstimates.isEmpty {
            Button { model.showSampling?() } label: { HStack { Text(model.samplingSummary); Spacer(); Text(L("管理采样")); Image(systemName: "chevron.right") }.font(AppFont.secondary) }.buttonStyle(.plain).foregroundStyle(model.samplingNeedsAttention ? Palette.warn : Palette.accent)
        }
        if hasUsageData {
            if compact {
                DisclosureGroup(filters.dateRange == nil ? L("近 {0} 天趋势与每日明细" , max(7, filters.days)) : L("所选日期趋势与每日明细"), isExpanded: $trendExpanded) { UsageChart(days: recentDays, rows: recentRows, cost: false, modelNames: usageDashboard.modelDisplayNames ?? [:]).equatable().frame(minHeight: 220); DailyUsage(days: recentDays, rows: recentRows, compact: true, modelNames: usageDashboard.modelDisplayNames ?? [:]).equatable() }.font(AppFont.secondary)
                Button { model.showDetailPage?("agent") } label: { Text(L("用量详情")).frame(maxWidth: .infinity) }.buttonStyle(.aieyes).accessibilityLabel(L("打开用量详情"))
            } else {
                HStack { Text(filters.isToday ? L("近 7 天趋势") : L("使用趋势")).font(AppFont.section); Spacer(); Picker(L("统计指标"), selection: $costMode) { Text("Token").tag(false); Text(L("API 等价成本")).tag(true) }.pickerStyle(.segmented).frame(width: 230) }
                ChartPairLayout { trendSurface; modelSurface }
                Surface(title: L("每日明细")) { DailyUsage(days: usageDashboard.trendDays, rows: usageDashboard.dayModels, compact: false, cost: costMode, modelNames: usageDashboard.modelDisplayNames ?? [:]).equatable() }
                Surface(title: filters.dateRange == nil ? L("过去 365 天") : L("所选日期用量")) { if filters.dateRange == nil { Heatmap(days: usageDashboard.heatmap, cost: costMode).equatable() } else { YearlyHeatmaps(days: usageDashboard.heatmap, cost: costMode) } }
            }
        }
    }
    private var trendSurface: some View { Surface(title: L("每日用量")) { UsageChart(days: usageDashboard.trendDays, rows: usageDashboard.dayModels, cost: costMode, modelNames: usageDashboard.modelDisplayNames ?? [:]).equatable().frame(minHeight: DesignTokens.chartSurfaceHeight) } }
    private var modelSurface: some View { Surface(title: L("模型分布")) { ModelChart(models: usageDashboard.models, cost: costMode, modelNames: usageDashboard.modelDisplayNames ?? [:]).equatable().frame(minHeight: DesignTokens.chartSurfaceHeight) } }
    private var usageEmptyState: some View {
        let onlyQuota = model.settings.sources.allSatisfy { $0.provider == "deepseek" }
        let failed = usageDashboard.sources.contains { $0.enabled && $0.status?.failureReason != nil }
        return Surface(title: onlyQuota ? L("此来源提供账户额度") : failed ? L("记录同步失败") : hasActiveFilters ? L("没有匹配的用量记录") : model.dataTime == 0 ? L("尚未同步用量记录") : L("此时间范围暂无用量")) {
            Text(onlyQuota ? L("连接账户查看额度与余额；添加日志目录后分析用量。") : failed ? L("请检查对应来源的错误后重试。") : L("保留筛选与日期入口，可同步记录或扩大范围。")).font(AppFont.secondary).foregroundStyle(.secondary)
            HStack { if onlyQuota { Button(L("添加日志来源")) { model.openSettings(.init(provider: "codex", section: "machines")) } } else { Button(L("同步记录")) { Task { await model.scan() } }; Button(L("扩大到最近一年")) { filters.rangeChoice = "365"; if compact { model.showDetailPage?("agent") } }; if hasActiveFilters { Button(L("清除筛选")) { filters.provider = "all"; filters.account = "all"; filters.source = "all"; filters.model = "all" } } } }
        }
    }
    private var hasActiveFilters: Bool { filters.provider != "all" || filters.source != "all" || filters.model != "all" || !filters.isToday }
    private var hasUsageData: Bool {
        usageDashboard.summary.total > 0 || usageDashboard.summary.hasNativeCredits || usageDashboard.trendDays.contains { $0.total > 0 || $0.hasNativeCredits } || usageDashboard.heatmap.contains { $0.total > 0 || $0.hasNativeCredits }
    }
    private var connectionGuide: some View {
        Surface(title: L("启用你的第一个 Agent")) {
            ViewThatFits(in: .horizontal) {
                HStack { connectionButtons }
                VStack(alignment: .leading, spacing: 8) { connectionButtons }
            }
        }
    }
    @ViewBuilder private var connectionButtons: some View {
        Button(L("接入 Agent"), systemImage: "folder") { model.openSettings(.init(section: "machines")) }.buttonStyle(.aieyesPrimary).controlSize(.large)
        Button(L("配置向导"), systemImage: "checklist") { model.showDetailPage?("setup") }
    }
    @ViewBuilder private var activeFilters: some View {
        if hasActiveFilters {
            filterChips
        }
    }
    private var filterChips: some View {
            FlowLayout {
                if filters.provider != "all" { filterChip("Agent：" + Format.provider(filters.provider)) { filters.provider = "all" } }
                if !filters.isToday { filterChip(filters.rangeTitle) { filters.rangeChoice = "1" } }
                Button(L("清除全部")) { filters.provider = "all"; filters.account = "all"; filters.source = "all"; filters.model = "all"; filters.rangeChoice = "1" }
                if filters.source != "all" {
                    filterChip(L("数据源：") + (model.settings.sources.first { $0.id == filters.source }?.name ?? filters.source)) { filters.source = "all" }
                }
                if filters.model != "all" { filterChip(L("模型：") + usageDashboard.modelName(filters.model)) { filters.model = "all" } }
            }
    }
    private func filterChip(_ title: String, clear: @escaping () -> Void) -> some View {
        HStack(spacing: 8) {
            Text(title).font(AppFont.secondary).lineLimit(2).help(title)
            Button(action: clear) { Image(systemName: "xmark.circle.fill").frame(width: DesignTokens.iconButtonSize, height: DesignTokens.iconButtonSize) }.buttonStyle(.plain).accessibilityLabel(L("清除") + title)
        }.padding(.leading, 10).background(Palette.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
    }
    private var recentDays: [Aggregate] { filters.dateRange == nil ? Array(usageDashboard.trendDays.suffix(max(7, filters.days))) : usageDashboard.trendDays }
    private var selectableSources: [AgentSource] {
        model.settings.sources.filter { filters.provider == "all" || CoreContract.sameAgent($0.provider, filters.provider) }
    }
    private var recentRows: [DayModel] {
        let dates = Set(recentDays.map(\.key))
        return usageDashboard.dayModels.filter { dates.contains($0.day) }
    }
    private var sessionStrip: some View { SessionStrip(model: model, status: model.sessionStripStatus, expanded: $sessionsExpanded, compact: compact, provider: filters.provider) }
    private var displayedQuotaAccounts: [AgentAccount] {
        let selections = PanelAccountPreference.selections(panelAccounts, accounts: model.settings.accounts, order: usageDashboard.quotaOrder ?? [])
        return model.quotaAccounts.filter { account in
            (filters.provider == "all" || CoreContract.sameAgent(filters.provider, account.provider)) && (!compact || selections[account.provider]?.contains(account.key) == true)
        }
    }
    private var displayedQuotas: [Quota] { usageDashboard.quotas.filter { quota in !compact || displayedQuotaAccounts.contains { $0.key == quota.id } } }
    private var quotaProviders: [String] {
        let order = usageDashboard.quotaOrder ?? []
        return CoreContract.agentProviders.filter { provider in displayedQuotaAccounts.contains { CoreContract.sameAgent($0.provider, provider) } }.sorted { left, right in
            let leftRank = order.firstIndex { CoreContract.sameAgent(String($0.split(separator: ":", maxSplits: 1).first ?? ""), left) } ?? Int.max
            let rightRank = order.firstIndex { CoreContract.sameAgent(String($0.split(separator: ":", maxSplits: 1).first ?? ""), right) } ?? Int.max
            return leftRank == rightRank ? (CoreContract.agentProviders.firstIndex(of: left) ?? 0) < (CoreContract.agentProviders.firstIndex(of: right) ?? 0) : leftRank < rightRank
        }
    }
    private var quotaColumns: Int { compact ? 1 : workspaceCardColumns }
    private func quotaAccounts(_ provider: String) -> [AgentAccount] {
        displayedQuotaAccounts.filter { CoreContract.sameAgent($0.provider, provider) }.sorted {
            (usageDashboard.quotaOrder?.firstIndex(of: $0.key) ?? Int.max) < (usageDashboard.quotaOrder?.firstIndex(of: $1.key) ?? Int.max)
        }
    }
    // Pack consecutive single-account groups into a shared row, preserving provider order.
    private var quotaProviderRows: [[String]] {
        var rows: [[String]] = []
        for provider in quotaProviders {
            if quotaAccounts(provider).count == 1, let last = rows.last,
               last.count < quotaColumns, last.allSatisfy({ quotaAccounts($0).count == 1 }) {
                rows[rows.count - 1].append(provider)
            } else { rows.append([provider]) }
        }
        return rows
    }
    private func quotaGroup(_ provider: String) -> some View {
        let accounts = quotaAccounts(provider)
        return QuotaGroupSection(key: "quota.group." + scope + "." + provider, model: model, provider: provider,
            accountCount: accounts.count,
            quotas: displayedQuotas.filter { quota in CoreContract.sameAgent(quota.provider, provider) && accounts.contains { $0.key == quota.id } }) {
            BalancedGridLayout(minimumWidth: 1, spacing: 14, maximumColumns: min(quotaColumns, max(1, accounts.count))) {
                ForEach(accounts, id: \.key) { account in
                    if let quota = displayedQuotas.first(where: { $0.id == account.key }) { AccountObservedQuota(model: model, quota: quota, compact: compact, defaultExpanded: false) }
                    else { QuotaAwaitingCard(model: model, account: account, provider: provider) }
                }
            }
        }
    }
    private var quotaSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(L("账户额度")).font(compact ? AppFont.section : AppFont.title)
                RefreshIndicator(model: model, key: "quotas")
                Spacer()
                Button { model.showQuotaOrder?() } label: { Image(systemName: "arrow.up.arrow.down") }.help(L("调整账户顺序")).accessibilityLabel(L("调整账户顺序"))
                QuotaRefreshButton(model: model)
            }
            ForEach(Array(quotaProviderRows.enumerated()), id: \.offset) { _, providers in
                HStack(alignment: .top, spacing: 14) {
                    ForEach(providers, id: \.self) { provider in quotaGroup(provider).frame(maxWidth: .infinity, alignment: .topLeading) }
                    if providers.count < quotaColumns && providers.allSatisfy({ quotaAccounts($0).count == 1 }) {
                        ForEach(0..<(quotaColumns - providers.count), id: \.self) { _ in Color.clear.frame(maxWidth: .infinity).frame(height: 0) }
                    }
                }
            }
            if displayedQuotaAccounts.isEmpty {
                Surface { Text(L("尚未显示账户")).font(AppFont.section); Button(compact && !model.quotaAccounts.isEmpty ? L("选择面板账户") : L("接入账户")) { if compact && !model.quotaAccounts.isEmpty { model.showPanelAccounts?() } else { model.openSettings(.init(section: "accounts")) } } }
            }
            QuotaErrorNotice(model: model)
        }
    }
    private var filteredHosts: [Host] {
        model.settings.monitoredHosts.filter { host in
            if selectedServer != "all" && host.id != selectedServer { return false }
            if serverFilter == "all" { return true }
            if serverFilter == "paused" { return !host.enabled }
            guard host.enabled else { return false }
            guard let result = model.hosts.first(where: { $0.id == host.id }), let sample = result.sample else { return true }
            return result.error != nil || !sample.errors.isEmpty || Date().timeIntervalSince1970 - sample.timestamp > 10
        }
    }
    private var selectedServer: String { model.settings.monitoredHosts.contains { $0.id == serverSelection } ? serverSelection : "all" }
    private func serverButton(_ id: String, title: String) -> some View {
        Button { serverSelection = id } label: {
            Text(title).font(AppFont.secondary).lineLimit(1).padding(.horizontal, 10).padding(.vertical, 7)
                .foregroundStyle(selectedServer == id ? Palette.accent : Color.secondary)
                .background(selectedServer == id ? Palette.accent.opacity(0.16) : Color.clear, in: RoundedRectangle(cornerRadius: 8))
        }.buttonStyle(.plain).help(title).accessibilityLabel(title).accessibilityAddTraits(selectedServer == id ? .isSelected : [])
    }
    private var serverSelector: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 4) {
                serverButton("all", title: L("全部"))
                ForEach(model.settings.monitoredHosts) { host in serverButton(host.id, title: host.name.isEmpty ? host.target : host.name) }
            }
        }.scrollIndicators(.hidden).accessibilityLabel(L("选择服务器"))
            .onChange(of: model.settings.monitoredHosts.map(\.id)) { _, ids in if serverSelection != "all" && !ids.contains(serverSelection) { serverSelection = "all" } }
    }
    @ViewBuilder private var serverContent: some View {
        serverSelector
        HStack { if compact { Text(L("服务器")).font(AppFont.section) }; Text(L("{0} 台主机" , model.settings.monitoredHosts.filter(\.enabled).count)).font(AppFont.secondary).foregroundStyle(.secondary); RefreshIndicator(model: model, key: "hosts"); Spacer(); Button { model.openSettings(.init(category: "servers", section: "add-host")) } label: { Image(systemName: "plus") }.help(L("添加 SSH 主机")).accessibilityLabel(L("添加 SSH 主机")) }
        if model.settings.monitoredHosts.isEmpty { EmptyCard(icon: "server.rack", title: L("添加服务器"), subtitle: "", action: { model.openSettings(.init(category: "servers", section: "add-host")) }) }
        Picker(L("服务器筛选"), selection: $serverFilter) { Text(L("全部")).tag("all"); Text(L("异常")).tag("errors"); Text(L("已暂停")).tag("paused") }.pickerStyle(.segmented)
        if !model.settings.monitoredHosts.isEmpty && filteredHosts.isEmpty {
            Surface { Text(L("当前筛选没有匹配的服务器")).foregroundStyle(.secondary); Button(L("清除筛选")) { serverFilter = "all"; serverSelection = "all" } }
        }
        ForEach(filteredHosts) { host in
            ServerCard(host: host, result: model.hosts.first(where: { $0.id == host.id }), compact: compact, onRefresh: { Task { await model.sampleHosts(hostID: host.id) } }, onSettings: { model.openSettings(.init(category: "servers", hostID: host.id)) }).equatable()
        }
    }
}

private struct SurfaceHeader: View {
    @SurfaceModel var model: AppModel
    @Binding var tab: String
    var compact: Bool, activePage: String
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        HStack(spacing: 12) {
            BrandMark().frame(width: DesignTokens.panelHeadIconSize, height: DesignTokens.panelHeadIconSize)
            Spacer()
            Menu {
                Button(model.refreshLabel("scan")) { Task { await model.scan() } }.disabled(model.busy)
                Button(model.refreshLabel("quotas")) { Task { await model.refreshQuotas() } }.disabled(model.quotaBusy)
                Button(model.refreshLabel("prices")) { Task { await model.syncPrices() } }.disabled(model.busy || model.quotaBusy)
                Button(model.refreshLabel("hosts")) { Task { await model.sampleHosts() } }.disabled(model.hostRefreshBusy())
            } label: {
                Image(systemName: "arrow.clockwise").font(AppFont.secondary)
            }.menuStyle(.borderlessButton).menuIndicator(.hidden).tint(Color.primary).frame(width: DesignTokens.panelHeadIconSize, height: DesignTokens.panelHeadIconSize).help(L("刷新")).accessibilityLabel(L("刷新选项"))
        }.overlay {
            Picker(L("页面"), selection: Binding(get: { activePage }, set: { tab = $0 })) { Text("Agent").tag("agent"); Text(L("服务器")).tag("servers") }
                .pickerStyle(.segmented).labelsHidden().frame(width: DesignTokens.panelTabsContainerWidth)
        }.padding(.horizontal, compact ? DesignTokens.panelChromeInset : 28).padding(.vertical, compact ? DesignTokens.panelHeadPaddingY : 16)
    }
}

private struct SurfaceFooter: View {
    @SurfaceModel var model: AppModel
    var compact: Bool, activePage: String
    private var footerStatus: String { (activePage == "servers" ? model.serverStatusText : model.statusText) + L(" · 出站 · ") + (model.networkBusy ? L("测试中…") : model.networkTest?.label ?? L("连接未测试")) }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: compact ? 3 : 8) {
            // Read-only status (M-15): refreshing has its own button, menu item and ⌘R.
            HStack(spacing: 6) {
                Circle().fill(model.busy || model.quotaBusy || model.serverBusy ? Palette.accent : !model.actionFailures.isEmpty ? Palette.danger : (activePage == "servers" ? model.hasStaleHosts : model.hasStaleUsage) ? Palette.warn : Palette.ok).frame(width: 7, height: 7)
                Text(footerStatus).font(.system(size: compact ? DesignTokens.fontCaption : DesignTokens.fontSecondary)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.tail)
            }.frame(maxWidth: .infinity, alignment: .leading)
                .help(footerStatus + L("\n应用出站测试不代表 SSH 或所有账户可用。\n") + (model.networkTest?.detail ?? L("可从更多菜单测试出站连接")))
                .accessibilityElement(children: .combine)
            // The main window keeps global actions in the menu bar and toolbar; only the panel repeats them.
            if compact { HStack(spacing: 8) {
                Spacer(minLength: 0)
                ThemeMenuButton(model: model)
                UpdateMenuButton()
                Button { model.showDetailPage?(activePage) } label: { Image(systemName: "arrow.up.left.and.arrow.down.right").frame(width: DesignTokens.iconButtonSize, height: DesignTokens.iconButtonSize).contentShape(Rectangle()) }.help(L("打开详情")).accessibilityLabel(L("打开详情"))
                Button { model.openSettings(.init(category: "general")) } label: { Image(systemName: "gearshape").frame(width: DesignTokens.iconButtonSize, height: DesignTokens.iconButtonSize).contentShape(Rectangle()) }.help(L("设置")).accessibilityLabel(L("设置")).keyboardShortcut(",")
                Menu {
                    Button(L("面板显示账户…")) { model.showPanelAccounts?() }; Button(model.isPinned ? L("取消固定面板") : L("固定面板")) { model.isPinned.toggle() }; Divider()
                    Button(model.networkBusy ? L("出站测试中…") : L("测试出站连接")) { Task { await model.testNetwork() } }.disabled(model.networkBusy)
                    Divider()
                    Button(L("退出 Aieyes")) { NSApplication.shared.terminate(nil) }.keyboardShortcut("q")
                } label: { Image(systemName: "ellipsis").frame(width: DesignTokens.iconButtonSize, height: DesignTokens.iconButtonSize).contentShape(Rectangle()) }.menuStyle(.borderlessButton).menuIndicator(.hidden).tint(Color.primary).fixedSize().accessibilityLabel(L("更多操作")).help(L("面板账户、固定、出站测试与退出"))
            } }
            }.buttonStyle(.plain).padding(.horizontal, compact ? DesignTokens.panelChromeInset : 28).padding(.vertical, compact ? DesignTokens.panelFootPaddingY : 10)
    }
}

/// Theme choices (§3.7): following the system stays one choice away instead of being lost to a two-state switch.
struct ThemeMenuButton: View {
    @SurfaceModel var model: AppModel
    @Environment(\.colorScheme) private var colorScheme
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    private var theme: String { model.settings.appearance?.theme ?? "system" }
    private var names: [(String, String)] { [("system", L("跟随系统")), ("light", L("浅色")), ("dark", L("深色"))] }
    var body: some View {
        let _ = interfaceLanguage;
        let label = L("主题：{0}", names.first { $0.0 == theme }?.1 ?? L("跟随系统"))
        Menu {
            Picker(L("主题"), selection: Binding(get: { theme }, set: { value in Task { await model.setTheme(value) } })) {
                ForEach(names, id: \.0) { Text($0.1).tag($0.0) }
            }.pickerStyle(.inline).labelsHidden()
        } label: {
            Image(systemName: theme == "system" ? "circle.lefthalf.filled" : colorScheme == .dark ? "moon" : "sun.max")
                .frame(width: DesignTokens.iconButtonSize, height: DesignTokens.iconButtonSize).contentShape(Rectangle())
        }.menuStyle(.borderlessButton).menuIndicator(.hidden).tint(Color.primary).fixedSize()
            .help(label).accessibilityLabel(label)
            .disabled(!model.settingsLoaded || model.settingsSaving)
    }
}

struct TokenBreakdown: View {
    var tokens: Tokens
    var inline = false
    private var values: [(String, String)] { [(L("普通输入"), Format.compact(tokens.input)), (L("输出"), Format.compact(tokens.output)), (L("缓存读取"), Format.compact(tokens.cacheRead)), (L("缓存写入"), Format.compact(tokens.cacheWrite)), (L("读取命中率"), Format.percent(tokens.cacheRate.map { $0 * 100 }))] }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
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
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 120), alignment: .leading)], alignment: .leading, spacing: 12) {
            ForEach([(L("普通输入"), Format.compact(tokens.input)), (L("输出"), Format.compact(tokens.output)), (L("缓存读取"), Format.compact(tokens.cacheRead)), (L("缓存写入"), Format.compact(tokens.cacheWrite)), (L("读取命中率"), Format.percent(tokens.cacheRate.map { $0 * 100 }))], id: \.0) { label, value in
                VStack(alignment: .leading, spacing: 6) { Text(label).font(AppFont.secondary).foregroundStyle(.secondary); Text(value).font(AppFont.section).monospacedDigit().textSelection(.enabled) }
            }
        }.padding(DesignTokens.statPadding).modifier(NeutralCard()).help(L("读取命中率 = 缓存读取 ÷（普通输入 + 缓存读取 + 缓存写入）"))
    }
}

struct StatCard: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var title: String, value: String, detail: String, icon: String, accent: Color
    var pricingIncomplete = false
    var onPricing: () -> Void = {}
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Text(title).font(.system(size: DesignTokens.fontSecondary, weight: DesignTokens.weightMedium)).foregroundStyle(.secondary)
                if pricingIncomplete {
                    PricingWarning(action: onPricing)
                }
                Spacer(); Image(systemName: icon).font(AppFont.secondary).foregroundStyle(accent) }
            Text(value).contentTransition(.numericText()).font(.system(size: DesignTokens.fontNumber, weight: DesignTokens.weightStrong, design: .default)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.9)
            if !detail.isEmpty { Text(detail).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1) }
        }.padding(DesignTokens.statPadding).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .modifier(NeutralCard())
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.3), value: value)
    }
}
struct EmptyCard: View {
    var icon: String, title: String, subtitle: String, action: () -> Void
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(spacing: 12) { Image(systemName: icon).font(.system(size: 32, weight: .light)).foregroundStyle(Palette.accent); if !subtitle.isEmpty { Text(subtitle).font(AppFont.body).foregroundStyle(.secondary) }; Button(title, action: action).buttonStyle(.aieyesPrimary) }
            .frame(maxWidth: .infinity).padding(.vertical, 36)
    }
}

/// Immutable inputs shared by the plot and daily list; pointer movement never regroups data.
private final class UsageChartData {
    let days: [Aggregate], rows: [DayModel], models: [String], dates: [String], axisDates: [String]
    let grouped: [String: [DayModel]]
    private static let cache: NSCache<NSString, UsageChartData> = { let value = NSCache<NSString, UsageChartData>(); value.countLimit = 8; value.totalCostLimit = 8 * 1024 * 1024; return value }()
    static func get(days: [Aggregate], rows: [DayModel]) -> UsageChartData {
        var hash = Hasher()
        for day in days { hash.combine(day.key); hash.combine(day.total); hash.combine(day.cost) }
        for row in rows { hash.combine(row.day); hash.combine(row.model); hash.combine(row.usage.total); hash.combine(row.usage.cost) }
        let key = String(hash.finalize()) as NSString
        if let known = cache.object(forKey: key), known.days == days && known.rows == rows { return known }
        let value = UsageChartData(days: days, rows: rows)
        cache.setObject(value, forKey: key, cost: max(1, rows.count * 256 + days.count * 128)); return value
    }
    private init(days: [Aggregate], rows: [DayModel]) {
        self.days = days; self.rows = rows
        models = Array(Set(rows.map(\.model))).sorted(); dates = days.map(\.key)
        axisDates = days.enumerated().filter { $0.offset % max(1, days.count / 7) == 0 }.map { $0.element.key }
        grouped = Dictionary(grouping: rows, by: \.day)
    }
}
struct UsageChart: View, Equatable {
    @AppStorage("appearance.chartPalette") private var chartPalette = "classic"
    @AppStorage("appearance.chartContrast") private var chartContrast = "high"
    var days: [Aggregate], rows: [DayModel], cost: Bool
    var modelNames: [String: String] = [:]
    @State private var selected: String?
    @State private var hidden = Set<String>()
    @AppStorage("analysis.modelLegendExpanded") private var legendExpanded = true
    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.days == rhs.days && lhs.rows == rhs.rows && lhs.cost == rhs.cost && lhs.modelNames == rhs.modelNames
    }
    private let data: UsageChartData
    private var models: [String] { data.models }
    init(days: [Aggregate], rows: [DayModel], cost: Bool, modelNames: [String: String] = [:]) {
        self.days = days; self.rows = rows; self.cost = cost; self.modelNames = modelNames
        data = UsageChartData.get(days: days, rows: rows)
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage
        VStack(alignment: .leading, spacing: 8) {
            usageChart
            selectedReading
            modelLegend
        }
    }
    private var visibleRows: [DayModel] { rows.filter { !hidden.contains($0.model) } }
    private var modelColors: [Color] { models.map { Palette.model($0, preset: chartPalette, contrast: chartContrast) } }
    private var usageChart: some View {
        Chart {
            ForEach(visibleRows) { row in usageBar(row) }
            if let selected {
                RuleMark(x: .value(L("所选日期"), selected))
                    .foregroundStyle(Palette.accent)
                    .lineStyle(StrokeStyle(lineWidth: 1, dash: [3]))
            }
        }
        .chartForegroundStyleScale(domain: models, range: modelColors)
        .chartXScale(domain: data.dates)
        .chartLegend(.hidden)
        .chartXSelection(value: Binding(get: { selected }, set: { if selected != $0 { selected = $0 } }))
        .chartXAxis { dateAxis }
        .chartYAxis { usageAxis }
        .chartOverlay { proxy in hoverOverlay(proxy) }
        .frame(height: DesignTokens.chartHeight)
        .focusable()
        .onKeyPress(.leftArrow) { move(-1); return .handled }
        .onKeyPress(.rightArrow) { move(1); return .handled }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(L("每日用量"))
        .accessibilityHint(L("左右方向键选择日期，下方提供精确读数"))
    }
    private func usageBar(_ row: DayModel) -> some ChartContent {
        let amount = cost ? row.usage.cost : row.usage.total
        let reading = cost ? Format.money(amount) + " USD" : Format.compact(amount) + " Token"
        return BarMark(x: .value(L("日期"), row.day), y: .value(cost ? "USD" : "Token", amount))
            .foregroundStyle(by: .value(L("模型"), row.model))
            .accessibilityLabel(Text(modelNames[row.model] ?? row.model))
            .accessibilityValue(Text(row.day + " · " + reading))
    }
    private var dateAxis: some AxisContent {
        AxisMarks(values: data.axisDates) { value in
            AxisValueLabel {
                if let label = value.as(String.self) { Text(String(label.suffix(5))).font(AppFont.secondary) }
            }
        }
    }
    private var usageAxis: some AxisContent {
        AxisMarks(position: .leading, values: .automatic(desiredCount: 3)) { value in
            AxisGridLine().foregroundStyle(.primary.opacity(DesignTokens.chartGridOpacity))
            AxisValueLabel {
                if let amount = value.as(Double.self) { Text(cost ? Format.money(amount) : Format.compact(amount)).font(AppFont.secondary) }
            }
        }
    }
    private func hoverOverlay(_ proxy: ChartProxy) -> some View {
        GeometryReader { geometry in
            Color.clear.contentShape(Rectangle()).onContinuousHover { phase in
                if case .active(let point) = phase, let frame = proxy.plotFrame {
                    let next = proxy.value(atX: point.x - geometry[frame].minX, as: String.self)
                    if selected != next { selected = next }
                }
            }
        }
    }
    private var selectedReading: some View {
        Group {
            if let day = days.first(where: { $0.key == selected }) {
                Text(day.key + " · " + (cost ? String(format: "%.2f USD", day.cost) : Format.compact(day.total) + " Token"))
                    .font(AppFont.secondary).monospacedDigit().textSelection(.enabled)
            }
        }.frame(height: DesignTokens.chartReadingHeight, alignment: .leading)
    }
    private var modelLegend: some View {
        DisclosureGroup(L("模型图例"), isExpanded: $legendExpanded) {
            FlowLayout { ForEach(models, id: \.self) { name in legendButton(name) } }
        }.font(AppFont.secondary)
    }
    private func legendButton(_ name: String) -> some View {
        let title = modelNames[name] ?? name
        let isHidden = hidden.contains(name)
        return Button {
            if isHidden { hidden.remove(name) } else { hidden.insert(name) }
        } label: {
            HStack(spacing: 6) {
                Circle().fill(Palette.model(name, preset: chartPalette, contrast: chartContrast))
                    .frame(width: DesignTokens.chartLegendDot, height: DesignTokens.chartLegendDot)
                Label(title, systemImage: isHidden ? "eye.slash" : "eye").strikethrough(isHidden)
            }.font(AppFont.secondary)
        }.buttonStyle(.plain).help(title)
            .accessibilityLabel((isHidden ? L("显示模型 ") : L("隐藏模型 ")) + title)
    }
    private func move(_ offset: Int) { guard !days.isEmpty else { return }; let index = selected.flatMap { key in days.firstIndex { $0.key == key } } ?? 0; selected = days[min(days.count - 1, max(0, index + offset))].key }
}

struct DailyUsage: View, Equatable {
    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.days == rhs.days && lhs.rows == rhs.rows && lhs.compact == rhs.compact && lhs.cost == rhs.cost && lhs.modelNames == rhs.modelNames
    }
    var days: [Aggregate], rows: [DayModel], compact: Bool
    var cost = false
    var modelNames: [String: String] = [:]
    private let data: UsageChartData
    init(days: [Aggregate], rows: [DayModel], compact: Bool, cost: Bool = false, modelNames: [String: String] = [:]) {
        self.days = days; self.rows = rows; self.compact = compact; self.cost = cost; self.modelNames = modelNames
        data = UsageChartData.get(days: days, rows: rows)
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        let grouped = data.grouped
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 10) {
                ForEach(days.reversed()) { day in DailyUsageDay(day: day, rows: grouped[day.key] ?? [], cost: cost, modelNames: modelNames) }
            }
        }.frame(height: min(400, CGFloat(days.count) * 38 + 10))
    }
}
private struct DailyUsageDay: View {
    let day: Aggregate, rows: [DayModel], cost: Bool
    let modelNames: [String: String]
    @State private var expanded = false
    private func exact(_ usage: Aggregate) -> String { String(format: "%.0f Token", usage.total) + " · " + (usage.costAvailable ? String(format: "%.6f USD", usage.cost) : L("API 等价成本 —")) + (usage.hasDisplayCredits ? " · " + usage.nativeCreditText : "") + L(" · 读取命中率 ") + Format.percent(usage.tokens.cacheRate.map { $0 * 100 }) }
    private func display(_ usage: Aggregate) -> String { Format.compact(usage.total) + " Token · " + (usage.costAvailable ? String(format: "%.6f USD", usage.cost) : L("API 等价成本 —")) + L(" · 读取命中率 ") + Format.percent(usage.tokens.cacheRate.map { $0 * 100 }) }
    private func modelRow(_ row: DayModel, day: String) -> some View {
        let value = exact(row.usage)
        let label = day + " · " + (modelNames[row.model] ?? row.model) + " · " + value
        return VStack(alignment: .leading, spacing: 4) {
            Text(modelNames[row.model] ?? row.model).fontWeight(.medium).textSelection(.enabled)
            Text(display(row.usage)).help(value).monospacedDigit().textSelection(.enabled)
            if row.usage.hasDisplayCredits { Text(row.usage.nativeCreditText).textSelection(.enabled) }
        }.font(AppFont.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 5)
            .accessibilityElement(children: .combine).accessibilityLabel(label)
            .contextMenu { Button(L("复制模型与精确数值")) {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(day + "\t" + (modelNames[row.model] ?? row.model) + "\t" + value, forType: .string)
            } }
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        DisclosureGroup(isExpanded: $expanded) {
            if expanded {
                Text(display(day)).help(exact(day)).font(AppFont.secondary).monospacedDigit().textSelection(.enabled)
                ForEach(rows) { row in modelRow(row, day: day.key) }
                if day.hasDisplayCredits { Text(day.nativeCreditText).font(AppFont.secondary).textSelection(.enabled) }
                if day.total == 0 && !day.hasNativeCredits { Text(L("当日用量为零")).font(AppFont.secondary).foregroundStyle(.secondary) }
            }
        } label: { HStack { Text(day.key); Spacer(); Text(cost ? Format.money(day.cost) + " USD" : Format.compact(day.total) + " Token").monospacedDigit() }.font(AppFont.secondary) }
    }
}
private struct YearlyHeatmaps: View {
    let days: [Aggregate], cost: Bool
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        let grouped = Dictionary(grouping: days) { String($0.key.prefix(4)) }
        let years = grouped.keys.sorted(by: >)
        LazyVStack(alignment: .leading, spacing: 12) {
            ForEach(years, id: \.self) { year in YearHeatmap(year: year, days: grouped[year] ?? [], cost: cost, initiallyExpanded: year == years.first) }
        }
    }
}
private struct YearHeatmap: View {
    let year: String, days: [Aggregate], cost: Bool
    @State private var expanded: Bool
    init(year: String, days: [Aggregate], cost: Bool, initiallyExpanded: Bool) { self.year = year; self.days = days; self.cost = cost; _expanded = State(initialValue: initiallyExpanded) }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage; DisclosureGroup(year, isExpanded: $expanded) { if expanded { Heatmap(days: days, cost: cost).equatable() } } }
}

struct ModelChart: View, Equatable {
    @AppStorage("appearance.chartPalette") private var chartPalette = "classic"
    @AppStorage("appearance.chartContrast") private var chartContrast = "high"
    var models: [Aggregate], cost: Bool
    var modelNames: [String: String] = [:]
    @State private var showAll = false
    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.models == rhs.models && lhs.cost == rhs.cost && lhs.modelNames == rhs.modelNames
    }
    private var ranked: [Aggregate] { models.sorted { value($0) == value($1) ? $0.key < $1.key : value($0) > value($1) } }
    private func value(_ item: Aggregate) -> Double { cost ? item.cost : item.total }
    private var total: Double { models.reduce(0) { $0 + value($1) } }
    private var slices: [Aggregate] {
        let remainder = Array(ranked.dropFirst(5))
        guard !remainder.isEmpty else { return ranked }
        return Array(ranked.prefix(5)) + [Aggregate(key: L("其他"), total: remainder.reduce(0) { $0 + $1.total }, cost: remainder.reduce(0) { $0 + $1.cost })]
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        let ringKeys = Set(ranked.prefix(5).map(\.key))
        HStack(alignment: .top, spacing: 16) {
            Chart(slices) { item in SectorMark(angle: .value(cost ? "USD" : "Token", value(item)), innerRadius: .ratio(DesignTokens.modelChartInnerRatio), angularInset: DesignTokens.modelChartAngularInset).foregroundStyle(Palette.model(item.key, preset: chartPalette, contrast: chartContrast)).cornerRadius(DesignTokens.modelChartCorner).accessibilityLabel(Text(modelNames[item.key] ?? item.key)) }.frame(width: DesignTokens.modelChartSize, height: DesignTokens.modelChartHeight)
                .chartBackground { _ in VStack { Text("\(models.count)").font(.title2); Text(L("模型")).font(AppFont.secondary) } }.accessibilityLabel(L("前五模型与其他；完整数值见列表"))
            VStack(alignment: .leading, spacing: 10) {
                ForEach(showAll ? ranked : Array(ranked.prefix(5))) { item in
                    // Models past the top five appear in the ring as its "其他" slice.
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Circle().fill(Palette.model(ringKeys.contains(item.key) ? item.key : L("其他"), preset: chartPalette, contrast: chartContrast)).frame(width: DesignTokens.chartLegendDot, height: DesignTokens.chartLegendDot)
                            .alignmentGuide(.firstTextBaseline) { d in d[VerticalAlignment.center] + DesignTokens.fontSecondary * 0.35 }
                        VStack(alignment: .leading, spacing: 3) { Text(modelNames[item.key] ?? item.key).font(AppFont.secondary).textSelection(.enabled).help(modelNames[item.key] ?? item.key); Text((cost ? Format.money(item.cost) + " USD" : Format.compact(item.total) + " Token") + " · " + Format.percent(total > 0 ? value(item) / total * 100 : 0)).font(AppFont.secondary).monospacedDigit().foregroundStyle(.secondary) }
                    }
                        .contextMenu { Button(L("复制模型与精确数值")) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString((modelNames[item.key] ?? item.key) + "\t" + String(item.total) + " Token\t" + String(item.cost) + " USD", forType: .string) } }
                }
                if ranked.count > 5 { Button(showAll ? L("收起其他模型") : L("展开其他 {0} 个模型" , ranked.count - 5)) { showAll.toggle() }.font(AppFont.secondary) }
            }.frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

@MainActor private final class HeatmapLayout {
    let days: [Aggregate], cost: Bool, leading: Int, maximum: Double, labels: [String: String]
    private static let cache: NSCache<NSString, HeatmapLayout> = { let value = NSCache<NSString, HeatmapLayout>(); value.totalCostLimit = 4 * 1024 * 1024; value.countLimit = 12; return value }()
    static func get(days: [Aggregate], cost: Bool) -> HeatmapLayout {
        var hash = Hasher(); hash.combine(cost); hash.combine(TimeZone.current.identifier); hash.combine(Locale.current.identifier)
        for day in days { hash.combine(day.key); hash.combine(day.total); hash.combine(day.cost); hash.combine(day.tokens.input); hash.combine(day.tokens.output); hash.combine(day.tokens.cacheRead); hash.combine(day.tokens.cacheWrite) }
        let key = String(hash.finalize()) as NSString
        if let known = cache.object(forKey: key), known.days == days && known.cost == cost { return known }
        let value = HeatmapLayout(days: days, cost: cost)
        cache.setObject(value, forKey: key, cost: max(1, days.count * 256)); return value
    }
    private init(days: [Aggregate], cost: Bool) {
        self.days = days; self.cost = cost
        var calendar = Calendar(identifier: .gregorian); calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        let parts = days.first?.key.split(separator: "-").compactMap { Int($0) } ?? []
        let date = parts.count == 3 ? calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2], hour: 12)) : nil
        leading = date.map { (Calendar.current.component(.weekday, from: $0) + 5) % 7 } ?? 0
        maximum = max(1, days.map { cost ? $0.cost : $0.total }.max() ?? 1)
        labels = Dictionary(days.map { ($0.key, $0.key + " · " + (cost ? String(format: "%.6f USD", $0.cost) : Format.compact($0.total) + " Token")) }, uniquingKeysWith: { _, new in new })
    }
}
struct Heatmap: View, Equatable {
    var days: [Aggregate], cost: Bool
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.days == rhs.days && lhs.cost == rhs.cost }
    @State private var selected = 0
    @State private var showDay = false
    private var layout: HeatmapLayout { HeatmapLayout.get(days: days, cost: cost) }
    private func label(_ day: Aggregate) -> String { layout.labels[day.key] ?? day.key }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        let cached = layout
        let leading = cached.leading, maximum = cached.maximum
        VStack(alignment: .leading, spacing: 10) {
            GeometryReader { proxy in
                let columns = max(1, (days.count + leading + 6) / 7), step = (proxy.size.width - 22) / CGFloat(columns), size = max(DesignTokens.heatCellMin, min(DesignTokens.heatCellMax, step - DesignTokens.heatCellGap))
                VStack(alignment: .leading, spacing: 6) {
                    ZStack(alignment: .topLeading) { ForEach(Array(days.enumerated().filter { $0.offset == 0 || $0.element.key.hasSuffix("-01") }), id: \.element.key) { index, day in Text(AieyesLocale.month(Int(day.key.dropFirst(5).prefix(2)) ?? 1)).font(.system(size: DesignTokens.fontHint)).offset(x: 22 + CGFloat((index + leading) / 7) * step) } }.frame(height: 15).accessibilityHidden(true)
                    HStack(alignment: .top, spacing: 3) {
                        VStack(spacing: 3) { ForEach(0..<7) { row in Text(row.isMultiple(of: 2) ? AieyesLocale.weekday(row) : "").font(.system(size: DesignTokens.fontHint)).frame(width: 16, height: size) } }.accessibilityHidden(true)
                        ForEach(0..<columns, id: \.self) { column in VStack(spacing: 3) {
                            ForEach(0..<7) { row in
                                let index = column * 7 + row - leading
                                if days.indices.contains(index) {
                                    let day = days[index], value = cost ? day.cost : day.total
                                    RoundedRectangle(cornerRadius: DesignTokens.heatCellRadius).fill(value == 0 ? Color.primary.opacity(DesignTokens.heatEmptyOpacity) : Palette.accent.opacity(0.25 + 0.75 * sqrt(value / maximum))).frame(width: size, height: size)
                                        .overlay { if selected == index { RoundedRectangle(cornerRadius: DesignTokens.heatCellRadius).strokeBorder(Color.primary, lineWidth: 1.5) } }
                                        .help(cached.labels[day.key] ?? day.key).accessibilityLabel(cached.labels[day.key] ?? day.key).accessibilityAddTraits(.isButton).accessibilityAction { selected = index; showDay = true }.onTapGesture { selected = index; showDay = true }
                                } else { Color.clear.frame(width: size, height: size) }
                            }
                        }.frame(width: step - 3) }
                    }
                }
            }.frame(height: 130).focusable()
                .onKeyPress(.leftArrow) { move(-7); return .handled }.onKeyPress(.rightArrow) { move(7); return .handled }
                .onKeyPress(.upArrow) { move(-1); return .handled }.onKeyPress(.downArrow) { move(1); return .handled }
                .onKeyPress(.return) { showDay = true; return .handled }
                .accessibilityElement(children: .contain).accessibilityLabel(L("年度用量")).accessibilityHint(L("方向键选择日期，回车查看当天详情"))
            if days.indices.contains(selected) { Text(label(days[selected])).font(AppFont.secondary).textSelection(.enabled) }
            HStack { Text(L("零")); ForEach(0..<5) { n in RoundedRectangle(cornerRadius: DesignTokens.heatCellRadius).fill(n == 0 ? Color.primary.opacity(DesignTokens.heatEmptyOpacity) : Palette.accent.opacity(0.25 + Double(n) * 0.1875)).frame(width: 12, height: 12) }; Text(L("多")) }.font(AppFont.secondary).foregroundStyle(.secondary)
            DisclosureGroup(L("年度数据表 · 精确值")) {
                Table(days) {
                    TableColumn(L("日期"), value: \.key)
                    TableColumn("Token") { day in Text(Format.compact(day.total)).help(String(format: "%.0f Token", day.total)).textSelection(.enabled) }
                    TableColumn(L("API 等价成本 USD")) { day in Text(String(format: "%.6f", day.cost)).textSelection(.enabled) }
                }.frame(height: 260)
            }.font(AppFont.secondary)
        }.sheet(isPresented: $showDay) { if days.indices.contains(selected) { let day = days[selected]; VStack(alignment: .leading, spacing: 14) { Text(day.key).font(AppFont.title); Text(label(day)).textSelection(.enabled); CacheSummaryCard(tokens: day.tokens); Button(L("关闭")) { showDay = false }.keyboardShortcut(.cancelAction) }.padding(DesignTokens.dialogPadding).frame(width: 520) } }
    }
    private func move(_ offset: Int) { selected = min(max(0, days.count - 1), max(0, selected + offset)) }
}

private struct QuotaRefreshButton: View {
    @SurfaceModel var model: AppModel
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage; Button(model.quotaBusy ? L("读取中…") : L("刷新额度")) { Task { await model.refreshQuotas() } }.font(AppFont.caption).disabled(model.quotaBusy) }
}
private struct QuotaErrorNotice: View {
    @SurfaceModel var model: AppModel
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage; if let error = model.quotaError { Label(L(error), systemImage: "exclamationmark.triangle").font(AppFont.caption).foregroundStyle(Palette.warn).textSelection(.enabled) } }
}
private struct QuotaAwaitingCard: View {
    @SurfaceModel var model: AppModel
    let account: AgentAccount, provider: String
    @Environment(\.collapsedQuotaHeight) private var collapsedHeight
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    private var labels: some View {
        VStack(alignment: .leading, spacing: DesignTokens.spaceCardContent) {
            Text(account.name).font(AppFont.section)
            Text(model.quotaBusy ? L("正在读取账户额度…") : L("等待首次额度查询")).font(AppFont.secondary).foregroundStyle(.secondary)
        }
    }
    var body: some View {
        let _ = interfaceLanguage
        Surface(tint: Palette.category(provider), minimumHeight: collapsedHeight) { labels }
            .background { GeometryReader { proxy in
                Surface(tint: Palette.category(provider)) { labels }.frame(width: proxy.size.width)
                    .fixedSize(horizontal: false, vertical: true)
                    .background(GeometryReader { measured in Color.clear.preference(key: CollapsedQuotaMeasurements.self, value: [provider + ":" + account.id: ceil(measured.size.height)]) })
                    .hidden().accessibilityHidden(true).allowsHitTesting(false)
            } }
    }
}

struct CollapsedQuotaHeight: EnvironmentKey { static let defaultValue: CGFloat = 0 }
struct CollapsedQuotaMeasurements: PreferenceKey {
    static let defaultValue: [String: CGFloat] = [:]
    static func reduce(value: inout [String: CGFloat], nextValue: () -> [String: CGFloat]) { value.merge(nextValue(), uniquingKeysWith: max) }
}
extension EnvironmentValues {
    var collapsedQuotaHeight: CGFloat {
        get { self[CollapsedQuotaHeight.self] }
        set { self[CollapsedQuotaHeight.self] = newValue }
    }
}

private struct QuotaGroupSection<Content: View>: View {
    let model: AppModel
    let provider: String
    let accountCount: Int
    let quotas: [Quota]
    let content: Content
    @AppStorage private var expanded: Bool
    @State private var collapsedHeight: CGFloat = 0
    @State private var showingFailures = false
    @State private var openingSettings = false
    @FocusState private var headerFocused: Bool
    @FocusState private var failureButtonFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var failures: [Quota] { quotas.filter { $0.queryFailure != nil } }

    init(key: String, model: AppModel, provider: String, accountCount: Int, quotas: [Quota], @ViewBuilder content: () -> Content) {
        self.model = model; self.provider = provider; self.accountCount = accountCount; self.quotas = quotas; self.content = content()
        _expanded = AppStorage(wrappedValue: true, key)
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 7) {
                Button { expanded.toggle() } label: {
                    HStack(spacing: 7) {
                        Image(systemName: expanded ? "chevron.down" : "chevron.right").font(AppFont.caption)
                        Circle().fill(Palette.category(provider)).frame(width: 7, height: 7)
                        Text(Format.provider(provider)).font(AppFont.secondary.weight(.medium))
                        Spacer(minLength: 0)
                        Text(L("{0} 个账户" , accountCount)).font(AppFont.caption).foregroundStyle(.secondary)
                    }.contentShape(Rectangle())
                }.buttonStyle(.plain).focused($headerFocused)
                    .accessibilityLabel(Format.provider(provider) + L("，{0} 个账户，" , accountCount) + (expanded ? L("折叠") : L("展开")))
                if !failures.isEmpty || showingFailures {
                    Button { openingSettings = false; showingFailures.toggle() } label: {
                        Label(L("{0} 个账户查询失败" , failures.count), systemImage: "exclamationmark.triangle").font(AppFont.caption).foregroundStyle(Palette.warn)
                    }.buttonStyle(.plain).focused($failureButtonFocused).help(L("查看失败原因与重试"))
                        .opacity(failures.isEmpty ? 0 : 1).disabled(failures.isEmpty).accessibilityHidden(failures.isEmpty)
                        .popover(isPresented: $showingFailures) { failureList }
                }
            }
            if expanded {
                content.padding(.top, 8).environment(\.collapsedQuotaHeight, collapsedHeight)
                    .onPreferenceChange(CollapsedQuotaMeasurements.self) { values in
                        let next = values.values.max() ?? 0
                        if abs(collapsedHeight - next) > 0.5 { collapsedHeight = next }
                    }
            }
        }
        .animation(reduceMotion ? nil : .spring(response: 0.24, dampingFraction: 0.9), value: expanded)
        .onChange(of: showingFailures) { _, showing in
            if !showing && !openingSettings {
                if failures.isEmpty { headerFocused = true } else { failureButtonFocused = true }
            }
        }
    }
    private var failureList: some View {
        QuotaFailureList(model: model, provider: provider, failures: failures, close: { showingFailures = false }, openSettings: { quota in
            openingSettings = true; showingFailures = false
            model.openSettings(.init(provider: provider, accountKey: quota.id, section: "accounts"))
        })
    }
}
private struct QuotaFailureList: View {
    @SurfaceModel var model: AppModel
    let provider: String
    let failures: [Quota]
    let close: () -> Void
    let openSettings: (Quota) -> Void
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(Format.provider(provider) + L(" · 额度查询失败")).font(AppFont.section)
                Spacer()
                Button(L("关闭"), action: close).keyboardShortcut(.cancelAction)
            }
            if failures.isEmpty { Text(L("当前没有查询失败的账户")).font(AppFont.secondary) }
            else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        ForEach(failures) { quota in
                            VStack(alignment: .leading, spacing: 8) {
                                let reason = UserFacingError(quota.queryFailure ?? "", context: L("额度查询失败"))
                                Text(quota.name).font(AppFont.section).textSelection(.enabled)
                                Text(L(reason.details ?? reason.summary)).font(AppFont.secondary).textSelection(.enabled)
                                    .fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, alignment: .leading)
                                HStack {
                                    Button(model.quotaBusy ? L("查询中…") : L("重试查询")) { Task { await model.refreshQuotas(accountID: quota.accountId) } }.disabled(model.quotaBusy)
                                    Button(L("账户设置")) {
                                        openSettings(quota)
                                    }
                                }
                            }.id(quota.id)
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }.frame(maxHeight: 300)
            }
        }.padding(16).frame(width: 350)
    }
}

private struct AccountObservedQuota: View {
    @SurfaceModel var model: AppModel
    @ObservedObject var status: AccountStatusProjection
    let quota: Quota
    var compact = false
    var defaultExpanded = false
    init(model: AppModel, quota: Quota, compact: Bool = false, defaultExpanded: Bool = false) {
        _model = SurfaceModel(wrappedValue: model); self.quota = quota; self.compact = compact; self.defaultExpanded = defaultExpanded
        status = model.accountStatusStore.projection(quota.id)
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        QuotaCard(quota: quota, compact: compact, collapsible: true, defaultExpanded: defaultExpanded,
            estimate: model.dashboard.quotaEstimates?.first { $0.accountKey == quota.id },
            deviceCounts: model.accountDeviceCounts(quota.id),
            onSwitch: model.canSwitchAccount(quota.id) ? { model.showAccountSwitch?(quota.id) } : nil,
            onEstimate: { group in model.showEstimate?(quota, false, group) },
            onCredits: { model.showEstimate?(quota, true, nil) },
            creditEstimate: model.dashboard.creditEstimates?.first { $0.accountKey == quota.id },
            quotaHistory: model.dashboard.presentationEstimates(account: quota.id),
            creditHistory: model.dashboard.presentationEstimates(account: quota.id, credits: true),
            onAnalysis: { model.openAnalysis(from: compact ? "panel" : "overview", account: quota.id, provider: quota.provider) },
            onRetry: { Task { await model.refreshQuotas(accountID: quota.accountId) } },
            actionOwner: ObjectIdentifier(model)).equatable()
    }
}

struct QuotaCard: View, Equatable {
    var quota: Quota
    var compact = false
    var collapsible = false
    var sourceName = ""
    var estimate: QuotaEstimate?
    var deviceCounts: AccountDeviceCounts?
    var onSwitch: (() -> Void)?
    var onEstimate: ((String?) -> Void)?
    var onCredits: (() -> Void)?
    var creditEstimate: QuotaEstimate?
    var quotaHistory: [QuotaEstimate] = []
    var creditHistory: [QuotaEstimate] = []
    var onAnalysis: (() -> Void)?
    var onRetry: (() -> Void)?
    private var actionOwner: ObjectIdentifier?
    static func == (lhs: Self, rhs: Self) -> Bool {
        // Opt in only where actions capture the same retained model and quota.
        guard let owner = lhs.actionOwner, owner == rhs.actionOwner else { return false }
        return lhs.quota == rhs.quota && lhs.compact == rhs.compact && lhs.collapsible == rhs.collapsible
            && lhs.defaultExpanded == rhs.defaultExpanded && lhs.sourceName == rhs.sourceName
            && lhs.estimate == rhs.estimate && lhs.creditEstimate == rhs.creditEstimate
            && lhs.quotaHistory == rhs.quotaHistory && lhs.creditHistory == rhs.creditHistory
            && lhs.deviceCounts == rhs.deviceCounts && (lhs.onSwitch == nil) == (rhs.onSwitch == nil)
            && (lhs.onEstimate == nil) == (rhs.onEstimate == nil) && (lhs.onCredits == nil) == (rhs.onCredits == nil)
            && (lhs.onAnalysis == nil) == (rhs.onAnalysis == nil) && (lhs.onRetry == nil) == (rhs.onRetry == nil)
    }
    private var bestCredit: QuotaEstimate? { EstimatePresentation.credit(creditHistory) ?? EstimatePresentation.canonical([creditEstimate].compactMap { $0 }).first }
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var defaultExpanded: Bool
    @AppStorage private var storedExpanded: Bool?
    private var isMeasurement = false
    @Environment(\.collapsedQuotaHeight) private var collapsedHeight
    private var measuring: QuotaCard { var copy = self; copy.isMeasurement = true; return copy }
    private var expanded: Bool { !isMeasurement && (!collapsible || (storedExpanded ?? defaultExpanded)) }
    init(quota: Quota, compact: Bool = false, collapsible: Bool = false, defaultExpanded: Bool = false, sourceName: String = "", estimate: QuotaEstimate? = nil, deviceCounts: AccountDeviceCounts? = nil, onSwitch: (() -> Void)? = nil, onEstimate: ((String?) -> Void)? = nil, onCredits: (() -> Void)? = nil, creditEstimate: QuotaEstimate? = nil, quotaHistory: [QuotaEstimate] = [], creditHistory: [QuotaEstimate] = [], onAnalysis: (() -> Void)? = nil, onRetry: (() -> Void)? = nil, actionOwner: ObjectIdentifier? = nil) {
        self.actionOwner = actionOwner
        self.defaultExpanded = defaultExpanded; self.onAnalysis = onAnalysis; self.onRetry = onRetry
        self.deviceCounts = deviceCounts; self.onSwitch = onSwitch
        self.quotaHistory = quotaHistory; self.creditHistory = creditHistory
        self.collapsible = collapsible; self.quota = quota; self.compact = compact; self.sourceName = sourceName; self.estimate = estimate; self.onEstimate = onEstimate; self.onCredits = onCredits; self.creditEstimate = creditEstimate
        _storedExpanded = AppStorage(NativePanelKey.expansion(.quota, id: quota.id, compact: compact))
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        if isMeasurement {
            fullCard.fixedSize(horizontal: false, vertical: true)
                .background(GeometryReader { proxy in Color.clear.preference(key: CollapsedQuotaMeasurements.self, value: [quota.id: ceil(proxy.size.height)]) })
        } else {
            fullCard
                .background(alignment: .topLeading) {
                    if collapsible { GeometryReader { proxy in
                        AnyView(measuring).frame(width: proxy.size.width).fixedSize(horizontal: false, vertical: true)
                            .hidden().accessibilityHidden(true).allowsHitTesting(false)
                    } }
                }
        }
    }
    @ViewBuilder private var balanceContent: some View {
        if let wallet = quota.extraUsage {
            VStack(alignment: .leading, spacing: 8) {
                ViewThatFits(in: .horizontal) {
                    HStack { Text(L("额外用量余额")).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(); Spacer(); Text(wallet.amount(wallet.balance)).font(AppFont.section).monospacedDigit().fixedSize() }
                    VStack(alignment: .leading, spacing: 4) { Text(L("额外用量余额")).font(AppFont.secondary).foregroundStyle(.secondary); Text(wallet.amount(wallet.balance)).font(AppFont.section).monospacedDigit().fixedSize(horizontal: false, vertical: true) }
                }
                if expanded {
                    Text(L("钱包总额 ") + wallet.amount(wallet.total)).font(AppFont.secondary).foregroundStyle(.secondary)
                    Text(L("本月扣费 ") + wallet.amount(wallet.monthlyUsed)).font(AppFont.secondary).foregroundStyle(.secondary)
                    Text(wallet.monthlyLimitLabel).font(AppFont.secondary).foregroundStyle(.secondary)
                    HelpNote(title: L("额外用量"), text: L("钱包与订阅额度分别计算"))
                }
            }
        }
        if let balances = quota.balances { ForEach(balances) { balance in
            VStack(alignment: .leading, spacing: 8) {
                HStack { Text(L("可用余额")).font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Text(balance.currency + " " + balance.total).font(AppFont.section).monospacedDigit() }
                if expanded { HStack { Text(L("赠送 ") + balance.granted); Spacer(); Text(L("充值 ") + balance.toppedUp) }.font(AppFont.secondary).foregroundStyle(.secondary) }
            }
        } }
        if quota.isAvailable == false { Text(L("余额不足")).font(AppFont.secondary).foregroundStyle(Palette.warn) }
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
                    Text(group).font(.system(size: DesignTokens.fontSecondary, weight: DesignTokens.weightMedium)).fixedSize(horizontal: false, vertical: true)
                    HStack(alignment: .top, spacing: 14) {
                        ForEach(quota.windows.filter { $0.groupLabel == group }.sorted { ($0.windowMinutes ?? 0) < ($1.windowMinutes ?? 0) }) { window in
                            VStack(alignment: .leading, spacing: 5) {
                                HStack { Text(window.windowMinutes == CoreContract.weekMinutes ? "7d" : window.windowMinutes == CoreContract.fiveHourMinutes ? "5h" : L(window.name)); Spacer(minLength: 2); Text(Format.percent(max(0, 100-window.usedPercent))).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent)) }.font(AppFont.secondary)
                                ResourceBar(percent: 100-window.usedPercent, tint: Palette.quota(window.usedPercent))
                                QuotaResetLabel(window: window)
                            }.frame(maxWidth: .infinity).accessibilityElement(children: .combine).accessibilityLabel(L(window.name) + L("，剩余 ") + Format.percent(max(0, 100-window.usedPercent)))
                        }
                    }
                    if expanded, let onEstimate {
                        let groupId = quota.windows.first { $0.groupLabel == group }?.groupId ?? group
                        EstimateEntry(records: quotaHistory.filter { $0.groupId == groupId }, quota: quota, group: groupId) { onEstimate(groupId) }
                    }
                }
            }
        }
    }
    private var headerAccessibility: String {
        var text = Format.provider(quota.provider) + "，" + quota.name
        if let email = quota.identity?.email, email != quota.name { text += "，" + email }
        if let plan = quota.plan { text += L("，订阅 ") + Format.subscription(plan) }
        if !expanded && quota.provider == "codex" { text += "，" + Format.creditBalance(quota.credits, estimate: bestCredit) + "，Bank Reset " + (quota.bankReset.map { String($0.availableCount) } ?? L("未知")) + L(" 次可用") }
        text += expanded ? L("，折叠额度") : L("，展开额度")
        return text
    }
    @ViewBuilder private var cardHeader: some View {
            Button { withAnimation(reduceMotion ? nil : .spring(response: 0.24, dampingFraction: 0.86)) { if collapsible { storedExpanded = !expanded } } } label: {
            HStack(spacing: 8) {
                ProviderMark(provider: quota.provider)
                VStack(alignment: .leading, spacing: 2) {
                    // Long names stop at two lines; the full text stays in the tooltip and the accessible name.
                    Text(quota.name).font(.system(size: DesignTokens.fontSecondary, weight: DesignTokens.weightStrong)).lineLimit(2).truncationMode(.tail).fixedSize(horizontal: false, vertical: true).textSelection(.enabled).help(quota.name)
                    if let email = quota.identity?.email, email != quota.name {
                        Text(email).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle).textSelection(.enabled).help(email)
                    }
                }.frame(minWidth: 0, alignment: .leading)
                if let plan = quota.plan, !Format.subscription(plan).isEmpty { SubscriptionBadge(plan: plan).frame(maxWidth: DesignTokens.quotaBadgeWidth) }
                Spacer(minLength: 0)
                if collapsible { Image(systemName: expanded ? "chevron.up" : "chevron.down").font(.system(size: DesignTokens.fontSecondary, weight: DesignTokens.weightStrong)).foregroundStyle(.secondary) }
            }.contentShape(Rectangle())
            }.buttonStyle(.plain).help(collapsible ? (expanded ? L("折叠额度") : L("展开额度")) : quota.name)
                .accessibilityLabel(headerAccessibility)
            if expanded, let onSwitch {
                FlowLayout(spacing: 8) {
                    Button(action: onSwitch) {
                        Label(L("切换账户"), systemImage: "arrow.left.arrow.right")
                            .font(AppFont.secondary.weight(.medium)).foregroundStyle(Palette.accent)
                            .lineLimit(1).fixedSize(horizontal: true, vertical: false)
                            .frame(minWidth: 124)
                            .padding(.horizontal, 10).padding(.vertical, 6)
                            .background(Palette.accent.opacity(0.10), in: RoundedRectangle(cornerRadius: DesignTokens.radiusAccountSwitch))
                            .overlay { RoundedRectangle(cornerRadius: DesignTokens.radiusAccountSwitch).strokeBorder(Palette.accent.opacity(0.30), lineWidth: 1) }
                            .contentShape(RoundedRectangle(cornerRadius: DesignTokens.radiusAccountSwitch))
                    }.buttonStyle(.plain)
                    if let deviceCounts {
                        Text(deviceCounts.inlineSummary).font(AppFont.secondary).foregroundStyle(.secondary)
                            .monospacedDigit().lineLimit(1).fixedSize(horizontal: true, vertical: false).help(deviceCounts.summary)
                    }
                }
            }
            if expanded {
                HStack { Spacer(); if quota.updatedAt > 0 { Text("\(quota.origin == "log" ? L("记录") : L("更新")) \(Format.date(quota.updatedAt))") } }.font(AppFont.secondary).foregroundStyle(.secondary)
                if let note = quota.metadataError { Text(L(note) + (quota.identity?.stale == true && quota.plan != nil ? L(" · 显示上次订阅") : "")).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled) }
            }
    }
    private var fullCard: some View {
        Surface(spacing: compact ? 9 : 14, padding: compact ? 12 : 16, tint: Palette.category(quota.provider), minimumHeight: !isMeasurement && collapsible && !expanded ? collapsedHeight : nil) {
            cardHeader
            if !expanded {
                if quota.provider == "codex" {
                    FlowLayout(spacing: 12) {
                        CreditBalanceLabel(balance: quota.credits, estimate: bestCredit, compact: true).fixedSize(horizontal: true, vertical: false)
                        Text(L("Bank Reset {0} 次可用" , quota.bankReset.map { String($0.availableCount) } ?? "—")).font(AppFont.secondary).foregroundStyle(.secondary).monospacedDigit()
                    }
                }
                balanceContent
                if quota.provider == "antigravity" { agyWindows }
                else { BalancedGridLayout(minimumWidth: 150, spacing: 12, maximumColumns: 2) {
                    ForEach(quota.provider == "antigravity" ? quota.windows : Array(quota.windows.prefix(2))) { window in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Text(window.windowMinutes == CoreContract.fiveHourMinutes ? "5h" : window.windowMinutes == CoreContract.weekMinutes ? "7d" : L(window.name)).lineLimit(2)
                                Spacer(minLength: 0)
                                Text(L("剩余 ") + Format.percent(max(0, 100 - window.usedPercent))).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent)).fixedSize()
                            }.font(AppFont.secondary)
                            ResourceBar(percent: 100 - window.usedPercent, tint: Palette.quota(window.usedPercent))
                            QuotaResetLabel(window: window)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                }
                if quota.error != nil { HStack { Label(L("额度更新失败"), systemImage: "exclamationmark.triangle"); Spacer(); if let onRetry { Button(L("重试"), action: onRetry) } }.font(AppFont.secondary).foregroundStyle(Palette.warn) }
            }
            if expanded {
            balanceContent
            if quota.provider == "antigravity" { agyWindows }
            else if !quota.windows.isEmpty { LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .leading), count: expanded ? 1 : 2), alignment: .leading, spacing: expanded ? 14 : 8) {
            ForEach(quota.windows) { window in
                VStack(spacing: 6) {
                    ViewThatFits(in: .horizontal) {
                        HStack { Text(L(window.name)); Spacer(minLength: 6); Text(L("剩余 ") + Format.percent(max(0, 100 - window.usedPercent))).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent)) }
                        VStack(alignment: .leading, spacing: 3) { Text(L(window.name)); Text(L("剩余 ") + Format.percent(max(0, 100 - window.usedPercent))).monospacedDigit().foregroundStyle(Palette.quota(window.usedPercent)) }.frame(maxWidth: .infinity, alignment: .leading)
                    }.font(AppFont.secondary)
                    ResourceBar(percent: 100 - window.usedPercent, tint: Palette.quota(window.usedPercent))
                    QuotaResetLabel(window: window).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            }
            }
            if let onEstimate, quota.provider != "antigravity", quota.windows.contains(where: { $0.windowMinutes == CoreContract.fiveHourMinutes || $0.windowMinutes == CoreContract.weekMinutes }) || estimate != nil {
                EstimateEntry(records: quotaHistory.isEmpty ? [estimate].compactMap { $0 } : quotaHistory, quota: quota) { onEstimate(nil) }
            }
            if expanded && !sourceName.isEmpty { Text(sourceName).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1) }
            if quota.provider == "codex" {
                CreditBalanceLabel(balance: quota.credits, estimate: bestCredit).fixedSize(horizontal: false, vertical: true)
                    .help(quota.creditsUpdatedAt.map { L("更新于 ") + Format.date($0) } ?? L("尚未取得 credits 信息"))
                if let onCredits { EstimateEntry(records: creditHistory.isEmpty ? [creditEstimate].compactMap { $0 } : creditHistory, credits: true, action: onCredits) }
            }
            if expanded, let bank = quota.bankReset {
                Divider().opacity(0.5)
                DisclosureGroup {
                    if let credits = bank.credits { ForEach(credits) { credit in
                        HStack { VStack(alignment: .leading, spacing: 3) { Text(credit.title ?? "Reset Credit").font(AppFont.secondary); Text(["available":L("可用"), "redeeming":L("处理中"), "redeemed":L("已使用"), "unknown":L("未知状态")][credit.status] ?? credit.status).font(AppFont.secondary).foregroundStyle(.secondary) }; Spacer(); Text(credit.expiresAt.map { L("到期 {0}" , Format.date($0)) } ?? L("无到期时间")).font(AppFont.secondary).foregroundStyle(.secondary) }
                    } }
                    if let updated = quota.bankUpdatedAt { Text("\(Format.date(updated))").font(AppFont.secondary).foregroundStyle(.secondary) }
                } label: {
                    HStack { Image(systemName: "rectangle.stack").foregroundStyle(Palette.accent); Text("Bank Reset"); Spacer(); Text(L("{0} 次可用" , bank.availableCount)).foregroundStyle(Palette.accent) }.font(.system(size: DesignTokens.fontSecondary, weight: DesignTokens.weightMedium))
                }
            }
            if let error = quota.error {
                if expanded { HStack(alignment: .top) { ErrorNotice(message: error, context: L("额度查询失败")); if let onRetry { Button(L("重试"), action: onRetry) } } }
            }
            if let onAnalysis { Button(L("查看分析"), systemImage: "arrow.up.right", action: onAnalysis).font(AppFont.caption) }
            if quota.windows.isEmpty && (quota.balances ?? []).isEmpty && quota.extraUsage == nil && quota.error == nil && quota.credits == nil { Text(L("暂无额度数据")).font(AppFont.secondary).foregroundStyle(.secondary) }
            }
        }
    }
}

struct ServerCard: View, Equatable {
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.host == rhs.host && lhs.result == rhs.result && lhs.compact == rhs.compact }
    var host: Host, result: HostResult?, compact: Bool
    var onRefresh: () -> Void = {}
    var onSettings: () -> Void = {}
    @AppStorage private var expanded: Bool
    init(host: Host, result: HostResult?, compact: Bool, onRefresh: @escaping () -> Void = {}, onSettings: @escaping () -> Void = {}) {
        self.host = host; self.result = result; self.compact = compact; self.onRefresh = onRefresh; self.onSettings = onSettings
        _expanded = AppStorage(wrappedValue: false, NativePanelKey.expansion(.server, id: host.id, compact: compact))
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Surface(spacing: 8, padding: 12) {
            HStack(spacing: 8) {
            DisclosureGroup(isExpanded: $expanded) {
                EmptyView()
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: "server.rack").font(.system(size: DesignTokens.fontPanelTitle)).foregroundStyle(Palette.accent)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(host.name.isEmpty ? host.target : host.name).font(AppFont.section).lineLimit(1).help(host.name.isEmpty ? host.target : host.name)
                        if expanded { Text(host.target).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1).help(host.target) }
                    }
                    Spacer()
                    SurfaceTimeline(interval: 1) { context in
                        let label = status(at: context.date)
                        HStack(spacing: 6) { Circle().fill(label == L("正常") ? Palette.ok : label == L("已暂停") ? Color.gray : label == L("连接失败") ? Palette.danger : Palette.warn).frame(width: 6, height: 6); Text(label).font(AppFont.secondary).foregroundStyle(.secondary) }
                    }

                }
            }
                Button(L("监控设置"), action: onSettings).font(AppFont.secondary).accessibilityLabel((host.name.isEmpty ? host.target : host.name) + L("的监控设置"))
            }
            ServerResourceSummary(host: host, sample: result?.sample, expanded: expanded)
                .opacity(host.enabled ? 1 : 0.65)
            if !expanded, let error = result?.error { Label(L(error), systemImage: "exclamationmark.triangle").font(AppFont.caption).foregroundStyle(Palette.warn).lineLimit(2).help(L(error)) }
            if !expanded, let sample = result?.sample, !sample.errors.isEmpty { Label(L("部分资源采集失败"), systemImage: "exclamationmark.triangle").font(AppFont.caption).foregroundStyle(Palette.warn) }
            if expanded {
                if let sample = result?.sample {
                    VStack(alignment: .leading, spacing: 16) {
                        if host.metrics.contains("cpu"), let rows = sample.cpu {
                            let cpus = host.selectedDevices("cpu", rows)
                            LazyMetricDisclosure(key: metricKey("cpu")) {
                                ForEach(cpus.filter { $0.id != "cpu" }) { cpu in
                                    VStack(spacing: 4) {
                                        metricRow(cpu.id, Format.percent(cpu.utilization))
                                        ResourceBar(percent: cpu.utilization)
                                    }.padding(.vertical, 3)
                                }
                                if host.shows("cpuTimes"), let cpu = cpus.first(where: { $0.id == "cpu" }) { metricRow("user / system", "\(Format.percent(cpu.userPercent)) / \(Format.percent(cpu.systemPercent))"); metricRow("iowait / steal", "\(Format.percent(cpu.iowaitPercent)) / \(Format.percent(cpu.stealPercent))") }
                            } label: { MetricHeading(icon: "cpu", title: L("CPU 明细"), value: "") }
                        }
                        if host.metrics.contains("memory"), let m = sample.memory {
                            LazyMetricDisclosure(key: metricKey("memory")) {
                                metricRow(L("可用"), Format.bytes(m.available)); if host.shows("memoryCache") { metricRow(L("缓存 / Buffer"), "\(Format.bytes(m.cached)) / \(Format.bytes(m.buffers))") }
                                if host.shows("swap") { metricRow("Swap", "\(Format.bytes(m.swapUsed)) / \(Format.bytes(m.swapTotal))"); ResourceBar(percent: Format.capacityPercent(m.swapUsed, m.swapTotal)) }
                            } label: { MetricHeading(icon: "memorychip", title: L("内存明细"), value: "") }
                        }
                        if host.shows("gpuThermals") {
                            ForEach(host.selectedDevices("gpu", sample.gpu)) { gpu in
                                LazyMetricDisclosure(key: metricKey("gpu:" + gpu.id)) {
                                    metricRow(L("温度 / 功耗"), "\(gpu.temperature.map { String(format: "%.0f°C", $0) } ?? "—") / \(gpu.powerWatts.map { String(format: "%.1f W", $0) } ?? "—")")
                                } label: { MetricHeading(icon: "rectangle.3.group", title: L("GPU {0} 明细" , gpu.id), value: "") }
                            }
                        }
                        if host.metrics.contains("filesystems"), let rows = sample.filesystems {
                            let filesystems = host.selectedDevices("filesystems", rows)
                            LazyMetricDisclosure(key: metricKey("filesystems")) {
                                ForEach(filesystems) { fs in VStack(alignment: .leading, spacing: 4) {
                                    Text(fs.id).font(AppFont.secondary).lineLimit(1).truncationMode(.middle).help(fs.id)
                                    if host.shows("fsAvailable") { metricRow(L("可用"), Format.bytes(fs.available)) }
                                    if host.shows("fsType") { metricRow(fs.device ?? "", fs.type ?? "") }
                                    if host.shows("inodes"), let total = fs.inodes, let free = fs.inodesFree, total > 0 { metricRow("inode", Format.percent((total - free) / total * 100)) }
                                }.padding(.vertical, 4) }
                            } label: { MetricHeading(icon: "internaldrive", title: L("文件系统"), value: L("{0} 个挂载点" , filesystems.count)) }
                        }
                        if host.metrics.contains("disk"), let rows = sample.disk {
                            let disks = host.selectedDevices("disk", rows)
                            LazyMetricDisclosure(key: metricKey("disk")) {
                                ForEach(disks) { disk in VStack(spacing: 4) { metricRow(disk.id, L("读 {0}" , Format.speed(disk.readBytesPerSecond))); metricRow("", L("写 {0}" , Format.speed(disk.writeBytesPerSecond))); if host.shows("diskIops") { metricRow(L("IOPS 读 / 写"), "\(disk.readIops.map(Format.compact) ?? "—") / \(disk.writeIops.map(Format.compact) ?? "—")") }; if host.shows("diskBusy") { metricRow(L("忙碌率"), Format.percent(disk.busyMsPerSecond.map { min(100, max(0, $0 / 10)) })); ResourceBar(percent: disk.busyMsPerSecond.map { $0 / 10 }) } }.padding(.vertical, 3) }
                            } label: { MetricHeading(icon: "arrow.left.arrow.right", title: L("磁盘 I/O"), value: L("{0} 个设备" , disks.count)) }
                        }
                        if host.metrics.contains("network"), let rows = sample.network {
                            let networks = host.selectedDevices("network", rows)
                            LazyMetricDisclosure(key: metricKey("network")) {
                                ForEach(networks) { net in VStack(spacing: 4) {
                                    metricRow(net.id, "↓ \(Format.speed(net.rxBytesPerSecond))")
                                    metricRow("", "↑ \(Format.speed(net.txBytesPerSecond))")
                                    if host.shows("networkErrors") { metricRow(L("错误 / 丢包"), "\(Format.counterTotal(net.rxErrors, net.txErrors)) / \(Format.counterTotal(net.rxDrops, net.txDrops))") }
                                }.padding(.vertical, 3) }
                            } label: { MetricHeading(icon: "network", title: L("网络"), value: L("{0} 个网卡" , networks.count)) }
                        }
                        ForEach(sample.errors.keys.filter { host.metrics.contains($0) }.sorted(), id: \.self) { key in metricRow(["gpu":"GPU", "cpu":"CPU", "memory":L("内存"), "filesystems":L("文件系统"), "disk":L("磁盘 I/O"), "network":L("网络")][key] ?? key, L("采集失败")) }
                        Text(L("负载 ") + (sample.load.isEmpty ? "—" : sample.load.map { String(format: "%.2f", $0) }.joined(separator: " / "))).font(AppFont.secondary).foregroundStyle(.secondary)
                    }.padding(.top, 14)
                } else if host.enabled { Text(L(result?.error == nil ? L("等待首次采样") : L("连接失败"))).font(AppFont.secondary).foregroundStyle(.secondary).padding(.top, 10) }
                if let error = result?.error { ErrorNotice(message: error).font(AppFont.secondary).foregroundStyle(Palette.warn).padding(.top, 6) }
            }
            HStack {
                Text(L("采样 ") + (result?.sample.map { Format.date($0.timestamp) } ?? "—") + (host.enabled ? "" : L(" · 已暂停，保留旧读数")))
                Spacer()
                if host.enabled { Button(L("刷新"), action: onRefresh).accessibilityLabel(L("刷新服务器 ") + (host.name.isEmpty ? host.target : host.name)) }
            }.font(AppFont.secondary).foregroundStyle(.secondary)
        }
    }
    private func status(at date: Date) -> String {
        if !host.enabled { return L("已暂停") }
        if result?.error != nil { return L("连接失败") }
        guard let sample = result?.sample else { return L("等待采样") }
        if date.timeIntervalSince1970 - sample.timestamp > 10 { return L("数据已延迟") }
        return sample.errors.isEmpty ? L("正常") : L("部分采集失败")
    }
    private func metricKey(_ group: String) -> String { (compact ? "panel." : "detail.") + host.id + ":" + group }
    private func metricRow(_ key: String, _ value: String) -> some View { HStack { Text(key).lineLimit(1).truncationMode(.middle); Spacer(minLength: 8); Text(value).monospacedDigit() }.font(AppFont.secondary).foregroundStyle(.secondary).padding(.vertical, 2) }
}
struct LazyMetricDisclosure<Content: View, Label: View>: View {
    @AppStorage private var expanded: Bool
    @ViewBuilder var content: () -> Content
    @ViewBuilder var label: () -> Label
    init(key: String, @ViewBuilder content: @escaping () -> Content, @ViewBuilder label: @escaping () -> Label) {
        _expanded = AppStorage(wrappedValue: false, NativePanelKey.Prefix.metric.rawValue + key)
        self.content = content; self.label = label
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading) {
            DisclosureGroup(isExpanded: $expanded) { EmptyView() } label: { label() }
            if expanded { LazyVStack(alignment: .leading, spacing: 4) { content() } }
        }
    }
}
struct MetricHeading: View {
    var icon: String, title: String, value: String
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        HStack(spacing: 8) { Image(systemName: icon).frame(width: 15).foregroundStyle(.secondary); Text(title); Spacer(); Text(value).monospacedDigit().foregroundStyle(.secondary) }.font(.system(size: DesignTokens.fontSecondary, weight: DesignTokens.weightMedium))
    }
}

struct ServerResourceSummary: View {
    var host: Host, sample: MetricSample?
    var expanded = false
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 10) {
            Grid(alignment: .topLeading, horizontalSpacing: 10) {
                GridRow {
                    if host.metrics.contains("cpu") {
                        ResourceBlock(title: "CPU", percent: sample?.cpu?.first { $0.id == "cpu" }?.utilization, category: "cpu")
                    }
                    if host.metrics.contains("memory") {
                        ResourceBlock(title: L("内存"), percent: Format.capacityPercent(sample?.memory?.used, sample?.memory?.total), category: "memory", detail: sample?.memory == nil ? "" : Format.capacityPair(sample?.memory?.used, sample?.memory?.total))
                    }
                }
            }.fixedSize(horizontal: false, vertical: true)
            if host.metrics.contains("gpu") {
                let gpus = host.selectedDevices("gpu", sample?.gpu)
                BalancedGridLayout(minimumWidth: 290, spacing: 10) {
                    ForEach(gpus) { gpu in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text("GPU " + gpu.id).fontWeight(.medium).fixedSize()
                                Text(gpu.name ?? "—").foregroundStyle(.secondary).lineLimit(1).help(gpu.name ?? "—")
                            }.font(AppFont.secondary)
                            HStack(alignment: .top, spacing: 12) {
                                ResourceStrip(title: L("利用率"), percent: gpu.utilization, tint: Palette.category("gpu")).frame(width: host.shows("gpuMemory") ? 86 : nil)
                                if host.shows("gpuMemory") {
                                    ResourceStrip(title: L("显存"), percent: Format.capacityPercent(gpu.memoryUsedMiB, gpu.memoryTotalMiB),
                                                  detail: Format.capacityPair(gpu.memoryUsedMiB.map { $0 * 1048576 }, gpu.memoryTotalMiB.map { $0 * 1048576 }), tint: Palette.category("gpu"))
                                }
                            }

                        }.frame(maxWidth: .infinity, alignment: .leading).padding(10).background(Palette.category("gpu").opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                    }
                }
            }
            if host.metrics.contains("filesystems") {
                let filesystems = host.selectedDevices("filesystems", sample?.filesystems)
                if !filesystems.isEmpty { VStack(alignment: .leading, spacing: 6) {
                    Text(L("文件系统")).font(AppFont.secondary).foregroundStyle(.secondary)
                    BalancedGridLayout(minimumWidth: 290, spacing: 9) {
                        ForEach(filesystems) { fs in ResourceStrip(title: fs.id, percent: Format.capacityPercent(fs.used, fs.total), detail: Format.capacityPair(fs.used, fs.total), tint: Palette.category("storage")) }
                    }
                }.padding(10).background(Palette.category("storage").opacity(0.08), in: RoundedRectangle(cornerRadius: 12)) }
            }
            if host.metrics.contains("network") {
                let networks = host.selectedDevices("network", sample?.network)
                BalancedGridLayout(minimumWidth: 290, spacing: 9) {
                    ForEach(networks) { net in
                        HStack { Text(net.id).lineLimit(1); Spacer(); Text("↓ " + Format.speed(net.rxBytesPerSecond) + " · ↑ " + Format.speed(net.txBytesPerSecond)).monospacedDigit() }.font(AppFont.caption).padding(10).background(Palette.category("network").opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
                    }
                }
            }
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 14) { missingResources }
                VStack(alignment: .leading, spacing: 4) { missingResources }
            }.font(AppFont.caption).foregroundStyle(.secondary)
            if expanded, host.shows("uptime") {
                Text(L("连续运行 ") + Format.uptime(sample?.uptime)).font(AppFont.secondary).monospacedDigit().foregroundStyle(.secondary)
            }
            if expanded, host.shows("networkTotals"), host.metrics.contains("network") {
                let networks = host.selectedDevices("network", sample?.network)
                if networks.isEmpty { Text(sample?.network == nil ? L("累计流量 —") : L("累计流量 无已选网卡")).font(AppFont.secondary).foregroundStyle(.secondary) }
                ForEach(networks) { net in
                    ViewThatFits(in: .horizontal) {
                        HStack { Text(net.id + L(" 累计")); Text("↓ \(Format.bytes(net.rxBytes)) · ↑ \(Format.bytes(net.txBytes))").monospacedDigit() }
                        VStack(alignment: .leading, spacing: 3) { Text(net.id + L(" 累计")); Text("↓ \(Format.bytes(net.rxBytes)) · ↑ \(Format.bytes(net.txBytes))").monospacedDigit() }
                    }.font(AppFont.secondary).foregroundStyle(.secondary)
                }
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
    @ViewBuilder private var missingResources: some View {
        if host.metrics.contains("gpu"), host.selectedDevices("gpu", sample?.gpu).isEmpty { Text(sample?.gpu == nil ? "GPU —" : L("GPU 无已选设备")) }
        if host.metrics.contains("filesystems"), host.selectedDevices("filesystems", sample?.filesystems).isEmpty { Text(sample?.filesystems == nil ? L("存储 —") : L("无已选挂载点")) }
        if host.metrics.contains("network"), host.selectedDevices("network", sample?.network).isEmpty { Text(sample?.network == nil ? L("网络 —") : L("无已选网卡")) }
    }
}

struct ResourceBlock: View {
    var title: String, percent: Double?, category: String
    var detail = ""
    private var value: Double? { percent.flatMap { $0.isFinite ? min(100, max(0, $0)) : nil } }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Text(title).font(AppFont.caption)
                if let value, value >= 90 { Image(systemName: "exclamationmark.triangle").font(AppFont.caption).foregroundStyle(Palette.danger).help(L("高负载")).accessibilityLabel(L("高负载")) }
                Spacer(minLength: 4)
                Text(value.map { String(format: "%.0f%%", $0) } ?? "—").font(.system(size: DesignTokens.fontPanelTitle, weight: DesignTokens.weightStrong)).monospacedDigit().foregroundStyle((value ?? 0) >= 90 ? Palette.danger : Palette.category(category))
            }
            ResourceBar(percent: value, tint: Palette.category(category))
            if !detail.isEmpty { Text(detail).font(AppFont.caption).foregroundStyle(.secondary).monospacedDigit() }
        }.padding(.horizontal, 10).padding(.vertical, 8).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(Palette.category(category).opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Palette.category(category).opacity(0.23)))
            .accessibilityElement(children: .combine)
    }
}

struct ResourceStrip: View {
    var title: String, percent: Double?
    var detail = ""
    var tint: Color = Palette.accent
    private var value: Double? { percent.flatMap { $0.isFinite ? min(100, max(0, $0)) : nil } }
    private var label: String { (detail.isEmpty ? "" : detail + " · ") + (value.map { String(format: "%.0f%%", $0) } ?? "—") }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(spacing: 3) {
            HStack(spacing: 5) {
                Text(title).font(AppFont.secondary).lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 0)
                if (value ?? 0) >= 90 { Image(systemName: "exclamationmark.triangle").font(.system(size: DesignTokens.fontCaption)).foregroundStyle(Palette.danger) }
                Text(label).font(.system(size: DesignTokens.fontCaption)).monospacedDigit().lineLimit(1).fixedSize().foregroundStyle((value ?? 0) >= 90 ? Palette.danger : Color.primary)
            }.frame(height: 17)
            ResourceBar(percent: value, tint: tint)
        }.frame(maxWidth: .infinity, alignment: .leading).help(title + " " + label)
            .accessibilityElement(children: .ignore).accessibilityLabel(title + " " + label + ((value ?? 0) >= 90 ? L("，高负载") : ""))
    }
}

struct ResourceBar: View {
    var percent: Double?
    var tint: Color? = nil
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(.primary.opacity(DesignTokens.progressTrackOpacity))
                if let percent, percent.isFinite {
                    Capsule().fill(tint ?? Palette.accent)
                        .frame(width: proxy.size.width * min(1, max(0, percent / 100)))
                }
            }
        }.frame(height: DesignTokens.progressHeight)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.35), value: percent)
            .accessibilityLabel(percent.map { Format.percent($0) } ?? L("暂无数据"))
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
                func findScroll(_ view: NSView) -> NSScrollView? {
                    if let scroll = view as? NSScrollView { return scroll }
                    return view.subviews.lazy.compactMap(findScroll).first
                }
                guard let scroll = self.enclosingScrollView ?? self.superview.flatMap(findScroll) ?? self.superview?.superview.flatMap(findScroll) else { return }
                // Scroller style and visibility follow System Settings ("Show scroll bars").
                if scroll.drawsBackground { scroll.drawsBackground = false }
            }
        }
    }
    func makeNSView(context: Context) -> Anchor { Anchor() }
    func updateNSView(_ view: Anchor, context: Context) { view.configure() }
}

struct RefreshIndicator: View {
    @SurfaceModel var model: AppModel
    let key: String
    @State private var showing = false
    private var failures: [ActionFailure] { model.actionFailures.filter { !$0.dismissed && ($0.action == key || (key == "scan" && $0.action == "prices")) } }
    private var hasError: Bool { model.actionFailures.contains { $0.action == key || (key == "scan" && $0.action == "prices") } || (key == "scan" && model.dashboardError != nil) }
    private var statusLabel: String {
        L("{0}状态 · {1}；点击查看详情", model.refreshLabel(key), model.refreshStates[key]?.busy == true ? L("进行中") : hasError ? L("部分失败") : model.refreshStates[key]?.succeededAt != nil ? L("已更新") : L("尚未刷新"))
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Button { showing.toggle() } label: {
            ZStack {
                if model.refreshStates[key]?.busy == true || (key == "scan" && model.dashboardPending) { ProgressView().controlSize(.small).scaleEffect(0.7) }
                // A sync mark rather than a "verified" check; problems use the warning triangle (Web shares the rule).
                else { Image(systemName: hasError ? "exclamationmark.triangle" : "arrow.triangle.2.circlepath").foregroundStyle(hasError ? Palette.warn : Color.secondary).opacity(hasError || model.refreshStates[key]?.succeededAt != nil ? 1 : 0.6) }
            }.frame(width: 22, height: 22)
        }.buttonStyle(.plain).help(statusLabel).accessibilityLabel(statusLabel)
        .popover(isPresented: $showing) {
            VStack(alignment: .leading, spacing: 12) {
                Text(model.refreshLabel(key) + L("状态")).font(AppFont.section)
                if let time = model.refreshStates[key]?.succeededAt { Text(L("上次成功：") + time.formatted(Date.FormatStyle(date: .abbreviated, time: .shortened).locale(AieyesLocale.locale))).font(AppFont.secondary) }
                else if failures.isEmpty && hasError { Text(L("本次错误提示已关闭")).font(AppFont.secondary) }
                else { Text(model.refreshStates[key]?.busy == true ? L("正在刷新…") : L("尚未刷新")).font(AppFont.secondary) }
                ForEach(failures) { failure in
                    HStack { VStack(alignment: .leading) { Text(failure.name); ErrorNotice(message: failure.reason).font(AppFont.secondary).textSelection(.enabled) }; Spacer(); Button(L("重试")) { Task { await model.retry(failure) } }; Button(L("关闭")) { model.dismissFailure(failure) } }
                }
                if key == "scan", model.dashboardPending || model.dashboardError != nil {
                    Text((model.dashboardPending ? L("正在更新，仍显示上一结果：") : L("更新失败，仍显示上一结果：")) + model.appliedScope).font(AppFont.secondary)
                    if let error = model.dashboardError { ErrorNotice(message: error).font(AppFont.secondary).textSelection(.enabled); Button(L("重试查询")) { Task { await model.reload() } } }
                }
                if key == "scan", let message = model.message, !model.actionFailures.contains(where: { $0.dismissed && $0.reason == message }) { Text(L(message)).font(AppFont.secondary).textSelection(.enabled) }
            }.padding(16).frame(width: 360)
        }
    }
}

private struct SessionStrip: View {
    @SurfaceModel var model: AppModel
    @ObservedObject var status: SessionStripStore
    @Binding var expanded: Bool
    var compact: Bool
    var provider = "all"
    private var sessions: [LiveSession] { status.snapshot.sessions.filter { (provider == "all" || CoreContract.sameAgent($0.provider, provider)) && $0.phase != .idle }.sorted { rank($0.phase) < rank($1.phase) } }
    private var failures: [SessionSourceHealth] { status.snapshot.sources.filter { $0.state != "healthy" && (provider == "all" || CoreContract.sameAgent($0.provider, provider)) } }
    private func rank(_ phase: SessionPhase) -> Int { phase == .needsInput ? 0 : phase == .failed ? 1 : phase == .unknown ? 2 : 3 }
    private var counts: String {
        let phases: [SessionPhase] = [.needsInput, .failed, .working, .thinking, .tool, .unknown, .complete, .interrupted]
        return phases.compactMap { phase in let count = sessions.filter { $0.phase == phase }.count; return count > 0 ? "\(phase.label) \(count)" : nil }.joined(separator: " · ")
    }
    private var summary: String {
        if status.snapshot.unavailable { return L("会话读取失败") }
        if !failures.isEmpty { return L("{0} 个来源异常" , failures.count) }
        return L("{0} 个会话" , sessions.count)
    }
    private var phase: SessionPhase { status.snapshot.unavailable || !failures.isEmpty ? .unknown : sessions.first?.phase ?? .idle }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        if !sessions.isEmpty || !failures.isEmpty || status.snapshot.unavailable {
        DisclosureGroup(isExpanded: $expanded) {
            VStack(spacing: 12) {
                ForEach(sessions) { session in
                    HStack(spacing: 10) {
                        ActivityIndicator(phase: session.phase)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(Format.provider(session.provider) + " · " + session.machine).lineLimit(1)
                            SurfaceTimeline(interval: 60) { context in
                                Text(L("最后更新 ") + session.updatedAt.formatted(Date.FormatStyle(date: .omitted, time: .shortened).locale(AieyesLocale.locale))).font(AppFont.secondary).foregroundStyle(.secondary)
                            }
                        }
                        Spacer()
                        Text(session.phase.label).foregroundStyle(session.phase.color)
                    }
                }
                if sessions.isEmpty { Text(status.snapshot.unavailable ? L("读取失败") : L("暂无会话动态")).foregroundStyle(.secondary) }
                ForEach(failures) { source in
                    let machine = source.machineId == "local" ? L("本机") : model.settings.hosts.first(where: { $0.id == source.machineId })?.name ?? source.machineId
                    Text(Format.provider(source.provider) + " · " + machine + ": " + L(source.message))
                        .font(AppFont.secondary).foregroundStyle(Palette.warn).frame(maxWidth: .infinity, alignment: .leading)
                }
            }.padding(.top, 12)
        } label: {
            HStack(spacing: 9) {
                ActivityIndicator(phase: phase)
                Text(summary).font(.system(size: DesignTokens.fontBody, weight: DesignTokens.weightMedium)).help(L("已启用的本机和 SSH 会话"))
                Spacer()
                Text(counts).font(AppFont.caption).foregroundStyle(sessions.contains { $0.phase == .needsInput || $0.phase == .failed } ? Palette.warn : Color.secondary).lineLimit(2)
            }
        }
        .padding(compact ? 8 : 14).modifier(NeutralCard())
        }
    }
}
