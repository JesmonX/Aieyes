import SwiftUI

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

struct PanelQuotaSummary: View {
    var quota: Quota
    var open: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button(action: open) {
                HStack { Text(quota.name).fontWeight(.semibold).lineLimit(1); Spacer(); Text("实时账户限额").foregroundStyle(.secondary); Image(systemName: "chevron.right") }
            }.buttonStyle(.plain).accessibilityLabel("查看 " + quota.name + " 的实时限额")
            if let balance = quota.balances?.first { Text(balance.currency + " " + balance.total).font(AppFont.section).monospacedDigit() }
            HStack(spacing: 12) {
                ForEach(Array(quota.windows.prefix(2))) { window in
                    VStack(alignment: .leading, spacing: 3) {
                        HStack { Text(window.windowMinutes == 300 ? "5h" : window.windowMinutes == 10080 ? "7d" : window.name).lineLimit(1); Spacer(); Text(Format.percent(max(0, 100 - window.usedPercent))).monospacedDigit() }
                        ResourceBar(percent: 100 - window.usedPercent, tint: Palette.quota(window.usedPercent))
                    }.frame(maxWidth: .infinity)
                }
            }
            TimelineView(.periodic(from: .now, by: 60)) { context in
                Text(quota.error == nil ? Format.resetCountdown(quota.windows.compactMap(\.resetsAt).min(), now: context.date) : "限额更新失败 · 查看详情")
                    .font(.system(size: 13)).foregroundStyle(quota.error == nil ? Color.secondary : Palette.warn)
            }
        }.font(AppFont.secondary).padding(10).modifier(NeutralCard())
    }
}
