/* One list behind the general page chips, the search box and the aieyes:// settings
   links, so a new page cannot be missed in any of them. Mirrors SettingsIndex.swift. */
(() => {
  const generalPageKeys = ['appearance', 'panel', 'desktop', 'system', 'network', 'refresh', 'notifications', 'webhooks', 'advanced', 'updates'];
  const pageTitles = { appearance: "外观", panel: "面板", desktop: "桌宠与会话", system: "启动与系统", network: "网络代理", refresh: "刷新与历史", notifications: "通知", advanced: "高级", updates: "更新与关于" };
  const categoryTitles = { integrations: "接入与账户", hosts: "机器与采集", prices: "模型价格", wakeups: "定时唤醒" };
  const generalPageTitle = (key) => key === 'webhooks' ? 'Webhook' : AieyesI18n.t(pageTitles[key] ?? pageTitles.updates);
  const categoryTitle = (key) => AieyesI18n.t(categoryTitles[key] ?? "设置");
  // `keywords` are matched, never displayed, so Chinese ones stay template literals outside the catalog.
  const entry = (title, category, page, keywords = [], platforms = null) => ({ title, category, page, keywords, platforms });
  const entries = [
    entry("语言", 'general', 'appearance', ["language", "English", `简体中文`]),
    entry("主题", 'general', 'appearance', ["theme", "dark", "light", `深色`, `浅色`]),
    entry("强调色", 'general', 'appearance', ["accent", "color"]),
    entry("主窗口材质", 'general', 'appearance', ["material", "mica", "acrylic"], ['windows']),
    entry("悬浮面板材质", 'general', 'appearance', ["material", "mica", "acrylic"], ['windows']),
    entry("面板额度样式", 'general', 'appearance', ["quota", "ring", `圆环`]),
    entry("主窗口额度样式", 'general', 'appearance', ["quota", "ring", `圆环`]),
    entry("图表配色", 'general', 'appearance', ["chart", "palette"]),
    entry("图表对比度", 'general', 'appearance', ["chart", "contrast"]),
    entry("面板宽度", 'general', 'panel', ["panel", "width", `紧凑`]),
    entry("面板显示账户", 'general', 'panel', ["panel", "accounts"]),
    entry("账户顺序", 'general', 'panel', ["order", "sort", `排序`]),
    entry("显示方式", 'general', 'desktop', ["pet", "tray", `桌面宠物`, `托盘`]),
    entry("桌宠缩放", 'general', 'desktop', ["pet", "scale", `大小`]),
    entry("显示活跃会话数", 'general', 'desktop', ["pet", "count", `会话数`]),
    entry("托盘图标显示额度圆环", 'general', 'desktop', ["tray", "ring", "quota", `托盘`]),
    entry("登录时启动 Aieyes", 'general', 'system', ["login", "startup", "autostart", `开机`]),
    entry("应用链接", 'general', 'system', ["aieyes://", "link", "url", `命令`]),
    entry("网络代理", 'general', 'network', ["proxy", "http", "socks"]),
    entry("额度查询", 'general', 'refresh', ["refresh", "interval", `间隔`]),
    entry("日志同步", 'general', 'refresh', ["refresh", "interval", `间隔`]),
    entry("服务器前台", 'general', 'refresh', ["refresh", "interval", `间隔`]),
    entry("服务器后台", 'general', 'refresh', ["refresh", "interval", `间隔`]),
    entry("精确计数", 'general', 'refresh', ["fork", "history"]),
    entry("全量重扫", 'general', 'refresh', ["rescan", "history", `历史记录`]),
    entry("会话提示音", 'general', 'notifications', ["sound"]),
    entry("订阅额度重置提醒", 'general', 'notifications', ["quota", "reset", "notification"]),
    entry("系统通知", 'general', 'notifications', ["notification", "permission", `权限`]),
    entry("服务器告警", 'general', 'notifications', ["alert", "cpu", "memory", `内存`, `阈值`]),
    entry("发送 Webhook 通知", 'general', 'webhooks', ["webhook", "slack", "bark", `飞书`, `企业微信`]),
    entry("通知事件", 'general', 'webhooks', ["webhook", "event", `会话完成`, `等待输入`, `额度重置`]),
    entry("隐藏免费账户的额度卡片", 'general', 'advanced', ["free", "quota"]),
    entry("后台采样", 'general', 'advanced', ["sampling", "estimate", `估值`]),
    entry("检查更新", 'general', 'updates', ["update", "version", `版本`]),
    entry("本次更新", 'general', 'updates', ["release notes", "changelog", `更新内容`]),
    entry("接入与账户", 'integrations', null, ["agent", "account", "api key", `登录`]),
    entry("机器与采集", 'hosts', null, ["ssh", "host", "server", `目录`, `服务器`]),
    entry("模型价格", 'prices', null, ["price", "pricing", "mapping", `映射`]),
    entry("定时唤醒", 'wakeups', null, ["wakeup", "schedule"])
  ];
  const pagePath = (category, page) => page ? `${AieyesI18n.t("设置")} › ${generalPageTitle(page)}` : categoryTitle(category);
  // A page whose content this platform (or this window) does not carry is not offered.
  const available = (key) => (window.AieyesSettingsPages?.has(key) ?? false) && (key !== 'panel' || !PANEL);
  const platform = () => document.documentElement.dataset.platform ?? '';
  const search = (query) => {
    const needle = String(query ?? '').trim().toLowerCase();
    if (!needle) return [];
    return entries.filter((row) => (row.category !== 'general' || available(row.page)) && (!row.platforms || row.platforms.includes(platform())) && [row.title, AieyesI18n.t(row.title), pagePath(row.category, row.page), ...row.keywords].some((text) => text.toLowerCase().includes(needle)));
  };
  window.AieyesSettingsIndex = { generalPageKeys, generalPageTitle, categoryTitle, pagePath, available, entries, search };
})();
