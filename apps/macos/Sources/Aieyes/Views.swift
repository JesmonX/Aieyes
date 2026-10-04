import SwiftUI
import Charts

enum Palette {
    static let accent = Color(nsColor: NSColor(name: nil) { appearance in
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? NSColor(srgbRed: 0.55, green: 0.65, blue: 1, alpha: 1)
            : NSColor(srgbRed: 0.25, green: 0.36, blue: 0.82, alpha: 1)
    })
    static let colors: [Color] = [accent, .teal, .purple, .orange, .pink, .cyan, .green, .indigo]
    static func model(_ name: String) -> Color {
        let hash = name.utf8.reduce(UInt32(2166136261)) { ($0 ^ UInt32($1)) &* 16777619 }
        return Color(hue: Double(hash % 3600) / 3600, saturation: 0.64, brightness: 0.8)
    }
}

struct Surface<Content: View>: View {
    var title: String?
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let title { Text(title).font(.system(size: 15, weight: .semibold)) }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading).padding(16)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 18))
        .overlay(RoundedRectangle(cornerRadius: 18).strokeBorder(LinearGradient(colors: [Palette.accent.opacity(0.22), .teal.opacity(0.08), .purple.opacity(0.16)], startPoint: .topLeading, endPoint: .bottomTrailing)))
        .shadow(color: Palette.accent.opacity(0.045), radius: 10, y: 4)
    }
}

