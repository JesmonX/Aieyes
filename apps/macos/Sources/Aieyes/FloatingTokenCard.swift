import SwiftUI
import AppKit

struct FloatingTokenCard: View {
    var total: Double
    var tokens: Tokens
    var inline = false
    @State private var expanded = false
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Button { expanded.toggle() } label: {
            if inline {
                HStack(spacing: 5) { Text("Token").foregroundStyle(.secondary); Text(Format.compact(total)).bold().monospacedDigit(); Image(systemName: "chevron.down").font(AppFont.caption).foregroundStyle(.secondary) }
            } else {
            VStack(alignment: .leading, spacing: 8) {
                HStack { Text(L("总 Token")); Spacer(); Image(systemName: "chevron.down") }.font(AppFont.secondary).foregroundStyle(.secondary)
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text(Format.compact(total)).font(.system(size: DesignTokens.fontNumber, weight: DesignTokens.weightStrong))
                    Text("(" + Format.percent(tokens.cacheRate.map { $0 * 100 }) + ")").font(.system(size: DesignTokens.fontCaption)).foregroundStyle(.secondary)
                }.monospacedDigit()
            }.frame(maxWidth: .infinity, alignment: .leading).padding(DesignTokens.tokenCardPadding).modifier(NeutralCard())
            }
        }.buttonStyle(.plain).accessibilityLabel(L("总 Token，点击查看详情"))
            .help(L("总 Token：") + String(format: "%.0f", total))
            .popover(isPresented: $expanded, arrowEdge: .bottom) {
                VStack(alignment: .leading) { CacheSummaryCard(tokens: tokens); HelpNote(title: L("缓存读取命中率"), text: L("缓存读取 ÷（普通输入＋缓存读取＋缓存写入），不包含输出 Token。")) }.padding(10).frame(width: 340)
            }
    }
}
