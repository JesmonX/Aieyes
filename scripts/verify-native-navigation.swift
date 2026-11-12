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
        precondition(list.numberOfRows == 12)
        let overview = pageHost()!
        // Native List selection exercises the SwiftUI binding and retained page host.
        list.selectRowIndexes(IndexSet(integer:2), byExtendingSelection:false)
        try await settle()
        precondition(model.detailPage == "analysis" && window.title == "用量分析 — Aieyes")
        precondition(pageHost() !== overview)
        // Sessions is a monitoring page of its own, after servers.
        list.selectRowIndexes(IndexSet(integer:4), byExtendingSelection:false)
        try await settle()
        precondition(model.detailPage == "sessions" && window.title == "会话 — Aieyes")
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

        // Settings search: results replace the navigation, a result opens its settings page,
        // and clearing the field brings the navigation back.
        func outline() -> NSOutlineView {
            guard let found = descendants(view).compactMap({ $0 as? NSOutlineView }).first(where: { !$0.isHiddenOrHasHiddenAncestor }) else { fatalError("Missing sidebar list") }
            return found
        }
        guard let search = descendants(view).compactMap({ $0 as? NSSearchField }).first else { fatalError("Missing settings search field") }
        func enter(_ text: String) { search.stringValue = text; _ = search.sendAction(search.action, to: search.target) }
        enter("zzzz-no-setting")
        try await settle()
        precondition(outline().numberOfRows == 2, "An unmatched query shows only the empty note")
        enter("webhook")
        try await settle()
        let results = outline()
        precondition(results.numberOfRows >= 3, "A query lists every matching setting under one heading")
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(search)
        guard let editor = window.fieldEditor(false, for: search) as? NSTextView else { fatalError("Missing search editor") }
        editor.doCommand(by: #selector(NSResponder.insertNewline(_:)))
        try await settle()
        precondition(model.detailPage == "settings" && model.settingsTab == "general" && model.generalPage == "webhooks", "A result opens the page that holds the setting")
        enter("")
        try await settle()
        precondition(outline().numberOfRows == 12, "Clearing the query restores the navigation")

        // Panel → main window: the overview takes the request, scrolls to the account's card and
        // outlines it once.
        let info: CoreInfo = try await model.engine.call("hello")
        try model.acceptCoreInfo(info)
        model.settings = try await model.engine.call("settings.get"); model.settingsDraft = model.settings
        await model.reload(); await model.refreshQuotas()
        model.setWindowVisible(true, window: "detail")
        // The app delegate owns this hook; it shows the window and selects the page.
        model.showDetailPage = { page in model.requestedDetailPage = page; model.detailPage = page }
        window.setContentSize(NSSize(width: 1000, height: 520))
        outline().selectRowIndexes(IndexSet(integer: 2), byExtendingSelection: false)
        try await settle()
        precondition(model.detailPage == "analysis")
        guard let target = model.quotaCardAccounts(model.overviewDashboard.quotas).last?.key else { fatalError("The fixture needs quota accounts") }
        model.showAccount(target)
        try await Task.sleep(for: .milliseconds(1200))
        view.layoutSubtreeIfNeeded()
        precondition(model.detailPage == "agent", "The request opens the overview")
        precondition(model.accountFocus.request == nil, "The overview takes the request")
        precondition(model.accountFocus.highlighted == target, "The requested card is outlined")
        guard let page = pageHost(), let scroll = descendants(page).compactMap({ $0 as? NSScrollView }).max(by: { $0.frame.height < $1.frame.height }) else { fatalError("Missing overview scroll view") }
        let viewport = scroll.contentView.bounds
        precondition((scroll.documentView?.bounds.height ?? 0) > viewport.height + 200, "The fixture overview must be taller than the window")
        precondition(viewport.origin.y > 40, "The overview scrolls to the card, got offset \(viewport.origin.y)")
        try await Task.sleep(for: .milliseconds(Int(AccountFocusStore.hold * 1000) + 700))
        precondition(model.accountFocus.highlighted == nil, "The outline fades by itself")
        // An account without a card ends the request without touching the page.
        model.showAccount("codex:no-such-account")
        try await Task.sleep(for: .milliseconds(600))
        precondition(model.accountFocus.request == nil && model.accountFocus.highlighted == nil)

        // Server minute history, read through the same call as the servers page. The fixture hour
        // has a pause in sampling and a stretch without a GPU reading: both stay breaks in the
        // line, and a missing reading is never drawn as zero.
        model.outbound.attach(model)
        await model.outbound.refreshHistory()
        guard let host = model.settings.hosts.first else { fatalError("The fixture needs a server") }
        let trend = model.hostTrend(host.id)
        precondition(trend.minutes.count == 51, "The fixture hour holds 51 minutes, got \(trend.minutes.count)")
        precondition(trend.minutes.filter { $0.gpu == nil }.count == 5, "A minute without a GPU reading stays empty")
        let cpu = trend.series(.cpu), gpu = trend.series(.gpu)
        precondition(cpu.points.count == 51 && gpu.points.count == 46, "A minute without the reading is skipped")
        precondition(!gpu.points.contains { $0.value == 0 }, "A missing reading is never a zero")
        precondition(cpu.hasLine && cpu.latest != nil && gpu.latest != nil, "The newest minute carries the marker")
        func runs(_ series: TrendSeries) -> Int {
            var count = 0
            TrendLineShape(series: series).path(in: CGRect(x: 0, y: 0, width: 120, height: 20)).forEach { if case .move = $0 { count += 1 } }
            return count
        }
        precondition(runs(cpu) == 2, "The pause in sampling breaks the CPU line once, got \(runs(cpu)) runs")
        precondition(runs(gpu) == 3, "The pause and the missing GPU readings break the GPU line twice, got \(runs(gpu)) runs")
        precondition(!cpu.summary.isEmpty && cpu.peak.map { $0 > (cpu.points.map(\.value).max() ?? 100) } == true, "The CPU peak comes from each minute's own maximum")
        print("Native navigation: eye/toolbar fully hide and restore, retained pages, sessions page, full-width recovery, saved visibility, minimum width, one persistent toolbar toggle, settings search, account focus and server minute history passed")
    }
}
