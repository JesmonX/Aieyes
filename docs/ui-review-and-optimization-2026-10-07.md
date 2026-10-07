# 桌面端与 mac 端 UI 复核与优化方案 · 2026-10-07

复核基线 `6882e30`（main，v0.1.14 之后）。覆盖四个表面：macOS 菜单栏入口与其面板、桌面端悬浮面板、Windows 原生悬浮胶囊、Linux 悬浮球窗口。

本轮结论：**架构和令牌体系已经成型，两端信息结构基本对齐；主要问题是最近三次提交回退了若干已经确认过的面板设计，并留下不可达入口、失效样式和四套并存的阶段色。** 建议顺序是"状态一致性 → 常驻入口统一 → 面板密度 → 令牌与清理"，而不是继续加视觉细节。

## 0. 复核范围、证据与边界

**本次阅读的主要实现**（其他为佐证）：

| 平台 | 文件 | 行数 |
| --- | --- | ---: |
| macOS | `Views.swift`、`AieyesApp.swift`、`SettingsView.swift`、`QuotaTools.swift`、`SessionActivity.swift`、`AuditComponents.swift`、`FloatingTokenCard.swift`、`MultiSelectPicker.swift`、`WakeupsView.swift`、`Models.swift`、`Palette.swift`、`Typography.swift`、`MonitorSelection.swift` | 3 587 |
| 桌面 Web | `app.js`、`style.css`、`tokens.css`、`panel.css`、`floating.css`、`floating.js`、`desktop.js`、`sessions.js`、`analysis.js`、`tooltip.js`、`multiselect.js`、`ui-state.js`、`network.js`、`updates.js`、`refresh.js`、`index.html`、`floating.html` | 2 959 |
| Tauri 外壳 | `desktop.rs`、`capsule.rs`、`passive_window.rs`、`main.rs`、三份 `tauri*.conf.json` | 3 247 |

**截图证据的时间边界（重要）**：`.local/` 下的界面截图生成于 10-06 19:37–23:39，早于 `ae4c8f8`（10-06 22:06）、`ccbce1b`（10-07 00:10）、`6882e30`（10-07 11:08）。复核中已发现截图与当前代码不一致（例如 mac 面板截图里有"缓存 Token / 读取命中率"整行，而当前源码中不存在该文案；mac 面板截图里限额在"今日概览"之前，当前源码相反）。**因此下文所有结论以 HEAD 源码为准，截图仅用于判断整体观感。** 若需要以截图作验收依据，必须用当前提交重新生成 `.local/ui-audit-implementation/` 与 `.local/ui-previews/`。

**本轮未执行**：Windows / Linux 实机、辅助技术全流程、真实账户与 SSH。所有"行为"结论都来自源码与既有测试脚本的引用关系，不是运行观测。

## 1. 当前 UI 架构

### 1.1 表面与平台映射

| 表面 | macOS | Windows | Linux |
| --- | --- | --- | --- |
| 常驻入口 | 菜单栏 `NSStatusItem(variableLength)` + `NSHostingView<MenuActivityLabel>` | 原生 Win32/GDI+ 胶囊 144×44，独立线程与独立消息泵 | 同一个 WebView 窗口（160×60）内 CSS 药丸 144×44 |
| 悬浮面板 | `NSPopover` 450×`min(720, 屏高-34)` | 同进程 WebView 窗口，450×720（收起时不销毁，仅隐藏） | 同一 WebView 窗口由 160×60 放大到 450×720 |
| 详情 | `NSWindow` 1080×800 + 原生 `NSToolbar` | 主窗 1120×800，Windows 下无边框透明 + Mica | 主窗 1120×800，保留原生窗口装饰 |
| 设置 | 独立 `NSWindow` 620×440…760×600 | 主窗内 settings 页面 | 同 Windows |
| 长流程 | 采样窗口 530×400；估值窗口 530×600，按「账户+口径」各一个 | 主窗内 `dialog`（采样/估值为 `#quota-dialog`） | 同 Windows |
| 内容实现 | SwiftUI + AppKit 混排，`RootView(compact:)` 一套视图两种密度 | 单页 `index.html` + `floating.html`，`PANEL` 常量分支 | 同 Windows |

