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
    @State private var guideActive = false
    @State private var workspaceWidth: CGFloat = 1120
    private var monitoring: [(String, String, String)] { [("agent", L("总览"), "square.grid.2x2"), ("analysis", L("用量分析"), "chart.xyaxis.line"), ("servers", L("服务器"), "server.rack")] }
    private var configuration: [(String, String, String)] { [("integrations", L("接入与账户"), "person.crop.circle"), ("servers", L("机器与采集"), "desktopcomputer"), ("prices", L("模型价格"), "dollarsign.circle"), ("wakeups", L("定时唤醒"), "alarm")] }
    private var title: String {
        if model.detailPage == "setup" { return L("配置向导") }
        if model.detailPage != "settings" { return monitoring.first { $0.0 == model.detailPage }?.1 ?? L("总览") }
        return model.settingsTab == "general" ? L("设置") : configuration.first { $0.0 == model.settingsTab }?.1 ?? L("接入与账户")
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        NavigationSplitView {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Button { withAnimation(reduceMotion ? nil : .spring(response: 0.24, dampingFraction: 0.88)) { collapsed.toggle() } } label: { BrandMark().frame(width: 30, height: 30).frame(width: 34, height: 34).contentShape(Rectangle()) }.buttonStyle(.plain).help(collapsed ? L("展开导航") : L("收起导航")).accessibilityLabel(collapsed ? L("展开导航") : L("收起导航")).accessibilityValue(collapsed ? L("已收起") : L("已展开"))
                    if !collapsed { Text("Aieyes").font(AppFont.title); Spacer() }
                }.padding(.horizontal, 12).padding(.top, 18)
                if collapsed {
                    ScrollView { VStack(spacing:4) {
                        ForEach(monitoring, id:\.0) { key,title,icon in navigationButton(title,icon:icon,selected:model.detailPage == key) { select(key) } }
                        Divider()
                        ForEach(configuration, id:\.0) { key,title,icon in navigationButton(title,icon:icon,selected:model.detailPage == "settings" && model.settingsTab == key) { choose("settings:" + key) } }
                        navigationButton(L("设置"),icon:"gearshape",selected:selectedNavigation == "settings:general") { choose("settings:general") }
                        navigationButton(L("配置向导"),icon:"questionmark.circle",selected:model.detailPage == "setup") { select("setup") }
                    }.padding(.horizontal,8) }
                } else {
                    List(selection:Binding<String?>(get:{selectedNavigation},set:{ if let value = $0 { choose(value) } })) {
                        Section(L("监测")) { ForEach(monitoring,id:\.0) { key,title,icon in Label(title,systemImage:icon).tag(key) } }
                        Section(L("配置")) { ForEach(configuration,id:\.0) { key,title,icon in Label(title,systemImage:icon).tag("settings:" + key) } }
                        Label(L("设置"),systemImage:"gearshape").tag("settings:general")
                        Label(L("配置向导"),systemImage:"questionmark.circle").tag("setup")
                    }.listStyle(.sidebar)
                }
            }.frame(width: collapsed ? 62 : 214).background {
                if reduceTransparency || previewAccessible || contrast == .increased { Color(nsColor: .windowBackgroundColor) }
                else { Rectangle().fill(.ultraThinMaterial) }
            }
            .navigationSplitViewColumnWidth(min:collapsed ? 62 : 190,ideal:collapsed ? 62 : 214,max:collapsed ? 62 : 260)
        } detail: {
            VStack(alignment: .leading, spacing: 0) {
                if guideActive && model.detailPage == "settings" {
                    HStack { Button(L("配置向导")) { select("setup") }; Spacer() }.padding(.horizontal, 22).padding(.bottom, 12)
                }
                RetainedPageHost(selection: model.detailPage, active: model.isWindowVisible("detail")) { key in
                    switch key {
                    case "setup": SetupGuideView(model: model) { guideActive = false; select("agent") }
                    case "settings": SettingsView(model: model)
                    default: RootView(model: model, compact: false, page: key).equatable()
                    }
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            }.frame(minWidth: 0, maxWidth: .infinity, maxHeight: .infinity)
        }.navigationSplitViewStyle(.balanced).toolbar(removing:.sidebarToggle)
        .frame(minWidth: 760, minHeight: 480).aieyesAccent()
        .background(WindowPageTitle(title:title + " — Aieyes"))
        .environment(\.workspaceCardColumns, workspaceWidth >= 1800 ? 4 : workspaceWidth >= 1390 ? 3 : workspaceWidth >= 974 ? 2 : 1)
        .background { GeometryReader { geometry in Color.clear.onChange(of: geometry.size.width, initial: true) { _, width in workspaceWidth = width } } }
        .onAppear { select(model.requestedDetailPage ?? model.detailPage); model.requestedDetailPage = nil }
        .onChange(of: model.requestedDetailPage) { _, page in if let page { select(page); model.requestedDetailPage = nil } }
        .onChange(of: model.detailPage) { _, page in select(page) }
        .onChange(of: model.visibleWindows) { _, _ in model.settingsWindowVisible = model.detailPage == "settings" && model.isWindowVisible("detail") }
    }
    private var selectedNavigation: String { model.detailPage == "settings" ? "settings:" + model.settingsTab : model.detailPage }
    private func choose(_ value:String) {
        if value.hasPrefix("settings:") { model.openSettings(.init(category:String(value.dropFirst(9))));select("settings") }
        else { select(value) }
    }
    @ViewBuilder private func groupLabel(_ text: String) -> some View {
        if collapsed { Divider().padding(.vertical, 6) }
        else { Text(text).font(AppFont.caption).foregroundStyle(.secondary).padding(.horizontal, 10).padding(.top, 10).padding(.bottom, 3) }
    }
    private func navigationButton(_ title: String, icon: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 10) { Image(systemName: icon).frame(width: 22); if !collapsed { Text(title).lineLimit(1); Spacer(minLength: 0) } }
                .font(AppFont.body).padding(.horizontal, 10).padding(.vertical, 6)
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
        if page == "setup" { guideActive = true; if !CommandLine.arguments.contains("--render") { UserDefaults.standard.set(true, forKey: "setup.seen.v1") } }
        model.settingsWindowVisible = page == "settings" && model.isWindowVisible("detail")
        model.setServerVisible(page == "servers", window: "detail")
        if page == "servers" { Task { await model.sampleHosts() } }
    }
}

