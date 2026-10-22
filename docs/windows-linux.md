# Windows / Linux 桌面版

Windows 和 Linux 默认显示桌宠，可通过文件、资源链接、npx 命令或 npm 素材包导入，在带 idle 预览的库中切换，也可明确切换为系统托盘。两者使用已有的 Tauri 详情与设置界面，以及同一个 Rust 核心。macOS 继续使用 SwiftUI / AppKit 原生应用。格式与恢复规则见[桌宠说明](pets.md)。

## 日常操作

- 桌宠显示已启用的本机及 SSH Codex、Claude、AGY 会话状态和活跃数量。单击会展开与 macOS 菜单栏一致的面板：顶部居中切换 Agent / 服务器，内含账户限额、今日概览、近 7 天用量、会话状态与刷新菜单；底部操作依次为主题、更新、详情、设置、更多。拖动移动，松手后距工作区边缘 48 逻辑像素内自动贴边；右键或 Shift+F10 打开菜单。菜单提供「设置」「桌宠设置」、切换显示方式、重置位置、暂时隐藏桌宠和退出。
- 应对全屏场景可暂时隐藏桌宠：菜单或「设置 → 常规 → 桌面显示」中的「暂时隐藏桌宠」。隐藏期间托盘入口保留，可从托盘菜单或设置页恢复；隐藏偏好在重启后保留；没有可用托盘时恢复桌宠入口。
- 桌宠位置和显示方式保存在数据目录的 `desktop.json`，重启后恢复；恢复时将位置限制在当前显示器工作区内，避免更换显示器后落在屏幕外。
- Linux 托盘用图标颜色和文字显示状态，点击展开原生菜单。部分桌面只显示图标，完整文字仍在菜单内；可在「设置 → 常规 → 桌面显示」切换到桌宠。
- 显示方式为「桌面宠物」或「系统状态栏 / 托盘」，默认桌面宠物。检测不到可用托盘时，即使选择了托盘也会使用桌宠。
- 固定面板同步主窗口的配置与采样，慢请求或失败时保留旧读数并标记数据延迟／连接失败。Windows 主窗口最低 640×440，打开或切换 DPI 时会适配当前工作区。
- 关闭详情窗口后继续运行，使用菜单或设置页的「退出 Aieyes」退出。会话监测在 Rust 后台线程运行，不依赖隐藏窗口的 JavaScript 计时器，也不会被 SSH 或限额查询阻塞。

「设置 → Agent 接入 → 对应 Agent」中的账户归档操作保留账户历史、价格快照、数据源关联与导入进度，只停止账户限额查询；数据源仍同步用量。展开「已归档账户」可恢复。概览的账户筛选保留归档账户，并标注「已归档」。旧客户端试图直接删除有历史的账户时，核心会拒绝保存，以保护历史记录。

「设置 → Agent 接入」选择并启用 Agent，在「机器与目录」选择本机或 SSH 机器，使用机器开关旁的目录设置管理日志来源。在同页账户区域添加或登录账户，再连接设备并配置限额；登录失败或取消不会创建空账户。已启用且关联来源的账户会在启动后自动刷新限额，失败后按独立间隔重试。保存失败保留输入并支持重试；概览刷新保留已展开的每日明细与键盘焦点。


## 窗口与界面

Windows 使用自绘标题栏，支持拖动、双击最大化和右上角窗口按钮。默认客户区为 1294×840：1080×800 内容区外加 214px 侧栏和 40px 标题栏，侧栏收起为 72px。展开的悬浮面板为 450×720。Windows 11 build 22621 起请求 Desktop Acrylic，Windows 10 1809 起及较早的 Windows 11 使用原生 Blur；不支持、关闭透明效果或高对比度时回退不透明。主窗口与悬浮面板独立报告材质，Windows 窗口圆角为 8px。Linux 主窗口保留原生装饰；桌宠使用 GTK/Cairo 原生透明窗口，450×720 面板独立显示，14px 圆角与实色内容底色。打开面板不隐藏桌宠，面板外点击和 Esc 可关闭；固定时仅停止外部点击自动关闭。实现、尺寸和验证限制见 [Windows 外观对齐](windows-appearance.md)。

设置包含 Agent 接入、服务器、价格、定时唤醒、常规五个标签；账户、API 和机器目录集中在对应 Agent 下，应用代理位于「常规 → 网络代理」；协议下拉自动拼接 Host 和端口，也可输入完整代理 URL。价格支持模型 ID 与名称搜索，更新固定查询项目 Release。概览以四张卡片显示总 Token、成本、缓存命中率与输入／输出／缓存明细，计价不足仅显示感叹号；移除数据来源板块。服务器以圆环和状态条显示资源。