面板宽度两端都是 450，尺寸口径一致；高度按可用工作区收敛，但收敛余量不同（macOS 34pt，桌面 24px）。面板内容优先级、筛选作用域、账户顺序、5h/7d/credits 数值口径在两端一致。

### 1.2 macOS 结构

- `AieyesApp.swift` 是唯一的窗口宿主：`AppDelegate` 持有 `statusItem`、`popover`、`detailWindow`、`settingsWindow`、`samplingWindow`、`estimateWindows`，并实现 `NSPopoverDelegate` / `NSWindowDelegate` / `NSToolbarDelegate`。
- 视图层按职责切分为 `Views.swift`（面板与详情）、`SettingsView.swift`（5 个标签 + 5 个编辑器 sheet）、`QuotaTools.swift`（采样管理、账户排序、估值窗口）、`WakeupsView.swift`（定时唤醒）、`SessionActivity.swift`（会话状态与菜单栏标签）。
- 令牌层是 `Palette.swift`（可适应浅深色的强调/正常/注意/错误四色 + 卡片圆角 16/浮层 20/控件 9）与 `Typography.swift`（正文 15 / 辅助 14 / 节标题 16 / 页标题 22）。
- 卡片统一由 `NeutralCard` 修饰器实现，在减少透明度或增大对比度时改用不透明系统色并加粗边框（`Views.swift:26-37`），这是两端无障碍分支里最完整的一处。
- `--render` 模式可在离屏窗口批量截图，是当前唯一的原生视觉回归手段。

### 1.3 Windows / Linux 结构

- 主窗由 `tauri.conf.json` 定义 1120×800（最小 840×600）；Windows 覆盖为无边框 + 透明 + Mica + 阴影，最小 640×440。主窗内容自绘 40px 标题栏（`#titlebar`），由 `desktop.js` 仅在 Windows 下显示。
- `floating` 窗口承载面板；Windows 上它是 450×720 的 WebView（收起时隐藏），常驻入口另由 `capsule.rs` 的原生胶囊承担，因此**胶囊不经过 WebView**，面板渲染线程卡死时其消息泵与右键菜单仍可用。Linux 上同一个窗口既是球也是面板。
- `passive_window.rs` 通过子类化给 Windows 面板窗口加 `WS_EX_NOACTIVATE` 并拦截 `WM_MOUSEACTIVATE`，用 `SW_SHOWNOACTIVATE` 显示，因此**Windows 面板不会抢走前台应用焦点**。
- 托盘使用 Tauri 动态菜单，图标复用 `icons/brand.rgba` 并在右上角叠加相位状态圆点（`desktop.rs` 的 `icon()`），不重新着色品牌图。

### 1.4 共享层与状态广播

两端都通过 JSON-RPC 调用 `aieyes-core`。桌面端额外做了一层事件广播：`desktop:status`（外壳状态）、`desktop:panel`（面板开合与固定）、`desktop:refresh-status`（按动作的进行中/成功时间/逐项失败）、`desktop:data-changed`（数据失效）、`desktop:settings`（配置失效）、`desktop:hosts` / `desktop:hosts-error`（采样结果）、`desktop:network`（连接测试）。`main.rs` 的 `update_events` / `refresh_feedback` 已把"成功时间"与"数据失效"分开，且不广播设置内容本身（有对应单元测试）。这部分设计是本项目里最扎实的一层，本轮没有发现回归。

### 1.5 令牌对照

| 项 | macOS | 桌面端 | 是否一致 |
| --- | --- | --- | --- |
| 强调色 | `#5566d9` / `#a3b3ff` | `--accent` 同值 | 一致 |
| 正常/注意/错误 | `#176e58`/`#895712`/`#b83245`（深色对应） | `--ok`/`--warn`/`--danger` 同值 | 一致 |
| 卡片圆角 / 浮层 / 控件 | 16 / 20 / 9 | `--radius-card`/`--radius-overlay`/`--radius-control` 同值 | 一致 |
| 字号 | 15 / 14 / 16 / 22 | `--font-body` 等四项同值 | 一致 |
| 阶段色 | 系统语义色 `.teal/.purple/.blue/...` | 4 处硬编码 RGB | **不一致**（见 P1-4） |
| 资源告警色 | `Palette.danger/warn` 与系统 `.red/.orange` 混用 | `--resource-warn`/`--resource-high` | **不一致**（见 P1-6） |
| 卡片底 | `.regularMaterial` | `--panel` 半透明纯色 | 平台合理差异 |
| 提示气泡 | 系统 `toolTip` | 自绘 `.hint` | **不一致**（见 P2-11） |

