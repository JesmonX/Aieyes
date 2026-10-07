# UI 优化实施细则 · 2026-10-07

以 `ui-review-and-optimization-2026-10-07.md` 和本次用户确认作为实施依据。历史审计中的单账户摘要、置顶和硬性首屏账户预算不再适用。

## 已确定的行为

1. 面板宽 450，高度适配工作区；540 高度验收今日双指标可见、正文可滚动、固定底部可用。顺序为会话、筛选、今日双指标、账户限额、采样和趋势。完整账户卡可折叠成关键限额摘要，摘要显示账户名、前两个限额窗口及余额/credits；展开恢复完整内容。废弃置顶及“查看全部”弹窗。
2. 每个 Agent 的面板账户选择范围为 0～5。首次按现有账户顺序取前 5 个；显式空选择保持为空。面板“更多”提供选择入口，详情仍显示全部。选择仅在本机保存，不改变账户查询开关、数据采集或共享配置。账户归档后不占可选名额。
3. macOS 鼠标打开面板不主动激活应用；再次点击菜单栏项关闭面板。未固定时外部点击关闭，固定时保持。排序、账户选择、设置、估值和采样用独立窗口承载，离开面板时先关闭面板。下次打开不继承模态会话造成的失效关闭行为。
4. 七个会话阶段为 idle / working / thinking / tool / complete / interrupted / unknown；恢复中、恢复失败、状态不全作为独立外壳反馈。阶段色由一张表管理，CSS 与 Rust 使用相同浅深色，macOS 保留表中系统语义色映射。品牌图不重着色。计数默认关闭，可开启，显示为徽标；所有状态均有可读名称。
5. Linux 胶囊窗口与内容均为 144×44，吸附与面板锚点按可见尺寸计算。Windows 和 Linux 增加即时悬停/按下反馈，不加持续动画。重建菜单明确为“重建面板（用于界面无响应）”。Windows 非激活胶囊的键盘分支不能作为已验证的可达入口；键盘文档只描述实际可聚焦的 Web 入口及系统托盘。
6. 面板底部为状态一行、动作一排。出站结果并入状态全文及 tooltip，测试放入更多菜单。两个菜单共用背景、阴影、宽度和行高。用量详情统一为整宽次按钮；趋势为一个带范围的折叠项，内部直接展示每日明细。
7. Windows / Linux 主窗最小内容尺寸统一 640×440；macOS 保留原生 760×480。Web 编辑器按对象设宽：来源 570、主机 620、账户 560、价格 520；主机先“连接并读取设备”、后监控参数。初始焦点取消，Escape 和 Tab 保持现有草稿保护。
8. 资源用量 ≥90% 为危险，≥70% 为注意；限额剩余 <10% 为危险、≤30% 为注意。圆环与进度条共用语义色。面板圆环 60，主窗 72；高负载提供文字，未知数值不当成零。
9. macOS 设置去掉重复正文标题；顶部明确保存规则。面板页签、趋势、服务器筛选和账户展开状态持久保存并区分详情。桌面面板沿用现有重建恢复机制。
10. 删除逐条证实无引用的组件、选择器、命令和令牌；保留有效 quota-error 展开/收起规则。统一面板字号令牌，移除自定义面板缩放开合，减少动效设置关闭新增过渡。

## 保存语义（用户已确认）

桌面端统一为编辑器更新草稿、顶部保存全部，包括来源、主机、账户、通用配置和模型映射。运行与轮询只读取已提交配置。保存失败保留草稿；凭据在总保存时准备，SSH 密码保存失败清理新引用；价格、定时唤醒及本机显示偏好独立保存。未保存计数按变更的来源/主机/账户各一项、通用配置一项、模型映射一项，暂存凭据计入对应行，避免重复计数。提交前重新读取运行配置；发现其他窗口已经修改时保留草稿并阻止覆盖，放弃后从最新配置重新编辑。成功提交或放弃时重置表单基线，关闭/退出与应用更新使用同一份未保存状态。

## 组件语义与共同样本

