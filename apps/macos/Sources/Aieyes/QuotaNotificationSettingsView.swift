import SwiftUI

struct QuotaNotificationSettingsView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var service: QuotaNotificationService
    @State private var soundError: String?
    private func binding(_ key: WritableKeyPath<QuotaResetNotificationSettings, Bool?>, fallback: Bool) -> Binding<Bool> {
        Binding(get: { model.settingsDraft.quotaResetNotifications?[keyPath: key] ?? fallback }, set: { value in
            var preferences = model.settingsDraft.quotaResetNotifications ?? QuotaResetNotificationSettings()
            preferences[keyPath: key] = value; model.settingsDraft.quotaResetNotifications = preferences
        })
    }
    private func soundRow(_ kind: String, title: String, enabled: WritableKeyPath<SessionSoundSettings, Bool?>, tone: WritableKeyPath<SessionSoundSettings, String?>) -> some View {
        HStack(spacing: 10) {
            Toggle(title, isOn: Binding(get: { model.settingsDraft.sessionSounds?[keyPath: enabled] ?? true }, set: { value in
                var preferences = model.settingsDraft.sessionSounds ?? SessionSoundSettings(); preferences[keyPath: enabled] = value; model.settingsDraft.sessionSounds = preferences
            }))
            Picker(kind == "needsInput" ? L("等待输入音色") : L("任务完成音色"), selection: Binding(get: { (model.settingsDraft.sessionSounds ?? SessionSoundSettings()).tone(for: kind) }, set: { value in
                var preferences = model.settingsDraft.sessionSounds ?? SessionSoundSettings(); preferences[keyPath: tone] = value; model.settingsDraft.sessionSounds = preferences
            })) {
                ForEach(CoreContract.sessionSoundTones, id: \.id) { Text(L($0.name)).tag($0.id) }
            }.labelsHidden().frame(width: 116)
            Button(L("试听")) { soundError = model.sessionSounds.preview(kind, preferences: model.settingsDraft.sessionSounds) }
                .accessibilityLabel(kind == "needsInput" ? L("试听等待输入") : L("试听任务完成"))
        }
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Section(L("会话提示音")) {
            soundRow("needsInput", title: L("等待输入提示音"), enabled: \.needsInput, tone: \.needsInputTone)
            soundRow("complete", title: L("任务完成提示音"), enabled: \.complete, tone: \.completeTone)
            HStack(spacing: 10) {
                Text(L("提示音音量"))
                HelpNote(title: L("会话提示音"), text: L("仅在新状态发生时播放；关闭面板后仍有效。音量相对于系统输出，0% 为静音。"))
                Spacer()
                Slider(value: Binding(get: { Double((model.settingsDraft.sessionSounds ?? SessionSoundSettings()).playbackVolume) }, set: { value in
                    var preferences = model.settingsDraft.sessionSounds ?? SessionSoundSettings(); preferences.volume = value; model.settingsDraft.sessionSounds = preferences
                }), in: 0...1, step: 0.01).frame(maxWidth: 220).accessibilityLabel(L("提示音音量"))
                Text("\(Int(((model.settingsDraft.sessionSounds ?? SessionSoundSettings()).playbackVolume * 100).rounded()))%")
                    .monospacedDigit().frame(width: 44, alignment: .trailing)
            }
            if let soundError { ErrorNotice(message: soundError) }
        }
        Section(L("订阅额度重置提醒")) {
            Toggle(L("提前重置时提醒"), isOn: binding(\.unexpected, fallback: true))
            Toggle(L("到期重置时提醒"), isOn: binding(\.natural, fallback: false))
            DisclosureGroup(L("检测条件")) {
                Text(L("账户可单独设置提醒规则。退出应用或电脑休眠时暂停检测。")).font(AppFont.secondary).foregroundStyle(.secondary)
                Text(L("以实时查询确认的额度重置为准，按上次预计重置时间区分提前与到期。持续满额时不提醒；关闭窗口后仍会检测。")).font(AppFont.secondary).foregroundStyle(.secondary)
            }
        }
        Section(L("系统通知")) {
            Text(L(service.permission)).font(AppFont.secondary)
            HStack {
                Button(L("发送测试通知")) { Task { await service.test() } }
                HelpNote(title: L("系统通知"), text: L("通知显示和声音由系统通知及专注模式设置控制。"))
            }
            if let message = service.testMessage { Text(L(message)).font(AppFont.secondary).foregroundStyle(.secondary) }
            if let error = service.error { ErrorNotice(message: error) }
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
