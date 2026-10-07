import SwiftUI

struct ProviderMark: View {
    var provider: String
    private static let images: [String: NSImage] = {
        let source = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("desktop/web")
        return ["codex", "claude", "antigravity", "deepseek"].reduce(into: [:]) { result, key in
            let name = "provider-" + key
            let url = Bundle.main.url(forResource: name, withExtension: "png") ?? source.appendingPathComponent(name + ".png")
            if let image = NSImage(contentsOf: url) { result[key] = image }
        }
    }()
    var body: some View {
        Group {
            if let image = Self.images[provider == "agy" ? "antigravity" : provider] {
                Image(nsImage: image).resizable().renderingMode(provider == "codex" ? .template : .original).scaledToFit()
            } else { Image(systemName: "terminal").resizable().scaledToFit().foregroundStyle(.secondary) }
        }.frame(width: 36, height: 36).help(Format.provider(provider)).accessibilityLabel(Format.provider(provider))
    }
}
struct ProviderIdentity: View {
    var provider: String
    var body: some View { HStack(spacing: 5) { ProviderMark(provider: provider); Text(Format.provider(provider)) } }
}
struct SubscriptionBadge: View {
    var plan: String
    var body: some View {
        Text(Format.subscription(plan)).font(.system(size: 14, weight: .semibold)).lineLimit(1)
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
        TimelineView(.periodic(from: .now, by: 30)) { context in
            Text(Format.quotaReset(window, now: context.date)).font(AppFont.secondary).foregroundStyle(.secondary)
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

struct EstimateSummary: View {
    var records: [QuotaEstimate]
    var credits = false
    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            ForEach(EstimatePresentation.entries(records, credits: credits)) { entry in
                VStack(alignment: .leading, spacing: 2) {
                    Text(entry.label + " ≈ " + Format.money(entry.value) + " USD").monospacedDigit().foregroundStyle(Palette.accent)
                    if entry.historical { Text("历史采样 · " + Format.date(entry.record.checkpointAt) + (entry.record.originalEstimateId == nil ? "" : " · 已修正")).foregroundStyle(.secondary) }
                }
            }
            if let latest = EstimatePresentation.canonical(records).first, !latest.hasValue || latest.status == "pending" {
                Text("最新采样 · " + latest.statusLabel + " · " + latest.issueLabel).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }.font(AppFont.secondary).frame(maxWidth: .infinity, alignment: .leading)
    }
}
