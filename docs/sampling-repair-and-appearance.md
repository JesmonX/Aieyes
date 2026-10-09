# 采样修复与统一外观

## 采样边界及修复

Codex 旧解析器把会话元数据时间或上一次累计用量时间作为区间起点。这会将采样开始后启动的新任务误判为跨边界，尤其影响恢复的会话与延迟开始的自动检查任务。

解析器 v2 读取 `task_started`，仅当本次累计差值等于 `last_token_usage`、任务时间有效时采用更准确的任务边界。跨任务累积、缺失时间和真正跨越采样起点的记录继续阻止估值。本机旧游标在下次扫描时重放，SSH 采集只额外传递任务生命周期和时间，不传递对话内容。重复来源保留经过验证的边界，事件身份、去重和账户归属不变。

估值新增 `calculationStatus`、`calculationVersion`，将计算有效性与 active/pending/completed 生命周期分开。未达阈值、无用量、价格缺失和边界不明显示各自原因。5h、7d 同期/整周、7d 容量倍率及 credits 分别选择最近有效值，并标明历史采样时间。最新无效记录不再遮住旧金额。

采样历史中的“按原记录修复估值”调用 `quotaEstimates.repair` 或 `creditEstimates.repair`，参数为 `{ "id": "原记录 ID" }`。接口同步所选来源、重算边界，并使用原记录的价格快照与容量倍率。已结束且无金额的记录才能修复；价格依据不足时仍失败。修复生成独立记录，通过 `originalEstimateId` 与原记录关联，携带 `repairedAt`，不会覆盖原始结果；重复调用返回同一修正版。普通价格重算仍不改变已结束记录。

## 面板账户

两端使用 `panel.accounts.v2`（Web 端带 `aieyes.` 前缀），按 Agent 保存 auto/custom 模式。auto 动态选择当前排序的前五个有效账户；custom 保留明确选择，空列表表示主动隐藏。非空旧选择全部失效时回退当前默认账户，读取或过滤不会把临时空结果写回。原 v1 偏好可迁移，账户管理中提供“恢复默认显示”。

## 外观

设置增加 `appearance: { theme, accent }`。theme 为 system/light/dark，accent 为 indigo/blue/teal/purple；默认跟随系统、靛蓝。通用设置的主题和强调色选择即时保存，所有窗口同步；Windows 原生悬浮入口同步明暗与强调色，语义状态颜色不变。

主面板和菜单栏／悬浮面板提供太阳／月亮按钮，根据当前显示切换浅色或深色并保存到同一份配置；跟随系统仍可在通用设置选择。按钮只保存主题，保留其他未保存输入，保存失败时保留原主题。macOS 显式同步弹出面板、内容视图与所在窗口的外观，并监听应用的有效外观，避免面板继承菜单栏锚点的主题。Web 窗口接收主题变化时同步本地设置及未修改的表单字段，避免下一次重绘恢复旧主题。

桌面端控件统一采用当前 macOS 面板的圆角、分段切换、输入框、下拉、弹层与卡片层级。保留悬停、按下、展开与入场动效，并遵循减少动态效果、减少透明度和强制颜色设置。订阅标签增至 14，长名称限制在合理宽度内。

重置倒计时达到一天时省略分钟，例如“1 天 2 小时后重置”；不足一天仍显示小时和分钟。旁边的绝对重置时间继续保留分钟，不修改底层限额快照。

## 验证入口

- `cargo test --workspace --locked`：原有核心回归及任务边界、游标升级、去重、修复幂等与冻结价格回归。
- `python3 scripts/test-collectors.py`：SSH 生命周期元数据白名单和内容隔离。
- `sh scripts/test-native.sh`：原生逻辑、账户选择与重置格式回归。
- `npm --prefix apps/desktop run test:ui`：桌面交互、历史金额、修复入口、账户替换、八种主题组合与保存。
- `npm --prefix apps/desktop run test:theme`：先构建 debug 核心，再验证两个浏览器窗口的主题双向同步、重绘、跟随系统、草稿保留、保存失败及重新打开。
- `sh scripts/test-native-theme.sh`：先执行 `sh scripts/build-macos.sh debug`，再验证真实 macOS 弹出面板及 SwiftUI 明暗同步、与锚点相反的主题、重新打开与外观变化。
- `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked`：桌面壳状态与广播回归。

本轮在 macOS 上执行核心、Swift、Tauri 和 Chromium 验证，并生成浅深色原生与桌面截图。Windows/Linux 的系统材质、原生悬浮窗口与实际多屏交互仍需对应系统实机验收。本机数据库备份、修复记录和截图放在被 Git 忽略的 `.local/` 下。
