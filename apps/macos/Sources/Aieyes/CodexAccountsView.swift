import SwiftUI

struct CodexProfile: Decodable, Identifiable {
    struct Identity: Decodable { var key: String, workspace: String, email: String, plan: String }
    var id: String, name: String, identity: Identity, current: Bool?
}
struct CodexProcess: Decodable, Identifiable { var pid: Int, name: String, canClose: Bool; var parentPid: Int?, blockingReason: String?; var id: Int { pid } }
struct CodexInspection: Decodable { var profiles: [CodexProfile], processes: [CodexProcess], version: String?, storageMode: String; var storedIdentity: String?, selected: CodexSelectedConnection?; var cliAvailable: Bool?, resolvedBinary: String?, cliError: String?, cliErrorCode: String? }
extension CodexInspection {
    /// Codex's cli_auth_credentials_store value; other configured values are shown as written.
    var storageLabel: String { switch storageMode { case "file": L("文件"); case "keyring": L("系统凭据库"); case "auto": L("自动"); default: storageMode } }
}
struct CodexOperation: Decodable, Identifiable {
    var operationId: String, status: String, authUrl: String?, userCode: String?, error: String?, message: String?, processes: [CodexProcess]?
    var accountArchived: Bool?, quotaEnabled: Bool?
    var accountId: String?, statusWarning: String?
    var preservedProcesses: [CodexProcess]?
    var id: String { operationId }
}

