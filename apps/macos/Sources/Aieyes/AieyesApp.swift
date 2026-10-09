import SwiftUI
import AppKit
import Combine

@main struct AieyesApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    var body: some Scene { SwiftUI.Settings { EmptyView() } }
}

@MainActor final class AppDelegate: NSObject, NSApplicationDelegate, NSPopoverDelegate, NSWindowDelegate, NSToolbarDelegate {
    private var statusItem: NSStatusItem!
    private var statusLabel: NSHostingView<MenuActivityLabel>?
    private let popover = NSPopover()
    private var detailWindow: NSWindow?
    private var pageControl: NSSegmentedControl?
    private var settingsWindow: NSWindow?
    private var samplingWindow: NSWindow?
    private var quotaOrderWindow: NSWindow?
    private var panelAccountsWindow: NSWindow?
    private var globalClickMonitor: Any?
    private var localEventMonitor: Any?
    private var estimateWindows: [String: NSWindow] = [:]
    private var approvingSettingsClose = false
    private var cancellables = Set<AnyCancellable>()
    private var model: AppModel!
    private var titleUpdatePending = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        if let index = CommandLine.arguments.firstIndex(of: "--render"), CommandLine.arguments.count > index + 1 {
            UserDefaults.standard.setVolatileDomain(["panel.accounts.v1": "{}", "panel.accounts.v2": "{\"version\":2,\"providers\":{}}", "panel.page": "agent", "detail.page": "agent", "panel.trendExpanded": false, "panel.sessionsExpanded": false, "panel.serverFilter": "all", "menu.showCount": false], forName: UserDefaults.argumentDomain)
            model = AppModel(autostart: false)
            Task { await renderSnapshots(to: CommandLine.arguments[index + 1]) }
            return
        }
        model = AppModel()
        model.$settings.map(\.appearance).removeDuplicates().sink { appearance in
            UserDefaults.standard.set(appearance?.accent ?? "indigo", forKey: "appearance.accent")
            NSApp.appearance = appearance?.theme == "dark" ? NSAppearance(named: .darkAqua) : appearance?.theme == "light" ? NSAppearance(named: .aqua) : nil
        }.store(in: &cancellables)
        AppUpdater.shared.prepareInstallation = { [weak self] finalizing in
            guard let self, await self.prepareUpdate() else { return false }
            // Saving a draft can yield to another window's operation; recheck before suspending work.
            guard !self.model.busy, !self.model.quotaBusy, !self.model.serverBusy, !self.model.settingsSaving, !self.model.repricing, self.model.estimateBusy.isEmpty else { return false }
            self.model.installingUpdate = finalizing
            return true
        }
        AppUpdater.shared.finishedInstallationAttempt = { [weak self] in self?.model.installingUpdate = false }
        AppUpdater.shared.start()
        model.showSettings = { [weak self] in self?.openSettings() }
        model.showDetail = { [weak self] in self?.openDetail() }
        model.showDetailPage = { [weak self] page in self?.openDetail(page: page) }
        model.showEstimate = { [weak self] quota, credits in self?.openEstimate(quota, credits: credits) }
        model.showSampling = { [weak self] in self?.openSampling() }
        model.showQuotaOrder = { [weak self] in self?.openQuotaOrder() }
        model.showPanelAccounts = { [weak self] in self?.openPanelAccounts() }
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = statusItem.button {
            button.target = self; button.action = #selector(togglePopover)
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
        }
        if let button = statusItem.button {
            button.image = nil
            let label = StatusHostingView(rootView: MenuActivityLabel(model: model))
            button.addSubview(label); statusLabel = label
            updateTitle()
        }
        // Own dismissal explicitly: transient popovers can lose outside dismissal
        // after a sheet, and can close on status mouse-down then reopen on mouse-up.
        popover.behavior = .applicationDefined; popover.animates = true; popover.delegate = self
        popover.contentViewController = PanelContainer(model: model)
        popover.contentSize = NSSize(width: 450, height: 720)
        model.objectWillChange.sink { [weak self] _ in self?.scheduleTitleUpdate() }.store(in: &cancellables)
        NotificationCenter.default.publisher(for: UserDefaults.didChangeNotification).sink { [weak self] _ in self?.scheduleTitleUpdate() }.store(in: &cancellables)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(wake), name: NSWorkspace.didWakeNotification, object: nil)
        globalClickMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { [weak self] _ in
            Task { @MainActor in self?.dismissOutsidePanel() }
        }
        localEventMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown, .keyDown]) { [weak self] event in
            guard let self else { return event }
            if event.type == .keyDown, event.window === self.detailWindow, event.modifierFlags.contains(.command), let key = event.charactersIgnoringModifiers, ["1", "2"].contains(key) {
                self.model.requestedDetailPage = key == "1" ? "agent" : "servers"; return nil
            }
            guard self.popover.isShown else { return event }
            if event.type == .keyDown {
                if event.keyCode == 53, event.window === self.popover.contentViewController?.view.window { self.popover.performClose(nil); return nil }
            } else if let window = event.window,
                      window !== self.popover.contentViewController?.view.window,
                      window !== self.statusItem.button?.window,
                      window.level.rawValue < NSWindow.Level.popUpMenu.rawValue {
                // A popup menu tracks in its own window; selecting an item must
                // not tear down its parent popover before the action is delivered.
                self.dismissOutsidePanel()
            }
            return event
        }
        NotificationCenter.default.addObserver(self, selector: #selector(applicationDeactivated), name: NSApplication.didResignActiveNotification, object: nil)
        if CommandLine.arguments.contains("--show-detail") { openDetail() }
    }
    @objc private func togglePopover() {
        guard let button = statusItem.button else { return }
        if NSApp.currentEvent?.type == .rightMouseUp {
            popover.performClose(nil)
            let menu = NSMenu()
            menu.addItem(withTitle: "打开详情", action: #selector(detailAction), keyEquivalent: "").target = self
            menu.addItem(withTitle: "设置…", action: #selector(settingsAction), keyEquivalent: ",").target = self
            if !model.runningEstimates.isEmpty { menu.addItem(withTitle: "管理采样…", action: #selector(samplingAction), keyEquivalent: "").target = self }
            menu.addItem(.separator())
            menu.addItem(withTitle: "退出 Aieyes", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
            statusItem.menu = menu; button.performClick(nil); statusItem.menu = nil; return
        }
        if popover.isShown { popover.performClose(nil) }
        else {
            model.panelHeight = min(720, max(360, (button.window?.screen?.visibleFrame.height ?? 800) - 34))
            popover.contentSize = NSSize(width: 450, height: model.panelHeight)
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY); model.setWindowVisible(true, window: "panel") }
    }
    private func dismissOutsidePanel() { if popover.isShown && !model.isPinned { popover.performClose(nil) } }
    @objc private func applicationDeactivated() { dismissOutsidePanel() }
    func popoverDidClose(_ notification: Notification) { model.setWindowVisible(false, window: "panel") }
    @objc private func detailAction() { openDetail() }
    @objc private func settingsAction() { openSettings() }
    @objc private func samplingAction() { openSampling() }
    @objc private func wake() { Task { await model.scan() } }
    private func scheduleTitleUpdate() {
        guard !titleUpdatePending else { return }
        titleUpdatePending = true
        DispatchQueue.main.async { [weak self] in
            self?.titleUpdatePending = false; self?.updateTitle()
        }
    }
    private func updateTitle() {
        guard let button = statusItem.button, let label = statusLabel else { return }
        let scale = button.window?.backingScaleFactor ?? 2
        let width = ceil(label.fittingSize.width * scale) / scale
        if statusItem.length != width { statusItem.length = width }
        let frame = NSRect(x: 0, y: 0, width: width, height: button.bounds.height)
        if label.frame != frame { label.frame = frame }
        var tooltip = "Aieyes · 今日全部数据 · " + model.sessionSummary + (model.sessionPhase.map { " · " + $0.rawValue } ?? "")
        if !model.runningEstimates.isEmpty { tooltip += " · " + model.samplingSummary }
        if button.toolTip != tooltip { button.toolTip = tooltip; button.setAccessibilityLabel(tooltip) }
        if let window = settingsWindow, window.isDocumentEdited != model.settingsDirty { window.isDocumentEdited = model.settingsDirty }
        let page = model.detailPage == "servers" ? 1 : 0
        if pageControl?.selectedSegment != page { pageControl?.selectedSegment = page }
    }
    private func openDetail(page: String? = nil) {
        if let page { model.requestedDetailPage = page; model.detailPage = page }
        else if detailWindow == nil, let saved = UserDefaults.standard.string(forKey: "detail.page"), ["agent", "servers"].contains(saved) { model.detailPage = saved }
        popover.performClose(nil)
        if detailWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 800), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            let toolbar = NSToolbar(identifier: "AieyesDetailsToolbar"); toolbar.delegate = self; toolbar.displayMode = .iconOnly; window.toolbar = toolbar; window.toolbarStyle = .unified
            window.contentMinSize = NSSize(width: 760, height: 480)
            window.title = "Aieyes"; window.titlebarAppearsTransparent = true; window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: RootView(model: model, compact: false, page: page ?? model.detailPage)); window.center(); window.delegate = self
            window.setFrameAutosaveName("AieyesDetails"); detailWindow = window
        }
        if let window = detailWindow { fitToVisibleScreen(window) }
        detailWindow?.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true); model.setWindowVisible(true, window: "detail")
    }
    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] { [.init("pages"), .flexibleSpace, .init("refresh")] }
    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] { toolbarAllowedItemIdentifiers(toolbar) }
    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier, willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        let item = NSToolbarItem(itemIdentifier: identifier)
        if identifier.rawValue == "pages" {
            let control = NSSegmentedControl(labels: ["Agent", "服务器"], trackingMode: .selectOne, target: self, action: #selector(detailPageChanged(_:)))
            control.selectedSegment = model.detailPage == "servers" ? 1 : 0; control.setAccessibilityLabel("详情页面，Command 1 切换 Agent，Command 2 切换服务器"); control.toolTip = "Agent（⌘1） · 服务器（⌘2）"; pageControl = control; item.view = control; item.label = "页面"
        } else if identifier.rawValue == "refresh" {
            let button = NSPopUpButton(frame: .zero, pullsDown: true); button.addItem(withTitle: "刷新")
            for (index, title) in ["同步记录", "刷新限额", "同步价格", "刷新服务器"].enumerated() { let entry = NSMenuItem(title: title, action: #selector(toolbarRefresh(_:)), keyEquivalent: ""); entry.tag = index; entry.target = self; button.menu?.addItem(entry) }
            button.setAccessibilityLabel("刷新选项"); item.view = button; item.label = "刷新"
        }
        return item
    }
    @objc private func detailPageChanged(_ sender: NSSegmentedControl) { model.requestedDetailPage = sender.selectedSegment == 1 ? "servers" : "agent" }
    @objc private func toolbarRefresh(_ sender: NSMenuItem) { Task { switch sender.tag { case 0: await model.scan(); case 1: await model.refreshQuotas(); case 2: await model.syncPrices(); default: await model.sampleHosts() } } }
    private func openSettings() {
        popover.performClose(nil)
        if settingsWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 600), styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
            window.title = "Aieyes 设置"; window.titlebarAppearsTransparent = true; window.isReleasedWhenClosed = false; window.delegate = self
            window.contentMinSize = NSSize(width: 620, height: 440)
            window.contentView = NSHostingView(rootView: SettingsView(model: model))
            if let screen = NSScreen.main { window.setContentSize(NSSize(width: min(760, screen.visibleFrame.width - 40), height: min(600, screen.visibleFrame.height - 70))) }
            window.center(); window.setFrameAutosaveName("AieyesSettings"); settingsWindow = window
        }
        if let window = settingsWindow { fitToVisibleScreen(window) }
        settingsWindow?.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
    }
    private func prepareUpdate() async -> Bool {
        guard !model.busy, !model.quotaBusy, !model.serverBusy, !model.settingsSaving, !model.repricing, model.estimateBusy.isEmpty else { return false }
        guard settingsWindow?.attachedSheet == nil else { settingsWindow?.makeKeyAndOrderFront(nil); return false }
        guard model.settingsDirty else { return true }
        openSettings()
        guard let window = settingsWindow else { return false }
        let alert = NSAlert()
        alert.messageText = "更新前保存配置更改？"
        alert.informativeText = "更新会重启 Aieyes。采样记录将保留。"
        alert.addButton(withTitle: "保存并更新"); alert.addButton(withTitle: "放弃更改并更新"); alert.addButton(withTitle: "取消")
        let response: NSApplication.ModalResponse = await withCheckedContinuation { continuation in
            alert.beginSheetModal(for: window) { continuation.resume(returning: $0) }
        }
        if response == .alertFirstButtonReturn { return await model.saveSettingsDraft() }
        if response == .alertSecondButtonReturn { model.discardSettingsDraft(); return true }
        return false
    }
    private func openEstimate(_ quota: Quota, credits: Bool = false) {
        let key = quota.id + (credits ? ":credits" : ":quota")
        popover.performClose(nil)
        let window = estimateWindows[key] ?? NSWindow(contentRect: NSRect(x: 0, y: 0, width: 530, height: 600), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        if estimateWindows[key] == nil {
            window.title = quota.name + (credits ? " · credit 价值" : " · 额度估值"); window.isReleasedWhenClosed = false
            window.contentMinSize = NSSize(width: 490, height: 360)
            window.center(); estimateWindows[key] = window
        }
        window.title = quota.name + (credits ? " · credit 价值" : " · 额度估值")
        window.contentView = NSHostingView(rootView: QuotaEstimateView(model: model, quota: quota, onClose: { [weak window] in window?.close() }, credits: credits))
        fitToVisibleScreen(window); window.deminiaturize(nil); window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
    }
    private func openSampling() {
        popover.performClose(nil)
        if samplingWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 530, height: 400), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            window.title = "管理采样"; window.isReleasedWhenClosed = false
            window.contentMinSize = NSSize(width: 470, height: 260)
            window.contentView = NSHostingView(rootView: SamplingManagementView(model: model))
            window.center(); samplingWindow = window
        }
        if let window = samplingWindow { fitToVisibleScreen(window); window.deminiaturize(nil); window.makeKeyAndOrderFront(nil) }
        NSApp.activate(ignoringOtherApps: true)
    }
    private func openQuotaOrder() {
        popover.performClose(nil)
        if quotaOrderWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 488, height: 430), styleMask: [.titled, .closable], backing: .buffered, defer: false)
            window.title = "调整账户顺序"; window.isReleasedWhenClosed = false; window.center(); quotaOrderWindow = window
        }
        guard let window = quotaOrderWindow else { return }
        window.contentView = NSHostingView(rootView: QuotaOrderView(model: model, onClose: { [weak window] in window?.close() }))
        fitToVisibleScreen(window); window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
    }
    private func openPanelAccounts() {
        popover.performClose(nil)
        if panelAccountsWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 488, height: 528), styleMask: [.titled, .closable], backing: .buffered, defer: false)
            window.title = "面板显示账户"; window.isReleasedWhenClosed = false; window.center(); panelAccountsWindow = window
        }
        guard let window = panelAccountsWindow else { return }
        window.contentView = NSHostingView(rootView: PanelAccountsView(model: model, onClose: { [weak window] in window?.close() }))
        fitToVisibleScreen(window); window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
    }
    private func fitToVisibleScreen(_ window: NSWindow) {
        guard let screen = window.screen ?? NSScreen.main else { return }
        let visible = screen.visibleFrame.insetBy(dx: 8, dy: 8)
        var frame = window.frame
        frame.size.width = min(frame.width, visible.width); frame.size.height = min(frame.height, visible.height)
        frame.origin.x = min(max(frame.minX, visible.minX), visible.maxX-frame.width)
        frame.origin.y = min(max(frame.minY, visible.minY), visible.maxY-frame.height)
        window.setFrame(frame, display: true)
    }
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard sender === settingsWindow else { return true }
        if approvingSettingsClose { approvingSettingsClose = false; return true }
        guard !model.settingsSaving, !model.repricing, sender.attachedSheet == nil else { return false }
        guard model.settingsDirty else { return true }
        let alert = NSAlert()
        alert.messageText = "保存配置更改？"
        alert.informativeText = "关闭设置前，保存全部配置，或放弃本次未保存的更改。Aieyes 将继续在菜单栏运行。"
        alert.addButton(withTitle: "保存")
        alert.addButton(withTitle: "放弃更改")
        alert.addButton(withTitle: "取消")
        alert.buttons[2].keyEquivalent = "\u{1b}"
        alert.beginSheetModal(for: sender) { [weak self, weak sender] response in
            guard let self, let sender else { return }
            if response == .alertFirstButtonReturn {
                Task { @MainActor in
                    if await self.model.saveSettingsDraft() { self.approvingSettingsClose = true; sender.performClose(nil) }
                }
            } else if response == .alertSecondButtonReturn {
                self.model.discardSettingsDraft(); self.approvingSettingsClose = true; sender.performClose(nil)
            }
        }
        return false
    }
    func windowWillClose(_ notification: Notification) {
        if (notification.object as? NSWindow) === detailWindow { model.setWindowVisible(false, window: "detail") }
    }
    func windowDidChangeOcclusionState(_ notification: Notification) {
        guard let window = notification.object as? NSWindow, window === detailWindow else { return }
        model.setWindowVisible(window.occlusionState.contains(.visible) && !window.isMiniaturized, window: "detail")
    }
    func windowDidMiniaturize(_ notification: Notification) {
        if (notification.object as? NSWindow) === detailWindow { model.setWindowVisible(false, window: "detail") }
    }
    func windowDidDeminiaturize(_ notification: Notification) {
        if (notification.object as? NSWindow) === detailWindow { model.setWindowVisible(true, window: "detail") }
    }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if settingsWindow?.attachedSheet != nil { settingsWindow?.makeKeyAndOrderFront(nil); return .terminateCancel }
        guard model != nil else { return .terminateNow }
        guard !model.settingsSaving, !model.repricing else { return .terminateCancel }
        guard model.settingsDirty else { return .terminateNow }
        openSettings()
        guard let window = settingsWindow, window.attachedSheet == nil else { return .terminateCancel }
        let alert = NSAlert()
        alert.messageText = "退出前保存配置更改？"
        alert.informativeText = "保存全部配置，或放弃本次未保存的更改。"
        alert.addButton(withTitle: "保存并退出")
        alert.addButton(withTitle: "放弃更改并退出")
        alert.addButton(withTitle: "取消")
        alert.buttons[2].keyEquivalent = "\u{1b}"
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self else { sender.reply(toApplicationShouldTerminate: false); return }
            if response == .alertFirstButtonReturn {
                Task { @MainActor in sender.reply(toApplicationShouldTerminate: await self.model.saveSettingsDraft()) }
            } else if response == .alertSecondButtonReturn {
                self.model.discardSettingsDraft(); sender.reply(toApplicationShouldTerminate: true)
            } else { sender.reply(toApplicationShouldTerminate: false) }
        }
        return .terminateLater
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationWillTerminate(_ notification: Notification) {
        if let globalClickMonitor { NSEvent.removeMonitor(globalClickMonitor) }; if let localEventMonitor { NSEvent.removeMonitor(localEventMonitor) }
        model?.engine.stop(); model?.metricsEngine.stop()
    }
    private func capture<V: View>(_ content: V, size: NSSize, dark: Bool, to url: URL, opaqueBackground: Bool = true) async throws {
        let view = NSHostingView(rootView: content.frame(width: size.width, height: size.height).background(opaqueBackground ? Color(nsColor: .windowBackgroundColor) : Color.clear).environment(\.colorScheme, dark ? .dark : .light))
        view.frame = NSRect(origin: .zero, size: size)
        let window = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.isOpaque = opaqueBackground
        window.backgroundColor = opaqueBackground ? .windowBackgroundColor : .clear
        window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        window.contentView = view
        window.setFrameOrigin(NSPoint(x: -20000, y: -20000))
        window.orderBack(nil)
        defer { window.close() }
        try await Task.sleep(nanoseconds: 200_000_000)
        view.layoutSubtreeIfNeeded()
        view.displayIfNeeded()
        guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { throw ClientError.message("界面渲染失败") }
        view.cacheDisplay(in: view.bounds, to: bitmap)
        if let data = bitmap.representation(using: .png, properties: [:]) { try data.write(to: url) }
    }
    private func renderSnapshots(to directory: String) async {
        do {
            model.settings = try await model.engine.call("settings.get"); model.settingsDraft = model.settings; model.settingsLoaded = true
            await model.reload()
            await model.loadPrices()
            await model.refreshSessions()
            await model.testNetwork()
            if ProcessInfo.processInfo.environment["AIEYES_UI_SCENARIO"] != nil { await model.scan(); await model.refreshQuotas(); await model.sampleHosts() }
            let root = URL(fileURLWithPath: directory)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            for dark in [false, true] {
                try await capture(RootView(model: model), size: NSSize(width: 450, height: 720), dark: dark, to: root.appendingPathComponent("translucent-panel-\(dark ? "dark" : "light").png"), opaqueBackground: false)
                try await capture(RootView(model: model).environment(\.previewAccessibleSurfaces, true), size: NSSize(width: 450, height: 720), dark: dark, to: root.appendingPathComponent("opaque-panel-\(dark ? "dark" : "light").png"), opaqueBackground: false)
                for compact in [true, false] {
                    try await capture(RootView(model: model, compact: compact), size: NSSize(width: compact ? 450 : 1080, height: compact ? 720 : 1000), dark: dark, to: root.appendingPathComponent("\(compact ? "menubar" : "dashboard")-\(dark ? "dark" : "light").png"))
                }
                let sessions = model.sessions, unavailable = model.sessionsUnavailable
                model.sessionsUnavailable = false
                for phase in [SessionPhase.idle, .working, .thinking, .tool, .complete, .interrupted, .unknown] {
                    model.sessions = phase == .idle ? [] : [LiveSession(id: "preview", source: "Codex", phase: phase, updatedAt: Date())]
                    try await capture(MenuActivityLabel(model: model), size: NSSize(width: 180, height: 28), dark: dark, to: root.appendingPathComponent("menu-symbol-\(phase.symbol)-\(dark ? "dark" : "light").png"))
                }
                model.sessions = sessions; model.sessionsUnavailable = unavailable
            }
            try await capture(TokenBreakdown(tokens: model.dashboard.summary.tokens, inline: true), size: NSSize(width: 414, height: 90), dark: false, to: root.appendingPathComponent("token-breakdown.png"))
            for tab in ["sources", "servers", "prices", "wakeups", "general"] {
                model.settingsTab = tab
                for dark in [false, true] { try await capture(SettingsView(model: model), size: NSSize(width: 760, height: 600), dark: dark, to: root.appendingPathComponent("settings-\(tab)-\(dark ? "dark" : "light").png")) }
            }
            for dark in [false, true] {
                try await capture(RootView(model: model, compact: false, page: "servers"), size: NSSize(width: 1080, height: 800), dark: dark, to: root.appendingPathComponent("servers-\(dark ? "dark" : "light").png"))
                if let host = model.settings.hosts.first {
                    for compact in [false, true] {
                        let key = "server.expanded." + (compact ? "panel." : "detail.") + host.id
                        let previous = UserDefaults.standard.object(forKey: key)
                        defer { if let previous { UserDefaults.standard.set(previous, forKey: key) } else { UserDefaults.standard.removeObject(forKey: key) } }
                        for expanded in [false, true] {
                            UserDefaults.standard.set(expanded, forKey: key)
                            let card = ServerCard(host: host, result: model.hosts.first { $0.id == host.id }, compact: compact)
                            let name = "server-" + (compact ? "panel" : "minimum") + (expanded ? "-expanded" : "-folded") + (dark ? "-dark" : "-light") + ".png"
                            try await capture(card, size: NSSize(width: compact ? 360 : 620, height: expanded ? 900 : 640), dark: dark, to: root.appendingPathComponent(name))
                        }
                    }
                }
                model.panelHeight = 540
                try await capture(RootView(model: model), size: NSSize(width: 450, height: 540), dark: dark, to: root.appendingPathComponent("low-panel-\(dark ? "dark" : "light").png"))
                model.panelHeight = 720
                try await capture(RootView(model: model, compact: false), size: NSSize(width: 760, height: 480), dark: dark, to: root.appendingPathComponent("minimum-detail-\(dark ? "dark" : "light").png"))
                try await capture(RootView(model: model).environment(\.previewAccessibleSurfaces, true), size: NSSize(width: 450, height: 720), dark: dark, to: root.appendingPathComponent("increased-contrast-\(dark ? "dark" : "light").png"))
                for (kind, id) in [("host", model.settings.hosts.first?.id), ("source", model.settings.sources.first?.id), ("account", model.settings.accounts.first?.key)] {
                    if let id, let request = model.removalRequest(kind, id: id) {
                        try await capture(DangerConfirmation(title: request.title, explanation: request.explanation, affected: request.affected, confirmLabel: "确认移除", cancel: {}, confirm: {}), size: NSSize(width: 510, height: 340), dark: dark, to: root.appendingPathComponent("confirm-\(kind)-\(dark ? "dark" : "light").png"))
                    }
                }
            }
            model.settingsTab = "prices"
            try await capture(SettingsView(model: model), size: NSSize(width: 620, height: 440), dark: false, to: root.appendingPathComponent("settings-prices-small.png"))
            let savedSources = model.settings.sources, savedDashboard = model.dashboard
            model.settings.sources = []; model.settingsDraft = model.settings; model.dashboard = Dashboard()
            model.panelHeight = 540
            try await capture(RootView(model: model), size: NSSize(width: 450, height: 540), dark: false, to: root.appendingPathComponent("onboarding-small.png"))
            model.settings.sources = savedSources; model.settingsDraft = model.settings; model.dashboard = savedDashboard
            model.selectedSource = savedSources.first?.id ?? "all"
            model.selectedModel = model.dashboard.modelOptions?.first ?? "gpt-review"
            try await capture(RootView(model: model), size: NSSize(width: 450, height: 540), dark: false, to: root.appendingPathComponent("filtered-panel-small.png"))
            model.selectedSource = "all"; model.selectedModel = "all"; model.panelHeight = 720
            model.selectedModel = "ui-filter-failure"
            await model.reload()
            try await capture(RootView(model: model, compact: false), size: NSSize(width: 760, height: 480), dark: false, to: root.appendingPathComponent("filter-failed.png"))
            model.selectedModel = "all"; await model.reload()
            try await capture(RootView(model: model, compact: false), size: NSSize(width: 760, height: 480), dark: false, to: root.appendingPathComponent("filter-restored.png"))
            if let quota = model.dashboard.quotas.first {
                try await capture(QuotaCard(quota: quota), size: NSSize(width: 420, height: 450), dark: false, to: root.appendingPathComponent("single-quota.png"))
            }
            for dark in [false, true] {
                if let agy = model.dashboard.quotas.first(where: { $0.provider == "agy" }) {
                    try await capture(QuotaCard(quota: agy, compact: true), size: NSSize(width: 414, height: 340), dark: dark, to: root.appendingPathComponent("agy-compact-\(dark ? "dark" : "light").png"))
                }
                if let account = model.dashboard.quotas.first(where: { $0.provider == "codex" }) {
                    try await capture(QuotaEstimateView(model: model, quota: account), size: NSSize(width: 518, height: 620), dark: dark, to: root.appendingPathComponent("quota-estimate-\(dark ? "dark" : "light").png"))
                    try await capture(QuotaEstimateView(model: model, quota: account, credits: true), size: NSSize(width: 518, height: 620), dark: dark, to: root.appendingPathComponent("credit-estimate-\(dark ? "dark" : "light").png"))
                }
            }
            try await capture(PanelAccountsView(model: model, onClose: {}), size: NSSize(width: 488, height: 528), dark: false, to: root.appendingPathComponent("panel-accounts.png"))
            if let quota = model.dashboard.quotas.first {
                var overrides = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
                overrides["quota.expanded.v2.panel." + quota.id] = false
                UserDefaults.standard.setVolatileDomain(overrides, forName: UserDefaults.argumentDomain)
                try await capture(QuotaCard(quota: quota, compact: true, collapsible: true), size: NSSize(width: 414, height: 200), dark: false, to: root.appendingPathComponent("quota-folded-summary.png"))
                try await capture(QuotaCard(quota: quota, compact: true, collapsible: true), size: NSSize(width: 414, height: 200), dark: true, to: root.appendingPathComponent("quota-folded-summary-dark.png"))
                var dormant = quota
                for index in dormant.windows.indices { dormant.windows[index].resetsAt = Date().timeIntervalSince1970 - 86400 }
                try await capture(QuotaCard(quota: dormant), size: NSSize(width: 414, height: 450), dark: false, to: root.appendingPathComponent("quota-dormant-reset.png"))
                dormant.windows = [QuotaWindow(name: "5h", usedPercent: 0, windowMinutes: 300, resetsAt: Date().timeIntervalSince1970 + 7200), QuotaWindow(name: "7d", usedPercent: 20, windowMinutes: 10080, resetsAt: Date().timeIntervalSince1970 + 172800)]
                try await capture(QuotaCard(quota: dormant), size: NSSize(width: 414, height: 450), dark: false, to: root.appendingPathComponent("quota-full-window-reset.png"))
            }
            try await capture(QuotaOrderView(model: model), size: NSSize(width: 488, height: 430), dark: false, to: root.appendingPathComponent("quota-order.png"))
            for provider in ["agy", "deepseek"] {
                let source = AgentSource(name: Format.provider(provider), provider: provider, path: "")
                try await capture(SourceEditor(source: source, hosts: model.settings.hosts, accounts: model.settings.accounts, onSave: { _, _, _ in }), size: NSSize(width: 570, height: 660), dark: false, to: root.appendingPathComponent(provider + "-source.png"))
            }
            if let host = model.settings.hosts.first {
                try await capture(HostEditor(host: host, onSave: { _, _ in }), size: NSSize(width: 620, height: 650), dark: false, to: root.appendingPathComponent("host-editor.png"))
            }
            if let source = model.settings.sources.first(where: { $0.hostId != nil }) {
                try await capture(SourceEditor(source: source, hosts: model.settings.hosts, accounts: model.settings.accounts, onSave: { _, _, _ in }), size: NSSize(width: 570, height: 660), dark: false, to: root.appendingPathComponent("remote-source.png"))
            }
        } catch { fputs("\(error.localizedDescription)\n", stderr); model.engine.stop(); model.metricsEngine.stop(); exit(1) }
        model.engine.stop(); model.metricsEngine.stop(); NSApp.terminate(nil)
    }
}

private final class StatusHostingView: NSHostingView<MenuActivityLabel> {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