## 2. 已验证的问题

### 2.1 P1 · 状态与一致性

**P1-1 macOS 面板**
如果面板在打开状态 点击菜单栏图标会导致面板也被唤起 且此时点击外部不会是使得单栏的悬浮窗口关闭。 同时 如果在悬浮窗口内点击排序后 即使关闭排序窗口 也会使得悬浮窗口无法在点击外部时关闭。 类似这些可能还会有别的触发条件 清仔细检查


**P1-2 「置顶账户」与「查看全部账户列表」在两端都不可达，但 README 仍在描述它。**
- macOS：`showingAccounts` 只有被置为 `false` 的地方（`Views.swift:65/103/119/128`），没有一处设为 `true`，因此 `allAccounts` sheet 无法打开；`@AppStorage("quota.pinned")`（`Views.swift:67`）只在该不可达 sheet 里写入（`:119`），从未被读取，置顶不会影响任何排序或摘要。
- 桌面：`#all-quotas` 在 `app.js` 中只有事件绑定（`:1003`）而没有生成标记，`openAllQuotas()`（`:1015-1025`）只能从该分支进入，`localStorage['quota.pinned']` 也只在该函数里写入（`:1022`）。
- `README.md:54` 仍写着"面板默认显示当前或置顶账户的限额摘要，通过「查看全部」访问完整账户列表"。
后果不只是文档过期：**面板因此没有任何账户收敛机制**（见 P1-7），而 10-06 审计的 C01/C07 正是围绕这一点立项的。
方向：二选一。要么补齐入口（面板首屏只渲染置顶或首个账户的 `PanelQuotaSummary` + 一行「查看全部 N 个账户」），要么明确废弃并把 README、`panel.css`、`PanelQuotaSummary` 一起删除。不要保留"没有入口的入口函数"。
- 用户批注建议：废弃


**P1-4 阶段色有 4 套硬编码，加 macOS 的系统色共 5 处。**

| 来源 | working | thinking | tool | complete | interrupted |
| --- | --- | --- | --- | --- | --- |
| `capsule.rs` `paint()` | `#2c9d88` | `#ac78dd` | `#629cef` | `#44ad76` | `#d9a44c` |
| `desktop.rs` `icon()` | `#28a496` | `#a168dd` | `#4682e6` | `#31a565` | `#d99434` |
| `style.css` `.phase-mark` | `#17957f` | `#8f57c6` | `#3f7fd6` | `#219355` | `#b1761f` |
| `floating.css` `body[data-phase]` | `#2c9d88` | `#ac78dd` | `#629cef` | `#44ad76` | `#d9a44c` |
| macOS `SessionPhase.color` | `.teal` | `.purple` | `.blue` | `.green` | `.orange` |

同一个"进行中"在胶囊、托盘图标、会话行里是三种不同的绿色；`.phase-mark` 那一列偏暗，因为它是为浅色文字前景定的，而胶囊/托盘是图形填充色。macOS 用系统语义色，能跟随深色模式与增大对比度，桌面端做不到。
方向：在 `tokens.css` 定义一组 `--phase-*` 变量并让 `app.js` 通过 `getComputedStyle` 读取（它已经在读 `--border`/`--muted`/`--accent`/`--resource-*`）；`capsule.rs` 与 `desktop.rs icon()` 保留各自的 Rust 常量，但值必须从同一张表派生，并在 `scripts/check-native-capsule.sh` 里加一致性断言。

