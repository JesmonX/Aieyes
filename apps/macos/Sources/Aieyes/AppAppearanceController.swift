import AppKit
import Combine

/// Popovers inherit their anchor's appearance, which can differ from the app.
@MainActor final class AppAppearanceController {
    private weak var popover: NSPopover?
    private var observation: AnyCancellable?

    init(popover: NSPopover) {
        self.popover = popover
        observation = NSApp.publisher(for: \.effectiveAppearance).sink { [weak self] _ in
            self?.updatePanel()
        }
    }

    func apply(theme: String, accent: String) {
        UserDefaults.standard.set(accent, forKey: "appearance.accent")
        NSApp.appearance = theme == "dark" ? NSAppearance(named: .darkAqua) : theme == "light" ? NSAppearance(named: .aqua) : nil
        updatePanel()
    }

    func updatePanel() {
        let appearance = NSApp.effectiveAppearance
        popover?.appearance = appearance
        popover?.contentViewController?.view.appearance = appearance
        popover?.contentViewController?.view.window?.appearance = appearance
    }
}
