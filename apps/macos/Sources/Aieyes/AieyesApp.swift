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
        model.showSettings = { [weak self] in self?.openSettings() }
        model.showDetail = { [weak self] in self?.openDetail() }
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "eye", accessibilityDescription: "Aieyes")
            button.image?.isTemplate = true; button.imagePosition = .imageLeading
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
            menu.addItem(.separator())
            menu.addItem(withTitle: "退出 Aieyes", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
            statusItem.menu = menu; button.performClick(nil); statusItem.menu = nil; return
        }
        if popover.isShown { popover.performClose(nil) }
        else { NSApp.activate(ignoringOtherApps: true); popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY); model.panelVisible = true }
    }
    func popoverDidClose(_ notification: Notification) { model.panelVisible = detailWindow?.isVisible == true }
    @objc private func detailAction() { openDetail() }
    @objc private func settingsAction() { openSettings() }
    @objc private func wake() { Task { await model.scan() } }
    private func updateTitle() {
        guard let button = statusItem.button, let label = statusLabel else { return }
        let width = label.fittingSize.width
        statusItem.length = width
        label.frame = NSRect(x: 0, y: 0, width: width, height: button.bounds.height)
        button.toolTip = "Aieyes · " + model.sessionSummary + (model.sessionPhase.map { " · " + $0.rawValue } ?? "")
        button.setAccessibilityLabel(button.toolTip)
        popover.behavior = model.isPinned ? .applicationDefined : .transient
    }
    private func openDetail() {
        popover.performClose(nil)
        if detailWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 800), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            window.title = "Aieyes"; window.titlebarAppearsTransparent = true; window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: RootView(model: model, compact: false)); window.center(); window.delegate = self
            window.setFrameAutosaveName("AieyesDetails"); detailWindow = window
        }
        detailWindow?.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true); model.panelVisible = true
    }
    private func openSettings() {
        popover.performClose(nil)
        if settingsWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 600), styleMask: [.titled, .closable], backing: .buffered, defer: false)
            window.title = "Aieyes 设置"; window.isReleasedWhenClosed = false; window.center(); settingsWindow = window
        }
        settingsWindow?.contentView = NSHostingView(rootView: SettingsView(model: model))
        settingsWindow?.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
    }
    func windowWillClose(_ notification: Notification) { if (notification.object as? NSWindow) === detailWindow { model.panelVisible = popover.isShown } }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationWillTerminate(_ notification: Notification) { model?.engine.stop() }
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
            model.settings = try await model.engine.call("settings.get")
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
            for tab in ["sources", "servers", "prices", "general"] {
                model.settingsTab = tab
                try await capture(SettingsView(model: model), size: NSSize(width: 760, height: 600), dark: false, to: root.appendingPathComponent("settings-\(tab).png"))
            }
            if let quota = model.dashboard.quotas.first {
                try await capture(QuotaCard(quota: quota), size: NSSize(width: 420, height: 450), dark: false, to: root.appendingPathComponent("single-quota.png"))
            }
            for provider in ["agy", "deepseek"] {
                let source = AgentSource(name: Format.provider(provider), provider: provider, path: "")
                try await capture(SourceEditor(source: source, hosts: model.settings.hosts, accounts: model.settings.accounts, onSave: { _, _ in }), size: NSSize(width: 570, height: 660), dark: false, to: root.appendingPathComponent(provider + "-source.png"))
            }
            if let host = model.settings.hosts.first {
                try await capture(HostEditor(host: host, onSave: { _ in }), size: NSSize(width: 620, height: 650), dark: false, to: root.appendingPathComponent("host-editor.png"))
            }
            if let source = model.settings.sources.first(where: { $0.hostId != nil }) {
                try await capture(SourceEditor(source: source, hosts: model.settings.hosts, accounts: model.settings.accounts, onSave: { _, _ in }), size: NSSize(width: 570, height: 660), dark: false, to: root.appendingPathComponent("remote-source.png"))
            }
        } catch { fputs("\(error.localizedDescription)\n", stderr) }
        NSApp.terminate(nil)
    }
}

private final class StatusHostingView: NSHostingView<MenuActivityLabel> {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
