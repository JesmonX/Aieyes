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
| Antigravity / agy | 标准统计记录导入；agy CLI 只读查询模型组 5h/7d 限额，本机已实测 |
| DeepSeek | 官方余额查询，显示币种、总额、赠送及充值余额；需配置 API Key |
| OpenRouter 定价 | 缺失模型与维度清单、映射、手动补价、自动补计缺项、重算；联网同步待验证 |
| Linux SSH 监控 | 可勾选挂载点及设备、12 项细分显示；独立并行采样、前台约 2 秒间隔；3 台实机验证通过 |
| 账户与数据源 | 统一入口、来源内新建或关联账户、共享限额、保留历史归属 |
| 远程历史、前置命令 | 限额查询可单独配置代理前置命令；远端只回传统计字段 |
| 代理 | HTTP/HTTPS/SOCKS5、来源覆盖、连接测试与延迟；macOS 静态系统代理，其他平台代理环境变量；PAC 和代理认证界面尚未实现 |
| 应用内更新 | 显示当前版本，支持手动及每日检查、签名校验、一键安装重启；Linux 支持 AppImage 与 DEB |
| Windows/Linux | 已实现悬浮球、动态托盘、自动回退及本机会话状态；悬浮球单击或悬停展开与 macOS 菜单栏一致的面板，支持贴边与暂时隐藏；界面为半透明圆润风格，含统一动效与悬停提示（见 [Windows 界面刷新](docs/windows-ui-refresh.md)）；Windows NSIS、Linux deb/AppImage 原生 CI 构建通过；桌面交互和安装卸载待实测 |

服务器通过 CPU、内存、GPU 圆环和设备状态条展示实时资源，尚未保存长期采样历史。Token 历史保存在 SQLite 中。

设置包含数据源、服务器、价格、定时唤醒和通用五个标签。代理位于通用，选择协议并填写 Host、端口，或使用自定义 URL；价格支持按模型 ID 和名称搜索，顶部可「保存并重算」。普通保存跳过历史重算，概览按需后台刷新，macOS 显示保存阶段与耗时。更新按钮在应用内检查、下载安装；无新版时明确提示当前已是最新版本。概览仅保留主指标与输入、输出、缓存明细，计价不足以感叹号进入价格设置。

macOS 配置编辑器的「应用到草稿」更新草稿，设置顶部「保存应用配置」提交全部配置（包括 API Key 与 SSH 密码）；关闭或退出时可保存、放弃或取消。价格条目独立即时保存。Windows 来源/主机编辑器保存后立即生效，删除主机会先说明关联来源受到的影响。服务器可选择 SSH 配置/密钥或账号密码，密码保存在系统凭据库。两端均自动查询已启用账户的限额。macOS 额度采样使用独立窗口，关闭后继续后台采样，菜单栏保留采样标记和管理入口。

Windows 悬浮面板同步设置与服务器采样，固定面板持续显示最新读数；错误与过期数据有明确状态，退出入口位于面板「更多」。本轮完整修复与验证见 [跨平台交互和 UI 评审](docs/cross-platform-ui-review-2026-10-05.md)。 10-06 后续统一刷新、空态、原生确认与中性卡片的落地和验收范围见 [实施记录](docs/ui-reform-2026-10-06-implementation.md)。后续面板层级、草稿保护、数据时间、分析交互和长流程改版见 [全面 UI 审计实施记录](docs/ui-audit-and-redesign-2026-10-06-implementation.md)。

在「设置 → 数据源」添加数据源，选择 agy 或 DeepSeek，并开启「关联账户与限额」新建或关联账户；保存后刷新限额。agy 使用本机或 SSH 位置当前登录的 CLI；DeepSeek 在数据源中填写 API Key。面板默认显示当前或置顶账户的限额摘要，通过「查看全部」访问完整账户列表。

在「设置 → 服务器 → 编辑」点击「读取设备」，按组下拉选择文件系统、GPU、CPU 核心、磁盘、网卡及细分项。选择面板支持搜索、全选、清空和反选；有搜索词时仅操作匹配结果。前台服务器页约每 2 秒发起一次采样（加上请求耗时），后台使用设置间隔；新配置默认 10 秒，旧配置保留。

限额支持账户排序；全部账户默认展开，仅同一 Agent 有多个账户时允许用户逐个折叠。悬浮小窗与主窗口均按概览、账户、其他细分排列；小窗点击总 Token 浮出输入、输出与缓存明细，主窗口直接显示明细。credit 余额与估值入口同排，显示当前余额的 API 等价美元价值。额度估值优先采样 5h 总额度，显示 5h 价值、7d 同期换算和近期容量倍率预估；正常周期重置后自动接续，异常或提前重置后需要确认。旧 7d 采样及历史保留。价格映射区位于顶部，每个模型支持一键填入右侧模型 ID；删除映射后完整重算会清除无法匹配的旧计价。悬浮面板底部显示连接延迟与更新按钮，新版本附感叹号，固定按钮移至「更多」。使用口径与验证范围见 [5h 采样和连接功能](docs/five-hour-and-connection-improvements.md)，原有归因边界见 [限额估值与 UI 评审](docs/quota-value-and-macos-review.md)。

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

Tauri 界面在安装目标系统构建依赖后运行：

```sh
cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --features custom-protocol
```

Windows 使用 `./scripts/build-desktop.ps1 debug`；Linux 使用 `sh scripts/build-desktop.sh debug`。将 `debug` 改为 `release` 可构建安装包，需要先安装 Tauri v2 CLI。macOS DMG 使用 `sh scripts/build-macos.sh release --dmg`，并需要 Python 3.11+。完整依赖、操作方式和平台验证范围见 [Windows / Linux 桌面版](docs/windows-linux.md)。

悬浮球单击或悬停展开与 macOS 菜单栏一致的面板，拖动松手后自动贴边，可暂时隐藏以应对全屏场景；右键打开菜单，Linux 托盘通过原生菜单打开详情和设置。「设置 → 通用 → 桌面显示」可切换模式或重置位置。关闭详情窗口后继续监测，选择「退出 Aieyes」才退出应用。

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
