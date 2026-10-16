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
                    Text("找到你的目标或问题，查看还需要哪些配置。状态来自已保存配置和上次观察结果，保存过设置不代表连接已验证。").foregroundStyle(.secondary)
                    FlowLayout(spacing: 12) {
                        Picker("使用场景", selection: Binding(get: { scene }, set: { savedScene = $0 })) {
                            Text("仅本机").tag("local"); Text("仅 SSH 服务器").tag("ssh"); Text("本机与 SSH").tag("mixed")
                        }.fixedSize()
                        Button(observations.loading ? "正在读取状态…" : "重新读取状态") { Task { await observations.load() } }.disabled(observations.loading)
                        Button("返回总览", action: finish)
                    }
                    if !observations.errors.isEmpty { Text("部分状态读取失败，相关项目待确认。可以重新读取，或前往对应页面查看。").font(AppFont.secondary).foregroundStyle(Palette.warn) }
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
        let status = warnings > 0 ? "\(warnings) 项配置或异常需检查" : pending > 0 ? "\(pending) 项待配置或验证" : "查看当前配置与验证方法"
        return Surface {
            DisclosureGroup(isExpanded: expansion(goal.id)) {
                VStack(alignment: .leading, spacing: 12) {
                    Divider()
                    if goal.id == "network" { networkHelp }
                    else { ForEach(Array(goal.steps.enumerated()), id: \.offset) { index, text in Text("\(index + 1). " + text).fixedSize(horizontal: false, vertical: true) } }
                    Text("当前配置").font(AppFont.secondary.weight(.semibold))
                    ForEach(Array(rows.enumerated()), id: \.offset) { _, row in checkRow(row) }
                    if goal.id == "usage" {
                        FlowLayout(spacing: 8) {
                            ForEach(CoreContract.providers, id: \.self) { provider in
                                Button { model.openSettings(.init(provider: provider, section: "accounts")) } label: { ProviderIdentity(provider: provider, markSize: 20) }
                            }
                        }
                    }
                    FlowLayout(spacing: 8) {
                        ForEach(Array(goal.actions.enumerated()), id: \.offset) { _, action in Button(action.title) { model.openSettings(action.destination) } }
                    }
                    Text("怎样确认生效：" + goal.verification).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
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
            Text(row.text).font(AppFont.secondary).foregroundStyle(row.level == "warn" ? Palette.warn : Color.secondary).fixedSize(horizontal: false, vertical: true)
            Button("查看设置") { model.openSettings(row.destination) }
            Divider()
        }
    }
    @ViewBuilder private var networkHelp: some View {
        if scene != "ssh" { Text("本机：进入网络代理，选择系统代理、直连或指定代理。Agent 的本机目录可单独覆盖应用代理，需要一起核对。") }
        if scene != "local" {
            Text("服务器：进入“账户 → 对应机器 → 前置命令”，按远端实际网络填写以下一种方式。只有需要代理时才配置，服务器可直连时可以留空。")
            command("export http_proxy=http://PROXY_HOST:PORT\nexport https_proxy=http://PROXY_HOST:PORT")
            Text("或加载服务器上已有的代理脚本：").foregroundStyle(.secondary)
            command("source /path/to/proxy")
            Text("代理地址必须在服务器上可达；服务器中的 127.0.0.1 指服务器自身。SSH 的 ProxyJump / ProxyCommand 只决定 SSH 如何连接，应用代理不会自动传给远端 Agent。前置命令用于此账户在此机器上的授权、切换、额度查询及定时唤醒。")
        }
    }
    private func command(_ text: String) -> some View {
        Text(text).font(.system(size: 12, design: .monospaced)).textSelection(.enabled).fixedSize(horizontal: false, vertical: true).frame(maxWidth: .infinity, alignment: .leading).padding(10).background(Palette.accent.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
    }
}
