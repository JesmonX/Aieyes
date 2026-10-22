import SwiftUI

private struct NativeAPIConfig: Decodable {
    var configPath: String
    var revision: String
    var baseUrl: String
    var model: String
    var hasApiKey: Bool
    var noKey: Bool
    var contextWindow: Int?
    var maxOutputTokens: Int?
    var nativeRevision: String?
    var authMode: String?
    var effort: String?
}

struct NativeAPISettingsSection: View {
    @ObservedObject var model: AppModel
    let provider: String
    private var name: String { Format.provider(provider) }
    @State private var editing: AgentSource?
    @State private var message: String?
    private var sources: [AgentSource] { model.settings.sources.filter { $0.provider == provider && $0.hostId == nil && $0.subscription == nil && !$0.unbound } }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 10) {
            Text(L("验证 {0} 端点后，保存到本机 {1} 配置并设为默认模型。已有配置会备份，Key 保存在原生配置文件中。" , provider == "claude" ? "Anthropic Messages" : "Chat Completions", name)).font(AppFont.secondary).foregroundStyle(.secondary)
            if ["pi", "omp"].contains(provider) { Text(L("额度由当前模型服务商提供；本地 Token 和预估费用不代表可用余额。")).font(AppFont.secondary).foregroundStyle(.secondary) }
            ForEach(sources) { source in Button(source.name + L(" · 配置 API")) { editing = source } }
            if sources.isEmpty { Text(L("先在机器与目录中添加本机数据目录。")).font(AppFont.secondary).foregroundStyle(.secondary) }
            if let message { Text(L(message)).font(AppFont.secondary).foregroundStyle(.secondary) }
        }.sheet(item: $editing) { source in NativeAPIEditor(model: model, source: source) { message = L("API 已验证并保存，可重新打开 {0} 使用。" , name) } }
    }
}

