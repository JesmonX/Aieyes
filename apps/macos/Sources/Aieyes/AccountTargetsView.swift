import SwiftUI

@MainActor final class AccountTargetsModel: ObservableObject {
    @Published var rows: [AccountTarget] = []
    @Published var errors: [String] = []
    @Published var checking = Set<String>()
    private var generation = 0
    func load(_ model: AppModel, accountKey: String? = nil, includeUnavailable: Bool = false) async {
        generation += 1
        let revision = generation, reader = EngineClient()
        var params: [String: Any] = accountKey.map { ["accountKey": $0] } ?? [:]
        if includeUnavailable { params["includeUnavailable"] = true }
        errors = []
        do {
            let initial: [AccountTarget] = try await reader.call("accounts.targets", params: params)
            guard revision == generation, !Task.isCancelled else { return }
            rows = initial
            let sources = Set(rows.filter { target in target.provider == "codex" && target.available != false && model.settings.sources.contains(where: { source in source.id == target.sourceId && model.settings.sourceEnabled(source) }) }.map(\.sourceId))
            checking = sources
            await withTaskGroup(of: (String, String?).self) { group in
                for source in sources {
                    group.addTask {
                        let client = EngineClient()
                        do {
                            let result: AccountTargetRefresh = try await client.call("codexAuth.reconcile", params: ["sourceId": source, "accountId": accountKey?.replacingOccurrences(of: "codex:", with: "") ?? ""])
                            return (source, result.issues.isEmpty ? nil : result.issues.map(\.message).joined(separator: "；"))
                        } catch { return (source, error.localizedDescription) }
                    }
                }
                for await (source, error) in group {
                    guard revision == generation else { continue }
                    checking.remove(source)
                    if let error { errors.append((rows.first { $0.sourceId == source }?.machineName ?? source) + "：" + error) }
                    do {
                        let saved: Settings = try await reader.call("settings.get")
                        guard revision == generation, !Task.isCancelled else { continue }
                        model.acceptConfiguration(saved)
                        let next: [AccountTarget] = try await reader.call("accounts.targets", params: params)
                        if revision == generation && !Task.isCancelled { rows = next }
                    } catch { if revision == generation && !Task.isCancelled { errors.append(error.localizedDescription) } }
                }
            }
        } catch { if revision == generation && !Task.isCancelled { errors.append(error.localizedDescription) } }
        if revision == generation { checking = []; await model.loadAccountStatuses() }
    }
}

struct AccountTargetsView: View {
    @ObservedObject var model: AppModel
    let accountKey: String
    @StateObject private var targets = AccountTargetsModel()
    @State private var loginSource: AgentSource?
    @State private var intent = "login"
    private var account: AgentAccount? { model.settings.accounts.first { $0.key == accountKey && $0.archived != true } }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 14) {
            HStack { Text(L("切换账户 · ") + (account?.name ?? L("账户"))).font(AppFont.title); Spacer(); Button(L("重新检查")) { Task { await targets.load(model, accountKey: accountKey) } }.disabled(!targets.checking.isEmpty) }
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(targets.errors, id: \.self) { Text($0).foregroundStyle(Palette.warn).textSelection(.enabled) }
                    if targets.rows.isEmpty {
                        Text(L("暂无可登录或切换的机器。请启用 Agent、选择机器并开启目录采集。")).foregroundStyle(.secondary)
                        Button(L("打开机器与采集设置")) { model.openSettings(.init(provider: "codex", section: "machines")) }
                    }
                    ForEach(targets.rows) { target in
                        HStack {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(target.machineName).font(AppFont.section)
                                Text(target.path).font(AppFont.secondary).textSelection(.enabled)
                                Text(L(target.reason ?? (target.current == true ? L("正在使用") : target.credential == true ? L("已登录，尚未选用") : target.credential == false ? L("未登录") : L("登录状态待确认")))).foregroundStyle(.secondary)
                                if let reason = target.draftReason { Text(L(reason)).font(AppFont.secondary).foregroundStyle(Palette.warn) }
                            }
                            Spacer()
                            if targets.checking.contains(target.sourceId) { ProgressView().controlSize(.small) }
                            // Native references are detected automatically; there is nothing to scan or switch.
                            if target.nativeConnectionId == nil && (target.current != true || target.bound == false || target.canDraft == false) {
                                Button(target.bound == false ? L("完成关联") : target.canSwitch ? L("切换") : target.credential == false ? L("登录") : L("检查 / 登录")) {
                                    intent = target.canSwitch ? "switch" : "login"
                                    loginSource = model.settings.sources.first { $0.id == target.sourceId }
                                }.disabled(targets.checking.contains(target.sourceId) && !target.canSwitch)
                            }
                        }.padding(12).background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 10))
                    }
                }
            }
        }.padding(20).frame(minWidth: DesignTokens.dialogTargetsWidth, minHeight: 360).aieyesAccent()
        .task { await targets.load(model, accountKey: accountKey) }
        .sheet(item: $loginSource, onDismiss: { Task { await targets.load(model, accountKey: accountKey) } }) { source in
            AccountConnectionView(model: model, account: account, initialSourceID: source.id, intent: intent)
        }
    }
}

extension AppModel {
    func canSwitchAccount(_ key: String) -> Bool {
        settings.accounts.contains { $0.key == key && $0.provider == "codex" && $0.archived != true } && !accountMachineIDs("codex").isEmpty
    }
}
