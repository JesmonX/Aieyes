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
                    QuotaRingDetail(model: model, quota: quota, group: entry.item?.group, compact: compact) { withMotion(.expand) { open = nil } }
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
        }.modifier(AccountCardActions(model: model, account: entry.account, opensMainWindow: compact))
            .modifier(AccountFocusHighlight(store: model.accountFocus, account: compact ? "" : entry.account.key, radius: style == .tile ? Palette.radiusCard : 12))
    }
}

/// The selected ring's reset rail and compact valuations, not a second account card.
private struct QuotaRingDetail: View {
    @SurfaceModel var model: AppModel
    let quota: Quota
    let group: String?
    let compact: Bool
    let close: () -> Void
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    private var windows: [QuotaWindow] { quota.windows.filter { group == nil || $0.groupLabel == group } }
    private var groupID: String? { windows.first?.groupId ?? group }
    private var records: [QuotaEstimate] { model.dashboard.presentationEstimates(account: quota.id).filter { group == nil || $0.groupId == groupID } }
    var body: some View {
        let _ = interfaceLanguage
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 7) {
                ProviderMark(provider: quota.provider, size: 16)
                Text(quota.identity.flatMap { $0.email.isEmpty ? $0.providerAccountId : $0.email } ?? quota.name)
                    .font(AppFont.secondary.weight(.medium)).textSelection(.enabled).fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, alignment: .leading)
                if let plan = quota.plan, Format.subscription(plan).count <= 10 { SubscriptionBadge(plan: plan).frame(maxWidth: 76) }
                if model.canSwitchAccount(quota.id) {
                    Button { model.showAccountSwitch?(quota.id) } label: { Image(systemName: "arrow.left.arrow.right") }.help(L("切换账户")).accessibilityLabel(L("切换账户"))
                }
                if compact {
                    Button { model.showAccount(quota.id) } label: { Image(systemName: "macwindow") }.help(L("在主窗口中查看")).accessibilityLabel(L("在主窗口中查看"))
                }
                Button(action: close) { Image(systemName: "chevron.up") }.help(L("收起")).accessibilityLabel(L("收起"))
            }.buttonStyle(.plain)
            if group != nil || quota.plan.map({ Format.subscription($0).count > 10 }) == true {
                FlowLayout(spacing: 6) {
                    if let group { Text(group).font(AppFont.secondary.weight(.medium)).foregroundStyle(.secondary) }
                    if let plan = quota.plan, Format.subscription(plan).count > 10 { SubscriptionBadge(plan: plan) }
                }
            }
            RingDetailColumns {
                if !windows.isEmpty || !(quota.meters ?? []).isEmpty { primaryDetails }
                secondaryDetails
            }
            if let bank = quota.bankReset {
                Divider().opacity(0.5)
                DisclosureGroup {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 180), alignment: .leading)], alignment: .leading, spacing: 6) {
                        ForEach(bank.credits ?? []) { credit in
                            VStack(alignment: .leading, spacing: 2) {
                                Text(credit.title ?? "Reset Credit")
                                Text(L(["available":"可用", "redeeming":"处理中", "redeemed":"已使用"][credit.status] ?? "未知状态") + " · " + (credit.expiresAt.map { Format.date($0) } ?? L("无到期时间"))).foregroundStyle(.secondary)
                            }.font(AppFont.secondary.weight(.medium)).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 3)
                        }
                    }
                    if (bank.credits ?? []).isEmpty { Text(L("未提供")).font(AppFont.secondary.weight(.medium)).foregroundStyle(.secondary) }
                } label: {
                    HStack {
                        Label("Bank Reset", systemImage: "rectangle.stack").foregroundStyle(.secondary)
                        Spacer(minLength: 4)
                        Text(L("{0} 次可用", bank.availableCount)).monospacedDigit()
                    }.font(AppFont.secondary.weight(.medium))
                }.tint(.primary).padding(.vertical, 4)
            }
            if let note = quota.metadataError { Text(L(note)).font(AppFont.secondary.weight(.medium)).foregroundStyle(.secondary) }
            if let error = quota.error {
                ErrorNotice(message: error)
                Button(L("重试")) { Task { await model.refreshQuotas(accountID: quota.accountId) } }
            }
        }.padding(10).modifier(NeutralCard())
    }
    private var primaryDetails: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !windows.isEmpty {
                QuotaDetailGrid(columns: min(2, windows.count)) {
                    ForEach(windows) { window in
                        QuotaDetailMetric(
                            title: window.windowMinutes == 300 ? "5h" : window.windowMinutes == 10080 ? "7d" : window.name,
                            reset: window.resetsAt.map { window.windowMinutes == 300 ? Format.time($0) : Format.date($0) },
                            resetHelp: window.resetsAt.map { Format.date($0) },
                            usedPercent: window.usedPercent.isFinite ? window.usedPercent : nil,
                            healthy: QuotaRingModel.healthy(quota.provider))
                    }
                }.frame(maxHeight: .infinity)
            }
            if isQoderCredits(quota) || quota.provider == "kiro", let meters = quota.meters, !meters.isEmpty {
                QuotaDetailGrid(columns: min(2, meters.count)) {
                    ForEach(meters) { meter in
                        QuotaDetailMetric(title: L(meter.label), reset: meter.resetsAt.map { Format.date($0) },
                                          resetHelp: meter.resetsAt.map { Format.date($0) },
                                          usedPercent: meter.fraction.map { $0 * 100 }, healthy: QuotaRingModel.healthy(quota.provider),
                                          summary: quota.provider == "kiro" && meter.remaining != nil ? meter.remainingSummary : quota.meterSummary(meter), showsMissingReset: false)
                    }
                }.frame(maxHeight: .infinity)
            } else if !(quota.meters ?? []).isEmpty {
                RingDetailColumns {
                    ForEach(quota.meters ?? []) { meter in
                        HStack(spacing: 8) {
                                let share = meter.total.flatMap { $0 > 0 ? max(0, min(1, (meter.remaining ?? 0) / $0)) : nil } ?? 0
                                ZStack {
                                    Circle().stroke(Color.primary.opacity(0.1), lineWidth: 3)
                                    // A meter's share is data: the provider's series colour, as on the rings.
                                    Circle().trim(from: 0, to: share).stroke(QuotaRingModel.healthy(quota.provider), style: StrokeStyle(lineWidth: 3, lineCap: .round)).rotationEffect(.degrees(-90))
                                    Image(systemName: "c.circle").foregroundStyle(QuotaRingModel.healthy(quota.provider))
                                }.frame(width: 28, height: 24)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(L(meter.label)).foregroundStyle(.secondary)
                                Text(quota.meterSummary(meter)).monospacedDigit()
                                if let reset = meter.resetsAt { Label(Format.date(reset), systemImage: "arrow.clockwise") }
                            }.font(AppFont.secondary.weight(.medium)).fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                }
            }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
    private var secondaryDetails: some View {
        VStack(alignment: .leading, spacing: 4) {
            if !windows.isEmpty { estimateValues(records, credits: false) }
            ForEach(quota.balances ?? [], id: \.currency) { balance in
                Label(balance.currency + " " + balance.total + " · " + L("赠送 {0}", balance.granted) + " · " + L("充值 {0}", balance.toppedUp), systemImage: "creditcard").font(AppFont.secondary.weight(.medium))
            }
            if let wallet = quota.extraUsage {
                Label(wallet.amount(wallet.balance) + " · " + wallet.monthlyLimitLabel, systemImage: "creditcard").font(AppFont.secondary.weight(.medium))
            }
            if quota.provider == "codex" {
                Divider().opacity(0.5)
                Button { model.showEstimate?(quota, true, nil) } label: {
                    HStack(spacing: 8) {
                        Label(L("余额"), systemImage: "cylinder.split.1x2").foregroundStyle(.secondary)
                        Spacer(minLength: 4)
                        Text(creditSummary).foregroundStyle(.primary).monospacedDigit().multilineTextAlignment(.trailing).fixedSize(horizontal: false, vertical: true)
                        Image(systemName: "chevron.right").foregroundStyle(.secondary)
                    }.font(AppFont.secondary.weight(.medium)).padding(.vertical, 2).contentShape(Rectangle())
                }.buttonStyle(.plain)
            }
            Divider().opacity(0.5)
            Button { model.openAnalysis(from: compact ? "panel" : "overview", account: quota.id, provider: quota.provider) } label: {
                HStack(spacing: 8) {
                    Label(L("查看分析"), systemImage: "arrow.up.right").foregroundStyle(.secondary)
                    Spacer(minLength: 4)
                    Image(systemName: "chevron.right").foregroundStyle(.secondary)
                }.font(AppFont.secondary.weight(.medium)).padding(.vertical, 2).contentShape(Rectangle())
            }.buttonStyle(.plain)
        }
    }
    private var creditSummary: String {
        let full = Format.creditBalance(quota.credits, estimate: EstimatePresentation.credit(model.dashboard.presentationEstimates(account: quota.id, credits: true)))
        let prefix = L("余额 ")
        return full.hasPrefix(prefix) ? String(full.dropFirst(prefix.count)) : full
    }
    private func estimateValues(_ history: [QuotaEstimate], credits: Bool) -> some View {
        let entries = EstimatePresentation.entries(history, credits: credits, quota: quota, group: groupID)
        let summary = entries.isEmpty ? "—" : credits ? entries.map { $0.label + " ≈ " + Format.compactMoney($0.value) }.joined(separator: " · ") : EstimatePresentation.summary(history, quota: quota, group: groupID)
        return VStack(spacing: 0) {
            if !credits { Divider().opacity(0.5) }
            Button {
                if credits { model.showEstimate?(quota, true, nil) } else { model.showEstimate?(quota, false, groupID) }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "chart.line.uptrend.xyaxis").foregroundStyle(.secondary)
                    Text(L(credits ? "Credits 估值" : "额度估值")).foregroundStyle(.secondary)
                    Spacer(minLength: 4)
                    Text(summary).foregroundStyle(.primary).monospacedDigit()
                        .fixedSize(horizontal: false, vertical: true).multilineTextAlignment(.trailing)
                    Image(systemName: "chevron.right").foregroundStyle(.secondary)
                }.font(AppFont.secondary.weight(.medium)).padding(.vertical, 2).contentShape(Rectangle())
            }.buttonStyle(.plain).help(entries.map { $0.label + " ≈ " + Format.money($0.value) + " USD · " + ($0.historical ? L("历史采样") + " · " : "") + Format.date($0.record.checkpointAt) }.joined(separator: "\n"))
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
            if let symbol = extra.symbol { Image(systemName: symbol).font(.system(size: DesignTokens.fontCaption, weight: .semibold)) }
            Text(extra.text).monospacedDigit().lineLimit(1)
        }.fontWeight(.semibold).foregroundStyle(extra.color)
    }
}

