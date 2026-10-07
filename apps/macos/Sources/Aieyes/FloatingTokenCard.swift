import SwiftUI
import AppKit

struct FloatingTokenCard: View {
    var total: Double
    var tokens: Tokens
    @State private var hovering = false
    @State private var expanded = false
    var body: some View {
        Button { expanded.toggle() } label: {
            VStack(alignment: .leading, spacing: 8) {
                HStack { Text("总 Token"); Spacer(); Image(systemName: "sparkle") }.font(AppFont.secondary).foregroundStyle(.secondary)
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text(Format.compact(total)).font(.system(size: 24, weight: .semibold))
                    Text("(" + Format.percent(tokens.cacheRate.map { $0 * 100 }) + ")").font(.system(size: 12)).foregroundStyle(.secondary)
                }.monospacedDigit()
                Text("点击查看详情").font(.system(size: 11)).foregroundStyle(.secondary).opacity(hovering ? 1 : 0)
            }.frame(maxWidth: .infinity, alignment: .leading).padding(12).modifier(NeutralCard())
        }.buttonStyle(.plain).onHover { hovering = $0 }.accessibilityLabel("总 Token，点击查看详情")
            .popover(isPresented: $expanded, arrowEdge: .bottom) {
                CacheSummaryCard(tokens: tokens).padding(10).frame(width: 340)
            }
    }
}
