import SwiftUI

enum Palette {
    // BEGIN GENERATED CHART PALETTES
    struct ChartPreset {
        let id: String, name: String
        let lightHigh, lightLow, darkHigh, darkLow: [UInt32]
        func colors(dark: Bool, contrast: String) -> [UInt32] {
            dark ? (contrast == "low" ? darkLow : darkHigh) : (contrast == "low" ? lightLow : lightHigh)
        }
    }
    static let chartPresets: [ChartPreset] = [
        ChartPreset(id: "classic", name: "经典",
            lightHigh: [0x6575ed, 0x308d87, 0x986ec1, 0xa77642, 0xba648f, 0x388b9b, 0x668a56, 0x7d77c4],
            lightLow: [0xa0aaf4, 0x68bdb7, 0xc2a2e2, 0xe3a25a, 0xe29abe, 0x51bdd1, 0x96b986, 0xadaadd],
            darkHigh: [0x707eee, 0x39a8a0, 0xa879d5, 0xe3a159, 0xd875a6, 0x4bbbd0, 0x7ca768, 0x857fcc],
            darkLow: [0x4d57a0, 0x316564, 0x6b5285, 0x72583f, 0x7e4e69, 0x37636f, 0x4f6248, 0x5b5887]),
        ChartPreset(id: "tableau", name: "Tableau · 经典分析",
            lightHigh: [0x4e79a7, 0xbb6d21, 0xd85456, 0x588985, 0x4f8e46, 0x957e2e, 0xa27195, 0xb26d74, 0x9c755f, 0x867e7c],
            lightLow: [0x99b2cc, 0xf39b42, 0xed9899, 0x7cbab5, 0x88bc81, 0xcbac3f, 0xc9a4bf, 0xf1959e, 0xc2ab9d, 0xb6ada9],
            darkHigh: [0x668bb3, 0xf28e2b, 0xe25c5d, 0x76b7b2, 0x59a14f, 0xedc949, 0xaf7aa1, 0xff9da7, 0xa4816c, 0xbab0ac],
            darkLow: [0x425f80, 0x7e532d, 0x944447, 0x486163, 0x41653f, 0x685c37, 0x71546c, 0x77535a, 0x70584c, 0x605c5e]),
        ChartPreset(id: "tol", name: "Paul Tol · 明快",
            lightHigh: [0x4477aa, 0xd05968, 0x228833, 0x8d812f, 0x4489a0, 0xaa3377, 0x808080],
            lightLow: [0x95b2cf, 0xf3949f, 0x84bd8d, 0xc0b040, 0x5dbada, 0xd79fbf, 0xaeaeae],
            darkHigh: [0x608cb7, 0xee6677, 0x429950, 0xccbb44, 0x66ccee, 0xc26c9d, 0xbbbbbb],
            darkLow: [0x3c5f85, 0x8d4853, 0x246a31, 0x645d36, 0x3e6271, 0xa03271, 0x5c5c60]),
        ChartPreset(id: "set2", name: "ColorBrewer · 柔彩",
            lightHigh: [0x4a8c77, 0xbd6a4a, 0x7180a3, 0xb16995, 0x6a8a36, 0x957f1c, 0x927d5f, 0x808080],
            lightLow: [0x65bfa3, 0xfc946b, 0x9fafd3, 0xe995c8, 0x91bc49, 0xcbad26, 0xc7aa81, 0xaeaeae],
            darkHigh: [0x66c2a5, 0xfc8d62, 0x8da0cb, 0xe78ac3, 0xa6d854, 0xffd92f, 0xe5c494, 0xb3b3b3],
            darkLow: [0x40635b, 0x7f5243, 0x545d72, 0x77516c, 0x52623a, 0x685c2e, 0x665b4f, 0x5c5c60]),
        ChartPreset(id: "nord", name: "Nord · 北欧",
            lightHigh: [0x5e81ac, 0x738663, 0xae715e, 0x967690, 0x5f8692, 0x917e56, 0xbf616a, 0x668686],
            lightLow: [0x9bb1cc, 0x9db686, 0xdaa290, 0xc4a6bf, 0x82b7c6, 0xc7ab75, 0xd9a1a6, 0x8bb6b6],
            darkHigh: [0x6a8bb2, 0xa3be8c, 0xd08770, 0xb48ead, 0x88c0d0, 0xebcb8b, 0xc56f77, 0x8fbcbb],
            darkLow: [0x495e7b, 0x566051, 0x77554d, 0x695768, 0x4c606a, 0x655c4b, 0x874c54, 0x4f6063]),
    ]
    // END GENERATED CHART PALETTES
    static func adaptive(_ light: UInt32, _ dark: UInt32) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let hex = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
            return NSColor(srgbRed: Double((hex >> 16) & 255) / 255, green: Double((hex >> 8) & 255) / 255, blue: Double(hex & 255) / 255, alpha: 1)
        })
    }
    static var accent: Color { accent(named: UserDefaults.standard.string(forKey: "appearance.accent") ?? "system") }
    static func accent(named name: String) -> Color {
        let pair: (UInt32, UInt32)
        switch name {
        // The default follows System Settings, so Aieyes controls match the ones AppKit draws.
        case "system": return Color(nsColor: .controlAccentColor)
        case "blue": pair = (DesignTokens.accentBlueLight, DesignTokens.accentBlueDark)
        case "teal": pair = (DesignTokens.accentTealLight, DesignTokens.accentTealDark)
        case "purple": pair = (DesignTokens.accentPurpleLight, DesignTokens.accentPurpleDark)
        default: pair = (DesignTokens.accentLight, DesignTokens.accentDark)
        }
        return adaptive(pair.0, pair.1)
    }
    /// Text and symbols on a solid accent fill. AppKit draws white on the system accent in both appearances.
    static func accentInk(named name: String) -> Color { name == "system" ? .white : adaptive(DesignTokens.accentInkLight, DesignTokens.accentInkDark) }
    static func category(_ key: String) -> Color {
        switch key {
        case "codex": return adaptive(DesignTokens.categoryCodexLight, DesignTokens.categoryCodexDark)
        case "claude": return adaptive(DesignTokens.categoryClaudeLight, DesignTokens.categoryClaudeDark)
        case "antigravity": return adaptive(DesignTokens.categoryAntigravityLight, DesignTokens.categoryAntigravityDark)
        case "deepseek", "deepseek-harness": return adaptive(DesignTokens.categoryDeepseekLight, DesignTokens.categoryDeepseekDark)
        case "cpu": return adaptive(DesignTokens.categoryCpuLight, DesignTokens.categoryCpuDark)
        case "memory": return adaptive(DesignTokens.categoryMemoryLight, DesignTokens.categoryMemoryDark)
        case "gpu": return adaptive(DesignTokens.categoryGpuLight, DesignTokens.categoryGpuDark)
        case "storage": return adaptive(DesignTokens.categoryStorageLight, DesignTokens.categoryStorageDark)
        case "network": return adaptive(DesignTokens.categoryNetworkLight, DesignTokens.categoryNetworkDark)
        default: return accent
        }
    }
    static var balanceBlue: Color { accent }
    static let ok = adaptive(DesignTokens.okLight, DesignTokens.okDark)
    static let warn = adaptive(DesignTokens.warnLight, DesignTokens.warnDark)
    static let danger = adaptive(DesignTokens.dangerLight, DesignTokens.dangerDark)
    static let cardBorder = adaptive(DesignTokens.cardStrokeLight, DesignTokens.cardStrokeDark)
    static let radiusCard: CGFloat = DesignTokens.radiusCard
    static func quota(_ used: Double) -> Color {
        guard used.isFinite else { return .secondary }
        let remaining = min(100, max(0, 100 - used))
        return remaining < 10 ? danger : remaining <= 30 ? warn : accent
    }
    private static var modelSlots: [String: [String: Int]] = [:]
    private static var modelUsedSlots: [String: Set<Int>] = [:]
    private static let modelLock = NSLock()
    static func chartPreset(named name: String) -> ChartPreset {
        chartPresets.first { $0.id == name } ?? chartPresets[0]
    }
    static func registerModels(_ names: [String]) {
        for preset in chartPresets { for name in Set(names).sorted() { _ = modelSlot(name, preset: preset) } }
    }
    private static func modelSlot(_ name: String, preset: ChartPreset) -> Int {
        modelLock.lock(); defer { modelLock.unlock() }
        if let slot = modelSlots[preset.id]?[name] { return slot }
        let hash = name.utf8.reduce(UInt32(2166136261)) { ($0 ^ UInt32($1)) &* 16777619 }
        let count = preset.lightHigh.count
        var slot = Int(hash % UInt32(count))
        let used = modelUsedSlots[preset.id] ?? []
        for _ in 0..<count { if !used.contains(slot) { break }; slot = (slot + 1) % count }
        modelSlots[preset.id, default: [:]][name] = slot
        modelUsedSlots[preset.id, default: []].insert(slot)
        return slot
    }
    static func model(_ name: String, preset: String? = nil, contrast: String? = nil) -> Color {
        let family = chartPreset(named: preset ?? UserDefaults.standard.string(forKey: "appearance.chartPalette") ?? "classic")
        let level = contrast ?? UserDefaults.standard.string(forKey: "appearance.chartContrast") ?? "high"
        let slot = modelSlot(name, preset: family)
        return adaptive(family.colors(dark: false, contrast: level)[slot], family.colors(dark: true, contrast: level)[slot])
    }
    /// "其他" folds several models, so it takes a neutral instead of a model slot and never repeats a model's color.
    static let otherModels = adaptive(DesignTokens.faintLight, DesignTokens.faintDark)
}

