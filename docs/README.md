# 文档索引

本目录只保存长期有效的规格、接入说明和功能摘要。各轮实施方案、审计、验收与证据记录保留在本地 `docs/internal/`，不提交到仓库；其结论汇总在[实现摘要与尚存问题](changes.md)。

## 总览

- [产品规格](product-spec.md)
- [技术设计](architecture.md)
- [开发、实现状态与验证](development.md)
- [实现摘要与尚存问题](changes.md)
- [发布流程](releases.md)，各版本说明见 [release-notes/](release-notes/)
- [手动验证与维护入口](manual-verification.md)

## Agent 接入与账户

- [账户、数据源、前置命令与计价](adapters.md)
- [Agent 订阅、原生用量与账户接入](agent-subscriptions.md)
- [Agent 接入、账户与设备设置](agents-and-accounts-settings.md)
- [Codex 多账号管理](codex-accounts.md) · [Codex API 接入](codex-api.md)
- [Qoder / Qoder CN](qoder-usage.md)
- [刷新、监测与 Antigravity 接入](refresh-monitoring-antigravity.md)

## 额度、估值与历史

- [5h 采样和连接功能](five-hour-and-connection-improvements.md)
- [Credits 估值与订阅定时唤醒](credits-and-wakeups.md)
- [总览布局、菜单与滚动估值](overview-and-rolling-estimates.md)
- [采样修复与统一外观](sampling-repair-and-appearance.md)
- [订阅额度重置提醒](quota-reset-notifications.md)
- [历史日期范围](history-date-ranges.md) · [手动全量重扫](manual-history-rescan.md)

## 会话状态与桌宠

- [桌面宠物与会话状态](desktop-pets.md) · [桌宠导入与选择](pets.md)
- [Codex 活动会话来源](codex-session-sources.md) · [Claude 会话状态来源](claude-session-sources.md)
- [Headless 与内部会话排除规则](headless-rules.md)
- [会话提示音](session-sounds.md)

## 界面与平台

- [三端控件与样式规格](ui-control-spec.md)，设计数据：`ui-design-tokens.json`、`ui-phase-colors.json`、`chart-palettes.json`
- [图表预置配色](chart-palettes.md) · [Agent 图标](provider-icons.md)
- [Windows / Linux 桌面版](windows-linux.md) · [Windows 外观与交互](windows-appearance.md)
- [官网](website.md)

## 生成源数据

`core-contracts.json`、`ui-design-tokens.json`、`ui-phase-colors.json`、`chart-palettes.json` 由 `scripts/sync-*.py` 生成到各端代码，修改后运行对应同步脚本；`protocols/` 保存已核对的外部协议 schema。
