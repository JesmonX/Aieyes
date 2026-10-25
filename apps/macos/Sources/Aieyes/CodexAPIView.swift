import SwiftUI

private struct CodexAPIStatus: Decodable {
    var configRevision: String
    var defaultProvider: String?
    var selected: CodexSelectedConnection?
}
private struct APISwitchRequest: Identifiable {
    var sourceID: String
    var connection: CodexAPIConnection?
    var id: String { sourceID + ":" + (connection?.id ?? "account") }
}
struct CodexAPISettingsSection: View {
    @ObservedObject var model: AppModel
    @State private var editing: CodexAPIConnection?
    @State private var removing: CodexAPIConnection?
    @State private var switching: APISwitchRequest?
    @State private var statuses: [String: CodexAPIStatus] = [:]
    @State private var busy = false
    @State private var message: String?
    private var connections: [CodexAPIConnection] { model.settings.codexApis ?? [] }
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == "codex" } }
    private var availableSources: [AgentSource] { sources.filter { model.settings.sourceEnabled($0) } }
    private func available(_ id: String) -> Bool { availableSources.contains { $0.id == id } }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        if !sources.isEmpty || !connections.isEmpty {
            Section(L("Codex · API 连接")) {
                ForEach(connections) { connection in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            VStack(alignment: .leading) {
                                Text(connection.name + (isSelected(connection) ? L(" · 当前 API") : ""))
                                Text(location(connection.sourceId)).font(AppFont.secondary).foregroundStyle(.secondary)
                                Text(connection.baseUrl).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled)
                                if isSelected(connection) && !isApplied(connection) { Text(L("有已保存的更改，切换后生效")).font(AppFont.secondary).foregroundStyle(.secondary) }
                            }
                            Spacer()
                            Button(L("设置")) { editing = connection }.disabled(!available(connection.sourceId))
                        }
                        HStack {
                            Button(L("测试连接")) { perform { try await probe(connection) } }
                            Button(isApplied(connection) ? L("正在使用") : L("切换到此 API")) { switching = APISwitchRequest(sourceID: connection.sourceId, connection: connection) }.disabled(isApplied(connection))
                            Button(L("移除")) { removing = connection }
                        }.disabled(!available(connection.sourceId))
                        if !available(connection.sourceId) { Text(L("请在“机器与采集”恢复此位置的采集或绑定。")).font(AppFont.secondary).foregroundStyle(.secondary) }
                    }
                }
                ForEach(Array(Set(connections.map(\.sourceId))).sorted(), id: \.self) { sourceID in
                    HStack {
                        Text(location(sourceID) + " · " + selectedLabel(sourceID)).font(AppFont.secondary).foregroundStyle(.secondary)
                        Spacer()
                        Button(L("刷新状态")) { perform { try await refresh(sourceID) } }
                        Button(L("切换到已保存账户")) { switching = APISwitchRequest(sourceID: sourceID) }
                    }.disabled(!available(sourceID))
                }
                Button(L("添加 API"), systemImage: "plus") { var c = CodexAPIConnection(); c.sourceId = availableSources.first?.id ?? ""; editing = c }.disabled(availableSources.isEmpty)
                if availableSources.isEmpty {
                    Text(L("暂无可用位置。请启用 Codex、选择机器并开启目录采集。")).font(AppFont.secondary).foregroundStyle(.secondary)
                    Button(L("机器与采集设置")) { model.openSettings(.init(provider: "codex", section: "machines")) }
                }
                if connections.isEmpty { Text(L("密钥保存在所选设备。")).font(AppFont.secondary).foregroundStyle(.secondary) }
                if let message { Text(L(message)).font(AppFont.secondary).textSelection(.enabled) }
            }.disabled(busy)
                .task { for id in Set(connections.map(\.sourceId)) where available(id) { try? await refresh(id) } }
                .sheet(item: $editing) { connection in
                    CodexAPIEditor(model: model, connection: connection) { editing = nil; Task { try? await refresh(connection.sourceId) } }
                }
                .sheet(item: $switching) { request in
                    CodexConnectionSwitchSheet(model: model, request: request) {
                        switching = nil; message = L("登录方式已切换，可重新打开 Codex 并恢复会话。")
                        Task { try? await refresh(request.sourceID); _ = await model.refreshCodexAccountStatuses(sourceID: request.sourceID, using: model.engine); await model.reload() }
                    }
                }
                .sheet(item: $removing) { connection in
                    DangerConfirmation(title: L("移除 API 连接？"), explanation: L("移除设备上的连接与密钥，历史用量保留。"), affected: [connection.name], confirmLabel: L("移除连接"), cancel: { removing = nil }, confirm: {
                        removing = nil
                        perform {
                            struct Saved: Decodable { var settings: Settings }
                            let result: Saved = try await model.engine.call("codexApi.remove", params: ["sourceId":connection.sourceId,"id":connection.id,"revision":connection.revision])
                            model.acceptConfiguration(result.settings); statuses.removeValue(forKey: connection.sourceId)
                        }
                    })
                }
                .updateOperation(L("Codex API 配置"), active: busy)
        }
    }
    private func location(_ id: String) -> String {
        guard let source = sources.first(where: { $0.id == id }) else { return L("来源已移除") }
        return model.machineName(source.hostId ?? "local") + " · " + source.displayLocation
    }
    private func selectedLabel(_ id: String) -> String {
        guard let selected = statuses[id]?.selected else { return L("状态待检查") }
        return selected.kind == "api" ? L("当前使用 ") + (selected.name ?? "API") : L("当前使用账户")
    }
    private func isSelected(_ c: CodexAPIConnection) -> Bool { statuses[c.sourceId]?.selected?.connectionId == c.id }
    private func isApplied(_ c: CodexAPIConnection) -> Bool { isSelected(c) && statuses[c.sourceId]?.selected?.revision == c.revision }
    private func refresh(_ id: String) async throws {
        guard available(id) else { statuses[id] = nil; return }
        statuses[id] = try await model.engine.call("codexApi.status", params: ["sourceId":id])
    }
    private func probe(_ c: CodexAPIConnection) async throws {
        struct Probe: Decodable { var message: String; var httpStatus: Int? }
        let result: Probe = try await model.engine.call("codexApi.probe", params: ["sourceId":c.sourceId,"id":c.id])
        message = result.message + (result.httpStatus.map { " · HTTP \($0)" } ?? "")
    }
    private func perform(_ action: @escaping () async throws -> Void) {
        guard !busy else { return }; busy = true; message = nil
        Task { defer { busy = false }; do { try await action() } catch { message = error.localizedDescription } }
    }
}

