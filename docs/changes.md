# 实现摘要与尚存问题

详细实施、审计、验收记录仅保留在本地 `docs/internal/`（不入库）；发布说明见 [release-notes/](release-notes/)。本文按领域汇总 2026-10 以来的主要变更，日期与版本为近似对应。

## 新增功能

### Agent 接入与账户

- 多账户与 Agents 设置（10-09，v0.1.22）：按 Agent 选择机器与记录目录，同一 Agent 多账户、归档与恢复，Codex 登录、已有登录接入和独立限额刷新。见 [agents-and-accounts-settings.md](agents-and-accounts-settings.md)、[codex-accounts.md](codex-accounts.md)。
- Antigravity 统一（10-09，v0.1.21）：合并 agy 与 Antigravity，接入 CLI 会话 Token 历史与模型组估值；v0.1.27 起支持登录检查、邮箱与订阅识别。见 [refresh-monitoring-antigravity.md](refresh-monitoring-antigravity.md)。
- Codex 账户切换（10-11，v0.1.30）：显示“已登录设备／使用中设备”，切换前按进程身份确认退出范围，保留独立 API 实例与沙箱服务。
- Codex API 连接（10-13，v0.1.35）：在 Agent 接入页管理兼容 Responses API 的连接，与订阅账户共用 Codex home 和对话历史，切换前预检受影响进程，密钥只存于目标设备。见 [codex-api.md](codex-api.md)。
- 五类订阅接入（10-15，v0.1.40）：ZCode、Qoder、Grok、Cursor、OpenCode 的额度或 API 余额查询，OpenCode 本机 SQLite 历史。见 [agent-subscriptions.md](agent-subscriptions.md)、[qoder-usage.md](qoder-usage.md)。
- SSH 主机导入与配置指南（10-16，v0.1.43）：从本机 SSH config 预览并导入主机，按目标组织的配置向导。
- 五类原生 Agent（10-19，v0.1.49）：Kimi Code、DeepSeek Harness、Pi、Oh My Pi、CodeBuddy 的本机与 SSH 历史、会话状态；多种客户端的本机原生 API 配置入口；Kimi 用量与钱包、DeepSeek 官方余额。见 [adapters.md](adapters.md)。
- Qoder 身份与状态（10-21，v0.1.53）：本机与 SSH 读取当前登录身份，环境配置与历史状态恢复。见 [qoder-usage.md](qoder-usage.md)。
- 已有账户与原生连接扫描（10-22，v0.1.54）：按机器或按 Agent 扫描已有登录和原生 API 连接，只保存定位与非敏感元数据。
- Agent 入口管理（10-28 至 10-29，v0.1.61–v0.1.64）：常用入口、搜索选择、拖动排序；移除入口只暂停采集并保留历史。
- 零配置账户发现与新 Agent（10-31，v0.1.68）：Copilot、Cline、Kilo、Kiro、Roo、Trae／Trae CN、WorkBuddy 等的账户发现、历史导入和额度；ChatGPT Work 与 Codex 分开筛选、共享账户与额度；独立 Qoder CN；启动、凭据或 WAL 变化及 SSH 下自动发现账户。各 Agent 支持范围见 [agent-subscriptions.md](agent-subscriptions.md)。
- 账户发现收尾（10-31，v0.1.69）：保留归档账户，不再重新发现已删除身份，清理过期凭据产生的陈旧账户。

### 额度与估值

