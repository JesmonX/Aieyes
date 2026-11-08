import AppKit
import Combine
import WidgetKit

enum DisplayScenario: String, CaseIterable, Identifiable {
    case live, idle, working, thinking, tool, needsInput, complete, failed, lowQuota, unavailable
    var id: String { rawValue }
    var title: String {
        switch self {
        case .live: return L("实时数据")
        case .lowQuota: return L("额度不足")
        case .unavailable: return L("状态不可用")
        default: return L(phase.title)
        }
    }
    var phase: DisplayPhase {
        switch self {
        case .live, .idle: return .idle
        case .working: return .working
        case .thinking: return .thinking
        case .tool: return .tool
        case .needsInput, .lowQuota: return .needsInput
        case .complete: return .complete
        case .failed: return .failed
        case .unavailable: return .unknown
        }
    }
}

enum DisplayPreferences {
    static let dockEnabled = "display.dock.enabled"
    static let dockStyle = "display.dock.style"
    static let dockBadge = "display.dock.badge"
    static let animations = "display.animations"
    static let cardEnabled = "display.card.enabled"
    static let cardSize = "display.card.size"
    static let ribbonEnabled = "display.ribbon.enabled"
    static let onTop = "display.onTop"
    static let allSpaces = "display.allSpaces"
    static let locked = "display.locked"
    static let resetPosition = Notification.Name("AieyesResetDisplayPositions")
    static var isLabBuild: Bool { Bundle.main.object(forInfoDictionaryKey: "AieyesDisplayLab") as? Bool == true }
}

@MainActor final class DisplayStore: ObservableObject {
    @Published private(set) var snapshot = DisplaySnapshot()
    @Published var scenario = DisplayScenario.live { didSet { demoStartedAt = Date(); refresh(force: true) } }
    @Published private(set) var widgetMessage = ""
    @Published private(set) var installedWidgets: Int?
    private weak var model: AppModel?
    private var subscriptions = Set<AnyCancellable>()
    private var pending = false
    private var demoStartedAt = Date()
    private var nextWidgetReload = Date.distantPast
    private var widgetReload: DispatchWorkItem?
    private var stopped = false
    var widgetAvailable: Bool {
        guard let directory = Bundle.main.builtInPlugInsURL else { return false }
        return FileManager.default.fileExists(atPath: directory.appendingPathComponent("AieyesWidgets.appex").path)
    }

