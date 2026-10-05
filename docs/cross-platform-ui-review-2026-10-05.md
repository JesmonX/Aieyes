# Windows / macOS 交互与 UI 评审 · 2026-10-05

**修复状态：本轮评审中的功能缺陷与 UI 优化已实施。以下原评审正文保留修复前的证据；当前行为和验证如下。**

- Windows：修复编辑器 Tab 循环、删除影响确认、刷新后的焦点／展开／滚动恢复、重复表单 ID、空态导航、托盘隐藏命令、标题栏拖动和更多菜单。面板与主窗同步配置和服务器结果，主窗负责自动采样；错误与数据年龄独立更新，失败不再冒充成功刷新。
- macOS：稳定设置窗口与草稿，统一“完成更新草稿、顶部保存全部”；关闭／退出支持保存、放弃与取消。API Key 随草稿提交，失败保留重试。价格独立即时保存并显示就地错误；启动／启用账户自动查询限额；多窗口可见性独立记录。
- 两端：核心新增 modelOptions，保留同级模型选择；macOS 显示可清除的有效筛选。首次连接入口置顶，提供本机日志／账户限额／SSH，空图表改简短提示；账户关系解释和 DeepSeek／agy 默认关联已补充。
- 布局：面板铺满 420px 宽度，Token 明细改紧凑三列，关键辅助文字增大；静态卡片不再 hover 上移或随每次刷新重播入场。退出移入更多菜单，浅深色的关键文本令牌及主按钮对比度通过 4.5:1 检查（不代表全应用辅助技术审计）。macOS 映射区独立滚动，设置／面板适配较低屏高；Windows 最小主窗 640×440，启动／恢复／DPI 改变时按工作区校正。
- 统计与状态：面板图表标题匹配数据范围，macOS 每日明细在成本模式显示 USD；RPC 整体失败、逐项失败、等待首次限额、旧采样、缓存读取分别呈现。主机采样成功后清除对应的旧错误。

验证：核心 Rust 38 项、Tauri Rust 12 项（macOS 宿主，含 custom-protocol）、桌面／多选／发布 Node 22 项通过；两套 Chromium UI 回归通过；Swift 原生构建及 `scripts/test-native.sh`（含新增草稿、凭据与额度 mock 回归）通过，核心与 Tauri 严格 Clippy／格式检查通过。新增 UI 回归已接入 `npm run test:ui`，macOS 回归已接入原生测试脚本。

修复后快照：[浅色面板](../.local/ui-previews/floating-panel-fixed-light.png)、[深色面板](../.local/ui-previews/floating-panel-fixed-dark.png)、[首次引导](../.local/ui-previews/onboarding-fixed.png)、[640×440 编辑器](../.local/ui-previews/editor-minimum-fixed.png)、[macOS 25 条映射小窗口](../.local/ux-review-2026-10-05/macos-fixed/settings-prices-small.png)。

仍需目标设备验收的范围：Windows 真实 WebView2／Mica／Snap／多屏 DPI，macOS VoiceOver、原生保存确认按钮与真实多屏行为。本轮未发布安装包，未修改真实账户或服务器数据。

原评审基于提交 `1de567c`。由主审与 3 个 subagent 分别检查 macOS、Windows 原生外壳、Web UI，再交叉核对；评审阶段未修改产品代码或真实账户、服务器配置。

最值得优先投入的是设置保存、键盘可达性、面板数据同步和数据可信度。现有字体、圆角、浅深色与资源圆环已有一致基础；继续增加装饰之前，应先确保用户能完成操作、知道保存结果、理解当前数据范围。

**验证范围**

