import AppKit
import SwiftUI
@testable import Aieyes

@main struct VerifyNativeNavigation {
    @MainActor static func main() async throws {
        _ = NSApplication.shared
        NSApp.finishLaunching()
        AXUIElementSetAttributeValue(AXUIElementCreateApplication(getpid()), "AXManualAccessibility" as CFString, kCFBooleanTrue)
        let suite = "aieyes.navigation-check." + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set(false, forKey: "main.sidebarCollapsed")
        let model = AppModel(autostart: false)
        model.settingsLoaded = true
        defer { model.stop() }
        var view = NSHostingView(rootView: MainWindowView(model: model).defaultAppStorage(defaults))
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
        func clickEye() {
            var visited = Set<ObjectIdentifier>()
            func find(_ node: NSObject, depth: Int = 0) -> NSObject? {
                guard depth < 40, visited.insert(ObjectIdentifier(node)).inserted else { return nil }
                func value(_ name: String) -> AnyObject? {
                    let selector = NSSelectorFromString(name)
                    return node.responds(to: selector) ? node.perform(selector)?.takeUnretainedValue() : nil
                }
                if value("accessibilityIdentifier") as? String == "main.sidebarEyeToggle" { return node }
                let children = ((value("accessibilityChildren") as? [NSObject]) ?? []) + ((node as? NSView)?.subviews ?? [])
                for child in children { if let found = find(child, depth: depth + 1) { return found } }
                return nil
            }
            let selector = NSSelectorFromString("accessibilityPerformPress")
            guard let eye = find(view), eye.responds(to: selector), let implementation = eye.method(for: selector) else {
                fatalError("Missing eye button")
            }
            typealias Action = @convention(c) (AnyObject, Selector) -> Bool
            precondition(unsafeBitCast(implementation, to: Action.self)(eye, selector), "Eye button must accept presses")
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
        for width: CGFloat in [1000, 760] {
            window.setContentSize(NSSize(width: width, height: width == 760 ? 480 : 700))
            try await settle()
            for useEye in [true, false] {
                let expandedDetailWidth = overview.frame.width
                if useEye { clickEye() } else { clickSidebarToggle() }
                try await settle()
                precondition(list.isHiddenOrHasHiddenAncestor, "Either toggle must fully hide the sidebar")
                precondition(defaults.bool(forKey: "main.sidebarCollapsed"), "Hidden state must be saved")
                precondition(overview.frame.width > expandedDetailWidth + 180, "Hiding must release the entire sidebar width")
                _ = sidebarToggle()
                clickSidebarToggle()
                try await settle()
                precondition(!list.isHiddenOrHasHiddenAncestor, "Toolbar toggle must restore the sidebar")
                precondition(!defaults.bool(forKey: "main.sidebarCollapsed"), "Restored state must be saved")
                _ = sidebarToggle()
                precondition(list.enclosingScrollView!.frame.width >= 190, "Restoring must show full navigation, not a compact rail")
                precondition(pageHost() === overview && model.detailPage == "agent")
            }
            // The toolbar must also recover an eye collapse while its animation is in flight.
            clickEye()
            try await Task.sleep(for: .milliseconds(50))
            clickSidebarToggle()
            try await settle()
            precondition(!list.isHiddenOrHasHiddenAncestor && !defaults.bool(forKey: "main.sidebarCollapsed"))
            precondition(pageHost() === overview && model.detailPage == "agent")
        }
        clickEye()
        try await settle()
        view = NSHostingView(rootView: MainWindowView(model: model).defaultAppStorage(defaults))
        window.contentView = view
        try await settle()
        guard let restoredList = descendants(view).compactMap({ $0 as? NSOutlineView }).first else { fatalError("Missing restored navigation List") }
        precondition(restoredList.isHiddenOrHasHiddenAncestor, "Reopening must honor the saved collapsed state")
        clickSidebarToggle()
        try await settle()
        precondition(!restoredList.isHiddenOrHasHiddenAncestor && restoredList.enclosingScrollView!.frame.width >= 190)
        precondition(!defaults.bool(forKey: "main.sidebarCollapsed"))
        print("Native navigation: eye/toolbar fully hide and restore, retained pages, full-width recovery, saved visibility, minimum width and one persistent toolbar toggle passed")
    }
}
