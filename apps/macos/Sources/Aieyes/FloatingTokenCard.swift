import SwiftUI
import AppKit

struct FloatingTokenCard: View {
    var total: Double
    var tokens: Tokens
    var inline = false
    @State private var hovering = false
    @State private var expanded = false
    var body: some View {
        Button { expanded.toggle() } label: {
            if inline {
                HStack(spacing: 5) { Text("Token").foregroundStyle(.secondary); Text(Format.compact(total)).bold().monospacedDigit(); Image(systemName: "chevron.down").font(AppFont.caption).foregroundStyle(.secondary) }
            } else {
            VStack(alignment: .leading, spacing: 8) {
                HStack { Text("总 Token"); Spacer(); Image(systemName: "sparkle") }.font(AppFont.secondary).foregroundStyle(.secondary)
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text(Format.compact(total)).font(.system(size: DesignTokens.fontNumber, weight: DesignTokens.weightStrong))
                    Text("(" + Format.percent(tokens.cacheRate.map { $0 * 100 }) + ")").font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary)
                }.monospacedDigit()
                Text("点击查看详情").font(.system(size: DesignTokens.fontHint)).foregroundStyle(.secondary).opacity(hovering ? 1 : 0)
            }.frame(maxWidth: .infinity, alignment: .leading).padding(DesignTokens.tokenCardPadding).modifier(NeutralCard())
            }
        }.buttonStyle(.plain).onHover { hovering = $0 }.accessibilityLabel("总 Token，点击查看详情")
            .help("总 Token：" + String(format: "%.0f", total) + "；点击查看输入、输出与缓存明细")
            .popover(isPresented: $expanded, arrowEdge: .bottom) {
                CacheSummaryCard(tokens: tokens).padding(10).frame(width: 340)
            }
    }
}
