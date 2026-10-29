# 手动验证与维护入口

从仓库根目录运行。共享 Python 检查支持 Python 3.9+，旧版本复用仓库内 Tomli 解析 TOML；发布打包仍按对应构建脚本的依赖要求执行。共享检查和浏览器检查先执行 `npm ci --prefix apps/desktop`；浏览器检查安装 Playwright Chromium，WebKit 通道另安装 WebKit。核心集成检查使用先构建的 `target/debug/aieyes-core`。原生检查需要 macOS、Swift 和已解析的 Sparkle 依赖。

自动化入口：`sh scripts/ci-checks.sh [--rust]`、`npm run test:ui --prefix apps/desktop`、`sh scripts/test-native.sh`。共享检查包含发布、采集、唤醒、增量历史、旧账户历史、Antigravity 身份、Node 交互、重置格式及生成契约；远端 Codex 权限/锁测试依赖 POSIX，在 Unix 检查任务执行。Web 测试链包含真实核心的主题同步；`verify-models.swift` 在原生测试链中使用真实核心输出。面板偏好和重置格式由两端读取同一组 JSON 夹具。

统一工作区的导航、账户摘要、筛选隔离与本轮验证结果见 [2026-10-15 工作区 UI 改版](workspace-redesign-2026-10-15.md)。`node scripts/test-workspace-ui.cjs` 可单独运行工作区交互检查并生成 Web 预览，原生 24 张截图矩阵的命令见该记录。

以下工具保留为按需验证或维护入口；性能基准、真实联网下载及截图矩阵不作为每次 CI 的默认步骤。

| 工具 | 用途与入口 |
|---|---|
| [compare-ui.py](../scripts/compare-ui.py) | 对照已生成的两端截图，`python3 scripts/compare-ui.py`；先查看脚本定义的截图目录 |
| [generate-icons.py](../scripts/generate-icons.py) | 重新生成品牌图标，`python3 scripts/generate-icons.py`；会更新图标资产 |
| [test-native-performance.sh](../scripts/test-native-performance.sh) | `sh scripts/test-native-performance.sh`，链接真实 Swift debug 对象并测量视图性能 |
| [test-server-performance.sh](../scripts/test-server-performance.sh) | `sh scripts/test-server-performance.sh`；`--deadlines-only` 单独验证调度暂停、恢复和退出 |
| [test-streaming-core.py](../scripts/test-streaming-core.py) | `python3 scripts/test-streaming-core.py`，真实核心与临时 SSH 流式夹具；测试中需要本机进程及监听权限 |
| [test-update-transport.py](../scripts/test-update-transport.py) | `python3 scripts/test-update-transport.py`，真实核心子进程和本机录制代理，不访问公网 |
| [benchmark-performance.py](../scripts/benchmark-performance.py) | `python3 scripts/benchmark-performance.py --soak-seconds 3600`，默认使用 release 核心及临时合成库；可用 `--binary` 和 `--output` 指定路径 |
| [test-runtime-performance.sh](../scripts/test-runtime-performance.sh) | `sh scripts/test-runtime-performance.sh .local/runtime-performance 2400 600`，菜单/桌宠真实图层动画、隐藏页面与物理内存长测；只使用隔离数据 |
| [benchmark-runtime-hotpaths.py](../scripts/benchmark-runtime-hotpaths.py) | `python3 scripts/benchmark-runtime-hotpaths.py --before <旧核心>`，合成采样历史与未变化会话读取对照 |
| [test-estimate-display.py](../scripts/test-estimate-display.py) | `python3 scripts/test-estimate-display.py`，调用真实核心并比较 Web/Swift 完整历史与展示投影；已纳入性能 UI 套件 |
| [benchmark-remote-history.py](../scripts/benchmark-remote-history.py) | `python3 scripts/benchmark-remote-history.py`，全量与增量采集性能对照 |
| [verify-update-transfer.swift](../scripts/verify-update-transfer.swift) | 可选真实联网读取、下载和签名校验；需要包含 Sparkle 更新源、公钥的临时 plist，见 [传输说明](macos-performance-and-update-transport.md)；不安装更新 |
| [test-ui-native-render.sh](../scripts/test-ui-native-render.sh) | `sh scripts/test-ui-native-render.sh`，合成场景原生截图矩阵；可用 `AIEYES_RENDER_BINARY`、`AIEYES_RENDER_ROOT` 指定构建与输出 |