private struct WindowPageTitle: NSViewRepresentable {
    var title:String
    final class TitleView:NSView {
        var title = ""
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); apply() }
        func apply() { if window?.title != title { window?.title = title } }
    }
    func makeNSView(context:Context) -> TitleView { TitleView() }
    func updateNSView(_ view:TitleView,context:Context) { view.title = title;view.apply() }
}

/// Keep a page's SwiftUI identity and native controls, without laying out hidden pages.
/// The owner retains editors/sheets; only the selected hosting view belongs to the window.
struct RetainedPageHost<Content: View>: NSViewRepresentable {
    let selection: String
    var active = true
    @ViewBuilder var content: (String) -> Content
    func makeNSView(context: Context) -> Container { Container() }
    func updateNSView(_ view: Container, context: Context) {
        view.show(selection, active: active, environment: context.environment, content: content(selection))
    }
    static func dismantleNSView(_ view: Container, coordinator: ()) { view.deactivate() }
    struct Page: View {
        let content: Content, environment: EnvironmentValues
        var active: Bool
        // A hosting view is a new SwiftUI root. Copy only public presentation
        // values: the parent's private render/accessibility context belongs to
        // its original graph and must never be installed in another host.
        private var presentation: some View {
            content.environment(\.workspaceCardColumns, environment.workspaceCardColumns)
                .environment(\.previewAccessibleSurfaces, environment.previewAccessibleSurfaces)
                .environment(\.translucentPanel, environment.translucentPanel)
                .environment(\.surfaceActive, active)
                .environment(\.colorScheme, environment.colorScheme)
            // Read-only accessibility values are supplied by AppKit to each host.
        }
        var body: some View {
            presentation.environment(\.layoutDirection, environment.layoutDirection)
                .environment(\.locale, environment.locale)
                .environment(\.calendar, environment.calendar)
                .environment(\.timeZone, environment.timeZone)
                .environment(\.displayScale, environment.displayScale)
                .font(environment.font).controlSize(environment.controlSize)
                .disabled(!environment.isEnabled)
        }
    }
    @MainActor final class Container: NSView {
        @MainActor final class Entry {
            let host: NSHostingView<Page>
            weak var responder: NSView?
            init(_ content: Content, environment: EnvironmentValues) {
                host = NSHostingView(rootView: Page(content: content, environment: environment, active: false))
                host.sizingOptions = []
                host.autoresizingMask = [.width, .height]
            }
        }
        private(set) var entries: [String: Entry] = [:]
        private(set) var selected: String?
        override var isFlipped: Bool { true }
        func show(_ key: String, active: Bool, environment: EnvironmentValues, content: Content) {
            let changed = selected != key
            if changed, let old = selected.flatMap({ entries[$0] }) {
                // Detachment can defer SwiftUI's representable update. Persist
                // the latest position before taking the old graph off-window.
                ScrollPositionStore.shared.flushAll()
                let responder = (window?.firstResponder as? NSTextView).flatMap { $0.isFieldEditor ? $0.delegate as? NSView : $0 } ?? window?.firstResponder as? NSView
                if let responder, responder === old.host || responder.isDescendant(of: old.host) { old.responder = responder; window?.makeFirstResponder(nil) }
                if old.host.rootView.active { old.host.rootView.active = false }
                old.host.removeFromSuperview()
            }
            let entry: Entry
            if let known = entries[key] {
                entry = known
                entry.host.rootView = Page(content: content, environment: environment, active: active)
            } else { entry = Entry(content, environment: environment); entries[key] = entry }
            selected = key
            if entry.host.rootView.active != active { entry.host.rootView.active = active }
            if entry.host.superview !== self { entry.host.frame = bounds; addSubview(entry.host) }
            if changed, let responder = entry.responder {
                DispatchQueue.main.async { [weak self, weak responder] in
                    guard let self, self.selected == key, let responder, responder.window === self.window else { return }
                    self.window?.makeFirstResponder(responder)
                }
            }
        }
        func deactivate() {
            for entry in entries.values where entry.host.rootView.active { entry.host.rootView.active = false }
            ScrollPositionStore.shared.flushAll()
        }
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
