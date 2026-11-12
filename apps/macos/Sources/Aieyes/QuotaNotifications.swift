import AppKit
import Foundation
import Combine
import UserNotifications

private struct QuotaResetEvent: Decodable {
    struct Window: Decodable { var name: String, usedPercent: Double, expectedResetAt: Double }
    var id: String, provider: String, name: String, kind: String
    var windows: [Window]
    var title: String { kind == "unexpected" ? L("订阅额度提前重置") : L("订阅额度已重置") }
    var body: String {
        let details = windows.map { window in
            var text = L("{0}：剩余 {1}%" , window.name, String(format: "%.0f", 100 - window.usedPercent))
            if kind == "unexpected" { text += L("（原预计 {0}）" , Format.date(window.expectedResetAt)) }
            return text
        }.joined(separator: "；")
        return "\(Format.provider(provider)) · \(name)\n\(details)"
    }
}

@MainActor final class QuotaNotificationService: NSObject, ObservableObject, UNUserNotificationCenterDelegate {
    @Published private(set) var permission = L("系统通知状态未读取")
    @Published private(set) var error: String?
    @Published private(set) var testMessage: String?
    /// Counts reset events the core confirmed since launch; the desktop pet celebrates each new one.
    @Published private(set) var confirmedResets = 0
    private let engine = EngineClient(configurationOnly: true)
    private let since = Int(Date().timeIntervalSince1970)
    private var task: Task<Void, Never>?
    private var draining = false
    private var started = false
    private var latestRevisions: [String: Int] = [:]
    private var notificationRevision: Int?
    private var configurationRevision: Int?
    private var observers: [NSObjectProtocol] = []
    private var asked = false
    private var stopped = false
    // Events already taken from core but not yet delivered (permission off or add() failed); retried once authorized.
    private struct Pending { var id: String, title: String, body: String, queuedAt: Date }
    private var pending: [Pending] = []
    // `error` also reports the user's test notification; only background failures clear themselves.
    private var backgroundError = false
    private var center: UNUserNotificationCenter? {
        // macOS asserts instead of returning an error for an unbundled executable.
        guard Bundle.main.bundleIdentifier != nil else {
            permission = L("系统通知需要从 Aieyes.app 启动")
            return nil
        }
        return UNUserNotificationCenter.current()
    }
    func start() {
        guard !started, !stopped, let center else { return }
        started = true; center.delegate = self
        observers.append(NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in Task { @MainActor in await self?.refreshPermission(); await self?.flushPending() } })
        task = Task { [weak self] in await self?.drain(); self?.task = nil }
    }
    // AppModel owns the five-second cross-process revision fallback. Reuse its
    // snapshot instead of polling a second core and draining on unrelated writes.
    func observe(_ revisions: [String: Int]) {
        for (key, value) in revisions where value > latestRevisions[key, default: -1] { latestRevisions[key] = value }
        guard started, !stopped, task == nil,
              configurationRevision != latestRevisions["settings", default: 0] || notificationRevision != latestRevisions["notifications", default: 0] else { return }
        task = Task { [weak self] in await self?.drain(); self?.task = nil }
    }
    func drain() async {
        guard !draining, !stopped, started else { return }; draining = true
        defer { draining = false }
        do {
            if latestRevisions.isEmpty {
                let values: [String: Int] = try await engine.call("runtime.revisions")
                for (key, value) in values where value > latestRevisions[key, default: -1] { latestRevisions[key] = value }
            }
            repeat {
                let revision = latestRevisions
                if configurationRevision == nil || configurationRevision != revision["settings"] {
                    let settings: Settings = try await engine.call("settings.get")
                    let enabled = settings.accounts.contains { a in
                        a.quotaEnabled && a.archived != true && settings.supports(a.provider, "quotaReset")
                        && settings.sources.contains { settings.sourceEnabled($0) && a.uses($0) }
                        && ((a.quotaResetNotifications?.unexpected ?? settings.quotaResetNotifications?.unexpected ?? true)
                            || (a.quotaResetNotifications?.natural ?? settings.quotaResetNotifications?.natural ?? false))
                    }
                    if enabled && !asked { asked = true; await authorize() }
                    else { await refreshPermission() }
                    configurationRevision = revision["settings"] ?? 0
                    await flushPending()
                    }
                if notificationRevision == nil || notificationRevision != revision["notifications"] {
                    let events: [QuotaResetEvent] = try await engine.call("quotaResetNotifications.take", params: ["since":since])
                    notificationRevision = revision["notifications"] ?? 0
                    let now = Date()
                    let fresh = events.filter { event in !pending.contains { $0.id == event.id } }
                    for event in fresh { pending.append(Pending(id: event.id, title: event.title, body: event.body, queuedAt: now)) }
                    if !fresh.isEmpty { confirmedResets += 1 }
                    await flushPending()
                }
            } while !stopped && !Task.isCancelled && (configurationRevision != latestRevisions["settings", default: 0] || notificationRevision != latestRevisions["notifications", default: 0])
            if backgroundError && pending.isEmpty { error = nil; backgroundError = false }
        } catch { if !Task.isCancelled && !stopped { self.error = error.localizedDescription; backgroundError = true } }
    }
    /// Delivers queued events; keeps undelivered ones (at most 20, no older than 6 hours) for the next attempt.
    private func flushPending() async {
        let cutoff = Date().addingTimeInterval(-6 * 3600)
        pending.removeAll { $0.queuedAt < cutoff }
        if pending.count > 20 { pending.removeFirst(pending.count - 20) }
        guard !pending.isEmpty else { return }
        var failed: [Pending] = []
        for item in pending {
            guard !Task.isCancelled, !stopped else { return }
            if !(await deliver(id: item.id, title: item.title, body: item.body)) { failed.append(item) }
        }
        pending = failed
        backgroundError = !failed.isEmpty
        if failed.isEmpty { error = nil }
    }
    func refreshPermission() async {
        // Snapshot and model-test instances never contact the user's notification center.
        guard started, !stopped, let center else { return }
        let settings = await center.notificationSettings()
        switch settings.authorizationStatus {
        case .authorized, .provisional: permission = L("系统通知已授权")
        case .denied: permission = L("系统通知未授权，请在系统设置 → 通知 → Aieyes 中允许通知")
        case .notDetermined: permission = L("尚未请求系统通知权限")
        @unknown default: permission = L("无法读取通知权限，请使用测试通知验证")
        }
    }
    private func authorize() async {
        guard let center else { return }
        if await center.notificationSettings().authorizationStatus == .notDetermined {
            do { _ = try await center.requestAuthorization(options: [.alert, .sound]) }
            catch { self.error = error.localizedDescription }
        }
        await refreshPermission()
    }
    @discardableResult private func deliver(id: String, title: String, body: String) async -> Bool {
        guard let center else { error = permission; return false }
        let status = await center.notificationSettings().authorizationStatus
        guard status == .authorized || status == .provisional else {
            error = L("通知未发送：请先允许 Aieyes 发送系统通知。"); return false
        }
        let content = UNMutableNotificationContent()
        content.title = title; content.body = body; content.sound = .default
        do { try await center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil)); return true }
        catch { self.error = L("通知发送失败：") + L(error.localizedDescription); return false }
    }
    /// Server alerts share the permission handling and retry queue with quota resets.
    func post(id: String, title: String, body: String) async {
        guard started, !stopped else { return }
        if !asked { asked = true; await authorize() }
        if !pending.contains(where: { $0.id == id }) { pending.append(Pending(id: id, title: title, body: body, queuedAt: Date())) }
        await flushPending()
    }
    func test() async {
        error = nil; backgroundError = false; testMessage = nil; await authorize()
        await deliver(id: UUID().uuidString, title: L("Aieyes 测试通知"), body: L("收到此通知表示系统通知可正常显示。"))
        if error == nil { testMessage = L("测试通知已提交，请检查系统显示。") }
    }
    func stop() { for observer in observers { NotificationCenter.default.removeObserver(observer) }; observers.removeAll(); stopped = true; task?.cancel(); task = nil; engine.stop() }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .list, .sound])
    }
}
