import SwiftUI

struct NativeCreditsView: View {
    let usage: Aggregate
    var models: [Aggregate] = []
    var compact = false
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(usage.nativeCreditText).font(AppFont.secondary).fontWeight(.medium).textSelection(.enabled)
            if !compact {
                ForEach((usage.nativeCredits ?? [:]).keys.sorted(), id: \.self) { provider in
                    if let row = usage.nativeCredits?[provider] { Text(Format.provider(provider) + "：" + row.detail).font(AppFont.caption).foregroundStyle(.secondary).textSelection(.enabled) }
                }
            }
            Text("来自原生计量记录；不代表已扣费金额或账户剩余额度。" + (usage.total == 0 ? "原生记录没有可用的 Token 统计，API 等价成本暂不可计算。" : "")).font(AppFont.caption).foregroundStyle(.secondary)
            if !compact && models.contains(where: \.hasNativeCredits) {
                DisclosureGroup("按模型查看 Credits") {
                    ForEach(models.filter(\.hasNativeCredits)) { row in Text(row.key + " · " + row.nativeCreditText).font(AppFont.secondary).textSelection(.enabled) }
                }.font(AppFont.secondary)
            }
        }
    }
}

struct PricingWarning: View {
    var action: () -> Void
    @FocusState private var focused: Bool
    @State private var showingHint = false
    private let hint = "计价未完成：部分模型缺少价格，点击设置模型价格。"
    var body: some View {
        Button { showingHint = false; action() } label: {
            Image(systemName: "exclamationmark.circle").foregroundStyle(Palette.warn)
        }.buttonStyle(.plain).focused($focused).help(hint)
            .accessibilityLabel("计价未完成，设置模型价格")
            .onChange(of: focused) { _, value in showingHint = value }
            .popover(isPresented: $showingHint) { Text(hint).font(AppFont.secondary).padding(12).frame(maxWidth: 260) }
    }
}

struct ProviderMark: View {
    var provider: String
    var size: CGFloat = DesignTokens.providerMarkSize
    @Environment(\.colorScheme) private var colorScheme
    private static let images: [String: NSImage] = {
        let source = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("desktop/web")
        return Set(Array(CoreContract.providerIcons.values) + Array(CoreContract.providerDarkIcons.values)).reduce(into: [:]) { result, key in
            let name = "provider-" + key
            let url = Bundle.main.url(forResource: name, withExtension: "png") ?? source.appendingPathComponent(name + ".png")
            if let image = NSImage(contentsOf: url) { result[key] = image }
        }
    }()
    var body: some View {
        Group {
            if let asset = (colorScheme == .dark ? CoreContract.providerDarkIcons[provider] : nil) ?? CoreContract.providerIcons[provider], let image = Self.images[asset] {
                Image(nsImage: image).resizable().renderingMode(CoreContract.monochromeIcons.contains(provider) ? .template : .original).scaledToFit()
            } else { Image(systemName: "terminal").resizable().scaledToFit().frame(width: size * 2 / 3, height: size * 2 / 3).foregroundStyle(.secondary) }
        }.frame(width: size, height: size).help(Format.provider(provider)).accessibilityLabel(Format.provider(provider))
    }
}
struct ProviderIdentity: View {
    var provider: String
    var markSize: CGFloat = DesignTokens.providerMarkSize
    var body: some View { HStack(spacing: 5) { ProviderMark(provider: provider, size: markSize); Text(Format.provider(provider)) } }
}
struct SubscriptionBadge: View {
    var plan: String
    var body: some View {
        Text(Format.subscription(plan)).font(.system(size: DesignTokens.fontSecondary, weight: DesignTokens.weightStrong)).lineLimit(1)
            .padding(.horizontal, 6).padding(.vertical, 3)
            .foregroundStyle(Palette.accent).background(Palette.accent.opacity(0.12), in: Capsule())
            .help(plan).accessibilityLabel("订阅 " + Format.subscription(plan))
    }
}
struct CreditBalanceLabel: View {
    var balance: CreditsBalance?
    var estimate: QuotaEstimate?
    var compact = false
    var body: some View {
        HStack(spacing: 4) {
            Label("余额", systemImage: "c.circle").foregroundStyle(Palette.balanceBlue)
            Text(compact ? Format.creditPrimary(balance) : String(Format.creditBalance(balance, estimate: estimate).dropFirst(3)))
                .monospacedDigit().lineLimit(compact ? 1 : nil)
        }.font(AppFont.secondary).help(Format.creditBalance(balance, estimate: estimate))
    }
}
struct QuotaResetLabel: View {
    var window: QuotaWindow
    var body: some View {
        SurfaceTimeline(interval: 30) { context in
            Text(Format.quotaReset(window, now: context.date)).font(AppFont.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
        }
    }
}

/// Wrapping chips preserve their intrinsic label width without shrinking numeric content.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrange(subviews, width: proposal.width ?? 600).size
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let layout = arrange(subviews, width: bounds.width)
        for (index, point) in layout.points.enumerated() {
            subviews[index].place(at: CGPoint(x: bounds.minX + point.x, y: bounds.minY + point.y), proposal: ProposedViewSize(width: min(subviews[index].sizeThatFits(.unspecified).width, bounds.width), height: nil))
        }
    }
    private func arrange(_ subviews: Subviews, width: CGFloat) -> (size: CGSize, points: [CGPoint]) {
        var x: CGFloat = 0, y: CGFloat = 0, height: CGFloat = 0, points: [CGPoint] = []
        for view in subviews {
            let size = view.sizeThatFits(ProposedViewSize(width: width, height: nil))
            if x > 0 && x + size.width > width { x = 0; y += height + spacing; height = 0 }
            points.append(CGPoint(x: x, y: y)); x += size.width + spacing; height = max(height, size.height)
        }
        return (CGSize(width: width, height: y + height), points)
    }
}