Tauri 宿主测试先运行 `node scripts/prepare-wakeup.cjs --debug`，再运行 `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --features custom-protocol`。Windows/macOS Tauri 测试在上述 Cargo 命令中增加 `--features native-tray`；Linux 不启用此特性。打包资源只匹配真实核心二进制，CI 也按此顺序准备资源。

测试输出放在 `.local/`；历史快照与过期安装包放在 `.local/archive/YYYY-MM-DD/`。这些文件不代表当前源码或可发布产物。

桌宠图形可单独执行 `sh scripts/test-native-pets.sh`，也随 `sh scripts/test-native.sh` 在 Core/macOS 工作流的 Apple Silicon 任务中运行。原生验证脚本通过 `scripts/native-verifiers.sh` 复用一次 debug 构建的 App 对象，以 `@testable import Aieyes` 链接，不再逐个重新编译源码；与 Foundation 同名的类型（如 `Host`）需在验证脚本中用 `typealias` 指明。窗口显示与隐藏检查按就绪状态等待，最多 5 秒，避免固定延迟误判。

`verify-pets` 的菜单 symbol effect、SwiftUI 窗口与截图集成测试在独立子进程运行。仅当 Intel CI 使用空设备或 Apple Paravirtual GPU，且子进程收到 SIGABRT 并输出 MTLLoader 的 `Target device architecture is nil` 驱动断言时，明确标记该环境不支持 SwiftUI 渲染；其他失败均使测试失败。关键帧时序、纯几何和 Core Animation 窗口生命周期仍验证，菜单 symbol effect 在有 Metal 设备时验证。真实图形验收运行 `.build/verify-pets --require-metal-device`，禁止以上降级；`.build/verify-pets --without-metal-device` 可验证无 GPU 分支。Intel 与 Apple Silicon 均纳入原生 CI，物理 GPU 和支持渲染的 Apple Silicon 环境执行完整窗口与截图检查。

跨语言契约维护：编辑 [core-contracts.json](core-contracts.json)，运行 `python3 scripts/sync-core-contracts.py`（需要 rustfmt）；`--check` 检查生成文件，已纳入 CI。可用 `target/debug/aieyes-core --call methods.manifest` 查看核心通道、失效事件和刷新键。

## 2026-10-26 界面优化：实机验收清单

[优化方案](ui-review-and-optimization-2026-10-26.md) 阶段 2 的原生改动无法在 CI 中完成验收，按平台和系统版本记录如下。“已验证”注明日期与方式，其余项发布前需在真实桌面逐项确认。

**Windows 11 25H2（build 26200）**：2026-10-27 用 dev 构建运行 `pwsh -File scripts/test-windows-panel.ps1 -VerifyMaterials`，43 项检查通过；2026-10-28 用含 W-12、W-16 改动的 dev 构建复跑，46 项检查通过。

