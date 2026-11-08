import AppKit
import SwiftUI
import Combine

private final class DesktopDisplayPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

private struct DesktopDisplayContent: View {
    @ObservedObject var store: DisplayStore
    var ribbon: Bool
    @AppStorage(DisplayPreferences.cardSize) private var savedSize = DisplayCardSize.medium.rawValue
    var body: some View {
        let size = DisplayCardSize(rawValue: savedSize) ?? .medium
        Group {
            if ribbon { DisplayRibbon(snapshot: store.snapshot) }
            else { DisplayCard(snapshot: store.snapshot, size: size, live: true) }
        }
        .background(DisplayCardBackground(phase: store.snapshot.phase))
        .clipShape(RoundedRectangle(cornerRadius: ribbon ? 22 : 22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 22).strokeBorder(.primary.opacity(0.09), lineWidth: 1))
        .help(L("单击打开总览，拖动移动，右键显示选项"))
    }
}

private final class DraggableDisplayHost<Content: View>: NSHostingView<Content> {
    var open: (() -> Void)?
    var options: (() -> NSMenu)?
    var moved: (() -> Void)?
    var spokenLabel: (() -> String)?
    private var start: NSPoint?
    private var windowOrigin: NSPoint?
    private var dragged = false
    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .button }
    override func accessibilityLabel() -> String? { spokenLabel?() }
    override func accessibilityHelp() -> String? { L("单击打开总览，拖动移动，右键显示选项") }
    override func accessibilityPerformPress() -> Bool { open?(); return open != nil }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func mouseDown(with event: NSEvent) {
        guard let window else { return }
        start = window.convertPoint(toScreen: event.locationInWindow); windowOrigin = window.frame.origin; dragged = false
    }
    override func mouseDragged(with event: NSEvent) {
        guard let start, let windowOrigin, let window else { return }
        let pointer = window.convertPoint(toScreen: event.locationInWindow)
        let dx = pointer.x - start.x, dy = pointer.y - start.y
        if hypot(dx, dy) > 4 { dragged = true }
        if dragged && !UserDefaults.standard.bool(forKey: DisplayPreferences.locked) {
            window.setFrameOrigin(NSPoint(x: windowOrigin.x + dx, y: windowOrigin.y + dy))
        }
    }
    override func mouseUp(with event: NSEvent) {
        guard start != nil else { return }
        if dragged { moved?() } else { open?() }
        start = nil; windowOrigin = nil; dragged = false
    }
    override func rightMouseDown(with event: NSEvent) { if let menu = options?() { NSMenu.popUpContextMenu(menu, with: event, for: self) } }
}

@MainActor final class DesktopDisplaysController: NSObject {
    private let store: DisplayStore
    private let open: () -> Void
    private let lab: () -> Void
    private var panels: [String: NSPanel] = [:]
    private var subscriptions = Set<AnyCancellable>()
    private var pending = false