private struct NativeAPIEditor: View {
    @ObservedObject var model: AppModel
    let source: AgentSource
    let saved: () -> Void
    private var name: String { Format.provider(source.provider) }
    @Environment(\.dismiss) private var dismiss
    @State private var loaded: NativeAPIConfig?
    @State private var configPath = ""
    @State private var baseUrl = ""
    @State private var modelID = ""
    @State private var contextWindow = ""
    @State private var maxOutputTokens = ""
    @State private var apiKey = ""
    @State private var noKey = false
    @State private var authMode = "api-key"
    @State private var effort = ""
    @State private var busy = false
    @State private var error: String?
    @State private var message: String?
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 14) {
            HStack { Text(name + L(" · API 连接")).font(AppFont.title); Spacer(); Button(L("完成")) { dismiss() }.disabled(busy) }
            if source.provider == "qoder" { Text(L("此配置目录需先完成 qodercli login；保存通过原生 CLI 的自定义模型管理接口。")).font(AppFont.secondary).foregroundStyle(.secondary) }
            TextField(name + L(" 配置文件"), text: $configPath)
            Button(L("读取此配置")) { Task { await read() } }
            TextField(source.provider == "claude" ? L("API 根地址（不含 /v1）") : L("Base URL（含 /v1）"), text: $baseUrl)
            TextField(L("模型 ID"), text: $modelID)
            if ["kimi", "deepseek-harness", "zcode", "grok", "qoder", "pi", "omp"].contains(source.provider) { TextField(L("模型上下文窗口（Token）"), text: $contextWindow) }
            if ["deepseek-harness", "zcode", "grok", "qoder", "pi", "omp"].contains(source.provider) { TextField(L("最大输出 Token"), text: $maxOutputTokens) }
            if source.provider == "claude" { Picker(L("认证方式"), selection: $authMode) { Text("API Key · X-Api-Key").tag("api-key"); Text("Bearer Token").tag("auth-token") } }
            if ["claude", "zcode", "codebuddy", "pi", "omp"].contains(source.provider) { Picker(L("推理强度"), selection: $effort) { if ["codebuddy", "pi", "omp"].contains(source.provider) { Text(L("不发送推理强度")).tag("disabled") } else { Text(source.provider == "zcode" ? L("服务端默认") : L("客户端默认")).tag("") }; ForEach(source.provider == "codebuddy" ? ["minimal", "low", "medium", "high", "xhigh", "max"] : ["low", "medium", "high", "xhigh", "max"], id: \.self) { Text($0).tag($0) } } }
            if source.provider == "codebuddy" { Text(L("新连接默认使用 low；模型不支持推理参数时，选择不发送推理强度。")).font(AppFont.secondary).foregroundStyle(.secondary) }
            SecureField(source.provider == "claude" ? L("API Key / Token（留空保留）") : L("API Key（留空保留已保存 Key）"), text: $apiKey)
            Toggle(L("服务无需 Key"), isOn: $noKey)
            Text(L("验证成功后保存并设为 {0} 默认模型。配置路径应与客户端实际使用的文件一致；客户端启动参数或环境变量可能覆盖文件设置。" , name)).font(AppFont.secondary).foregroundStyle(.secondary)
            if let message { Text(L(message)).font(AppFont.secondary) }
            if let error { Text(L(error)).foregroundStyle(.red).textSelection(.enabled) }
            Button(busy ? L("处理中…") : L("验证并保存")) { Task { await save() } }.buttonStyle(.aieyesPrimary).disabled(loaded == nil)
        }.padding(24).frame(width: 560).disabled(busy)
            .task { await read() }
            .interactiveDismissDisabled(busy)
            .updateOperation(name + L(" API 配置"), active: busy)
    }
    private func read() async {
        busy = true; error = nil; defer { busy = false }
        do {
            let result: NativeAPIConfig = try await model.engine.call("agentApi.inspect", params: ["sourceId": source.id, "configPath": configPath])
            loaded = result; configPath = result.configPath; baseUrl = result.baseUrl; modelID = result.model; noKey = result.noKey; apiKey = ""
            contextWindow = result.contextWindow.map(String.init) ?? ""
            maxOutputTokens = result.maxOutputTokens.map(String.init) ?? ""
            authMode = result.authMode ?? "api-key"; effort = result.effort.flatMap { $0.isEmpty ? nil : $0 } ?? (["codebuddy", "pi", "omp"].contains(source.provider) ? "low" : "")
            message = result.hasApiKey ? L("已读取，Key 已保存") : L("已读取")
        } catch {
            self.error = error.localizedDescription
            if loaded == nil, let location: NativeAPIConfig = try? await model.engine.call("agentApi.inspect", params: ["sourceId": source.id, "locationOnly": true]) { configPath = location.configPath }
        }
    }
    private func save() async {
        guard let loaded else { return }
        guard configPath == loaded.configPath else { error = L("配置路径已改变，请先读取此配置"); return }
        busy = true; error = nil; defer { busy = false }
        do {
            var params: [String: Any] = ["sourceId": source.id, "configPath": configPath, "revision": loaded.revision, "baseUrl": baseUrl, "model": modelID, "apiKey": apiKey, "noKey": noKey]
            if ["kimi", "deepseek-harness", "zcode", "grok", "qoder", "pi", "omp"].contains(source.provider) {
                guard let context = Int(contextWindow), (1024...16_777_216).contains(context) else { throw ClientError.message(L("请输入有效的模型上下文窗口大小")) }
                params["contextWindow"] = context
            }
            if ["deepseek-harness", "zcode", "grok", "qoder", "pi", "omp"].contains(source.provider) {
                guard let output = Int(maxOutputTokens), output > 0 else { throw ClientError.message(L("请输入有效的最大输出 Token 数")) }
                params["maxOutputTokens"] = output
            }
            if let revision = loaded.nativeRevision { params["nativeRevision"] = revision }
            if ["claude", "zcode", "codebuddy", "pi", "omp"].contains(source.provider) { params["effort"] = effort }
            if source.provider == "claude" { params["authMode"] = authMode }
            let _: NativeAPIConfig = try await model.engine.call("agentApi.save", params: params)
            apiKey = ""; saved(); dismiss()
        } catch { self.error = error.localizedDescription }
    }
}
