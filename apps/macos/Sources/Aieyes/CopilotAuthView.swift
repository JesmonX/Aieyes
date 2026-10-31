import AppKit
import SwiftUI

/// Mirrors `copilotAuth.*`. Only the user code and GitHub's page address reach the UI; the token stays in core.
struct CopilotAuthState: Decodable {
    var phase: String
    var message: String?
    var deviceFlow: Bool?
    var userCode: String?
    var verificationUri: String?
    var login: String?
    var commands: [String]?
}

/// "GitHub 授权" for Copilot. With Aieyes' own GitHub OAuth App this runs the device flow;
/// without one it guides `copilot login` / `gh auth login` and re-detects. No other app's client id is used.
struct CopilotAuthView: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var engine = EngineClient(configurationOnly: true)
    @State private var status: CopilotAuthState?
    @State private var error: String?
    @State private var sending = false
    private var waiting: Bool { ["waiting", "authorized"].contains(status?.phase ?? "") }
    private var finished: Bool { ["detected", "authorized"].contains(status?.phase ?? "") }
    private func call(_ method: String) async {
        guard !sending else { return }
        sending = true; defer { sending = false }; error = nil
        do {
            status = try await engine.call("copilotAuth." + method)
            if finished { await model.refreshIdentitySettings(); await model.refreshQuotas() }
        } catch { self.error = L(error.localizedDescription) }
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 18) {
            Text(L("GitHub 授权")).font(AppFont.title)
            HStack { if waiting || status == nil { ProgressView().controlSize(.small) }; Text(L(status?.message ?? L("正在检查 GitHub 登录…"))).textSelection(.enabled) }
            if status?.phase == "waiting", let code = status?.userCode {
                Text(L("在 GitHub 页面输入授权码")).font(AppFont.secondary).foregroundStyle(.secondary)
                Text(code).font(.system(.title2, design: .monospaced)).textSelection(.enabled)
                HStack {
                    if let raw = status?.verificationUri, let url = URL(string: raw), url.scheme == "https" { Link(L("打开授权网页"), destination: url) }
                    Button(L("复制授权码")) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(code, forType: .string) }
                }
            }
            if status?.phase == "external" {
                ForEach(status?.commands ?? [], id: \.self) { command in
                    Text(command).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                }
            }
            if let error { ErrorNotice(message: error) }
            HStack {
                if status?.phase == "external" { Button(L("重新检测")) { Task { await call("status") } }.disabled(sending) }
                Spacer()
                Button(waiting && status?.phase == "waiting" ? L("取消授权") : L("完成")) {
                    Task { if status?.phase == "waiting" { await call("cancel") }; dismiss() }
                }.keyboardShortcut(.cancelAction).disabled(sending)
            }
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogAgyWidth).interactiveDismissDisabled(sending)
        .task {
            await call("start")
            while !Task.isCancelled, waiting {
                do { try await Task.sleep(for: .seconds(2)) } catch { break }
                if !sending { await call("status") }
            }
        }
        .onDisappear { if status?.phase == "waiting" { Task { let _: CopilotAuthState? = try? await engine.call("copilotAuth.cancel") } } }
    }
}
