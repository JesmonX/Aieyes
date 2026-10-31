import SwiftUI

struct BuddyAuthState: Decodable {
    var id: String?
    var phase: String
    var message: String?
    var authUrl: String?
    var connected: Bool
}

struct BuddyAuthView: View {
    @ObservedObject var model: AppModel
    let provider: String
    @Environment(\.dismiss) private var dismiss
    @State private var engine = EngineClient(configurationOnly: true)
    @State private var status: BuddyAuthState?
    @State private var error: String?
    @State private var sending = false
    @State private var region = "cn"
    private var waiting: Bool { ["waiting", "authorized"].contains(status?.phase ?? "") }
    private func call(_ method: String) async {
        guard !sending else { return }
        sending = true; defer { sending = false }; error = nil
        do {
            status = try await engine.call("buddyAuth." + method, params: ["provider": provider, "region": region, "id": status?.id ?? ""])
            if status?.connected == true || method == "logout" { await model.refreshIdentitySettings(); await model.refreshQuotas() }
        } catch { self.error = L(error.localizedDescription) }
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage
        VStack(alignment: .leading, spacing: 18) {
            Text(L("登录查看余额")).font(AppFont.title)
            Text(L("可选登录，使用非公开接口；可随时退出登录。仅用于查看余额。")).foregroundStyle(.secondary)
            Picker(L("登录地区"), selection: $region) {
                Text(L("中国大陆")).tag("cn")
                Text(L("国际版")).tag("global")
            }.disabled(sending || waiting)
            if let message = status?.message { Text(L(message)).textSelection(.enabled) }
            if let raw = status?.authUrl, let url = URL(string: raw), url.scheme == "https" { Link(L("打开授权网页"), destination: url) }
            if waiting { ProgressView().controlSize(.small) }
            if let error { ErrorNotice(message: error) }
            HStack {
                Button(L("登录查看余额")) { Task { await call("start") } }.disabled(sending || waiting)
                if status?.connected == true { Button(L("退出所有余额登录")) { Task { await call("logout") } }.disabled(sending) }
                Spacer()
                Button(waiting ? L("取消授权") : L("完成")) { Task { if waiting { await call("cancel") }; dismiss() } }.keyboardShortcut(.cancelAction).disabled(sending)
            }
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogAgyWidth).interactiveDismissDisabled(sending)
        .task {
            await call("status")
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(2)) } catch { break }
                if waiting && !sending { await call("status") }
            }
        }
        .onDisappear {
            if waiting { let id = status?.id ?? ""; Task { let _: BuddyAuthState? = try? await engine.call("buddyAuth.cancel", params: ["id": id]) } }
        }
    }
}
