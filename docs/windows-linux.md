# Windows / Linux 桌面版

Windows 默认使用置顶悬浮球；Linux 优先使用桌面状态栏／托盘。两者使用已有的 Tauri 详情与设置界面，以及同一个 Rust 核心。macOS 继续使用 SwiftUI / AppKit 原生应用。

## 日常操作

- 悬浮球显示本机 Codex 会话状态和活跃数量。拖动移动，单击打开概览，右键或 Shift+F10 打开菜单。菜单可打开设置、切换显示方式、重置位置或退出。
- 悬浮球位置和显示方式保存在数据目录的 `desktop.json`，重启后恢复；恢复时将位置限制在当前显示器工作区内，避免更换显示器后落在屏幕外。
- Linux 托盘用图标颜色和文字显示状态，点击展开原生菜单。部分桌面只显示图标，完整文字仍在菜单内；可在「设置 → 通用 → 桌面显示」切换到悬浮球。
- 默认显示方式为「跟随系统」。也可明确选择「悬浮球」或「系统状态栏 / 托盘」。检测不到可用托盘时，即使选择了托盘也会使用悬浮球。
- 关闭详情窗口后继续运行，使用菜单或设置页的「退出 Aieyes」退出。会话监测在 Rust 后台线程运行，不依赖隐藏窗口的 JavaScript 计时器，也不会被 SSH 或限额查询阻塞。

## Linux 桌面适配

每约 15 秒检查 AppIndicator 动态库和会话 D-Bus 的 `org.kde.StatusNotifierWatcher.IsStatusNotifierHostRegistered`。检测通过时创建原生托盘；服务未注册、库缺失、查询失败或图标创建失败时回退为悬浮球。托盘服务恢复后，「跟随系统」模式自动恢复托盘。

检测工具为 GLib 提供的 `gdbus`；没有此工具时采用悬浮球。GNOME 通常需要桌面提供 AppIndicator 扩展才能显示托盘；KDE 等桌面是否显示图标旁文字取决于具体实现。Tauri 的 Linux 托盘不发送鼠标点击事件，因此所有必需操作均通过原生菜单提供。

Wayland 下窗口位置、置顶和拖动由合成器决定，可能无法达到 X11 的效果；定位请求失败不会阻止悬浮球显示。不同桌面组合仍需实测。

## 会话范围

读取已启用的本机 Codex 数据源，不采集其他软件或远程会话的运行状态。显示进行中、思考中、执行工具、已完成、已中断、状态待确认；遵循系统减少动态效果设置。

每约 5 秒检查已知日志，每约 30 秒发现新日志；每个文件最多读取末尾 256 KiB，只返回状态、来源名称、脱敏标识和更新时间。3 分钟无更新的活跃会话标记为待确认，完成／中断保留 90 秒，待确认最多保留 1 小时。日志状态不代表进程存活。部分日志不可读时显示提示，不能将其视为已确认空闲。

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
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  patchelf libglib2.0-bin openssh-client

# 开发版
sh scripts/build-desktop.sh debug
./apps/desktop/src-tauri/target/debug/aieyes-desktop

# deb 和 AppImage
npm install --global @tauri-apps/cli@2
sh scripts/build-desktop.sh release
```

输出目录：`apps/desktop/src-tauri/target/release/bundle/`。deb 声明了 WebKitGTK、GTK、AppIndicator、GLib 工具和 SSH 客户端运行依赖。AppImage 所在系统仍需提供桌面服务；无托盘服务时使用悬浮球。缺少 FUSE 的系统可尝试 `./Aieyes*.AppImage --appimage-extract-and-run`。

CLI 也可使用 `cargo install tauri-cli --version '^2' --locked` 安装；构建脚本会优先选择已安装的 `tauri` 命令，其次使用 `cargo tauri`。npm 预编译版本安装更快，Node 仅用于构建与测试，运行应用无需 Node。

## 数据与验证

Tauri 默认数据目录由系统应用数据路径和 `app.aieyes.desktop` 标识组成：Windows 为 `%APPDATA%\app.aieyes.desktop`，Linux 通常为 `~/.local/share/app.aieyes.desktop`（遵循 `XDG_DATA_HOME`）。`AIEYES_DATA_DIR` 可以覆盖此目录。独立核心 CLI 的默认数据路径与 Tauri 不同，如需共享请显式传入同一目录。

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
node --test scripts/test-desktop.mjs
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --features custom-protocol
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --locked --features custom-protocol -- -D warnings
```

`.github/workflows/desktop.yml` 在 main 更新或 PR 时构建 Windows / Ubuntu 安装包，生成 CI artifacts。默认只执行 JavaScript 语法和轻量交互测试，Rust 直接进行一次 release 构建；省去重复的 fmt、Clippy 和测试编译。使用 Rust 依赖缓存、npm 预编译 Tauri CLI、取消同分支过期任务及免压缩上传来加快构建。需要完整 Rust 测试时，可手动运行工作流并勾选 `full_checks`。

已在 macOS 宿主验证共享核心、桌面逻辑、前端事件和配置解析；目标系统验证结果见 GitHub Actions。Windows 专用 `.cmd` 执行测试仅在 Windows 启用完整测试时运行。

目标平台验收：Windows 拖动与点击／右键、重启后位置、不同 DPI／多屏、隐藏窗口后的会话更新、npm CLI 查询；Linux 有／无托盘、托盘服务退出及恢复、X11／Wayland、退出应用；最后检查安装与卸载流程。浏览器视觉预览本轮因自动审批服务 404 未执行。
