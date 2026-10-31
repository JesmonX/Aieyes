import SwiftUI

/// Native credentials are detected automatically at launch and when their files change.
@MainActor struct NativeConnectionsSection: View {
    @SurfaceModel var model: AppModel
    let provider: String, kind: String
    @State private var removing: String?
    @State private var error: String?
    @State private var expanded = false
    @State private var confirmingRemoveAll = false
    private var rows: [NativeConnection] { (model.settings.nativeConnections ?? []).filter { CoreContract.sameAgent($0.provider, provider) && $0.kind == kind } }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        if !rows.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text((kind == "api" ? L("原生 API 连接") : L("已发现的原生登录凭据")) + "（\(rows.count)）").font(AppFont.section)
                    Spacer()
                    // Unlinking everything at once is confirmed first; it only forgets references, never files.
                    Button(L("取消全部引用"), role: .destructive) { confirmingRemoveAll = true }.disabled(removing != nil)
                        .confirmationDialog(L("取消全部引用？"), isPresented: $confirmingRemoveAll) {
                            Button(L("取消全部引用"), role: .destructive) { remove(rows.map(\.id)) }
                        } message: { Text(L("将取消 {0} 个引用。凭据文件不会被删除；之后在 Agent 中重新登录会自动识别。", rows.count)) }
                }
                DisclosureGroup(kind == "api" ? L("查看连接") : L("查看凭据"), isExpanded: $expanded) {
                ForEach(rows) { row in
                    HStack(alignment: .top) {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(row.displayName)
                            Text(model.machineName(row.machineId) + " · " + row.statusLabel + (row.accountKey == nil ? "" : L(" · 已关联账户"))).font(AppFont.secondary).foregroundStyle(.secondary)
                            if !row.clientLabel.isEmpty { Text(row.clientLabel).font(AppFont.secondary).foregroundStyle(.secondary) }
                            if !row.models.isEmpty { Text(row.models.joined(separator: "、")).font(AppFont.secondary) }
                            Text(row.configPath).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled)
                            Text(L(row.note)).font(AppFont.secondary).foregroundStyle(.secondary)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                        Button(L("取消引用")) { remove([row.id]) }.disabled(removing != nil)
                    }
                }
                Text(L("凭据保留在原机器的 Agent 配置中；取消引用不会删除原文件。")).font(AppFont.secondary).foregroundStyle(.secondary)
                }
                if let error { Text(L(error)).foregroundStyle(Palette.warn) }
            }
        }
    }
    private func remove(_ ids: [String]) {
        guard removing == nil else { return }
        removing = ids.first
        Task {
            defer { removing = nil }
            do {
                let result: NativeRemovalResult = try await model.engine.call("accounts.native.remove", params: ["connectionIds":ids])
                model.acceptConfiguration(result.settings); error = nil
            } catch { self.error = error.localizedDescription }
        }
    }
}
private struct NativeRemovalResult: Decodable { var settings: Settings }