**P1-5 Linux 悬浮球的几何比 Windows 内缩 8px。**
Windows 胶囊是 144×44 的原生窗口，圆角路径覆盖 `0.5…143.5`，视觉与窗口边界一致（`capsule.rs`）。Linux 是 160×60 的 Tauri 窗口（`desktop.rs:24-25` 的 `BALL_SIZE/BALL_HEIGHT`），内部由 `floating.css:4` 的 `place-items:center` 把 144×44 的药丸居中，四周各留 8px 透明边。
而吸附、默认位置与面板锚点全部按窗口盒计算：`snap_rect` 把窗口贴到工作区边缘后，**药丸离屏幕边仍有 8px**；默认位置 `x = 工作区右 - 160 - 24` 让药丸离边 32px；`panel_rect` 的间隙也按窗口边算，面板与药丸的实际视觉间距比 Windows 多 8px。两端"同一个入口"的贴边行为与面板间距不一致。
方向：让 Linux 窗口尺寸等于药丸尺寸（144×44），或把 `BALL_SIZE/BALL_HEIGHT` 改回 144×44 并把 `place-items` 改为 `normal` + 显式尺寸；随后 `scripts/test-desktop.mjs` 里的矩形吸附用例需要同步。

**P1-6 同一个数值，圆环和进度条给出不同颜色。**
`Views.swift` 内同一个文件里：`ResourceRing` 用 `value >= 90 ? Palette.danger : value >= 70 ? Palette.warn : Palette.accent`（`:855`），`ResourceBar` 用 `percent >= 90 ? .red : percent >= 70 ? .orange : Palette.accent`（`:872`）。系统 `.red/.orange` 比本项目的中性 danger/warn 更饱和，服务器卡片里会出现两种红。
方向：`ResourceBar` 的默认分支改用 `Palette.danger/warn`，与圆环、限额条、`--resource-high/warn` 的阈值语义对齐（两端都是 90/70 与剩余 10/30 两套阈值，需要一并核对）。

**P1-7 面板的账户列表没有上限。**
桌面 `quotaSection` 渲染 `state.dashboard.quotas` 的全部（`app.js`），macOS 紧凑面板同样 `ForEach(model.dashboard.quotas)`（`Views.swift:349`）。因为 P1-2 的收敛入口不存在，20 个账户就是 20 张卡（每张默认展开、可单独折叠）。审计的容量验收里明确包含"20 账户"场景。
方向：（用户批注 建议每个agent最多展示5个账户 且用户可以决定每个agent展示的账户（0～5））

### 2.2 P2 · 视觉与密度

**P2-1 菜单栏项同时出现两个 22px 圆形图标。** `MenuActivityLabel` 是"品牌图 22 + 相位圆 22 + 计数 + 可选指标 + 采样图标（+计数）"（`SessionActivity.swift:105-121`）。默认 `menuMetric = "icon"` 时 `menuText` 为空，于是菜单栏里最显眼的就是并排两个圆形图标，其中相位圆是 22pt 底衬 + 14pt 字形——比系统状态项常规尺寸大。建议：默认形态压缩为"品牌图 + 相位点（6–8px，无圆形底衬）"，把计数与指标作为可选；相位圆的底衬只在会话活跃时出现。

**P2-2 面板 footer 在 450 宽下过密。** macOS footer 是一行"状态文字 + 出站 + 更新 + 详情 + 设置 + 更多"（`Views.swift:146-166`），状态文字在"记录同步于 下午9:48 · 记录已同步"这种长度下会被压缩，全文只在 `.help` 里；桌面用两行列布局（`panel.css:63-65` 的 `.panel-connection`）缓解了同样的问题。建议两端统一为"状态一行（可截断 + tooltip），动作一排"的两行结构，并把"出站"从常驻按钮降为状态文字的一部分。

**P2-3 同一个面板里两个菜单外观不同。** `#panel-menu` 用 `--surface-strong` + `backdrop-filter: blur(20px)`（`panel.css:21`），`#panel-more-menu` 用不透明的 `--solid`（`panel.css:73`），两者圆角也不同（`--r-md` vs `--r-md` 相同，但最小宽度 132 vs 230、内边距 6 vs 12）。建议两者共用同一浮层配方（`--solid` + `--shadow-float`，或都用半透明），并统一最小宽度与行高。