private struct RingTime: View {
    var arc: QuotaRingArc
    var now: Date
    var percent = false
    var body: some View {
        HStack(spacing: 2) {
            if Palette.quotaCritical(arc.used) { Image(systemName: "exclamationmark.triangle").font(.system(size: 10, weight: .semibold)) }
            let text = percent ? QuotaRingModel.remainingText(arc) : QuotaRingModel.resetText(arc, now: now)
            Text(text).monospacedDigit().lineLimit(1).numericTransition(value: text)
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
                        QuotaRingView(kind: item.kind, healthy: QuotaRingModel.healthy(item.quota.provider), lineWidth: compact ? 6 : 6.5, composition: RingBalanceComposition.make(item.quota), gap: 2.5, refreshedAt: item.quota.updatedAt)
                        center(now: context.date).padding(compact ? 9 : 11)
                    }.frame(width: size, height: size).opacity(item.quota.isStale ? 0.6 : 1)
                    HStack(spacing: 4) {
                        ProviderMark(provider: item.quota.provider, size: 14)
                        Text(compactQuotaGroup(item.title)).lineLimit(1).truncationMode(.tail)
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
        if let meter = item.meter {
            VStack(spacing: 3) {
                let text = showPercent ? item.kind.top.map(QuotaRingModel.remainingText) ?? "—" : qoderCreditNumber(meter.remaining)
                Text(text).font(font).monospacedDigit().lineLimit(1).minimumScaleFactor(0.7).numericTransition(value: text)
                if let arc = item.meterReset, arc.resetsAt != nil {
                    Text(QuotaRingModel.resetText(arc, now: now)).font(.system(size: DesignTokens.fontCaption, weight: DesignTokens.weightMedium)).foregroundStyle(.secondary).monospacedDigit()
                }
            }
        } else if isQoderCredits(item.quota), case .duo(let top, let bottom) = item.kind {
            VStack(spacing: 3) {
                Text(showPercent ? QuotaRingModel.remainingText(top) : qoderCreditNumber(item.quota.meters?.first { $0.id == "addOnQuota" }?.remaining))
                Rectangle().fill(Palette.cardBorder.opacity(0.6)).frame(width: 22, height: 1)
                Text(showPercent ? QuotaRingModel.remainingText(bottom) : qoderCreditNumber(item.quota.meters?.first { $0.id == "userQuota" }?.remaining))
            }.font(font).monospacedDigit().lineLimit(1).minimumScaleFactor(0.7)
        } else if let split = RingBalanceComposition.make(item.quota) {
            RingBalanceCenter(split: split, healthy: QuotaRingModel.healthy(item.quota.provider), percent: showPercent).font(font)
        } else {
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
            Text(amount).font(font).monospacedDigit().lineLimit(1).minimumScaleFactor(0.55).foregroundStyle(low ? Palette.danger : Color.primary).numericTransition(value: amount)
        case .empty:
            if item.quota.queryFailure != nil { Text("!").font(.system(size: 17, weight: .bold)).foregroundStyle(Palette.danger) }
            else { Text("—").font(font).foregroundStyle(.secondary) }
            }
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
    private var size: CGFloat { RingBalanceComposition.make(item.quota) != nil ? 72 : compact ? 48 : 56 }
    var body: some View {
        let _ = interfaceLanguage
        SurfaceTimeline(interval: 30) { context in
            Button(action: action) {
                VStack(alignment: .leading, spacing: 5) {
                    HStack(spacing: 5) {
                        ProviderMark(provider: item.quota.provider, size: 14)
                        Text(compactQuotaGroup(item.title)).font(.system(size: DesignTokens.fontCaption + 1, weight: DesignTokens.weightMedium)).lineLimit(1).truncationMode(.tail)
                        Spacer(minLength: 4)
                        if item.group == nil, item.meter == nil, let plan = item.quota.plan, !Format.subscription(plan).isEmpty { Text(Format.subscription(plan)).font(.system(size: DesignTokens.fontSecondary, weight: DesignTokens.weightMedium)).foregroundStyle(.secondary).lineLimit(1).fixedSize() }
                    }
                    HStack(alignment: .center, spacing: 10) {
                        ZStack {
                            QuotaRingView(kind: item.kind, healthy: QuotaRingModel.healthy(item.quota.provider), lineWidth: compact ? 4.5 : 5, composition: RingBalanceComposition.make(item.quota), gap: 2, refreshedAt: item.quota.updatedAt)
                            center
                        }.frame(width: size, height: size).opacity(item.quota.isStale ? 0.6 : 1)
                        if let meter = item.meter {
                            VStack(alignment: .leading, spacing: 3) {
                                Text(qoderCreditNumber(meter.remaining) + "/\u{200B}" + qoderCreditNumber(meter.total)).monospacedDigit().lineLimit(2).fixedSize(horizontal: false, vertical: true)
                                if let arc = item.meterReset, arc.resetsAt != nil {
                                    Text(QuotaRingModel.resetText(arc, now: context.date)).foregroundStyle(.secondary).monospacedDigit()
                                }
                            }.font(.system(size: DesignTokens.fontCaption + 1, weight: DesignTokens.weightMedium))
                        } else if isQoderCredits(item.quota) { RingCreditFacts(quota: item.quota) }
                        else { times(now: context.date).font(.system(size: DesignTokens.fontCaption + 1, weight: DesignTokens.weightMedium)) }
                        let extras = ringExtras(item, now: context.date).prefix(2)
                        if !extras.isEmpty { Spacer(minLength: 0); VStack(alignment: .trailing, spacing: 3) { ForEach(Array(extras)) { RingExtraLabel(extra: $0) } }.font(.system(size: DesignTokens.fontSecondary, weight: DesignTokens.weightStrong)) }
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
                Text(String(format: "%.0f", top.remaining)).foregroundStyle(Palette.quota(top.used)).numericTransition(value: top.remaining)
                Text(String(format: "%.0f", bottom.remaining)).foregroundStyle(Palette.quota(bottom.used)).numericTransition(value: bottom.remaining)
            }.font(font).monospacedDigit()
        case .single(let top):
            Text(String(format: "%.0f", top.remaining)).font(font).monospacedDigit().foregroundStyle(Palette.quota(top.used)).numericTransition(value: top.remaining)
        case .coin:
            if let split = RingBalanceComposition.make(item.quota) { RingBalanceCenter(split: split, healthy: QuotaRingModel.healthy(item.quota.provider)).font(.system(size: DesignTokens.fontCaption, weight: DesignTokens.weightStrong)).padding(7) }
            else { ProviderMark(provider: item.quota.provider, size: size * 0.5) }
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

private struct RingCreditFacts: View {
    let quota: Quota
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            ForEach(["addOnQuota", "userQuota"], id: \.self) { id in
                let meter = quota.meters?.first { $0.id == id }
                Text(qoderCreditNumber(meter?.remaining) + "/" + qoderCreditNumber(meter?.total))
                    .monospacedDigit().lineLimit(1).minimumScaleFactor(0.8)
                    .help(id == "userQuota" ? "Plan credits" : "Addon credits")
                    .accessibilityLabel((id == "userQuota" ? "Plan credits " : "Addon credits ") + qoderCreditNumber(meter?.remaining) + "/" + qoderCreditNumber(meter?.total))
            }
        }.font(.system(size: DesignTokens.fontCaption + 1, weight: DesignTokens.weightMedium))
    }
}

private struct RingBalanceCenter: View {
    let split: RingBalanceComposition
    let healthy: Color
    var percent = false
    var body: some View {
        VStack(spacing: 3) {
            Text(percent ? String(format: "%.0f%%", split.share * 100) : split.granted).foregroundStyle(RingBalanceComposition.grantedTint).help(L("赠送 {0}", split.granted))
            Rectangle().fill(Palette.cardBorder.opacity(0.6)).frame(width: 22, height: 1)
            Text(percent ? String(format: "%.0f%%", (1 - split.share) * 100) : split.paid).foregroundStyle(healthy).help(L("充值 {0}", split.paid))
        }.monospacedDigit().lineLimit(1).minimumScaleFactor(0.7)
    }
}

/// Reflows existing detail groups without reserving an empty column for sparse accounts.
struct RingDetailColumns: Layout {
    var spacing: CGFloat = 12
    private func columns(_ width: CGFloat, _ count: Int) -> Int { max(1, min(count, width >= 600 ? 2 : 1)) }
    private func rows(_ width: CGFloat, _ subviews: Subviews) -> (cell: CGFloat, rows: [[Int]]) {
        let cols = columns(width, subviews.count), cell = max(0, (width - spacing * CGFloat(cols - 1)) / CGFloat(cols))
        return (cell, stride(from: 0, to: subviews.count, by: cols).map { Array($0..<min($0 + cols, subviews.count)) })
    }
    private func naturalHeight(_ row: [Int], cell: CGFloat, _ subviews: Subviews) -> CGFloat {
        row.map { subviews[$0].sizeThatFits(.init(width: cell, height: nil)).height }.max() ?? 0
    }
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 320, layout = rows(width, subviews)
        let natural = layout.rows.enumerated().reduce(CGFloat(0)) { $0 + naturalHeight($1.element, cell: layout.cell, subviews) + ($1.offset > 0 ? spacing : 0) }
        return CGSize(width: width, height: natural)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let layout = rows(bounds.width, subviews), natural = layout.rows.enumerated().reduce(CGFloat(0)) { $0 + naturalHeight($1.element, cell: layout.cell, subviews) + ($1.offset > 0 ? spacing : 0) }
        let share = layout.rows.isEmpty ? 0 : max(0, bounds.height - natural) / CGFloat(layout.rows.count)
        var y = bounds.minY
        for row in layout.rows {
            let height = naturalHeight(row, cell: layout.cell, subviews) + share
            for (offset, index) in row.enumerated() {
                subviews[index].place(at: CGPoint(x: bounds.minX + CGFloat(offset) * (layout.cell + spacing), y: y), anchor: .topLeading, proposal: .init(width: layout.cell, height: height))
            }
            y += height + spacing
        }
    }
}

/// Metric cards keep their natural size while content fits, then share any extra height the facts column adds.
private struct QuotaDetailGrid: Layout {
    var columns: Int
    var spacing: CGFloat = 8
    private func rows(_ width: CGFloat, _ subviews: Subviews) -> (cell: CGFloat, rows: [[Int]]) {
        let cols = max(1, min(columns, subviews.count)), cell = max(0, (width - spacing * CGFloat(cols - 1)) / CGFloat(cols))
        return (cell, stride(from: 0, to: subviews.count, by: cols).map { Array($0..<min($0 + cols, subviews.count)) })
    }
    private func naturalHeight(_ row: [Int], cell: CGFloat, _ subviews: Subviews) -> CGFloat {
        row.map { subviews[$0].sizeThatFits(.init(width: cell, height: nil)).height }.max() ?? 0
    }
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 320, layout = rows(width, subviews)
        let natural = layout.rows.enumerated().reduce(CGFloat(0)) { $0 + naturalHeight($1.element, cell: layout.cell, subviews) + ($1.offset > 0 ? spacing : 0) }
        return CGSize(width: width, height: proposal.height.map { max(natural, $0) } ?? natural)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let layout = rows(bounds.width, subviews), natural = layout.rows.enumerated().reduce(CGFloat(0)) { $0 + naturalHeight($1.element, cell: layout.cell, subviews) + ($1.offset > 0 ? spacing : 0) }
        let share = layout.rows.isEmpty ? 0 : max(0, bounds.height - natural) / CGFloat(layout.rows.count)
        var y = bounds.minY
        for row in layout.rows {
            let height = naturalHeight(row, cell: layout.cell, subviews) + share
            for (offset, index) in row.enumerated() {
                subviews[index].place(at: CGPoint(x: bounds.minX + CGFloat(offset) * (layout.cell + spacing), y: y), anchor: .topLeading, proposal: .init(width: layout.cell, height: height))
            }
            y += height + spacing
        }
    }
}

/// Shared expanded quota styling for time windows and credit buckets.
private struct QuotaDetailMetric: View {
    let title: String
    let reset: String?
    let resetHelp: String?
    let usedPercent: Double?
    let healthy: Color
    var summary: String? = nil
    var showsMissingReset = true
    private var remaining: Double? { usedPercent.map { max(0, min(100, 100 - $0)) } }
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                Text(title)
                Spacer(minLength: 0)
                if let reset {
                    Text(reset).monospacedDigit().multilineTextAlignment(.trailing)
                        .fixedSize(horizontal: false, vertical: true).help(resetHelp ?? reset)
                } else if showsMissingReset {
                    Text(L("未提供")).monospacedDigit().multilineTextAlignment(.trailing)
                        .fixedSize(horizontal: false, vertical: true).help(L("未提供"))
                }
            }.font(AppFont.secondary.weight(.medium)).foregroundStyle(.secondary)
            Spacer(minLength: 4)
            Text(Format.percent(remaining)).font(.system(size: 19, weight: .semibold)).monospacedDigit()
                .accessibilityLabel(L("剩余 {0}", Format.percent(remaining)))
            Spacer(minLength: 4)
            ResourceBar(percent: remaining, tint: Palette.quotaBar(usedPercent ?? 0, healthy: healthy))
            if let summary {
                Spacer(minLength: 4)
                Text(summary).font(AppFont.secondary.weight(.medium)).monospacedDigit().fixedSize(horizontal: false, vertical: true)
            }
        }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).padding(8)
            .background(Palette.accent.opacity(0.06), in: RoundedRectangle(cornerRadius: 9))
    }
}
