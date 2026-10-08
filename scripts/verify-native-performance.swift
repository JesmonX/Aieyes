import AppKit
import SwiftUI
import ScreenCaptureKit
@testable import Aieyes

@MainActor final class PerformanceDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        Task {
            do { try await run(); NSApp.terminate(nil) }
            catch { fputs("Native performance verification failed: \(error)\n", stderr); exit(1) }
        }
    }
    func image(_ view: NSView, to url: URL) throws {
        view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
        let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)!
        view.cacheDisplay(in: view.bounds, to: bitmap)
        try bitmap.representation(using: .png, properties: [:])!.write(to: url)
    }
    func run() async throws {
        let root = URL(fileURLWithPath: CommandLine.arguments.dropFirst().first ?? ".local/performance-2026-10-08")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let model = AppModel(autostart: false)
        model.settingsLoaded = true
        model.settingsDraft = model.settings
        let view = NSHostingView(rootView: SettingsView(model: model).background(Color(nsColor: .windowBackgroundColor)))
        let window = NSWindow(contentRect: NSRect(x: 50, y: 50, width: 760, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
        window.title = "Aieyes 性能验证（合成数据）"; window.isReleasedWhenClosed = false; window.contentView = view
        window.orderFront(nil)
        var measurements: [[String: Any]] = []
        for count in [466, 1000] {
            model.prices = (0..<count).map { ModelPrice(id: "provider/model-\($0)", name: "Model \($0)", input: 1e-6, output: 3e-6) }
            var durations: [Double] = []
            for _ in 0..<3 {
                model.settingsTab = "general"
                try await Task.sleep(for: .milliseconds(80)); view.layoutSubtreeIfNeeded()
                let start = ContinuousClock.now
                model.settingsTab = "prices"
                try await Task.sleep(for: .milliseconds(20))
                view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
                let duration = start.duration(to: .now)
                durations.append(Double(duration.components.seconds) * 1000 + Double(duration.components.attoseconds) / 1e15)
            }
            measurements.append(["models": count, "switchMilliseconds": durations])
            try image(view, to: root.appendingPathComponent("prices-\(count).png"))
        }
        window.close()
        // Use the same real popover container as production. No desktop background
        // approximation: this window is supplied by NSPopover and WindowServer.
        model.prices = []
        if ProcessInfo.processInfo.environment["AIEYES_CORE_PATH"] != nil {
            model.settings = try await model.engine.call("settings.get"); model.settingsDraft = model.settings
            await model.reload(); await model.refreshSessions()
        }
        var panels: [[String: Any]] = []
        for (index, screen) in NSScreen.screens.enumerated() {
            for dark in [false, true] {
                let anchor = NSWindow(contentRect: NSRect(x: screen.visibleFrame.midX - 225, y: screen.visibleFrame.maxY - 90, width: 450, height: 40), styleMask: [.borderless], backing: .buffered, defer: false)
                anchor.isReleasedWhenClosed = false; anchor.contentView = NSView(frame: NSRect(x: 0, y: 0, width: 450, height: 40))
                anchor.appearance = NSAppearance(named: dark ? .darkAqua : .aqua); anchor.orderFront(nil)
                let popover = NSPopover(); popover.animates = false; popover.behavior = .applicationDefined
                let controller = PanelContainer(model: model)
                popover.contentViewController = controller
                model.panelHeight = min(720, screen.visibleFrame.height - 150)
                popover.contentSize = NSSize(width: 450, height: model.panelHeight)
                popover.show(relativeTo: anchor.contentView!.bounds, of: anchor.contentView!, preferredEdge: .minY)
                model.setWindowVisible(true, window: "panel")
                guard let panel = controller.view.window else { throw ClientError.message("No native popover window") }
                panel.appearance = anchor.appearance; panel.acceptsMouseMovedEvents = true
                try await Task.sleep(for: .milliseconds(150))
                let tint = controller.view.subviews.compactMap { $0 as? PanelTintView }.first!
                let updates = tint.colorUpdates
                for step in 0..<30 {
                    let x = CGFloat(step % 10) / 9 * 440 + 5
                    let point = controller.view.convert(NSPoint(x: x, y: 200 + CGFloat(step % 3) * 90), to: nil)
                    if let event = NSEvent.mouseEvent(with: .mouseMoved, location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: panel.windowNumber, context: nil, eventNumber: step, clickCount: 0, pressure: 0) { panel.sendEvent(event) }
                    if step % 10 == 0 { model.message = "后台刷新验证 \(step)" }
                    try await Task.sleep(for: .milliseconds(16))
                }
                precondition(tint.colorUpdates == updates, "Hover/content refresh must not rewrite the full-panel tint")
                let name = "popover-screen\(index + 1)-\(dark ? "dark" : "light")"
                try image(controller.view, to: root.appendingPathComponent(name + "-content.png"))
                var captured = false
                if CGPreflightScreenCaptureAccess(),
                   let content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true),
                   let captureWindow = content.windows.first(where: { $0.windowID == CGWindowID(panel.windowNumber) }) {
                    let filter = SCContentFilter(desktopIndependentWindow: captureWindow)
                    let configuration = SCStreamConfiguration()
                    configuration.width = Int(panel.frame.width * screen.backingScaleFactor)
                    configuration.height = Int(panel.frame.height * screen.backingScaleFactor)
                    configuration.showsCursor = false
                    if let bitmap = try? await SCScreenshotManager.captureImage(contentFilter: filter, configuration: configuration) {
                        try NSBitmapImageRep(cgImage: bitmap).representation(using: .png, properties: [:])!.write(to: root.appendingPathComponent(name + ".png")); captured = true
                    }
                }
                panels.append(["screen": index + 1, "theme": dark ? "dark" : "light", "tintUpdatesDuringHover": tint.colorUpdates - updates, "windowServerCapture": captured])
                model.message = nil; model.setWindowVisible(false, window: "panel"); popover.close(); anchor.close()
            }
        }
        let report: [String: Any] = ["pricing": measurements, "popovers": panels, "reduceTransparency": NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency,
            "note": "Synthetic mouseMoved events and content refresh verify tint isolation, not physical-pointer tracking or absence of the reported intermittent seam."]
        try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]).write(to: root.appendingPathComponent("report.json"))
        print(String(data: try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]), encoding: .utf8)!)
    }
}
@main struct NativePerformance {
    static func main() {
        let app = NSApplication.shared
        let delegate = PerformanceDelegate(); app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}