Windows 和 Linux 正文、表单、图表和原生桌宠使用随应用内置的 HarmonyOS Sans SC（鸿蒙黑体），无需安装或联网；「设置 → 常规 → 外观」可查看字体许可。主要数值为 24px，资源条统一为 4px 实色，模型名旁的复制按钮已移除。Linux 的 GTK 菜单也使用鸿蒙字体，并保留系统字号；合成器绘制的标题栏和托盘菜单遵循桌面环境。支持浅深色、减少动态效果、减少透明度和高对比度。过渡与入场动画只作用于 transform 和 opacity，遵循系统减少动态效果设置。鼠标悬停或键盘聚焦带说明的控件约半秒后显示统一提示气泡，气泡固定在视口内、不会撑宽页面，按下鼠标、滚动或 Esc 立即关闭。服务器编辑使用分组下拉多选；搜索后的全选、清空、反选仅作用于匹配结果。高级设置仍支持手填设备表达式，旧配置直接兼容。

编辑弹窗内显示验证与保存错误。打开弹窗后，后台页面不可操作，Windows 标题栏仍可拖动或使用最小化、最大化和关闭按钮。Tab 在弹窗控件与标题栏按钮之间循环；Esc 优先关闭多选面板，再关闭编辑弹窗。保存期间禁用重复提交与取消。

## Linux 桌面适配

每约 15 秒检查会话 D-Bus 的 `org.kde.StatusNotifierWatcher.IsStatusNotifierHostRegistered`。检测通过时创建 StatusNotifierItem 托盘；托盘服务直接通过 D-Bus 提供，不依赖当前 GTK/GDK 后端，因此在 XWayland 桌宠模式下也不会随窗口后端失效。服务未注册、查询失败或图标创建失败时回退为桌宠；无需安装 AppIndicator 动态库。托盘服务恢复后，明确选择托盘的用户恢复托盘显示。

Linux 在桌宠模式也保留可用的托盘图标。可用性检测通过进程内 GIO 查询会话 D-Bus，不启动系统 `gdbus`，避免 AppImage 内置 GLib 与宿主工具的版本冲突。托盘服务句柄失效时重新创建并恢复菜单；图标保持 Active，桌宠的隐藏状态单独控制菜单文字，后台刷新不会把「显示桌宠」改回「隐藏桌宠」。GNOME 通常需要桌面提供 AppIndicator/StatusNotifier 扩展才能显示托盘；KDE 等桌面是否显示图标旁文字取决于具体实现。托盘使用标准 DBusMenu，所有必需操作（打开面板、打开详情、设置、检查更新、隐藏/恢复桌宠、退出）均通过原生菜单提供。

Esc 由当前获得焦点的面板处理，优先关闭菜单或弹窗，再关闭面板；固定面板也可明确关闭。在其它窗口按 Esc 不影响固定面板。从托盘打开面板或详情时使用当前 X11 服务端时间激活窗口，避免 D-Bus 菜单缺少输入时间戳而无法获得焦点。

Wayland 会话中，未设置 `GDK_BACKEND` 且 XWayland 可连接时自动使用 X11 后端，以支持桌宠定位、置顶和完整的外部点击关闭。显式设置 `GDK_BACKEND=wayland` 时保留原生 Wayland；绝对定位与置顶受合成器限制，仍可用托盘、Esc 和面板关闭按钮操作。应用不会修改系统会话或桌面扩展。

## 会话范围

读取已启用的本机及 SSH Codex、Claude Code、AGY 数据源。支持输入/权限等待、最终失败、思考、工具执行、完成、中断及待确认。各来源独立报告健康状态；本机约每秒更新，SSH 每台机器独立约每 5 秒更新。具体接入、导入格式和状态边界见[桌面宠物与会话状态](desktop-pets.md)。

## Windows 构建

需要稳定版 Rust（MSVC 工具链）、Visual Studio 2022 C++ 桌面开发工具和 Windows SDK、WebView2 Runtime。安装包在缺少 WebView2 时使用微软在线引导程序安装，因此首次安装可能需要联网。SSH 监控需要系统 OpenSSH Client，远端需要 Python 3。

在项目根目录的 PowerShell 执行：