**P2-4 设置页标题重复三层。** macOS 是窗口标题"Aieyes 设置" + 内容区 22pt "设置" + 状态行 + 两个按钮（`SettingsView.swift:24-35`）；桌面是主窗 `h1` "设置" + 标签栏。建议 macOS 让原生标题栏承担标题（窗口标题已足够），内容区只保留状态行与保存/放弃，节省约 60–70pt 垂直空间——这在最小 620×440 下正好是一个表单行的余量。

**P2-5 面板里同一个趋势入口，两端文案与嵌套结构不同。** 桌面：`<details>` 标题"展开趋势"，内部再嵌一层"每日明细" details（`app.js:363`）。macOS：一个 `DisclosureGroup("近 N 天趋势与每日明细")` 同时包含图表与 `DailyUsage`（`Views.swift:217`）。建议统一为"一个折叠项、标题含范围、内部不再嵌第二层折叠"，避免 450px 宽度下三级缩进。

**P2-6 「用量详情」形态不同。** 桌面是整宽按钮 `.panel-wide`（`app.js:364`、`panel.css:47`），macOS 是无边框强调色文字按钮（`Views.swift:218`）。建议统一为"贴底整宽次按钮"或"强调色文字入口"，两端取其一。

**P2-7 胶囊缺少悬停与按下反馈，计数没有徽标语言。** `capsule.rs` 的 `paint()` 没有 hover/pressed 分支，Windows 只有原生 tooltip；Linux 球有 `#ball:hover` 变亮（`floating.css:6`）。胶囊右侧计数与标签同色同字号（`GdipDrawString` 的 count 矩形 `X=111 W=29`），而托盘图标用右上角圆点徽标表达状态——同一个应用里两种"计数/状态"语言。建议：胶囊加 hover 底衬变化与按下轻微内缩（仍保持"始终无动画"的既有决定），计数改为独立的 pill 底色。

**P2-8 主窗最小尺寸两端不同。** 基础配置 840×600，Windows 覆盖为 640×440（`tauri.conf.json` / `tauri.windows.conf.json`），而 `style.css` 的响应式断点在 700/850/1100px。也就是说 640–840 这一档只在 Windows 可达，Linux 永远看不到那段布局。建议要么把 Linux 也放到 640×440 以便共用一套响应式验证，要么在文档里明确"窄档仅 Windows"。

**P2-9 编辑器的宽度与字段顺序两端不同。** 桌面所有编辑器复用同一个 660px `dialog`；macOS 按对象区分（来源 570、主机 620、账户 560、价格 520）。字段顺序上，桌面把"监控指标与设备"折叠区移到"连接并读取设备"按钮之前（`app.js:757-760`），macOS 保持按钮在上、折叠区在下（`SettingsView.swift:505-522`）。建议统一为 macOS 的顺序（先动作、后参数），并让桌面按对象区分宽度，至少把价格编辑器收窄。

**P2-10 桌面端的数字与状态色写法分散。** `style.css`/`panel.css` 里同时存在 `--font-*` 令牌与大量硬编码 `13px/12.5px/11px/24px/30px/26px`；`.stats .cache-card { grid-column: 1/-1 }`（`style.css:498/535`）在面板里永不生效，因为面板渲染的是 `tokenSummary` 而不是 `cacheCard`。建议把面板相关字号提升为令牌（`--font-panel-*`），并删除面板不使用的统计网格断言，避免"看起来在处理某场景、实际不生效"的规则堆积。

### 2.3 P3 · 死代码与失效样式

已确认无任何引用的 CSS 类（CSS 中存在、`index.html`/`floating.html`/所有 `*.js` 中都不出现）：

`agy-details`、`credit-value`、`day-head`、`heat-foot`、`heat-key`、`inline`、`panel-cache`、`panel-quota-summary`、`panel-summary`、`panel-token-breakdown`、`pricing-notice`、`quota-all`、`quota-credit-time`、`quota-credits`、`token-card`、`unavailable-credits`。

其中 `panel.css:104-113` 的整块 `.panel-summary` 规则、`panel.css:122-123` 的 `.panel-cache`、`panel.css:115` 的 `.quota-all` 正是"面板只显示摘要"那套设计，说明该设计在 CSS 里还留着、在代码里已经撤销。同时这些类被当作状态选择器使用，规则因此静默失效：`style.css:508-510`（`.quota-credit-time`）、`:521-522`（`.unavailable-credits`）需要逐条核实后再删；`:528-529`（`.quota-error` 的展开/收起文案切换）仍然有效，不要一并删除。