    init(model: AppModel) {
        self.model = model
        model.objectWillChange.sink { [weak self] _ in self?.schedule() }.store(in: &subscriptions)
        model.live.objectWillChange.sink { [weak self] _ in self?.schedule() }.store(in: &subscriptions)
        Timer.publish(every: 30, on: .main, in: .common).autoconnect().sink { [weak self] _ in self?.refresh(force: true) }.store(in: &subscriptions)
        refresh(force: true)
    }
    private func schedule() {
        guard !pending, !stopped else { return }; pending = true
        DispatchQueue.main.async { [weak self] in self?.pending = false; self?.refresh() }
    }
    func refresh(force: Bool = false, now: Date = Date()) {
        guard let model, !stopped else { return }
        var next: DisplaySnapshot
        if scenario == .live { next = Self.liveSnapshot(model: model, now: now) }
        else {
            next = .sample(scenario.phase, language: AieyesLocale.language, now: demoStartedAt)
            next.capturedAt = now.timeIntervalSince1970
            if scenario == .lowQuota { next.quotas[0].remaining = 8; next.quotas[1].remaining = 19 }
        }
        var comparison = next; comparison.capturedAt = snapshot.capturedAt
        guard force || comparison != snapshot else { return }
        snapshot = next
        publishWidget(forceReload: false)
    }
    static func liveSnapshot(model: AppModel, now: Date = Date()) -> DisplaySnapshot {
        var value = DisplaySnapshot()
        value.capturedAt = now.timeIntervalSince1970
        value.language = AieyesLocale.language
        value.activeCount = model.activeSessions.count
        value.waitingCount = model.sessions.filter { $0.phase == .needsInput }.count
        value.failedCount = model.sessions.filter { $0.phase == .failed }.count
        value.sessionsUnavailable = model.sessionsUnavailable && model.sessions.isEmpty
        // Attention always wins, independently of the order of session rows.
        let priorities: [(SessionPhase, DisplayPhase)] = [(.needsInput, .needsInput), (.failed, .failed), (.tool, .tool), (.thinking, .thinking), (.working, .working), (.complete, .complete)]
        value.phase = priorities.first { phase, _ in model.sessions.contains { $0.phase == phase } }?.1 ?? .idle
        if value.sessionsUnavailable { value.phase = .unknown }
        let dashboard = model.menuDashboard
        if dashboard.generatedAt > 0 {
            value.totalTokens = dashboard.summary.total
            value.costUSD = dashboard.summary.costAvailable ? dashboard.summary.cost : nil
            value.cachePercent = dashboard.summary.tokens.cacheRate.map { $0 * 100 }
            value.usageDay = Calendar.current.startOfDay(for: Date(timeIntervalSince1970: dashboard.generatedAt)).timeIntervalSince1970
        }
        value.usageUnavailable = model.dashboardError != nil
        // Match the menu's account order and free-plan visibility, not the analysis filters.
        if let account = dashboard.quotas.first(where: { !(model.settings.hideFreeQuotaCards == true && $0.freePlan == true) && !$0.windows.isEmpty }) {
            let short = account.windows.first { $0.windowMinutes == 300 }
            let long = account.windows.first { $0.windowMinutes == 10_080 }
            let windows = short != nil && long != nil ? [short!, long!] : Array(account.windows.prefix(2))
            value.quotas = windows.filter { $0.usedPercent.isFinite }.map {
                DisplayQuota(provider: Format.provider(account.provider), title: $0.windowMinutes == 300 ? "5h" : $0.windowMinutes == 10_080 ? "7d" : $0.name,
                    remaining: min(100, max(0, 100 - $0.usedPercent)), resetsAt: $0.resetsAt, stale: account.isStale || account.error != nil)
            }
        }
        return value
    }
    func syncWidgets() {
        refresh(force: true)
        publishWidget(forceReload: true)
        guard widgetAvailable else { return }
        WidgetCenter.shared.getCurrentConfigurations { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success(let widgets): self?.installedWidgets = widgets.filter { $0.kind == DisplaySnapshot.widgetKind }.count
                case .failure: self?.installedWidgets = nil
                }
            }
        }
    }
    private func publishWidget(forceReload: Bool) {
        guard let url = DisplaySnapshotFile.url(), widgetAvailable else { return }
        do { try DisplaySnapshotFile.write(snapshot, to: url) }
        catch { widgetMessage = L("小组件快照写入失败：") + error.localizedDescription; return }
        widgetMessage = L("快照已同步，显示时间由系统安排。")
        let now = Date()
        if forceReload || now >= nextWidgetReload {
            widgetReload?.cancel(); widgetReload = nil
            WidgetCenter.shared.reloadTimelines(ofKind: DisplaySnapshot.widgetKind)
            nextWidgetReload = now.addingTimeInterval(60)
        } else if widgetReload == nil {
            let work = DispatchWorkItem { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, !self.stopped else { return }
                    self.widgetReload = nil; self.publishWidget(forceReload: true)
                }
            }
            widgetReload = work
            DispatchQueue.main.asyncAfter(deadline: .now() + nextWidgetReload.timeIntervalSince(now), execute: work)
        }
    }
    func stop() {
        guard !stopped else { return }; stopped = true
        subscriptions.removeAll(); widgetReload?.cancel(); widgetReload = nil
        snapshot.appRunning = false
        publishWidget(forceReload: true)
    }
}
