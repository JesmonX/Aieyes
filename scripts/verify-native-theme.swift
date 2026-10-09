import AppKit
import SwiftUI
@testable import Aieyes

@MainActor final class ThemeObservation { var dark: Bool? }
struct ThemeProbe: View {
    @Environment(\.colorScheme) private var scheme
    let observation: ThemeObservation
    var body: some View {
        Color.clear.onAppear { observation.dark = scheme == .dark }
            .onChange(of: scheme) { _, value in observation.dark = value == .dark }
    }
}
@MainActor final class ThemeDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        Task { do { try await verify(); print("Native theme: live popover/SwiftUI/tint sync, opposite anchor, reopen and system appearance passed"); NSApp.terminate(nil) }
            catch { fputs("Native theme failed: \(error)\n", stderr); exit(1) } }
    }
    func verify() async throws {
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let original = NSApp.appearance
        let accent = UserDefaults.standard.object(forKey: "appearance.accent")
        defer { NSApp.appearance = original; if let accent { UserDefaults.standard.set(accent, forKey: "appearance.accent") } else { UserDefaults.standard.removeObject(forKey: "appearance.accent") } }
        let model = AppModel(autostart: false); model.settingsLoaded = true
        let anchor = NSWindow(contentRect: NSRect(x: 300, y: 740, width: 450, height: 30), styleMask: [.borderless], backing: .buffered, defer: false)
        anchor.isReleasedWhenClosed = false; anchor.appearance = NSAppearance(named: .aqua)
        anchor.contentView = NSView(frame: NSRect(x: 0, y: 0, width: 450, height: 30)); anchor.orderFront(nil)
        defer { anchor.close() }
        let popover = NSPopover(); popover.animates = false; popover.behavior = .applicationDefined
        let controller = PanelContainer(model: model); popover.contentViewController = controller
        popover.contentSize = NSSize(width: 450, height: 600); model.panelHeight = 600
        let observation = ThemeObservation()
        let probe = NSHostingView(rootView: ThemeProbe(observation: observation)); probe.frame = NSRect(x: 0, y: 0, width: 1, height: 1); controller.view.addSubview(probe)
        let appearance = AppAppearanceController(popover: popover)
        appearance.apply(theme: "dark", accent: "teal")
        func show() { appearance.updatePanel(); popover.show(relativeTo: anchor.contentView!.bounds, of: anchor.contentView!, preferredEdge: .minY) }
        show()
        defer { popover.close() }
        func assertTheme(_ dark: Bool) async throws {
            try await Task.sleep(for: .milliseconds(200))
            let expected: NSAppearance.Name = dark ? .darkAqua : .aqua
            precondition(popover.appearance?.bestMatch(from: [.darkAqua, .aqua]) == expected)
            precondition(controller.view.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == expected)
            precondition(controller.view.window?.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == expected)
            precondition(observation.dark == dark, "SwiftUI colorScheme must follow the saved app theme")
            let tint = controller.view.subviews.compactMap { $0 as? PanelTintView }.first!
            let color = NSColor(cgColor: tint.layer!.backgroundColor!)!.usingColorSpace(.deviceRGB)!
            precondition((color.redComponent < 0.5) == dark, "Native panel tint did not update")
        }
        try await assertTheme(true)
        for theme in ["light", "dark"] {
            appearance.apply(theme: theme, accent: "teal"); try await assertTheme(theme == "dark")
            controller.view.layoutSubtreeIfNeeded(); controller.view.displayIfNeeded()
            let bitmap = controller.view.bitmapImageRepForCachingDisplay(in: controller.view.bounds)!
            controller.view.cacheDisplay(in: controller.view.bounds, to: bitmap)
            try bitmap.representation(using: .png, properties: [:])!.write(to: root.appendingPathComponent("popover-" + theme + ".png"))
        }
        popover.close(); appearance.apply(theme: "light", accent: "teal"); show(); try await assertTheme(false)
        popover.close(); appearance.apply(theme: "dark", accent: "teal"); show(); try await assertTheme(true)
        appearance.apply(theme: "system", accent: "teal")
        try await assertTheme(NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua)
        // Exercise the effectiveAppearance observer without changing the user's OS preference.
        NSApp.appearance = NSAppearance(named: .aqua); try await assertTheme(false)
        NSApp.appearance = NSAppearance(named: .darkAqua); try await assertTheme(true)
    }
}
@main struct VerifyNativeTheme {
    static func main() {
        let app = NSApplication.shared, delegate = ThemeDelegate(); app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}