其他确认项：

| 项 | 位置 | 说明 |
| --- | --- | --- |
| `PanelQuotaSummary` | `AuditComponents.swift:26` | 从未被引用； |
| `RootView.header` 的 `if !compact` 分支 | `Views.swift:132/135` | 非紧凑模式已不再渲染 header（`:74`），这两个分支恒不执行 |
| `desktop_panel_cursor_inside` | `desktop.rs` | 命令已注册，但 web 端无任何调用方，仅被 `scripts/test-*.cjs` 断言；生产路径上悬停判定未生效 |
| `--window-sheen` | `tokens.css:20/89`（强制高对比处另见 `:119`） | 浅深色都设为 `none`，仅为 `style.css:20` 消费，属历史兼容 |
| `--r-lg` / `--r-xl` | `tokens.css:46/47` | 两者都是 16px；另有旧别名 `--radius` |
| 面板内 `.stats .cache-card` | `style.css:498/535` | 面板不产出 `cache-card`，规则不生效 |
| `capsule.rs` 的 `WM_KEYDOWN` 分支 | `capsule.rs` | 窗口为 `WS_EX_NOACTIVATE` + 返回 `MA_NOACTIVATE`，点击不激活窗口，Enter/Space 处理能否到达需实机确认（见 2.4） |


## 3. 优化方案

### 3.1 常驻入口（mac 菜单栏项、Windows 胶囊、Linux 球）

三端应当共用一套状态语义，只在承载方式上分叉。

1. **状态枚举归一**：确立 `idle / working / thinking / tool / complete / interrupted / unknown` 七态，以及"状态不全""恢复中""恢复失败""待确认"四个外壳态；把这七态的色值写进 `tokens.css` 的 `--phase-*`，`capsule.rs`、`desktop.rs icon()`、macOS `SessionPhase` 都从同一张表取值（macOS 保留系统语义色作为映射目标即可）。
2. **信息层级归一**：三种入口都按"品牌 + 状态指示 + 计数 + 可选指标"排列，默认只显示前两项。macOS 去掉相位圆的底衬（或只在活跃时显示）；Windows 胶囊把计数改为 pill 徽标；Linux 球保持现状但补齐与 Windows 一致的 hover/按下反馈。
3. **几何归一**：Linux 窗口尺寸改回 144×44（或药丸改为填满 160×60），使贴边距离、默认位置与面板间距三端一致；同步更新 `scripts/test-desktop.mjs` 的矩形吸附断言。
4. **可达性与反馈**：胶囊补 hover 底衬与按下反馈，保持"无持续动画"的既有结论；确认键盘可达性后决定保留或删除 `WM_KEYDOWN` 分支；`F10+Shift` 与 `ContextMenu` 的菜单入口在两端文档中写明。
5. **恢复路径**：`refresh-floating` 目前是"销毁并重建 WebView"，用户可见为闪烁，并且有 10 秒超时与"恢复失败"状态。建议只在真正无响应时使用，并在菜单项文案上写明后果（"重建面板（用于界面无响应）"）。

### 3.2 悬浮面板

面板是四类表面里问题最集中的一处，建议一次改到位：

1. **首屏预算固定**：按 450 宽、540 高为基准，把首屏定义为"header 48 / 会话行 32 / 账户摘要 100–120 / 今日双指标 110–125 / 缓存或次级入口 48–60 / footer 44"。这正是审计 §4.3 给出的预算，目前没有实现（P1-3、P1-7）。
3. **优先级排序**：明确"今日双指标 → 账户限额"，并写回审计。
4. **底部收敛**：状态与动作分两行（P2-2）；出站测试降级为状态文字的 tooltip；「更多」菜单与刷新菜单统一浮层配方（P2-3）。
5. **趋势入口统一**：单一折叠项、标题含范围、内部不再嵌套（P2-5）；「用量详情」形态两端一致（P2-6）。
6. **承载方式统一**：macOS 恢复非激活承载（P1-1），Windows 保持 `passive_window.rs`，Linux 保持现方案但在文档中说明差异原因（Wayland 无法绝对定位）。
7. **状态恢复**：桌面已按 `localStorage` 键恢复页签、筛选、滚动与账户折叠，macOS 用 `@AppStorage` 分 `panel`/`detail` 两套键。建议在两端都补一条"面板重建后恢复展开状态"的回归（`desktop_panel_ready` 已有 generation 机制，macOS 侧需要等价钩子）。

