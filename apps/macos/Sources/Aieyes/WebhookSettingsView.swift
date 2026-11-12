import SwiftUI

/// Sustained CPU or memory alerts for monitored hosts, shown with the other notification settings.
struct ServerAlertSettingsSection: View {
    @ObservedObject var model: AppModel
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    private static let durations = [60, 180, 300, 600, 900, 1800]
    private var preferences: ServerAlertSettings { model.settingsDraft.serverAlerts ?? ServerAlertSettings() }
    private func binding<Value>(_ key: WritableKeyPath<ServerAlertSettings, Value>) -> Binding<Value> {
        Binding(get: { preferences[keyPath: key] }, set: { value in
            var next = preferences; next[keyPath: key] = value; model.settingsDraft.serverAlerts = next
        })
    }
    /// A duration saved elsewhere stays selectable instead of leaving the picker blank.
    private var durations: [Int] { (Self.durations + [preferences.durationSeconds]).reduce(into: [Int]()) { if !$0.contains($1) { $0.append($1) } }.sorted() }
    var body: some View {
        let _ = interfaceLanguage
        Section(L("服务器告警")) {
            Toggle(L("CPU 或内存持续偏高时发送系统通知"), isOn: binding(\.enabled))
            // Thresholds also drive the webhook server alert, so they stay visible either way.
            threshold(L("CPU 阈值"), binding(\.cpuPercent))
            threshold(L("内存阈值"), binding(\.memoryPercent))
            Picker(L("持续时间"), selection: binding(\.durationSeconds)) {
                ForEach(durations, id: \.self) { Text(L("{0} 分钟", String($0 / 60))).tag($0) }
            }
            Text(L("使用率高于阈值并持续所选时间后通知一次；回落后再次升高会重新通知。Webhook 的服务器告警使用同样的阈值，检查按服务器刷新间隔进行。")).font(AppFont.secondary).foregroundStyle(.secondary)
        }
    }
    private func threshold(_ title: String, _ value: Binding<Double>) -> some View {
        Stepper(value: value, in: 50...100, step: 5) {
            HStack { Text(title); Spacer(); Text(verbatim: String(format: "%.0f%%", value.wrappedValue)).monospacedDigit().foregroundStyle(.secondary) }
        }
    }
}

/// Webhook receivers and the events sent to them, under Settings → General → Webhook.
struct WebhookSettingsView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var service: OutboundEventService
    @State private var editor: WebhookTarget?
    @State private var adding = false
    @State private var removing: WebhookTarget?
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    private var preferences: WebhookSettings { model.settingsDraft.webhooks ?? WebhookSettings() }
    private func update(_ change: (inout WebhookSettings) -> Void) {
        var next = preferences; change(&next); model.settingsDraft.webhooks = next
    }
    private func binding(_ key: WritableKeyPath<WebhookSettings, Bool>) -> Binding<Bool> {
        Binding(get: { preferences[keyPath: key] }, set: { value in update { $0[keyPath: key] = value } })
    }
    private func host(_ target: WebhookTarget) -> String { URLComponents(string: target.url.trimmingCharacters(in: .whitespacesAndNewlines))?.host ?? "" }
    var body: some View {
        let _ = interfaceLanguage
        Section("Webhook") {
            Toggle(L("发送 Webhook 通知"), isOn: binding(\.enabled))
            Text(L("事件发生时向接收地址发送一条消息：会话消息包含 Agent、机器与项目文件夹名，额度消息包含账户与额度窗口，服务器消息包含主机与使用率。")).font(AppFont.secondary).foregroundStyle(.secondary)
        }
        Section(L("接收地址")) {
            ForEach(preferences.targets) { target in row(target) }
            if preferences.targets.isEmpty { Text(L("尚未添加接收地址")).foregroundStyle(.secondary) }
            HStack {
                Button(L("添加地址")) { adding = true; editor = WebhookTarget() }.disabled(preferences.targets.count >= 8)
                if preferences.targets.count >= 8 { Text(L("最多 8 个接收地址")).font(AppFont.secondary).foregroundStyle(.secondary) }
            }
        }
        .sheet(item: $editor) { target in
            WebhookTargetEditor(target: target, isNew: adding) { saved in
                service.clearWebhookTest(saved.id)
                update { preferences in
                    if let index = preferences.targets.firstIndex(where: { $0.id == saved.id }) { preferences.targets[index] = saved }
                    else { preferences.targets.append(saved) }
                }
            }
        }
        .confirmationDialog(L("移除此接收地址？"), isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }), titleVisibility: .visible) {
            Button(L("移除"), role: .destructive) {
                if let target = removing { service.clearWebhookTest(target.id); update { $0.targets.removeAll { $0.id == target.id } } }
                removing = nil
            }
            Button(L("取消"), role: .cancel) { removing = nil }
        } message: { Text(L("移除后不再向该地址发送消息。")) }
        Section(L("通知事件")) {
            Toggle(L("订阅额度重置"), isOn: binding(\.quotaReset))
            Toggle(L("会话等待输入"), isOn: binding(\.sessionNeedsInput))
            Toggle(L("会话完成"), isOn: binding(\.sessionComplete))
            Toggle(L("会话执行失败"), isOn: binding(\.sessionFailed))
            Toggle(L("服务器告警"), isOn: binding(\.serverAlert))
            Text(L("额度重置发送每一次已确认的重置；服务器告警使用“通知”中的阈值与持续时间。")).font(AppFont.secondary).foregroundStyle(.secondary)
        }
    }
    @ViewBuilder private func row(_ target: WebhookTarget) -> some View {
        let state = service.webhookTests[target.id]
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Toggle(isOn: Binding(get: { target.enabled }, set: { value in update { preferences in
                    if let index = preferences.targets.firstIndex(where: { $0.id == target.id }) { preferences.targets[index].enabled = value }
                } })) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(target.name.isEmpty ? host(target) : target.name).lineLimit(1)
                        // The address can hold a token, so the list shows the host only.
                        Text(verbatim: WebhookTarget.formatTitle(target.format) + " · " + host(target)).font(AppFont.secondary).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                Spacer(minLength: 8)
                if state == .sending { ProgressView().controlSize(.small) }
                Button(L("发送测试")) { Task { await service.testWebhook(target) } }.disabled(state == .sending)
                Button { adding = false; editor = target } label: { Image(systemName: "pencil") }.buttonStyle(.borderless).help(L("编辑")).accessibilityLabel(L("编辑"))
                Button { removing = target } label: { Image(systemName: "trash") }.buttonStyle(.borderless).help(L("移除")).accessibilityLabel(L("移除"))
            }
            if state == .delivered { Label(L("测试消息已送达"), systemImage: "checkmark.circle").font(AppFont.secondary).foregroundStyle(Palette.ok) }
            if case .failed(let message) = state { ErrorNotice(message: message, context: L("测试消息未送达")) }
        }
    }
}

