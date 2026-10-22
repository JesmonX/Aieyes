import SwiftUI
import AppKit

/// A single retained workspace; changing pages does not tear down editors or scroll views.
struct MainWindowView: View {
    let model: AppModel
    @StateObject private var navigation: WindowNavigationStore
    @MainActor init(model: AppModel) {
        self.model = model; _navigation = StateObject(wrappedValue: WindowNavigationStore(model: model))
    }
    @AppStorage("main.sidebarCollapsed") private var collapsed = false
    @AppStorage(NativePanelKey.detailPage.rawValue) private var savedPage = "agent"
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.previewAccessibleSurfaces) private var previewAccessible
    @State private var visitedSettings = false
    @State private var guideActive = false
    @State private var workspaceWidth: CGFloat = 1120
    private var monitoring: [(String, String, String)] { [("agent", L("Agent 总览"), "square.grid.2x2"), ("analysis", L("用量分析"), "chart.xyaxis.line"), ("servers", L("服务器"), "server.rack")] }
    private var configuration: [(String, String, String)] { [("integrations", L("接入与账户"), "person.crop.circle"), ("servers", L("机器与采集"), "desktopcomputer"), ("prices", L("模型价格"), "dollarsign.circle"), ("wakeups", L("定时唤醒"), "alarm")] }
    private var preferences: [(String, String, String)] { [("appearance", L("外观"), "paintpalette"), ("desktop", L("桌宠与会话"), "pawprint"), ("network", L("网络代理"), "network"), ("refresh", L("刷新与历史"), "clock.arrow.circlepath"), ("notifications", L("通知"), "bell"), ("updates", L("更新与关于"), "info.circle")] }
    private var title: String {
        if model.detailPage == "setup" { return L("配置向导") }
        if model.detailPage != "settings" { return monitoring.first { $0.0 == model.detailPage }?.1 ?? L("Agent 总览") }
        return model.settingsTab == "general" ? preferences.first { $0.0 == model.generalPage }?.1 ?? L("外观") : configuration.first { $0.0 == model.settingsTab }?.1 ?? L("接入与账户")
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Button { withAnimation(reduceMotion ? nil : .spring(response: 0.24, dampingFraction: 0.88)) { collapsed.toggle() } } label: { BrandMark().frame(width: 30, height: 30).frame(width: 34, height: 34).contentShape(Rectangle()) }.buttonStyle(.plain).help(collapsed ? L("展开导航") : L("收起导航")).accessibilityLabel(collapsed ? L("展开导航") : L("收起导航")).accessibilityValue(collapsed ? L("已收起") : L("已展开"))
                    if !collapsed { Text("Aieyes").font(AppFont.title); Spacer() }
                }.padding(.horizontal, 12).padding(.top, 18)
                ScrollView {
                    VStack(alignment: .leading, spacing: 4) {
                        groupLabel(L("监测"))
                        ForEach(monitoring, id: \.0) { key, title, icon in
                            navigationButton(title, icon: icon, selected: model.detailPage == key) { select(key) }
                        }
                        groupLabel(L("配置"))
                        navigationButton(L("配置向导"), icon: "checklist", selected: model.detailPage == "setup") { select("setup") }
                        ForEach(configuration, id: \.0) { key, title, icon in
                            navigationButton(title, icon: icon, selected: model.detailPage == "settings" && model.settingsTab == key) { model.openSettings(.init(category: key)); select("settings") }
                        }
                        groupLabel(L("偏好"))
                        ForEach(preferences, id: \.0) { key, title, icon in
                            navigationButton(title, icon: icon, selected: model.detailPage == "settings" && model.settingsTab == "general" && model.generalPage == key) { model.openSettings(.init(category: "general", section: key)); select("settings") }
                        }
                    }.padding(.horizontal, 8).padding(.bottom, 12)
                }.scrollIndicators(.hidden)
            }.frame(width: collapsed ? 62 : 214).background {
                if reduceTransparency || previewAccessible || contrast == .increased { Color(nsColor: .windowBackgroundColor) }
                else { Rectangle().fill(.ultraThinMaterial) }
            }
            Divider().opacity(0.5)
            VStack(alignment: .leading, spacing: 0) {
                Text(title).font(AppFont.title).padding(.horizontal, 22).padding(.vertical, 15)
                if guideActive && model.detailPage == "settings" {
                    HStack { Button(L("配置向导")) { select("setup") }; Spacer() }.padding(.horizontal, 22).padding(.bottom, 12)
                }
                ZStack {
                    if model.detailPage == "setup" { SetupGuideView(model: model) { guideActive = false; select("agent") } }
                    ForEach(monitoring, id: \.0) { key, _, _ in
                        RootView(model: model, compact: false, page: key).equatable()
                            .opacity(model.detailPage == key ? 1 : 0)
                            .allowsHitTesting(model.detailPage == key)
                            .disabled(model.detailPage != key)
                            .accessibilityHidden(model.detailPage != key)
                    }
                    if visitedSettings || model.detailPage == "settings" {
                        SettingsView(model: model)
                            .environment(\.surfaceActive, model.detailPage == "settings" && model.isWindowVisible("detail"))
                            .opacity(model.detailPage == "settings" ? 1 : 0)
                            .allowsHitTesting(model.detailPage == "settings")
                            .disabled(model.detailPage != "settings")
                            .accessibilityHidden(model.detailPage != "settings")
                    }
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            }.frame(minWidth: 0, maxWidth: .infinity, maxHeight: .infinity)
        }.frame(minWidth: 760, minHeight: 480).aieyesAccent()
        .environment(\.workspaceCardColumns, workspaceWidth >= 1800 ? 4 : workspaceWidth >= 1390 ? 3 : workspaceWidth >= 974 ? 2 : 1)
        .background { GeometryReader { geometry in Color.clear.onChange(of: geometry.size.width, initial: true) { _, width in workspaceWidth = width } } }
        .onAppear { select(model.requestedDetailPage ?? model.detailPage); model.requestedDetailPage = nil }
        .onChange(of: model.requestedDetailPage) { _, page in if let page { select(page); model.requestedDetailPage = nil } }
        .onChange(of: model.detailPage) { _, page in select(page) }
        .onChange(of: model.visibleWindows) { _, _ in model.settingsWindowVisible = model.detailPage == "settings" && model.isWindowVisible("detail") }
    }
    @ViewBuilder private func groupLabel(_ text: String) -> some View {
        if collapsed { Divider().padding(.vertical, 6) }
        else { Text(text).font(AppFont.caption).foregroundStyle(.secondary).padding(.horizontal, 10).padding(.top, 14).padding(.bottom, 3) }
    }
    private func navigationButton(_ title: String, icon: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 10) { Image(systemName: icon).frame(width: 22); if !collapsed { Text(title); Spacer(minLength: 0) } }
                .font(AppFont.body).padding(.horizontal, 10).padding(.vertical, 9)
                .frame(maxWidth: .infinity, alignment: collapsed ? .center : .leading)
                .foregroundStyle(selected ? Palette.accent : Color.secondary)
                .background(selected ? Palette.accent.opacity(0.16) : .clear, in: RoundedRectangle(cornerRadius: 8))
                .contentShape(Rectangle())
        }.buttonStyle(.plain).help(title).accessibilityLabel(title).accessibilityAddTraits(selected ? .isSelected : [])
    }
    private func select(_ value: String) {
        let page = ["agent", "analysis", "servers", "settings", "setup"].contains(value) ? value : "agent"
        if model.detailPage != page { model.detailPage = page }
        if !CommandLine.arguments.contains("--render") { savedPage = page }
        if page == "settings" { visitedSettings = true }
        if page == "setup" { guideActive = true; if !CommandLine.arguments.contains("--render") { UserDefaults.standard.set(true, forKey: "setup.seen.v1") } }
        model.settingsWindowVisible = page == "settings" && model.isWindowVisible("detail")
        model.setServerVisible(page == "servers", window: "detail")
        if page == "servers" { Task { await model.sampleHosts() } }
    }
}

