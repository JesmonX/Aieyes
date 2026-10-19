import SwiftUI

/// Reads existing results only. Opening help never probes a CLI or runs a task.
@MainActor final class SetupGuideObservations: ObservableObject {
    @Published var dashboard: Dashboard?
    @Published var wakeups: [WakeRecord]?
    @Published var errors: Set<String> = []
    @Published var loading = false
    private let dashboardEngine = EngineClient(configurationOnly: true)
    private let taskEngine = EngineClient(configurationOnly: true)
    func load() async {
        guard !loading, !CommandLine.arguments.contains("--render") else { return }
        loading = true; errors = []
        defer { loading = false }
        async let summary: Dashboard = dashboardEngine.call("dashboard.summary")
        async let tasks: [WakeRecord] = taskEngine.call("wakeups.list")
        do { dashboard = try await summary } catch { dashboard = nil; errors.insert("dashboard") }
        do { wakeups = try await tasks } catch { wakeups = nil; errors.insert("wakeups") }
    }
}

struct SetupGuideView: View {
    @ObservedObject var model: AppModel
    var finish: () -> Void
    @StateObject private var observations = SetupGuideObservations()
    @AppStorage("setup.scene.v1") private var savedScene = ""
    @AppStorage("setup.expanded.v2") private var expanded = ""
    @AppStorage(NativePanelKey.accounts.rawValue) private var panelPreference = "{}"
    private var scene: String { ["local", "ssh", "mixed"].contains(savedScene) ? savedScene : model.settings.hosts.isEmpty ? "local" : model.settings.localDeviceEnabled == false ? "ssh" : "mixed" }
    private var checks: SetupGuideChecks {
        .init(settings: model.settings, dashboard: observations.errors.contains("dashboard") ? nil : observations.dashboard ?? model.dashboard, hosts: model.hosts, wakeups: observations.wakeups, errors: observations.errors, scene: scene, panelPreference: panelPreference)
    }
    private func expansion(_ id: String) -> Binding<Bool> {
        Binding(get: { expanded.split(separator: ",").contains(Substring(id)) }, set: { value in
            var ids = Set(expanded.split(separator: ",").map(String.init))
            if value { ids.insert(id) } else { ids.remove(id) }
            expanded = ids.sorted().joined(separator: ",")
        })
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Surface(title: "你想用 Aieyes 做什么？") {
                    Text("选择目标完成配置，下方显示最近读取的状态。").foregroundStyle(.secondary)
                    FlowLayout(spacing: 12) {
                        Picker("使用场景", selection: Binding(get: { scene }, set: { savedScene = $0 })) {
                            Text("仅本机").tag("local"); Text("仅 SSH 服务器").tag("ssh"); Text("本机与 SSH").tag("mixed")
                        }.fixedSize()
                        Button(observations.loading ? "正在读取状态…" : "重新读取状态") { Task { await observations.load() } }.disabled(observations.loading)
                        Button("返回总览", action: finish)
                    }
                    if !observations.errors.isEmpty { Text("部分状态读取失败，请重试或打开对应设置。").font(AppFont.secondary).foregroundStyle(Palette.warn) }
                }
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 340), spacing: 14, alignment: .top)], alignment: .leading, spacing: 14) {
                    ForEach(SetupGuide.goals.filter { $0.id != "hosts" || scene != "local" }) { goal in goalCard(goal) }
                }
                Surface {
                    DisclosureGroup(isExpanded: expansion("more")) {
                        VStack(alignment: .leading, spacing: 16) {
                            ForEach(Array(SetupGuide.more.enumerated()), id: \.offset) { _, item in
                                Divider()
                                Text(item.0).font(AppFont.section)
                                Text(item.1).foregroundStyle(.secondary)
                                Button("了解与设置") { model.openSettings(item.2) }
                            }
                        }.padding(.top, 12)
                    } label: {
                        VStack(alignment: .leading, spacing: 6) { Text("更多功能").font(AppFont.section); Text("多账户与切换、API 连接、桌宠与会话、外观和更新").font(AppFont.secondary).foregroundStyle(.secondary) }
                    }
                }
            }.padding(22).background(OverlayScrollStyle()).background(ScrollMemory(key: "setup", ready: !observations.loading))
        }.font(AppFont.body).task { await observations.load() }
    }
    private func goalCard(_ goal: SetupGuide.Goal) -> some View {
        let rows = checks.rows(goal.id)
        let warnings = rows.filter { $0.level == "warn" }.count, pending = rows.filter { $0.level == "pending" }.count
        let status = warnings > 0 ? "\(warnings) 项需检查" : pending > 0 ? "\(pending) 项待配置或验证" : "查看当前状态"
        return Surface {
            DisclosureGroup(isExpanded: expansion(goal.id)) {
                VStack(alignment: .leading, spacing: 12) {
                    Divider()
                    if goal.id == "network" { networkHelp }
                    else { ForEach(Array(goal.steps.enumerated()), id: \.offset) { index, text in Text("\(index + 1). " + text).fixedSize(horizontal: false, vertical: true) } }
                    Text("当前状态").font(AppFont.secondary.weight(.semibold))
                    ForEach(Array(rows.enumerated()), id: \.offset) { _, row in checkRow(row) }
                    if goal.id == "usage" {
                        FlowLayout(spacing: 8) {
                            ForEach(CoreContract.agentProviders, id: \.self) { provider in
                                Button { model.openSettings(.init(provider: provider, section: "accounts")) } label: { ProviderIdentity(provider: provider, markSize: 20) }
                            }
                        }
                    }
                    FlowLayout(spacing: 8) {
                        ForEach(Array(goal.actions.enumerated()), id: \.offset) { _, action in Button(action.title) { model.openSettings(action.destination) } }
                    }
                    if !goal.help.isEmpty {
                        DisclosureGroup("更多说明") {
                            Text(goal.help).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }.padding(.top, 12)
            } label: {
                VStack(alignment: .leading, spacing: 8) {
                    Text(goal.title).font(AppFont.section).fixedSize(horizontal: false, vertical: true)
                    Text(goal.detail).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    Text(status).font(AppFont.secondary).foregroundStyle(warnings > 0 ? Palette.warn : Color.secondary)
                }
            }
        }
    }
    private func checkRow(_ row: SetupGuide.Check) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(row.label).font(AppFont.secondary.weight(.semibold)).fixedSize(horizontal: false, vertical: true)
            FlowLayout(spacing: 8) {
                Text(row.status.title).font(AppFont.caption.weight(.medium))
                    .padding(.horizontal, 7).padding(.vertical, 3)
                    .background((row.level == "warn" ? Palette.warn : row.level == "ok" ? Palette.ok : Palette.accent).opacity(0.12), in: Capsule())
                    .foregroundStyle(row.level == "warn" ? Palette.warn : row.level == "ok" ? Palette.ok : Palette.accent)
                Text(row.text).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            if let time = row.updatedAt, time > 0 {
                Text("上次数据：" + Date(timeIntervalSince1970: time).formatted(date: .numeric, time: .standard)).font(AppFont.caption).foregroundStyle(.secondary)
            }
            if !row.details.isEmpty {
                DisclosureGroup("状态说明") { Text(row.details).font(AppFont.secondary).foregroundStyle(.secondary) }
            }
            Button("查看设置") { model.openSettings(row.destination) }
            Divider()
        }
    }
    @ViewBuilder private var networkHelp: some View {
        if scene != "ssh" {
            Text("本机：检查应用代理及目录的独立代理。")
            Button("网络设置") { model.openSettings(.init(category: "general", section: "network")) }
        }
        if scene != "local" {
            Text("服务器：在下方选择账户与机器，按需配置前置命令。")
            DisclosureGroup("服务器代理配置示例") {
                VStack(alignment: .leading, spacing: 10) {
                    Text("设置服务器可访问的代理地址：")
                    command("export http_proxy=http://PROXY_HOST:PORT\nexport https_proxy=http://PROXY_HOST:PORT")
                    Text("或加载已有代理脚本：")
                    command("source /path/to/proxy")
                    Text("127.0.0.1 指服务器自身。应用代理不会自动传给远端；ProxyJump / ProxyCommand 仅用于 SSH 连接。前置命令用于账户授权、切换、额度查询和定时唤醒。")
                }.font(AppFont.secondary).foregroundStyle(.secondary)
            }
        }
    }
    private func command(_ text: String) -> some View {
        Text(text).font(.system(size: 12, design: .monospaced)).textSelection(.enabled).fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, alignment: .leading).padding(10).background(Palette.accent.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
    }
}
