import AppKit
import Combine
import QuartzCore
import SwiftUI

struct PetOrbitQuota: Equatable {
    var accountName: String
    var provider: String
    var kind: QuotaRingKind
    var summary: String
    var refreshedAt: Double?
}

struct PetOrbitAgent: Identifiable, Equatable {
    var provider: String
    var displayProvider: String
    var phase: SessionPhase
    var activeCount: Int
    var overflowProviders: [String] = []
    var quota: PetOrbitQuota?
    var id: String { provider }
    var isOverflow: Bool { provider == PetOrbitLayout.overflowProvider }
    /// Spoken summary of the badge: agent, whether it waits for input, and its session count.
    var accessibilitySummary: String {
        [title, phase == .needsInput ? L("等待输入") : nil, L("{0} 个进行中", activeCount)]
            .compactMap { $0 }.joined(separator: " · ")
    }
    var title: String {
        isOverflow ? L("其他运行中 Agent") : Format.provider(displayProvider)
    }
    var tooltip: String {
        if isOverflow {
            return [
                L("还有 {0} 个运行中 Agent", overflowProviders.count),
                L("{0} 个进行中", activeCount),
                L("单击查看会话"),
            ].joined(separator: " · ")
        }
        return [
            title,
            L("{0} 个进行中", activeCount),
            quota?.summary,
            L("单击查看会话"),
        ].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

struct PetOrbitSlot: Equatable {
    /// Center coordinates in top-left origin logical space (SwiftUI coordinate space).
    var cx: CGFloat
    var cy: CGFloat
    /// Standard screen-space angle in degrees (0 = right, -90 = top, -180 = left).
    var angleDeg: Double
}

enum PetOrbitLayout {
    static let baseWidth: CGFloat = 144
    static let padX: CGFloat = 44
    static let padTop: CGFloat = 44
    static let nodeSize: CGFloat = 32
    static let hitRadius: CGFloat = 16.5
    static let maxSlots = 7
    static let overflowProvider = "__overflow__"

    static func padding(hasOrbit: Bool, scale: Double) -> (x: CGFloat, top: CGFloat) {
        hasOrbit ? (padX * scale, padTop * scale) : (0, 0)
    }

    static func slots(count: Int, scale: Double = 1) -> [PetOrbitSlot] {
        guard count > 0 else { return [] }
        let angles: [Double] = switch count {
        case 1: [-90]
        case 2: [-132, -48]
        case 3: [-155, -90, -25]
        case 4: [-168, -116, -64, -12]
        case 5: [-176, -133, -90, -47, -4]
        case 6: [-180, -144, -108, -72, -36, 0]
        default: [-184, -153, -121, -90, -59, -27, 4]
        }
        let cx0 = (padX + 72) * scale
        let cy0 = (padTop + 78) * scale
        let rx = 84 * scale
        let ry = 76 * scale
        return angles.prefix(min(count, maxSlots)).map { angle in
            let rad = angle * .pi / 180
            return PetOrbitSlot(cx: cx0 + rx * cos(rad), cy: cy0 + ry * sin(rad), angleDeg: angle)
        }
    }

    static func preferredEdge(for slot: PetOrbitSlot, flipped: Bool = true) -> NSRectEdge {
        if slot.angleDeg <= -125 { return .minX }
        if slot.angleDeg >= -55 { return .maxX }
        return flipped ? .minY : .maxY
    }

    static func formatQuotaSummary(_ kind: QuotaRingKind, now: Date = Date()) -> String {
        func arcText(_ arc: QuotaRingArc, includeWindow: Bool) -> String {
            let prefix: String
            if includeWindow {
                if arc.windowMinutes == CoreContract.fiveHourMinutes { prefix = "5h " }
                else if arc.windowMinutes == CoreContract.weekMinutes { prefix = "7d " }
                else { prefix = "" }
            } else {
                prefix = ""
            }
            let remaining = QuotaRingModel.remainingText(arc)
            let reset = QuotaRingModel.resetText(arc, now: now)
            if reset == "—" || reset == "5h" || reset == "7d" {
                return prefix + remaining
            }
            return "\(prefix)\(remaining) (\(reset))"
        }
        switch kind {
        case .duo(let top, let bottom):
            return arcText(top, includeWindow: true) + " · " + arcText(bottom, includeWindow: true)
        case .single(let top):
            return arcText(top, includeWindow: false)
        case .coin(let amount, _):
            return amount
        case .empty:
            return ""
        }
    }
}

@MainActor final class PetOrbitStore: ObservableObject {
    @Published private(set) var agents: [PetOrbitAgent] = []
    @Published var selectedProvider: String? = nil
    @Published var hoveredProvider: String? = nil
    /// Opens an agent's session popover without the mouse; set by the pet window for accessibility actions.
    var openAgent: ((PetOrbitAgent) -> Void)?
    private var subscriptions = Set<AnyCancellable>()
    private var pending = false
    private weak var model: AppModel?

    init(model: AppModel) {
        self.model = model
        model.live.$sessions.sink { [weak self] _ in self?.schedule() }.store(in: &subscriptions)
        model.$menuDashboard.sink { [weak self] _ in self?.schedule() }.store(in: &subscriptions)
        model.$settings.sink { [weak self] _ in self?.schedule() }.store(in: &subscriptions)
        Timer.publish(every: 30, tolerance: 2, on: .main, in: .common).autoconnect()
            .sink { [weak self] _ in self?.schedule() }.store(in: &subscriptions)
        refresh()
    }

    private func schedule() {
        guard !pending else { return }
        pending = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.pending = false
            self.refresh()
        }
    }

    func refresh(now: Date = Date()) {
        guard let model else { return }
        let next = Self.project(
            sessions: model.live.sessions,
            settings: model.settings,
            quotas: model.menuDashboard.quotas,
            now: now
        )
        if agents != next { agents = next }
        if let hovered = hoveredProvider, !next.contains(where: { $0.provider == hovered }) {
            hoveredProvider = nil
        }
    }

    private static func phasePriority(_ phase: SessionPhase) -> Int {
        switch phase {
        case .needsInput: 0
        case .failed: 1
        case .working, .thinking, .tool: 2
        case .complete, .interrupted: 3
        case .unknown, .idle: 4
        }
    }

    private static func providerOrder(_ agentId: String) -> Int {
        CoreContract.agentProviders.firstIndex(of: agentId) ?? Int.max
    }

    static func project(
        sessions: [LiveSession],
        settings: Settings,
        quotas: [Quota],
        now: Date = Date()
    ) -> [PetOrbitAgent] {
        struct Bucket {
            var agentId: String
            var displayProvider: String
            var allSameRaw = true
            var phase: SessionPhase
            var activeCount = 0
            var sourceIds: [String] = []
        }
        var grouped: [String: Bucket] = [:]
        for session in sessions where session.phase.active {
            let agentId = CoreContract.agentProvider(session.provider)
            if var bucket = grouped[agentId] {
                bucket.activeCount += 1
                if phasePriority(session.phase) < phasePriority(bucket.phase) {
                    bucket.phase = session.phase
                }
                if bucket.displayProvider != session.provider { bucket.allSameRaw = false }
                if !session.sourceId.isEmpty && !bucket.sourceIds.contains(session.sourceId) {
                    bucket.sourceIds.append(session.sourceId)
                }
                grouped[agentId] = bucket
            } else {
                grouped[agentId] = Bucket(
                    agentId: agentId,
                    displayProvider: session.provider,
                    allSameRaw: true,
                    phase: session.phase,
                    activeCount: 1,
                    sourceIds: session.sourceId.isEmpty ? [] : [session.sourceId]
                )
            }
        }
        var items = grouped.values.map { bucket -> PetOrbitAgent in
            let displayProvider = bucket.allSameRaw ? bucket.displayProvider : bucket.agentId
            let quota = matchQuota(agentId: bucket.agentId, sourceIds: bucket.sourceIds, settings: settings, quotas: quotas, now: now)
            return PetOrbitAgent(
                provider: bucket.agentId,
                displayProvider: displayProvider,
                phase: bucket.phase,
                activeCount: bucket.activeCount,
                overflowProviders: [],
                quota: quota
            )
        }
        items.sort { a, b in
            let (oa, ob) = (providerOrder(a.provider), providerOrder(b.provider))
            return oa != ob ? oa < ob : a.provider < b.provider
        }
        if items.count > PetOrbitLayout.maxSlots {
            let head = Array(items.prefix(PetOrbitLayout.maxSlots - 1))
            let tail = Array(items.dropFirst(PetOrbitLayout.maxSlots - 1))
            var phase = tail[0].phase
            var count = 0
            var providers: [String] = []
            for item in tail {
                if phasePriority(item.phase) < phasePriority(phase) { phase = item.phase }
                count += item.activeCount
                providers.append(item.provider)
            }
            return head + [PetOrbitAgent(
                provider: PetOrbitLayout.overflowProvider,
                displayProvider: PetOrbitLayout.overflowProvider,
                phase: phase,
                activeCount: count,
                overflowProviders: providers,
                quota: nil
            )]
        }
        return items
    }

    private static func matchQuota(
        agentId: String,
        sourceIds: [String],
        settings: Settings,
        quotas: [Quota],
        now: Date
    ) -> PetOrbitQuota? {
        var accountIds: [String] = []
        for sourceId in sourceIds {
            if let source = settings.sources.first(where: { $0.id == sourceId }),
               !source.accountId.isEmpty, !accountIds.contains(source.accountId) {
                accountIds.append(source.accountId)
            }
        }
        func build(_ quota: Quota) -> PetOrbitQuota? {
            guard let item = QuotaRingModel.items(quota).first, item.kind != .empty else { return nil }
            return PetOrbitQuota(
                accountName: quota.name,
                provider: quota.provider,
                kind: item.kind,
                summary: PetOrbitLayout.formatQuotaSummary(item.kind, now: now),
                refreshedAt: quota.snapshotAge
            )
        }
        for accountId in accountIds {
            if let quota = quotas.first(where: { CoreContract.sameAgent($0.provider, agentId) && $0.accountId == accountId }),
               let built = build(quota) {
                return built
            }
        }
        return quotas
            .filter { CoreContract.sameAgent($0.provider, agentId) && !(settings.hideFreeQuotaCards == true && $0.freePlan == true) }
            .compactMap(build)
            .first
    }

    static func activeSessions(for agent: PetOrbitAgent, in sessions: [LiveSession]) -> [LiveSession] {
        let matches: [LiveSession] = sessions.filter { session in
            guard session.phase.active else { return false }
            if agent.isOverflow {
                return agent.overflowProviders.contains(CoreContract.agentProvider(session.provider))
            }
            return CoreContract.sameAgent(session.provider, agent.provider)
        }
        return matches.sorted { a, b in
            let (ra, rb) = (a.phase == .needsInput ? 0 : 1, b.phase == .needsInput ? 0 : 1)
            if ra != rb { return ra < rb }
            let (sa, sb) = (a.turnStartedAt ?? 0, b.turnStartedAt ?? 0)
            return sa != sb ? sa > sb : a.id < b.id
        }
    }
}

/// GPU-composited breathing glow behind each orbiting agent badge. Pauses when hidden or under Reduce Motion.
final class PetOrbitPulseLayerView: NSView {
    private let haloLayer = CAShapeLayer()
    private var tintColor: NSColor = .controlAccentColor
    private var duration: Double = 2.2
    private var staggerOffset: Double = 0
    private var requested = false
    private var reduced = false
    private var visibility: AnyCancellable?
    private(set) var animationStarts = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.masksToBounds = false
        haloLayer.fillColor = nil
        haloLayer.lineCap = .round
        layer?.addSublayer(haloLayer)
    }
    convenience init() { self.init(frame: .zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        haloLayer.frame = bounds
        let inset = max(2, min(bounds.width, bounds.height) * 0.16)
        haloLayer.path = CGPath(ellipseIn: bounds.insetBy(dx: inset, dy: inset), transform: nil)
        haloLayer.lineWidth = max(2.0, min(bounds.width, bounds.height) * 0.1)
        CATransaction.commit()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        visibility = nil
        if let window {
            visibility = NotificationCenter.default.publisher(for: NSWindow.didChangeOcclusionStateNotification, object: window)
                .sink { [weak self] _ in self?.sync() }
        }
        sync()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        applyColors()
    }

    func configure(color: Color, needsInput: Bool, index: Int, playing: Bool, reduced: Bool) {
        let nextColor = NSColor(color)
        let nextDuration = needsInput ? 1.5 : 2.2
        let nextOffset = Double(index) * 0.35
        let timingChanged = duration != nextDuration || staggerOffset != nextOffset
        tintColor = nextColor
        duration = nextDuration
        staggerOffset = nextOffset
        requested = playing
        self.reduced = reduced
        applyColors()
        if timingChanged { stop() }
        sync()
    }

    private func applyColors() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            let cg = tintColor.cgColor
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            haloLayer.strokeColor = cg
            haloLayer.shadowColor = cg
            haloLayer.shadowOpacity = 0.65
            haloLayer.shadowRadius = 4
            haloLayer.shadowOffset = .zero
            CATransaction.commit()
        }
    }

    private func sync() {
        let visible = window?.isVisible == true && window?.occlusionState.contains(.visible) == true
        guard requested && !reduced && visible else {
            stop()
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            haloLayer.opacity = requested ? 0.3 : 0
            CATransaction.commit()
            return
        }
        guard haloLayer.animation(forKey: "orbit.pulse") == nil else { return }
        let opacity = CABasicAnimation(keyPath: "opacity")
        opacity.fromValue = 0.16
        opacity.toValue = 0.54

        let scale = CABasicAnimation(keyPath: "transform.scale")
        scale.fromValue = 0.98
        scale.toValue = 1.16

        let group = CAAnimationGroup()
        group.animations = [opacity, scale]
        group.duration = duration / 2
        group.autoreverses = true
        group.repeatCount = .infinity
        group.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        group.isRemovedOnCompletion = false
        group.beginTime = haloLayer.convertTime(CACurrentMediaTime(), from: nil) - staggerOffset
        haloLayer.add(group, forKey: "orbit.pulse")
        animationStarts += 1
    }

    func stop() {
        haloLayer.removeAnimation(forKey: "orbit.pulse")
    }
}

