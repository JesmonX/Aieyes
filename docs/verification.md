# 验证记录 · 2026-10-03

## Windows Release 回归修复 · v0.1.1

- `v0.1.0` 的 Release 在 Windows 核心测试阶段失败：临时目录的短路径与规范化长路径直接比较不相等；其他三个构建任务成功，发布任务跳过，未公开 Release。[失败运行](https://github.com/JesmonX/Aieyes/actions/runs/37131417067)。同期 Windows/Linux 桌面打包工作流成功。
- 凭据目录断言改为规范化两侧路径，继续检查凭据未逃逸目录、文件内容与 Unix 权限；Linux 采集器测试在 Windows 上显式创建 `os.statvfs` 模拟，保留全部采集断言。
- 本机 Rust 查询来源回归 4 项、Python 发布校验 3 项、采集器 4 项通过；删除进程内 `os.statvfs` 后重跑采集器 4 项也通过，覆盖 Windows 缺少该 API 的环境。原生 Windows 验证由新版本 Release 工作流执行。
- 所有应用版本和 Cargo 锁文件同步为 `0.1.1`，macOS 构建号递增为 2；保留 `v0.1.0` 原标签。

## Release 与界面改进

- 新增四任务 Release 矩阵，交付 macOS Intel / Apple Silicon DMG、Windows NSIS、Linux DEB / AppImage；只有全部检查通过且附件完整才公开 Release。actionlint 检查两个工作流通过。
- Node 12 项测试、Python 发布校验 3 项、采集器 4 项通过；覆盖选项隔离、搜索批量操作、清空持久化、窗口按钮、公开版本防覆盖、上传失败保留草稿及重跑。
- Rust 核心 30 项测试通过。本机 `/etc/profile.d/clash.sh` 含不兼容 dash 的语法，因此 shell 回归在临时 mount namespace 中屏蔽该宿主脚本后通过；没有修改主机配置或测试代码。
- Chromium 集成回归通过，覆盖 256 核心、100 挂载点、暂不可用设备、禁用类别保留选项、取消编辑、读取失败、Esc 焦点、弹出层边界、高对比度及 150%/200% 缩放。浅深色截图位于 `.local/ui-previews/`。
- Windows GNU 目标的 Tauri 编译检查通过，包括 Mica、平台窗口配置和主窗口权限；使用临时目录中的交叉工具链，只做编译检查，不代表 Windows 安装或实机运行通过。
- 当前宿主为 Ubuntu 20.04，缺少 WebKitGTK 4.1，GLib 版本不足；无法在本机完成 Linux 桌面构建。宿主无 Swift，macOS 编译、原生选择模型测试、DMG/签名检查已接入 macOS CI，尚未在本轮运行。新工作流尚未实际发布 Release；三平台安装卸载与 Windows 原生材质需实机验收。

## Windows / Linux 桌面支持

- Windows 默认悬浮球；Linux 检测托盘宿主后优先使用动态状态栏，检测失败或服务退出时回退悬浮球，服务恢复后可自动恢复托盘。通用设置支持手动切换。
- 悬浮球支持拖动、位置保存、启动时屏幕工作区校正、点击详情、右键菜单、键盘菜单及退出。会话采样由 Rust 独立线程运行，每约 5 秒更新，关闭详情后继续。
- 新增 Rust `sessions.list` 与有界日志尾部读取；状态、过期、来源范围、去重、部分写入及不可读提示已覆盖测试。Windows 后台命令不创建控制台，CLI 解析优先使用原生程序／npm Windows 启动器，避免误选同名 POSIX 脚本。
- macOS 宿主上的 Rust 核心 30 项、Tauri 桌面 3 项、Node 前端交互 4 项、Python 采集器 4 项测试通过；核心和桌面 Clippy 均以 `-D warnings` 通过。平台打包配置已由 Tauri 配置类型解析验证；JavaScript、JSON、shell 语法与文档链接检查通过。
- 新增 Windows PowerShell 和 Linux shell 构建脚本、NSIS / deb / AppImage 配置、Windows / Ubuntu CI 打包工作流（默认轻量检查，完整 Rust 测试手动启用）。[首次 GitHub Actions 原生构建](https://github.com/JesmonX/Aieyes/actions/runs/37125369069) 已通过，代码提交 `bf1e2a2`，产出 Windows NSIS、Linux deb 和 AppImage；两个平台均完成轻量前端检查，Rust 完整测试本次未开启。Windows 专用 `.cmd` 执行测试及实际系统托盘、透明窗口、多屏 DPI、Wayland、安装卸载仍待目标平台验证。
- 此前阻止 GitHub 写入的审批服务故障已恢复，源码和工作流已上传到 Public 仓库 `JesmonX/Aieyes`，项目根目录 Git 已与远端同步。浏览器视觉预览仍未执行；上述前端验证使用模拟 Tauri API 的事件测试，不替代真实桌面验收。操作及依赖见 [Windows / Linux 桌面版](windows-linux.md)。

## 原生卡片密度与提示优化

- 限额与服务器卡片支持独立折叠，按账户／主机 ID 保存状态。收起后保留限额剩余量或余额、服务器 CPU／内存百分比与进度条。展开提供 CPU、内存、GPU、显存与文件系统进度条。
- 今日概览取消记录数、已计价百分比及补价横幅；仅有用量且计价未完成时显示感叹号，悬浮说明需要设置价格，点击打开价格设置。
- 近 7 天图表、图例、明细共用日期范围；图例空间不足时使用自适应网格。主滚动区与模型分布图例采用自动隐藏的原生 overlay 滚动条。
- macOS 开发包构建及签名校验通过，产物为 `dist/Aieyes.app`。
- 本轮截图入口在 `_RegisterApplication` / `NSApplication` 初始化阶段以 SIGABRT 退出，尚未执行视图代码。当前会话无法初始化 AppKit 图形上下文，新增交互及浅深色布局仍需桌面验收；此前截图不作为本轮验证。

## 已通过

- Rust 核心 11 项单元测试、11 项回归测试。新增覆盖旧账户迁移、同账户多个来源共用限额、独立账户与无账户来源、今日与 7 天模型统计、部分补价、代理前置命令继承／覆盖／失败终止。
- Python 远程历史、Linux 采集器、Claude 远程限额字段过滤与错误处理，共 3 项测试。
- Rust Clippy，所有目标启用 `-D warnings`。
- SwiftUI/AppKit macOS 应用构建与开发包 ad-hoc 签名校验。
- Swift 原生模型对现有数据库副本响应的解码，及今日总量、7 天模型聚合、待计价 Token 与 v2 账户关联一致性验证。
- Tauri 外壳宿主平台编译检查，含 `custom-protocol` 静态资源嵌入。
- JavaScript 语法、Python 语法、Info.plist 和文档相对链接检查。
- JSON-RPC 在错误输入之后继续处理下一条合法请求。
- 本机 Codex/CC 目录实际扫描；未变化的第二次扫描新增 0 条、读取 0 字节，总量保持一致。
- Codex 5h/7d 日志快照展示数据保留；空快照不覆盖最近一次完整结果。

- 原生浅色／深色菜单栏面板与详情页、账户／数据源／价格设置、远程来源编辑页截图检查。预览输出在 `.local/review-v2/previews/`。渲染入口改用 NSHostingView，兼容系统控件，且不执行来源扫描或限额请求。
- macOS 开发包已重新生成并通过 `codesign --verify --deep --strict`。

## 尚未完成的运行验证

- 本轮已完成原生离屏布局截图；尚未完成菜单栏实际点击、编辑保存等完整桌面交互验收。离屏多窗口截图时 AppKit 输出过 NSTableView reentrant 警告，未阻止构建或渲染。
- 先前 SSH 实机验证受运行环境限制。本轮远程前置命令与 Claude 查询采用隔离测试，代理和服务商联网链路仍需实机验收。
- OpenRouter 请求未成功；Codex App Server 查询进程提前退出。价格与实时限额的联网行为待在正常桌面环境验证。
- 当前未发现可供读取的 Antigravity 实例，原生自动采集尚未实现。
- Windows/Linux 完成 Tauri 宿主编译与 JavaScript 语法检查，未运行目标平台安装包；Apple Silicon 未构建原生二进制。

当前 Mac 开发包为 Intel 构建。测试目录与本机数据位于 `.local/`，不包含在源码或应用资源中；调试应用通过构建时路径使用该目录。


## 2026-10-03：agy、余额和服务器显示增强

- 26 项 Rust 测试通过（新增模型组额度、余额精度及币种、余额失败缓存、查询来源跳过历史导入和凭据权限验证）；Clippy 严格检查通过。
- 4 项 Python 采集器测试通过，包含挂载点转义、可用空间、inode 与首次双采样；JavaScript 语法及单账户余额 / 文件系统细分渲染检查通过。
- 本机 agy 真实只读查询返回 4 个窗口，约 4.5 秒；CLI 报告模型调用为 0。
- 已有 3 台 SSH 主机均采样成功，并行首次约 0.94 秒；单台复用连接约 0.71 秒。真实验证挂载点发现、仅保留选定挂载点、显式隐藏网卡和 CPU 速率。
- macOS 开发包构建通过；Tauri `custom-protocol` 宿主编译通过。原生浅深色面板、单账户展开卡片、agy / DeepSeek 编辑器和服务器设置截图位于 `.local/feature-previews/`。
- DeepSeek 官方响应格式和保存流程已用固定样例验证，本机未配置真实 API Key，因此尚未完成真实余额联网查询。浏览器连接不可用，网页端未完成真实点击验收；原生设置也仍需完整点击验收。
- 上述 SSH 成功结果替代前轮“运行环境拒绝连接”的限制；Antigravity 原生历史自动采集、目标 Windows/Linux 安装包验证不在本轮范围。
