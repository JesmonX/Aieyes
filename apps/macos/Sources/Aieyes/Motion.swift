import SwiftUI
import AppKit

/// Shared motion vocabulary. Durations come from docs/ui-design-tokens.json, so the Web build moves at
/// the same pace. Every entry point resolves to no animation while Reduce Motion is on.
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
    /// For code outside a view body (actions, AppKit). Views read the environment value instead.
    @MainActor static var reduced: Bool { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion }
}

/// `withAnimation` that honours Reduce Motion; the change still applies, only without motion.
@MainActor @discardableResult
func withMotion<Result>(_ motion: Motion, _ body: () throws -> Result) rethrows -> Result {
    try withAnimation(motion.animation(reduced: Motion.reduced), body)
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
    /// Headline numbers roll digit by digit between refreshes instead of jumping.
    func numericTransition<Value: Equatable>(value: Value) -> some View {
        contentTransition(.numericText()).motion(.slow, value: value)
    }
}