- 限额价值估算（10-05）：手动采样积累 5h／7d 样本，异常进入待确认并保留最后有效区间。
- 自动估值（10-12，v0.1.33）：自动积累 5h、7d 与 credits 的可信样本，跟随共享 Codex home 的实际登录账户。v0.1.35 起按原生请求增量与实际路由归因，不再要求选择订阅模式。见 [credits-and-wakeups.md](credits-and-wakeups.md)。
- 精确计数（10-17，v0.1.45；10-22 扩展到全部 Agent，v0.1.55）：可选扣除 fork 继承的父历史，切换后重读并按原价格重算受影响估值。见 [manual-history-rescan.md](manual-history-rescan.md)。
- 订阅额度重置提醒（10-15，v0.1.40）：区分非预期重置与自然重置，按账户覆盖，三端系统通知。见 [quota-reset-notifications.md](quota-reset-notifications.md)。
- 滚动估值（10-19，v0.1.51）：最近 90 天有效区间、七天半衰权重；Antigravity 各模型组与窗口独立核验，支持后台恢复已有历史证据。见 [overview-and-rolling-estimates.md](overview-and-rolling-estimates.md)。
- Codex 多账户联合估值（10-21，v0.1.53）：结合会话创建者、登录身份观测、切换边界与请求侧额度指纹核验归属；v0.1.57 增加指定账户、时间范围的历史恢复与预览。见 [overview-and-rolling-estimates.md](overview-and-rolling-estimates.md)。
- 价格智能引用（10-22，v0.1.54）：按模型名称、版本、日期与供应商生成可编辑映射；v0.1.70 支持多关键词乱序搜索。
- 暂停设备与估值覆盖（10-25，v0.1.60）：暂停设备退出实时覆盖范围，恢复时重新核对。
- Gemini 4 Argon 独立估值与周额度（11-01，v0.1.70）。

### 会话状态与桌宠

- 动画桌宠（10-13，v0.1.35）：三端统一随会话状态变化的桌宠，支持导入 Codex Pet 格式，默认附带月薪喵。见 [desktop-pets.md](desktop-pets.md)、[pets.md](pets.md)。
- 会话监测扩展（10-13，v0.1.35）：本机与 SSH 上的 Codex、Claude Code、AGY 会话状态，Claude Code 与 AGY 通过生命周期 hooks 接入。见 [claude-session-sources.md](claude-session-sources.md)。
- 桌宠缩放与多会话汇总（10-13，v0.1.36）：50%–200% 缩放，按状态优先级选主动作并显示各状态数量。
- 桌宠库（10-14，v0.1.38）：支持 ZIP、pet.json、资源链接、Petdex 等页面和 npm 写法，只下载解析素材，持久化去重。
- Codex 会话来源过滤（10-19，v0.1.51）：仅计入 `cli`、`vscode` 创建的会话。见 [codex-session-sources.md](codex-session-sources.md)。
- Linux 原生桌宠与 StatusNotifier 托盘（10-21，v0.1.53）。
- 子会话归并（10-22，v0.1.55）：有明确关系的子会话汇总到主会话。
- 会话提示音（10-24，v0.1.57；10-25 增加六款音色与应用音量，v0.1.60）。见 [session-sounds.md](session-sounds.md)。
- 非交互会话过滤（10-28 至 10-29，v0.1.61、v0.1.65）：排除 Claude SDK、MCP、GitHub Action 与 `claude -p` 等无头会话，不占用近期会话读取名额。见 [headless-rules.md](headless-rules.md)。
- 匀速动画调试开关（10-29，v0.1.64）。

### 服务器监控

- 紧凑资源摘要（10-09，v0.1.21）：CPU、内存及每张 GPU 的紧凑显示，连续运行时间与所选网卡流量。见 [five-hour-and-connection-improvements.md](five-hour-and-connection-improvements.md)。
- 并行采样与本机监测（10-10，v0.1.27）：服务器逐台返回结果，本机 CPU、内存、Swap、磁盘、网络监测，分别配置各类刷新间隔。见 [refresh-monitoring-antigravity.md](refresh-monitoring-antigravity.md)。
- 资源监控与 Agent 采集分离（10-14，v0.1.37）：本机与每台 SSH 主机独立开关。
- 主机筛选与平衡网格（10-19 至 10-22，v0.1.51、v0.1.54）。

### 界面与交互