struct RootView: View {
    @ObservedObject var model: AppModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var compact = true
    @State private var tab = "agent"
    @State private var costMode = false
    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().opacity(0.55)
            ScrollView {
                VStack(alignment: .leading, spacing: compact ? 14 : 22) {
                    if let message = model.message {
                        HStack(spacing: 8) {
                            Text(message).font(.system(size: 14)).textSelection(.enabled)
                            Spacer(); Button { model.message = nil } label: { Image(systemName: "xmark") }.buttonStyle(.plain)
                        }.padding(10).background(.quaternary, in: RoundedRectangle(cornerRadius: 9))
                    }
                    if tab == "agent" { agentContent } else { serverContent }
                }.padding(compact ? 18 : 28).background(OverlayScrollStyle())
            }
            Divider().opacity(0.55)
            footer
        }
        .frame(width: compact ? 450 : nil, height: compact ? 720 : nil)
        .frame(minWidth: compact ? nil : 880, minHeight: compact ? nil : 650)
        .background {
            ZStack {
                Rectangle().fill(.ultraThinMaterial)
                LinearGradient(colors: [Palette.accent.opacity(0.10), .clear, Color.teal.opacity(0.07), Color.purple.opacity(0.06)], startPoint: .topLeading, endPoint: .bottomTrailing)
            }
        }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: tab)
        .font(.system(size: 15))
        .tint(Palette.accent)
        .onChange(of: model.provider) { _, _ in model.selectedModel = "all"; model.selectedAccount = "all"; model.selectedSource = "all"; Task { await model.reload() } }
        .onChange(of: model.selectedAccount) { _, _ in model.selectedSource = "all"; Task { await model.reload() } }
        .onChange(of: model.selectedSource) { _, _ in Task { await model.reload() } }
        .onChange(of: model.selectedModel) { _, _ in Task { await model.reload() } }
        .onChange(of: model.range) { _, _ in Task { await model.reload() } }
        .onChange(of: tab) { _, value in model.serverTabVisible = value == "servers"; if value == "servers" { Task { await model.sampleHosts() } } }
    }
    private var header: some View {
        HStack(spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: 9).fill(Palette.accent.gradient).frame(width: 34, height: 34)
                Image(systemName: "eye").font(.system(size: 18, weight: .medium)).foregroundStyle(.white)
            }
            if !compact { VStack(alignment: .leading, spacing: 1) { Text("Aieyes").font(.headline); EmptyView() } }
            Picker("页面", selection: $tab) { Text("Agent").tag("agent"); Text("服务器").tag("servers") }
                .pickerStyle(.segmented).labelsHidden().frame(maxWidth: compact ? .infinity : 230)
            if !compact { Spacer() }
            Menu {
                Button("同步记录") { Task { await model.scan() } }
                Button("刷新限额") { Task { await model.refreshQuotas() } }
                Button("同步价格") { Task { await model.syncPrices() } }
                Button("刷新服务器") { Task { await model.sampleHosts() } }
            } label: {
                if model.busy || model.serverBusy { ProgressView().controlSize(.regular).frame(width: 18) }
                else { Image(systemName: "arrow.clockwise").font(.system(size: 14)) }
            }.menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().disabled(model.busy || model.serverBusy).help("刷新")
        }.padding(.horizontal, compact ? 18 : 28).padding(.vertical, 16)
    }
    private var footer: some View {
        HStack(spacing: 8) {
            Text(model.busy ? model.activity : model.dashboard.generatedAt > 0 ? Format.time(model.dashboard.generatedAt) : "正在加载")
                .font(.system(size: 14)).foregroundStyle(.secondary)
            Spacer()
            if compact {
                Button { model.isPinned.toggle() } label: { Image(systemName: model.isPinned ? "pin.fill" : "pin") }.help(model.isPinned ? "取消固定" : "固定面板")
                Button { model.showDetail?() } label: { Image(systemName: "arrow.up.left.and.arrow.down.right") }.help("打开详情")
            }
            Button { model.showSettings?() } label: { Image(systemName: "gearshape") }.help("设置")
            Menu { Button("退出 Aieyes") { NSApplication.shared.terminate(nil) }.keyboardShortcut("q") } label: { Image(systemName: "ellipsis") }
                .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
        }.buttonStyle(.plain).padding(.horizontal, compact ? 18 : 28).padding(.vertical, 12)
    }
    @ViewBuilder private var agentContent: some View {
        sessionStrip
        HStack(spacing: 8) {
            Picker("Agent", selection: $model.provider) {
                Text("全部 Agent").tag("all")
                ForEach(["codex", "claude", "antigravity", "agy", "deepseek", "custom"], id: \.self) { Text(Format.provider($0)).tag($0) }
            }.labelsHidden()
            Picker("账户", selection: $model.selectedAccount) {
                Text("全部账户").tag("all"); Text("无账户 / API").tag("none")
                ForEach(model.settings.accounts.filter { model.provider == "all" || $0.provider == model.provider }, id: \.key) { Text($0.name).tag($0.key) }
            }.labelsHidden()
            if !compact {
                Picker("数据源", selection: $model.selectedSource) {
                    Text("全部数据源").tag("all")
                    ForEach(selectableSources) { Text($0.name).tag($0.id) }
                }.labelsHidden()
                Picker("模型", selection: $model.selectedModel) {
                    Text("全部模型").tag("all")
                    ForEach(Array(Set(model.dashboard.dayModels.map(\.model) + (model.selectedModel == "all" ? [] : [model.selectedModel]))).sorted(), id: \.self) { Text($0).tag($0) }
                }.labelsHidden()
            }
        }.controlSize(.regular)
        if !model.dashboard.quotas.isEmpty { quotaSection }
        HStack(alignment: .firstTextBaseline) {
            Text(model.range == 1 ? "今日概览" : "使用概览").font(.system(size: compact ? 19 : 24, weight: .semibold))
            Spacer()
            Picker("时间范围", selection: $model.range) {
                Text("今日").tag(1); Text("最近 7 天").tag(7); Text("最近 30 天").tag(30); Text("最近 90 天").tag(90); Text("最近一年").tag(365)
            }.labelsHidden().fixedSize().controlSize(.regular)
        }
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: compact ? 2 : 4), spacing: 10) {
            StatCard(title: "总 Token", value: Format.compact(model.dashboard.summary.total), detail: "", icon: "sparkle", accent: Palette.accent)
            StatCard(title: "API 等价成本", value: model.dashboard.summary.pricedTokens > 0 ? Format.money(model.dashboard.summary.cost) : "—", detail: "", icon: "dollarsign.circle", accent: .teal, pricingIncomplete: model.dashboard.summary.total > 0 && model.dashboard.summary.pricedTokens < model.dashboard.summary.total, onPricing: { model.openPricing() })
            if !compact {
                StatCard(title: "缓存命中率", value: Format.percent(model.dashboard.summary.tokens.cacheRate.map { $0 * 100 }), detail: "", icon: "square.3.layers.3d", accent: .purple)
                TokenBreakdownCard(tokens: model.dashboard.summary.tokens)
            }
        }
        if compact {
            VStack(spacing: 8) { HStack { Text("缓存命中率"); Spacer(); Text(Format.percent(model.dashboard.summary.tokens.cacheRate.map { $0 * 100 })) }; TokenBreakdown(tokens: model.dashboard.summary.tokens, inline: true) }.font(.system(size: 14)).foregroundStyle(.secondary).monospacedDigit()
        }
        if compact {
            Surface {
                HStack { Text("近 7 天用量").font(.system(size: 15, weight: .semibold)); Spacer(); EmptyView() }
                UsageChart(days: recentDays, rows: recentRows, cost: false).frame(height: 85)
                ModelKey(rows: recentRows)
                DailyUsage(days: recentDays, rows: recentRows, compact: true)
            }
            Button { model.showDetail?() } label: { HStack { Text("用量详情"); Spacer(); Image(systemName: "arrow.up.right") }.font(.system(size: 14)) }.buttonStyle(.plain)
        } else {
            HStack {
                Text(model.range == 1 ? "近 7 天趋势" : "使用趋势").font(.title3.weight(.semibold)); Spacer()
                Picker("统计指标", selection: $costMode) { Text("Token").tag(false); Text("API 等价成本").tag(true) }.pickerStyle(.segmented).frame(width: 230)
            }
            HStack(alignment: .top, spacing: 18) {
                Surface(title: "每日用量 · 按模型") { UsageChart(days: model.dashboard.trendDays, rows: model.dashboard.dayModels, cost: costMode).frame(height: 230); ModelKey(rows: model.dashboard.dayModels) }
                Surface(title: "所选范围 · 模型分布") { ModelChart(models: model.dashboard.models, cost: costMode).frame(height: 230) }.frame(width: 340)
            }
            Surface(title: "每日明细") { DailyUsage(days: model.dashboard.trendDays, rows: model.dashboard.dayModels, compact: false) }
            Surface(title: "过去 365 天") { Heatmap(days: model.dashboard.heatmap, cost: costMode) }
        }
        if model.settings.sources.isEmpty { EmptyCard(icon: "tray.and.arrow.down", title: "添加数据源", subtitle: "", action: { model.settingsTab = "sources"; model.showSettings?() }) }
    }
    private var recentDays: [Aggregate] { Array(model.dashboard.trendDays.suffix(7)) }
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
                            Text(URL(fileURLWithPath: session.id).deletingPathExtension().lastPathComponent.suffix(8)).font(.system(size: 14, design: .monospaced)).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Text(session.phase.rawValue).foregroundStyle(session.phase.color)
                    }
                }
                if model.sessions.isEmpty { Text(model.sessionsUnavailable ? "读取失败" : "暂无会话动态").foregroundStyle(.secondary) }
                Text("本机 Codex · 日志状态").font(.system(size: 14)).foregroundStyle(.secondary)
            }.padding(.top, 12)
        } label: {
            HStack(spacing: 9) {
                ActivityIndicator(phase: model.sessionPhase)
                Text(model.sessionSummary).font(.system(size: 15, weight: .medium))
                Spacer()
                if let phase = model.sessionPhase { Text(phase.rawValue).font(.system(size: 14)).foregroundStyle(phase.color) }
            }
        }
        .padding(14).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder((model.sessionPhase?.color ?? Palette.accent).opacity(0.22)))
    }
    private var quotaSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("账户限额").font(.system(size: compact ? 19 : 24, weight: .semibold)); if model.dashboard.quotas.count > 1 { Text("\(model.dashboard.quotas.count)").font(.system(size: 14)).foregroundStyle(.secondary) }; Spacer(); Button("刷新") { Task { await model.refreshQuotas() } }.font(.system(size: 14)).disabled(model.busy) }
            if compact { ForEach(model.dashboard.quotas) { quota in QuotaCard(quota: quota, compact: model.dashboard.quotas.filter { $0.provider == quota.provider }.count > 1, sourceName: "") } }
            else { LazyVGrid(columns: [GridItem(.adaptive(minimum: 340), spacing: 16, alignment: .top)], alignment: .leading, spacing: 16) { ForEach(model.dashboard.quotas) { QuotaCard(quota: $0, sourceName: "") } } }
        }
    }
    @ViewBuilder private var serverContent: some View {
        HStack { VStack(alignment: .leading, spacing: 4) { Text("服务器").font(.system(size: compact ? 22 : 28, weight: .semibold, design: .default)); Text("\(model.settings.hosts.filter(\.enabled).count) 台主机").font(.system(size: 14)).foregroundStyle(.secondary) }; Spacer(); Button { model.settingsTab = "servers"; model.showSettings?() } label: { Image(systemName: "plus") }.help("添加主机") }
        if model.settings.hosts.isEmpty { EmptyCard(icon: "server.rack", title: "添加服务器", subtitle: "", action: { model.settingsTab = "servers"; model.showSettings?() }) }
        ForEach(model.settings.hosts) { host in
            ServerCard(host: host, result: model.hosts.first(where: { $0.id == host.id }), compact: compact)
        }
    }
}

