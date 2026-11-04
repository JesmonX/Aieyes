import AppKit
import SwiftUI
@testable import Aieyes

@main struct VerifyNativeNavigation {
    @MainActor static func main() async throws {
        _ = NSApplication.shared
        NSApp.finishLaunching()
        let defaults = UserDefaults.standard
        let saved = defaults.volatileDomain(forName: UserDefaults.argumentDomain)
        defer { defaults.setVolatileDomain(saved, forName: UserDefaults.argumentDomain) }
        func collapse(_ value: Bool) {
            defaults.setVolatileDomain(saved.merging(["main.sidebarCollapsed": value]) { _, next in next }, forName: UserDefaults.argumentDomain)
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
        }
        collapse(false)
        let model = AppModel(autostart: false)
        model.settingsLoaded = true
        defer { model.stop() }
        let view = NSHostingView(rootView: MainWindowView(model: model))
        let window = NSWindow(contentRect: NSRect(x:100,y:100,width:1000,height:700), styleMask:[.titled,.closable,.resizable,.fullSizeContentView], backing:.buffered, defer:false)
        window.isReleasedWhenClosed = false
        window.toolbar = NSToolbar(identifier: "VerifyNavigationToolbar")
        window.toolbarStyle = .unified
        window.contentView = view
        window.orderFrontRegardless()
        defer { window.close() }
        func settle() async throws {
            try await Task.sleep(for: .milliseconds(500))
            view.layoutSubtreeIfNeeded()
        }
        func descendants(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap(descendants) }
        func pageHost() -> NSView? { descendants(view).first { String(describing:type(of:$0)) == "NSHostingView<Page>" } }
        try await settle()
        // This fixture has no other toolbar actions, so both an extra system toggle
        // and a missing recovery control must fail, including while the sidebar is hidden.
        func sidebarToggle() -> NSView {
            let controls = window.toolbar?.items.compactMap(\.view) ?? []
            precondition(controls.count == 1, "Expected one sidebar toggle, found \(controls.count)")
            let visible = window.toolbar?.visibleItems?.compactMap(\.view) ?? []
            precondition(visible.count == 1 && visible[0].window === window, "Sidebar toggle must stay visible")
            return controls[0]
        }
        _ = sidebarToggle()
        func clickSidebarToggle() {
            let control = sidebarToggle()
            let point = control.convert(NSPoint(x: control.bounds.midX, y: control.bounds.midY), to: nil)
            for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                let event = NSEvent.mouseEvent(with: type, location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                              windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
                NSApp.sendEvent(event)
            }
        }
        guard let list = descendants(view).compactMap({ $0 as? NSOutlineView }).first else { fatalError("Missing native navigation List") }
        precondition(list.numberOfRows == 11)
        let overview = pageHost()!
        // Native List selection exercises the SwiftUI binding and retained page host.
        list.selectRowIndexes(IndexSet(integer:2), byExtendingSelection:false)
        try await settle()
        precondition(model.detailPage == "analysis" && window.title == "用量分析 — Aieyes")
        precondition(pageHost() !== overview)
        list.selectRowIndexes(IndexSet(integer:1), byExtendingSelection:false)
        try await settle()
        precondition(model.detailPage == "agent" && window.title == "总览 — Aieyes")
        precondition(pageHost() === overview, "Returning to a page retains its native view identity")
        collapse(true)
        try await settle()
        _ = sidebarToggle()
        let rail = descendants(view).compactMap { $0 as? NSScrollView }.first { $0.frame.width > 0 && $0.frame.width < 100 }
        precondition(rail != nil && abs(rail!.frame.width - 62) < 1, "Collapsed rail must remain 62pt")
        window.setContentSize(NSSize(width:760,height:480))
        try await settle()
        precondition(pageHost() === overview && model.detailPage == "agent")
        collapse(false)
        try await settle()
        precondition(descendants(view).contains { $0 is NSOutlineView })
        for width: CGFloat in [1000, 760] {
            window.setContentSize(NSSize(width: width, height: 700))
            try await settle()
            for collapsed in [true, false] {
                // Change the eye button's preference and immediately click the toolbar,
                // without waiting for the rail's layout/animation to finish.
                collapse(collapsed)
                clickSidebarToggle()
                try await settle()
                precondition(list.isHiddenOrHasHiddenAncestor, "Toolbar toggle must hide the sidebar")
                _ = sidebarToggle()
                clickSidebarToggle()
                try await settle()
                precondition(!list.isHiddenOrHasHiddenAncestor, "Toolbar toggle must restore the sidebar")
                _ = sidebarToggle()
                if collapsed {
                    precondition(abs(list.enclosingScrollView!.frame.width - 62) < 1, "Restoring the sidebar must preserve the eye's collapsed rail")
                }
                precondition(pageHost() === overview && model.detailPage == "agent")
            }
        }
        print("Native navigation: List selection, page titles, retained pages, 62pt rail, minimum width and one persistent sidebar toggle through interleaved collapse/hide/show passed")
    }
}
