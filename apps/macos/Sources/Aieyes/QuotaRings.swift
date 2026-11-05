import SwiftUI

/// Rings in place of the account cards. Tiles flow in columns; tapping one opens its full
/// card right below the tile's row, so switching accounts, estimates and analysis stay one tap away.
struct QuotaRingSection: View {
    let model: AppModel
    let style: QuotaCardStyle
    let compact: Bool
    let accounts: [AgentAccount]
    let quotas: [Quota]
    // Snapshot renders can open one tile (AIEYES_UI_RING_OPEN) to check the inline card.
    @State private var open: String? = CommandLine.arguments.contains("--render") ? ProcessInfo.processInfo.environment["AIEYES_UI_RING_OPEN"] : nil
    private struct Entry: Identifiable { let id: String; let account: AgentAccount; let item: QuotaRingItem? }
    private var entries: [Entry] {
        accounts.flatMap { account -> [Entry] in
            guard let quota = quotas.first(where: { $0.id == account.key }) else { return [Entry(id: account.key, account: account, item: nil)] }
            return QuotaRingModel.items(quota).map { Entry(id: $0.id, account: account, item: $0) }
        }
    }
    var body: some View {
        let entries = entries
        QuotaTileLayout(minimumWidth: style == .dial ? 96 : compact ? 180 : 240, maximumColumns: style == .tile && compact ? 2 : .max,
                        spacing: style == .dial ? 4 : 8, rowSpacing: 8) {
            ForEach(entries) { entry in
                tile(entry).id(entry.id)
                if open == entry.id, let quota = entry.item?.quota {
                    QuotaRingDetail(model: model, quota: quota, compact: compact) { withMotion(.expand) { open = nil } }
                        .layoutValue(key: QuotaFullWidth.self, value: true).id(entry.id + ".detail")
                        .transition(.opacity)
                }
            }
        }
        .motion(.expand, value: open)
        .onChange(of: entries.map(\.id)) { _, ids in if let open, !ids.contains(open) { self.open = nil } }
    }
    @ViewBuilder private func tile(_ entry: Entry) -> some View {
        let selected = open == entry.id
        let toggle = { withMotion(.expand) { open = selected ? nil : entry.id } }
        Group {
            if let item = entry.item {
                switch style {
                case .tile: QuotaRingTile(item: item, compact: compact, selected: selected, action: toggle)
                default: QuotaDialTile(item: item, compact: compact, selected: selected, action: toggle)
                }
            } else {
                QuotaRingAwaitingTile(model: model, account: entry.account, style: style, compact: compact)
            }
        }.modifier(AccountCardActions(model: model, account: entry.account))
    }
}

/// The opened account card. It always shows every detail; the close button or the tile folds it.
private struct QuotaRingDetail: View {
    let model: AppModel
    let quota: Quota
    let compact: Bool
    let close: () -> Void
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage
        AccountObservedQuota(model: model, quota: quota, compact: compact, collapsible: false)
            .overlay(alignment: .topTrailing) {
                Button(action: close) { Image(systemName: "xmark").font(.system(size: DesignTokens.fontCaption, weight: DesignTokens.weightStrong)).frame(width: 24, height: 24).contentShape(Rectangle()) }
                    .buttonStyle(.plain).foregroundStyle(.secondary).padding(compact ? 8 : 10)
                    .help(L("收起")).accessibilityLabel(L("收起"))
            }
    }
}