- 设置整合（10-04 起多轮）：10-13 调整为 Agent 接入、服务器、价格、定时唤醒、常规五类；10-25 改为八个主导航与六类偏好（v0.1.60）。见 [ui-control-spec.md](ui-control-spec.md)。
- 图表配色（10-11，v0.1.30）：五套配色，支持明暗主题与高低对比度。见 [chart-palettes.md](chart-palettes.md)。
- 历史日期范围（10-13，v0.1.36）：新增“所有”与自定义日期，跨年热力图按需加载。见 [history-date-ranges.md](history-date-ranges.md)。
- 工作区改版（10-15，v0.1.42）：统一侧栏分组，Agent 面板按会话状态、今日用量、账户限额排列，用量分析独立成页，各页筛选隔离。
- 全应用文案统一（10-16，v0.1.44）：错误提示提供可复制的诊断详情并遮蔽常见凭据。
- 中英文切换（10-22，v0.1.54）：三端共用离线词库，切换保留草稿。
- 官网（10-22 起，v0.1.54–v0.1.55）：中英文官网、直接下载入口。见 [website.md](website.md)。
- macOS 原生侧栏与窗口材质选择（10-28，v0.1.61）：Windows 可分别选择主窗口与面板材质。见 [windows-appearance.md](windows-appearance.md)。
- 提供商图标更新（10-19 起，v0.1.46、v0.1.50）。见 [provider-icons.md](provider-icons.md)。

### 跨平台、发布与更新

- 更新传输（10-08，v0.1.20）：更新清单与安装包遵循应用代理，限时、有限重试和取消，保留 Sparkle 签名校验。
- 更新前协调（10-10 起）：等待进行中的数据与账户任务，保护未保存编辑。见 [releases.md](releases.md)。
- Windows/Linux 内容样式对齐 macOS（10-14，v0.1.38）与 Windows 控件、动效改进（10-15，v0.1.41）。见 [windows-linux.md](windows-linux.md)。
- 定时唤醒改进（10-13 至 10-14，v0.1.35、v0.1.38）：隔离临时会话执行、重新部署入口、按账户与机器保存前置命令。见 [credits-and-wakeups.md](credits-and-wakeups.md)。
- Linux 安装包（10-21，v0.1.53）：新增 Fedora RPM，移除 AppIndicator 运行依赖，Wayland 下优先 XWayland；v0.1.61 起主窗口保留原生 Wayland，桌宠与面板使用独立 X11 辅助进程。
- CI 与发布流水线（10-17 起）：新增 Intel macOS 日常 CI；发布标签只触发 Release，仅版本与发布说明变化时复用 main 上已通过的 CI。见 [development.md](development.md)。

## 修复与优化

### 额度与查询

- 重置时间展示（10-09）：改为遵循接口返回的 `resetsAt`，不再为满额或 0% 窗口推算新周期；v0.1.21 的推算规则已撤回。
- Codex CLI 兼容（10-08，v0.1.20）：旧版 CLI 不支持 `--stdio`、旧限额接口空参数协议；失败提示区分位置、阶段与错误类型。
- 前置命令继承（10-12，v0.1.32）：SSH 与本机模型查询使用所选账户的前置命令和 Shell，隔离前置命令输出。
- 额度查询调度（10-13，v0.1.36）：间隔统一 5–86400 秒，同一账户互斥，最多并发四个账户，合并重复请求。
- 账户切换后复用旧额度基线的问题（10-14，v0.1.38）。
- 系统时钟回拨（10-29，v0.1.64）：引入每账户 `observationRevision`，经校验的实时查询可替换“未来”缓存并清除旧占锁错误；占锁提示只用于真实锁竞争。
- 原生凭据审计（10-31）：修正 ZCode 授权头、DeepSeek Harness 钱包结构校验、Grok 多登录排序、Antigravity 不再执行 `/usage`，身份无法确认时拒绝更新并保留快照。
- Qoder 余额、额度分桶与公开账户 ID 展示（10-31 至 11-01，v0.1.69–v0.1.70）。

### 历史导入与计数