struct TokenBreakdown: View {
    var tokens: Tokens
    var inline = false
    private var values: [(String, Double)] { [("输入", tokens.input), ("输出", tokens.output), ("缓存", tokens.cacheRead + tokens.cacheWrite)] }
    var body: some View {
        if inline {
            HStack(spacing: 12) {
                ForEach(values, id: \.0) { label, value in
                    VStack(alignment: .leading, spacing: 5) {
                        Text(label).font(.system(size: 14)).foregroundStyle(.secondary)
                        Text(Format.compact(value)).font(.system(size: 18, weight: .semibold)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.6)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        } else {
            VStack(spacing: 5) {
                ForEach(values, id: \.0) { label, value in
                    HStack { Text(label).font(.system(size: 14)).foregroundStyle(.secondary); Spacer(); Text(Format.compact(value)).font(.system(size: 16, weight: .semibold)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.6) }
                }
            }
        }
    }
}
struct TokenBreakdownCard: View {
    var tokens: Tokens
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Token 明细").font(.system(size: 14, weight: .medium)).foregroundStyle(.secondary)
            TokenBreakdown(tokens: tokens)
        }.padding(14).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(LinearGradient(colors: [Color.orange.opacity(0.14), Color.orange.opacity(0.04)], startPoint: .topLeading, endPoint: .bottomTrailing), in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(Color.orange.opacity(0.16)))
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
                    Button(action: onPricing) { Image(systemName: "exclamationmark.circle").foregroundStyle(.orange) }
                        .buttonStyle(.plain)
                        .help("计价覆盖未达 100%，部分模型缺少价格。请设置模型价格以补齐成本。")
                        .accessibilityLabel("计价未完成，设置模型价格")
                }
                Spacer(); Image(systemName: icon).font(.system(size: 14)).foregroundStyle(accent) }
            Text(value).contentTransition(.numericText()).font(.system(size: 28, weight: .semibold, design: .default)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.6)
            if !detail.isEmpty { Text(detail).font(.system(size: 14)).foregroundStyle(.secondary).lineLimit(1) }
        }.padding(14).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(LinearGradient(colors: [accent.opacity(0.14), accent.opacity(0.04)], startPoint: .topLeading, endPoint: .bottomTrailing), in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(accent.opacity(0.16)))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.3), value: value)
    }
}
struct EmptyCard: View {
    var icon: String, title: String, subtitle: String, action: () -> Void
    var body: some View {
        VStack(spacing: 12) { Image(systemName: icon).font(.system(size: 32, weight: .light)).foregroundStyle(Palette.accent); if !subtitle.isEmpty { Text(subtitle).font(.callout).foregroundStyle(.secondary) }; Button(title, action: action).buttonStyle(.borderedProminent) }
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
            AxisValueLabel { if let label = value.as(String.self) { Text(String(label.suffix(5))).font(.system(size: 14)) } }
        } }
        .chartYAxis { AxisMarks(position: .leading, values: .automatic(desiredCount: 3)) { value in AxisGridLine().foregroundStyle(.primary.opacity(0.06)); AxisValueLabel { if let n = value.as(Double.self) { Text(cost ? Format.money(n) : Format.compact(n)).font(.system(size: 14)) } } } }
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
                Text(name).font(.system(size: 14)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }.help(name)
        }
    }
}