```powershell
# 开发版可执行文件
./scripts/build-desktop.ps1 debug
./apps/desktop/src-tauri/target/debug/aieyes-desktop.exe

# 当前用户安装包，可选择简体中文 / English
npm install --global @tauri-apps/cli@2
./scripts/build-desktop.ps1 release
```

安装包输出：`apps/desktop/src-tauri/target/release/bundle/nsis/*.exe`。

后台查询不弹出控制台窗口。Codex / agy 支持从 PATH 解析 `.exe` 和 npm 的 `.cmd` / `.bat` 启动器，并补查 `%APPDATA%\npm`；也可在数据源中填写完整程序路径。本机路径支持 `~/`、`~\`、空格和 Unicode；远程路径继续按远端 POSIX shell 处理。

## Linux 构建

以下依赖命令面向 Ubuntu 22.04 / Debian 系列：

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev librsvg2-dev \
  patchelf libglib2.0-bin openssh-client rpm libarchive-tools

# 开发版
sh scripts/build-desktop.sh debug
./apps/desktop/src-tauri/target/debug/aieyes-desktop

# DEB、Fedora RPM 和 AppImage
npm install --global @tauri-apps/cli@2
sh scripts/build-desktop.sh release
```

输出目录：`apps/desktop/src-tauri/target/release/bundle/`。DEB/RPM 声明了 WebKitGTK、GTK、GLib 工具、polkit 和 SSH 客户端运行依赖。Fedora 推荐 RPM，以使用系统维护的图形运行库；AppImage 提供便携运行。AppImage 所在系统仍需提供桌面服务；无托盘服务时使用桌宠。缺少 FUSE 的系统可尝试 `./Aieyes*.AppImage --appimage-extract-and-run`。

CLI 也可使用 `cargo install tauri-cli --version '^2' --locked` 安装；构建脚本会优先选择已安装的 `tauri` 命令，其次使用 `cargo tauri`。npm 预编译版本安装更快，Node 仅用于构建与测试，运行应用无需 Node。

## Fedora 安装与迁移

```sh
sudo dnf install ./Aieyes-版本-linux-x64.rpm
/usr/bin/aieyes-desktop
```

RPM 依赖 `webkit2gtk4.1`、`gtk3`、`glib2`、`openssh-clients` 和 `polkit`，推荐 `xorg-x11-server-Xwayland`。GNOME 的顶部托盘仍需要 AppIndicator 扩展；扩展缺失时应用保留桌宠入口。

从解包后的 AppImage 迁移时，先退出旧进程，再安装 RPM。默认数据目录相同，账户、历史和桌宠库沿用原数据。检查 `~/.local/bin/aieyes` 与 `~/.local/share/applications/aieyes.desktop`：若仍指向旧的 `AppRun`，停用该旧启动器，使用 RPM 安装的启动项或 `/usr/bin/aieyes-desktop`；应用不会自动覆盖自定义启动器。解包目录本身不会被识别为已安装 RPM。

## 包体积与原生窗口验收

Linux 构建脚本使用 `--no-default-features`，仅使用 ksni 托盘；Windows/macOS 的平台配置显式启用 `native-tray` 特性。Linux 构建在 Cargo `OUT_DIR` 暂存前端资源，包含与 Windows 相同的三份原始 HarmonyOS 字体、来源清单及许可。WebView 和 GTK/Pango 原生文字共用这份内嵌资源；原生端仅在应用生命周期内写入私有临时目录，通过独立 Fontconfig 配置注册，不安装系统字体。独立桌面 Cargo 工程启用符号剥离、Thin LTO 与单个 codegen unit，保留 panic unwind。`scripts/check-linux-package-size.py` 分别解开 DEB、RPM 和 AppImage，检查各包的两个 ELF 均为已剥离符号的 x86-64 程序、glibc 要求不高于 2.35，核对三份字体及许可的内嵌资源索引，并比较程序代码、压缩资源、桌面启动项和图标。构建前还会按来源清单校验原始字体的大小及 SHA-256。Tauri 的包类型标记及 AppImage 的 ELF 路径修补允许不同，其余程序内容必须一致。正式 Linux 分发仍在 Ubuntu 22.04 构建；在 Fedora 上生成的测试包不作为通用 glibc 兼容基线。

```sh
python3 scripts/check-linux-package-size.py
# Fedora 已安装的 RPM：使用独立数据和临时测试背景，不修改现有应用设置
python3 scripts/test-linux-desktop.py \
  --binary /usr/bin/aieyes-desktop --core /usr/lib/Aieyes/binaries/aieyes-core \
  --package /path/to/Aieyes-0.1.54-linux-x64.rpm \
  --cycles 100 --duration 600 --scenario all --report .local/linux-native/rpm
```