struct PersistentSection<Content: View>: View {
    let title: String, detail: String
    var tint: Color
    @AppStorage private var expanded: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @ViewBuilder var content: Content
    init(key: String, title: String, detail: String = "", tint: Color = .secondary, initiallyExpanded: Bool = true, @ViewBuilder content: () -> Content) {
        self.title = title; self.detail = detail; self.tint = tint; self.content = content()
        _expanded = AppStorage(wrappedValue: initiallyExpanded, key)
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        DisclosureGroup(isExpanded: $expanded) { content.padding(.top, 8) } label: {
            HStack(spacing: 7) { Circle().fill(tint).frame(width: 7, height: 7); Text(title).font(AppFont.secondary.weight(.medium)); Spacer(minLength: 0); Text(detail).font(AppFont.caption).foregroundStyle(detail.contains("⚠") ? Palette.warn : Color.secondary) }
        }.animation(reduceMotion ? nil : .spring(response: 0.24, dampingFraction: 0.9), value: expanded)
    }
}

/// Records the native clip view, so refreshes never reset the reading position.
struct ScrollMemory: NSViewRepresentable {
    let key: String
    var ready = true
    @Environment(\.surfaceActive) private var active
    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeNSView(context: Context) -> NSView { NSView() }
    func updateNSView(_ view: NSView, context: Context) {
        context.coordinator.setActive(active)
        DispatchQueue.main.async { context.coordinator.attach(view.enclosingScrollView, key: key, ready: ready) }
    }
    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) { coordinator.detach() }
    @MainActor final class Coordinator {
        private weak var scroll: NSScrollView?
        private var observer: NSObjectProtocol?
        private var key = ""
        private var restorePoint: NSPoint?
        private var lastPoint: NSPoint?
        private var active = true
        private var ready = false
        func setActive(_ value: Bool) {
            if active && !value { ScrollPositionStore.shared.flush(key) }
            if !active && value && !key.isEmpty { restorePoint = ScrollPositionStore.shared.position(for: key) }
            active = value
        }
        func attach(_ next: NSScrollView?, key: String, ready: Bool) {
            self.ready = ready
            guard let next else { return }
            if scroll === next, self.key == key { if ready && active { restore() }; return }
            detach(); scroll = next; self.key = key
            if !CommandLine.arguments.contains("--render") {
                restorePoint = ScrollPositionStore.shared.position(for: key)
            }
            if ready && active { restore() }
            next.contentView.postsBoundsChangedNotifications = true
            observer = NotificationCenter.default.addObserver(forName: NSView.boundsDidChangeNotification, object: next.contentView, queue: .main) { [weak self, weak next] _ in
                MainActor.assumeIsolated {
                    guard let self, let next, self.active, self.ready, self.restorePoint == nil, !CommandLine.arguments.contains("--render"), next.window?.isVisible == true else { return }
                    let point = next.contentView.bounds.origin
                    guard self.lastPoint != point else { return }; self.lastPoint = point
                    ScrollPositionStore.shared.record(point, for: self.key)
                }
            }
        }
        private func restore() {
            guard let next = scroll, let point = restorePoint, let document = next.documentView else { return }
            let size = next.contentView.bounds.size
            next.contentView.scroll(to: NSPoint(x: max(0, min(point.x, document.bounds.width - size.width)), y: max(0, min(point.y, document.bounds.height - size.height))))
            next.reflectScrolledClipView(next.contentView); lastPoint = next.contentView.bounds.origin; restorePoint = nil
        }
        func detach() { ScrollPositionStore.shared.flush(key); if let observer { NotificationCenter.default.removeObserver(observer) }; observer = nil; scroll = nil; restorePoint = nil; lastPoint = nil }
        deinit { if let observer { NotificationCenter.default.removeObserver(observer) } }
    }
}