| 组件 | 规格 | 共同样本 |
| --- | --- | --- |
| StatusBadge | 图标/文字与颜色并存，未知不等于空闲 | thinking，2 个活跃会话；状态不全；恢复失败 |
| MetricCard | 主值可复制、等宽数字；无数据用 — | 2.40B Token；API 等价成本 $100,000.25 |
| QuotaCard | 账户名、窗口、剩余量、重置时间；折叠持久保存 | 5h 剩余 70%，7d 剩余 45%；20 个账户每 Agent 最多 5 个 |
| FilterBar | 区分请求筛选与已应用筛选，失败保留上一结果 | 今日 / 全部 Agent；失败后恢复已应用筛选 |
| EditableRow | 动作有对象名称；归档保留历史且可恢复 | 工作账户；训练服务器；本机日志 |
| InlineError | 原因和可执行重试，成功提示不覆盖新错误 | SSH 连接失败；限额读取失败 |
| EmptyState | 保留筛选和设置入口，不把未知显示为零 | 无来源；筛选无记录；面板选择 0 个账户 |
| SaveBar | 明示保存作用域、未保存数量与保存中状态 | 配置两项修改；价格即时保存；任务独立保存 |
| TaskProgress | 可关闭、可恢复，不重复提交 | 采样进行中；待确认来源；恢复失败 |

## 验证与证据

所有测试与截图均使用隔离的合成数据，不修改真实账户、SSH、定时任务或个人显示偏好。

| 检查 | 验证范围 |
| --- | --- |
| 桌面 8 个浏览器回归脚本 | 草稿跨页保存、顶部提交、放弃与失败重试、外部修改冲突、凭据清理、保存后更新、模型映射与重算、采样和定时唤醒 |
| `node scripts/test-desktop.mjs`（15 项） | 每 Agent 上限、显式 0、首次排序、归档清理、入口点击/拖动、Escape、菜单焦点及外壳导航 |
| `sh scripts/test-native.sh` | Swift 草稿提交/回滚、筛选快照、会话生命周期、凭据与采样失败、配色对比度、面板账户偏好 |
| Swift 原生构建 | SwiftUI / AppKit 全应用编译与链接 |
| 桌面 Rust 测试（18 项） | 胶囊吸附、面板锚点、工作区与混合 DPI、品牌图保留、配置与状态广播 |
| `sh scripts/check-native-capsule.sh` | 使用 Windows 目标类型检查原生胶囊与非激活窗口适配器，宿主胶水使用替身；阶段表一致性检查 |
| 六状态原生截图矩阵 | 空态、单账户、多账户、长名称/大数字、失败、20 账户；浅深色、450×540、760×480、620×440、折叠摘要与选择窗口 |
| 高对比截图像素断言 | Web 不透明 Canvas 与可见 CanvasText；原生浅深色不透明表面与实色边框 |

以上检查已通过。浏览器 8 个脚本均完成验证，原生六状态矩阵生成 267 张截图；最终宿主构建另核对恢复详情页签时工具栏选中项保持一致。

本轮同时修复回归中发现的两个问题：切换深浅色时不再用旧状态关闭面板；顶部保存后不再因旧表单基线残留而阻止应用更新。清理已弃用命令的测试替身，并把“单账户不可折叠”的旧断言更新为“折叠摘要及重建后恢复”。

当前源码截图位于 `.local/ui-audit-implementation/` 和 `.local/ui-previews/`。主要样本：

- [桌面 450×540 浅色](../.local/ui-audit-implementation/panel-five-540-light.png)、[深色](../.local/ui-audit-implementation/panel-five-540-dark.png)、[强制高对比](../.local/ui-audit-implementation/panel-forced-colors.png)
- [macOS 450×540](../.local/ui-audit-implementation/macos/multi/low-panel-light.png)、[折叠摘要](../.local/ui-audit-implementation/macos/single/quota-folded-summary.png)、[20 账户选择](../.local/ui-audit-implementation/macos/capacity/panel-accounts.png)
- [桌面筛选失败](../.local/ui-audit-implementation/filter-failed.png)、[恢复](../.local/ui-audit-implementation/filter-restored.png)；[macOS 筛选失败](../.local/ui-audit-implementation/macos/multi/filter-failed.png)、[恢复](../.local/ui-audit-implementation/macos/multi/filter-restored.png)
- [macOS 最小设置窗口](../.local/ui-audit-implementation/macos/long/settings-prices-small.png)、[增大对比度](../.local/ui-audit-implementation/macos/multi/increased-contrast-dark.png)

运行边界：Windows / Linux 实机、Windows 完整安装包、VoiceOver 全流程、真实 SSH，以及 macOS 菜单栏反复点击/外部点击的真实交互本轮未观测；这些不能用类型检查和离屏截图代替。Linux 的绝对位置与置顶仍受 Wayland 合成器限制，见 [桌面平台说明](windows-linux.md)。本轮不发布安装包。
