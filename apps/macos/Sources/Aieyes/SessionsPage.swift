import SwiftUI

/// Detail page for sessions: what is running now and what ended recently.
/// Lifecycle metadata only — prompts, tool arguments and replies are never read or shown.
struct SessionsPage: View {
    let model: AppModel
    @StateObject private var status: SessionStripStore
    @AppStorage("detail.sessions.provider") private var provider = "all"
    @State private var focusMessage: String?
    @State private var focusGeneration = 0
    @State private var focusingID: String?
    @Namespace private var chips
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    @MainActor init(model: AppModel) {
        self.model = model
        _status = StateObject(wrappedValue: SessionStripStore(model: model))
    }

    private var providers: [String] {
        Array(Set(status.snapshot.sessions.filter { $0.phase != .idle }.map { CoreContract.agentProvider($0.provider) })).sorted()
    }
    /// A remembered filter for an agent with no sessions right now falls back to everything.
    private var selection: String { provider == "all" || providers.contains(provider) ? provider : "all" }
    private func matches(_ value: String) -> Bool { selection == "all" || CoreContract.sameAgent(value, selection) }
    private var visible: [LiveSession] { status.snapshot.sessions.filter { $0.phase != .idle && matches($0.provider) } }
    private static func rank(_ phase: SessionPhase) -> Int { phase == .needsInput ? 0 : phase == .unknown ? 2 : 1 }
    /// Waiting sessions lead; the rest keep a stable order within a turn so rows do not jump every second.
    private var running: [LiveSession] {
        visible.filter { $0.phase.active || $0.phase == .unknown }.sorted { a, b in
            let (ra, rb) = (Self.rank(a.phase), Self.rank(b.phase))
            if ra != rb { return ra < rb }
            let (sa, sb) = (a.turnStartedAt ?? 0, b.turnStartedAt ?? 0)
            return sa != sb ? sa > sb : a.id < b.id
        }
    }
    private var ended: [LiveSession] {
        visible.filter { !$0.phase.active && $0.phase != .unknown }.sorted { a, b in
            a.updatedAt != b.updatedAt ? a.updatedAt > b.updatedAt : a.id < b.id
        }
    }
    private var failures: [SessionSourceHealth] { status.snapshot.sources.filter { $0.state != "healthy" && matches($0.provider) } }
    private var processSources: [SessionSourceHealth] {
        var seen = Set<String>()
        return status.snapshot.sources.filter { $0.processRunning != nil && matches($0.provider) && seen.insert($0.machineId + ":" + $0.provider).inserted }
    }
    private func machineName(_ id: String) -> String {
        id == "local" ? L("本机") : model.settings.hosts.first(where: { $0.id == id })?.name ?? id
    }