struct EstimateEntry: View {
    var records: [QuotaEstimate]
    var credits = false
    var quota: Quota? = nil
    var group: String? = nil
    var action: () -> Void
    private var entries: [EstimatePresentation.Entry] { EstimatePresentation.entries(records, credits: credits, quota: quota, group: group) }
    private var latest: QuotaEstimate? { EstimatePresentation.canonical(records).first }
    private var label: String {
        if credits { return "credits 估值（1000 credits ≈ " + Format.compactMoney(entries.first?.value) + "）" }
        return EstimatePresentation.summary(records, quota: quota, group: group)
    }
    private var help: String {
        let rolling = latest?.samplingMode == "rolling" ? (latest?.calculationNote ?? "") + "\n" : ""
        let lines: [String] = entries.map { entry in
            let amount = Format.money(entry.value)
            let historical = entry.historical ? "历史采样 · " : ""
            let date = Format.date(entry.record.checkpointAt)
            return "\(entry.label) ≈ \(amount) USD · \(historical)\(date)"
        }
        let visibility = credits ? "" : "\n" + EstimatePresentation.visibility(quota, records: records, group: group).help
        return rolling + lines.joined(separator: "\n") + visibility
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Button(action: action) {
                HStack(spacing: 5) {
                    Image(systemName: "chart.line.uptrend.xyaxis")
                    Text(label).monospacedDigit().lineLimit(1).minimumScaleFactor(0.85).layoutPriority(1)
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                }.font(.system(size: DesignTokens.fontEntry)).frame(minHeight: 30).contentShape(Rectangle())
            }.buttonStyle(.plain).foregroundStyle(Palette.accent).help(help).accessibilityLabel(label + "，" + help)
            if latest?.samplingMode == "rolling" { Text("滚动估值 · " + String(latest?.sampleCount ?? 0) + " 个有效片段").font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary).help(help) }
            if entries.contains(where: { $0.record.originalEstimateId != nil }) { Text("已修正").font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary) }
            if entries.contains(where: \.historical) { Text("历史采样").font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary).help(help) }
            if let latest, !latest.hasValue || latest.status == "pending" {
                Text("最新采样 · " + latest.statusLabel + " · " + latest.issueLabel).font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}