/// Small extra facts beside a ring: credits, reset cards, balance, hidden windows, data age.
private struct RingExtra: Identifiable {
    var id: String, text: String, symbol: String? = nil, color: Color = .secondary
}
private func ringExtras(_ item: QuotaRingItem, now: Date = Date()) -> [RingExtra] {
    var out: [RingExtra] = []
    let quota = item.quota
    if quota.provider == "codex" {
        if let credits = quota.credits, credits.hasCredits || credits.unlimited || credits.balance != nil { out.append(RingExtra(id: "credits", text: compactCredits(credits), symbol: "c.circle")) }
        if let bank = quota.bankReset, bank.availableCount > 0 { out.append(RingExtra(id: "bank", text: String(bank.availableCount), symbol: "arrow.counterclockwise")) }
    }
    if item.kind.top != nil, item.group == nil, let balance = quota.balances?.first { out.append(RingExtra(id: "balance", text: balance.currency + " " + balance.total)) }
    if !item.hidden.isEmpty {
        let lowest = item.hidden.map(\.usedPercent).filter(\.isFinite).max()
        out.append(RingExtra(id: "more", text: "+\(item.hidden.count)", color: lowest.map { Palette.quotaCritical($0) || 100 - $0 <= 30 ? Palette.quota($0) : .secondary } ?? .secondary))
    }
    if quota.isStale, let age = quota.snapshotAge { out.append(RingExtra(id: "stale", text: L("上次 {0}", Format.relative(Date(timeIntervalSince1970: age), now: now)), color: Palette.warn)) }
    return out
}
/// Credits in a few characters: 990, 1.2k, 12k.
private func compactCredits(_ credits: CreditsBalance) -> String {
    guard !credits.unlimited, let raw = credits.balance, let value = Double(raw.replacingOccurrences(of: ",", with: "")), value.isFinite else { return Format.creditPrimary(credits) }
    if value >= 10_000 { return String(format: "%.0fk", (value / 1000).rounded(.down)) }
    if value >= 1_000 { return String(format: "%.1fk", (value / 100).rounded(.down) / 10) }
    return String(format: value >= 100 ? "%.0f" : "%.1f", value.rounded(.down))
}
private struct RingExtraLabel: View {
    var extra: RingExtra
    var body: some View {
        HStack(spacing: 2) {
            if let symbol = extra.symbol { Image(systemName: symbol).font(.system(size: 10)) }
            Text(extra.text).monospacedDigit().lineLimit(1)
        }.foregroundStyle(extra.color)
    }
}

private struct RingTime: View {
    var arc: QuotaRingArc
    var now: Date
    var percent = false
    var body: some View {
        HStack(spacing: 2) {
            if Palette.quotaCritical(arc.used) { Image(systemName: "exclamationmark.triangle").font(.system(size: 10, weight: .semibold)) }
            Text(percent ? QuotaRingModel.remainingText(arc) : QuotaRingModel.resetText(arc, now: now)).monospacedDigit().lineLimit(1)
        }.foregroundStyle(Palette.quota(arc.used))
    }
}

private struct RingTileBackground: ViewModifier {
    var selected: Bool, hovering: Bool, card: Bool
    func body(content: Content) -> some View {
        if card {
            content.modifier(NeutralCard())
                .overlay { if selected { RoundedRectangle(cornerRadius: Palette.radiusCard).strokeBorder(Palette.accent, lineWidth: 1.5) } }
                .overlay { if hovering && !selected { RoundedRectangle(cornerRadius: Palette.radiusCard).fill(Color.primary.opacity(0.04)).allowsHitTesting(false) } }
        } else {
            content.background(RoundedRectangle(cornerRadius: 12).fill(selected ? Palette.accent.opacity(DesignTokens.controlTintOpacity) : hovering ? Color.primary.opacity(0.05) : .clear))
        }
    }
}