- 远程历史增量传输与续传（10-10，v0.1.27）；并行导入事务与锁竞争（10-11，v0.1.30）；SSH 全量重扫超时（10-12，v0.1.31）。
- 历史去重与死代码清理（10-10 代码审计）：供应商表集中、旧设置键与失效迁移移除、遗留别名层整理。
- Antigravity 截断模型名补全，无法确认模型时保留 Token 并标为待计价（v0.1.31）。
- fork 对话父会话累计 Token 重复计入（10-13，v0.1.35），以及分支历史按会话元数据识别父会话（v0.1.43）。
- Qoder 状态密钥识别、配置读取失败可能覆盖原文件、暂停后旧扫描写入（10-21 审查，v0.1.53）。
- 历史文件原地替换、损坏行、超大记录与压缩损坏保留诊断和重试位置（10-22，v0.1.55）。
- 估值归属：Antigravity 空闲模型组重置时间移动导致有效消耗被拒（v0.1.51）；Gemini 4 Argon 与 Gemini Models 混算（v0.1.70）。

### 会话状态与桌宠

- 旧会话因数据库维护重新计入活动、AGY “待定”数量累积（10-19，v0.1.52）。
- 桌宠动画停顿、缩放后文字清晰度、Windows 透明边缘（10-14，v0.1.37）。
- 桌宠只在素材、动作行、帧数或帧时长变化时重播，修正 macOS 离散关键帧数量（10-29，v0.1.64）。
- Windows 桌宠与面板置顶保持、macOS 弹出层层级（10-30 至 10-31，v0.1.66）。
- Linux AppImage 托盘、Esc 层级与键盘焦点（10-22，v0.1.55）。

### 界面

- 多轮 UI 审计与实施（10-04 至 10-07、10-14、10-15、10-25、10-26、10-28）：表单草稿保护、统一“注意”组件、状态枚举归一、长文本截断、平台惯例（单实例、Snap Layouts、托盘面板）等。见 [ui-control-spec.md](ui-control-spec.md)。
- 胶囊面板点击外部不收起、展开抢焦点（10-08，v0.1.19）。
- 账户卡片布局、单账户分组并排、Antigravity 折叠卡片显示全部模型组（10-19，v0.1.52）。
- 面板首次布局与缩放后尺寸、折叠卡片统一高度（10-24，v0.1.57）。
- 每日与年度表格对齐、热力图月份标签、窄窗口账户选择（10-30，v0.1.66）。
- 模型环形图“其他”使用中性色，不与模型撞色（v0.1.65）。
- 已解决模型保存映射后立即移出待计价列表（v0.1.70）。

### 性能

- 10-10 性能审计与实施：历史统计每日聚合缓存、独立查询通道，服务器逐台返回，各任务按到期时间调度。
- 10-08：价格页按需加载与独立读取，面板底色与动态内容重绘隔离（v0.1.20）。
- 10-17（v0.1.47）：菜单栏与桌宠改用原生动画机制，常驻摘要使用轻量采样投影，会话目录文件监听，设备状态批量检查。隔离合成场景下 Intel macOS UI CPU P95 为 3.28%。
- 10-19（v0.1.49–v0.1.50）：仪表盘按修订复用快照，主机监控常驻调度，SQLite 增量游标，HTTP 客户端复用。
- 10-21 至 10-25（v0.1.58）：macOS 滚动位置持久化、刷新影响范围收窄、页面保留与按需布局；本机回放中主要切页和刷新场景 UI 进程 CPU 时间下降约 90%–98%。

## 尚存问题

以下仅列文档明确标为未验证、未实现或未达标且后续文档未推翻的项目。

### 真实账户与服务商