/// Observe accent changes and share control finishes across windows and sheets.
private struct AieyesAccent: ViewModifier {
    @AppStorage("appearance.accent") private var name = "system"
    func body(content: Content) -> some View {
        content.tint(Palette.accent(named: name)).accentColor(Palette.accent(named: name))
            .buttonStyle(.aieyes)
            .textFieldStyle(.aieyes)
    }
}

/// Keep native Button/TextField semantics; only their painted surfaces change.
private struct AieyesControlSurface: ViewModifier {
    var tint: Color? = nil
    /// A solid fill replaces the glass finish (the primary action).
    var fill: Color? = nil
    var field = false
    var hovered = false
    var pressed = false
    var editing = false
    @Environment(\.isFocused) private var focused
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast

    private var opaque: Bool { reduceTransparency || contrast == .increased }
    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: DesignTokens.radiusControl)
        content.background {
            if let fill { shape.fill(fill) } else {
                shape.fill(Color(nsColor: .controlBackgroundColor).opacity(opaque ? 1 : field ? DesignTokens.fieldFillOpacity : DesignTokens.controlFillOpacity))
                if let tint { shape.fill(tint.opacity(DesignTokens.controlTintOpacity)) }
            }
            if hovered || pressed { shape.fill(Color.primary.opacity(pressed ? DesignTokens.controlPressedOpacity : DesignTokens.controlHoverOpacity)) }
        }
        .overlay {
            shape.strokeBorder(focused || editing ? Palette.accent : (fill ?? tint ?? Color.primary).opacity(contrast == .increased ? 1 : DesignTokens.controlBorderOpacity), lineWidth: focused || editing || contrast == .increased ? 2 : DesignTokens.borderWidth)
                .allowsHitTesting(false)
        }
    }
}