    init(store: DisplayStore, open: @escaping () -> Void, lab: @escaping () -> Void) {
        self.store = store; self.open = open; self.lab = lab
        super.init()
        NotificationCenter.default.publisher(for: UserDefaults.didChangeNotification).sink { [weak self] _ in self?.schedule() }.store(in: &subscriptions)
        NotificationCenter.default.publisher(for: NSApplication.didChangeScreenParametersNotification).sink { [weak self] _ in self?.clampAll() }.store(in: &subscriptions)
        NotificationCenter.default.publisher(for: DisplayPreferences.resetPosition).sink { [weak self] _ in self?.resetPositions() }.store(in: &subscriptions)
        sync()
    }
    private func schedule() {
        guard !pending else { return }; pending = true
        DispatchQueue.main.async { [weak self] in self?.pending = false; self?.sync() }
    }
    private func sync() {
        let defaults = UserDefaults.standard
        for key in ["card", "ribbon"] {
            let enabled = defaults.bool(forKey: key == "card" ? DisplayPreferences.cardEnabled : DisplayPreferences.ribbonEnabled)
            guard enabled else { panels[key]?.orderOut(nil); continue }
            let panel = panels[key] ?? makePanel(key)
            panel.title = key == "card" ? L("Aieyes 桌面卡片") : L("Aieyes 状态条")
            let size = key == "ribbon" ? NSSize(width: 320, height: 54) : (DisplayCardSize(rawValue: defaults.string(forKey: DisplayPreferences.cardSize) ?? "") ?? .medium).dimensions
            let old = panel.frame
            if old.size != size { panel.setFrame(NSRect(x: old.minX, y: old.maxY - size.height, width: size.width, height: size.height), display: true) }
            let floating = defaults.bool(forKey: DisplayPreferences.onTop)
            panel.level = floating ? .floating : NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopIconWindow)) + 1)
            var behavior: NSWindow.CollectionBehavior = [.stationary, .ignoresCycle, .fullScreenNone]
            if defaults.object(forKey: DisplayPreferences.allSpaces) as? Bool != false { behavior.insert(.canJoinAllSpaces) }
            panel.collectionBehavior = behavior
            panel.setFrame(Self.clamped(panel.frame, screens: NSScreen.screens.map(\.visibleFrame)), display: true)
            if !panel.isVisible { panel.orderFrontRegardless() }
        }
    }
    private func makePanel(_ key: String) -> NSPanel {
        let size = key == "ribbon" ? CGSize(width: 320, height: 54) : (DisplayCardSize(rawValue: UserDefaults.standard.string(forKey: DisplayPreferences.cardSize) ?? "") ?? .medium).dimensions
        let panel = DesktopDisplayPanel(contentRect: NSRect(origin: .zero, size: size), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.title = key == "card" ? L("Aieyes 桌面卡片") : L("Aieyes 状态条")
        panel.isReleasedWhenClosed = false; panel.hidesOnDeactivate = false
        panel.isFloatingPanel = true; panel.becomesKeyOnlyIfNeeded = true
        panel.isOpaque = false; panel.backgroundColor = .clear; panel.hasShadow = true
        panel.isMovableByWindowBackground = false
        let host = DraggableDisplayHost(rootView: DesktopDisplayContent(store: store, ribbon: key == "ribbon"))
        host.open = open
        host.spokenLabel = { [weak store, weak panel] in
            guard let snapshot = store?.snapshot else { return panel?.title ?? "Aieyes" }
            return (panel?.title ?? "Aieyes") + " · " + snapshot.text(snapshot.phase.title) + " · " + DisplaySnapshot.compact(snapshot.totalTokens) + " Token"
        }
        host.options = { [weak self] in self?.menu(key) ?? NSMenu() }
        host.moved = { [weak self, weak panel] in
            guard let panel else { return }
            panel.setFrame(Self.clamped(panel.frame, screens: NSScreen.screens.map(\.visibleFrame)), display: true)
            self?.savePosition(key, panel: panel)
        }
        host.setAccessibilityRole(.button); host.setAccessibilityLabel(panel.title)
        host.setAccessibilityHelp(L("单击打开总览，拖动移动，右键显示选项"))
        panel.contentView = host
        if let point = UserDefaults.standard.dictionary(forKey: "display." + key + ".position"), let x = point["x"] as? Double, let y = point["y"] as? Double, x.isFinite, y.isFinite {
            panel.setFrameOrigin(NSPoint(x: x, y: y))
        } else { panel.setFrameOrigin(defaultOrigin(key, size: size)) }
        panels[key] = panel
        return panel
    }
    private func defaultOrigin(_ key: String, size: CGSize) -> CGPoint {
        let screen = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1280, height: 800)
        return key == "ribbon" ? NSPoint(x: screen.midX - size.width / 2, y: screen.maxY - size.height - 16) : NSPoint(x: screen.maxX - size.width - 26, y: screen.maxY - size.height - 90)
    }
    static func clamped(_ frame: NSRect, screens: [NSRect]) -> NSRect {
        guard !screens.isEmpty else { return frame }
        let screen = screens.max { a, b in
            let left = frame.intersection(a), right = frame.intersection(b)
            return (left.isNull ? 0 : left.width * left.height) < (right.isNull ? 0 : right.width * right.height)
        } ?? screens[0]
        let x = min(max(frame.minX, screen.minX + 8), max(screen.minX + 8, screen.maxX - frame.width - 8))
        let y = min(max(frame.minY, screen.minY + 8), max(screen.minY + 8, screen.maxY - frame.height - 8))
        return NSRect(x: x, y: y, width: frame.width, height: frame.height)
    }
    private func clampAll() {
        for (key, panel) in panels {
            panel.setFrame(Self.clamped(panel.frame, screens: NSScreen.screens.map(\.visibleFrame)), display: true)
            savePosition(key, panel: panel)
        }
    }
    private func savePosition(_ key: String, panel: NSPanel) {
        UserDefaults.standard.set(["x": panel.frame.minX, "y": panel.frame.minY], forKey: "display." + key + ".position")
    }
    private func resetPositions() {
        for key in ["card", "ribbon"] {
            UserDefaults.standard.removeObject(forKey: "display." + key + ".position")
            if let panel = panels[key] { panel.setFrameOrigin(defaultOrigin(key, size: panel.frame.size)) }
        }
        clampAll()
    }
    private func menu(_ key: String) -> NSMenu {
        let menu = NSMenu()
        for (title, action) in [(L("打开总览"), #selector(openAction)), (L("显示实验室"), #selector(labAction)), (L("锁定位置"), #selector(lockAction)), (L("隐藏此显示"), #selector(hideAction(_:)))] {
            let item = menu.addItem(withTitle: title, action: action, keyEquivalent: "")
            item.target = self; item.representedObject = key
            if action == #selector(lockAction) { item.state = UserDefaults.standard.bool(forKey: DisplayPreferences.locked) ? .on : .off }
        }
        return menu
    }
    @objc private func openAction() { open() }
    @objc private func labAction() { lab() }
    @objc private func lockAction() { UserDefaults.standard.set(!UserDefaults.standard.bool(forKey: DisplayPreferences.locked), forKey: DisplayPreferences.locked) }
    @objc private func hideAction(_ sender: NSMenuItem) { UserDefaults.standard.set(false, forKey: sender.representedObject as? String == "ribbon" ? DisplayPreferences.ribbonEnabled : DisplayPreferences.cardEnabled) }
    func stop() { subscriptions.removeAll(); for panel in panels.values { panel.close() }; panels.removeAll() }
}
