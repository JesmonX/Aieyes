import Foundation

/// Goal content mirrors the desktop guide; configuration never implies connectivity.
enum SetupGuide {
    static func needsSetup(_ settings: Settings) -> Bool {
        settings.accounts.isEmpty && settings.hosts.isEmpty && settings.sources.allSatisfy { $0.id == "local-" + $0.provider } && settings.proxy.mode == "system"
    }
    struct Action { let title: String; var destination: SettingsDestination }
    struct Goal: Identifiable {
        let id: String, title: String, detail: String
        let steps: [String], verification: String
        let actions: [Action]
    }
    struct Check {
        let label: String, text: String
        var destination: SettingsDestination
        var level = "info"
    }
    static let goals: [Goal] = [
        .init(id: "usage", title: "我想查看 Agent 用量和限额", detail: "用量需要采集日志；限额需要连接账户并开启查询。",
              steps: ["启用要使用的 Agent，选择本机或服务器，再核对日志目录及采集开关。", "在该 Agent 下添加账户，使用已有登录、引导登录或填写 API Key / Cookie，并连接到对应机器。DeepSeek 余额通过本机 API Key 查询，无需日志目录。"],
              verification: "手动同步记录、刷新限额，分别检查采集状态、账户错误和更新时间。支持能力以对应 Agent 页面为准。", actions: [.init(title: "机器与目录", destination: .init(section: "machines")), .init(title: "账户与限额", destination: .init(section: "accounts"))]),
        .init(id: "network", title: "国外 Agent 查询失败，或唤醒连不上网络？", detail: "本机检查应用代理；服务器在账户设备前置命令中配置代理。",
              steps: [],
              verification: "本机先在网络代理页测试出站连接，再刷新具体账户限额。服务器先确认 SSH 可连接，再查询该账户；已有唤醒任务可查看运行记录。出站测试和 SSH 成功都不能代表所有 Agent 可联网。", actions: []),
        .init(id: "refresh", title: "限额怎么很久才更新？", detail: "调整自动查询间隔；单次请求耗时长时，先检查网络和登录。",
              steps: ["在“刷新与历史”调整全局限额间隔；账户的查询间隔会覆盖全局值，留空才沿用全局。", "记录采集、限额查询和服务器监控有各自的间隔。缩短限额间隔不会加速正在进行的请求；查询慢或超时请检查网络、登录及目标机器执行环境。"],
              verification: "保存后查看账户实际采用的间隔；可以手动刷新一次，确认查询结果和更新时间。", actions: [.init(title: "调整全局刷新", destination: .init(category: "general", section: "refresh")), .init(title: "检查网络", destination: .init(category: "general", section: "network"))]),
        .init(id: "hosts", title: "我想采集服务器 Agent，或查看服务器资源", detail: "添加 SSH 主机；Agent 采集和资源监控可以分别开启。",
              steps: ["导入 SSH config 或手动添加主机，确认身份验证和 SSH 连接可用。跳板连接沿用 SSH 配置中的 ProxyJump / ProxyCommand。", "查看资源时开启资源监控；采集 Agent 时还需开启 Agent 采集，并在 Agent 的机器与目录中选择服务器。远端 Agent 联网另行配置账户前置命令。"],
              verification: "连接并读取设备，在服务器页检查资源数据；在 Agent 总览分别检查记录采集和限额查询。", actions: [.init(title: "配置 SSH 主机", destination: .init(category: "hosts")), .init(title: "选择采集目录", destination: .init(section: "machines"))]),
        .init(id: "wakeups", title: "我想每天自动唤醒订阅", detail: "确认账户、执行机器和网络，再保存任务并部署。",
              steps: ["选择支持唤醒的订阅账户与执行机器，确认目标机器的 CLI 已登录且能联网；服务器代理使用该账户在此机器上的前置命令。", "读取模型与执行能力，选择模型和每天时刻；按目标机器时区执行。保存草稿后仍需部署，修改草稿后需要更新部署。", "退出 Aieyes 后已部署任务仍可运行，但不会唤醒休眠电脑，离线或错过时刻不补发。"],
              verification: "部署后检查启用状态与运行记录。需要测试时主动点击“立即运行”，这会实际调用模型并可能消耗额度。", actions: [.init(title: "配置定时唤醒", destination: .init(category: "wakeups"))]),
        .init(id: "panel", title: "配置了账户，为什么面板没显示？", detail: "检查账户归档、额度显示开关及面板账户选择。",
              steps: ["账户必须未归档，并开启“显示并查询额度”。在账户设置中调整面板选择。", "自动模式显示每种 Agent 排序前五个账户；手动模式可以选择任意数量或留空，新增账户不会自动加入手动选择。"],
              verification: "打开面板确认账户是否出现；面板选中不代表该账户最近一次查询成功。", actions: [.init(title: "调整面板账户", destination: .init(section: "accounts"))]),
        .init(id: "prices", title: "有用量却没有金额，或提示未计价？", detail: "先补齐模型价格和映射，需要时再了解额度估值与采样。",
              steps: ["在模型价格中补齐有用量的模型价格；模型名称不同的情况配置映射，再同步价格或重新计价。", "API 等价成本由日志用量和模型价格计算；订阅额度价值需要估值或采样，结果不是可兑换余额。"],
              verification: "在用量分析中检查未计价提示和金额。仅限额查询成功不能证明用量计价完整。", actions: [.init(title: "模型价格与映射", destination: .init(category: "prices")), .init(title: "了解估值与采样", destination: .init(section: "valuation"))]),
        .init(id: "notifications", title: "我想在额度重置时收到提醒", detail: "配置提醒规则、账户覆盖项，并验证系统通知。",
              steps: ["在通知设置中选择非预期重置、自然重置提醒；账户可单独覆盖全局规则。额度一直满额时不会提醒。", "允许 Aieyes 发送系统通知，并检查勿扰模式。提醒依赖实时限额查询；退出应用或电脑休眠时暂停检测。"],
              verification: "在通知页发送测试通知确认系统可显示，再检查账户查询是否正常。", actions: [.init(title: "通知规则与测试", destination: .init(category: "general", section: "notifications")), .init(title: "账户提醒覆盖", destination: .init(section: "accounts"))]),
    ]
    static let more: [(String, String, SettingsDestination)] = [
        ("多账户与切换", "同一 Agent 可添加多个账户并连接不同机器；在账户页区分“已登录”和“正在使用”，按需切换。", .init(section: "accounts")),
        ("Codex API 连接", "需要通过 API 使用 Codex 时，在 Codex 的 API 连接区域配置；与订阅账户分开管理。", .init(provider: "codex", section: "api")),
        ("桌宠与会话", "配置桌宠资源、大小和会话展示；要看账户限额，请先选择面板账户。", .init(category: "general", section: "desktop")),
        ("外观", "选择主题、强调色和图表配色。", .init(category: "general", section: "appearance")),
        ("更新与关于", "查看版本、检查更新及更新设置。", .init(category: "general", section: "updates")),
    ]
}

