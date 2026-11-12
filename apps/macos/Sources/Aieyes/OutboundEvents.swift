import AppKit
import Foundation

/// One minute of a host's readings; a metric the host does not report stays nil and is never drawn as zero.
struct HostMinute: Decodable, Equatable {
    var t: Double
    var cpu: Double?, cpuMax: Double?, memory: Double?, gpu: Double?
}
private struct HostHistorySnapshot: Decodable {
    struct Row: Decodable { var id: String, minutes: [HostMinute] }
    var hosts: [Row]
}
private struct HostAlertEvent: Decodable {
    var id: String, hostId: String, name: String, metric: String
    var value: Double, threshold: Double, durationSeconds: Int, detectedAt: Double
    var title: String { metric == "memory" ? L("{0} 内存使用率持续偏高", name) : L("{0} CPU 使用率持续偏高", name) }
    var body: String { L("当前 {0}%，已持续 {1} 分钟", String(format: "%.0f", value), String(max(1, durationSeconds / 60))) }
}
private struct WebhookDispatchResult: Decodable { var delivered: Int, failed: Int, pending: Int }
private struct WebhookTestResult: Decodable { var delivered: Bool }
/// Core `app.launch`: which version is running and whether it just changed.
struct LaunchState: Decodable, Equatable { var version: String, previousVersion: String?, updated: Bool }

/// Outbound work that only the app can start: webhook delivery, server alerts and the minute
/// history behind the server sparklines. Each call goes to the core process that owns the state.
@MainActor final class OutboundEventService: ObservableObject {
    /// Minute rollups per host id; empty until the first read of the servers page.
    @Published private(set) var history: [String: [HostMinute]] = [:]
    @Published private(set) var launch: LaunchState?
    /// Per-target result of "send test", shown next to that target until it is edited again.
    @Published private(set) var webhookTests: [String: WebhookTestState] = [:]
    enum WebhookTestState: Equatable { case sending, delivered, failed(String) }

    private weak var model: AppModel?
    private let since = Int(Date().timeIntervalSince1970)
    private var loop: Task<Void, Never>?
    private var retry: Task<Void, Never>?
    private var dispatching = false, dispatchAgain = false
    private var stopped = false

    /// Binds the service to its model without starting the poll loop, for a caller that reads on
    /// its own schedule: the snapshot renderer and the window checks.
    func attach(_ model: AppModel) { self.model = model }

    func start(_ model: AppModel) {
        guard loop == nil, !stopped else { return }
        attach(model)
        Task { await self.recordLaunch(); await self.dispatchWebhooks() }
        loop = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, !self.stopped else { return }
                await self.poll()
                do { try await Task.sleep(for: .seconds(15)) } catch { return }
            }
        }
    }

    private func poll() async {
        guard let model, !stopped, !model.installingUpdate else { return }
        if needsAlertPolling(model.settings) {
            // Taking the queue also lets that process hand its own alert webhooks to the shared store.
            let events: [HostAlertEvent]? = try? await model.metricsEngine.call("hostAlerts.take", params: ["since": since])
            if model.settings.serverAlerts?.enabled == true {
                for event in events ?? [] { await model.resetNotifications.post(id: "server-alert-" + event.id, title: event.title, body: event.body) }
            }
        }
        if model.serverTabVisible { await refreshHistory() }
    }

    private func needsAlertPolling(_ settings: Settings) -> Bool {
        if settings.serverAlerts?.enabled == true { return true }
        guard let webhooks = settings.webhooks, webhooks.enabled, webhooks.serverAlert else { return false }
        return webhooks.targets.contains { $0.enabled && !$0.url.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    /// Reads the minute rollups for the visible server page; a host that left monitoring disappears.
    func refreshHistory() async {
        guard let model, !stopped else { return }
        guard let snapshot: HostHistorySnapshot = try? await model.metricsEngine.call("hosts.history") else { return }
        var next: [String: [HostMinute]] = [:]
        for row in snapshot.hosts { next[row.id] = row.minutes }
        if next != history { history = next }
    }

    /// Delivers queued webhooks. Called when the shared `webhooks` revision moves and by the retry timer.
    func dispatchWebhooks() async {
        guard let model, !stopped else { return }
        if dispatching { dispatchAgain = true; return }
        dispatching = true; defer { dispatching = false }
        repeat {
            dispatchAgain = false
            guard !model.installingUpdate else { return }
            let result: WebhookDispatchResult? = try? await model.networkEngine.call("webhooks.dispatch")
            if stopped { return }
            // A failed call or a target still behind the backoff is retried on the shared timer.
            if result == nil || (result?.pending ?? 0) > 0, retry == nil { scheduleRetry() }
        } while dispatchAgain && !stopped
    }

    private func scheduleRetry() {
        retry = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(60)) } catch { return }
            guard let self else { return }
            self.retry = nil
            await self.dispatchWebhooks()
        }
    }

    /// Sends one test message through the network lane, so nothing waits on it.
    func testWebhook(_ target: WebhookTarget) async {
        guard let model, !stopped else { return }
        webhookTests[target.id] = .sending
        do {
            let payload: [String: Any] = ["id": target.id, "name": target.name, "url": target.url, "format": target.format, "enabled": true]
            let _: WebhookTestResult = try await model.networkEngine.call("webhooks.test", params: ["target": payload])
            webhookTests[target.id] = .delivered
        } catch { webhookTests[target.id] = .failed(error.localizedDescription) }
    }

    func clearWebhookTest(_ id: String) { webhookTests[id] = nil }

    /// Records this launch and reports a version change; the first report after an update is the one shown.
    func recordLaunch() async {
        guard let model, launch == nil else { return }
        guard let state: LaunchState = try? await model.engine.call("app.launch") else { return }
        launch = state
        if state.updated { showsWhatsNew = true }
    }
    /// The one-time banner after an update; cleared when dismissed or once the notes are opened.
    @Published var showsWhatsNew = false
    /// The release-notes sheet in the main window.
    @Published var presentingWhatsNew = false

    func stop() {
        stopped = true
        loop?.cancel(); loop = nil
        retry?.cancel(); retry = nil
    }
}
