import AppKit
import Foundation
import ServiceManagement
import SwiftUI

/// Panel width presets. The compact width is the one the panel already falls back to on
/// narrow displays; a screen that cannot fit the chosen width still shrinks it.
enum PanelWidth: String, CaseIterable, Identifiable {
    case standard, compact
    var id: String { rawValue }
    var points: CGFloat { self == .compact ? 360 : 450 }
    var title: String {
        switch self {
        case .standard: return L("标准")
        case .compact: return L("紧凑")
        }
    }
    static var current: PanelWidth { PanelWidth(rawValue: UserDefaults.standard.string(forKey: NativePanelKey.panelWidth.rawValue) ?? "") ?? .standard }
}

/// The Dock badge shows how many sessions wait for input. Aieyes is a menu bar app, so
/// the Dock tile only exists while a window is open; the badge follows either way.
@MainActor enum DockBadge {
    static let key = "aieyes.dock.badge"
    static func update(waiting: Int) {
        let shown = UserDefaults.standard.object(forKey: key) as? Bool ?? true
        let value = shown && waiting > 0 ? String(waiting) : nil
        if NSApp.dockTile.badgeLabel != value { NSApp.dockTile.badgeLabel = value }
    }
}

/// Login item registration through `SMAppService`. Registering needs a bundled app, so
/// snapshot rendering and unbundled verifier builds show the setting as unavailable.
@MainActor final class LoginItemController: ObservableObject {
    @Published private(set) var available = Bundle.main.bundleIdentifier != nil && !CommandLine.arguments.contains("--render")
    @Published private(set) var enabled = false
    @Published private(set) var needsApproval = false
    @Published private(set) var error: String?
    private var observers: [NSObjectProtocol] = []

    init() {
        refresh()
        // The user can change the login item in System Settings while the page is open.
        observers.append(NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        })
    }
    deinit { for observer in observers { NotificationCenter.default.removeObserver(observer) } }

    func refresh() {
        guard available else { return }
        let status = SMAppService.mainApp.status
        enabled = status == .enabled || status == .requiresApproval
        needsApproval = status == .requiresApproval
    }
    func set(_ value: Bool) {
        guard available else { return }
        error = nil
        do {
            if value { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
        } catch { self.error = error.localizedDescription }
        refresh()
    }
    func openSystemSettings() { SMAppService.openSystemSettingsLoginItems() }
}

/// `aieyes://` links navigate only. Any app or web page can open a URL scheme, so the
/// parser accepts a fixed set of destinations and never reads query items, fragments or
/// any other part of the link as an instruction.
enum AieyesLink: Equatable {
    case panel
    case page(String)
    case settings(category: String, section: String?)

    /// Detail pages a link may open; `overview` is accepted for the overview page.
    static let pages = ["agent", "analysis", "servers", "sessions"]
    /// Settings categories a link may open.
    static let categories = ["integrations": "integrations", "accounts": "integrations", "machines": "servers", "prices": "prices", "wakeups": "wakeups"]

    init?(_ url: URL) {
        guard url.scheme?.lowercased() == "aieyes" else { return nil }
        // `open` is accepted as a prefix: aieyes://open/sessions equals aieyes://sessions.
        let parts = ([url.host ?? ""] + url.pathComponents)
            .map { $0.lowercased() }
            .filter { !$0.isEmpty && $0 != "/" && $0 != "open" }
        guard let first = parts.first else { return nil }
        let rest = Array(parts.dropFirst())
        switch first {
        case "panel":
            self = .panel
        case "settings":
            if let name = rest.first, let category = Self.categories[name] { self = .settings(category: category, section: nil) }
            else if let name = rest.first, SettingsIndex.generalPageKeys.contains(name) { self = .settings(category: "general", section: name) }
            else { self = .settings(category: "general", section: nil) }
        default:
            let page = first == "overview" ? "agent" : first
            guard Self.pages.contains(page) else { return nil }
            self = .page(page)
        }
    }
}

/// Login item, Dock badge and the link list; one page under Settings → General.
struct SystemIntegrationSettings: View {
    @StateObject private var loginItem = LoginItemController()
    @AppStorage(DockBadge.key) private var dockBadge = true
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    private let links = ["aieyes://panel", "aieyes://overview", "aieyes://analysis", "aieyes://servers", "aieyes://sessions", "aieyes://settings"]
    var body: some View {
        let _ = interfaceLanguage
        Section(L("启动")) {
            Toggle(L("登录时启动 Aieyes"), isOn: Binding(get: { loginItem.enabled }, set: { loginItem.set($0) }))
                .disabled(!loginItem.available)
            if loginItem.needsApproval {
                HStack(spacing: 8) {
                    Text(L("已注册，需要在系统设置的“登录项”中允许。")).font(AppFont.secondary).foregroundStyle(.secondary)
                    Button(L("打开登录项")) { loginItem.openSystemSettings() }
                }
            }
            if let error = loginItem.error { ErrorNotice(message: error, context: L("登录项设置未完成")) }
            if !loginItem.available { Text(L("登录项需要在 Aieyes.app 中设置。")).font(AppFont.secondary).foregroundStyle(.secondary) }
        }
        Section("Dock") {
            Toggle(L("在 Dock 图标上显示等待输入的会话数"), isOn: $dockBadge)
            Text(L("打开窗口时 Dock 显示 Aieyes 图标，徽标为等待输入的会话数。")).font(AppFont.secondary).foregroundStyle(.secondary)
        }
        Section(L("应用链接")) {
            Text(L("其他应用、脚本和快捷指令可以用下面的链接打开面板或页面。链接只用于导航。")).font(AppFont.secondary).foregroundStyle(.secondary)
            ForEach(links, id: \.self) { link in
                HStack {
                    Text(verbatim: link).font(.system(size: DesignTokens.fontSecondary, design: .monospaced)).textSelection(.enabled)
                    Spacer()
                    Button(L("复制")) { Pasteboard.copy(link) }.buttonStyle(.plain).foregroundStyle(Palette.accent)
                }
            }
        }
    }
}