窗口验收脚本需要 Python GObject、Pillow、GNOME Shell、Xwayland、GStreamer 的 PipeWire 插件、libXtst 和 ffmpeg。真实 GNOME 会话通过临时 Mutter RemoteDesktop/ScreenCast 会话输入鼠标键盘并录屏；按协商得到的视频像素尺寸换算分数缩放，不依赖 XTest 向 XWayland 投递输入。测试期间该桌面应避免同时进行其它鼠标键盘操作。

默认执行 100 次桌宠／托盘交替打开，以及桌宠／外部点击／Esc 交替关闭；另验收主窗口首次显示与重开、菜单和弹窗的 Esc 顺序、固定面板、隐藏和恢复、面板跟随拖动、设置页显示方式切换、退出重启后的偏好与位置、浅深色和中英文同步。600 秒观察同时检查透明角、非空且持续变化的动画，以及合成器录屏中的黑帧。录屏使用真实时间戳，截图、日志、包校验值和 JSON 报告位于指定的报告目录。`--duration 0` 明确跳过动画观察，不报告动画通过。

AppImage 应把 `--binary` 指向实际文件，可增加 `--launch-arg=--appimage-extract-and-run` 验证无 FUSE 启动；脚本会跟踪启动器的真实子进程。DEB 在相应发行版安装后使用其 `/usr/bin/aieyes-desktop` 运行。

增加 `--virtual` 使用隔离 Xvfb/Openbox（需要安装两者）。通过 `dbus-run-session -- python3 scripts/test-linux-desktop.py … --virtual --tray-cycle` 可模拟无托盘宿主、宿主出现、丢失和 watcher 重启，并验证回退及恢复；不会接管用户的会话总线。`--desktop KDE` 只验证非 GNOME 环境标识下的后端选择，不代替 KDE 实机验收。`--backend x11 --language en --theme dark` 可补测显式后端和另一套外观。真实 GNOME 上可增加 `--shell-tray`，识别顶部栏中可见的品牌图标、实际点击并截图原生菜单；无法唯一识别时会报错。

## 数据与验证

Tauri 默认数据目录由系统应用数据路径和 `app.aieyes.desktop` 标识组成：Windows 为 `%APPDATA%\app.aieyes.desktop`，Linux 通常为 `~/.local/share/app.aieyes.desktop`（遵循 `XDG_DATA_HOME`）。`AIEYES_DATA_DIR` 可以覆盖此目录。独立核心 CLI 的默认数据路径与 Tauri 不同，如需共享请显式传入同一目录。

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
node --test scripts/test-desktop.mjs
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --no-default-features --features custom-protocol
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --locked --no-default-features --features custom-protocol -- -D warnings
# Windows/macOS 直接 Cargo 构建时增加 --features native-tray
```

`.github/workflows/desktop.yml` 在 main 更新或 PR 时构建 Windows / Ubuntu 安装包，生成 CI artifacts。默认执行 JavaScript 语法、交互、版本与发布校验，并在 Linux 运行浏览器集成测试及截图；Rust 直接进行一次 release 构建。使用 Rust 依赖缓存、npm 预编译 Tauri CLI、取消同分支过期任务及免压缩上传来加快构建。需要完整 Rust 测试时，可手动运行工作流并勾选 `full_checks`。

2026-10-03 [首次原生构建](https://github.com/JesmonX/Aieyes/actions/runs/37125369069) 已通过：Windows 生成 NSIS 安装包，Ubuntu 22.04 生成 deb 和 AppImage，两个平台的 JavaScript 语法与 4 项交互测试均通过。产物在运行页面的 Artifacts，保留 14 天。

0.1.54 的 CI 产物来源、修复候选、Fedora 与 Ubuntu 原生验收及 macOS 对照见 [Linux 包一致性报告](linux-consistency-0.1.54.md)。报告分别记录实际安装包、浏览器矩阵和模拟桌面环境的覆盖范围；Windows 原生、多屏和显式 Wayland 的限制单独列出。


## 本地浏览器验证

需要 Node.js 24。测试使用内存中的 IPC 数据，不访问个人账户或服务器：

```sh
npm ci --prefix apps/desktop
cd apps/desktop
npx playwright install chromium
npm run test:ui
```

截图输出至项目 `.local/ui-previews/`，包括浅深色概览、多选面板及最小尺寸窗口。完整发布流程见 [发布说明](releases.md)。
