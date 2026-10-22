import Foundation

/// Goal content mirrors the desktop guide; configuration never implies connectivity.
enum SetupGuide {
    static func needsSetup(_ settings: Settings) -> Bool {
        settings.accounts.isEmpty && settings.hosts.isEmpty && settings.sources.allSatisfy { $0.id == "local-" + $0.provider } && settings.proxy.mode == "system"
    }
    struct Action { let title: String; var destination: SettingsDestination }
    struct Goal: Identifiable {
        let id: String, title: String, detail: String
        let steps: [String], help: String
        let actions: [Action]
    }
    enum CheckState: String {
        case configured, pending, missing, paused, queryFailed, querySucceeded, collectionFailed, collected
        case unconfigured, reconfirm, custom, inherited, archived, selected, excluded, noTasks, draft
        case pendingRemoval, pendingSync, modified, deployed, stopped, missingSource, priceMissing, noPriceGaps, sampled
        var title: String {
            switch self {
            case .configured: return L("已配置")
            case .pending: return L("待验证")
            case .missing: return L("待配置")
            case .paused: return L("已暂停")
            case .queryFailed: return L("查询失败")
            case .querySucceeded: return L("查询成功")
            case .collectionFailed: return L("采集异常")
            case .collected: return L("采集成功")
            case .unconfigured: return L("未配置")
            case .reconfirm: return L("待确认")
            case .custom: return L("单独设置")
            case .inherited: return L("沿用全局")
            case .archived: return L("已归档")
            case .selected: return L("已选择")
            case .excluded: return L("未选择")
            case .noTasks: return L("无任务")
            case .draft: return L("未部署")
            case .pendingRemoval: return L("待移除")
            case .pendingSync: return L("待同步")
            case .modified: return L("待部署")
            case .deployed: return L("已部署")
            case .stopped: return L("已停用")
            case .missingSource: return L("目录变化")
            case .priceMissing: return L("缺少价格")
            case .noPriceGaps: return L("未报缺价")
            case .sampled: return L("采样成功")
            }
        }
    }
    struct Check {
        let label: String
        var text: String
        var destination: SettingsDestination
        var level = "info"
        var status: CheckState = .configured
        var details = ""
        var updatedAt: Double? = nil
    }
    static let goals: [Goal] = [
        .init(id: "usage", title: L("查看用量与额度"), detail: L("采集日志统计用量，连接账户查询额度。"),
              steps: [L("启用 Agent，选择机器和日志目录。"), L("开启日志采集并添加账户。"), L("登录或填写 API Key / Cookie。")], help: "",
              actions: [.init(title: L("机器与目录"), destination: .init(section: "machines")), .init(title: L("账户与额度"), destination: .init(section: "accounts"))]),
        .init(id: "network", title: L("排查网络"), detail: L("分别检查本机应用网络和服务器账户连接。"),
              steps: [], help: "",
              actions: []),
        .init(id: "refresh", title: L("调整刷新频率"), detail: L("分别设置额度查询、日志同步和资源监控频率。"),
              steps: [L("设置全局刷新间隔。"), L("按需为账户设置独立查询间隔。")], help: L("账户间隔优先于全局设置。调整间隔不会加速当前请求；查询慢时请检查网络和登录状态。"),
              actions: [.init(title: L("全局刷新设置"), destination: .init(category: "general", section: "refresh")), .init(title: L("网络设置"), destination: .init(category: "general", section: "network"))]),
        .init(id: "hosts", title: L("连接服务器"), detail: L("通过 SSH 采集日志或监控服务器资源。"),
              steps: [L("导入 SSH 配置或添加服务器。"), L("按需开启资源监控和 Agent 采集。"), L("在 Agent 中选择服务器及日志目录。")], help: L("SSH 跳板连接沿用 ProxyJump / ProxyCommand。账户代理在对应机器的前置命令中设置。"),
              actions: [.init(title: L("添加 SSH 服务器"), destination: .init(category: "hosts")), .init(title: L("选择采集目录"), destination: .init(section: "machines"))]),
        .init(id: "wakeups", title: L("设置定时唤醒"), detail: L("按设定时间调用订阅模型。可用于帮助规划 5h 重置时间。"),
              steps: [L("选择订阅账户与运行机器。"), L("读取可用模型，设置每日时刻。"), L("保存草稿，再部署任务。")], help: L("修改草稿后需更新部署。执行时间、启用状态和运行记录见任务页。"),
              actions: [.init(title: L("定时唤醒设置"), destination: .init(category: "wakeups"))]),
        .init(id: "panel", title: L("选择面板账户"), detail: L("选择要在本机面板展示的账户。"),
              steps: [L("开启账户的额度显示与查询。"), L("选择面板账户或使用自动模式。")], help: L("自动模式显示每类前 5 个账户。手动模式可留空；新增账户默认加入面板，可随时取消。"),
              actions: [.init(title: L("调整面板账户"), destination: .init(section: "accounts"))]),
        .init(id: "prices", title: L("补齐模型价格"), detail: L("通过日志用量和模型价格计算 API 等价成本。"),
              steps: [L("补齐模型价格和名称映射。"), L("同步价格后重新计价。")], help: L("额度估值见“估值与采样”，与实际余额分开显示。"),
              actions: [.init(title: L("模型价格与映射"), destination: .init(category: "prices")), .init(title: L("估值与采样"), destination: .init(section: "valuation"))]),
        .init(id: "notifications", title: L("设置额度提醒"), detail: L("订阅额度提前或到期重置时提醒。"),
              steps: [L("选择全局或单账户提醒规则。"), L("发送测试通知并检查系统设置。")], help: L("提醒依赖实时额度查询。退出应用或电脑休眠时暂停检测。"),
              actions: [.init(title: L("通知设置"), destination: .init(category: "general", section: "notifications")), .init(title: L("账户提醒设置"), destination: .init(section: "accounts"))]),
    ]
    static let more: [(String, String, SettingsDestination)] = [
        (L("多账户与切换"), L("添加多个账户，管理各机器的登录与当前使用账户。"), .init(section: "accounts")),
        (L("Codex API 连接"), L("在 Codex 中添加 API 连接，按需切换登录方式。"), .init(provider: "codex", section: "api")),
        (L("桌宠与会话"), L("选择桌宠、调整大小和会话展示。"), .init(category: "general", section: "desktop")),
        (L("外观"), L("选择主题、强调色和图表配色。"), .init(category: "general", section: "appearance")),
        (L("更新与关于"), L("查看版本、检查更新及更新设置。"), .init(category: "general", section: "updates")),
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
    private func machineName(_ id: String) -> String { id == "local" ? L("本机") : settings.hosts.first { $0.id == id }.map { $0.name.isEmpty ? $0.target : $0.name } ?? L("已移除设备") }
    private func accountTarget(_ a: AgentAccount) -> SettingsDestination { .init(accountKey: a.key) }
    private func general(_ section: String) -> SettingsDestination { .init(category: "general", section: section) }
    private func quota(_ a: AgentAccount) -> SetupGuide.Check {
        let available = accountSources(a).filter { inScene($0) && settings.sourceEnabled($0) }
        let q = dashboard?.quotas.first { $0.id == a.key }
        var check = SetupGuide.Check(label: Format.provider(a.provider) + " · " + a.name, text: L("刷新账户额度以验证连接"), destination: accountTarget(a), level: "pending", status: .pending)
        if !a.quotaEnabled { check = .init(label: check.label, text: L("开启“显示并查询额度”"), destination: check.destination, level: "warn", status: .paused) }
        else if available.isEmpty { check = .init(label: check.label, text: L("关联账户并启用对应机器的采集"), destination: check.destination, level: "warn", status: .missing) }
        else if q?.error != nil { check = .init(label: check.label, text: L("查看账户错误后重试"), destination: check.destination, level: "warn", status: .queryFailed, details: L("仍显示上次数据时，请核对更新时间。"), updatedAt: q?.updatedAt) }
        else if let q, ["live", "command"].contains(q.origin), q.updatedAt > 0, available.contains(where: { $0.id == q.sourceId }) {
            check = .init(label: check.label, text: L("上次实时查询已完成"), destination: check.destination, level: "ok", status: .querySucceeded, updatedAt: q.updatedAt)
        }
        return check
    }
    func rows(_ id: String) -> [SetupGuide.Check] {
        switch id {
        case "usage":
            var result: [SetupGuide.Check] = sources.filter { $0.provider != "deepseek" && relevantSource($0) }.map { source in
                let status = dashboard?.sources.first { $0.id == source.id }?.status
                let enabled = settings.sourceEnabled(source)
                var check = SetupGuide.Check(label: Format.provider(source.provider) + " · " + machineName(source.hostId ?? "local") + " · " + source.name, text: L("同步记录以验证日志采集"), destination: .init(provider: source.provider, section: "machines"), level: "pending", status: .pending)
                if !enabled { check.text = L("检查机器、采集开关和目录绑定"); check.level = "warn"; check.status = .missing }
                else if status?.failureReason != nil { check.text = L("检查日志目录和机器连接"); check.level = "warn"; check.status = .collectionFailed }
                else if let time = status?.updatedAt, time > 0 { check.text = L("上次日志采集已完成"); check.level = "ok"; check.status = .collected; check.updatedAt = time }
                return check
            }
            if result.isEmpty { result.append(.init(label: L("用量采集"), text: L("选择 Agent、机器和日志目录"), destination: .init(section: "machines"), level: "pending", status: .missing)) }
            let active = accounts()
            if active.isEmpty { result.append(.init(label: L("账户与额度"), text: L("添加账户并完成认证"), destination: .init(section: "accounts"), level: "pending", status: .missing)) }
            else { result += active.map(quota) }
            return result
        case "network": return networkRows()
        case "refresh":
            var result = [SetupGuide.Check(label: L("全局自动刷新"), text: L("额度 {0} 秒 · 记录 {1} 秒" , settings.refreshSeconds, settings.historyInterval), destination: general("refresh"))]
            if scene != "local" { result.append(.init(label: L("服务器监控"), text: L("前台 {0} 秒 · 后台 {1} 秒" , settings.foregroundInterval, settings.serverRefreshSeconds), destination: general("refresh"))) }
            result += accounts().map { a in .init(label: a.name, text: L("每 {0} 秒查询" , a.quotaRefreshSeconds ?? settings.refreshSeconds), destination: accountTarget(a), status: a.quotaRefreshSeconds == nil ? .inherited : .custom) }
            return result
        case "hosts":
            if settings.hosts.isEmpty { return [.init(label: L("SSH 连接"), text: L("导入 SSH 配置或添加服务器"), destination: .init(category: "servers"), level: "pending", status: .missing)] }
            return settings.hosts.map { host in
                let observed = hosts.first { $0.id == host.id }
                let switches = L("Agent 采集{0} · 资源监控{1}" , host.enabled ? L("已开启") : L("已暂停"), host.monitoringEnabled != false ? L("已开启") : L("已暂停"))
                return .init(label: host.name.isEmpty ? host.target : host.name, text: switches, destination: .init(category: "servers", hostID: host.id), level: observed?.error != nil ? "warn" : "info", status: observed?.error != nil ? .collectionFailed : observed?.sample != nil ? .sampled : .pending, details: L("此状态来自资源监控；账户查询结果在账户页查看。"))
            }
        case "wakeups": return wakeupRows()
        case "panel":
            let selected = PanelAccountPreference.selections(panelPreference, accounts: settings.accounts, order: dashboard?.quotaOrder ?? [])
            let result: [SetupGuide.Check] = accounts(includeArchived: true).map { a in
                let status: SetupGuide.CheckState = a.archived == true ? .archived : !a.quotaEnabled ? .paused : dashboard?.quotaOrder == nil ? .reconfirm : selected[a.provider]?.contains(a.key) == true ? .selected : .excluded
                let text = a.archived == true ? L("恢复账户后可加入面板") : !a.quotaEnabled ? L("开启“显示并查询额度”") : status == .selected ? L("此账户将在面板显示") : L("在账户设置中调整面板选择")
                return .init(label: a.name, text: text, destination: accountTarget(a), level: a.archived == true || !a.quotaEnabled ? "warn" : "info", status: status)
            }
            return result.isEmpty ? [.init(label: L("面板账户"), text: L("添加账户后选择面板显示项"), destination: .init(section: "accounts"), level: "pending", status: .missing)] : result
        case "prices":
            let count = dashboard?.pricingGaps.count ?? 0
            return [.init(label: L("用量计价"), text: dashboard == nil ? L("查看模型价格和用量分析") : count > 0 ? L("补齐 {0} 项模型价格" , count) : L("当前统计未发现缺价模型"), destination: .init(category: "prices"), level: count > 0 ? "warn" : "info", status: dashboard == nil ? .reconfirm : count > 0 ? .priceMissing : .noPriceGaps)]
        case "notifications":
            let unexpected = settings.quotaResetNotifications?.unexpected != false ? L("开启") : L("关闭")
            let natural = settings.quotaResetNotifications?.natural == true ? L("开启") : L("关闭")
            return [.init(label: L("全局重置提醒"), text: L("提前重置{0} · 到期重置{1}" , unexpected, natural), destination: general("notifications"), details: L("发送测试通知，检查系统是否允许显示。"))]
                + accounts().filter { $0.quotaResetNotifications != nil }.map { .init(label: $0.name, text: L("查看此账户的提醒规则"), destination: accountTarget($0), status: .custom) }
        default: return []
        }
    }
    private func networkRows() -> [SetupGuide.Check] {
        var result: [SetupGuide.Check] = []
        if scene != "ssh" {
            let mode = ["system":L("跟随系统代理"), "direct":L("直连"), "custom":L("指定代理")][settings.proxy.mode] ?? L("跟随系统代理")
            result.append(.init(label: L("本机应用网络"), text: mode + L(" · 可测试出站连接"), destination: general("network")))
            result += sources.filter { $0.hostId == nil && $0.proxy != nil && $0.proxy?.mode != "inherit" }.map { source in
                .init(label: Format.provider(source.provider) + " · " + source.name, text: L("此目录使用独立代理设置"), destination: .init(provider: source.provider, section: "machines"), status: .custom)
            }
        }
        if scene != "local" {
            if settings.hosts.isEmpty { result.append(.init(label: L("SSH 服务器"), text: L("添加服务器后配置账户网络"), destination: .init(category: "servers"), level: "pending", status: .missing)) }
            var devices: [SetupGuide.Check] = []
            for a in accounts() {
                for machine in Set(accountSources(a).compactMap(\.hostId)).sorted() {
                    let device = a.deviceSettings?.first { $0.machineId == machine }
                    let needsConfirm = device?.preCommandResetRequired == true
                    let configured = device?.preCommand.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                    devices.append(.init(label: a.name + " · " + machineName(machine), text: needsConfirm ? L("重新确认并保存前置命令") : configured ? L("刷新账户额度以检查连接") : L("需要代理时设置前置命令"), destination: .init(accountKey: a.key, section: "command", machineID: machine), level: needsConfirm ? "warn" : "info", status: needsConfirm ? .reconfirm : configured ? .configured : .unconfigured, details: L("服务器可直连时无需填写；命令内容仅在账户设置中查看。")))
                }
            }
            if devices.isEmpty { result.append(.init(label: L("服务器账户网络"), text: L("连接账户后配置对应机器"), destination: .init(section: "accounts"), level: "pending", status: .missing)) }
            result += devices
        }
        return result
    }
    private func wakeupRows() -> [SetupGuide.Check] {
        let target = SettingsDestination(category: "wakeups")
        guard let wakeups else { return [.init(label: L("任务状态"), text: errors.contains("wakeups") ? L("读取失败，请前往任务页重试") : L("前往任务页读取状态"), destination: target, level: "pending", status: .reconfirm)] }
        let records = wakeups.filter { record in settings.sources.first { $0.id == record.task.sourceId }.map(inScene) ?? true }
        if records.isEmpty { return [.init(label: L("定时任务"), text: L("按需创建并部署任务"), destination: target, status: .noTasks)] }
        return records.map { record in
            var check = SetupGuide.Check(label: record.task.name, text: L("部署后才会自动运行"), destination: target, level: "pending", status: .draft)
            if !settings.sources.contains(where: { $0.id == record.task.sourceId }) { check.text = L("检查任务引用的日志目录"); check.level = "warn"; check.status = .missingSource }
            else if let d = record.deployment {
                if d.state == "pending-removal" { check.text = L("完成远程部署清理"); check.level = "warn"; check.status = .pendingRemoval }
                else if d.state == "settings-pending" { check.text = L("重试同步账户配置"); check.status = .pendingSync }
                else if d.state == "deployment-unconfirmed" { check.text = L("检查目标机器的部署状态"); check.status = .reconfirm }
                else if d.changed { check.text = L("更新部署以应用草稿修改"); check.status = .modified }
                else if d.state == "deployed" { check.text = d.enabled ? L("实际执行结果见运行记录") : L("启用后恢复定时运行"); check.level = "info"; check.status = d.enabled ? .deployed : .stopped }
                else { check.text = L("前往任务页检查部署状态"); check.status = .reconfirm }
            }
            return check
        }
    }
}