/// One receiver. Saving requires a complete http(s) address, so the autosaved draft never holds a half-typed one.
struct WebhookTargetEditor: View {
    @Environment(\.dismiss) private var dismiss
    @State var target: WebhookTarget
    let isNew: Bool
    let onSave: (WebhookTarget) -> Void
    @State private var initial: WebhookTarget?
    @State private var confirmDiscard = false
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    private var address: String { target.url.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var valid: Bool {
        guard let parts = URLComponents(string: address), let scheme = parts.scheme?.lowercased(), ["http", "https"].contains(scheme), let host = parts.host else { return false }
        return !host.isEmpty
    }
    private var dirty: Bool { initial != nil && initial != target }
    private var note: String {
        switch target.format {
        case "text": return L("以 POST 发送纯文本，标题与内容各占一行。")
        case "bark": return L("填写 Bark 推送地址，以设备密钥结尾。")
        case "slack": return L("填写 Slack Incoming Webhook 地址。")
        case "wechat": return L("填写企业微信群机器人的 Webhook 地址。")
        case "feishu": return L("填写飞书群机器人的 Webhook 地址。")
        default: return L("以 POST 发送 JSON，含 event、title、text、at 与 data 字段。")
        }
    }
    private func commit() {
        var value = target
        value.url = address; value.name = target.name.trimmingCharacters(in: .whitespacesAndNewlines)
        onSave(value); dismiss()
    }
    var body: some View {
        let _ = interfaceLanguage
        VStack(spacing: 0) {
            HStack { Text(isNew ? L("添加接收地址") : L("编辑接收地址")).font(AppFont.title); Spacer() }.padding(22)
            Form {
                LabeledContent(L("名称")) { TextField(L("名称"), text: $target.name, prompt: Text(L("可选"))).labelsHidden() }
                Picker(L("格式"), selection: $target.format) {
                    ForEach(WebhookTarget.formats, id: \.self) { Text(WebhookTarget.formatTitle($0)).tag($0) }
                }
                LabeledContent(L("地址")) { TextField(L("地址"), text: $target.url, prompt: Text(verbatim: "https://")).labelsHidden() }
                Text(note).font(AppFont.secondary).foregroundStyle(.secondary)
                if !address.isEmpty && !valid { Text(L("请输入以 http:// 或 https:// 开头的完整地址")).font(AppFont.secondary).foregroundStyle(Palette.warn) }
            }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle()).textFieldStyle(.aieyes)
            HStack {
                Spacer()
                Button(L("取消")) { if dirty { confirmDiscard = true } else { dismiss() } }.keyboardShortcut(.cancelAction)
                Button(L("保存")) { commit() }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction).disabled(!valid)
            }.padding(20)
        }
        .font(AppFont.body).frame(width: 540, height: EditorLayout.height(400)).aieyesAccent()
        .interactiveDismissDisabled(dirty)
        .updateDraftGuard("Webhook", snapshot: [target.id, target.name, target.url, target.format].joined(separator: "\u{1f}"), dirty: dirty, saving: false,
                          save: { guard valid else { return false }; commit(); return true }, discard: { dismiss() })
        .onAppear { if initial == nil { initial = target } }
        .discardDraftConfirmation($confirmDiscard) { dismiss() }
    }
}
