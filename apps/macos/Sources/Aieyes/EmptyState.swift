import SwiftUI

/// One empty-state language for every page: a symbol on concentric rings (the quota-ring motif),
/// a title, one line of guidance and at most one action.
struct EmptyState: View {
    var icon: String, title: String
    var message = ""
    var actionTitle: String? = nil
    var action: () -> Void = {}
    /// Panel-sized variant: smaller mark and tighter padding.
    var compact = false
    var tint: Color = Palette.accent
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        let size: CGFloat = compact ? 56 : 76
        VStack(spacing: compact ? 10 : 14) {
            ZStack {
                Circle().strokeBorder(tint.opacity(0.12), lineWidth: 1).frame(width: size, height: size)
                Circle().trim(from: 0.06, to: 0.40).stroke(tint.opacity(0.40), style: StrokeStyle(lineWidth: 2, lineCap: .round))
                    .frame(width: size - 2, height: size - 2).rotationEffect(.degrees(-90))
                Circle().fill(tint.opacity(DesignTokens.controlTintOpacity)).frame(width: size * 0.66, height: size * 0.66)
                Image(systemName: icon).font(.system(size: size * 0.30, weight: .regular)).foregroundStyle(tint)
            }.accessibilityHidden(true)
            VStack(spacing: 4) {
                Text(title).font(AppFont.section).multilineTextAlignment(.center)
                if !message.isEmpty {
                    Text(message).font(AppFont.secondary).foregroundStyle(.secondary).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                }
            }
            if let actionTitle { Button(actionTitle, action: action).buttonStyle(.aieyesPrimary) }
        }
        .frame(maxWidth: .infinity).padding(.vertical, compact ? 20 : 36).padding(.horizontal, 16)
    }
}
