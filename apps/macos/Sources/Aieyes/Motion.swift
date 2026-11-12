import SwiftUI
import AppKit

/// Shared motion vocabulary. Durations come from docs/ui-design-tokens.json, so the Web build moves at
/// the same pace. Every entry point resolves to no animation while Reduce Motion is on, except page
/// swaps, which keep a plain cross-fade (`Motion.page(reduced:)`).
enum Motion {
    /// Hover and press feedback.
    case fast
    /// Page and state swaps.
    case standard
    /// Values that change in place: numbers, bars, chart marks.
    case slow
    /// Disclosure, expand and collapse.
    case expand
    /// Selection indicators and reordering.
    case spring

    var animation: Animation {
        switch self {
        case .fast: .easeOut(duration: DesignTokens.motionFast)
        case .standard: .easeInOut(duration: DesignTokens.motionBase)
        case .slow: .easeInOut(duration: DesignTokens.motionSlow)
        case .expand: .spring(response: DesignTokens.motionExpand, dampingFraction: 0.88)
        case .spring: .spring(response: DesignTokens.motionSlow, dampingFraction: 0.82)
        }
    }
    func animation(reduced: Bool) -> Animation? { reduced ? nil : animation }
    /// Seconds, for AppKit and Core Animation code that cannot take a SwiftUI `Animation`.
    var duration: Double {
        switch self {
        case .fast: DesignTokens.motionFast
        case .standard: DesignTokens.motionBase
        case .slow, .spring: DesignTokens.motionSlow
        case .expand: DesignTokens.motionExpand
        }
    }
    /// For code outside a view body (actions, AppKit). Views read the environment value instead.
    @MainActor static var reduced: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }
    /// Page swaps are the one exception to "no animation under Reduce Motion": the pages still
    /// cross-fade (`fast`, a dissolve without movement), as the main window's page container does.
    static func page(reduced: Bool) -> Animation { reduced ? Motion.fast.animation : Motion.standard.animation }
}

extension AnyTransition {
    /// The incoming page rises a few points while it fades in; Reduce Motion keeps the fade alone.
    static func page(reduced: Bool) -> AnyTransition {
        reduced ? .opacity : .asymmetric(insertion: .opacity.combined(with: .offset(y: 6)), removal: .opacity)
    }
}

/// Trackpad feedback for snaps and reorders. Hardware without a haptic engine stays silent.
@MainActor enum Haptics {
    /// An item settled into a slot or against an edge.
    static func alignment() { NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .default) }
    /// A discrete step, such as a row moving one place.
    static func step() { NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .default) }
}

/// `withAnimation` that honours Reduce Motion; the change still applies, only without motion.
@MainActor @discardableResult
func withMotion<Result>(_ motion: Motion, _ body: () throws -> Result) rethrows -> Result {
    try withAnimation(motion.animation(reduced: Motion.reduced), body)
}

private struct PageMotionModifier<Value: Equatable>: ViewModifier {
    let value: Value
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    func body(content: Content) -> some View { content.animation(Motion.page(reduced: reduceMotion), value: value) }
}

private struct MotionModifier<Value: Equatable>: ViewModifier {
    let motion: Motion
    let value: Value
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    func body(content: Content) -> some View { content.animation(motion.animation(reduced: reduceMotion), value: value) }
}

extension View {
    /// `.animation(_:value:)` with a named curve; passes nil while Reduce Motion is on.
    func motion<Value: Equatable>(_ motion: Motion, value: Value) -> some View { modifier(MotionModifier(motion: motion, value: value)) }
    /// Animates a page swap keyed by `value`; pair it with `.transition(.page(reduced:))` on the pages.
    func pageMotion<Value: Equatable>(value: Value) -> some View { modifier(PageMotionModifier(value: value)) }
    /// Headline numbers roll digit by digit between refreshes instead of jumping.
    func numericTransition<Value: Equatable>(value: Value) -> some View {
        contentTransition(.numericText()).motion(.slow, value: value)
    }
}