struct DailyUsage: View {
    var days: [Aggregate], rows: [DayModel], compact: Bool
    @ViewBuilder var body: some View {
        if compact {
            DisclosureGroup("每日明细") { dailyRows.padding(.top, 8) }.font(.system(size: 14))
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
                            Text(Format.compact(row.usage.total)).monospacedDigit()
                            if !compact { Text("缓存 " + Format.percent(row.usage.tokens.cacheRate.map { $0 * 100 })).foregroundStyle(.secondary).frame(width: 100, alignment: .trailing) }
                        }.font(.system(size: 14)).padding(.vertical, 3)
                    }
                    if day.total == 0 { Text("当日暂无记录").font(.system(size: 14)).foregroundStyle(.secondary) }
                } label: {
                    HStack { Text(String(day.key.suffix(5))); Spacer(); Text(Format.compact(day.total)).monospacedDigit(); Text("缓存 " + Format.percent(day.tokens.cacheRate.map { $0 * 100 })).foregroundStyle(.secondary).frame(width: 100, alignment: .trailing) }.font(.system(size: 14))
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
                VStack(spacing: 3) { Text("\(models.count)").font(.system(size: 25, weight: .semibold, design: .default)); Text("模型").font(.system(size: 14)).foregroundStyle(.secondary) }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) { ForEach(models) { item in
                    HStack(alignment: .top, spacing: 6) {
                        Circle().fill(Palette.model(item.key)).frame(width: 6, height: 6).padding(.top, 4)
                        VStack(alignment: .leading, spacing: 4) { Text(item.key).font(.system(size: 14, weight: .medium)).lineLimit(2); Text(cost ? Format.money(item.cost) : Format.compact(item.total)).font(.system(size: 14)).foregroundStyle(.secondary) }
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
                    VStack(spacing: 3) { ForEach(0..<7) { row in Text(["一", "", "三", "", "五", "", "日"][row]).font(.system(size: 14)).foregroundStyle(.secondary).frame(width: 16, height: size) } }
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
                .font(.system(size: 14)).foregroundStyle(.secondary)
        }
    }
}