| 项目 | 状态 | 验收方式 |
| --- | --- | --- |
| W-05 全屏时隐藏桌宠 | 已验证 | 脚本用覆盖整个屏幕的无边框窗口（系统报告 `QUNS_BUSY`）检查隐藏，关闭后原位恢复、置顶、不抢焦点；全屏期间提示音静音需人工确认 |
| W-06 面板为工具窗口 | 已验证 | 脚本断言 `WS_EX_TOOLWINDOW` 且无 `WS_EX_APPWINDOW`；可再目测任务栏和 Alt+Tab |
| W-08 材质 | 已验证 | 脚本读取 `DWMWA_SYSTEMBACKDROP_TYPE`（面板 3、主窗口 2），在可见边框内采样：未激活面板随背景变化（Acrylic），主窗口不透出后面的窗口（Mica） |
| W-16 鼠标钩子独立线程 | 已验证 | 上述脚本的外部点击关闭与 12 次开合通过；面板打开时挂起桌宠线程 2.5 s（超过系统 1 s 的钩子超时上限），期间点击面板外，背景程序约 0.1 s 内收到点击、面板关闭；线程恢复后钩子仍有效 |
| W-12 首帧后显示主窗口 | 已验证 | 全新数据目录冷启动，逐帧截取窗口上部：前 3 次窗口上屏的第一帧都已有标题栏和侧栏，热启动约 0.97 s 上屏；之后在最终构建上又测 8 次，7 次第一帧完整，1 次空白不超过 47 ms；cloak 时间为 84–470 ms，最长一次等到 0.5 s 的原生兜底才解除，窗口约 1.5 s 才上屏。改动前先出现空白窗口 0.33–0.51 s；只把显示推迟到页面绘制后仍有 2/3 次空白约 0.14 s（隐藏时绘制的帧在显示后不复用），所以窗口先以 DWM cloak 方式显示，页面在显示后的窗口内绘制一帧再解除 |
| W-13 拖动结束后再按 DPI 适配 | 待验证 | 在不同缩放的两块屏幕之间拖动主窗口：拖动中窗口不跳离光标，松开后才贴合新屏幕工作区 |
| W-01 标题栏按钮 | 已验证 | 2026-10-28 用 UI 自动化读取按钮位置并取样屏幕像素：三个按钮 46×40、无间隙，关闭按钮贴齐右上角；悬停关闭为 `#C42B1C`；点击其他窗口后图形变淡（最深像素 418→589），重新激活后恢复；最大化后关闭按钮到达屏幕右上角，指针放在屏幕角落即悬停关闭。按下态尚未检查 |
| W-02 Snap Layouts 与系统菜单 | 已验证 | 2026-10-28：最大化按钮下的窗口为原生命中目标；悬停后按钮下方出现 Snap Layouts 浮层，按钮悬停底色与页面自身悬停相同；单击最大化/还原，目标随按钮移动，关闭按钮仍由页面处理。右键标题栏弹出系统菜单（还原、移动、大小、最小化、最大化、关闭），Esc 关闭；拖动标题栏移动窗口，双击最大化/还原。Windows 10 上只需确认拖动、双击和系统菜单 |
| W-03 桌宠标签 | 部分验证 | 2026-10-28 在设置中切换语言和主题后截取桌宠（分层窗口，需 `CAPTUREBLT`）：英文为“Idle”“Import a pet”，没有中文；深色主题为深底浅字；阶段色圆点正常。修复了启动时的回退：原生层改为启动时读取已保存的语言和主题，页面不再把 localStorage 中可能过期的外观推给原生层，所以英文、深色用户重启后桌宠第一帧即为英文深色。计数徽标需要活跃会话，尚未验证 |
| W-04 右键菜单 | 已验证 | 2026-10-28 用 UI 自动化读取菜单项：输入框为表情符号、撤消、剪切、复制、粘贴、粘贴为纯文本、全选；选中文字只有复制；普通文字处没有菜单 |
| W-10 托盘图标 | 待验证 | 100%/150% 下徽标清晰；明暗跟随任务栏（不是应用主题），切换系统主题后更新 |
| W-11 通知 | 待验证 | 安装版点击通知打开主窗口；更新重启后任务栏按钮合并 |
| W-14、W-15 | 已验证 | 2026-10-28：Ctrl+F、F3 不产生新窗口，截图中也没有查找栏；“选择本地文件…”打开的对话框所有者为主窗口、没有 `WS_EX_APPWINDOW`，打开期间主窗口禁用，关闭后恢复 |

**Windows 11 21H2（build 22000–22620）**：未测试。检查 DWM 圆角、旧版模糊回退的可读性（系统没有 `DWMWA_SYSTEMBACKDROP_TYPE`，`-VerifyMaterials` 按模糊验证两个窗口）。

**Windows 10（build 17763 及以上）**：未测试。直角窗口保留阴影、网页无圆角（W-07）；旧版模糊与不透明回退可读；拖动、双击标题栏、最小化/最大化/关闭、边缘贴靠、Win+方向键、边缘缩放。

