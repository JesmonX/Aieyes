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
    private var petWindow: PetWindowController?
    private let dockPresence = DockPresence()
    private var appearanceController: AppAppearanceController?
    private var detailWindow: NSWindow?
    private var accountSwitchWindow: NSWindow?
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
    private var measuredMenu: MenuStatusSnapshot?
    private var measuredShowCount: Bool?
    private var measuredScale: CGFloat?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        if let index = CommandLine.arguments.firstIndex(of: "--render"), CommandLine.arguments.count > index + 1 {
            UserDefaults.standard.setVolatileDomain(NativePanelKey.snapshotOverrides(), forName: UserDefaults.argumentDomain)
            model = AppModel(autostart: false)
            Task { await renderSnapshots(to: CommandLine.arguments[index + 1]) }
            return
        }
        model = AppModel()
        appearanceController = AppAppearanceController(popover: popover)
        model.$settings.map(\.appearance).removeDuplicates().sink { [weak self] appearance in
            self?.appearanceController?.apply(theme: appearance?.theme ?? "system", accent: appearance?.accent ?? "indigo", chartPalette: appearance?.chartPalette ?? "classic", chartContrast: appearance?.chartContrast ?? "high")
        }.store(in: &cancellables)
        AppUpdater.shared.prepareInstallation = { [weak self] _ in
            guard let self else { return false }
            return await self.prepareUpdate()
        }
        AppUpdater.shared.finishedInstallationAttempt = { [weak self] in Task { _ = await EngineClient.coordinateUpdate("operations.resume"); self?.model.installingUpdate = false } }
        AppUpdater.shared.start()
        model.showSettings = { [weak self] in self?.openSettings() }
        model.showDetailPage = { [weak self] page in self?.openDetail(page: page) }
        model.$settingsLoaded.filter { $0 }.prefix(1).sink { [weak self] _ in
            guard let self, !UserDefaults.standard.bool(forKey: "setup.seen.v1") else { return }
            UserDefaults.standard.set(true, forKey: "setup.seen.v1")
            if SetupGuide.needsSetup(self.model.settings) { self.openDetail(page: "setup") }
        }.store(in: &cancellables)
        model.showEstimate = { [weak self] quota, credits, group in self?.openEstimate(quota, credits: credits, group: group) }
        model.showAccountSwitch = { [weak self] key in self?.openAccountSwitch(key) }
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
        appearanceController?.updatePanel()
        popover.contentSize = NSSize(width: 450, height: 720)
        petWindow = PetWindowController(model: model, checkUpdates: { AppUpdater.shared.check() }, open: { [weak self] anchor, toggle in
            guard let self else { return }
            if self.popover.isShown { if toggle { self.popover.performClose(nil) }; return }
            NSApp.activate(ignoringOtherApps: true)
            self.popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minY)
            self.model.setWindowVisible(true, window: "panel")
        })
        model.live.objectWillChange.sink { [weak self] _ in self?.scheduleTitleUpdate() }.store(in: &cancellables)
        model.objectWillChange.sink { [weak self] _ in self?.scheduleTitleUpdate() }.store(in: &cancellables)
        model.menuStatus.$snapshot.dropFirst().sink { [weak self] _ in self?.scheduleTitleUpdate() }.store(in: &cancellables)
        NotificationCenter.default.publisher(for: UserDefaults.didChangeNotification).sink { [weak self] _ in self?.scheduleTitleUpdate() }.store(in: &cancellables)
        for name in [NSWindow.didChangeBackingPropertiesNotification, NSApplication.didChangeScreenParametersNotification] {
            NotificationCenter.default.publisher(for: name).sink { [weak self] _ in
                self?.measuredMenu = nil; self?.scheduleTitleUpdate()
            }.store(in: &cancellables)
        }
        NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification).sink { [weak self] _ in
            self?.measuredMenu = nil; self?.scheduleTitleUpdate()
        }.store(in: &cancellables)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(wake), name: NSWorkspace.didWakeNotification, object: nil)
        globalClickMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown]) { [weak self] _ in
            Task { @MainActor in self?.dismissOutsidePanel() }
        }
        localEventMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .otherMouseDown, .keyDown]) { [weak self] event in
            guard let self else { return event }
            if event.type == .keyDown, event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command, event.charactersIgnoringModifiers == "," {
                self.openSettings(); return nil
            }
            if event.type == .keyDown, event.window === self.detailWindow, event.modifierFlags.contains(.command), let key = event.charactersIgnoringModifiers, ["1", "2", "3"].contains(key) {
                self.model.requestedDetailPage = key == "1" ? "agent" : key == "2" ? "analysis" : "servers"; return nil
            }
            guard self.popover.isShown else { return event }
            if event.type == .keyDown {
                if event.keyCode == 53, event.window === self.popover.contentViewController?.view.window { self.popover.performClose(nil); return nil }
            } else if let window = event.window,
                      window !== self.popover.contentViewController?.view.window,
                      window !== self.statusItem.button?.window,
                      window !== self.petWindow?.window,
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
            menu.autoenablesItems = false
            let visible = UserDefaults.standard.object(forKey: "aieyes.pet.enabled") as? Bool ?? true
            for (title, action) in NativeContextMenu.entries(pet: false, visible: visible, sampling: !model.runningEstimates.isEmpty) {
                if action == "separator" { menu.addItem(.separator()); continue }
                let selector: Selector = switch action {
                case "open": #selector(detailAction)
                case "settings": #selector(settingsAction)
                case "sampling": #selector(samplingAction)
                case "check-updates": #selector(updateAction)
                case "hide": #selector(togglePetAction)
                default: #selector(quitAction)
                }
                let item = menu.addItem(withTitle: title, action: selector, keyEquivalent: action == "settings" ? "," : action == "quit" ? "q" : "")
                item.target = self
            }
            statusItem.menu = menu; button.performClick(nil); statusItem.menu = nil; return
        }
        if popover.isShown { popover.performClose(nil) }
        else { showPopover() }
    }
    private func showPopover() {
        guard let button = statusItem.button else { return }
        model.panelHeight = min(720, max(360, (button.window?.screen?.visibleFrame.height ?? 800) - 34))
        popover.contentSize = NSSize(width: 450, height: model.panelHeight)
        appearanceController?.updatePanel()
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        model.setWindowVisible(true, window: "panel")
        NSApp.activate(ignoringOtherApps: true)
    }
    private func dismissOutsidePanel() { if popover.isShown && !model.isPinned { popover.performClose(nil) } }
    @objc private func applicationDeactivated() { dismissOutsidePanel() }
    func popoverDidClose(_ notification: Notification) { model.setWindowVisible(false, window: "panel") }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        let windows = [detailWindow, panelAccountsWindow, quotaOrderWindow, samplingWindow].compactMap { $0 } + Array(estimateWindows.values)
        if let window = windows.first(where: { $0.isKeyWindow }) ?? windows.first(where: { $0.isVisible || $0.isMiniaturized }) {
            showWindow(window)
        } else { showPopover() }
        return false
    }
    private func showWindow(_ window: NSWindow) {
        dockPresence.open(window)
        window.deminiaturize(nil)
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }
    @objc private func detailAction() { openDetail() }
    @objc private func settingsAction() { model.openSettings() }
    @objc private func updateAction() { AppUpdater.shared.check() }
    @objc private func togglePetAction() {
        let visible = UserDefaults.standard.object(forKey: "aieyes.pet.enabled") as? Bool ?? true
        UserDefaults.standard.set(!visible, forKey: "aieyes.pet.enabled")
    }
    @objc private func quitAction() { NSApp.terminate(nil) }
    @objc private func samplingAction() { openSampling() }
    @objc private func wake() { model.resumeAfterWake() }
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
        let snapshot = model.menuStatus.snapshot
        let showCount = UserDefaults.standard.bool(forKey: NativePanelKey.showCount.rawValue)
        if measuredMenu != snapshot || measuredShowCount != showCount || measuredScale != scale {
            label.layoutSubtreeIfNeeded()
            let width = ceil(label.fittingSize.width * scale) / scale
            if statusItem.length != width { statusItem.length = width }
            measuredMenu = snapshot; measuredShowCount = showCount; measuredScale = scale
        }
        let width = statusItem.length
        let frame = NSRect(x: 0, y: 0, width: width, height: button.bounds.height)
        if label.frame != frame { label.frame = frame }
        var tooltip = "Aieyes · 今日全部数据 · " + model.sessionSummary + (model.sessionPhase.map { " · " + $0.rawValue } ?? "")
        if !model.runningEstimates.isEmpty { tooltip += " · " + model.samplingSummary }
        if button.toolTip != tooltip { button.toolTip = tooltip; button.setAccessibilityLabel(tooltip) }
        if let window = detailWindow, window.isDocumentEdited != model.settingsDirty { window.isDocumentEdited = model.settingsDirty }

    }
    private func openDetail(page: String? = nil) {
        if let page { model.requestedDetailPage = page; model.detailPage = page }
        else if detailWindow == nil, let saved = UserDefaults.standard.string(forKey: NativePanelKey.detailPage.rawValue), ["agent", "analysis", "servers", "settings"].contains(saved) { model.detailPage = saved }
        popover.performClose(nil)
        if detailWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 800), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            let toolbar = NSToolbar(identifier: "AieyesDetailsToolbar"); toolbar.delegate = self; toolbar.displayMode = .iconOnly; window.toolbar = toolbar; window.toolbarStyle = .unified
            window.contentMinSize = NSSize(width: 760, height: 480)
            window.title = "Aieyes"; window.titlebarAppearsTransparent = true; window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: MainWindowView(model: model)); window.center(); window.delegate = self
            window.setFrameAutosaveName("AieyesDetails"); detailWindow = window
        }
        if let window = detailWindow { fitToVisibleScreen(window) }
        if let window = detailWindow { showWindow(window) }; model.setWindowVisible(true, window: "detail")
    }
    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] { [.flexibleSpace, .init("refresh")] }
    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] { toolbarAllowedItemIdentifiers(toolbar) }
    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier, willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        let item = NSToolbarItem(itemIdentifier: identifier)
        if identifier.rawValue == "refresh" {
            let button = NSPopUpButton(frame: .zero, pullsDown: true); button.addItem(withTitle: "刷新")
            for (index, title) in ["同步记录", "刷新额度", "同步价格", "刷新服务器"].enumerated() { let entry = NSMenuItem(title: title, action: #selector(toolbarRefresh(_:)), keyEquivalent: ""); entry.tag = index; entry.target = self; button.menu?.addItem(entry) }
            button.setAccessibilityLabel("刷新选项"); item.view = button; item.label = "刷新"
        }
        return item
    }
    @objc private func toolbarRefresh(_ sender: NSMenuItem) { Task { switch sender.tag { case 0: await model.scan(); case 1: await model.refreshQuotas(); case 2: await model.syncPrices(); default: await model.sampleHosts() } } }
    private func openSettings() {
        openDetail(page: "settings")
        model.settingsWindowVisible = true
    }
    private func prepareUpdate() async -> Bool {
        model.installingUpdate = true
        var completed = false
        defer { if !completed { model.installingUpdate = false } }
        model.settingsRefreshTask?.cancel()
        var started: [String: Date] = [:], deadline = Date().addingTimeInterval(30)
        func blockers() -> [String] { Array(Set(model.updateBlockers + Array(AppUpdater.shared.operationBlockers.values))).sorted() }
        while !blockers().isEmpty {
            for task in blockers() where started[task] == nil { started[task] = Date() }
            let tasks = blockers().map { $0 + "（已等待 " + String(Int(Date().timeIntervalSince(started[$0] ?? Date()))) + " 秒）" }.joined(separator: "、")
            AppUpdater.shared.preparationStatus("等待当前任务完成：" + tasks)
            if Date() >= deadline {
                guard await updateChoice("更新仍在等待", message: tasks + "。可取消等待，回到对应页面处理。", buttons: ["继续等待", "取消并返回"]) == 0 else { return false }
                deadline = Date().addingTimeInterval(30)
            }
            do { try await Task.sleep(for: .milliseconds(250)) } catch { return false }
        }
        // Prepare nested editors first; saving a child may update its parent's draft.
        let editors = AppUpdater.shared.editorPreparations.sorted { $0.value.order > $1.value.order }.map(\.key)
        for id in editors {
            guard let editor = AppUpdater.shared.editorPreparations[id] else { continue }
            guard await editor.prepare() else { detailWindow?.makeKeyAndOrderFront(nil); return false }
            try? await Task.sleep(for: .milliseconds(100))
        }
        guard model.settingsDirty else { completed = await quiesceForUpdate(); return completed }
        openSettings()
        let response = await updateChoice("更新前保存配置更改？", message: "更新会重启 Aieyes。采样记录将保留。", buttons: ["保存并更新", "放弃更改并更新", "取消"])
        if response == 0 { completed = await model.saveSettingsDraft(); if completed { completed = await quiesceForUpdate() }; if !completed { AppUpdater.shared.preparationError = model.settingsMessage ?? "设置保存失败，请检查设置页" }; return completed }
        if response == 1 { model.discardSettingsDraft(); completed = await quiesceForUpdate(); return completed }
        return false
    }
    private func quiesceForUpdate() async -> Bool {
        var blockers = await EngineClient.coordinateUpdate("operations.prepareUpdate")
        var deadline = Date().addingTimeInterval(30)
        while !blockers.isEmpty {
            AppUpdater.shared.preparationStatus("正在完成账户或配置操作：" + Array(Set(blockers)).sorted().joined(separator: "、"))
            do { try await Task.sleep(for: .milliseconds(250)) } catch { _ = await EngineClient.coordinateUpdate("operations.resume"); return false }
            if Date() >= deadline {
                guard await updateChoice("更新仍在等待", message: blockers.joined(separator: "、"), buttons: ["继续等待", "取消并返回"]) == 0 else { _ = await EngineClient.coordinateUpdate("operations.resume"); return false }
                deadline = Date().addingTimeInterval(30)
            }
            blockers = await EngineClient.coordinateUpdate("operations.status")
        }
        return true
    }
    private func openEstimate(_ quota: Quota, credits: Bool = false, group: String? = nil) {
        let key = quota.id + (credits ? ":credits" : ":quota")
        popover.performClose(nil)
        let window = estimateWindows[key] ?? NSWindow(contentRect: NSRect(x: 0, y: 0, width: 530, height: 600), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        if estimateWindows[key] == nil {
            window.title = quota.name + (credits ? " · credit 价值" : " · 额度估值"); window.isReleasedWhenClosed = false
            window.contentMinSize = NSSize(width: 490, height: 360)
            window.center(); estimateWindows[key] = window
        }
        window.title = quota.name + (credits ? " · credit 价值" : " · 额度估值")
        window.contentView = NSHostingView(rootView: QuotaEstimateView(model: model, quota: quota, onClose: { [weak window] in window?.close() }, credits: credits, initialGroup: group))
        fitToVisibleScreen(window); showWindow(window)
    }
    private func openAccountSwitch(_ key: String) {
        popover.performClose(nil)
        let window = accountSwitchWindow ?? NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 470), styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = "切换账户"; window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: AccountTargetsView(model: model, accountKey: key))
        if accountSwitchWindow == nil { window.center() }
        accountSwitchWindow = window; fitToVisibleScreen(window); showWindow(window)
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
        if let window = samplingWindow { fitToVisibleScreen(window); showWindow(window) }
    }
    private func openQuotaOrder() {
        popover.performClose(nil)
        if quotaOrderWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 488, height: 430), styleMask: [.titled, .closable], backing: .buffered, defer: false)
            window.title = "调整账户顺序"; window.isReleasedWhenClosed = false; window.center(); quotaOrderWindow = window
        }
        guard let window = quotaOrderWindow else { return }
        window.contentView = NSHostingView(rootView: QuotaOrderView(model: model, onClose: { [weak window] in window?.close() }))
        fitToVisibleScreen(window); showWindow(window)
    }
    private func openPanelAccounts() {
        popover.performClose(nil)
        if panelAccountsWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 488, height: 528), styleMask: [.titled, .closable], backing: .buffered, defer: false)
            window.title = "面板显示账户"; window.isReleasedWhenClosed = false; window.center(); panelAccountsWindow = window
        }
        guard let window = panelAccountsWindow else { return }
        window.contentView = NSHostingView(rootView: PanelAccountsView(model: model, onClose: { [weak window] in window?.close() }))
        fitToVisibleScreen(window); showWindow(window)
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
        guard sender === detailWindow else { return true }
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
        if (notification.object as? NSWindow) === detailWindow { model.settingsWindowVisible = false }
    }
    func windowDidChangeOcclusionState(_ notification: Notification) {
        guard let window = notification.object as? NSWindow else { return }
        let visible = window.occlusionState.contains(.visible) && !window.isMiniaturized
        if window === detailWindow { model.setWindowVisible(visible, window: "detail") }
        if window === detailWindow { model.settingsWindowVisible = visible && model.detailPage == "settings" }
    }
    func windowDidMiniaturize(_ notification: Notification) {
        if (notification.object as? NSWindow) === detailWindow { model.setWindowVisible(false, window: "detail") }
        if (notification.object as? NSWindow) === detailWindow { model.settingsWindowVisible = false }
    }
    func windowDidDeminiaturize(_ notification: Notification) {
        if (notification.object as? NSWindow) === detailWindow { model.setWindowVisible(true, window: "detail") }
        if (notification.object as? NSWindow) === detailWindow { model.settingsWindowVisible = model.detailPage == "settings" }
    }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if detailWindow?.attachedSheet != nil { detailWindow?.makeKeyAndOrderFront(nil); return .terminateCancel }
        guard model != nil else { return .terminateNow }
        guard !model.settingsSaving, !model.repricing else { return .terminateCancel }
        guard model.settingsDirty else { return .terminateNow }
        openSettings()
        guard let window = detailWindow, window.attachedSheet == nil else { return .terminateCancel }
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
        model?.stop()
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
    private func captureSetupGuide(to root: URL) async throws {
        let saved = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
        defer { UserDefaults.standard.setVolatileDomain(saved, forName: UserDefaults.argumentDomain) }
        for dark in [false, true] {
            let theme = dark ? "dark" : "light"
            var preview = saved
            preview["setup.scene.v1"] = "mixed"; preview["setup.expanded.v2"] = ""
            UserDefaults.standard.setVolatileDomain(preview, forName: UserDefaults.argumentDomain)
            try await capture(SetupGuideView(model: model, finish: {}), size: NSSize(width: 900, height: 800), dark: dark, to: root.appendingPathComponent("setup-guide-\(theme).png"))
            preview["setup.expanded.v2"] = "network"
            UserDefaults.standard.setVolatileDomain(preview, forName: UserDefaults.argumentDomain)
            try await capture(SetupGuideView(model: model, finish: {}), size: NSSize(width: 900, height: 1200), dark: dark, to: root.appendingPathComponent("setup-network-\(theme).png"))
            try await capture(SetupGuideView(model: model, finish: {}), size: NSSize(width: 546, height: 700), dark: dark, to: root.appendingPathComponent("setup-minimum-\(theme).png"))
            if let source = model.settings.sources.first(where: { $0.hostId != nil }), let machine = source.hostId, let account = model.settings.accounts.first(where: { $0.uses(source) }) {
                try await capture(AccountDevicesView(model: model, account: account, initialMachineID: machine, initialSection: "command"), size: NSSize(width: 680, height: 650), dark: dark, to: root.appendingPathComponent("setup-device-command-\(theme).png"))
            }
        }
    }
    private func renderSnapshots(to directory: String) async {
        do {
            model.settings = try await model.engine.call("settings.get"); model.settingsDraft = model.settings; model.settingsLoaded = true
            await model.loadAccountStatuses()
            await model.reload()
            await model.loadPrices()
            await model.refreshSessions()
            await model.loadPetLibrary()
            await model.testNetwork()
            if ProcessInfo.processInfo.environment["AIEYES_UI_SCENARIO"] != nil { await model.scan(); await model.refreshQuotas(); await model.sampleHosts() }
            UserDefaults.standard.setVolatileDomain(NativePanelKey.snapshotOverrides(panelQuotaExpanded: model.quotaAccounts.count <= 1), forName: UserDefaults.argumentDomain)
            let root = URL(fileURLWithPath: directory)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            if ProcessInfo.processInfo.environment["AIEYES_UI_LAYOUT_ONLY"] == "1" {
                for dark in [false, true] {
                    let theme = dark ? "dark" : "light"
                    if let quota = model.dashboard.quotas.first {
                        for width in [360, 414, 620, 1000] {
                            try await capture(QuotaCard(quota: quota, compact: true, collapsible: true, defaultExpanded: false), size: NSSize(width: width, height: 230), dark: dark, to: root.appendingPathComponent("quota-summary-\(width)-\(theme).png"))
                        }
                    }
                    if let host = model.settings.hosts.first {
                        for width in [360, 620, 1000] {
                            try await capture(ServerCard(host: host, result: model.hosts.first { $0.id == host.id }, compact: false), size: NSSize(width: width, height: 720), dark: dark, to: root.appendingPathComponent("server-four-gpu-\(width)-\(theme).png"))
                        }
                    }
                    try await capture(RootView(model: model), size: NSSize(width: 450, height: 720), dark: dark, to: root.appendingPathComponent("overview-panel-\(theme).png"))
                }
                model.stop(); NSApp.terminate(nil); return
            }
            if ProcessInfo.processInfo.environment["AIEYES_UI_SETUP_ONLY"] == "1" {
                try await captureSetupGuide(to: root)
                model.stop(); NSApp.terminate(nil); return
            }
            if ProcessInfo.processInfo.environment["AIEYES_UI_WORKSPACE_ONLY"] == "1" {
                for dark in [false, true] {
                    for page in ["agent", "analysis", "servers", "settings"] {
                        model.detailPage = page; model.settingsTab = "integrations"; model.settingsAgent = "codex"
                        try await capture(MainWindowView(model: model), size: NSSize(width: 1080, height: 800), dark: dark, to: root.appendingPathComponent("workspace-\(page)-\(dark ? "dark" : "light").png"))
                        if page == "agent" { try await capture(MainWindowView(model: model), size: NSSize(width: 1280, height: 900), dark: dark, to: root.appendingPathComponent("workspace-agent-1280-\(dark ? "dark" : "light").png")) }
                        try await capture(MainWindowView(model: model), size: NSSize(width: 760, height: 480), dark: dark, to: root.appendingPathComponent("workspace-\(page)-small-\(dark ? "dark" : "light").png"))
                    }
                    model.detailPage = "agent"
                    for height in [720, 440] {
                        model.panelHeight = CGFloat(height)
                        try await capture(RootView(model: model), size: NSSize(width: 450, height: height), dark: dark, to: root.appendingPathComponent("workspace-panel-\(height)-\(dark ? "dark" : "light").png"))
                    }
                    model.panelHeight = 720
                    try await capture(RootView(model: model).environment(\.previewAccessibleSurfaces, true), size: NSSize(width: 450, height: 720), dark: dark, to: root.appendingPathComponent("workspace-panel-accessible-\(dark ? "dark" : "light").png"))
                    var preview = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
                    preview[NativePanelKey.panelSessions.rawValue] = true
                    UserDefaults.standard.setVolatileDomain(preview, forName: UserDefaults.argumentDomain)
                    try await capture(RootView(model: model), size: NSSize(width: 450, height: 720), dark: dark, to: root.appendingPathComponent("workspace-sessions-\(dark ? "dark" : "light").png"))
                    preview[NativePanelKey.panelSessions.rawValue] = false
                    UserDefaults.standard.setVolatileDomain(preview, forName: UserDefaults.argumentDomain)
                }
                NSApp.terminate(nil); return
            }
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
            for tab in ["sources", "accounts", "servers", "prices", "wakeups", "general"] {
                model.settingsTab = tab
                for dark in [false, true] { try await capture(SettingsView(model: model), size: NSSize(width: 760, height: 600), dark: dark, to: root.appendingPathComponent("settings-\(tab)-\(dark ? "dark" : "light").png")) }
            }
            model.settingsTab = "servers"
            for dark in [false, true] {
                try await capture(SettingsView(model: model), size: NSSize(width: 620, height: 440), dark: dark, to: root.appendingPathComponent("settings-servers-small-\(dark ? "dark" : "light").png"))
            }
            model.settingsTab = "integrations"
            for agent in model.agentConfigurations {
                model.settingsAgent = agent.provider
                for dark in [false, true] {
                    try await capture(SettingsView(model: model), size: NSSize(width: 760, height: 600), dark: dark, to: root.appendingPathComponent("settings-agent-\(agent.provider)-\(dark ? "dark" : "light").png"))
                    if agent.provider != "deepseek" {
                        try await capture(AgentValuationView(model: model, provider: agent.provider), size: NSSize(width: 660, height: 650), dark: dark, to: root.appendingPathComponent("agent-valuation-\(agent.provider)-\(dark ? "dark" : "light").png"))
                    }
                }
            }
            model.settingsAgent = "codex"; model.settingsTab = "general"
            for page in ["appearance", "desktop", "network", "refresh", "notifications", "updates"] {
                model.generalPage = page
                for dark in [false, true] {
                    try await capture(SettingsView(model: model), size: NSSize(width: 760, height: 600), dark: dark, to: root.appendingPathComponent("settings-general-\(page)-\(dark ? "dark" : "light").png"))
                }
            }
            await model.loadPetLibrary()
            if !model.petLibrary.isEmpty {
                for dark in [false, true] {
                    try await capture(PetLibraryView(model: model).padding(20).environment(\.surfaceActive, true), size: NSSize(width: 530, height: 530), dark: dark, to: root.appendingPathComponent("pet-library-\(dark ? "dark" : "light").png"))
                }
            }
            model.generalPage = "appearance"
            if let connection = model.settings.codexApis?.first {
                for dark in [false, true] {
                    try await capture(Form { CodexAPISettingsSection(model: model) }.formStyle(.grouped), size: NSSize(width: 760, height: 600), dark: dark, to: root.appendingPathComponent("codex-api-settings-\(dark ? "dark" : "light").png"))
                    try await capture(CodexAPIEditor(model: model, connection: connection, onSaved: {}), size: NSSize(width: 630, height: 670), dark: dark, to: root.appendingPathComponent("codex-api-editor-\(dark ? "dark" : "light").png"))
                }
            }
            if let account = model.settings.accounts.first {
                for dark in [false, true] { try await capture(AccountDevicesView(model: model, account: account), size: NSSize(width: 680, height: 650), dark: dark, to: root.appendingPathComponent("account-devices-\(dark ? "dark" : "light").png")) }
            }
            for dark in [false, true] {
                try await capture(RootView(model: model, compact: false, page: "servers"), size: NSSize(width: 1080, height: 800), dark: dark, to: root.appendingPathComponent("servers-\(dark ? "dark" : "light").png"))
                if let host = model.settings.hosts.first {
                    for compact in [false, true] {
                        let key = NativePanelKey.expansion(.server, id: host.id, compact: compact)
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
            try await captureSetupGuide(to: root)
            for dark in [false, true] {
                let preview = SSHConfigResult(path: "~/.ssh/config", hosts: [.init(alias: "gpu-lab", hostName: "10.0.0.1", user: "alice", port: 2200), .init(alias: "dev-server", hostName: "dev.example", user: nil, port: nil)], warnings: ["Match 条件未执行；预览地址仅供参考，连接仍使用原 SSH 配置"])
                try await capture(SSHConfigImportView(model: model, preview: preview), size: NSSize(width: 650, height: 600), dark: dark, to: root.appendingPathComponent("ssh-import-\(dark ? "dark" : "light").png"))
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
                for dark in [false, true] {
                    try await capture(QuotaCard(quota: quota, deviceCounts: model.accountDeviceCounts(quota.id), onSwitch: {}), size: NSSize(width: 340, height: 450), dark: dark, to: root.appendingPathComponent("quota-device-counts-\(dark ? "dark" : "light").png"))
                }
                try await capture(QuotaCard(quota: quota), size: NSSize(width: 420, height: 450), dark: false, to: root.appendingPathComponent("single-quota.png"))
            }
            for dark in [false, true] {
                if let agy = model.dashboard.quotas.first(where: { $0.provider == "antigravity" }) {
                    try await capture(QuotaCard(quota: agy, compact: true, onEstimate: { _ in }, quotaHistory: (model.dashboard.quotaEstimates ?? []).filter { $0.accountKey == agy.id }), size: NSSize(width: 414, height: 460), dark: dark, to: root.appendingPathComponent("agy-compact-\(dark ? "dark" : "light").png"))
                }
                if let account = model.dashboard.quotas.first(where: { $0.provider == "codex" }) {
                    try await capture(QuotaEstimateView(model: model, quota: account), size: NSSize(width: 518, height: 620), dark: dark, to: root.appendingPathComponent("quota-estimate-\(dark ? "dark" : "light").png"))
                    try await capture(QuotaEstimateView(model: model, quota: account, credits: true), size: NSSize(width: 518, height: 620), dark: dark, to: root.appendingPathComponent("credit-estimate-\(dark ? "dark" : "light").png"))
                }
            }
            try await capture(PanelAccountsView(model: model, onClose: {}), size: NSSize(width: 488, height: 528), dark: false, to: root.appendingPathComponent("panel-accounts.png"))
            if let quota = model.dashboard.quotas.first {
                var overrides = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
                overrides[NativePanelKey.expansion(.quota, id: quota.id, compact: true)] = false
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
            for provider in ["antigravity", "deepseek"] {
                let source = AgentSource(name: Format.provider(provider), provider: provider, path: "")
                try await capture(SourceEditor(source: source, hosts: model.settings.hosts, onSave: { _ in }), size: NSSize(width: 570, height: 660), dark: false, to: root.appendingPathComponent(provider + "-source.png"))
            }
            if let host = model.settings.hosts.first {
                try await capture(HostEditor(host: host, onSave: { _, _ in }), size: NSSize(width: 620, height: 650), dark: false, to: root.appendingPathComponent("host-editor.png"))
            }
            if let source = model.settings.sources.first(where: { $0.hostId != nil }) {
                try await capture(SourceEditor(source: source, hosts: model.settings.hosts, onSave: { _ in }), size: NSSize(width: 570, height: 660), dark: false, to: root.appendingPathComponent("remote-source.png"))
            }
        } catch { fputs("\(error.localizedDescription)\n", stderr); model.stop(); exit(1) }
        model.stop(); NSApp.terminate(nil)
    }
}

private final class StatusHostingView: NSHostingView<MenuActivityLabel> {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
