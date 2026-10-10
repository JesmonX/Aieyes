import SwiftUI

struct AntigravityLoginView: View {
    @ObservedObject var model: AppModel
    let source: AgentSource
    @Environment(\.dismiss) private var dismiss
    @State private var engine = EngineClient(configurationOnly: true)
    @State private var status: AntigravityLoginState?
    @State private var code = ""
    @State private var error: String?
    @State private var sending = false
    @State private var refreshedSession: String?
    private var active: Bool { status.map { ["checking", "starting", "authorizing", "verifying"].contains($0.phase) } ?? true }
    private func call(_ method: String, extra: [String: Any] = [:]) async {
        guard !sending else { return }
        sending = true; defer { sending = false }; error = nil
        if ["start", "verify"].contains(method) { refreshedSession = nil }
        do {
            status = try await engine.call("agyAuth." + method, params: ["sourceId":source.id, "id":status?.id ?? ""].merging(extra) { _,new in new }); model.agyLoginActive = active
            if status?.authenticated == true, refreshedSession != status?.id {
                try await model.refreshAntigravityAccount(sourceID: source.id, status: status?.accountStatus)
                refreshedSession = status?.id
            }
        }
        catch { self.error = error.localizedDescription }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack { Text("Antigravity 登录").font(AppFont.title); Spacer(); Button("完成") { Task { if active { await call("cancel") }; dismiss() } }.disabled(sending) }
            Text(source.name).font(AppFont.section)
            Text("复用此机器的现有登录，或完成浏览器授权。登录后显示邮箱与订阅，登录凭据由 agy 保存在原机器。").font(AppFont.secondary).foregroundStyle(.secondary)
            HStack { if active { ProgressView().controlSize(.small) }; Text(status?.message ?? "准备检查…").textSelection(.enabled) }
            if let identity = status?.identity { Text(identity.summary).textSelection(.enabled).foregroundStyle(status?.current == false ? Palette.warn : Color.primary) }
            if let note = status?.metadataError { Text(note).font(AppFont.secondary).foregroundStyle(.secondary) }
            if let raw = status?.authUrl, let url = URL(string: raw) {
                HStack { Link("打开授权网页", destination: url); Button("复制授权网址") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(raw, forType: .string) } }
                SecureField("浏览器返回的授权码", text: $code)
                Button("提交授权码") { let value = code; code = ""; Task { await call("submitCode", extra: ["code":value]) } }.disabled(code.isEmpty || sending)
            }
            if status?.workspaceConfirmationRequired == true { Button("确认临时目录") { Task { await call("confirmWorkspace") } }.disabled(sending) }
            if let error { Text(error).foregroundStyle(Palette.warn).textSelection(.enabled) }
            HStack {
                if status != nil { Button("验证登录") { Task { await call("verify") } }.disabled(sending || status?.phase == "verifying" || status?.phase == "checking") }
                if !active || status == nil { Button("重新登录 / 检查") { Task { await call("start") } }.disabled(sending) }
                if active, status != nil { Button("取消授权") { Task { await call("cancel") } }.disabled(sending) }
            }
        }.padding(24).frame(width: 540).textFieldStyle(.roundedBorder).interactiveDismissDisabled(active || sending)
        .task {
            model.agyLoginActive = true; await call("start")
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { break }
                if !sending, status != nil, active || (status?.authenticated == true && refreshedSession != status?.id) { await call("status") }
            }
        }
        .onDisappear { model.agyLoginActive = false; if active, let id = status?.id { Task { let _: AntigravityLoginState? = try? await engine.call("agyAuth.cancel", params: ["sourceId":source.id,"id":id]) } } }
    }
}