- 多数新接入 Agent 的真实付费额度未实测（10-31）：Antigravity、DeepSeek 官方余额、ZCode 登录 JWT 与套餐、Qoder 新 qoder.cn 体系与 QoderWork 登录、Grok 订阅与后付费、OpenCode 付费池、Kimi Work、CodeBuddy IDE 与企业余额、Pi／Oh My Pi 上游订阅、WorkBuddy、Copilot、Cline、Kilo、Kiro、Trae／Trae CN 等。fixture 与模拟响应通过不等于真实额度已验证。
- Copilot 没有 Aieyes OAuth client id，不提供授权按钮（10-31）。
- Cursor 私有数据库中的请求级 Token 未接入，账户操作不在验收范围（10-31）。
- CodeBuddy 暂无已核实的公开剩余额度接口（10-19，v0.1.49）。
- Antigravity 自定义 `--user-data-dir` 与同时选择不同 CLI／IDE 账户未接入；Linux／Windows 的 IDE 实际 GUI 登录未验收（10-31）。
- DeepSeek Harness 凭据不含可离线确认的稳定用户 ID，平台账户余额未完成真实查询（10-31）。
- Cline：3.0.74 JSON 模式无已验证的续聊方法，ACP 场景需原生登录；`CLINE_DB_DATA_DIR`／`CLINE_SESSION_DATA_DIR` 覆盖目录未加入自动发现；仅含 fork seed 的孤立历史未按 `metadata.fork` 剔除（10-31）。
- Kilo：取消后原生未保存的用量无法对账；任意 channel 数据库未自动枚举，需显式指定 `KILO_DB`（10-31）。
- WorkBuddy 缺少每请求的时间与模型，无法还原安装前的精确月份或模型（10-31）。
- Roo、Kiro、Trae CN 本轮未进行客户端实际调用（10-31）。
- CodeBuddy 经典 Hooks 无“权限已获准”事件，等待标记可能保持到工具返回；Oh My Pi 首次模型响应前不写日志，无法提前显示工作中（10-22）。

### 估值

- Codex 多账户估值无法完整重建未被观测、且未留下请求侧额度差异的外部快速切换，估值不等同于账户账单（10-24）。
- 暂停设备退出实时覆盖的前提是暂停期间不在这些设备上使用同一账户（10-25）。

### 性能

- macOS 部分交互反馈仍超过 100 ms：10-21 复测中进入分析页与刷新切页首反馈中位数约 600 ms，滚动中刷新 33 ms 目标未达到；v0.1.58 说明部分反馈仍超过 100 ms，纯面板滚动 CPU 开销有所增加。
- 升级后首次历史重读可能占用数秒（10-21 实测约 8.85 秒），首次扫描的细粒度进度与取消尚未实现。
- 强制结束进程可能丢失最后约 250 ms 的滚动位置（10-21）。
- 性能数字来自隔离合成数据与模拟 SSH，长期运行内存趋势、真实网络与 Apple Silicon、Windows/Linux 原生进程未完整验证（10-10 至 10-19）。

### 平台实机验收

- Windows：Windows 10／11 的材质模糊与不透明回退、125%／150% 与混合 DPI 跨屏拖动、任务栏主题图标、安装版通知仍待实机确认（10-28）；WebView2 实机外观待验收（10-29，v0.1.65）。
- Windows 11 窗口圆角由网页绘制，未调用 DWM 圆角偏好（10-04 起已知限制）。
- macOS：多显示器与真实全屏 Space、点穿与拖动边缘、菜单栏对比度、完整 VoiceOver 交互待人工验收（10-28）；真实触控板手感与 WindowServer 合成未验收（10-24）。
- macOS 面板间歇性竖线问题仍需持续实机观察（10-08，v0.1.20）。
- Linux：GNOME／KDE 的 Wayland／X11 实际桌面（portal 主题切换、托盘坐标、焦点与点穿、分数缩放、外部浏览器启动）待验收；GTK 实机外观待验收（10-28 至 10-29）。显式原生 Wayland 下的定位与置顶受合成器限制。
- 各平台系统通知横幅的实际送达未完成实机验收（10-15，v0.1.40）。
- 对话框透字与默认深色面板灰度保留当前取舍，未作为已解决项（10-28）。