### 3.3 详情窗口

1. **窗口与工具区**：macOS 详情已用原生 toolbar（页面切换 + 刷新下拉），保持；建议把"页面"分段控件的 `accessibilityLabel` 与快捷键补齐，并确认 760×480 下内容可见高度（审计 §3 M13 的验收条件）。
2. **菜单栏口径**：macOS 菜单栏使用独立的 `menuDashboard`（无筛选、今日），面板用 `panelDashboard`，详情用 `dashboard`。三者已经分开，建议在 UI 上把"菜单栏口径"写进设置项说明（"今日全部 Token"已具备），避免用户把菜单栏数字当作当前筛选结果。
3. **资源颜色**：修正 `ResourceBar` 的默认色（P1-6），使同一卡片里圆环与进度条一致。
4. **服务器页**：两端都以"摘要行 + 展开明细"表达，macOS 的 `ResourceRing` 72pt 在 450 宽面板内会挤到 3 列并换行。建议面板内圆环降到 56–64pt、指标文字允许两行。
5. **慢请求与过期响应**：桌面已有 `dashboardRequest` 序号淘汰与"请求/已应用筛选"分离，macOS 有 `dashboardRequest` 计数。两端已有回归，建议补充"筛选失败后恢复已应用筛选"的截图基线。

### 3.4 设置

1. **标题层级**：macOS 去掉内容区 22pt 标题，改用窗口标题（P2-4）；桌面保留 `h1`。
2. **保存语义**：两端已是"编辑器更新草稿 → 顶部保存全部"，价格条目即时保存，任务独立保存。建议把这条规则写成设置页顶部的一行常驻说明（macOS 已有状态行"应用配置 · 未保存 N 项"，桌面已有 `draft-state`），并统一"未保存项数"的口径。
3. **编辑器一致性**：统一字段顺序（动作在前、参数在后，P2-9）；价格编辑器按对象收窄；`Escape` 行为、初始焦点（取消）、`Tab` 循环在两端都对齐（桌面已实现自定义 Tab 循环，macOS 依赖系统行为）。
4. **危险操作**：两端已用影响列表 + 红色确认 + 取消占位焦点。建议补一条"归档账户"的可逆说明（桌面文案已写明"保留历史"，macOS `DangerConfirmation` 的 `confirmLabel` 用"归档并保留历史"）。
5. **长列表**：macOS 价格页的映射区（`frame(maxHeight: 100)`）与待计价区（`frame(height: 100)`）在 620×440 下已是内嵌滚动，实测在 25 条映射时可用；建议把同样的上限策略用于账户列表与主机列表。

### 3.5 令牌与组件库

1. **收敛令牌**：删除 `--window-sheen`、`--r-lg`/`--r-xl` 重复与旧别名 `--radius`；把面板字号提升为令牌（P2-10）。
2. **补齐令牌**：新增 `--phase-*`（七态）与 `--count-badge`（计数徽标），并让 `app.js` 从 DOM 读取（已有先例）。
3. **组件规格表**：审计 §4.2 建议整理 `StatusBadge`、`MetricCard`、`QuotaSummary`、`FilterBar`、`EditableRow`、`InlineError`、`EmptyState`、`SaveBar`、`TaskProgress` 九类组件。当前两端各自已有事实上的对应物（`NeutralCard`/`Surface`、`StatCard`、`QuotaCard`/`PanelQuotaSummary`、`attention`、`draft-state`、`saved-feedback`、`#background-task`），建议**只补一份"语义规格 + 相同样本数据"的文档**，不强行共享渲染代码。
4. **清理**：删除 2.3 表中确认无用的类与组件；`PanelQuotaSummary` 若被 3.2 采用则保留，否则删除。

### 3.6 动效与反馈

