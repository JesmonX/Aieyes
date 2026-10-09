# Aieyes

跨平台 Agent 用量与 SSH 服务器监控工具。macOS 使用原生菜单栏，Windows 默认使用悬浮球，Linux 优先使用状态栏、不支持时自动回退悬浮球。SwiftUI/AppKit 与 Tauri 界面共享 Rust 核心和 SQLite 本地存储。

## 下载安装包

从 [GitHub Releases](https://github.com/JesmonX/Aieyes/releases) 下载正式版本：macOS 提供 Intel x64、Apple Silicon arm64 两个 DMG（macOS 14+），Windows 提供 x64 NSIS EXE，Linux 提供 x64 DEB 和 AppImage。发布页的 `SHA256SUMS` 包含全部安装包校验值。

[Release 工作流](https://github.com/JesmonX/Aieyes/actions/workflows/release.yml) 在推送 `v主版本.次版本.修订版本` 标签后构建，所有平台测试和打包成功后自动发布。默认 macOS 使用 ad-hoc 签名，Windows 暂未证书签名；发布前需要配置应用内更新签名密钥，详见 [发布说明](docs/releases.md)。首次运行新工作流前，仍可从 [Desktop 工作流](https://github.com/JesmonX/Aieyes/actions/workflows/desktop.yml) 下载保留 14 天的开发构建。

发布维护与验证方式见 [发布说明](docs/releases.md)。

## 当前开发版

macOS 开发包输出到 `dist/Aieyes.app`，目前已验证 Intel macOS 14+ 构建。双击后从系统菜单栏的眼睛图标打开；右键可打开详情、设置或退出。关闭详情窗口后继续留在菜单栏。

调试包的数据位于项目的 `.local/app/`，发布构建使用 `~/Library/Application Support/Aieyes/`。调试包包含当前项目路径，请保留项目目录。应用内置核心程序，运行时不需要 Node 或 Rust。

```sh
sh scripts/build-macos.sh debug
open dist/Aieyes.app
```

已有依赖缓存时可使用 `AIEYES_OFFLINE=1 sh scripts/build-macos.sh debug`。

## 功能状态

| 功能 | 当前状态 |
| --- | --- |
| 原生菜单栏、固定面板、详情与设置 | 当前界面已完成 macOS 原生构建、模拟数据截图及草稿/限额回归；VoiceOver 与真实多屏交互待实机验收 |
| Codex 历史统计 | 已用本机真实日志验证；增量导入、去重、账号和来源筛选 |
| Claude Code 历史统计 | 解析和缓存归一化已实现；合成格式测试通过，本机暂无有效用量 |
| 时间范围、趋势与模型 | 默认今日概览；7/30/90/365 天、按模型堆叠、逐日缓存率与 365 天热力图 |
| Codex 限额、Bank Reset | 日志限额已验证；实时协议由本机 CLI 核对，联网读取待验证；Bank Reset 只读 |
| Claude Code 限额 | 本机及远程 OAuth 读取、远程代理前置命令；联网读取待验证 |
| Antigravity（agy CLI） | 本机及 SSH 会话 Token 采集、标准 JSONL 导入、模型组 5h/7d 限额与分组估值；旧 agy 数据自动迁移 |
| DeepSeek | 官方余额查询，显示币种、总额、赠送及充值余额；需配置 API Key |
| OpenRouter 定价 | 缺失模型与维度清单、映射、手动补价、自动补计缺项、重算；联网同步待验证 |
| Linux SSH 监控 | 可勾选挂载点及设备、12 项细分显示；独立并行采样、前台约 2 秒间隔；3 台实机验证通过 |
| Agents 与账户 | Agent 启停与机器多选、独立账户页、跨机器共享额度、操作即保存 |
| 远程历史、前置命令 | 限额查询可单独配置代理前置命令；远端只回传统计字段 |
| 代理 | HTTP/HTTPS/SOCKS5、来源覆盖、连接测试与延迟；macOS 静态系统代理，其他平台代理环境变量；PAC 和代理认证界面尚未实现 |
| 应用内更新 | 显示当前版本，支持手动及每日检查、签名校验、一键安装重启；Linux 支持 AppImage 与 DEB |
| Windows/Linux | 已实现悬浮球、动态托盘、自动回退及本机会话状态；悬浮球单击展开，悬停查看提示，展开后呈现与 macOS 菜单栏一致的面板，支持贴边与暂时隐藏；界面为半透明圆润风格，含统一动效与悬停提示（见 [Windows 界面刷新](docs/windows-ui-refresh.md)）；Windows NSIS、Linux deb/AppImage 原生 CI 构建通过；桌面交互和安装卸载待实测 |

服务器通过 CPU、内存圆环和 GPU／文件系统细进度条展示实时资源，尚未保存长期采样历史。Token 历史保存在 SQLite 中。

设置包含 Agents、服务器、价格、账户、定时唤醒和通用六个标签。代理位于通用，选择协议并填写 Host、端口，或使用自定义 URL；价格支持按模型 ID 和名称搜索，顶部可「保存并重算」。普通保存跳过历史重算，概览按需后台刷新，macOS 显示保存阶段与耗时。更新按钮在应用内检查、下载安装；无新版时明确提示当前已是最新版本。概览仅保留主指标与输入、输出、缓存明细，计价不足以感叹号进入价格设置。

主面板与菜单栏／悬浮面板均提供太阳／月亮按钮，一键切换明暗并同步保存到「设置 → 通用 → 主题」。菜单栏弹出面板跟随应用主题，悬浮面板在跨窗口更新及重新打开后保持一致；仍可在通用设置选择跟随系统。

macOS 与 Windows/Linux 的 Agent 开关、机器勾选和服务器启停直接保存；账户、服务器及高级设置表单点击「保存」即生效，不再需要顶部二次保存。通用设置自动保存，失败保留输入并显示具体原因。配置读写使用独立通道，不等待远程额度或 SSH 操作；后台刷新与保存分开提示。核心按本次修改合并配置，独立窗口修改同一字段时明确报冲突，避免覆盖登录档案。服务器密码保存在系统凭据库。额度采样仍可在关闭工具窗口后后台运行。

Windows 悬浮面板同步设置与服务器采样，固定面板持续显示最新读数；错误与过期数据有明确状态，退出入口位于面板「更多」。本轮完整修复与验证见 [跨平台交互和 UI 评审](docs/cross-platform-ui-review-2026-10-05.md)。 10-06 后续统一刷新、空态、原生确认与中性卡片的落地和验收范围见 [实施记录](docs/ui-reform-2026-10-06-implementation.md)。后续面板层级、草稿保护、数据时间、分析交互和长流程改版见 [全面 UI 审计实施记录](docs/ui-audit-and-redesign-2026-10-06-implementation.md)。

在「设置 → Agents」启用 Codex、Claude Code、Antigravity、DeepSeek 或自定义 Agent；支持本机／SSH 的 Agent 可以多选机器。数据目录和 CLI 位于 Agent「高级设置」，服务器 Shell 位于服务器高级设置。前置命令位于每个账户的设备设置，仅用于账户操作和唤醒，不参与历史同步及监控。在「账户」添加登录或 API Key：Codex 登录成功后自动添加账号并查询额度，取消或失败不创建空账户；Claude Code、Antigravity 使用各机器已有登录；DeepSeek 在本机查询余额。账户按 Agent 分组，同时显示已登录／使用中的设备计数；优先查询位置在账户设置顶部。同一账户可以连接多个位置，额度只显示一份，凭据不跨机器复制。面板每种 Agent 可选择 0～5 个账户，可在账户页或「更多 → 面板显示账户」调整。停用保留历史、账户和配置，已归档账户需要明确恢复，也可删除配置并保留历史与凭证。删除会清理关联唤醒任务，弃用设备可确认强制解除管理。迁移与验证说明见 [Agents 与账户设置](docs/agents-and-accounts-settings.md)。

在「设置 → 服务器 → 编辑」点击「读取设备」，按组下拉选择文件系统、GPU、CPU 核心、磁盘、网卡及细分项。选择面板支持搜索、全选、清空和反选；有搜索词时仅操作匹配结果。前台服务器页约每 2 秒发起一次采样（加上请求耗时），后台使用设置间隔；新配置默认 10 秒，旧配置保留。

限额支持账户排序；账户卡默认展开，单个账户也可折叠为摘要。悬浮小窗与主窗口均按概览、账户、其他细分排列；小窗点击总 Token 浮出输入、输出与缓存明细，主窗口直接显示明细。credit 余额显示当前余额的 API 等价美元价值，下一行是每 1000 credits 的估值入口。5h／7d 额度估值合并为单行，7d 按“容量倍率／同期消耗”排列。额度估值优先采样 5h 总额度，显示 5h 价值、7d 同期换算和近期容量倍率预估；正常周期重置后自动接续，异常或提前重置后需要确认。旧 7d 采样及历史保留。价格映射区位于顶部，每个模型支持一键填入右侧模型 ID；删除映射后完整重算会清除无法匹配的旧计价。悬浮面板底部显示连接状态与更新按钮，新版本附感叹号，固定与出站测试入口位于「更多」。使用口径与验证范围见 [5h 采样和连接功能](docs/five-hour-and-connection-improvements.md)，原有归因边界见 [限额估值与 UI 评审](docs/quota-value-and-macos-review.md)。

Codex 账户新增 credits 余额及每 500 / 1000 credits 的 API 等价估值。设置中的“定时唤醒”支持每天多个固定时刻、本机或 SSH Linux 部署、模型与 effort 选择；退出应用后继续执行。使用方式、归因边界和验证范围见 [Credits 与定时唤醒](docs/credits-and-wakeups.md)。

## 构建与验证

macOS 需要完成 Xcode 初始化并安装 Rust。首次构建需获取 Cargo 依赖。

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 scripts/test-collectors.py
node --check apps/desktop/web/app.js
node --check apps/desktop/web/tooltip.js
sh scripts/build-macos.sh debug
```

主题同步回归（先完成上面的 debug 构建）：`node scripts/test-theme-sync-ui.cjs`；macOS 实际弹出面板明暗切换验证：`sh scripts/test-native-theme.sh`。

Tauri 界面在安装目标系统构建依赖后运行：

```sh
cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --features custom-protocol
```

Windows 使用 `./scripts/build-desktop.ps1 debug`；Linux 使用 `sh scripts/build-desktop.sh debug`。将 `debug` 改为 `release` 可构建安装包，需要先安装 Tauri v2 CLI。macOS DMG 使用 `sh scripts/build-macos.sh release --dmg`，并需要 Python 3.11+。完整依赖、操作方式和平台验证范围见 [Windows / Linux 桌面版](docs/windows-linux.md)。

悬浮球单击展开，悬停查看提示，展开后呈现与 macOS 菜单栏一致的面板，拖动松手后自动贴边，可暂时隐藏以应对全屏场景；右键打开菜单，Linux 托盘通过原生菜单打开详情和设置。「设置 → 通用 → 桌面显示」可切换模式或重置位置。关闭详情窗口后继续监测，选择「退出 Aieyes」才退出应用。

核心可独立运行 JSON-RPC stdio 或单次命令：

```sh
cargo run -p aieyes-core -- --data-dir .local/app --call sources.scan
cargo run -p aieyes-core -- --data-dir .local/app --call dashboard
```

原生界面渲染入口需在有 macOS 图形会话的终端执行：

```sh
dist/Aieyes.app/Contents/MacOS/Aieyes --render "$PWD/.local/previews"
```

## 菜单栏会话状态

菜单栏和面板显示本机 Codex 会话的日志状态：进行中、思考中、执行工具、已完成、中断。展开会话条可查看各会话，颜色与图标同时区分状态；动效遵循系统「减少动态效果」。界面使用中性卡片与分层字号；面板优先显示当前账户限额、今日用量与缓存摘要，详情提供完整分析。

每 5 秒检查已有日志，每 30 秒发现新会话。连续 3 分钟无日志更新会标记「状态待确认」，避免将退出或失联的会话一直显示为工作中；完成、中断保留 90 秒，待确认状态最多保留 1 小时。监测仅读取已启用的本机 Codex 数据源，不代表进程存活、审批等待或远程会话状态。

会话解析与过期行为验证：

```sh
swiftc -module-cache-path .build/swift-module-cache apps/macos/Sources/Aieyes/Models.swift apps/macos/Sources/Aieyes/EngineClient.swift apps/macos/Sources/Aieyes/SessionActivity.swift scripts/verify-sessions.swift -o .build/verify-sessions
.build/verify-sessions
```

## 接入与设计

- [账户、数据源、前置命令与计价](docs/adapters.md)
- [产品规格](docs/product-spec.md)
- [技术设计](docs/architecture.md)
- [实现与验收记录](docs/implementation-plan.md)
- [既有验证结果](docs/verification.md)
- [界面整合验收记录](docs/ui-simplification.md)

应用使用直接的状态与操作文案。演示数据不混入真实统计；项目不包含真实登录凭据或服务器私钥。

本轮 UI 实施与验收细则见 [2026-10-07 UI 优化实施](docs/ui-review-and-optimization-2026-10-07-implementation.md)。

采样误判修复、历史结果恢复、自动账户选择和主题配置见 [采样修复与统一外观](docs/sampling-repair-and-appearance.md)。

紧凑资源布局、Antigravity 统一与分组估值的迁移和验证见 [实施记录](docs/compact-antigravity.md)。

Codex 多账号管理的启用方式、凭据协调与验收边界见 [Codex 账号管理](docs/codex-accounts.md)。
