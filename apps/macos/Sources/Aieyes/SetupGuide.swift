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
            case .configured: return "已配置"
            case .pending: return "待验证"
            case .missing: return "待配置"
            case .paused: return "已暂停"
            case .queryFailed: return "查询失败"
            case .querySucceeded: return "查询成功"
            case .collectionFailed: return "采集异常"
            case .collected: return "采集成功"
            case .unconfigured: return "未配置"
            case .reconfirm: return "待确认"
            case .custom: return "单独设置"
            case .inherited: return "沿用全局"
            case .archived: return "已归档"
            case .selected: return "已选择"
            case .excluded: return "未选择"
            case .noTasks: return "无任务"
            case .draft: return "未部署"
            case .pendingRemoval: return "待移除"
            case .pendingSync: return "待同步"
            case .modified: return "待部署"
            case .deployed: return "已部署"
            case .stopped: return "已停用"
            case .missingSource: return "目录变化"
            case .priceMissing: return "缺少价格"
            case .noPriceGaps: return "未报缺价"
            case .sampled: return "采样成功"
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
        .init(id: "usage", title: "查看用量与额度", detail: "采集日志统计用量，连接账户查询额度。",
              steps: ["启用 Agent，选择机器和日志目录。", "开启日志采集并添加账户。", "登录或填写 API Key / Cookie。"], help: "",
              actions: [.init(title: "机器与目录", destination: .init(section: "machines")), .init(title: "账户与额度", destination: .init(section: "accounts"))]),
        .init(id: "network", title: "排查网络", detail: "分别检查本机应用网络和服务器账户连接。",
              steps: [], help: "",
              actions: []),
        .init(id: "refresh", title: "调整刷新频率", detail: "分别设置额度查询、日志同步和资源监控频率。",
              steps: ["设置全局刷新间隔。", "按需为账户设置独立查询间隔。"], help: "账户间隔优先于全局设置。调整间隔不会加速当前请求；查询慢时请检查网络和登录状态。",
              actions: [.init(title: "全局刷新设置", destination: .init(category: "general", section: "refresh")), .init(title: "网络设置", destination: .init(category: "general", section: "network"))]),
        .init(id: "hosts", title: "连接服务器", detail: "通过 SSH 采集日志或监控服务器资源。",
              steps: ["导入 SSH 配置或添加服务器。", "按需开启资源监控和 Agent 采集。", "在 Agent 中选择服务器及日志目录。"], help: "SSH 跳板连接沿用 ProxyJump / ProxyCommand。账户代理在对应机器的前置命令中设置。",
              actions: [.init(title: "添加 SSH 服务器", destination: .init(category: "hosts")), .init(title: "选择采集目录", destination: .init(section: "machines"))]),
        .init(id: "wakeups", title: "设置定时唤醒", detail: "按设定时间调用订阅模型。可用于帮助规划 5h 重置时间。",
              steps: ["选择订阅账户与运行机器。", "读取可用模型，设置每日时刻。", "保存草稿，再部署任务。"], help: "修改草稿后需更新部署。执行时间、启用状态和运行记录见任务页。",
              actions: [.init(title: "定时唤醒设置", destination: .init(category: "wakeups"))]),
        .init(id: "panel", title: "选择面板账户", detail: "选择要在本机面板展示的账户。",
              steps: ["开启账户的额度显示与查询。", "选择面板账户或使用自动模式。"], help: "自动模式显示每类前 5 个账户。手动模式可留空，新增账户需手动加入。",
              actions: [.init(title: "调整面板账户", destination: .init(section: "accounts"))]),
        .init(id: "prices", title: "补齐模型价格", detail: "通过日志用量和模型价格计算 API 等价成本。",
              steps: ["补齐模型价格和名称映射。", "同步价格后重新计价。"], help: "额度估值见“估值与采样”，与实际余额分开显示。",
              actions: [.init(title: "模型价格与映射", destination: .init(category: "prices")), .init(title: "估值与采样", destination: .init(section: "valuation"))]),
        .init(id: "notifications", title: "设置额度提醒", detail: "订阅额度提前或到期重置时提醒。",
              steps: ["选择全局或单账户提醒规则。", "发送测试通知并检查系统设置。"], help: "提醒依赖实时额度查询。退出应用或电脑休眠时暂停检测。",
              actions: [.init(title: "通知设置", destination: .init(category: "general", section: "notifications")), .init(title: "账户提醒设置", destination: .init(section: "accounts"))]),
    ]
    static let more: [(String, String, SettingsDestination)] = [
        ("多账户与切换", "添加多个账户，管理各机器的登录与当前使用账户。", .init(section: "accounts")),
        ("Codex API 连接", "在 Codex 中添加 API 连接，按需切换登录方式。", .init(provider: "codex", section: "api")),
        ("桌宠与会话", "选择桌宠、调整大小和会话展示。", .init(category: "general", section: "desktop")),
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
    private func quota(_ a: AgentAccount) -> SetupGuide.Check {
        let available = accountSources(a).filter { inScene($0) && settings.sourceEnabled($0) }
        let q = dashboard?.quotas.first { $0.id == a.key }
        var check = SetupGuide.Check(label: Format.provider(a.provider) + " · " + a.name, text: "刷新账户额度以验证连接", destination: accountTarget(a), level: "pending", status: .pending)
        if !a.quotaEnabled { check = .init(label: check.label, text: "开启“显示并查询额度”", destination: check.destination, level: "warn", status: .paused) }
        else if available.isEmpty { check = .init(label: check.label, text: "关联账户并启用对应机器的采集", destination: check.destination, level: "warn", status: .missing) }
        else if q?.error != nil { check = .init(label: check.label, text: "查看账户错误后重试", destination: check.destination, level: "warn", status: .queryFailed, details: "仍显示上次数据时，请核对更新时间。", updatedAt: q?.updatedAt) }
        else if let q, ["live", "command"].contains(q.origin), q.updatedAt > 0, available.contains(where: { $0.id == q.sourceId }) {
            check = .init(label: check.label, text: "上次实时查询已完成", destination: check.destination, level: "ok", status: .querySucceeded, updatedAt: q.updatedAt)
        }
        return check
    }
    func rows(_ id: String) -> [SetupGuide.Check] {
        switch id {
        case "usage":
            var result: [SetupGuide.Check] = sources.filter { $0.provider != "deepseek" && relevantSource($0) }.map { source in
                let status = dashboard?.sources.first { $0.id == source.id }?.status
                let enabled = settings.sourceEnabled(source)
                var check = SetupGuide.Check(label: Format.provider(source.provider) + " · " + machineName(source.hostId ?? "local") + " · " + source.name, text: "同步记录以验证日志采集", destination: .init(provider: source.provider, section: "machines"), level: "pending", status: .pending)
                if !enabled { check.text = "检查机器、采集开关和目录绑定"; check.level = "warn"; check.status = .missing }
                else if status?.failureReason != nil { check.text = "检查日志目录和机器连接"; check.level = "warn"; check.status = .collectionFailed }
                else if let time = status?.updatedAt, time > 0 { check.text = "上次日志采集已完成"; check.level = "ok"; check.status = .collected; check.updatedAt = time }
                return check
            }
            if result.isEmpty { result.append(.init(label: "用量采集", text: "选择 Agent、机器和日志目录", destination: .init(section: "machines"), level: "pending", status: .missing)) }
            let active = accounts()
            if active.isEmpty { result.append(.init(label: "账户与额度", text: "添加账户并完成认证", destination: .init(section: "accounts"), level: "pending", status: .missing)) }
            else { result += active.map(quota) }
            return result
        case "network": return networkRows()
        case "refresh":
            var result = [SetupGuide.Check(label: "全局自动刷新", text: "额度 \(settings.refreshSeconds) 秒 · 记录 \(settings.historyInterval) 秒", destination: general("refresh"))]
            if scene != "local" { result.append(.init(label: "服务器监控", text: "前台 \(settings.foregroundInterval) 秒 · 后台 \(settings.serverRefreshSeconds) 秒", destination: general("refresh"))) }
            result += accounts().map { a in .init(label: a.name, text: "每 \(a.quotaRefreshSeconds ?? settings.refreshSeconds) 秒查询", destination: accountTarget(a), status: a.quotaRefreshSeconds == nil ? .inherited : .custom) }
            return result
        case "hosts":
            if settings.hosts.isEmpty { return [.init(label: "SSH 连接", text: "导入 SSH 配置或添加服务器", destination: .init(category: "servers"), level: "pending", status: .missing)] }
            return settings.hosts.map { host in
                let observed = hosts.first { $0.id == host.id }
                let switches = "Agent 采集\(host.enabled ? "已开启" : "已暂停") · 资源监控\(host.monitoringEnabled != false ? "已开启" : "已暂停")"
                return .init(label: host.name.isEmpty ? host.target : host.name, text: switches, destination: .init(category: "servers", hostID: host.id), level: observed?.error != nil ? "warn" : "info", status: observed?.error != nil ? .collectionFailed : observed?.sample != nil ? .sampled : .pending, details: "此状态来自资源监控；账户查询结果在账户页查看。")
            }
        case "wakeups": return wakeupRows()
        case "panel":
            let selected = PanelAccountPreference.selections(panelPreference, accounts: settings.accounts, order: dashboard?.quotaOrder ?? [])
            let result: [SetupGuide.Check] = accounts(includeArchived: true).map { a in
                let status: SetupGuide.CheckState = a.archived == true ? .archived : !a.quotaEnabled ? .paused : dashboard?.quotaOrder == nil ? .reconfirm : selected[a.provider]?.contains(a.key) == true ? .selected : .excluded
                let text = a.archived == true ? "恢复账户后可加入面板" : !a.quotaEnabled ? "开启“显示并查询额度”" : status == .selected ? "此账户将在面板显示" : "在账户设置中调整面板选择"
                return .init(label: a.name, text: text, destination: accountTarget(a), level: a.archived == true || !a.quotaEnabled ? "warn" : "info", status: status)
            }
            return result.isEmpty ? [.init(label: "面板账户", text: "添加账户后选择面板显示项", destination: .init(section: "accounts"), level: "pending", status: .missing)] : result
        case "prices":
            let count = dashboard?.pricingGaps.count ?? 0
            return [.init(label: "用量计价", text: dashboard == nil ? "查看模型价格和用量分析" : count > 0 ? "补齐 \(count) 项模型价格" : "当前统计未发现缺价模型", destination: .init(category: "prices"), level: count > 0 ? "warn" : "info", status: dashboard == nil ? .reconfirm : count > 0 ? .priceMissing : .noPriceGaps)]
        case "notifications":
            let unexpected = settings.quotaResetNotifications?.unexpected != false ? "开启" : "关闭"
            let natural = settings.quotaResetNotifications?.natural == true ? "开启" : "关闭"
            return [.init(label: "全局重置提醒", text: "提前重置\(unexpected) · 到期重置\(natural)", destination: general("notifications"), details: "发送测试通知，检查系统是否允许显示。")]
                + accounts().filter { $0.quotaResetNotifications != nil }.map { .init(label: $0.name, text: "查看此账户的提醒规则", destination: accountTarget($0), status: .custom) }
        default: return []
        }
    }
    private func networkRows() -> [SetupGuide.Check] {
        var result: [SetupGuide.Check] = []
        if scene != "ssh" {
            let mode = ["system":"跟随系统代理", "direct":"直连", "custom":"指定代理"][settings.proxy.mode] ?? "跟随系统代理"
            result.append(.init(label: "本机应用网络", text: mode + " · 可测试出站连接", destination: general("network")))
            result += sources.filter { $0.hostId == nil && $0.proxy != nil && $0.proxy?.mode != "inherit" }.map { source in
                .init(label: Format.provider(source.provider) + " · " + source.name, text: "此目录使用独立代理设置", destination: .init(provider: source.provider, section: "machines"), status: .custom)
            }
        }
        if scene != "local" {
            if settings.hosts.isEmpty { result.append(.init(label: "SSH 服务器", text: "添加服务器后配置账户网络", destination: .init(category: "servers"), level: "pending", status: .missing)) }
            var devices: [SetupGuide.Check] = []
            for a in accounts() {
                for machine in Set(accountSources(a).compactMap(\.hostId)).sorted() {
                    let device = a.deviceSettings?.first { $0.machineId == machine }
                    let needsConfirm = device?.preCommandResetRequired == true
                    let configured = device?.preCommand.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                    devices.append(.init(label: a.name + " · " + machineName(machine), text: needsConfirm ? "重新确认并保存前置命令" : configured ? "刷新账户额度以检查连接" : "需要代理时设置前置命令", destination: .init(accountKey: a.key, section: "command", machineID: machine), level: needsConfirm ? "warn" : "info", status: needsConfirm ? .reconfirm : configured ? .configured : .unconfigured, details: "服务器可直连时无需填写；命令内容仅在账户设置中查看。"))
                }
            }
            if devices.isEmpty { result.append(.init(label: "服务器账户网络", text: "连接账户后配置对应机器", destination: .init(section: "accounts"), level: "pending", status: .missing)) }
            result += devices
        }
        return result
    }
    private func wakeupRows() -> [SetupGuide.Check] {
        let target = SettingsDestination(category: "wakeups")
        guard let wakeups else { return [.init(label: "任务状态", text: errors.contains("wakeups") ? "读取失败，请前往任务页重试" : "前往任务页读取状态", destination: target, level: "pending", status: .reconfirm)] }
        let records = wakeups.filter { record in settings.sources.first { $0.id == record.task.sourceId }.map(inScene) ?? true }
        if records.isEmpty { return [.init(label: "定时任务", text: "按需创建并部署任务", destination: target, status: .noTasks)] }
        return records.map { record in
            var check = SetupGuide.Check(label: record.task.name, text: "部署后才会自动运行", destination: target, level: "pending", status: .draft)
            if !settings.sources.contains(where: { $0.id == record.task.sourceId }) { check.text = "检查任务引用的日志目录"; check.level = "warn"; check.status = .missingSource }
            else if let d = record.deployment {
                if d.state == "pending-removal" { check.text = "完成远程部署清理"; check.level = "warn"; check.status = .pendingRemoval }
                else if d.state == "settings-pending" { check.text = "重试同步账户配置"; check.status = .pendingSync }
                else if d.state == "deployment-unconfirmed" { check.text = "检查目标机器的部署状态"; check.status = .reconfirm }
                else if d.changed { check.text = "更新部署以应用草稿修改"; check.status = .modified }
                else if d.state == "deployed" { check.text = d.enabled ? "实际执行结果见运行记录" : "启用后恢复定时运行"; check.level = "info"; check.status = d.enabled ? .deployed : .stopped }
                else { check.text = "前往任务页检查部署状态"; check.status = .reconfirm }
            }
            return check
        }
    }
}