/// B: a large ring with the reset times written inside, next to their halves. Hover shows percentages.
struct QuotaDialTile: View {
    let item: QuotaRingItem
    let compact: Bool
    let selected: Bool
    let action: () -> Void
    @State private var hovering = false
    @FocusState private var focused: Bool
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    private var size: CGFloat { compact ? 72 : 84 }
    var body: some View {
        let _ = interfaceLanguage
        SurfaceTimeline(interval: 30) { context in
            Button(action: action) {
                VStack(spacing: 4) {
                    ZStack {
                        QuotaRingView(kind: item.kind, healthy: QuotaRingModel.healthy(item.quota.provider), lineWidth: compact ? 6 : 6.5, gap: 2.5)
                        center(now: context.date).padding(compact ? 9 : 11)
                    }.frame(width: size, height: size).opacity(item.quota.isStale ? 0.6 : 1)
                    HStack(spacing: 4) {
                        ProviderMark(provider: item.quota.provider, size: 14)
                        Text(item.title).lineLimit(1).truncationMode(.tail)
                    }.font(.system(size: DesignTokens.fontCaption)).frame(maxWidth: .infinity)
                    let extras = ringExtras(item, now: context.date).prefix(2)
                    if !extras.isEmpty { HStack(spacing: 6) { ForEach(Array(extras)) { RingExtraLabel(extra: $0) } }.font(.system(size: DesignTokens.fontCaption)) }
                }
                .padding(.vertical, 4).padding(.horizontal, 2).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain).focused($focused)
            .modifier(RingTileBackground(selected: selected, hovering: hovering, card: false))
            .onHover { hovering = $0 }
            .help(QuotaRingModel.accessibility(item, now: context.date))
            .accessibilityLabel(QuotaRingModel.accessibility(item, now: context.date))
            .accessibilityValue(selected ? L("已展开") : L("已折叠"))
        }
    }
    @ViewBuilder private func center(now: Date) -> some View {
        let showPercent = hovering || focused
        let font = Font.system(size: compact ? DesignTokens.fontCaption + 1 : DesignTokens.fontSecondary, weight: DesignTokens.weightStrong)
        switch item.kind {
        case .duo(let top, let bottom):
            VStack(spacing: 3) {
                RingTime(arc: top, now: now, percent: showPercent)
                Rectangle().fill(Palette.cardBorder.opacity(0.6)).frame(width: 22, height: 1)
                RingTime(arc: bottom, now: now, percent: showPercent)
            }.font(font).minimumScaleFactor(0.7)
        case .single(let top):
            VStack(spacing: 2) {
                RingTime(arc: top, now: now, percent: true).font(font)
                Text(QuotaRingModel.resetText(top, now: now)).font(.system(size: DesignTokens.fontCaption, weight: DesignTokens.weightMedium)).foregroundStyle(.secondary).monospacedDigit()
            }.minimumScaleFactor(0.7)
        case .coin(let amount, let low):
            Text(amount).font(font).monospacedDigit().lineLimit(1).minimumScaleFactor(0.55).foregroundStyle(low ? Palette.danger : Color.primary)
        case .empty:
            if item.quota.queryFailure != nil { Text("!").font(.system(size: 17, weight: .bold)).foregroundStyle(Palette.danger) }
            else { Text("—").font(font).foregroundStyle(.secondary) }
        }
    }
}