struct CodexAPIEditor: View {
    @ObservedObject var model: AppModel
    let connection: CodexAPIConnection
    var onSaved: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var draft: CodexAPIConnection
    @State private var apiKey = ""
    @State private var busy = false
    @State private var error: String?
    init(model: AppModel, connection: CodexAPIConnection, onSaved: @escaping () -> Void) {
        self.model = model; self.connection = connection; self.onSaved = onSaved
        _draft = State(initialValue: connection)
    }
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == "codex" && model.settings.sourceEnabled($0) } }
    private var dirty: Bool { draft != connection || !apiKey.isEmpty }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(spacing: 0) {
            HStack { Text(connection.id.isEmpty ? L("添加 Codex API") : L("设置 Codex API")).font(AppFont.title); Spacer() }.padding()
            Form {
                Picker(L("保存到设备"), selection: $draft.sourceId) {
                    ForEach(sources) { source in Text(model.machineName(source.hostId ?? "local") + " · " + source.displayLocation).tag(source.id) }
                }.disabled(!connection.id.isEmpty)
                TextField(L("连接名称"), text: $draft.name)
                TextField("Base URL", text: $draft.baseUrl)
                SecureField(connection.id.isEmpty ? "API Key" : L("API Key（留空保留）"), text: $apiKey)
                Text(L("支持 OpenAI 和兼容 Responses API 的服务。保存后可切换使用。")).font(AppFont.secondary).foregroundStyle(.secondary)
                if let error { ErrorNotice(message: error) }
            }.formStyle(.grouped).disabled(busy)
            HStack {
                Spacer(); Button(L("取消")) { dismiss() }.keyboardShortcut(.cancelAction).disabled(busy)
                Button(busy ? L("保存中…") : L("保存")) { Task { _ = await save() } }.buttonStyle(.aieyesPrimary).disabled(busy || !sources.contains(where: { $0.id == draft.sourceId }))
            }.padding()
        }.frame(width: DesignTokens.dialogApiWidth, height: 385).aieyesAccent().interactiveDismissDisabled(busy)
            .onDisappear { apiKey = "" }
            .updateOperation(L("Codex API 配置"), active: busy)
            .updateDraftGuard(L("Codex API 连接"), snapshot: draftSnapshot(draft) + String(apiKey.hashValue), dirty: dirty, saving: busy, save: { await save() }, discard: { apiKey = ""; dismiss() })
    }
    private func save() async -> Bool {
        guard !busy else { return false }; busy = true; error = nil; defer { busy = false }
        do {
            struct Saved: Decodable { var settings: Settings }
            let result: Saved = try await model.engine.call("codexApi.save", params: ["connection":try JSONSerialization.jsonObject(with: JSONEncoder().encode(draft)),"apiKey":apiKey])
            apiKey = ""; model.acceptConfiguration(result.settings); onSaved(); Task { await model.reload() }; return true
        } catch { self.error = error.localizedDescription; return false }
    }
}

