import Foundation

/// One list behind the general page chips, the `aieyes://settings` links, the sidebar
/// search and the snapshot renderer, so a new page cannot be missed in any of them.
enum SettingsIndex {
    /// General settings pages in display order.
    static let generalPageKeys = ["appearance", "panel", "desktop", "system", "network", "refresh", "notifications", "webhooks", "channels", "advanced", "updates"]

    static func generalPageTitle(_ key: String) -> String {
        switch key {
        case "appearance": return L("外观")
        case "panel": return L("面板")
        case "desktop": return L("桌宠与会话")
        case "system": return L("启动与系统")
        case "network": return L("网络代理")
        case "refresh": return L("刷新与历史")
        case "notifications": return L("通知")
        case "webhooks": return "Webhook"
        case "channels": return L("聊天渠道")
        case "advanced": return L("高级")
        default: return L("更新与关于")
        }
    }

    static func categoryTitle(_ key: String) -> String {
        switch key {
        case "integrations": return L("接入与账户")
        case "servers": return L("机器与采集")
        case "prices": return L("模型价格")
        case "wakeups": return L("定时唤醒")
        default: return L("设置")
        }
    }

    /// A setting the sidebar search can open. `title` is the source string of the control's label.
    struct Entry: Identifiable {
        let title: String
        let category: String
        var page: String? = nil
        /// Extra terms that are not part of the label, such as the English name of a feature.
        /// They are matched, never displayed, so Chinese ones are raw strings outside the catalog.
        var keywords: [String] = []
        var id: String { category + "/" + (page ?? "") + "/" + title }
        var destination: SettingsDestination { SettingsDestination(category: category, section: page) }
        /// Where the setting lives, e.g. "设置 › 通知".
        var path: String {
            guard category == "general", let page else { return SettingsIndex.categoryTitle(category) }
            return SettingsIndex.categoryTitle("general") + " › " + SettingsIndex.generalPageTitle(page)
        }
        /// Both the source and the displayed label match, so search works in either language.
        fileprivate var haystack: [String] { [title, L(title), path] + keywords }
    }

    static let entries: [Entry] = [
        Entry(title: "语言", category: "general", page: "appearance", keywords: ["language", "English", "简体中文"]),
        Entry(title: "主题", category: "general", page: "appearance", keywords: ["theme", "dark", "light", #"深色"#, #"浅色"#]),
        Entry(title: "强调色", category: "general", page: "appearance", keywords: ["accent", "color"]),
        Entry(title: "面板额度样式", category: "general", page: "appearance", keywords: ["quota", "ring", #"圆环"#]),
        Entry(title: "主窗口额度样式", category: "general", page: "appearance", keywords: ["quota", "ring", #"圆环"#]),
        Entry(title: "图表配色", category: "general", page: "appearance", keywords: ["chart", "palette"]),
        Entry(title: "图表对比度", category: "general", page: "appearance", keywords: ["chart", "contrast"]),
        Entry(title: "面板宽度", category: "general", page: "panel", keywords: ["panel", "width", #"紧凑"#]),
        Entry(title: "面板显示账户", category: "general", page: "panel", keywords: ["panel", "accounts"]),
        Entry(title: "账户顺序", category: "general", page: "panel", keywords: ["order", "sort", #"排序"#]),
        Entry(title: "显示桌宠", category: "general", page: "desktop", keywords: ["pet", #"桌面宠物"#]),
        Entry(title: "全屏时隐藏", category: "general", page: "desktop", keywords: ["pet", "fullscreen"]),
        Entry(title: "菜单栏", category: "general", page: "desktop", keywords: ["menu bar", #"显示内容"#, #"额度圆环"#]),
        Entry(title: "登录时启动 Aieyes", category: "general", page: "system", keywords: ["login", "startup", "autostart", #"开机"#]),
        Entry(title: "在 Dock 图标上显示等待输入的会话数", category: "general", page: "system", keywords: ["dock", "badge", #"徽标"#]),
        Entry(title: "应用链接", category: "general", page: "system", keywords: ["aieyes://", "link", "url", "shortcuts", #"快捷指令"#]),
        Entry(title: "网络代理", category: "general", page: "network", keywords: ["proxy", "http", "socks"]),
        Entry(title: "额度查询", category: "general", page: "refresh", keywords: ["refresh", "interval", #"间隔"#]),
        Entry(title: "日志同步", category: "general", page: "refresh", keywords: ["refresh", "interval", #"间隔"#]),
        Entry(title: "服务器前台", category: "general", page: "refresh", keywords: ["refresh", "interval", #"间隔"#]),
        Entry(title: "服务器后台", category: "general", page: "refresh", keywords: ["refresh", "interval", #"间隔"#]),
        Entry(title: "精确计数", category: "general", page: "refresh", keywords: ["fork", "history"]),
        Entry(title: "全量重扫", category: "general", page: "refresh", keywords: ["rescan", "history", #"历史记录"#]),
        Entry(title: "会话提示音", category: "general", page: "notifications", keywords: ["sound"]),
        Entry(title: "订阅额度重置提醒", category: "general", page: "notifications", keywords: ["quota", "reset", "notification"]),
        Entry(title: "系统通知", category: "general", page: "notifications", keywords: ["notification", "permission", #"权限"#]),
        Entry(title: "服务器告警", category: "general", page: "notifications", keywords: ["alert", "cpu", "memory", #"内存"#, #"阈值"#]),
        Entry(title: "发送 Webhook 通知", category: "general", page: "webhooks", keywords: ["webhook", "slack", "bark", #"飞书"#, #"企业微信"#]),
        Entry(title: "通知事件", category: "general", page: "webhooks", keywords: ["webhook", "event", #"会话完成"#, #"等待输入"#, #"额度重置"#]),
        Entry(title: "飞书会话控制", category: "general", page: "channels", keywords: ["channel", "feishu", "lark", #"私聊"#, #"配对"#, #"回复"#, #"中断"#, #"继续"#]),
        Entry(title: "隐藏免费账户的额度卡片", category: "general", page: "advanced", keywords: ["free", "quota"]),
        Entry(title: "后台采样", category: "general", page: "advanced", keywords: ["sampling", "estimate", #"估值"#]),
        Entry(title: "检查更新", category: "general", page: "updates", keywords: ["update", "version", #"版本"#]),
        Entry(title: "本次更新", category: "general", page: "updates", keywords: ["release notes", "changelog", #"更新内容"#]),
        Entry(title: "接入与账户", category: "integrations", keywords: ["agent", "account", "api key", #"登录"#]),
        Entry(title: "机器与采集", category: "servers", keywords: ["ssh", "host", "server", #"目录"#, #"服务器"#]),
        Entry(title: "模型价格", category: "prices", keywords: ["price", "pricing", "mapping", #"映射"#]),
        Entry(title: "定时唤醒", category: "wakeups", keywords: ["wakeup", "schedule"]),
    ]

    /// Settings whose label, location or keywords contain the query; empty for a blank query.
    static func search(_ query: String) -> [Entry] {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !needle.isEmpty else { return [] }
        return entries.filter { entry in entry.haystack.contains { $0.lowercased().contains(needle) } }
    }
}