/// E: a small card per ring; remaining percentages inside the ring, reset times beside it.
struct QuotaRingTile: View {
    let item: QuotaRingItem
    let compact: Bool
    let selected: Bool
    let action: () -> Void
    @State private var hovering = false
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    private var size: CGFloat { compact ? 48 : 56 }
    var body: some View {
        let _ = interfaceLanguage
        SurfaceTimeline(interval: 30) { context in
            Button(action: action) {
                VStack(alignment: .leading, spacing: 5) {
                    HStack(spacing: 5) {
                        ProviderMark(provider: item.quota.provider, size: 14)
                        Text(item.title).font(.system(size: DesignTokens.fontCaption + 1, weight: DesignTokens.weightMedium)).lineLimit(1).truncationMode(.tail)
                        Spacer(minLength: 4)
                        if let plan = item.quota.plan, !Format.subscription(plan).isEmpty { Text(Format.subscription(plan)).font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary).lineLimit(1).fixedSize() }
                    }
                    HStack(alignment: .center, spacing: 10) {
                        ZStack {
                            QuotaRingView(kind: item.kind, healthy: QuotaRingModel.healthy(item.quota.provider), lineWidth: compact ? 4.5 : 5, gap: 2)
                            center
                        }.frame(width: size, height: size).opacity(item.quota.isStale ? 0.6 : 1)
                        times(now: context.date).font(.system(size: DesignTokens.fontCaption + 1, weight: DesignTokens.weightMedium))
                        Spacer(minLength: 0)
                        let extras = ringExtras(item, now: context.date).prefix(2)
                        if !extras.isEmpty { VStack(alignment: .trailing, spacing: 3) { ForEach(Array(extras)) { RingExtraLabel(extra: $0) } }.font(.system(size: DesignTokens.fontCaption)) }
                    }
                }
                .padding(.horizontal, 10).padding(.vertical, 8).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .modifier(RingTileBackground(selected: selected, hovering: hovering, card: true))
            .onHover { hovering = $0 }
            .help(QuotaRingModel.accessibility(item, now: context.date))
            .accessibilityLabel(QuotaRingModel.accessibility(item, now: context.date))
            .accessibilityValue(selected ? L("已展开") : L("已折叠"))
        }
    }
    @ViewBuilder private var center: some View {
        let font = Font.system(size: compact ? DesignTokens.fontCaption : DesignTokens.fontCaption + 1, weight: DesignTokens.weightStrong)
        switch item.kind {
        case .duo(let top, let bottom):
            VStack(spacing: 0) {
                Text(String(format: "%.0f", top.remaining)).foregroundStyle(Palette.quota(top.used))
                Text(String(format: "%.0f", bottom.remaining)).foregroundStyle(Palette.quota(bottom.used))
            }.font(font).monospacedDigit()
        case .single(let top):
            Text(String(format: "%.0f", top.remaining)).font(font).monospacedDigit().foregroundStyle(Palette.quota(top.used))
        case .coin: ProviderMark(provider: item.quota.provider, size: size * 0.5)
        case .empty:
            if item.quota.queryFailure != nil { Text("!").font(.system(size: 15, weight: .bold)).foregroundStyle(Palette.danger) }
            else { Text("—").font(font).foregroundStyle(.secondary) }
        }
    }
    @ViewBuilder private func times(now: Date) -> some View {
        switch item.kind {
        case .duo(let top, let bottom):
            VStack(alignment: .leading, spacing: 3) { RingTime(arc: top, now: now); RingTime(arc: bottom, now: now) }
        case .single(let top):
            RingTime(arc: top, now: now)
        case .coin(let amount, let low):
            VStack(alignment: .leading, spacing: 3) {
                Text(amount).monospacedDigit().lineLimit(1).minimumScaleFactor(0.7).foregroundStyle(low ? Palette.danger : Color.primary)
                if low { Text(L("余额不足")).font(.system(size: DesignTokens.fontCaption)).foregroundStyle(Palette.danger) }
            }
        case .empty:
            VStack(alignment: .leading, spacing: 3) {
                if item.quota.queryFailure != nil { Text(L("额度更新失败")).foregroundStyle(Palette.danger); Text(L("重试")).font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary) }
                else { Text(L("暂无额度数据")).foregroundStyle(.secondary) }
            }.lineLimit(1)
        }
    }
}

/// An account still waiting for its first reading, in the shape of the chosen tile.
private struct QuotaRingAwaitingTile: View {
    @SurfaceModel var model: AppModel
    let account: AgentAccount
    let style: QuotaCardStyle
    let compact: Bool
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage
        let status = model.quotaBusy ? L("正在读取账户额度…") : L("等待首次额度查询")
        let ring = QuotaRingView(kind: .empty, healthy: .secondary, lineWidth: style == .dial ? 6 : 4.5)
        Group {
            if style == .dial {
                VStack(spacing: 4) {
                    ring.frame(width: compact ? 72 : 84, height: compact ? 72 : 84)
                    HStack(spacing: 4) { ProviderMark(provider: account.provider, size: 14); Text(account.name).lineLimit(1) }.font(.system(size: DesignTokens.fontCaption))
                    Text(status).font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary).lineLimit(1)
                }.padding(.vertical, 4).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            } else {
                VStack(alignment: .leading, spacing: 5) {
                    HStack(spacing: 5) { ProviderMark(provider: account.provider, size: 14); Text(account.name).font(.system(size: DesignTokens.fontCaption + 1, weight: DesignTokens.weightMedium)).lineLimit(1) }
                    HStack(spacing: 10) { ring.frame(width: compact ? 48 : 56, height: compact ? 48 : 56); Text(status).font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary).lineLimit(2) }
                }.padding(.horizontal, 10).padding(.vertical, 8).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).modifier(NeutralCard())
            }
        }
        .modifier(LoadingPlaceholder(active: model.quotaBusy))
        .accessibilityElement(children: .combine).accessibilityLabel(account.name + "，" + status)
    }
}

