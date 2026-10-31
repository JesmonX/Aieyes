import AppKit
import SwiftUI

/// Mirrors `qoderCli.status`. Qoder desktop, IDE and QoderWork keep their logins
/// encrypted and are never read; quota checks borrow qodercli instead.
struct QoderCLIStatus: Decodable, Equatable {
    var apps: [String]
    var cli: Bool
    var account: Bool
    var offerInstall: Bool
    var command: String
    var message: String
    /// A Qoder account or an identified CLI login makes the install prompt moot.
    static func signedIn(_ settings: Settings) -> Bool {
        settings.accounts.contains { $0.provider == "qoder" && $0.archived != true }
            || (settings.nativeConnections ?? []).contains { $0.provider == "qoder" && $0.identityKey != nil }
    }
}

/// The one prompt for users with only a Qoder desktop app. The sheet belongs to the
/// accounts list, so detecting the login (which hides this row) keeps it open.
struct QoderCLIRow: View {
    @ObservedObject var model: AppModel
    let status: QoderCLIStatus
    let enabled: Bool
    var open: () -> Void
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        if !status.apps.isEmpty && !QoderCLIStatus.signedIn(model.settings) {
            HStack {
                Text(L("已安装 {0}；额度通过 qodercli 查询，安装并登录后显示", status.apps.joined(separator: " / "))).font(AppFont.secondary).foregroundStyle(.secondary)
                Spacer()
                Button(status.cli ? L("登录 qodercli") : L("安装 qodercli 并登录"), action: open).disabled(!enabled)
            }
        }
    }
}

/// "安装 qodercli 并登录": the official installer links `~/.local/bin/qodercli`, where
/// quota checks find it without a login shell. Re-detect reads the new local CLI login.
struct QoderCLIInstallView: View {
    @ObservedObject var model: AppModel
    var changed: (QoderCLIStatus) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var engine = EngineClient(configurationOnly: true)
    @State private var status: QoderCLIStatus?
    @State private var error: String?
    @State private var sending = false
    @State private var copied: String?
    private struct Scan: Decodable {}
    private var signedIn: Bool { QoderCLIStatus.signedIn(model.settings) }
    private func run(_ work: () async throws -> Void) async {
        guard !sending else { return }
        sending = true; defer { sending = false }; error = nil
        do { try await work() } catch { self.error = L(error.localizedDescription) }
    }
    private func load() async throws {
        let value: QoderCLIStatus = try await engine.call("qoderCli.status")
        status = value; changed(value)
    }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 14) {
            Text(L("安装 qodercli 并登录")).font(AppFont.title)
            HStack { if sending { ProgressView().controlSize(.small) }; Text(signedIn ? L("已识别 Qoder 登录") : L(status?.message ?? "正在检查 qodercli…")).textSelection(.enabled) }
            Text(L("在终端运行：")).font(AppFont.secondary).foregroundStyle(.secondary)
            Text(status?.command ?? "").font(.system(.body, design: .monospaced)).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            HStack {
                Button(L("复制命令")) {
                    NSPasteboard.general.clearContents()
                    copied = NSPasteboard.general.setString(status?.command ?? "", forType: .string) ? L("已复制") : L("复制失败，请重试")
                }.disabled(status == nil)
                if let copied { Text(copied).font(AppFont.secondary).foregroundStyle(.secondary) }
            }
            Text(L("登录时也可选择 Personal Access Token，在 {0} 生成后粘贴。", "https://qoder.com/account/integrations")).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            if let error { ErrorNotice(message: error) }
            HStack {
                Button(sending ? L("检查中…") : L("重新检测")) {
                    Task {
                        await run {
                            let _: Scan = try await engine.call("accounts.autoScan", params: ["targets": [["qoder", "local"]]])
                            await model.refreshIdentitySettings()
                            try await load()
                            if signedIn { await model.refreshQuotas() }
                        }
                    }
                }.disabled(sending)
                Spacer()
                Button(L("完成")) { dismiss() }.keyboardShortcut(.cancelAction).disabled(sending)
            }
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogAgyWidth).interactiveDismissDisabled(sending)
        .task { await run { try await load() } }
    }
}