    var body: some View {
        let _ = interfaceLanguage;
        ScrollView {
            VStack(alignment: .leading, spacing: DesignTokens.spaceSection) {
                if providers.count > 1 {
                    EdgeFadingScroller(selection: selection) {
                        HStack(spacing: 4) {
                            chip("all", title: L("全部"))
                            ForEach(providers, id: \.self) { chip($0, title: Format.provider($0)) }
                        }.motion(.spring, value: selection)
                    }
                }
                if status.snapshot.unavailable {
                    Surface { Label(L("会话读取失败"), systemImage: "exclamationmark.triangle").foregroundStyle(Palette.warn) }
                } else if visible.isEmpty && failures.isEmpty && processSources.isEmpty {
                    Surface {
                        EmptyState(icon: "terminal", title: L("暂无会话动态"), message: L("在终端或 IDE 中启动 Agent 后，会话会显示在这里"),
                                   actionTitle: model.settings.sources.isEmpty ? L("接入账户") : nil,
                                   action: { model.openSettings(.init(category: "integrations")) })
                    }
                } else {
                    if !visible.isEmpty { summary }
                    list(L("进行中"), running, empty: L("当前没有进行中的会话"), live: true)
                    list(L("最近结束"), ended, empty: L("暂无最近结束的会话"), live: false)
                }
                if !processSources.isEmpty {
                    Surface(title: L("IDE 运行状态")) {
                        ForEach(processSources) { source in
                            HStack(spacing: 10) {
                                Image(systemName: "desktopcomputer").foregroundStyle(.secondary).frame(width: 22)
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(Format.provider(source.provider) + " · " + machineName(source.machineId))
                                    Text(L("对话存储已加密，会话数量未知")).font(AppFont.secondary).foregroundStyle(.secondary)
                                }
                                Spacer()
                                Text(source.processRunning == true ? L("IDE 正在运行") : L("IDE 未运行")).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                if !failures.isEmpty {
                    Surface(title: L("{0} 个来源异常", failures.count)) {
                        ForEach(failures) { source in
                            Label(Format.provider(source.provider) + " · " + machineName(source.machineId) + ": " + L(source.message), systemImage: "exclamationmark.triangle")
                                .font(AppFont.secondary).foregroundStyle(Palette.warn).frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled)
                        }
                    }
                }
            }
            .padding(DesignTokens.contentPadding).background(OverlayScrollStyle()).background(ScrollMemory(key: "detail.sessions"))
        }
        .accessibilityIdentifier("sessions-overview")
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background { ZStack { Rectangle().fill(.ultraThinMaterial); Color(nsColor: .windowBackgroundColor).opacity(0.5) } }
        .font(AppFont.body).aieyesAccent()
    }

    private func chip(_ key: String, title: String) -> some View {
        Button(title) { provider = key }
            .buttonStyle(.chip(selected: selection == key, tint: key == "all" ? Palette.accent : Palette.category(key), in: chips)).id(key)
    }

    private var summary: some View {
        HStack(spacing: 12) {
            StatCard(title: L("进行中"), value: String(running.filter { $0.phase.active && $0.phase != .needsInput }.count), detail: "", icon: SessionPhase.working.symbol, accent: Palette.accent)
            StatCard(title: L("等待输入"), value: String(running.filter { $0.phase == .needsInput }.count), detail: "", icon: SessionPhase.needsInput.symbol, accent: Palette.warn)
            StatCard(title: L("最近结束"), value: String(ended.count), detail: "", icon: SessionPhase.complete.symbol, accent: Palette.ok)
        }.fixedSize(horizontal: false, vertical: true)
    }

    private func list(_ title: String, _ sessions: [LiveSession], empty: String, live: Bool) -> some View {
        Surface(title: title, spacing: 0) {
            if sessions.isEmpty { Text(empty).font(AppFont.secondary).foregroundStyle(.secondary).padding(.top, DesignTokens.spaceCardContent) }
            ForEach(Array(sessions.enumerated()), id: \.element.id) { index, session in
                if index > 0 { Divider().opacity(0.5) }
                row(session)
            }
            if live, let focusMessage {
                Text(focusMessage).font(AppFont.secondary).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(.top, 8).transition(.opacity)
            }
        }
    }

    private func row(_ session: LiveSession) -> some View {
        HStack(alignment: .center, spacing: 12) {
            ActivityIndicator(phase: session.phase)
            VStack(alignment: .leading, spacing: 3) {
                Text(session.project ?? Format.provider(session.provider)).font(AppFont.section).lineLimit(1).truncationMode(.middle)
                    .help(session.workspace ?? "")
                Text([Format.provider(session.provider), session.machine, session.shortIdentity.map { "#" + $0 }].compactMap { $0 }.joined(separator: " · ")).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1).help(session.nativeSession ?? "")
                SessionUsageLine(session: session)
            }
            Spacer(minLength: 12)
            VStack(alignment: .trailing, spacing: 3) {
                Text(session.phase.label).foregroundStyle(.primary)
                SurfaceTimeline(interval: session.turnEndedAt == nil && session.phase.active ? 1 : 60) { context in
                    Text(timing(session, now: context.date)).font(AppFont.secondary).foregroundStyle(.secondary).monospacedDigit().lineLimit(1)
                }
            }
            // SSH sessions are information only: no local window owns them.
            if session.focusable {
                Button { focus(session) } label: { Image(systemName: "arrow.up.forward.app") }
                    .buttonStyle(.borderless).help(session.focusHelp).accessibilityLabel(session.focusHelp)
                    .disabled(focusingID == session.id)
            }
        }
        .padding(.vertical, 10).contentShape(Rectangle())
    }

    /// "Turn 3m12s", or the last update time when no turn timing is known.
    private func timing(_ session: LiveSession, now: Date) -> String {
        let updated = session.updatedAt.formatted(Date.FormatStyle(date: .omitted, time: .shortened).locale(AieyesLocale.locale))
        guard let elapsed = session.turnElapsed(at: now) else { return L("最后更新 ") + updated }
        let turn = L("本轮 {0}", LiveSession.elapsedText(elapsed))
        return session.phase.active ? turn : turn + " · " + updated
    }

    private func focus(_ session: LiveSession) {
        guard focusingID == nil else { return }
        focusingID = session.id
        focusGeneration += 1
        let generation = focusGeneration
        Task { @MainActor in
            let outcome = await SessionFocus.focus(session, engine: SessionFocus.engine)
            focusingID = nil
            guard generation == focusGeneration else { return }
            withMotion(.expand) { focusMessage = SessionFocus.message(outcome) }
            try? await Task.sleep(for: .seconds(5))
            if generation == focusGeneration { withMotion(.expand) { focusMessage = nil } }
        }
    }
}