/// Tiles in equal columns. A full-width view (the opened card) waits until the current row
/// is complete, so the tiles after the opened one still fill that row first.
struct QuotaTileLayout: Layout {
    var minimumWidth: CGFloat
    var maximumColumns: Int = .max
    var spacing: CGFloat = 8
    var rowSpacing: CGFloat = 8
    private func rows(_ count: Int, columns: Int, full: (Int) -> Bool) -> [[Int]] {
        var rows: [[Int]] = [], current: [Int] = [], pending: [Int] = []
        for index in 0..<count {
            if full(index) { if current.isEmpty { rows.append([index]) } else { pending.append(index) }; continue }
            current.append(index)
            if current.count == columns { rows.append(current); current = []; rows += pending.map { [$0] }; pending = [] }
        }
        if !current.isEmpty { rows.append(current) }
        return rows + pending.map { [$0] }
    }
    private func arrange(_ width: CGFloat, _ subviews: Subviews) -> (CGSize, [(Int, CGRect)]) {
        let columns = max(1, min(maximumColumns, Int((width + spacing) / (minimumWidth + spacing))))
        let column = max(0, (width - spacing * CGFloat(columns - 1)) / CGFloat(columns))
        var frames: [(Int, CGRect)] = [], y: CGFloat = 0
        for row in rows(subviews.count, columns: columns, full: { subviews[$0][QuotaFullWidth.self] }) {
            if row.count == 1, subviews[row[0]][QuotaFullWidth.self] {
                let height = subviews[row[0]].sizeThatFits(ProposedViewSize(width: width, height: nil)).height
                frames.append((row[0], CGRect(x: 0, y: y, width: width, height: height))); y += height + rowSpacing
                continue
            }
            let height = row.map { subviews[$0].sizeThatFits(ProposedViewSize(width: column, height: nil)).height }.max() ?? 0
            for (offset, index) in row.enumerated() { frames.append((index, CGRect(x: CGFloat(offset) * (column + spacing), y: y, width: column, height: height))) }
            y += height + rowSpacing
        }
        return (CGSize(width: width, height: max(0, y - rowSpacing)), frames)
    }
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrange(proposal.width ?? minimumWidth * 4, subviews).0
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for (index, frame) in arrange(bounds.width, subviews).1 {
            subviews[index].place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY), proposal: ProposedViewSize(width: frame.width, height: frame.height))
        }
    }
}

/// Card / dial / two-column switch beside the quota heading.
struct QuotaStylePicker: View {
    @Binding var selection: String
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage
        Picker(L("额度显示样式"), selection: Binding(get: { QuotaCardStyle(rawValue: selection) ?? .card }, set: { style in withMotion(.standard) { selection = style.rawValue } })) {
            ForEach(QuotaCardStyle.allCases) { style in
                Image(systemName: style.symbol).help(style.help).accessibilityLabel(style.help).tag(style)
            }
        }
        .pickerStyle(.segmented).labelsHidden().fixedSize().help(L("额度显示样式"))
    }
}

/// "⌒ 5h  ⌣ 7d": which half is which, shown with the rings.
struct QuotaRingLegend: View {
    private func half(top: Bool) -> some View {
        Canvas { context, size in
            var path = Path()
            let radius = size.width / 2 - 1
            path.addArc(center: CGPoint(x: size.width / 2, y: top ? size.height - 1 : 1), radius: radius, startAngle: .degrees(top ? 180 : 180), endAngle: .degrees(top ? 360 : 0), clockwise: !top)
            context.stroke(path, with: .color(.secondary), style: StrokeStyle(lineWidth: 1.5, lineCap: .round))
        }.frame(width: 12, height: 7)
    }
    var body: some View {
        HStack(spacing: 3) { half(top: true); Text("5h"); Spacer().frame(width: 4); half(top: false); Text("7d") }
            .font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary).accessibilityHidden(true)
    }
}
