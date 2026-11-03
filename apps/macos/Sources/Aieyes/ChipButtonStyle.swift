import SwiftUI

/// Choice chips (agent filter, server picker, settings sections). One selected fill per group,
/// which slides between chips when the group passes a shared namespace and animates its selection.
struct ChipButtonStyle: ButtonStyle {
    var selected: Bool
    var tint: Color = Palette.accent
    var namespace: Namespace.ID? = nil
    var font: Font = AppFont.secondary
    func makeBody(configuration: Configuration) -> some View {
        ChipButtonBody(configuration: configuration, selected: selected, tint: tint, namespace: namespace, font: font)
    }
}

private struct ChipButtonBody: View {
    let configuration: ButtonStyle.Configuration
    let selected: Bool
    let tint: Color
    let namespace: Namespace.ID?
    let font: Font
    @Environment(\.isEnabled) private var enabled
    @Environment(\.isFocused) private var focused
    @Environment(\.colorSchemeContrast) private var contrast
    @State private var hovered = false
    private var shape: RoundedRectangle { RoundedRectangle(cornerRadius: DesignTokens.radiusControl) }
    var body: some View {
        configuration.label
            .font(font)
            .padding(.horizontal, DesignTokens.controlPaddingX).padding(.vertical, 7)
            .foregroundStyle(selected ? tint : Color.secondary)
            .background {
                if selected { indicator }
                if enabled && (hovered || configuration.isPressed) {
                    shape.fill(Color.primary.opacity(configuration.isPressed ? DesignTokens.controlPressedOpacity : DesignTokens.controlHoverOpacity))
                }
            }
            .overlay { if focused { shape.strokeBorder(Palette.accent, lineWidth: 2).allowsHitTesting(false) } }
            .contentShape(shape)
            .opacity(enabled ? 1 : 0.45)
            .onHover { hovered = $0 }
            .motion(.fast, value: hovered || configuration.isPressed)
            .accessibilityAddTraits(selected ? .isSelected : [])
    }
    /// Increased contrast adds an outline, so the selection never depends on a faint fill alone.
    @ViewBuilder private var indicator: some View {
        let fill = shape.fill(tint.opacity(DesignTokens.controlTintOpacity))
            .overlay { if contrast == .increased { shape.strokeBorder(tint, lineWidth: DesignTokens.borderWidth) } }
        if let namespace { fill.matchedGeometryEffect(id: "chip.selection", in: namespace) } else { fill }
    }
}

extension ButtonStyle where Self == ChipButtonStyle {
    static func chip(selected: Bool, tint: Color = Palette.accent, in namespace: Namespace.ID? = nil, font: Font = AppFont.secondary) -> ChipButtonStyle {
        ChipButtonStyle(selected: selected, tint: tint, namespace: namespace, font: font)
    }
}
