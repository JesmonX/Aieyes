import AppKit
import SwiftUI

@main struct VerifyPassivePanel {
    @MainActor static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        let foreground = NSWorkspace.shared.frontmostApplication?.processIdentifier
        let previousKey = app.keyWindow
        let anchorWindow = PassivePanel(contentRect: NSRect(x: 100, y: 700, width: 80, height: 30), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        anchorWindow.isReleasedWhenClosed = false
        let anchor = NSView(frame: NSRect(x: 0, y: 0, width: 80, height: 30))
        anchorWindow.contentView = anchor
        let menu = MenuPanel()
        let controller = NSViewController()
        controller.view = PassiveHostingView(rootView: Text("Window behavior fixture"))
        menu.contentViewController = controller
        var closed = 0
        menu.onClose = { closed += 1 }
        menu.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minY)
        precondition(menu.isShown)
        for panel in app.windows.compactMap({ $0 as? PassivePanel }) {
            precondition(panel.styleMask.contains(.nonactivatingPanel))
            precondition(!panel.canBecomeKey && !panel.canBecomeMain)
        }
        precondition(app.keyWindow === previousKey)
        precondition(NSWorkspace.shared.frontmostApplication?.processIdentifier == foreground)
        menu.performClose(nil)
        precondition(!menu.isShown && closed == 1)
        menu.performClose(nil)
        precondition(closed == 1)
        menu.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minY)
        precondition(menu.isShown && app.keyWindow === previousKey)
        menu.performClose(nil)
        precondition(closed == 2)
        print("Native passive window show/close preserves foreground application and key window")
    }
}
