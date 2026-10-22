# 0.1.54 Linux 包一致性验收

日期：2026-10-22。通过 `ssh fedora` 检查实际安装包，并在同一 Ubuntu 22.04 构建环境生成修复候选。最终候选仍标记为 0.1.54，未签名、未发布。用户随后要求 Linux 改用鸿蒙字体，并明确不重复验证；完整交互验收对应换字体前的修复包，字体版只做构建及包内容检查。

## 原始产物与范围

基线来自 [v0.1.54 的 CI 运行](https://github.com/JesmonX/Aieyes/actions/runs/39705470924)，提交 `ea961a57f992c8a2afd2e64af754666722f14a74`。该运行的 Linux、macOS Intel 和 Apple Silicon 构建成功；Windows 构建失败，发布步骤跳过。Linux artifact ID 为 `12394729625`，本次使用其中的 0.1.54 DEB、RPM 和 AppImage。

| 原始包 | SHA-256 |
| --- | --- |
| DEB | `d396d51f76e1f4b388ac2e664d435d8ffd4dbbbce7e9ef8bdd30002990b493bd` |
| RPM | `c3738ea0820139b81efa9f24a6ec67a55147773de65da8acd80f7c116e6a4920` |
| AppImage | `c2b4bd7fcbf83b2723e3e354f266d99d8ae28e9a954c4fc7307bbffb9ea7ed4e` |

Fedora 使用 GNOME 50.6、Wayland 会话及 XWayland，屏幕为 2560×1600、125% 缩放；自动后端选择落到 X11。RPM 与 AppImage 使用 Fedora 的真实图形会话。DEB 安装在 Ubuntu 22.04 Podman 容器中，使用 Ubuntu 的 GTK/WebKitGTK，并连接同一 GNOME 合成器。Fedora WebKitGTK 为 2.54.1，Ubuntu 为 2.50.4。

测试使用独立应用数据及 XDG 配置、缓存目录。鼠标键盘来自临时 Mutter RemoteDesktop 会话；录屏绑定其 ScreenCast 流并按实际视频尺寸换算坐标。托盘服务丢失测试使用另外的私有 D-Bus 与 Xvfb/Openbox。

## 发现与修复

| 问题 | 证据与修复 |
| --- | --- |
| AppImage 有桌宠和面板，但没有托盘 | 原始包内置 GLib 影响宿主 `gdbus`，在 Fedora 报 `undefined symbol: g_string_free_and_steal`。改用进程内 GIO 查询宿主注册状态，严格解析布尔返回值。 |
| Esc 越过菜单、弹窗直接关闭面板 | 原始 RPM 的菜单用例复现。删除 GTK 和全局 XInput 的重复 Escape 拦截，交由获得焦点的前端按菜单、弹窗、面板的顺序处理。固定只阻止被动关闭，明确关闭仍有效。 |
| 托盘打开面板时偶发未获得键盘焦点 | D-Bus 菜单回调没有 GTK 输入时间戳。使用当前 X11 服务端时间进行窗口激活，详情窗口重开使用相同处理。 |
| 隐藏桌宠后菜单文字被后台刷新覆盖 | 将托盘图标的 Active 状态与桌宠可见状态分开，后台更新不再覆盖「显示桌宠」。 |
| 失效托盘句柄仍被视为可用 | 检查 ksni 句柄关闭状态，按需重建，并清除菜单缓存以重放当前状态。 |
| 「桌宠设置」打开错误页面 | 原生动作使用了 `tab`，界面路由接受 `category`。改为定位「常规 → 桌宠与会话」，通过实际显示方式控件验证。此修复也适用于 Windows 的同一入口。 |
| Linux 字体与 Windows 不一致 | 主界面、悬浮面板改用内置鸿蒙 Regular / Medium / Bold，并显示字体许可。原生桌宠和 GTK 菜单共用同一份内嵌字体，通过私有 Fontconfig 配置注册；不修改字体、不安装系统字体，保留 GTK 控件原有字号。 |

Fedora 会话还存在独立的窗口装饰故障：普通 GTK 窗口同样停留在 Iconic、不可见，`_MUTTER_NEEDS_FRAME=1` 且没有 frame extents。已安装的 `mutter-x11-frames` 当时没有运行。以临时用户服务恢复后，同一个探测窗口变为 Normal、可见，并获得原生标题栏。辅助进程退出原因未确认；应用继续保留 GTK 原生装饰。探测结果保存在 `native-decorations.json`。

## 最终鸿蒙字体候选

在完整验收包的基础上接入鸿蒙字体，保留同一份 0.1.54 核心和功能修复。三个包均在同一 Ubuntu 22.04 环境重新构建；构建输入共 241 个，逐项与来源清单核对。工作区并行进行的核心和网站修改未加入这些候选。

| 最终候选包 | 大小 | SHA-256 |
| --- | ---: | --- |
| deb | 36.07 MiB | `287479583bcb6bd18069814bad1c2e95ca4c1172ed822f3684d414cde144c9a6` |
| rpm | 30.01 MiB | `f88ab17f4dec8420b42f852627a7170406567a2284e81893cc382288253d4546` |
| AppImage | 109.26 MiB | `a2611041a25965ae3a2af9a93a3e344c4a05f898910184a376d7683853ea85c3` |

构建前已校验三份原始字体的大小、SHA-256 和许可；打包后确认三包的桌面 ELF 均含有字体、许可及来源清单的资源索引。三包的归一化程序代码和压缩资源一致，启动项和图标一致；桌面程序要求 glibc 2.35，核心要求 2.34。核心二进制与换字体前的验收包完全相同。

Fedora 已通过 RPM 替换安装，Ubuntu 容器已通过 DEB 替换安装；两处已安装程序的哈希均与最终归档相符。AppImage 完成构建与包内容检查。按用户要求，字体改动后未重新运行桌面交互、动画、浏览器或 Rust 测试；下文的完整测试结果仅对应 `pre-harmony/` 中的验收包。

最终包位于 `.local/linux-consistency-0.1.54/candidate/`；同目录的 `package-checks.json`、`installed-payloads.json`、`build.log`、`source-manifest.json`、`implementation.patch`、`provenance.json` 和 `SHA256SUMS` 记录构建及校验依据。源清单摘要为 `e25839f440e5367cf377f96c5d7487be92ac9936260ab02b913cc38a6fb045cd`。字体原文件及其来源见 [字体清单](../apps/desktop/web/fonts/sources.json)，原生接入见 [linux_fonts.rs](../apps/desktop/src-tauri/src/linux_fonts.rs)。

## 字体改动前的验收包

完整桌面测试的构建基线为工作区 `5e16515` 加本次三个 Rust 文件的功能修复。该基线相对原始 v0.1.54 只有浏览器布局测试的舍入容差变化。构建使用 Ubuntu 22.04、Rust 1.99.0、Tauri CLI 2.12.1，一次生成三种包。AppImage 上游运行时下载曾失败，随后固定使用从官方地址下载的本地运行时，SHA-256 为 `617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46`。

| 候选包 | 大小 | SHA-256 |
| --- | ---: | --- |
| DEB | 21.49 MiB | `13c7404102c2b78ff9f1e76a07622bf83941ad0b74b359d044c4eb54fabd7407` |
| RPM | 15.44 MiB | `e0905d8f2062b2c4c2bf5f2dbd2bc6c22fef477c83d4926dacb1406dea1f8006` |
| AppImage | 94.68 MiB | `690988cb3292d13cf7a2654af98b982fbe310fc081f237dbca1fdeede7706053` |

当时的包检查已验证三个验收包中的桌面与核心程序：x86-64、已剥离符号、glibc 要求分别不高于 2.35 和 2.34。归一化 Tauri 包类型标记后，三个包的程序代码与常量资源相同；桌面启动项、StartupWMClass、图标均相同。测试时已安装 RPM、DEB 的二进制哈希与这些验收包相符。上述包和其完整测试证据均已归档到 `pre-harmony/`。

## 字体改动前的实机结果

每次完整测试包括 100 次混合交互：通过桌宠鼠标或托盘 D-Bus 菜单打开面板，通过桌宠、外部点击或 Esc 关闭。另检查首次主窗口、重开详情、菜单和弹窗层级、固定面板、后台刷新后的隐藏状态、隐藏桌宠时打开面板、拖动时共存和跟随、实际设置控件切到托盘、恢复桌宠、两次正常退出重启，以及主题和语言同步。

| 包 | 交互与完整功能 | 动画观察 | 动画样本 | 结果 |
| --- | --- | ---: | ---: | --- |
| RPM | 100 次及全部功能通过 | 600.051 秒 | 2,041 | 通过 |
| AppImage | 100 次及全部功能通过 | 600.257 秒 | 2,054 | 通过 |
| DEB | 100 次及全部功能通过 | 600.107 秒 | 2,182 | 通过 |

动画检查覆盖四角透明、画面非空、动画持续变化及合成器录屏黑帧。录制使用真实时间戳，视频长度因启动和尾帧会略短于观察时间，JSON 同时记录观察时长、样本数、不同帧数及实际视频时长。RPM、AppImage 各观察到 6 种不同动画帧，DEB 为 4 种；三包均未发现透明角闪黑或黑帧区间。

RPM 与 AppImage 均通过私有 D-Bus 的无宿主回退、宿主恢复、托盘模式恢复、隐藏后宿主丢失及 watcher 重启测试。RPM 另以 KDE 环境标识验证后端选择；此项不是 KDE 实机验证。

## Windows / macOS 对照与兼容性（字体改动前）

- Windows / Linux 共享浏览器外观与交互矩阵通过，32 项桌面 JavaScript 测试通过。
- Linux 桌面 Rust 测试 30 项通过，Clippy `-D warnings` 与格式检查通过。
- 使用同一 CI 的 Intel macOS 0.1.54 应用完成六种状态、浅深色、面板、详情、设置、最小尺寸、减少透明度及高对比度的原生截图与校验。
- 原始 Linux 0.1.54、Linux 修复候选及 Intel macOS 0.1.54 的 `hello`、IPC 方法清单一致；原始包与候选双向读取同一配置，macOS 核心也能原样保存、读取该配置。协议版本为 1，配置版本为 11。
- 窗口标题栏及材质保留平台特点。信息布局、入口、状态同步及面板操作按共享规则检查。最终字体版的应用文字与 Windows 共用鸿蒙；合成器绘制的标题栏、系统托盘菜单仍遵循桌面环境字体。

该 CI 没有产出可用的 Windows 安装包，本次未进行 Windows 原生桌面验收。本轮为单屏 125% 缩放，多屏和其它缩放比例未做实机覆盖。未把 Xvfb 的 KDE 环境标识测试当作 KDE 实机测试；显式原生 Wayland 的定位及置顶仍受合成器限制，完整交互验收采用 XWayland。托盘交互通过实际 D-Bus 菜单回调触发；未补做 `--shell-tray` 顶栏坐标点击或 AppImage `--appimage-extract-and-run` 额外场景，已完成的 AppImage 实机验收使用默认 FUSE 启动。

## 证据与复现

本机输出根目录为 `.local/linux-consistency-0.1.54/`，Fedora 为 `~/.cache/aieyes-consistency-0.1.54/`。`baseline/` 保留原始 CI 产物；`pre-harmony/` 保留完整桌面验收对应的包、JSON、截图和录屏；`candidate/` 包含最终字体版候选、`SHA256SUMS`、来源信息、构建输入清单及实现补丁。各测试目录包含 JSON、应用日志、原生截图和录屏。测试使用 [原生验收脚本](../scripts/test-linux-desktop.py)，用法见 [Linux 桌面说明](windows-linux.md#包体积与原生窗口验收)。

首次安装测试 RPM 时，DNF 清除了 Fedora 原有待执行的离线系统更新事务。该事务需要重新安排；本次没有代为执行系统更新。窗口装饰辅助服务 `aieyes-audit-x11-frames.service` 为会话内临时服务，保留运行以维持原生标题栏。任务使用的 macOS DMG 已卸载；构建用 Ubuntu 容器在收尾时停止。