private struct PetOrbitPulseView: NSViewRepresentable {
    var color: Color
    var needsInput: Bool
    var index: Int
    var playing: Bool
    var reduced: Bool
    func makeNSView(context: Context) -> PetOrbitPulseLayerView { PetOrbitPulseLayerView() }
    func updateNSView(_ view: PetOrbitPulseLayerView, context: Context) {
        view.configure(color: color, needsInput: needsInput, index: index, playing: playing, reduced: reduced)
    }
    static func dismantleNSView(_ view: PetOrbitPulseLayerView, coordinator: ()) { view.stop() }
}

struct PetOrbitNodeView: View {
    let agent: PetOrbitAgent
    let index: Int
    let scale: Double
    let playing: Bool
    let selected: Bool
    let hovered: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var colorScheme

    private var pulseColor: Color {
        if agent.phase == .needsInput { return Palette.warn }
        if agent.isOverflow { return Palette.accent }
        return QuotaRingModel.healthy(agent.displayProvider)
    }

    private var ringKind: QuotaRingKind {
        agent.quota?.kind ?? .empty
    }

    var body: some View {
        let size = PetOrbitLayout.nodeSize * scale
        let haloSize = size + 12 * scale
        ZStack {
            PetOrbitPulseView(
                color: pulseColor,
                needsInput: agent.phase == .needsInput,
                index: index,
                playing: playing,
                reduced: reduceMotion
            )
            .frame(width: haloSize, height: haloSize)

            ZStack {
                Circle()
                    .fill(.regularMaterial)
                Circle()
                    .fill(colorScheme == .dark ? Color.black.opacity(0.24) : Color.white.opacity(0.55))
                Circle()
                    .strokeBorder(
                        selected ? Palette.accent : Palette.cardBorder.opacity(hovered ? 0.65 : 0.36),
                        lineWidth: (selected ? 1.6 : 0.9) * scale
                    )
                if !agent.isOverflow {
                    QuotaRingView(
                        kind: ringKind,
                        healthy: QuotaRingModel.healthy(agent.displayProvider),
                        lineWidth: max(1.8, 2.5 * scale),
                        gap: 1.4 * scale,
                        track: Color.primary.opacity(0.18),
                        refreshedAt: agent.quota?.refreshedAt,
                        animated: !reduceMotion
                    )
                    .padding(1.2 * scale)
                    ProviderMark(provider: agent.displayProvider, size: 16 * scale)
                } else {
                    Text("+\(agent.overflowProviders.count)")
                        .font(.system(size: max(9, 11 * scale), weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(.primary)
                }
            }
            .frame(width: size, height: size)
            .shadow(color: Color.black.opacity(colorScheme == .dark ? 0.28 : 0.14), radius: 3 * scale, y: 1.5 * scale)
            .overlay(alignment: .bottomTrailing) {
                if !agent.isOverflow && agent.activeCount > 1 {
                    let countText = agent.activeCount > 9 ? "9+" : "\(agent.activeCount)"
                    Text(countText)
                        .font(.system(size: max(8, 9 * scale), weight: .bold))
                        .monospacedDigit()
                        .foregroundStyle(.white)
                        .padding(.horizontal, 3.5 * scale)
                        .frame(minWidth: 13 * scale, minHeight: 13 * scale)
                        .background(agent.phase == .needsInput ? Palette.warn : Palette.accent, in: Capsule())
                        .overlay(Capsule().strokeBorder(colorScheme == .dark ? Color.black.opacity(0.45) : Color.white.opacity(0.85), lineWidth: 1))
                        .offset(x: 3 * scale, y: 3 * scale)
                }
            }
            .scaleEffect(hovered || selected ? 1.08 : 1.0)
            .animation(reduceMotion ? nil : .easeOut(duration: 0.14), value: hovered || selected)
        }
        .frame(width: haloSize, height: haloSize)
        .allowsHitTesting(false)
    }
}

struct PetAgentSessionPopoverView: View {
    let model: AppModel
    let initialAgent: PetOrbitAgent
    let autoOpened: Bool
    @ObservedObject var live: LiveSessionState
    @ObservedObject var orbit: PetOrbitStore
    @ObservedObject var interaction: SessionInteractionStore
    var onOpenSessions: () -> Void
    var onClose: () -> Void

    @State private var selectedWaitingID: String?
    @State private var focusMessage: String?
    @State private var focusGeneration = 0
    @State private var focusingID: String?
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"

    init(
        model: AppModel,
        agent: PetOrbitAgent,
        orbit: PetOrbitStore,
        autoOpened: Bool = false,
        preferredSessionID: String? = nil,
        onOpenSessions: @escaping () -> Void,
        onClose: @escaping () -> Void
    ) {
        self.model = model
        self.initialAgent = agent
        self.autoOpened = autoOpened
        self.live = model.live
        self.orbit = orbit
        self.interaction = model.interaction
        self._selectedWaitingID = State(initialValue: preferredSessionID)
        self.onOpenSessions = onOpenSessions
        self.onClose = onClose
    }

    private var currentAgent: PetOrbitAgent {
        orbit.agents.first(where: { $0.provider == initialAgent.provider }) ?? initialAgent
    }

    private var activeSessions: [LiveSession] {
        PetOrbitStore.activeSessions(for: currentAgent, in: live.sessions)
    }

    private var waitingSessions: [LiveSession] {
        activeSessions.filter { $0.phase == .needsInput }
    }

    private var otherSessions: [LiveSession] {
        activeSessions.filter { $0.phase != .needsInput }
    }

    private var activeWaitingSession: LiveSession? {
        if let id = selectedWaitingID, let hit = waitingSessions.first(where: { $0.id == id }) {
            return hit
        }
        return waitingSessions.first
    }

    /// The session the user chose to send an instruction to. It is looked up among all sessions so
    /// the card and its draft survive the turn ending; once it asks for input it becomes a waiting card.
    private var followUpSession: LiveSession? {
        guard interaction.status.enabled, let id = interaction.followUpSessionID,
              let hit = live.sessions.first(where: { $0.id == id }), hit.phase != .needsInput,
              currentAgent.isOverflow
                ? currentAgent.overflowProviders.contains(CoreContract.agentProvider(hit.provider))
                : CoreContract.sameAgent(hit.provider, currentAgent.provider) else { return nil }
        return hit
    }

    static func offersFollowUp(enabled: Bool, session: LiveSession, agentWaiting: Bool) -> Bool {
        enabled && session.phase != .needsInput && !agentWaiting
    }

    private var showInteractionCard: Bool {
        interaction.status.enabled && (activeWaitingSession != nil || followUpSession != nil)
    }

    var body: some View {
        let _ = interfaceLanguage
        let agent = currentAgent
        let sessions = activeSessions
        let waiting = waitingSessions
        let others = otherSessions
        VStack(alignment: .leading, spacing: 10) {
            header(agent: agent, activeCount: sessions.count)
            Divider().opacity(0.5)
            if sessions.isEmpty && followUpSession == nil {
                VStack(alignment: .leading, spacing: 4) {
                    Text(L("当前会话已结束"))
                        .font(AppFont.body)
                        .foregroundStyle(.secondary)
                    Text(L("可打开会话页查看最近结束记录"))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 8)
            } else if activeWaitingSession == nil, let target = followUpSession {
                ScrollView {
                    VStack(alignment: .leading, spacing: 10) {
                        PetInteractionCardView(
                            session: target,
                            store: interaction,
                            autoFocus: true,
                            onFocusSession: { focus(target) },
                            onOpenSessions: onOpenSessions,
                            onCompleted: {}
                        )
                        HStack {
                            Spacer()
                            Button(L("返回会话列表")) { interaction.followUpSessionID = nil }
                                .controlSize(.small)
                        }
                        let rest = sessions.filter { $0.id != target.id }
                        if !rest.isEmpty {
                            Divider().opacity(0.4)
                            VStack(alignment: .leading, spacing: 6) {
                                ForEach(Array(rest.enumerated()), id: \.element.id) { index, session in
                                    if index > 0 { Divider().opacity(0.3) }
                                    sessionRow(session)
                                }
                            }
                        }
                    }
                    .padding(.vertical, 2)
                }
                .frame(maxHeight: 420)
            } else if showInteractionCard, let target = activeWaitingSession {
                if waiting.count > 1 {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(waiting) { item in
                                let picked = item.id == target.id
                                let name = Self.chipTitle(for: item, among: waiting)
                                Button {
                                    selectedWaitingID = item.id
                                } label: {
                                    HStack(spacing: 4) {
                                        Circle()
                                            .fill(Palette.warn)
                                            .frame(width: 6, height: 6)
                                            .accessibilityHidden(true)
                                        Text(name)
                                            .font(AppFont.caption)
                                            .lineLimit(1)
                                    }
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 4)
                                    .background(
                                        picked ? Palette.warn.opacity(0.18) : Color.primary.opacity(0.05),
                                        in: Capsule()
                                    )
                                    .overlay(
                                        Capsule().strokeBorder(picked ? Palette.warn.opacity(0.6) : Color.clear, lineWidth: 1)
                                    )
                                }
                                .buttonStyle(.plain)
                                .accessibilityLabel(L("{0} 等待输入", name))
                                .accessibilityAddTraits(picked ? .isSelected : [])
                            }
                        }
                    }
                }
                ScrollView {
                    VStack(alignment: .leading, spacing: 10) {
                        PetInteractionCardView(
                            session: target,
                            store: interaction,
                            autoFocus: !autoOpened,
                            onFocusSession: { focus(target) },
                            onOpenSessions: onOpenSessions,
                            onCompleted: {
                                // Sessions already answered from here wait on the agent, not on the user.
                                let remaining = waitingSessions.filter { item in
                                    item.id != target.id && !(interaction.payloads[item.id].map(interaction.isCompleted) ?? false)
                                }
                                if let next = remaining.first {
                                    selectedWaitingID = next.id
                                } else if autoOpened {
                                    onClose()
                                }
                            }
                        )
                        if !others.isEmpty {
                            Divider().opacity(0.4)
                            Text(L("其他进行中会话"))
                                .font(AppFont.caption)
                                .foregroundStyle(.secondary)
                            VStack(alignment: .leading, spacing: 6) {
                                ForEach(Array(others.enumerated()), id: \.element.id) { index, session in
                                    if index > 0 { Divider().opacity(0.3) }
                                    sessionRow(session)
                                }
                            }
                        }
                    }
                    .padding(.vertical, 2)
                }
                .frame(maxHeight: 420)
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(Array(sessions.enumerated()), id: \.element.id) { index, session in
                            if index > 0 { Divider().opacity(0.35) }
                            sessionRow(session)
                        }
                    }
                    .padding(.vertical, 2)
                }
                .frame(maxHeight: 230)
            }
            if let focusMessage {
                Text(focusMessage)
                    .font(AppFont.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .transition(.opacity)
            }
            Divider().opacity(0.5)
            HStack(spacing: 8) {
                Text(sessions.isEmpty ? L("空闲") : L("{0} 个进行中", sessions.count))
                    .font(AppFont.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button(L("全部会话")) {
                    onOpenSessions()
                }
                .controlSize(.small)
                Button(L("关闭")) {
                    interaction.markDismissed(sessions: waiting)
                    onClose()
                }
                .controlSize(.small)
            }
        }
        .padding(12)
        .frame(width: showInteractionCard ? 364 : 316)
        .font(AppFont.body)
        .aieyesAccent()
        // A follow-up picked before this agent started asking must not come back, long after the
        // click, once the waiting card is answered.
        .onChange(of: activeWaitingSession?.id) { _, waitingID in
            if waitingID != nil && interaction.followUpSessionID != nil { interaction.followUpSessionID = nil }
        }
    }

    /// Chip title for a waiting session; sessions that share a project name get their short identity.
    static func chipTitle(for session: LiveSession, among sessions: [LiveSession]) -> String {
        let name = session.project ?? Format.provider(session.provider)
        let shared = sessions.filter { ($0.project ?? Format.provider($0.provider)) == name }.count > 1
        guard shared, let identity = session.shortIdentity else { return name }
        return name + " #" + identity
    }

    private func header(agent: PetOrbitAgent, activeCount: Int) -> some View {
        HStack(alignment: .center, spacing: 10) {
            ZStack {
                if !agent.isOverflow {
                    QuotaRingView(
                        kind: agent.quota?.kind ?? .empty,
                        healthy: QuotaRingModel.healthy(agent.displayProvider),
                        lineWidth: 2.5,
                        gap: 1.5,
                        track: Color.primary.opacity(0.18),
                        refreshedAt: agent.quota?.refreshedAt
                    )
                    ProviderMark(provider: agent.displayProvider, size: 17)
                } else {
                    Circle().strokeBorder(Palette.accent.opacity(0.45), lineWidth: 2)
                    Text("+\(agent.overflowProviders.count)")
                        .font(.system(size: 11, weight: .semibold))
                        .monospacedDigit()
                }
            }
            .frame(width: 32, height: 32)

            VStack(alignment: .leading, spacing: 2) {
                Text(agent.title)
                    .font(AppFont.section)
                    .lineLimit(1)
                if let account = agent.quota?.accountName, !account.isEmpty {
                    Text(account)
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                } else if !agent.isOverflow {
                    Text(L("进行中会话"))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 2) {
                if let quota = agent.quota, !quota.summary.isEmpty {
                    quotaBadge(quota)
                } else if !agent.isOverflow {
                    Text(L("暂无额度"))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Text(L("{0} 个进行中", activeCount))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    @ViewBuilder
    private func quotaBadge(_ quota: PetOrbitQuota) -> some View {
        SurfaceTimeline(interval: 30) { context in
            let now = context.date
            switch quota.kind {
            case .duo(let top, let bottom):
                VStack(alignment: .trailing, spacing: 1) {
                    Text("\(top.label) \(QuotaRingModel.remainingText(top)) · \(QuotaRingModel.resetText(top, now: now))")
                        .font(AppFont.caption)
                        .monospacedDigit()
                        .foregroundStyle(Palette.quota(top.used))
                    Text("\(bottom.label) \(QuotaRingModel.remainingText(bottom))")
                        .font(.system(size: 11))
                        .monospacedDigit()
                        .foregroundStyle(Palette.quota(bottom.used))
                }
            case .single(let top):
                Text("\(QuotaRingModel.remainingText(top)) · \(QuotaRingModel.resetText(top, now: now))")
                    .font(AppFont.caption)
                    .monospacedDigit()
                    .foregroundStyle(Palette.quota(top.used))
            case .coin(let amount, let low):
                Text(amount)
                    .font(AppFont.caption)
                    .monospacedDigit()
                    .foregroundStyle(low ? Palette.danger : .primary)
            case .empty:
                Text(L("暂无额度"))
                    .font(AppFont.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func sessionRow(_ session: LiveSession) -> some View {
        let content = HStack(alignment: .top, spacing: 9) {
            ActivityIndicator(phase: session.phase)
                .padding(.top, 2)
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(session.project ?? Format.provider(session.provider))
                        .font(.system(size: DesignTokens.fontEntry, weight: .medium))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .help(session.workspace ?? "")
                    Spacer(minLength: 4)
                    Text(session.phase.label)
                        .font(AppFont.caption)
                        .foregroundStyle(session.phase.color)
                }
                HStack(spacing: 6) {
                    Text([
                        currentAgent.isOverflow ? Format.provider(session.provider) : nil,
                        session.machine,
                        session.shortIdentity.map { "#" + $0 },
                    ].compactMap { $0 }.joined(separator: " · "))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    SurfaceTimeline(interval: session.turnEndedAt == nil && session.phase.active ? 1 : 60) { context in
                        Text(timing(session, now: context.date))
                            .font(AppFont.caption)
                            .foregroundStyle(.secondary)
                            .monospacedDigit()
                            .lineLimit(1)
                    }
                }
                SessionUsageLine(session: session)
            }
            if session.focusable {
                Image(systemName: "arrow.up.forward.app")
                    .font(AppFont.caption)
                    .foregroundStyle(.secondary)
                    .padding(.top, 3)
            }
        }
        .padding(.vertical, 4)
        .contentShape(Rectangle())

        HStack(alignment: .top, spacing: 6) {
            if session.focusable {
                Button { focus(session) } label: { content }
                    .buttonStyle(.plain)
                    .help(session.focusHelp)
                    .accessibilityHint(session.focusHelp)
                    .disabled(focusingID == session.id)
            } else {
                content
            }
            // Whether an instruction can actually be sent is core's call; the card says so either way.
            // While this agent has a request waiting, its card owns the popover and the follow-up
            // card could not be shown, so the button is not offered.
            if Self.offersFollowUp(enabled: interaction.status.enabled, session: session, agentWaiting: activeWaitingSession != nil) {
                Button {
                    interaction.followUpSessionID = session.id
                } label: {
                    Image(systemName: "text.bubble")
                        .font(.system(size: 12, weight: .medium))
                        .padding(.top, 6)
                }
                .buttonStyle(.borderless)
                .help(L("发送后续指令"))
                .accessibilityLabel(L("发送后续指令"))
            }
        }
    }

    private func timing(_ session: LiveSession, now: Date) -> String {
        let updated = session.updatedAt.formatted(Date.FormatStyle(date: .omitted, time: .shortened).locale(AieyesLocale.locale))
        guard let elapsed = session.turnElapsed(at: now) else { return L("最后更新 ") + updated }
        return L("本轮 {0}", LiveSession.elapsedText(elapsed))
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
