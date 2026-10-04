# 界面整合验收记录 — 2026-10-04

设置统一为数据源、服务器、价格、通用四个入口。数据源内可新建或共享账户，关闭账户关联仅改变新增记录；代理使用协议与 Host、端口或自定义 URL；价格可搜索；更新固定查询项目仓库。概览保留四张指标卡，Token 明细三项互不重复，计价不足以感叹号进入价格设置。服务器提供资源圆环、设备状态条及简短连接状态。

## 已完成检查

- `cargo test --workspace --locked`：36 项通过，包含解除／恢复账户后的本机追加、完整重扫、模拟 SSH 全量重放、Codex 累计计数、历史筛选和价格快照，以及代理校验。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：通过。
- `node --test scripts/test-desktop.mjs scripts/test-multiselect.mjs scripts/test-release-publish.mjs`：12 项通过。
- `python3 scripts/test-collectors.py`：4 项通过。
- `scripts/test-ui.cjs`：Playwright 浏览器检查通过，使用内存 IPC 测试数据；覆盖新建／共享账户、取消及保存失败、归档恢复、代理回填与 IPv6、价格搜索、更新入口、计价提醒、历史来源筛选、服务器缺失值及状态，保留键盘和多选回归。当前使用缓存中的 Playwright，通过 `AIEYES_PLAYWRIGHT` 指定包路径；标准安装运行方式见 [平台说明](windows-linux.md)。
- Swift 的 8 个应用源文件和 `verify-proxy.swift` 完成语法解析检查；这不等于 Swift 类型检查或原生构建。

截图使用测试数据，并检查了浅深色、840×600 窗口和缩放场景：[概览](../.local/ui-previews/overview-light.png)、[数据源](../.local/ui-previews/sources-light.png)、[来源编辑器](../.local/ui-previews/source-editor-light.png)、[通用](../.local/ui-previews/general-light.png)、[价格搜索](../.local/ui-previews/prices-light.png)、[服务器](../.local/ui-previews/servers-light.png)、[服务器深色小窗口](../.local/ui-previews/servers-dark-compact.png)。截图为本地生成产物，不进入版本控制。

## 待原生验收

本机是 Linux，没有 Swift 工具链，尚未运行本轮 macOS 构建及原生截图。代理纯 Swift 验证已加入 `scripts/test-native.sh`，需在 macOS 运行该脚本及 `sh scripts/build-macos.sh debug`，并检查来源编辑／失败重试、菜单栏紧凑面板、服务器和代理交互。

浏览器 mock 不证明 WebView2、原生窗口、系统浏览器启动或真实账户／SSH／GitHub 联网可用性；Windows 与 macOS 的原生交互仍需目标系统验收。本轮未发布安装包。
