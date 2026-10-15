import SwiftUI

struct QuotaNotificationSettingsView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var service: QuotaNotificationService
    private func binding(_ key: WritableKeyPath<QuotaResetNotificationSettings, Bool?>, fallback: Bool) -> Binding<Bool> {
        Binding(get: { model.settingsDraft.quotaResetNotifications?[keyPath: key] ?? fallback }, set: { value in
            var preferences = model.settingsDraft.quotaResetNotifications ?? QuotaResetNotificationSettings()
            preferences[keyPath: key] = value; model.settingsDraft.quotaResetNotifications = preferences
        })
    }
    var body: some View {
        Section("订阅额度重置提醒") {
            Toggle("非预期重置时提醒", isOn: binding(\.unexpected, fallback: true))
            Toggle("自然重置时也提醒", isOn: binding(\.natural, fallback: false))
            Text("账户可单独覆盖这些默认设置。额度一直满额时不提醒；仅在实时查询确认重置后通知。关闭窗口后仍会检测，退出应用或电脑休眠时暂停。").font(AppFont.secondary).foregroundStyle(.secondary)
        }
        Section("系统通知") {
            Text(service.permission).font(AppFont.secondary)
            Button("发送测试通知") { Task { await service.test() } }
            if let error = service.error { Text(error).foregroundStyle(Palette.danger).textSelection(.enabled) }
            Text("通知的显示与声音遵循系统通知和专注模式设置。").font(AppFont.secondary).foregroundStyle(.secondary)
        }
        .task { await service.refreshPermission() }
    }
}

struct AccountResetNotificationPicker: View {
    let title: String
    let value: Bool?
    let inherited: Bool
    let changed: (Bool?) -> Void
    var body: some View {
        Picker(title, selection: Binding(get: { value.map { $0 ? "on" : "off" } ?? "inherit" }, set: { changed($0 == "inherit" ? nil : $0 == "on") })) {
            Text("沿用全局 · \(inherited ? "开启" : "关闭")").tag("inherit")
            Text("开启").tag("on")
            Text("关闭").tag("off")
        }
    }
}
