import SwiftUI
import AppKit
import Combine

@main struct AieyesApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    var body: some Scene { SwiftUI.Settings { EmptyView() } }
}

@MainActor final class AppDelegate: NSObject, NSApplicationDelegate, NSPopoverDelegate, NSWindowDelegate {
    private var statusItem: NSStatusItem!
    private var statusLabel: NSHostingView<MenuActivityLabel>?
    private let popover = NSPopover()
    private var detailWindow: NSWindow?
    private var settingsWindow: NSWindow?
    private var samplingWindow: NSWindow?
    private var estimateWindows: [String: NSWindow] = [:]
    private var approvingSettingsClose = false
    private var cancellables = Set<AnyCancellable>()
    private var model: AppModel!

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        if let index = CommandLine.arguments.firstIndex(of: "--render"), CommandLine.arguments.count > index + 1 {
            model = AppModel(autostart: false)
            Task { await renderSnapshots(to: CommandLine.arguments[index + 1]) }
            return
        }
        model = AppModel()
        AppUpdater.shared.prepareInstallation = { [weak self] finalizing in
            guard let self, await self.prepareUpdate() else { return false }
            // Saving a draft can yield to another window's operation; recheck before suspending work.
            guard !self.model.busy, !self.model.quotaBusy, !self.model.serverBusy, !self.model.settingsSaving, self.model.estimateBusy.isEmpty else { return false }
            self.model.installingUpdate = finalizing
            return true
        }
        AppUpdater.shared.finishedInstallationAttempt = { [weak self] in self?.model.installingUpdate = false }
        AppUpdater.shared.start()
        model.showSettings = { [weak self] in self?.openSettings() }
        model.showDetail = { [weak self] in self?.openDetail() }
        model.showEstimate = { [weak self] quota in self?.openEstimate(quota) }
        model.showSampling = { [weak self] in self?.openSampling() }
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
        popover.behavior = .transient; popover.animates = true; popover.delegate = self
        popover.contentViewController = NSHostingController(rootView: RootView(model: model))
        popover.contentSize = NSSize(width: 450, height: 720)
        model.objectWillChange.sink { [weak self] _ in DispatchQueue.main.async { self?.updateTitle() } }.store(in: &cancellables)
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(wake), name: NSWorkspace.didWakeNotification, object: nil)
        if CommandLine.arguments.contains("--show-detail") { openDetail() }
    }
    @objc private func togglePopover() {
        guard let button = statusItem.button else { return }
        if NSApp.currentEvent?.type == .rightMouseUp {
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
            NSApp.activate(ignoringOtherApps: true); popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY); model.setWindowVisible(true, window: "panel") }
    }
    func popoverDidClose(_ notification: Notification) { model.setWindowVisible(false, window: "panel") }
    @objc private func detailAction() { openDetail() }
    @objc private func settingsAction() { openSettings() }
    @objc private func samplingAction() { openSampling() }
    @objc private func wake() { Task { await model.scan() } }
    private func updateTitle() {
        guard let button = statusItem.button, let label = statusLabel else { return }
        let width = label.fittingSize.width
        statusItem.length = width
        label.frame = NSRect(x: 0, y: 0, width: width, height: button.bounds.height)
        button.toolTip = "Aieyes · " + model.sessionSummary + (model.sessionPhase.map { " · " + $0.rawValue } ?? "")
        if !model.runningEstimates.isEmpty { button.toolTip = (button.toolTip ?? "Aieyes") + " · " + model.samplingSummary }
        button.setAccessibilityLabel(button.toolTip)
        popover.behavior = model.isPinned ? .applicationDefined : .transient
        settingsWindow?.isDocumentEdited = model.settingsDirty
    }
    private func openDetail() {
        popover.performClose(nil)
        if detailWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 800), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            window.contentMinSize = NSSize(width: 760, height: 480)
            window.title = "Aieyes"; window.titlebarAppearsTransparent = true; window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: RootView(model: model, compact: false)); window.center(); window.delegate = self
            window.setFrameAutosaveName("AieyesDetails"); detailWindow = window
        }
        if let window = detailWindow { fitToVisibleScreen(window) }
        detailWindow?.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true); model.setWindowVisible(true, window: "detail")
    }
    private func openSettings() {
        popover.performClose(nil)
        if settingsWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 600), styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
            window.title = "Aieyes 设置"; window.isReleasedWhenClosed = false; window.delegate = self
            window.contentMinSize = NSSize(width: 620, height: 440)
            window.contentView = NSHostingView(rootView: SettingsView(model: model))
            if let screen = NSScreen.main { window.setContentSize(NSSize(width: min(760, screen.visibleFrame.width - 40), height: min(600, screen.visibleFrame.height - 70))) }
            window.center(); window.setFrameAutosaveName("AieyesSettings"); settingsWindow = window
        }
        if let window = settingsWindow { fitToVisibleScreen(window) }
        settingsWindow?.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
    }
    private func prepareUpdate() async -> Bool {
        guard !model.busy, !model.quotaBusy, !model.serverBusy, !model.settingsSaving, model.estimateBusy.isEmpty else { return false }
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
    private func openEstimate(_ quota: Quota) {
        popover.performClose(nil)
        let window = estimateWindows[quota.id] ?? NSWindow(contentRect: NSRect(x: 0, y: 0, width: 530, height: 600), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        if estimateWindows[quota.id] == nil {
            window.title = quota.name + " · 7d 整周价值"; window.isReleasedWhenClosed = false
            window.contentMinSize = NSSize(width: 490, height: 360)
            window.center(); estimateWindows[quota.id] = window
        }
        window.contentView = NSHostingView(rootView: QuotaEstimateView(model: model, quota: quota, onClose: { [weak window] in window?.close() }))
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
        guard !model.settingsSaving, sender.attachedSheet == nil else { return false }
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
    func windowDidMiniaturize(_ notification: Notification) {
        if (notification.object as? NSWindow) === detailWindow { model.setWindowVisible(false, window: "detail") }
    }
    func windowDidDeminiaturize(_ notification: Notification) {
        if (notification.object as? NSWindow) === detailWindow { model.setWindowVisible(true, window: "detail") }
    }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if settingsWindow?.attachedSheet != nil { settingsWindow?.makeKeyAndOrderFront(nil); return .terminateCancel }
        guard model != nil, model.settingsDirty else { return .terminateNow }
        guard !model.settingsSaving else { return .terminateCancel }
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
    func applicationWillTerminate(_ notification: Notification) { model?.engine.stop(); model?.metricsEngine.stop() }
    private func capture<V: View>(_ content: V, size: NSSize, dark: Bool, to url: URL) async throws {
        let view = NSHostingView(rootView: content.frame(width: size.width, height: size.height).background(Color(nsColor: .windowBackgroundColor)).environment(\.colorScheme, dark ? .dark : .light))
        view.frame = NSRect(origin: .zero, size: size)
        let window = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
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
            let root = URL(fileURLWithPath: directory)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            for dark in [false, true] {
                for compact in [true, false] {
                    try await capture(RootView(model: model, compact: compact), size: NSSize(width: compact ? 450 : 1080, height: compact ? 720 : 1000), dark: dark, to: root.appendingPathComponent("\(compact ? "menubar" : "dashboard")-\(dark ? "dark" : "light").png"))
                }
            }
            try await capture(TokenBreakdown(tokens: model.dashboard.summary.tokens, inline: true), size: NSSize(width: 414, height: 90), dark: false, to: root.appendingPathComponent("token-breakdown.png"))
            for tab in ["sources", "servers", "prices", "wakeups", "general"] {
                model.settingsTab = tab
                try await capture(SettingsView(model: model), size: NSSize(width: 760, height: 600), dark: false, to: root.appendingPathComponent("settings-\(tab).png"))
            }
            model.settingsTab = "prices"
            try await capture(SettingsView(model: model), size: NSSize(width: 620, height: 460), dark: false, to: root.appendingPathComponent("settings-prices-small.png"))
            let savedSources = model.settings.sources
            model.settings.sources = []; model.settingsDraft = model.settings
            model.panelHeight = 540
            try await capture(RootView(model: model), size: NSSize(width: 450, height: 540), dark: false, to: root.appendingPathComponent("onboarding-small.png"))
            model.settings.sources = savedSources; model.settingsDraft = model.settings
            model.selectedSource = savedSources.first?.id ?? "all"
            model.selectedModel = model.dashboard.modelOptions?.first ?? "gpt-review"
            try await capture(RootView(model: model), size: NSSize(width: 450, height: 540), dark: false, to: root.appendingPathComponent("filtered-panel-small.png"))
            model.selectedSource = "all"; model.selectedModel = "all"; model.panelHeight = 720
            if let quota = model.dashboard.quotas.first {
                try await capture(QuotaCard(quota: quota), size: NSSize(width: 420, height: 450), dark: false, to: root.appendingPathComponent("single-quota.png"))
            }
            for dark in [false, true] {
                if let agy = model.dashboard.quotas.first(where: { $0.provider == "agy" }) {
                    try await capture(QuotaCard(quota: agy, compact: true), size: NSSize(width: 414, height: 340), dark: dark, to: root.appendingPathComponent("agy-compact-\(dark ? "dark" : "light").png"))
                }
                if let account = model.dashboard.quotas.first(where: { $0.provider == "codex" }) {
                    try await capture(QuotaEstimateView(model: model, quota: account), size: NSSize(width: 518, height: 620), dark: dark, to: root.appendingPathComponent("quota-estimate-\(dark ? "dark" : "light").png"))
                    model.creditEstimateMode = true
                    try await capture(QuotaEstimateView(model: model, quota: account), size: NSSize(width: 518, height: 620), dark: dark, to: root.appendingPathComponent("credit-estimate-\(dark ? "dark" : "light").png"))
                    model.creditEstimateMode = false
                }
            }
            try await capture(QuotaOrderView(model: model), size: NSSize(width: 488, height: 430), dark: false, to: root.appendingPathComponent("quota-order.png"))
            for provider in ["agy", "deepseek"] {
                let source = AgentSource(name: Format.provider(provider), provider: provider, path: "")
                try await capture(SourceEditor(source: source, hosts: model.settings.hosts, accounts: model.settings.accounts, onSave: { _, _, _ in }), size: NSSize(width: 570, height: 660), dark: false, to: root.appendingPathComponent(provider + "-source.png"))
            }
            if let host = model.settings.hosts.first {
                try await capture(HostEditor(host: host, onSave: { _ in }), size: NSSize(width: 620, height: 650), dark: false, to: root.appendingPathComponent("host-editor.png"))
            }
            if let source = model.settings.sources.first(where: { $0.hostId != nil }) {
                try await capture(SourceEditor(source: source, hosts: model.settings.hosts, accounts: model.settings.accounts, onSave: { _, _, _ in }), size: NSSize(width: 570, height: 660), dark: false, to: root.appendingPathComponent("remote-source.png"))
            }
        } catch { fputs("\(error.localizedDescription)\n", stderr) }
        NSApp.terminate(nil)
    }
}

private final class StatusHostingView: NSHostingView<MenuActivityLabel> {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