现有约定是合理的（仅 transform/opacity、`animation-fill-mode: backwards`、`prefers-reduced-motion` 全关、胶囊无持续动画）。建议补三点：

1. 面板开合动画两端目前不同（桌面 `panel-in` 0.18s 缩放；macOS 依赖 popover 系统动画）。统一为"系统动画优先、无自定义缩放"，避免三端观感差异。
2. 数字更新：macOS 已用 `.contentTransition(.numericText())` 与合作 `animation(value:)`；桌面用 CSS 过渡。建议桌面为 `#activity`、统计值也补齐过渡，并在 `prefers-reduced-motion` 下直接切换。
3. 忙碌指示：桌面 `#activity` 有脉冲点，macOS 用 `ProgressView(.mini)`。建议 macOS 面板也采用"小圆点 + 状态文字"而非 spinner，减少面板内跳动。

### 3.7 无障碍

1. **提示气泡**：两端机制不同（P2-11 记于 1.5 表）。建议 macOS 继续用系统 tooltip（符合平台习惯），但为面板内被截断的状态文字补 `accessibilityLabel` 全文——目前只有 `.help`，读屏拿到的是截断内容。
2. **可达名称**：两端已大量使用 `accessibilityLabel`（含"查看 X 的实时限额""移除来源 X"等），建议补做一次遍历核对：面板 footer 图标、胶囊计数、菜单栏项、`#activity`。
3. **状态不只靠颜色**：阶段色在会话行同时有图标与文字，符合要求；资源圆环只有颜色与百分比，建议在 90% 以上补"高负载"文字或形状，以应对色觉障碍与强制高对比模式。
4. **强制高对比**：桌面 `forced-colors` 分支把表面改为 `Canvas`、进度条改为 `Highlight`；macOS 用 `colorSchemeContrast == .increased` 走不透明卡 + 2px 边框。两套都已存在，建议补一条自动断言（截图像素采样），因为这类分支最容易被后续样式覆盖。
5. **焦点不被遮挡**：面板内固定 header/footer + 滚动 body 的结构满足底线；建议为"更多"菜单展开时的焦点返回补回归（桌面 `floating.js:21-27` 已实现 `closeMenu(restoreFocus)`）。

## 5. 证据索引

源码定位：

- macOS 面板与详情：`apps/macos/Sources/Aieyes/Views.swift`
- macOS 窗口宿主与面板开合：`apps/macos/Sources/Aieyes/AieyesApp.swift`
- macOS 菜单栏项与会话相位：`apps/macos/Sources/Aieyes/SessionActivity.swift`
- macOS 令牌：`apps/macos/Sources/Aieyes/Palette.swift`、`Typography.swift`
- 桌面面板渲染：`apps/desktop/web/app.js`（`renderAgentPanel`、`quotaSection`、`openAllQuotas`）
- 桌面面板样式：`apps/desktop/web/panel.css`、`floating.css`、`tokens.css`
- Windows 胶囊：`apps/desktop/src-tauri/src/capsule.rs`
- 外壳状态与窗口：`apps/desktop/src-tauri/src/desktop.rs`、`passive_window.rs`、`main.rs`
- 窗口配置：`apps/desktop/src-tauri/tauri.conf.json`、`tauri.windows.conf.json`、`tauri.linux.conf.json`

历史文档（本轮结论与之有冲突处以本文件为准，冲突项见 P1-3、2.3）：

- `docs/ui-audit-and-redesign-2026-10-06.md`、`docs/ui-audit-and-redesign-2026-10-06-implementation.md`
- `docs/ui-reform-2026-10-06.md`、`docs/ui-reform-2026-10-06-implementation.md`
- `docs/cross-platform-ui-review-2026-10-05.md`、`docs/native-capsule-and-ui-sync.md`、`docs/windows-ui-refresh.md`

截图（均早于本次基线，需重新生成后再作验收依据）：

- macOS：`.local/ui-audit-implementation/macos/`
- 桌面面板与主窗：`.local/ui-previews/`、`.local/ui-audit-implementation/`
- 改革矩阵：`.local/ui-reform-2026-10-06/`

本轮未修改任何产品代码，未读取真实日志、凭据或远程主机，未发布安装包。