/// Button levels shared with the Web build (§5.2): one solid primary action per sheet or view, tinted for an
/// emphasized secondary action, secondary for the rest; a destructive role always draws in the danger color.
struct AieyesButtonStyle: ButtonStyle {
    enum Level { case secondary, tinted, primary }
    var level = Level.secondary
    func makeBody(configuration: Configuration) -> some View {
        AieyesButtonBody(configuration: configuration, level: level)
    }
}

private struct AieyesButtonBody: View {
    let configuration: ButtonStyle.Configuration
    let level: AieyesButtonStyle.Level
    @Environment(\.isEnabled) private var enabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.controlSize) private var controlSize
    @AppStorage("appearance.accent") private var accentName = "system"
    @State private var hovered = false
    private var destructive: Bool { configuration.role == .destructive }
    // Size follows .controlSize like native buttons (§5.1): small 20pt, regular 24pt, large 32pt.
    private var fontSize: CGFloat { switch controlSize { case .mini, .small: DesignTokens.fontCaption; case .large, .extraLarge: DesignTokens.fontBody; default: DesignTokens.fontSecondary } }
    private var minHeight: CGFloat { switch controlSize { case .mini, .small: 20; case .large, .extraLarge: DesignTokens.controlMd; default: DesignTokens.controlHeight } }
    private var solid: Bool { level == .primary && !destructive }
    private var tint: Color? { destructive ? Palette.danger : level == .tinted ? Palette.accent(named: accentName) : nil }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        configuration.label
            .font(.system(size: fontSize, weight: solid ? DesignTokens.weightMedium : DesignTokens.weightRegular))
            .foregroundStyle(destructive ? Palette.danger : solid ? Palette.accentInk(named: accentName) : Color.primary)
            .padding(.horizontal, DesignTokens.controlPaddingX).padding(.vertical, 2)
            .frame(minHeight: minHeight)
            .modifier(AieyesControlSurface(tint: tint, fill: solid ? Palette.accent(named: accentName) : nil, hovered: enabled && hovered, pressed: enabled && configuration.isPressed))
            .contentShape(RoundedRectangle(cornerRadius: DesignTokens.radiusControl))
            .opacity(enabled ? 1 : 0.45)
            .onHover { hovered = $0 }
            .animation(reduceMotion ? nil : .easeOut(duration: 0.13), value: hovered || configuration.isPressed)
    }
}

