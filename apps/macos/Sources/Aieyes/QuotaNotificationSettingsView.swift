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
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Section(L("订阅额度重置提醒")) {
            Toggle(L("提前重置时提醒"), isOn: binding(\.unexpected, fallback: true))
            Toggle(L("到期重置时提醒"), isOn: binding(\.natural, fallback: false))
            Text(L("账户可单独设置提醒规则。退出应用或电脑休眠时暂停检测。")).font(AppFont.secondary).foregroundStyle(.secondary)
            DisclosureGroup(L("检测条件")) {
                Text(L("以实时查询确认的额度重置为准，按上次预计重置时间区分提前与到期。持续满额时不提醒；关闭窗口后仍会检测。")).font(AppFont.secondary).foregroundStyle(.secondary)
            }
        }
        Section(L("系统通知")) {
            Text(service.permission).font(AppFont.secondary)
            Button(L("发送测试通知")) { Task { await service.test() } }
            if let message = service.testMessage { Text(L(message)).font(AppFont.secondary).foregroundStyle(.secondary) }
            if let error = service.error { ErrorNotice(message: error) }
            Text(L("通知显示和声音由系统通知及专注模式设置控制。")).font(AppFont.secondary).foregroundStyle(.secondary)
        }
        .task { await service.refreshPermission() }
    }
}

struct AccountResetNotificationPicker: View {
    let title: String
    let value: Bool?
    let inherited: Bool
    let changed: (Bool?) -> Void
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Picker(title, selection: Binding(get: { value.map { $0 ? "on" : "off" } ?? "inherit" }, set: { changed($0 == "inherit" ? nil : $0 == "on") })) {
            Text(L("沿用全局 · {0}" , inherited ? L("开启") : L("关闭"))).tag("inherit")
            Text(L("开启")).tag("on")
            Text(L("关闭")).tag("off")
        }
    }
}