struct QuotaCard: View {
    var quota: Quota
    var compact = false
    var sourceName = ""
    @AppStorage private var expanded: Bool
    init(quota: Quota, compact: Bool = false, sourceName: String = "") {
        self.quota = quota; self.compact = compact; self.sourceName = sourceName
        _expanded = AppStorage(wrappedValue: true, "quota.expanded." + quota.id)
    }
    var body: some View {
        fullCard
    }
    @ViewBuilder private var balanceContent: some View {
        if let balances = quota.balances { ForEach(balances) { balance in
            VStack(alignment: .leading, spacing: 8) {
                HStack { Text("可用余额").font(.system(size: 14)).foregroundStyle(.secondary); Spacer(); Text(balance.currency + " " + balance.total).font(.title3.weight(.semibold)).monospacedDigit() }
                if expanded { HStack { Text("赠送 " + balance.granted); Spacer(); Text("充值 " + balance.toppedUp) }.font(.system(size: 14)).foregroundStyle(.secondary) }
            }
        } }
        if quota.isAvailable == false { Text("余额不足").font(.system(size: 14)).foregroundStyle(.orange) }
    }
    private var fullCard: some View {
        Surface {
            Button { expanded.toggle() } label: {
            HStack {
                VStack(alignment: .leading, spacing: 3) { Text(quota.name).font(.system(size: 14, weight: .semibold)); if expanded { Text(quota.plan.map { "\(Format.provider(quota.provider)) · \($0.capitalized)" } ?? Format.provider(quota.provider)).font(.system(size: 14)).foregroundStyle(.secondary) } }
                Spacer()
                if expanded && quota.updatedAt > 0 { Text("\(quota.origin == "log" ? "记录" : "更新") \(Format.date(quota.updatedAt))").font(.system(size: 14)).foregroundStyle(.secondary) }
                Image(systemName: expanded ? "chevron.up" : "chevron.down").font(.system(size: 12, weight: .semibold)).foregroundStyle(.secondary)
            }.contentShape(Rectangle())
            }.buttonStyle(.plain).help(expanded ? "折叠限额" : "展开限额")
                .accessibilityLabel(quota.name + (expanded ? "，折叠限额" : "，展开限额"))
            balanceContent
            if !quota.windows.isEmpty { LazyVGrid(columns: Array(repeating: GridItem(.flexible(), alignment: .leading), count: expanded ? 1 : 2), alignment: .leading, spacing: expanded ? 14 : 8) {
            ForEach(quota.windows) { window in
                VStack(spacing: 6) {
                    ViewThatFits(in: .horizontal) {
                        HStack { Text(window.name); Spacer(minLength: 6); Text("剩余 " + Format.percent(max(0, 100 - window.usedPercent))).monospacedDigit() }
                        VStack(alignment: .leading, spacing: 3) { Text(window.name); Text("剩余 " + Format.percent(max(0, 100 - window.usedPercent))).monospacedDigit() }.frame(maxWidth: .infinity, alignment: .leading)
                    }.font(.system(size: 14))
                    ResourceBar(percent: 100 - window.usedPercent, tint: window.usedPercent >= 90 ? .orange : Palette.accent)
                    if expanded, let reset = window.resetsAt {
                        TimelineView(.periodic(from: .now, by: 60)) { context in
                            HStack {
                                Text("重置 \(Format.date(reset))")
                                Spacer()
                                if reset <= context.date.timeIntervalSince1970 { Text("等待同步") }
                            }.font(.system(size: 14)).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            }
            }
            if expanded && !sourceName.isEmpty { Text(sourceName).font(.system(size: 14)).foregroundStyle(.secondary).lineLimit(1) }
            if expanded, let bank = quota.bankReset {
                Divider().opacity(0.5)
                DisclosureGroup {
                    if let credits = bank.credits { ForEach(credits) { credit in
                        HStack { VStack(alignment: .leading, spacing: 3) { Text(credit.title ?? "Reset Credit").font(.system(size: 14)); Text(["available":"可用", "redeeming":"处理中", "redeemed":"已使用", "unknown":"未知状态"][credit.status] ?? credit.status).font(.system(size: 14)).foregroundStyle(.secondary) }; Spacer(); Text(credit.expiresAt.map { "到期 \(Format.date($0))" } ?? "无到期时间").font(.system(size: 14)).foregroundStyle(.secondary) }
                    } }
                    if let updated = quota.bankUpdatedAt { Text("\(Format.date(updated))").font(.system(size: 14)).foregroundStyle(.secondary) }
                } label: {
                    HStack { Image(systemName: "rectangle.stack").foregroundStyle(Palette.accent); Text("Bank Reset"); Spacer(); Text("\(bank.availableCount) 次可用").foregroundStyle(Palette.accent) }.font(.system(size: 14, weight: .medium))
                }
            }
            if let error = quota.error {
                Text(expanded ? error : "限额更新失败 · 展开查看").font(.system(size: 14)).foregroundStyle(.orange).help(error)
            }
            if quota.windows.isEmpty && (quota.balances ?? []).isEmpty && quota.error == nil { Text("暂无限额数据").font(.system(size: 14)).foregroundStyle(.secondary) }
        }
    }
}

struct ServerCard: View {
    var host: Host, result: HostResult?, compact: Bool
    @AppStorage private var expanded: Bool
    init(host: Host, result: HostResult?, compact: Bool) {
        self.host = host; self.result = result; self.compact = compact
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
                        HStack { Text("负载 " + sample.load.map { String(format: "%.2f", $0) }.joined(separator: " / ")); Spacer(); TimelineView(.periodic(from: .now, by: 1)) { context in Text("" + Date(timeIntervalSince1970: sample.timestamp).formatted(.dateTime.hour().minute().second()) + (context.date.timeIntervalSince1970 - sample.timestamp > 10 ? " · 数据已延迟" : "")) } }.font(.system(size: 14)).foregroundStyle(.secondary)
                    }.padding(.top, 14)
                } else if host.enabled { Text(result?.error == nil ? "等待首次采样" : "连接失败").font(.system(size: 14)).foregroundStyle(.secondary).padding(.top, 10) }
                if let error = result?.error { Text(error).font(.system(size: 14)).foregroundStyle(.orange).padding(.top, 6) }
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: "server.rack").font(.system(size: 20)).foregroundStyle(Palette.accent)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(host.name.isEmpty ? host.target : host.name).font(.headline).lineLimit(1).help(host.name.isEmpty ? host.target : host.name)
                        if expanded { Text(host.target).font(.system(size: 14)).foregroundStyle(.secondary).lineLimit(1).help(host.target) }
                    }
                    Spacer()
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        let label = status(at: context.date)
                        HStack(spacing: 6) { Circle().fill(label == "正常" ? Color.teal : label == "已暂停" ? Color.gray : label == "连接失败" ? Color.red : Color.orange).frame(width: 6, height: 6); Text(label).font(.system(size: 14)).foregroundStyle(.secondary) }
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
                if !sample.errors.isEmpty { Text("部分指标采集失败").font(.system(size: 14)).foregroundStyle(.orange) }
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    if context.date.timeIntervalSince1970 - sample.timestamp > 10 { Text("数据已延迟").font(.system(size: 14)).foregroundStyle(.orange) }
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
            HStack { Text(title).foregroundStyle(.secondary); Spacer(); Text(Format.percent(percent)).monospacedDigit() }.font(.system(size: 14))
            ResourceBar(percent: percent)
        }.frame(maxWidth: .infinity)
    }
    private func metricRow(_ key: String, _ value: String) -> some View { HStack { Text(key).lineLimit(1).truncationMode(.middle); Spacer(minLength: 8); Text(value).monospacedDigit() }.font(.system(size: 14)).foregroundStyle(.secondary).padding(.vertical, 2) }
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
                if let value, value > 0 { Circle().trim(from: 0, to: value / 100).stroke(value >= 90 ? Color.red : value >= 70 ? Color.orange : Palette.accent, style: StrokeStyle(lineWidth: 7, lineCap: .round)).rotationEffect(.degrees(-90)) }
                Text(Format.percent(value)).font(.system(size: 14, weight: .semibold)).monospacedDigit()
            }.frame(width: 72, height: 72)
            Text(title).font(.system(size: 14, weight: .medium)).lineLimit(1)
        }.frame(maxWidth: .infinity, alignment: .leading).accessibilityElement(children: .ignore).accessibilityLabel(title + " " + Format.percent(value))
    }
}

struct ResourceBar: View {
    var percent: Double?
    var tint: Color? = nil
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
