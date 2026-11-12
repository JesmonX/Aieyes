import SwiftUI

/// "Show this account in the main window". The panel asks; the overview page scrolls to the
/// account's card and outlines it once.
@MainActor final class AccountFocusStore: ObservableObject {
    struct Request: Equatable { let account: String; let token: Int }
    /// Waiting for the overview page to come on screen.
    @Published private(set) var request: Request?
    /// The account whose card carries the outline right now.
    @Published private(set) var highlighted: String?
    private var token = 0
    private var highlightToken = 0
    /// Seconds the outline stays before it fades.
    static let hold = 1.6
    /// Scroll target of an account's card.
    static func anchor(_ account: String) -> String { "account-focus:" + account }

    func show(_ account: String) { token += 1; request = Request(account: account, token: token) }
    /// The overview scrolled to the card: outline it, then let the outline fade.
    func arrived(_ request: Request) {
        guard self.request == request else { return }
        self.request = nil; highlightToken = request.token
        withMotion(.standard) { highlighted = request.account }
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(Self.hold))
            guard let self, self.highlightToken == request.token else { return }
            withMotion(.slow) { self.highlighted = nil }
        }
    }
    /// The overview has no card for the account, so the request ends without a scroll.
    func drop(_ request: Request) { if self.request == request { self.request = nil } }
}

/// Accent outline on the card the panel asked for.
struct AccountFocusHighlight: ViewModifier {
    @ObservedObject var store: AccountFocusStore
    let account: String
    var radius: CGFloat = Palette.radiusCard
    func body(content: Content) -> some View {
        content.overlay {
            if store.highlighted == account {
                RoundedRectangle(cornerRadius: radius).strokeBorder(Palette.accent, lineWidth: 2)
                    .allowsHitTesting(false).accessibilityHidden(true).transition(.opacity)
            }
        }
    }
}

/// The same accent outline, lit once and faded when a card's reading is replaced by a newer one.
/// The first reading and Reduce Motion draw nothing.
struct ReadingHighlight: ViewModifier {
    let stamp: Double?
    var radius: CGFloat = Palette.radiusCard
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var flashes = 0
    func body(content: Content) -> some View {
        content.keyframeAnimator(initialValue: 0.0, trigger: flashes) { view, glow in
            view.overlay {
                if glow > 0 {
                    RoundedRectangle(cornerRadius: radius).strokeBorder(Palette.accent.opacity(glow), lineWidth: 2)
                        .allowsHitTesting(false).accessibilityHidden(true)
                }
            }
        } keyframes: { _ in
            LinearKeyframe(0.7, duration: DesignTokens.motionFast)
            LinearKeyframe(0.7, duration: DesignTokens.motionSlow)
            CubicKeyframe(0, duration: DesignTokens.motionSlow * 2)
        }
        .onChange(of: stamp) { old, new in
            guard !reduceMotion, let old, let new, old > 0, new > old else { return }
            flashes += 1
        }
    }
}

/// Lives inside the overview's `ScrollViewReader` and carries out focus requests.
struct AccountFocusScroller: View {
    @ObservedObject var store: AccountFocusStore
    let proxy: ScrollViewProxy
    /// Makes the account's card visible (agent filter, collapsed group) and returns its scroll
    /// target; nil when the overview has no card for the account.
    let reveal: (String) -> String?
    @Environment(\.surfaceActive) private var active
    private struct Trigger: Equatable { let request: AccountFocusStore.Request?; let active: Bool }
    var body: some View {
        Color.clear.frame(width: 0, height: 0).accessibilityHidden(true)
            .task(id: Trigger(request: store.request, active: active)) {
                guard active, let request = store.request else { return }
                guard let target = reveal(request.account) else { store.drop(request); return }
                // The page restores its remembered scroll position when it comes on screen and
                // a newly opened group needs a layout pass; the scroll follows both.
                try? await Task.sleep(for: .milliseconds(180))
                guard !Task.isCancelled else { return }
                withMotion(.standard) { proxy.scrollTo(target, anchor: .center) }
                store.arrived(request)
            }
    }
}
