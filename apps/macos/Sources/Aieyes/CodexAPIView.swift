import SwiftUI
import AppKit

private struct CodexAPIStatus: Decodable {
    var configRevision: String
    var defaultProvider: String?, installed: Bool?, ready: Bool?
}
private struct CodexAPICommand: Decodable, Identifiable {
    var launchCommand: String
    var resumeCommand: String
    var id: String { launchCommand }
}
struct CodexAPISettingsSection: View {
    @ObservedObject var model: AppModel
    @State private var editing: CodexAPIConnection?
    @State private var removing: CodexAPIConnection?
    @State private var command: CodexAPICommand?
    @State private var statuses: [String: CodexAPIStatus] = [:]
    @State private var busy = false
    @State private var message: String?
    private var connections: [CodexAPIConnection] { model.settings.codexApis ?? [] }
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == "codex" } }
    var body: some View {
        if !sources.isEmpty || !connections.isEmpty {
            Section("Codex · API 连接") {
                Text("支持官方 OpenAI 与兼容 Responses API 的接口。API 会话可与订阅同时运行，并共用对话历史。").font(AppFont.secondary).foregroundStyle(.secondary)
                ForEach(connections) { connection in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            VStack(alignment: .leading) {
                                Text(connection.name + (isDefault(connection) ? " · 默认 API" : ""))
                                Text(location(connection.sourceId) + " · " + connection.model).font(AppFont.secondary).foregroundStyle(.secondary)
                                Text(connection.baseUrl).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled)
                            }
                            Spacer()
                            Button("设置") { editing = connection }
                        }
                        HStack {
                            Button("测试连接") { perform { try await probe(connection) } }
                            Button("启动 / 续聊命令") { perform {
                                command = try await model.engine.call("codexApi.command", params: ["sourceId":connection.sourceId,"id":connection.id])
                            } }
                            Button("设为默认") { perform { try await setDefault(connection.sourceId, connection: connection) } }.disabled(isDefault(connection))
                            Button("移除") { removing = connection }
                        }
                    }
                }
                ForEach(Array(Set(connections.map(\.sourceId))).sorted(), id: \.self) { sourceID in
                    HStack {
                        Text(location(sourceID) + " · " + (statuses[sourceID]?.ready == true ? "计量服务可用" : "服务待检查")).font(AppFont.secondary).foregroundStyle(.secondary)
                        Spacer()
                        Button("检查服务") { perform { try await refresh(sourceID) } }
                        Button("切回订阅") { perform { try await setDefault(sourceID, connection: nil) } }
                            .disabled(statuses[sourceID]?.defaultProvider?.hasPrefix("aieyes_api_") != true)
                    }
                }
                Button("添加 API", systemImage: "plus") { var c = CodexAPIConnection(); c.sourceId = sources.first?.id ?? ""; editing = c }.disabled(sources.isEmpty)
                if connections.isEmpty { Text("密钥保存在所选设备，API 费用与订阅估值分开计算。").font(AppFont.secondary).foregroundStyle(.secondary) }
                if let message { Text(message).font(AppFont.secondary).textSelection(.enabled) }
            }.disabled(busy)
                .task { for id in Set(connections.map(\.sourceId)) { try? await refresh(id) } }
                .sheet(item: $editing) { connection in
                    CodexAPIEditor(model: model, connection: connection) { editing = nil; Task { try? await refresh(connection.sourceId) } }
                }
                .sheet(item: $command) { value in
                    VStack(alignment: .leading, spacing: 16) {
                        Text("启动 API 会话").font(AppFont.title)
                        Text("在所选设备的终端执行。续聊前先结束当前轮次，再执行续聊命令并选择原对话。").foregroundStyle(.secondary)
                        ScrollView { Text(value.launchCommand).font(.system(.body, design: .monospaced)).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }.frame(maxHeight: 160)
                        ScrollView { Text(value.resumeCommand).font(.system(.body, design: .monospaced)).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }.frame(maxHeight: 160)
                        HStack { Spacer(); Button("完成") { command = nil }; Button("复制启动命令") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(value.launchCommand, forType: .string) }; Button("复制续聊命令") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(value.resumeCommand, forType: .string) } }
                    }.padding(24).frame(width: 580).aieyesAccent()
                }
                .sheet(item: $removing) { connection in
                    DangerConfirmation(title: "移除 API 连接？", explanation: "移除设备上的连接与密钥。正在使用此 API 的会话会失去连接，历史用量与费用记录保留。", affected: [connection.name], confirmLabel: "移除连接", cancel: { removing = nil }, confirm: {
                        removing = nil
                        perform {
                            struct Saved: Decodable { var settings: Settings }
                            let result: Saved = try await model.engine.call("codexApi.remove", params: ["sourceId":connection.sourceId,"id":connection.id,"revision":connection.revision])
                            model.acceptConfiguration(result.settings); statuses.removeValue(forKey: connection.sourceId)
                        }
                    })
                }
                .updateOperation("Codex API 配置", active: busy)
        }
    }
    private func location(_ id: String) -> String {
        guard let source = sources.first(where: { $0.id == id }) else { return "来源已移除" }
        return model.machineName(source.hostId ?? "local") + " · " + source.name
    }
    private func isDefault(_ c: CodexAPIConnection) -> Bool { statuses[c.sourceId]?.defaultProvider?.hasPrefix("aieyes_api_" + c.id + "_") == true }
    private func refresh(_ id: String) async throws { statuses[id] = try await model.engine.call("codexApi.status", params: ["sourceId":id]) }
    private func setDefault(_ id: String, connection: CodexAPIConnection?) async throws {
        try await refresh(id)
        let _: CodexAPIStatus = try await model.engine.call("codexApi.default", params: ["sourceId":id,"id":connection?.id ?? "subscription","revision":connection?.revision ?? "","configRevision":statuses[id]!.configRevision])
        try await refresh(id); message = "默认方式已更新，将用于后续启动或恢复的会话。已有会话继续使用原方式。"
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

private struct APIPriceDraft: Identifiable, Equatable, Encodable {
    var id = UUID(), model = "", input = "", output = "", cached = "", write = ""
    init(_ price: ModelPrice) {
        model = price.id
        func text(_ value: Double?) -> String { value.map { String($0 * 1_000_000) } ?? "" }
        input = text(price.input); output = text(price.output); cached = text(price.cacheRead); write = text(price.cacheWrite)
    }
    var fields: [String] { [model,input,output,cached,write] }
    func price() throws -> ModelPrice? {
        if fields.allSatisfy({ $0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) { return nil }
        func number(_ text: String) throws -> Double? {
            if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return nil }
            guard let value = Double(text), value.isFinite, value >= 0 else { throw NSError(domain: "Aieyes", code: 1, userInfo: [NSLocalizedDescriptionKey:"价格必须为非负数"]) }
            return value / 1_000_000
        }
        guard !model.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw NSError(domain: "Aieyes", code: 1, userInfo: [NSLocalizedDescriptionKey:"请填写价格对应的模型 ID，或移除该行"]) }
        return try ModelPrice(id:model.trimmingCharacters(in: .whitespacesAndNewlines),name:model,input:number(input),output:number(output),cacheRead:number(cached),cacheWrite:number(write),fetchedAt:Double(Int(Date().timeIntervalSince1970)))
    }
}
struct CodexAPIEditor: View {
    @ObservedObject var model: AppModel
    let connection: CodexAPIConnection
    var onSaved: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var draft: CodexAPIConnection
    @State private var prices: [APIPriceDraft]
    @State private var apiKey = ""
    @State private var busy = false
    @State private var revision: String?
    @State private var error: String?
    init(model: AppModel, connection: CodexAPIConnection, onSaved: @escaping () -> Void) {
        self.model = model; self.connection = connection; self.onSaved = onSaved
        _draft = State(initialValue: connection)
        _prices = State(initialValue: (connection.prices.isEmpty ? [ModelPrice(id:connection.model)] : connection.prices).map(APIPriceDraft.init))
    }
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == "codex" } }
    private var dirty: Bool { draft != connection || !apiKey.isEmpty || prices.map(\.fields) != (connection.prices.isEmpty ? [ModelPrice(id:connection.model)] : connection.prices).map(APIPriceDraft.init).map(\.fields) }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text(connection.id.isEmpty ? "添加 Codex API" : "设置 Codex API").font(AppFont.title); Spacer() }.padding()
            Form {
                Section {
                    Picker("保存到设备", selection: $draft.sourceId) {
                        ForEach(sources) { source in Text(model.machineName(source.hostId ?? "local") + " · " + source.name).tag(source.id) }
                    }.disabled(!connection.id.isEmpty)
                    TextField("连接名称", text: $draft.name)
                    TextField("Base URL", text: $draft.baseUrl)
                    SecureField(connection.id.isEmpty ? "API Key" : "API Key（留空保留）", text: $apiKey)
                    TextField("模型 ID", text: $draft.model)
                    Picker("推理强度", selection: $draft.effort) {
                        Text("沿用默认").tag("")
                        ForEach(["none","minimal","low","medium","high","xhigh","max","ultra"], id: \.self) { Text($0).tag($0) }
                    }
                    Picker("上游连接方式", selection: $draft.proxy.mode) { Text("目标设备系统代理").tag("system"); Text("直连").tag("direct"); Text("指定代理").tag("custom") }
                    if draft.proxy.mode == "custom" { TextField("代理地址", text: $draft.proxy.url); Text("远程设备支持 HTTP(S) 代理。").font(AppFont.secondary).foregroundStyle(.secondary) }
                }
                Section("API 价格 · USD / 百万 Token") {
                    Text("只用于此 API 的成本估算。留空会显示未计价用量。").font(AppFont.secondary).foregroundStyle(.secondary)
                    ForEach($prices) { $price in
                        VStack(alignment: .leading) {
                            TextField("模型 ID", text: $price.model)
                            HStack { TextField("输入", text: $price.input); TextField("输出", text: $price.output) }
                            HStack { TextField("缓存读取", text: $price.cached); TextField("缓存写入", text: $price.write) }
                            Button("移除此价格") { prices.removeAll { $0.id == price.id } }
                        }
                    }
                    Button("添加模型价格") { prices.append(APIPriceDraft(ModelPrice(id:draft.model))) }
                }
                Section {
                    Text(revision == nil ? "正在读取设备配置…" : "保存后，计量服务可在退出 Aieyes 后继续运行。").font(AppFont.secondary).foregroundStyle(.secondary)
                    Button("重新读取设备状态") { Task { await readStatus() } }
                    if let error { Text(error).foregroundStyle(Palette.warn).textSelection(.enabled) }
                }
            }.formStyle(.grouped).disabled(busy)
            HStack { Spacer(); Button("取消") { dismiss() }.keyboardShortcut(.cancelAction); Button(busy ? "部署中…" : "保存并部署") { Task { _ = await save() } }.buttonStyle(.borderedProminent).disabled(busy || revision == nil) }.padding()
        }.frame(width: 630, height: 670).aieyesAccent().interactiveDismissDisabled(busy)
            .task(id: draft.sourceId) { await readStatus() }
            .onChange(of: draft.model) { _, value in if prices.count == 1 && prices[0].model.isEmpty { prices[0].model = value } }
            .onDisappear { apiKey = "" }
            .updateOperation("Codex API 部署", active: busy)
            .updateDraftGuard("Codex API 连接", snapshot: draftSnapshot(draft) + draftSnapshot(prices) + String(apiKey.hashValue), dirty: dirty, saving: busy, save: { await save() }, discard: { apiKey = ""; dismiss() })
    }
    private func readStatus() async {
        let id = draft.sourceId; revision = nil
        do { let status: CodexAPIStatus = try await model.engine.call("codexApi.status", params: ["sourceId":id]); if draft.sourceId == id { revision = status.configRevision; error = nil } }
        catch { if draft.sourceId == id { self.error = error.localizedDescription } }
    }
    private func save() async -> Bool {
        guard !busy, let revision else { return false }; busy = true; error = nil; defer { busy = false }
        do {
            var value = draft; value.prices = try prices.compactMap { try $0.price() }
            struct Saved: Decodable { var settings: Settings }
            let result: Saved = try await model.engine.call("codexApi.save", params: ["connection":try JSONSerialization.jsonObject(with: JSONEncoder().encode(value)),"apiKey":apiKey,"configRevision":revision])
            apiKey = ""; model.acceptConfiguration(result.settings); onSaved(); Task { await model.reload() }; return true
        } catch { self.error = error.localizedDescription; return false }
    }
}