struct AieyesTextFieldStyle: TextFieldStyle {
    func _body(configuration: TextField<Self._Label>) -> some View {
        AieyesStyledField(content: configuration)
    }
}

private struct AieyesStyledField<Content: View>: View {
    let content: Content
    @State private var editing = false
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        content.textFieldStyle(.plain)
            .padding(.horizontal, DesignTokens.controlPaddingX).padding(.vertical, 3)
            .frame(minHeight: DesignTokens.inputHeight)
            .modifier(AieyesControlSurface(field: true, editing: editing))
            .background(FieldFocusObserver { editing = $0 })
    }
}

/// Observe the native field editor without adding a second FocusState binding,
/// which would override editors' existing programmatic focus and validation.
private struct FieldFocusObserver: NSViewRepresentable {
    var changed: (Bool) -> Void
    func makeNSView(context: Context) -> FieldFocusView { FieldFocusView(changed: changed) }
    func updateNSView(_ view: FieldFocusView, context: Context) { view.changed = changed; view.scheduleUpdate() }
}

private final class FieldFocusView: NSView {
    var changed: (Bool) -> Void
    private var focused = false
    private var updatePending = false
    private var observers: [NSObjectProtocol] = []
    init(changed: @escaping (Bool) -> Void) {
        self.changed = changed
        super.init(frame: .zero)
        for name in [NSControl.textDidBeginEditingNotification, NSControl.textDidEndEditingNotification, NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.scheduleUpdate() }
            })
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); scheduleUpdate() }
    func scheduleUpdate() {
        guard !updatePending else { return }
        updatePending = true
        DispatchQueue.main.async { [weak self] in self?.updatePending = false; self?.updateFocus() }
    }
    private func updateFocus() {
        let editor = window?.firstResponder as? NSTextView
        let field = (editor?.delegate as? NSTextField) ?? (window?.firstResponder as? NSTextField)
        var active = false
        if let field, let window, window.isKeyWindow, field.window === window {
            let point = convert(NSPoint(x: bounds.midX, y: bounds.midY), to: nil)
            active = field.convert(field.bounds, to: nil).contains(point)
        }
        if active != focused { focused = active; changed(active) }
    }
    deinit { for observer in observers { NotificationCenter.default.removeObserver(observer) } }
}

extension ButtonStyle where Self == AieyesButtonStyle {
    static var aieyes: AieyesButtonStyle { AieyesButtonStyle() }
    static var aieyesPrimary: AieyesButtonStyle { AieyesButtonStyle(level: .primary) }
    static var aieyesTinted: AieyesButtonStyle { AieyesButtonStyle(level: .tinted) }
}
extension TextFieldStyle where Self == AieyesTextFieldStyle {
    static var aieyes: AieyesTextFieldStyle { AieyesTextFieldStyle() }
}
extension View {
    func aieyesAccent() -> some View { modifier(AieyesAccent()) }
}
