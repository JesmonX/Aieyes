import SwiftUI

struct CodexProfile: Decodable, Identifiable {
    struct Identity: Decodable { var key: String, workspace: String, email: String, plan: String }
    var id: String, name: String, identity: Identity, current: Bool?
}
struct CodexProcess: Decodable, Identifiable { var pid: Int, name: String, canClose: Bool; var parentPid: Int?, blockingReason: String?; var id: Int { pid } }
struct CodexInspection: Decodable { var profiles: [CodexProfile], processes: [CodexProcess], version: String?, storageMode: String }
struct CodexOperation: Decodable, Identifiable {
    var operationId: String, status: String, authUrl: String?, userCode: String?, error: String?, message: String?, processes: [CodexProcess]?
    var accountArchived: Bool?, quotaEnabled: Bool?
    var accountId: String?, statusWarning: String?
    var preservedProcesses: [CodexProcess]?
    var id: String { operationId }
}

struct CodexAccountsView: View {
    @ObservedObject var model: AppModel
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
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == "codex" && $0.enabled } }
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
    var body: some View {
        Form {
            Section(intent == "switch" ? "切换账户" : "Codex 登录") {
                Text(intent == "switch" ? "选择目标账号后，检查并确认切换。" : account.map { "在所选机器登录「" + $0.name + "」，请使用同一个账号。" } ?? "登录或读取成功后自动添加账号，以邮箱命名；已添加的同一账号会自动识别。").foregroundStyle(.secondary)
                Text("切换会改变机器当前使用的账号。").foregroundStyle(.secondary)
                Picker("运行位置", selection: $sourceID) { Text("请选择来源").tag(""); ForEach(sources) { Text($0.name + ($0.hostId == nil ? " · 本机" : " · SSH")).tag($0.id) } }.disabled(busy || operation != nil)
                if let source { Text(source.path).font(AppFont.secondary).textSelection(.enabled) }
                if let error { HStack { Text(error).foregroundStyle(Palette.warn).textSelection(.enabled); Button("重试读取") { perform { if let id = switchedProfileID { await refreshAfterSwitch(profileID: id) } else { try await refresh() } } }.disabled(busy) } }
                if let notice { Text(notice).foregroundStyle(Palette.ok) }
                if busy { ProgressView().controlSize(.small) }
                if intent == "login" { HStack {
                    Button("读取当前登录账号") { perform { try await ensureManaged(); let _: Acknowledgement = try await call("adopt", ["accountId":accountID]); try await settingsChanged(); try await refresh() } }
                    Button(account == nil ? "登录新账号" : "登录此账号") { reauthID = nil; loginSheet = true }

                }.disabled(busy || model.settingsDirty || source == nil || operation != nil) }
                if let info { Text("\(info.version ?? "CLI 版本未知") · 认证存储 \(info.storageMode) · \(info.processes.count) 个相关进程").font(AppFont.secondary).foregroundStyle(.secondary) }
            }
            ForEach(visibleProfiles) { profile in
                Section {
                    HStack { Text(profile.name).font(AppFont.section); if profile.current == true { Text("当前日常账号").foregroundStyle(.secondary) } }
                    Text("\(profile.identity.email) · \(profile.identity.plan)").font(AppFont.secondary).textSelection(.enabled)
                    HStack {
                        if intent == "login" && !model.settings.accounts.contains(where: { $0.profileRefs.contains((source?.codexHomeId ?? "") + ":" + profile.id) }) {
                            Button("添加此账号") { perform { let _: Acknowledgement = try await call("profiles.bind", ["profileId":profile.id, "accountId":accountID]); try await settingsChanged(); notice = "账号已添加" } }
                        }
                        if intent == "login" { Button("重新登录") { reauthID = profile.id; loginSheet = true } }
                        if profile.current != true {
                            if intent == "switch" { Button("切换到此账户") { perform { await prepareSwitch(profile) } } }
                            if intent == "login" { Button("移除", role: .destructive) { removeProfile = profile } }
                        }
                    }.disabled(busy || model.settingsDirty || operation != nil)
                }
            }
            if let operation {
                Section("等待授权") {
                    if let code = operation.userCode { HStack { Text("设备码：" + code).font(AppFont.section).textSelection(.enabled); Button("复制设备码") { copy(code) } } }
                    if let url = operation.authUrl { HStack { Button("打开登录页面") { perform { let _: Acknowledgement = try await call("openUrl", ["url":url]) } }; Button("复制登录链接") { copy(url) } } }
                    else { Text("正在准备登录…") }
                    Button("取消登录") { perform { let _: Acknowledgement = try await call("login.cancel", ["operationId":operation.operationId]) } }
                    Text("请在浏览器完成授权。登录最多等待 10 分钟。").foregroundStyle(.secondary)
                }
            }
        }.formStyle(.grouped).aieyesAccent()
        .updateOperation("Codex 账户操作", active: busy)
        .onChange(of: busy) { _, value in onBusyChanged(value) }
        .task {
            if sourceID.isEmpty { sourceID = initialSourceID ?? sources.first?.id ?? "" }
            if let id = UserDefaults.standard.string(forKey:"codexLogin.operation"), let sid = UserDefaults.standard.string(forKey:"codexLogin.source"), sources.contains(where: { $0.id == sid }) {
                restoredAccountID = UserDefaults.standard.string(forKey: "codexLogin.account") ?? ""; sourceID = sid; operation = CodexOperation(operationId:id,status:"running",authUrl:nil,userCode:nil,error:nil,message:nil,processes:nil)
            }
        }
        .task(id: sourceID) {
            info = nil; prepared = nil; switchSheet = false; switchedProfileID = nil
            if !sourceID.isEmpty { perform {
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
                    let result: CodexOperation = try await call("login.status", ["operationId":id, "accountId":accountID])
                    if ["succeeded","failed","cancelled"].contains(result.status) {
                        operation = nil; UserDefaults.standard.removeObject(forKey:"codexLogin.operation")
                        if result.status == "succeeded" { notice = result.accountArchived == true ? "登录已保存，该账户已归档，可在账户页恢复。" : result.quotaEnabled == false ? "登录成功，该账户的额度查询已暂停。" : "登录成功，已启用此账号的额度查询。"; try await settingsChanged(); try await refresh() }
                        else { error = result.error ?? "登录已结束" }; break
                    }
                    operation = result
                } catch { self.error = error.localizedDescription }
                do { try await Task.sleep(for:.seconds(2)) } catch { break }
            }
        }
        .sheet(isPresented: $switchSheet, onDismiss: { prepared = nil; switchTarget = nil; switchError = nil }) { switchConfirmation }
        .sheet(isPresented:$loginSheet) {
            VStack(alignment:.leading, spacing:16) {
                Text(reauthID == nil ? "添加 Codex 账号" : "重新登录原账号").font(AppFont.section)
                Text("登录后自动以邮箱命名，可在账户设置中修改名称。").foregroundStyle(.secondary)
                if let error { Text(error).foregroundStyle(Palette.warn) }
                HStack { Button("取消") { loginSheet = false }; Spacer(); Button("开始登录") {
                    perform {
                        try await ensureManaged()
                        var params: [String:Any] = ["deviceCode":source?.hostId != nil]
                        if let reauthID { params["profileId"] = reauthID }
                        let result: CodexOperation = try await call("login.start",params)
                        UserDefaults.standard.set(accountID,forKey:"codexLogin.account"); UserDefaults.standard.set(sourceID,forKey:"codexLogin.source"); UserDefaults.standard.set(result.operationId,forKey:"codexLogin.operation")
                        operation = result; loginSheet = false
                    }
                }.buttonStyle(.borderedProminent) }.disabled(busy)
            }.padding(24).frame(width:450).interactiveDismissDisabled(busy)
        }
        .sheet(item:$removeProfile) { profile in
            DangerConfirmation(title:"移除账号档案？",explanation:"删除此位置保存的登录凭据，历史记录保留。",affected:[profile.name],confirmLabel:"确认移除",cancel:{removeProfile = nil},confirm:{removeProfile = nil;perform { let _: Acknowledgement = try await call("profiles.remove",["profileId":profile.id]);try await settingsChanged();try await refresh() }})
        }
    }
    private func call<T: Decodable>(_ method: String, _ parameters: [String:Any] = [:]) async throws -> T {
        var params = parameters; params["sourceId"] = sourceID; if params["accountId"] == nil { params["accountId"] = accountID }
        return try await engine.call("codexAuth." + method,params:params)
    }
    private func refresh() async throws {
        info = try await call("inspect")
    }
    private var switchConfirmation: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("确认切换账号").font(AppFont.title)
            Text((info?.profiles.first(where: { $0.current == true })?.name ?? "当前登录") + " → " + (switchTarget?.name ?? "目标账号")).font(AppFont.section)
            if let source { Text(source.name + " · " + source.path).font(AppFont.secondary).textSelection(.enabled) }
            Text("切换账号将关闭此设备上使用登录账号的 Codex 和 ChatGPT 进程及应用；已确认使用独立 API 凭据的实例及其辅助进程会保留。请先保存工作。").foregroundStyle(Palette.warn)
            if let switchError { Text(switchError).foregroundStyle(Palette.warn).textSelection(.enabled) }
            if busy { HStack { ProgressView().controlSize(.small); Text(prepared == nil ? "正在检查…" : "正在切换…") } }
            if let prepared {
                if let preserved = prepared.preservedProcesses, !preserved.isEmpty {
                    Text("将保留 \(preserved.count) 个使用独立 API 凭据的进程（含辅助进程）。").foregroundStyle(.secondary)
                }
                if (prepared.processes ?? []).contains(where: { !$0.canClose }) {
                    Text("暂时无法切换，请先处理以下进程，再重新检查。").foregroundStyle(Palette.warn)
                } else {
                    Text((prepared.processes ?? []).isEmpty ? "没有需要关闭的 Codex 或 ChatGPT 进程。" : "确认后将关闭以下进程及应用。")
                }
                ScrollView {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(prepared.processes ?? []) { process in
                            VStack(alignment: .leading, spacing: 3) {
                                Text("\(process.name) · PID \(process.pid)")
                                if let parent = process.parentPid { Text("父进程 PID \(parent)").font(AppFont.secondary).foregroundStyle(.secondary) }
                                if !process.canClose { Text(process.blockingReason ?? "无法确认归属，请手动关闭后重新检查").foregroundStyle(Palette.warn) }
                            }
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }.frame(maxHeight: 210)
            }
            Text("切换后请按需手动重新打开 Codex 或 ChatGPT 并恢复会话。").foregroundStyle(.secondary)
            HStack {
                Button("取消") { switchSheet = false }.keyboardShortcut(.cancelAction)
                Button("重新检查") { if let target = switchTarget { perform { await prepareSwitch(target) } } }
                Spacer()
                Button((prepared?.processes ?? []).isEmpty ? "确认切换" : "关闭并切换") { perform { await commitSwitch() } }
                    .buttonStyle(.borderedProminent)
                    .disabled(prepared == nil || (prepared?.processes ?? []).contains { !$0.canClose })
            }.disabled(busy || model.settingsDirty)
        }.padding(24).frame(width: 530).interactiveDismissDisabled(busy).aieyesAccent()
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
            let result: CodexOperation = try await call("switch.commit", ["operationId":prepared.operationId, "closeProcesses":!(prepared.processes ?? []).isEmpty])
            guard result.status == "succeeded" else { throw ClientError.message(result.error ?? "切换尚未完成，请重新检查") }
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
                notice = "账号已切换，请按需重新打开 Codex 或 ChatGPT 并恢复会话"
                if !updated { error = "账号已切换，设备状态刷新失败，请重试读取。" }
                else if let switchWarning { error = switchWarning }
            } else {
                notice = nil; error = "当前登录与目标账号不一致，可能已被其他程序修改，请重新检查。"
            }
        } catch {
            notice = "切换已提交，当前账号待确认。"
            self.error = "状态读取失败：" + error.localizedDescription
        }
    }

    private func settingsChanged() async throws {
        let saved: Settings = try await engine.call("settings.get")
        let newAccounts = saved.accounts.filter { a in !model.settings.accounts.contains { $0.key == a.key } }
        for account in newAccounts { PanelAccountPreference.includeNew(account, accounts: saved.accounts) }
        model.acceptConfiguration(saved)
    }
    private func ensureManaged() async throws {
        if source?.codexHomeId == nil { let _: Acknowledgement = try await call("enable"); try await settingsChanged() }
    }
    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        if NSPasteboard.general.setString(text, forType: .string) { notice = "已复制" } else { error = "复制失败，请重试" }
    }
    private func perform(_ action: @escaping @MainActor () async throws -> Void) {
        guard !busy else { return }; busy = true; error = nil; notice = nil
        Task { @MainActor in defer { busy = false }; do { try await action() } catch { self.error = error.localizedDescription } }
    }
}