struct CodexBillingSummaryView: View {
    let billing: CodexBillingSummary
    var compact = false
    var body: some View {
        Group {
            if compact { DisclosureGroup("Codex · API 估算成本 " + cost(billing.apiCost, billing.apiTokens, billing.apiPricedTokens)) { details.padding(.top, 8) } }
            else { VStack(alignment: .leading, spacing: 10) { Text("Codex 计费分项").font(AppFont.section); details } }
        }.padding(14).modifier(NeutralCard())
    }
    private var details: some View {
        VStack(alignment: .leading, spacing: 8) {
            row("API 估算成本", cost(billing.apiCost,billing.apiTokens,billing.apiPricedTokens), coverage(billing.apiTokens,billing.apiPricedTokens))
            row("订阅 API 等价估值", cost(billing.subscriptionEquivalentCost,billing.subscriptionTokens,billing.subscriptionPricedTokens), coverage(billing.subscriptionTokens,billing.subscriptionPricedTokens))
            row("未归属用量", Format.compact(billing.unknownTokens ?? 0) + " Token", "不参与混用订阅估值")
            if (billing.incompleteRequests ?? 0) > 0 || (billing.apiUnmeteredTokens ?? 0) > 0 {
                Text("有 \(billing.incompleteRequests ?? 0) 个不完整请求、\(Format.compact(billing.apiUnmeteredTokens ?? 0)) 个未计量 API Token；费用可能不完整。").font(AppFont.secondary).foregroundStyle(Palette.warn)
            }
        }
    }
    private func row(_ title: String, _ value: String, _ detail: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack { Text(title); Spacer(); Text(value).monospacedDigit() }
            Text(detail).font(AppFont.secondary).foregroundStyle(.secondary)
        }
    }
    private func cost(_ value: Double?, _ tokens: Double?, _ priced: Double?) -> String { (tokens ?? 0) > 0 && (priced ?? 0) == 0 ? "—" : Format.money(value ?? 0) + " USD" }
    private func coverage(_ tokens: Double?, _ priced: Double?) -> String { Format.compact(priced ?? 0) + " / " + Format.compact(tokens ?? 0) + " Token 已计价" }
}