**所有 Windows 版本**：125%/150%/混合 DPI；高对比度主题。`test-windows-panel.ps1` 是无 BOM 的 UTF-8，需要 PowerShell 7。

**Linux**：L-01、L-02、L-05、L-06、L-07、L-09、L-10、L-11 已补充实现。2026-10-28 在 `hw` 的隔离 Debian 12 环境完成真实 GTK/WebKitGTK 构建与虚拟 Wayland / X11 测试，覆盖启动、RPC、面板开关／固定／重建、辅助进程重启及耗尽重试后返回主窗口、窄设置页和 GNOME 原生小时制。Weston 与 Xvfb/Openbox 分开运行；没有物理显示器和完整 GNOME/KDE 会话，因此托盘／点穿／真实键盘／缩放／无合成器与浏览器启动仍待桌面验收。没有连接 Fedora。

**macOS**：M-13 的全尺寸内容与页面窗口标题、M-14 的单个菜单图形位置以及 M-16 的最小 11pt 标签、透明像素输入和同屏全屏隐藏已实施。原生 List 导航、页面身份保留、62pt 折叠栏、最小窗口，以及桌宠图形／可见性／缩放／透明蒙版方向检查通过。实际多显示器全屏 Space、真实鼠标点穿和完整辅助功能仍需人工验收。之前已通过的 Control-点击菜单、图表语义与唤醒时间选择器保留。

## 2026-10-28 工程治理与 WebKit 通道

控件的尺寸、操作、焦点和 CSS 所有权见 [三端控件规格](ui-control-spec.md)。`npm --prefix apps/desktop run lint:css` 与 `node scripts/test-css-rules.mjs` 已接入共享检查；生成令牌与阶段色仍使用原来的同步脚本维护。

`npm --prefix apps/desktop run test:webkit` 覆盖 8 页 × 浅深色 × 中英文 × 默认/最小尺寸，共 64 个组合，截图和清单输出到 `.build/ui-webkit/`。本机 Playwright WebKit 26.5 已通过该矩阵，抽查了最小尺寸的中文总览、英文深色设置页。CI 配置同时安装两个浏览器引擎并保存截图；远端 CI 尚未运行。

合成数据与预览后端分别保留当日统计、至少 7 日趋势与 365 日热力图，缺少记录的日期补零；显式日期范围内只补该范围。`test-desktop-preview.cjs` 检查这些边界，避免将当日数组复用成年度数据。

时钟检查使用 en-US/en-GB 两种运行时区域，分别切换中英文界面，验证 12/24 小时规则不随界面语言变化；macOS 本机验证当前系统时钟模板与显示结果一致。后续实施已在 Windows 原生用户小时制覆盖与 Linux 隔离 GNOME 设置中验证 12/24 小时；Fedora 未连接。Linux 构建与虚拟桌面证据见本轮实施记录，不能当成 GNOME/KDE 物理桌面通过。

最终 Windows 开发包再次通过 `test-windows-panel.ps1 -VerifyMaterials`：46 项检查通过，浮动面板背景差异 126/126，主窗口 0/152，DWM 类型仍为面板 3／主窗口 2。

## 2026-10-28 后续实施验收

完整变更、已通过检查和剩余人工项见 [三端 UI 实施记录](ui-implementation-2026-10-28.md)。Windows 新增 `powershell -File scripts/test-windows-preferences.ps1 -Binary <开发包 exe>`，需要普通权限交互会话、Node 和 Playwright；临时小时制及透明度在 finally 恢复。macOS GUI 中运行 `sh scripts/test-native-navigation.sh`，验证真实 List 选择、标题、页面保留与折叠栏。

Linux 先在隔离桌面会话中设置 `AIEYES_DATA_DIR=<绝对临时目录>`，再从相同环境启动 `WebKitWebDriver --port=4445`。运行 `python3 scripts/test-linux-native.py --binary <实际 Tauri 二进制> --data-dir <同一临时目录> --backend wayland --output <证据目录>`；X11 使用 `--backend x11`。脚本仅清理二进制路径与数据目录同时匹配的测试进程。
