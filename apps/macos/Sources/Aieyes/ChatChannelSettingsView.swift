import SwiftUI

struct ChatChannelSettingsView: View {
    @ObservedObject var service: ChatChannelService
    @State private var editing = false
    @AppStorage("aieyes.language") private var language = "zh-CN"
    var body: some View {
        let _ = language
        Group {
        Section(L("飞书会话控制")) {
            Text(L("在个人私聊中回复问题，或在中断、完成后继续原会话。支持本机与 SSH，按 Agent 和终端能力接入。")).font(AppFont.secondary).foregroundStyle(.secondary)
            if let status = service.status {
                LabeledContent(L("连接状态"), value: status.connectionTitle)
                LabeledContent(L("个人私聊"), value: status.paired ? L("已配对") : L("尚未配对"))
                if !status.appId.isEmpty { LabeledContent("App ID") { Text(verbatim: status.appId).textSelection(.enabled) } }
                if let error = status.connection.error { ErrorNotice(message: L(error), context: L("聊天渠道")) }
                if let error = status.deliveryError ?? status.workerError { ErrorNotice(message: L(error.message), context: L("聊天渠道")) }
                if let count = status.undelivered, count > 0 { Text(L("{0} 条消息未送达", count)).font(AppFont.secondary).foregroundStyle(.secondary) }
                HStack {
                    Button(L("配置飞书")) { editing = true }
                    Button(L("创建配对码")) { Task { await service.pair() } }.disabled(!status.enabled || !status.hasSecret || status.paired || service.busy)
                    if status.paired { Button(L("解除配对")) { Task { await service.unpair() } }.disabled(service.busy) }
                }
            } else { ProgressView(L("正在读取聊天渠道…")) }
            if let pairing = service.pairing, pairing.expiresAt > Date().timeIntervalSince1970 {
                Text(L("在飞书中私聊机器人，发送以下指令；配对码 10 分钟内有效。")).font(AppFont.secondary)
                HStack {
                    Text(verbatim: pairing.message).monospaced().textSelection(.enabled)
                    Spacer()
                    Button(L("复制配对指令")) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(pairing.message, forType: .string) }
                }
            }
            if let error = service.error { ErrorNotice(message: error, context: L("聊天渠道")) }
        }
        Section(L("接入步骤")) {
            Text(L("在飞书开放平台创建企业自建应用并启用机器人。"))
            Text(L("订阅 im.message.receive_v1，选择长连接，并开通接收单聊消息和以应用身份发送消息的权限。"))
            Text(L("发布应用，在此保存 App ID 与 App Secret，再生成配对码完成私聊绑定。"))
            Text(L("仅向已配对账号发送问题与最终回复。消息和指令在本机保留 24 小时；退出 Aieyes 或电脑休眠时暂停服务。")).font(AppFont.secondary).foregroundStyle(.secondary)
            Button(L("授权终端自动化")) { Task { await SessionChannelAutomation.authorize() } }
        }
        Section(L("渠道指令")) {
            Text(L("已提交或结果不确定的指令不会自动重发。会话已改变时，请发送新指令。")).font(AppFont.secondary).foregroundStyle(.secondary)
            if service.commands.isEmpty { Text(L("暂无渠道指令")).foregroundStyle(.secondary) }
            ForEach(service.commands.prefix(20)) { command in
                VStack(alignment: .leading, spacing: 6) {
                    HStack { Text(verbatim: command.id).monospaced(); Spacer(); Text(command.title).foregroundStyle(.secondary) }
                    Text(verbatim: command.body).lineLimit(4).textSelection(.enabled)
                    if !command.reason.isEmpty { Text(L(command.reason)).font(AppFont.secondary).foregroundStyle(.secondary) }
                    HStack {
                        if command.status == "blocked" { Button(L("重试")) { Task { await service.update(command, retry: true) } } }
                        if command.canCancel { Button(L("取消")) { Task { await service.update(command, retry: false) } } }
                    }.disabled(service.busy)
                }
            }
        }
        Section(L("可关联会话")) {
            if service.sessions.isEmpty { Text(L("启用渠道后，这里显示已监测的会话。")).foregroundStyle(.secondary) }
            ForEach(service.sessions.prefix(30)) { session in
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: session.title).textSelection(.enabled)
                    Text(L(session.control)).font(AppFont.secondary).foregroundStyle(.secondary)
                }
            }
        }
        }
        .task { service.settingsVisible = true; await service.refresh() }
        .onDisappear { service.settingsVisible = false }
        .sheet(isPresented: $editing) { if let status = service.status { ChatChannelEditor(service: service, initial: status) } }
    }
}

private struct ChatChannelEditor: View {
    @ObservedObject var service: ChatChannelService
    let initial: ChatChannelStatus
    @Environment(\.dismiss) private var dismiss
    @State private var enabled = false
    @State private var allowTerminal = false
    @State private var appID = ""
    @State private var secret = ""
    @State private var error: String?
    @State private var loaded = false
    @State private var saving = false
    private var dirty: Bool { loaded && (enabled != initial.enabled || allowTerminal != initial.allowTerminal || appID != initial.appId || !secret.isEmpty) }
    private func save() async -> Bool {
        guard !saving else { return false }
        saving = true; defer { saving = false }
        do {
            try await service.configure(revision: initial.revision, enabled: enabled, appID: appID.trimmingCharacters(in: .whitespacesAndNewlines), secret: secret, allowTerminal: allowTerminal)
            secret = ""; dismiss(); return true
        } catch { self.error = error.localizedDescription; return false }
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text(L("配置飞书")).font(AppFont.title); Spacer() }.padding(22)
            Form {
                Toggle(L("启用飞书会话控制"), isOn: $enabled)
                LabeledContent("App ID") { TextField("App ID", text: $appID, prompt: Text(verbatim: "cli_…")).labelsHidden() }
                LabeledContent("App Secret") { SecureField(initial.hasSecret ? L("已保存，留空保留") : L("输入应用密钥"), text: $secret).labelsHidden() }
                Toggle(L("允许按终端适配输入"), isOn: $allowTerminal)
                Text(L("优先使用原生接口；终端输入需要准确定位原进程和分屏。桌面锁定、权限不足或定位不明确时保留指令。")).font(AppFont.secondary).foregroundStyle(.secondary)
                if let error { ErrorNotice(message: error, context: L("配置飞书")) }
            }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle()).textFieldStyle(.aieyes)
            HStack {
                Spacer()
                Button(L("取消")) { secret = ""; dismiss() }.keyboardShortcut(.cancelAction)
                Button(L("保存")) { Task { _ = await save() } }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction).disabled(saving || appID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (!initial.hasSecret && secret.isEmpty))
            }.padding(20)
        }
        .font(AppFont.body).frame(width: 550, height: EditorLayout.height(420)).aieyesAccent()
        .interactiveDismissDisabled(dirty)
        .updateDraftGuard(L("聊天渠道"), snapshot: [appID, String(secret.hashValue), String(enabled), String(allowTerminal)].joined(separator: "|"), dirty: dirty, saving: saving, save: { await save() }, discard: { secret = ""; dismiss() })
        .onAppear { if !loaded { enabled = initial.enabled; allowTerminal = initial.allowTerminal; appID = initial.appId; loaded = true } }
    }
}
