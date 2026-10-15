import Foundation
import Combine
import UserNotifications

private struct QuotaResetEvent: Decodable {
    struct Window: Decodable { var name: String, usedPercent: Double, expectedResetAt: Double }
    var id: String, provider: String, name: String, kind: String
    var windows: [Window]
    var title: String { kind == "unexpected" ? "订阅额度提前重置" : "订阅额度已重置" }
    var body: String {
        let details = windows.map { window in
            var text = "\(window.name)：剩余 \(String(format: "%.0f", 100 - window.usedPercent))%"
            if kind == "unexpected" { text += "（原预计 \(Format.date(window.expectedResetAt))）" }
            return text
        }.joined(separator: "；")
        return "\(Format.provider(provider)) · \(name)\n\(details)"
    }
}

@MainActor final class QuotaNotificationService: NSObject, ObservableObject, UNUserNotificationCenterDelegate {
    @Published private(set) var permission = "系统通知状态未读取"
    @Published private(set) var error: String?
    private let engine = EngineClient(configurationOnly: true)
    private let since = Int(Date().timeIntervalSince1970)
    private var task: Task<Void, Never>?
    private var asked = false
    private var stopped = false
    private var center: UNUserNotificationCenter? {
        // macOS asserts instead of returning an error for an unbundled executable.
        guard Bundle.main.bundleIdentifier != nil else {
            permission = "系统通知需要从 Aieyes.app 启动"
            return nil
        }
        return UNUserNotificationCenter.current()
    }
    func start() {
        guard task == nil, !stopped, let center else { return }
        center.delegate = self
        task = Task { [weak self] in
            while !Task.isCancelled {
                guard let self, !self.stopped else { return }
                await self.poll()
                do { try await Task.sleep(nanoseconds: 5_000_000_000) } catch { return }
            }
        }
    }
    private func poll() async {
        do {
            let settings: Settings = try await engine.call("settings.get")
            let enabled = settings.accounts.contains { a in
                a.quotaEnabled && a.archived != true && a.provider != "deepseek"
                && settings.sources.contains { settings.sourceEnabled($0) && a.uses($0) }
                && ((a.quotaResetNotifications?.unexpected ?? settings.quotaResetNotifications?.unexpected ?? true)
                    || (a.quotaResetNotifications?.natural ?? settings.quotaResetNotifications?.natural ?? false))
            }
            if enabled && !asked { asked = true; await authorize() }
            else { await refreshPermission() }
            let events: [QuotaResetEvent] = try await engine.call("quotaResetNotifications.take", params: ["since":since])
            for event in events {
                guard !Task.isCancelled, !stopped else { return }
                await deliver(id: event.id, title: event.title, body: event.body)
            }
        } catch { if !Task.isCancelled && !stopped { self.error = error.localizedDescription } }
    }
    func refreshPermission() async {
        // Snapshot and model-test instances never contact the user's notification center.
        guard task != nil, !stopped, let center else { return }
        let settings = await center.notificationSettings()
        switch settings.authorizationStatus {
        case .authorized, .provisional: permission = "系统通知已授权"
        case .denied: permission = "系统通知未授权，请在系统设置 → 通知 → Aieyes 中允许通知"
        case .notDetermined: permission = "尚未请求系统通知权限"
        @unknown default: permission = "无法读取通知权限，请使用测试通知验证"
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
    private func deliver(id: String, title: String, body: String) async {
        guard let center else { error = permission; return }
        let status = await center.notificationSettings().authorizationStatus
        guard status == .authorized || status == .provisional else {
            error = "通知未发送：请先允许 Aieyes 发送系统通知。"; return
        }
        let content = UNMutableNotificationContent()
        content.title = title; content.body = body; content.sound = .default
        do { try await center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil)) }
        catch { self.error = "通知发送失败：" + error.localizedDescription }
    }
    func test() async {
        error = nil; await authorize()
        await deliver(id: UUID().uuidString, title: "Aieyes 测试通知", body: "订阅额度重置提醒已连接系统通知。")
    }
    func stop() { stopped = true; task?.cancel(); task = nil; engine.stop() }
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .list, .sound])
    }
}
