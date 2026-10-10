import AppKit

@MainActor final class RefuseClose: NSObject, NSWindowDelegate {
    func windowShouldClose(_ sender: NSWindow) -> Bool { false }
}

@main struct VerifyDockPresence {
    @MainActor static func main() async throws {
        _ = NSApplication.shared
        var policies: [NSApplication.ActivationPolicy] = []
        let dock = DockPresence { policies.append($0) }
        let settings = NSWindow(contentRect: .zero, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        let detail = NSWindow(contentRect: .zero, styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 100, height: 100), styleMask: [.nonactivatingPanel], backing: .buffered, defer: false)
        settings.isReleasedWhenClosed = false; detail.isReleasedWhenClosed = false
        panel.isReleasedWhenClosed = false
        let refusing = RefuseClose()

        panel.orderFront(nil)
        try await Task.sleep(for: .milliseconds(20))
        precondition(policies.isEmpty, "Opening only a panel must not create a Dock entry")
        dock.open(settings)
        try await Task.sleep(for: .milliseconds(20))
        precondition(policies == [.regular], "Opening settings must create a Dock entry")
        dock.open(detail)
        settings.close()
        detail.miniaturize(nil)
        try await Task.sleep(for: .milliseconds(20))
        precondition(policies == [.regular], "Closing settings must retain a minimized detail window's Dock entry")
        detail.deminiaturize(nil)
        detail.orderOut(nil)
        try await Task.sleep(for: .milliseconds(20))
        precondition(policies == [.regular], "Hiding an open window must preserve Dock access")
        detail.delegate = refusing
        detail.performClose(nil)
        try await Task.sleep(for: .milliseconds(20))
        precondition(policies == [.regular], "A cancelled close must preserve the icon")
        detail.delegate = nil
        detail.close()
        try await Task.sleep(for: .milliseconds(20))
        precondition(policies == [.regular, .accessory], "An open panel must not retain the Dock entry after all standalone windows close")
        dock.open(settings)
        settings.orderFront(nil)
        settings.close()
        try await Task.sleep(for: .milliseconds(20))
        precondition(policies == [.regular, .accessory, .regular, .accessory])
        panel.close()
        try await Task.sleep(for: .milliseconds(20))
        precondition(policies == [.regular, .accessory, .regular, .accessory])
        print("Dock lifecycle checks passed")
    }
}