struct SetupGuideChecks {
    let settings: Settings
    let dashboard: Dashboard?
    let hosts: [HostResult]
    let wakeups: [WakeRecord]?
    let errors: Set<String>
    let scene: String
    let panelPreference: String
    private func inScene(_ source: AgentSource) -> Bool { scene == "mixed" || (scene == "local" ? source.hostId == nil : source.hostId != nil) }
    private var sources: [AgentSource] { settings.sources.filter(inScene) }
    private func relevantSource(_ source: AgentSource) -> Bool {
        settings.agents?.first { $0.provider == source.provider }?.enabled == true || settings.accounts.contains { $0.archived != true && $0.provider == source.provider } || source.id != "local-" + source.provider
    }
    private func accountSources(_ a: AgentAccount) -> [AgentSource] { settings.sources.filter { a.uses($0) } }
    private func accounts(includeArchived: Bool = false) -> [AgentAccount] {
        settings.accounts.filter { a in
            let linked = accountSources(a)
            return (includeArchived || a.archived != true) && (linked.isEmpty || linked.contains(where: inScene))
        }
    }
    private func machineName(_ id: String) -> String { id == "local" ? "本机" : settings.hosts.first { $0.id == id }.map { $0.name.isEmpty ? $0.target : $0.name } ?? "已移除设备" }
    private func accountTarget(_ a: AgentAccount) -> SettingsDestination { .init(accountKey: a.key) }
    private func general(_ section: String) -> SettingsDestination { .init(category: "general", section: section) }
    private func stamp(_ time: Double) -> String { Date(timeIntervalSince1970: time).formatted(date: .numeric, time: .standard) }
    private func quota(_ a: AgentAccount) -> SetupGuide.Check {
        let available = accountSources(a).filter { inScene($0) && settings.sourceEnabled($0) }
        let q = dashboard?.quotas.first { $0.id == a.key }
        var text = "已配置，限额待验证", level = "pending"
        if !a.quotaEnabled { text = "尚未开启“显示并查询额度”"; level = "warn" }
        else if available.isEmpty { text = "待配置：关联账户并启用此场景的 Agent、机器和目录"; level = "warn" }
        else if q?.error != nil { text = "上次查询失败，请查看账户错误；旧数据不代表当前连接正常"; level = "warn" }
        else if let q, ["live", "command"].contains(q.origin), q.updatedAt > 0, available.contains(where: { $0.id == q.sourceId }) {
            text = "上次查询成功 · " + stamp(q.updatedAt); level = "ok"
        }
        return .init(label: Format.provider(a.provider) + " · " + a.name, text: text, destination: accountTarget(a), level: level)
    }
    func rows(_ id: String) -> [SetupGuide.Check] {
        switch id {
        case "usage":
            var result: [SetupGuide.Check] = sources.filter { $0.provider != "deepseek" && relevantSource($0) }.map { source in
                let status = dashboard?.sources.first { $0.id == source.id }?.status
                let enabled = settings.sourceEnabled(source)
                var text = "目录已配置，采集待验证", level = "pending"
                if !enabled { text = "待配置：检查 Agent、机器选择、采集开关及目录绑定"; level = "warn" }
                else if status?.error != nil || status?.partial == true { text = "采集存在异常，请检查目录和连接"; level = "warn" }
                else if let time = status?.updatedAt, time > 0 { text = "上次采集成功 · " + stamp(time); level = "ok" }
                return .init(label: Format.provider(source.provider) + " · " + machineName(source.hostId ?? "local") + " · " + source.name, text: text, destination: .init(provider: source.provider, section: "machines"), level: level)
            }
            if result.isEmpty { result.append(.init(label: "用量采集", text: "需要用量统计时，启用 Agent 并选择机器和日志目录；DeepSeek 余额无需日志目录", destination: .init(section: "machines"), level: "pending")) }
            let active = accounts()
            if active.isEmpty { result.append(.init(label: "账户与限额", text: "待配置：按 Agent 添加账户并连接登录；支持的登录方式以账户页为准", destination: .init(section: "accounts"), level: "pending")) }
            else { result += active.map(quota) }
            return result
        case "network": return networkRows()
        case "refresh":
            var result = [SetupGuide.Check(label: "全局自动刷新", text: "限额 \(settings.refreshSeconds) 秒 · 记录 \(settings.historyInterval) 秒", destination: general("refresh"))]
            if scene != "local" { result.append(.init(label: "服务器监控间隔", text: "前台 \(settings.foregroundInterval) 秒 · 后台 \(settings.serverRefreshSeconds) 秒", destination: general("refresh"))) }
            result += accounts().map { a in .init(label: a.name, text: a.quotaRefreshSeconds.map { "此账户每 \($0) 秒查询一次，覆盖全局设置" } ?? "沿用全局限额间隔：\(settings.refreshSeconds) 秒", destination: accountTarget(a)) }
            return result
        case "hosts":
            if settings.hosts.isEmpty { return [.init(label: "SSH 连接", text: "待配置：导入 SSH config 或手动添加服务器", destination: .init(category: "servers"), level: "pending")] }
            return settings.hosts.map { host in
                let observed = hosts.first { $0.id == host.id }
                let connection = observed?.error != nil ? "上次监控连接失败" : observed?.sample != nil ? "上次监控采样成功（不代表 Agent 联网正常）" : "连接待验证"
                let switches = "Agent 采集\(host.enabled ? "已开启" : "已暂停") · 资源监控\(host.monitoringEnabled != false ? "已开启" : "已暂停") · "
                return .init(label: host.name.isEmpty ? host.target : host.name, text: switches + connection, destination: .init(category: "servers", hostID: host.id), level: observed?.error != nil ? "warn" : "info")
            }
        case "wakeups": return wakeupRows()
        case "panel":
            let selected = PanelAccountPreference.selections(panelPreference, accounts: settings.accounts, order: dashboard?.quotaOrder ?? [])
            let result: [SetupGuide.Check] = accounts(includeArchived: true).map { a in
                let text = a.archived == true ? "账户已归档，恢复后才能显示在面板" : !a.quotaEnabled ? "尚未开启“显示并查询额度”" : dashboard?.quotaOrder == nil ? "面板选择待确认，请前往账户设置查看" : selected[a.provider]?.contains(a.key) == true ? "已选入面板" : "未选入面板，请调整账户选择"
                return .init(label: a.name, text: text, destination: accountTarget(a), level: a.archived == true || !a.quotaEnabled ? "warn" : "info")
            }
            return result.isEmpty ? [.init(label: "面板账户", text: "尚无此场景的账户，请先添加账户", destination: .init(section: "accounts"), level: "pending")] : result
        case "prices":
            let count = dashboard?.pricingGaps.count ?? 0
            return [.init(label: "用量计价", text: dashboard == nil ? "计价状态待确认，请前往模型价格和用量分析查看" : count > 0 ? "当前统计存在 \(count) 项价格缺口" : "当前统计未报告价格缺口；请结合实际用量确认", destination: .init(category: "prices"), level: count > 0 ? "warn" : "info")]
        case "notifications":
            let unexpected = settings.quotaResetNotifications?.unexpected != false ? "开启" : "关闭"
            let natural = settings.quotaResetNotifications?.natural == true ? "开启" : "关闭"
            return [.init(label: "全局重置提醒", text: "非预期重置\(unexpected) · 自然重置\(natural) · 系统权限待测试", destination: general("notifications"))] + accounts().filter { $0.quotaResetNotifications != nil }.map { .init(label: $0.name, text: "此账户设置了单独的提醒规则，请核对覆盖项", destination: accountTarget($0)) }
        default: return []
        }
    }
    private func networkRows() -> [SetupGuide.Check] {
        var result: [SetupGuide.Check] = []
        if scene != "ssh" {
            let mode = ["system":"跟随系统代理", "direct":"直连", "custom":"指定代理"][settings.proxy.mode] ?? "跟随系统代理"
            result.append(.init(label: "本机应用网络", text: mode + " · 到网络代理页测试出站连接", destination: general("network")))
            result += sources.filter { $0.hostId == nil && $0.proxy != nil && $0.proxy?.mode != "inherit" }.map { source in
                .init(label: Format.provider(source.provider) + " · " + source.name, text: "此本机来源覆盖应用代理，请在目录设置中核对", destination: .init(provider: source.provider, section: "machines"))
            }
        }
        if scene != "local" {
            if settings.hosts.isEmpty { result.append(.init(label: "SSH 服务器", text: "待配置：先添加服务器，再配置账户在目标机器上的网络", destination: .init(category: "servers"), level: "pending")) }
            var devices: [SetupGuide.Check] = []
            for a in accounts() {
                for machine in Set(accountSources(a).compactMap(\.hostId)).sorted() {
                    let device = a.deviceSettings?.first { $0.machineId == machine }
                    let text = device?.preCommandResetRequired == true ? "前置命令需重新确认，请在账户设备设置中保存" : device?.preCommand.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false ? "前置命令已保存；联网效果需通过账户查询或运行结果确认" : "未设置前置命令；服务器可直连时无需填写，需要代理时在此配置"
                    devices.append(.init(label: a.name + " · " + machineName(machine), text: text, destination: .init(accountKey: a.key, section: "command", machineID: machine), level: device?.preCommandResetRequired == true ? "warn" : "info"))
                }
            }
            if devices.isEmpty { result.append(.init(label: "服务器账户网络", text: "添加或连接账户后，在该账户的对应机器设置前置命令", destination: .init(section: "accounts"), level: "pending")) }
            result += devices
        }
        return result
    }
    private func wakeupRows() -> [SetupGuide.Check] {
        let target = SettingsDestination(category: "wakeups")
        guard let wakeups else { return [.init(label: "任务状态", text: errors.contains("wakeups") ? "状态读取失败，请前往定时唤醒页查看并重试" : "任务状态待确认", destination: target, level: "pending")] }
        let records = wakeups.filter { record in settings.sources.first { $0.id == record.task.sourceId }.map(inScene) ?? true }
        if records.isEmpty { return [.init(label: "定时任务", text: "尚无此场景的任务；需要时创建并部署", destination: target)] }
        return records.map { record in
            let text: String, level: String
            if !settings.sources.contains(where: { $0.id == record.task.sourceId }) { text = "任务引用的目录已变化，请到任务页检查"; level = "warn" }
            else if let deployment = record.deployment {
                if deployment.state == "pending-removal" { text = "部署待移除，请在任务页完成清理"; level = "warn" }
                else if deployment.state == "settings-pending" { text = "部署待同步，请在任务页重试"; level = "pending" }
                else if deployment.state == "deployment-unconfirmed" { text = "部署待确认，请检查任务状态"; level = "pending" }
                else if deployment.changed { text = "草稿已修改，需要更新部署"; level = "pending" }
                else if deployment.state == "deployed" { text = deployment.enabled ? "已保存启用的部署，实际执行请查看运行记录" : "部署已停用"; level = "info" }
                else { text = "部署状态待确认，请查看任务页"; level = "pending" }
            } else { text = "已保存草稿，尚未部署"; level = "pending" }
            return .init(label: record.task.name, text: text, destination: target, level: level)
        }
    }
}