- `node scripts/test-ui.cjs` 通过，检查了生成的主窗、来源编辑器、浅深色面板及小尺寸服务器截图。
- macOS `swift build --package-path apps/macos --scratch-path .local/review-swift-build --disable-sandbox -c debug` 通过。
- 使用独立模拟 JSON-RPC 核心运行原生 `--render`，生成菜单栏、详情、4 个设置标签和编辑器快照。模拟来源禁用，未读取真实会话、访问 SSH 或查询真实凭证。
- 额外 Chromium 探针复现了键盘卡住、删除关联配置、错误更新时间、重复标签 ID、面板折叠、配置陈旧、空态入口异常与宽度收缩。结果位于 [browser-probes.json](../.local/ux-review-2026-10-05/browser-probes.json)。
- Windows 是源码和 Chromium/mock IPC 检查，未验证 WebView2、DWM/Mica、Snap 或真实多屏 DPI；macOS 截图不等于菜单栏焦点、VoiceOver 和多窗口点击流程全部实测通过。

**优先修复的交互问题**

1. **[P1 · Windows · 已复现] 主机编辑器的 Tab 焦点会卡住。**

   打开编辑主机，保持“高级设置”收起，从名称连续按 Tab。焦点到“显示细分项”后无法继续到保存／取消。当前焦点循环未包含 `summary`，又用 `getClientRects()` 把收起 details 内不可见的输入算作可聚焦；调用其 `focus()` 失败。探针连续 Tab 30 次确认这一点。

   建议：纳入 summary，排除关闭 details 内部、inert 和不可见元素，修复正反向循环；保存后恢复到重新渲染后的对应“编辑”按钮。位置：[app.js:302](../apps/desktop/web/app.js#L302)、[app.js:274](../apps/desktop/web/app.js#L274)。

2. **[P1 · macOS · 源码确认] 再次进入设置会丢弃未保存草稿。**

   设置中改 SSH 主机或通用配置，切回详情后再次点设置。`openSettings()` 无条件替换已有窗口的 contentView，新 SettingsView 从已保存设置建立草稿，因此未保存内容直接消失。

   建议：已有窗口仅激活，保留编辑状态；显示未保存标记，关闭含改动的窗口时提供保存／放弃。位置：[AieyesApp.swift:91](../apps/macos/Sources/Aieyes/AieyesApp.swift#L91)、[SettingsView.swift:14](../apps/macos/Sources/Aieyes/SettingsView.swift#L14)。

3. **[P2 · macOS · 源码确认] 相似编辑器的提交范围与生效时机不同。**

   数据源／账户的保存会提交整个 settings 草稿，包含其它标签未保存的改动；主机的“完成”仅写草稿，必须再点顶部保存。测试采样、同步记录等操作也隐含保存整份草稿。这既可能漏存，也可能意外提交其它改动。

   建议：统一采用“编辑器更新草稿、顶部保存全部”，或统一采用单项立即保存；按钮文案、dirty 状态和失败反馈必须对应同一种语义。测试操作应能测试草稿，或明确标为“保存并测试”。位置：[SettingsView.swift:44](../apps/macos/Sources/Aieyes/SettingsView.swift#L44)、[SettingsView.swift:64](../apps/macos/Sources/Aieyes/SettingsView.swift#L64)、[SettingsView.swift:119](../apps/macos/Sources/Aieyes/SettingsView.swift#L119)。

4. **[P2 · Windows · 源码及探针确认] 主窗口和悬浮面板没有配置同步，固定监控面板也不持续更新。**

   面板只在启动调用一次 settings.get；重新打开、手动刷新只读取统计。模拟保存新增账户后再打开面板，持久配置已有新账户，下拉框仍只有“全部账户／无账户”，settings.get 次数仍为 1。主机改名、停用或删除也有同样问题。

   此外，PANEL 分支跳过所有采样轮询，主窗口采样又不广播结果。固定服务器面板只保留首次或手动采样的读数，“正常／数据延迟”也只在渲染时重新计算，因而可能将旧数据继续显示为正常。

   建议：统一采样与配置状态源，保存／采样后向面板推送；打开面板同步最新配置与数据，独立更新数据年龄。位置：[app.js:45](../apps/desktop/web/app.js#L45)、[app.js:465](../apps/desktop/web/app.js#L465)、[floating.js:57](../apps/desktop/web/floating.js#L57)、[app.js:163](../apps/desktop/web/app.js#L163)。

5. **[P2 · Windows · 已复现] 悬浮面板里的“添加服务器”走进不存在的编辑器。**

   无服务器时点击面板的添加按钮，当前窗口切到 settings，随后报 `Cannot set properties of null`。floating.html 没有编辑器 DOM，也没有转交主窗口。探针确认没有调用打开主窗口的 desktop_action。

   建议：以明确导航命令打开主窗口的服务器设置，并直接进入新增。位置：[app.js:178](../apps/desktop/web/app.js#L178)、[floating.html](../apps/desktop/web/floating.html)。[异常画面](../.local/ux-review-2026-10-05/panel-empty-add-host.png)。

6. **[P2 · Windows · 源码确认] 右键／托盘“暂时隐藏／恢复显示悬浮球”命令不匹配。**

   菜单 ID 是 `hide`，事件原样传给 action，而 action 仅接受 `hide-ball` / `show-ball`，落入“未知操作”。通用设置中的同功能按钮使用正确命令，因此两个入口行为不同。

   建议：统一菜单与设置页的动作映射，按当前状态切换隐藏／恢复。位置：[desktop.rs:179](../apps/desktop/src-tauri/src/desktop.rs#L179)、[desktop.rs:231](../apps/desktop/src-tauri/src/desktop.rs#L231)、[desktop.rs:483](../apps/desktop/src-tauri/src/desktop.rs#L483)。

7. **[P2 · macOS · 源码确认] 开启账户限额后仍不会自动进行首次查询。**

   bootstrap 和 save 不查询限额，tick 要求 lastQuota 不为 distantPast，即必须先手动查过。新 DeepSeek／agy 账户即使选择“显示并查询”也可能没有卡片，只能发现顶部刷新菜单后手动操作。Windows 主窗口已修复这一点。

   建议：启动和账户启用时首次查询；等待期间显示账户占位卡、读取中／失败重试状态。位置：[EngineClient.swift:116](../apps/macos/Sources/Aieyes/EngineClient.swift#L116)、[EngineClient.swift:189](../apps/macos/Sources/Aieyes/EngineClient.swift#L189)。

8. **[P2 · 两端 · 源码确认] 筛选结果与可见筛选控件不一致。**

   macOS 详情与菜单栏共用 source/model 筛选状态，但菜单栏隐藏这两个控件。详情筛某模型或来源后，菜单栏可见的“全部 Agent／全部账户”无法说明数据实际是子集，也无法直接清除此条件。

   两端的模型选项又都来自已按当前模型筛选过的 dayModels，选择 A 后 B 从列表消失，只能先选全部再选 B。

   建议：所有有效条件都有可见标签和清除入口；模型选项使用不受本维度筛选影响的集合。位置：[Views.swift:120](../apps/macos/Sources/Aieyes/Views.swift#L120)、[EngineClient.swift:126](../apps/macos/Sources/Aieyes/EngineClient.swift#L126)、[app.js:91](../apps/desktop/web/app.js#L91)、[store.rs:449](../crates/core/src/store.rs#L449)。

9. **[P2 · Windows · 已复现] 删除主机立即清空关联配置，没有影响说明或撤销。**

   点击减号即删除主机，关联来源同时变成 hostId=null、enabled=false；界面仅提示“已保存”。这里确认的是连接关系丢失，未据此断言历史用量被删除。

   建议：列出受影响的来源及停用后果，提供明确删除确认或可恢复操作；保留足以撤销的关联信息。位置：[app.js:252](../apps/desktop/web/app.js#L252)。

10. **[P2 · Windows · 部分复现] 刷新会破坏正在进行的阅读与键盘操作。**

    面板先替换 HTML 再读取展开状态，探针确认每日明细的展开项从 2 变 0。服务器页只保留展开状态，不恢复焦点；前台约每 2 秒重建内容，会打断键盘操作。保存编辑器后原触发按钮已被替换，实测焦点落到 BODY。

    建议：局部更新数值；必要的重建按稳定 ID 恢复展开、焦点和滚动位置。位置：[app.js:116](../apps/desktop/web/app.js#L116)、[app.js:131](../apps/desktop/web/app.js#L131)、[app.js:171](../apps/desktop/web/app.js#L171)、[app.js:469](../apps/desktop/web/app.js#L469)。

11. **[P2 · 两端 · 状态反馈] 成功、失败与进行中的反馈仍有遗漏。**

    Windows job 在 finally 中总写“更新于当前时间”；模拟同步失败后仍出现新时间，容易误读为数据已更新。macOS PriceEditor 缺少保存中禁用和弹窗内错误，失败仅写父窗口 message，用户可能重复点击。

    建议：显示最近成功时间，失败时保留旧时间并给出重试；价格编辑器采用现有来源编辑器的 async throwing／就地错误方式。位置：[app.js:25](../apps/desktop/web/app.js#L25)、[SettingsView.swift:68](../apps/macos/Sources/Aieyes/SettingsView.swift#L68)、[SettingsView.swift:426](../apps/macos/Sources/Aieyes/SettingsView.swift#L426)。

**已确认的布局问题**

- **[P2 · macOS] 模型映射多时，价格页无法容纳内容。** 固定 760×600 设置页内的映射 ForEach 不滚动。25 条模拟映射的原生截图中，价格列表被压缩消失，设置标题／标签与底部操作越界。建议给映射区独立限高滚动，或单独管理映射；整体可调整尺寸。位置：[SettingsView.swift:42](../apps/macos/Sources/Aieyes/SettingsView.swift#L42)、[SettingsView.swift:152](../apps/macos/Sources/Aieyes/SettingsView.swift#L152)。对比：[2 条映射](../.local/ux-review-2026-10-05/macos/settings-prices.png)、[25 条映射](../.local/ux-review-2026-10-05/macos-many-mappings/settings-prices.png)。
- **[P2 · Windows Web] 面板内容实际只用了约 327px 宽度。** 在 420×640 视口测得 #panel 宽 327.28px、左右各空约 46px。floating.css 的 `place-items:center` 在切换到 block 后仍留下 justify-items:center，panel 又没有显式宽度。浏览器临时改为 justify-items:normal 或 panel width:100% 后即铺满可用宽度。建议球态与面板态隔离布局规则，恢复卡片内容宽度。位置：[floating.css:14](../apps/desktop/web/floating.css#L14)、[panel.css:7](../apps/desktop/web/panel.css#L7)。证据：[panel-width.json](../.local/ux-review-2026-10-05/panel-width.json)、[现有面板截图](../.local/ui-previews/floating-panel-light.png)。目标 WebView2 仍需验证。

**UI 设计建议（与功能缺陷分开）**

| 优化点 | 当前体验与建议 | 对应位置 |
| --- | --- | --- |
| 首次使用引导 | 两端“添加数据源”都位于统计、图表之后。没有来源时，把连接卡放首屏，给出本机日志／账户限额／SSH 三种入口，再按实际接入显示图表。 | [Web](../apps/desktop/web/app.js#L101)、[macOS](../apps/macos/Sources/Aieyes/Views.swift#L165) |
| 账户关系文案 | “作为账户”难解释来源与共享限额的关系。建议“关联账户并查询限额”，补一句“多个来源可共用同一账户”；DeepSeek／agy 提供适合其只查余额或限额的默认配置。 | [Web 来源编辑器](../apps/desktop/web/app.js#L358)、[macOS 来源编辑器](../apps/macos/Sources/Aieyes/SettingsView.swift#L184) |
| 小面板信息密度 | 修复实际宽度后，将缓存率和输入／输出／缓存做紧凑一行或折叠次要信息；优先保证账户余额、今日 Token 与异常首屏可读，趋势和每日明细放后面。macOS 已有三列 Token 明细，可作为两端信息层级参考。 | [panel.css](../apps/desktop/web/panel.css)、[Views.swift](../apps/macos/Sources/Aieyes/Views.swift#L131) |
| 视觉层级 | Windows 多数卡片都使用大圆角、阴影和 hover 上移，静态统计卡也像可点击按钮。保留主指标层级，普通列表减少阴影，只有能操作的控件给位移反馈。macOS 限额区域和指标卡的上下留白也可适度压缩。 | [style.css](../apps/desktop/web/style.css#L107)、[Surface](../apps/macos/Sources/Aieyes/Views.swift#L19) |
| 字号与点击区域 | Windows 面板的状态／标签多为 12.5–13px；macOS 页脚图标为 plain 且未单独约束点击区域。建议关键辅助信息尽量 14px，图标操作约 28–32px 点击区，退出放进“更多”以减少误触。数值维持 tabular-nums／monospacedDigit。 | [panel.css](../apps/desktop/web/panel.css#L27)、[Views.swift:101](../apps/macos/Sources/Aieyes/Views.swift#L101) |
| 图表范围与单位 | Windows 面板选 30／90 天后仍写“近 7 天用量”；应动态标题或明确固定为最近 7 天。macOS 成本模式的每日明细仍是 Token，建议明确单位并补成本列，便于与图表核对。 | [app.js:124](../apps/desktop/web/app.js#L124)、[DailyUsage](../apps/macos/Sources/Aieyes/Views.swift#L335) |
| 表单和辅助技术 | 价格弹窗与背景映射表重复 field-id，探针确认弹窗模型 ID 没有关联 label。每个表单使用独立 ID；编辑／删除补目标名称，导航补选中状态，macOS Account/Price 的 Return/Escape 行为保持一致。 | [app.js:195](../apps/desktop/web/app.js#L195)、[app.js:452](../apps/desktop/web/app.js#L452)、[SettingsView.swift](../apps/macos/Sources/Aieyes/SettingsView.swift#L341) |
| 原生窗口习惯 | Windows 标题栏品牌子元素没有 drag-region 或指针透传，按所用 Tauri drag.js 的直接命中逻辑，文字／图标处与空白拖动行为不一致。应统一拖动区域。macOS 保留原生窗口控件，两端统一“关闭继续后台、退出结束应用”的提示。 | [index.html:5](../apps/desktop/web/index.html#L5) |

浅深色需分别核对真实合成背景上的对比度，尤其时间、辅助标签和失败色。普通文本可用至少 4.5:1 作为验收基线，大字为 3:1；这来自 [WCAG 2.2 对比度说明](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)。本轮未作完整对比度审计，不能把建议视作已测得所有控件不合格。

**建议实施顺序与验收**

1. 先修 P1 键盘与草稿保留，以及面板空态和右键命令；验收从打开编辑到保存／取消可全程键盘完成，重复打开设置不丢草稿。
2. 再统一主窗／面板状态、自动限额、筛选与成功时间；验收无需重启即可看到设置变化，固定面板数据更新，断网后旧读数明确过期。
3. 最后处理布局、首次引导、密度、文案与视觉反馈；用空数据、1／多个账户、长名称、25+ 映射、连接失败等状态做截图比较。

原生验收重点：Windows 1366×768@150%、1920×1080@200%、混合 DPI、Snap／标题栏、球的悬停开关与拖动吸附；macOS 低可用屏高的 450×720 面板、设置／详情来回打开、VoiceOver、增大对比度与减少透明度。浏览器 deviceScaleFactor 只改变渲染密度，不能替代操作系统文本缩放和混合屏幕测试。

旧 `windows-ui-review.md` 中已修复的主窗口取消接入、事务回滚、标题栏可操作和主窗口明细展开等未作为旧缺陷重复报告；本轮面板明细属于独立的新渲染路径。