struct CodexAccountsView: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    var targetAccountID: String? = nil
    var initialSourceID: String? = nil
    var intent = "login"
    var onBusyChanged: (Bool) -> Void = { _ in }
    @State private var restoredAccountID = ""
    @State private var sourceID = ""
    @State private var info: CodexInspection?
    @State private var operation: CodexOperation?
    @State private var prepared: CodexOperation?
    @State private var switchSheet = false
    @State private var switchTarget: CodexProfile?
    @State private var switchSourceID = ""
    @State private var switchError: String?
    @State private var switchedProfileID: String?
    @State private var switchSettingsPending = false
    @State private var switchWarning: String?
    @State private var busy = false
    @State private var error: String?
    @State private var notice: String?
    @State private var loginSheet = false
    @State private var reauthID: String?
    @State private var removeProfile: CodexProfile?
    private let engine = EngineClient()
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == "codex" && model.settings.sourceEnabled($0) } }
    private var source: AgentSource? { sources.first { $0.id == sourceID } }
    private var accountID: String { targetAccountID ?? restoredAccountID }
    private var account: AgentAccount? { model.settings.accounts.first { $0.id == accountID && $0.provider == "codex" } }
    private var visibleProfiles: [CodexProfile] {
        guard let account = model.settings.accounts.first(where: { $0.id == accountID && $0.provider == "codex" }) else { return info?.profiles ?? [] }
        return (info?.profiles ?? []).filter { profile in
            let reference = (source?.codexHomeId ?? "") + ":" + profile.id
            if let key = account.identityKey { return key == profile.identity.key }
            return account.profileRefs.contains(reference) || !model.settings.accounts.contains { $0.profileRefs.contains(reference) }
        }
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Form {
            Section(intent == "switch" ? L("切换账户") : L("Codex 登录")) {
                Text(intent == "switch" ? L("选择目标账户后，检查并确认切换。") : account.map { L("在所选机器登录「") + $0.name + L("」，请使用同一个账户。") } ?? L("登录成功后自动添加账户，默认以邮箱命名；已有账户会自动识别。")).foregroundStyle(.secondary)
                Text(L("切换会改变机器当前使用的账户。")).foregroundStyle(.secondary)
                Picker(L("运行位置"), selection: $sourceID) { Text(L("请选择来源")).tag(""); ForEach(sources) { Text(model.machineName($0.hostId ?? "local") + " · " + $0.displayLocation).tag($0.id).help($0.path) } }.disabled(busy || operation != nil)
                if sources.isEmpty { Text(L("暂无可用运行位置。请启用 Codex、选择机器并开启目录采集。")).foregroundStyle(.secondary); Button(L("机器与采集设置")) { model.openSettings(.init(provider: "codex", section: "machines")); dismiss() } }
                if let source { Text(source.path).font(AppFont.secondary).textSelection(.enabled) }
                if let error { HStack { ErrorNotice(message: error); Button(L("重试读取")) { perform { if let id = switchedProfileID { await refreshAfterSwitch(profileID: id) } else { try await refresh() } } }.disabled(busy) } }
                if let notice { Text(L(notice)).foregroundStyle(Palette.ok) }
                if busy { ProgressView().controlSize(.small) }
                if intent == "login" { HStack {
                    Button(L("读取当前登录账户")) { perform { try await ensureManaged(); let _: Acknowledgement = try await call("adopt", ["accountId":accountID]); try await settingsChanged(); try await refresh() } }
                    Button(account == nil ? L("登录新账户") : L("登录此账户")) { reauthID = nil; loginSheet = true }

                }.disabled(busy || source == nil || operation != nil) }
                if let info { Text(L("{0} · 认证存储 {1} · {2} 个相关进程" , info.version ?? L("CLI 版本未知"), info.storageLabel, info.processes.count)).font(AppFont.secondary).foregroundStyle(.secondary) }
                if let info, info.cliAvailable == false {
                    Text(info.cliError ?? L("Codex CLI 暂不可用")).foregroundStyle(Palette.warn).textSelection(.enabled)
                    if let binary = info.resolvedBinary { Text(L("检查的程序：") + binary).font(AppFont.secondary).textSelection(.enabled) }
                    Button(L("机器与 CLI 设置")) { model.openSettings(.init(provider: "codex", section: "machines")) }
                    Text(L("进程检测与 CLI 可调用性分别检查；已有登录档案仍可关联。")).font(AppFont.secondary).foregroundStyle(.secondary)
                }
            }
            ForEach(visibleProfiles) { profile in
                Section {
                    HStack { Text(profile.name).font(AppFont.section); if profile.current == true { Text(L("当前日常账户")).foregroundStyle(.secondary) } }
                    Text("\(profile.identity.email) · \(profile.identity.plan)").font(AppFont.secondary).textSelection(.enabled)
                    HStack {
                        if intent == "login" && !model.settings.accounts.contains(where: { $0.profileRefs.contains((source?.codexHomeId ?? "") + ":" + profile.id) }) {
                            Button(L("添加此账户")) { perform { let _: Acknowledgement = try await call("profiles.bind", ["profileId":profile.id, "accountId":accountID]); try await settingsChanged(); notice = L("账户已添加") } }
                        }
                        if intent == "login" { Button(L("重新登录")) { reauthID = profile.id; loginSheet = true } }
                        if profile.current != true {
                            if intent == "switch" || targetAccountID != nil { Button(L("切换账户")) { perform { await prepareSwitch(profile) } } }
                            if intent == "login" { Button(L("移除"), role: .destructive) { removeProfile = profile } }
                        }
                    }.disabled(busy || operation != nil || source == nil)
                }
            }
            if let operation {
                Section(L("等待授权")) {
                    if let code = operation.userCode { HStack { Text(L("设备码：") + code).font(AppFont.section).textSelection(.enabled); Button(L("复制设备码")) { copy(code) } } }
                    if let url = operation.authUrl { HStack { Button(L("打开登录页面")) { perform { let _: Acknowledgement = try await call("openUrl", ["url":url]) } }; Button(L("复制登录链接")) { copy(url) } } }
                    else { Text(L("正在准备登录…")) }
                    Button(L("取消登录")) { perform { let _: Acknowledgement = try await loginCall("login.cancel", ["operationId":operation.operationId]) } }
                    Text(L("请在浏览器完成授权。登录最多等待 10 分钟。")).foregroundStyle(.secondary)
                }
            }
        }.formStyle(.grouped).aieyesAccent()
        .updateOperation(L("Codex 账户操作"), active: busy)
        .onChange(of: busy) { _, value in onBusyChanged(value) }
        .onChange(of: sources.map(\.id)) { _, _ in if source == nil && operation == nil { sourceID = "" } }
        .task {
            if sourceID.isEmpty { sourceID = initialSourceID ?? sources.first?.id ?? "" }
            if let id = UserDefaults.standard.string(forKey:"codexLogin.operation"), let sid = UserDefaults.standard.string(forKey:"codexLogin.source"), sources.contains(where: { $0.id == sid }) {
                restoredAccountID = UserDefaults.standard.string(forKey: "codexLogin.account") ?? ""; sourceID = sid; operation = CodexOperation(operationId:id,status:"running",authUrl:nil,userCode:nil,error:nil,message:nil,processes:nil)
            }
        }
        .task(id: sourceID) {
            info = nil; prepared = nil; switchSheet = false; switchedProfileID = nil
            if source != nil { perform {
                try await refresh()
                if intent == "switch", targetAccountID != nil,
                   let profile = visibleProfiles.first(where: { $0.current != true && account?.profileRefs.contains((source?.codexHomeId ?? "") + ":" + $0.id) == true }) {
                    await prepareSwitch(profile)
                }
            } }
        }
        .task(id: operation?.operationId) {
            guard let id = operation?.operationId else { return }
            while !Task.isCancelled {
                do {
                    let result: CodexOperation = try await loginCall("login.status", ["operationId":id, "accountId":accountID])
                    if ["succeeded","failed","cancelled"].contains(result.status) {
                        operation = nil; UserDefaults.standard.removeObject(forKey:"codexLogin.operation")
                        if result.status == "succeeded" { notice = result.accountArchived == true ? L("登录已保存，该账户已归档，可在账户页恢复。") : result.quotaEnabled == false ? L("登录成功，该账户的额度查询已暂停。") : L("登录成功，已启用此账户的额度查询。"); try await settingsChanged(); try await refresh() }
                        else { error = result.error ?? L("登录已结束") }; break
                    }
                    operation = result
                } catch { self.error = error.localizedDescription }
                do { try await Task.sleep(for:.seconds(2)) } catch { break }
            }
        }
        .sheet(isPresented: $switchSheet, onDismiss: { prepared = nil; switchTarget = nil; switchError = nil }) { switchConfirmation }
        .sheet(isPresented:$loginSheet) {
            VStack(alignment:.leading, spacing:16) {
                Text(reauthID == nil ? L("添加 Codex 账户") : L("重新登录原账户")).font(AppFont.section)
                Text(L("默认以邮箱命名，可在账户设置中修改。")).foregroundStyle(.secondary)
                if let error { ErrorNotice(message: error).foregroundStyle(Palette.warn) }
                HStack { Button(L("取消")) { loginSheet = false }; Spacer(); Button(L("开始登录")) {
                    perform {
                        try await ensureManaged()
                        var params: [String:Any] = ["deviceCode":source?.hostId != nil]
                        if let reauthID { params["profileId"] = reauthID }
                        let result: CodexOperation = try await loginCall("login.start",params)
                        UserDefaults.standard.set(accountID,forKey:"codexLogin.account"); UserDefaults.standard.set(sourceID,forKey:"codexLogin.source"); UserDefaults.standard.set(result.operationId,forKey:"codexLogin.operation")
                        operation = result; loginSheet = false
                    }
                }.buttonStyle(.aieyesPrimary) }.disabled(busy)
            }.padding(DesignTokens.dialogPadding).frame(width:450).interactiveDismissDisabled(busy)
        }
        .sheet(item:$removeProfile) { profile in
            DangerConfirmation(title:L("移除账户档案？"),explanation:L("删除此位置保存的登录凭据，历史记录保留。"),affected:[profile.name],confirmLabel:L("确认移除"),cancel:{removeProfile = nil},confirm:{removeProfile = nil;perform { let _: Acknowledgement = try await call("profiles.remove",["profileId":profile.id]);try await settingsChanged();try await refresh() }})
        }
    }
    private func call<T: Decodable>(_ method: String, _ parameters: [String:Any] = [:]) async throws -> T {
        try await call(method, parameters, via: engine)
    }
    // The background login worker lives in the engine process that starts it.
    // Route login operations through the app-lifetime engine so closing this
    // view no longer kills an authorization that can be resumed later.
    private func loginCall<T: Decodable>(_ method: String, _ parameters: [String:Any] = [:]) async throws -> T {
        try await call(method, parameters, via: model.accountEngine)
    }
    private func call<T: Decodable>(_ method: String, _ parameters: [String:Any], via client: EngineClient) async throws -> T {
        var params = parameters; params["sourceId"] = sourceID; if params["accountId"] == nil { params["accountId"] = accountID }
        return try await client.call("codexAuth." + method,params:params)
    }
    private func refresh() async throws {
        info = try await call("inspect")
    }
    private var switchConfirmation: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(L("确认切换账户")).font(AppFont.title)
            Text((info?.profiles.first(where: { $0.current == true })?.name ?? L("当前登录")) + " → " + (switchTarget?.name ?? L("目标账户"))).font(AppFont.section)
            if let source { Text(source.name + " · " + source.path).font(AppFont.secondary).textSelection(.enabled) }
            Text(L("请先保存工作。切换将关闭相关登录进程；已确认使用独立 API 凭据的实例及其辅助进程会保留。")).foregroundStyle(Palette.warn)
            if let switchError { ErrorNotice(message: switchError).foregroundStyle(Palette.warn).textSelection(.enabled) }
            if busy { HStack { ProgressView().controlSize(.small); Text(prepared == nil ? L("正在检查…") : L("正在切换…")) } }
            if let prepared {
                if let preserved = prepared.preservedProcesses, !preserved.isEmpty {
                    Text(L("将保留 {0} 个使用独立 API 凭据的进程（含辅助进程）。" , preserved.count)).foregroundStyle(.secondary)
                }
                if (prepared.processes ?? []).contains(where: { !$0.canClose }) {
                    Text(L("暂时无法切换，请先处理以下进程，再重新检查。")).foregroundStyle(Palette.warn)
                } else {
                    Text((prepared.processes ?? []).isEmpty ? L("没有需要关闭的 Codex 或 ChatGPT 进程。") : L("确认后将关闭以下进程及应用。"))
                }
                ScrollView {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(prepared.processes ?? []) { process in
                            VStack(alignment: .leading, spacing: 3) {
                                Text("\(process.name) · PID \(process.pid)")
                                if let parent = process.parentPid { Text(L("父进程 PID {0}" , parent)).font(AppFont.secondary).foregroundStyle(.secondary) }
                                if !process.canClose { Text(process.blockingReason ?? L("无法确认归属，请手动关闭后重新检查")).foregroundStyle(Palette.warn) }
                            }
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }.frame(maxHeight: 210)
            }
            Text(L("确认后也会自动关闭随后出现的相关登录进程；独立 API 实例会保留。"))
            Text(L("切换后请按需手动重新打开 Codex 或 ChatGPT 并恢复会话。")).foregroundStyle(.secondary)
            HStack {
                Button(L("取消")) { switchSheet = false }.keyboardShortcut(.cancelAction)
                Button(L("重新检查")) { if let target = switchTarget { perform { await prepareSwitch(target) } } }
                Spacer()
                Button(L("关闭并切换")) { perform { await commitSwitch() } }
                    .buttonStyle(.aieyesPrimary)
                    .disabled(prepared == nil || (prepared?.processes ?? []).contains { !$0.canClose })
            }.disabled(busy)
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogApiWidth).interactiveDismissDisabled(busy).aieyesAccent()
    }
    private func prepareSwitch(_ profile: CodexProfile) async {
        switchTarget = profile; switchSourceID = sourceID; switchError = nil
        switchedProfileID = nil; prepared = nil; switchSheet = true
        do {
            let result: CodexOperation = try await call("switch.prepare", ["profileId": profile.id])
            guard switchSheet, switchSourceID == sourceID else { return }
            prepared = result
        } catch { switchError = error.localizedDescription }
    }
    private func commitSwitch() async {
        guard let prepared, let target = switchTarget, switchSourceID == sourceID else { return }
        do {
            let result: CodexOperation = try await call("switch.commit", ["operationId":prepared.operationId, "closeProcesses":true])
            guard result.status == "succeeded" else { throw ClientError.message(result.error ?? L("切换尚未完成，请重新检查")) }
            self.prepared = nil; switchedProfileID = target.id; switchSheet = false
            switchSettingsPending = result.accountId != nil; switchWarning = result.statusWarning
            await refreshAfterSwitch(profileID: target.id)
        } catch {
            switchError = error.localizedDescription
            self.prepared = nil
        }
    }
    private func refreshAfterSwitch(profileID: String) async {
        do {
            if switchSettingsPending { try await settingsChanged(); switchSettingsPending = false }
            try await refresh()
            let current = info?.profiles.first { $0.id == profileID }?.current == true
            let updated = await model.refreshCodexAccountStatuses(sourceID: sourceID, using: engine)
            if current {
                notice = L("账户已切换，请按需重新打开 Codex 或 ChatGPT 并恢复会话")
                if !updated { error = L("账户已切换，设备状态刷新失败，请重试读取。") }
                else if let switchWarning { error = switchWarning }
            } else {
                notice = nil; error = L("当前登录与目标账户不一致，请重新检查。")
            }
        } catch {
            notice = L("切换已提交，当前账户待确认。")
            self.error = L("状态读取失败：") + L(error.localizedDescription)
        }
    }

    private func settingsChanged() async throws {
        let saved: Settings = try await engine.call("settings.get")
        model.acceptConfiguration(saved)
    }
    private func ensureManaged() async throws {
        if source?.codexHomeId == nil { let _: Acknowledgement = try await call("enable"); try await settingsChanged() }
    }
    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        if NSPasteboard.general.setString(text, forType: .string) { notice = L("已复制") } else { error = L("复制失败，请重试") }
    }
    private func perform(_ action: @escaping @MainActor () async throws -> Void) {
        guard !busy else { return }; busy = true; error = nil; notice = nil
        Task { @MainActor in defer { busy = false }; do { try await action() } catch { self.error = error.localizedDescription } }
    }
}