private struct CodexConnectionSwitchSheet: View {
    @ObservedObject var model: AppModel
    let request: APISwitchRequest
    var completed: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var info: CodexInspection?
    @State private var profileID = ""
    @State private var prepared: CodexOperation?
    @State private var busy = false
    @State private var error: String?
    private var processes: [CodexProcess] { prepared?.processes ?? [] }
    private var available: Bool { model.settings.sources.contains { $0.id == request.sourceID && model.settings.sourceEnabled($0) } }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 16) {
            Text(L("切换 Codex 登录方式")).font(AppFont.title)
            if let connection = request.connection { Text(L("切换到 ") + connection.name).font(AppFont.section) }
            else if let info {
                Picker(L("目标账户"), selection: $profileID) { ForEach(info.profiles) { Text($0.name).tag($0.id) } }
                    .disabled(busy)
                    .onChange(of: profileID) { _, _ in if !busy { prepared = nil; Task { await prepare() } } }
            }
            Text(L("确认后会关闭下面列出的受影响进程。已确认使用独立 API 配置的实例会保留；请先保存工作。")).foregroundStyle(.secondary)
            if let error { ErrorNotice(message: error) }
            if !available { Text(L("请在“机器与采集”恢复采集或绑定后重新检查。")).foregroundStyle(.secondary) }
            if busy { ProgressView().controlSize(.small) }
            if prepared != nil && processes.isEmpty { Text(L("没有需要关闭的进程。")) }
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(processes) { process in
                        Text("\(process.name) · PID \(process.pid)")
                        if !process.canClose { Text(process.blockingReason ?? L("无法确认进程归属，请手动处理后重新检查")).foregroundStyle(Palette.warn) }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxHeight: 220)
            Text(L("确认后也会自动关闭随后出现的相关登录进程；独立 API 实例会保留。")).font(AppFont.secondary).foregroundStyle(.secondary)
            if let preserved = prepared?.preservedProcesses, !preserved.isEmpty { Text(L("将保留 {0} 个独立 API 进程。" , preserved.count)).font(AppFont.secondary).foregroundStyle(.secondary) }
            HStack {
                Button(L("取消")) { dismiss() }.keyboardShortcut(.cancelAction)
                Button(L("重新检查")) { Task { await prepare() } }.disabled(!available)
                Spacer()
                Button(L("关闭并切换")) { Task { await commit() } }
                    .buttonStyle(.aieyesPrimary).disabled(!available || prepared == nil || processes.contains { !$0.canClose })
            }.disabled(busy)
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogApiWidth).aieyesAccent().interactiveDismissDisabled(busy)
            .task { await prepare() }
            .updateOperation(L("Codex 登录切换"), active: busy)
    }
    private func prepare() async {
        guard !busy, available else { return }; busy = true; error = nil; prepared = nil; defer { busy = false }
        do {
            if model.settings.sources.first(where: { $0.id == request.sourceID })?.codexHomeId == nil {
                let _: Acknowledgement = try await model.engine.call("codexAuth.enable", params: ["sourceId":request.sourceID])
                let saved: Settings = try await model.engine.call("settings.get"); model.acceptConfiguration(saved)
            }
            let inspection: CodexInspection = try await model.engine.call("codexAuth.inspect", params: ["sourceId":request.sourceID]); info = inspection
            var target: [String: Any]
            if let c = request.connection { target = ["kind":"api","connectionId":c.id,"revision":c.revision] }
            else {
                if profileID.isEmpty { profileID = inspection.profiles.first(where: { $0.identity.key == inspection.storedIdentity })?.id ?? inspection.profiles.first?.id ?? "" }
                guard !profileID.isEmpty else { throw ClientError.message(L("请先在此机器读取当前登录或登录账户。")) }
                target = ["kind":"account","profileId":profileID]
            }
            prepared = try await model.engine.call("codexAuth.switch.prepare", params: ["sourceId":request.sourceID,"target":target])
        } catch { self.error = error.localizedDescription }
    }
    private func commit() async {
        guard !busy, available, let prepared else { return }; busy = true; error = nil; defer { busy = false }
        do {
            let result: CodexOperation = try await model.engine.call("codexAuth.switch.commit", params: ["sourceId":request.sourceID,"operationId":prepared.operationId,"closeProcesses":true])
            guard result.status == "succeeded" else { throw ClientError.message(result.error ?? L("切换尚未完成，请重新检查")) }
            if let saved: Settings = try? await model.engine.call("settings.get") { model.acceptConfiguration(saved) }
            completed()
        } catch { self.error = error.localizedDescription; self.prepared = nil }
    }
}
