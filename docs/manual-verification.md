# 手动验证与维护入口

从仓库根目录运行。共享 Python 检查支持 Python 3.9+，旧版本复用仓库内 Tomli 解析 TOML；发布打包仍按对应构建脚本的依赖要求执行。浏览器检查需要 `npm ci --prefix apps/desktop` 和 Playwright Chromium。核心集成检查使用先构建的 `target/debug/aieyes-core`。原生检查需要 macOS、Swift 和已解析的 Sparkle 依赖。

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

Tauri 宿主测试先运行 `node scripts/prepare-wakeup.cjs --debug`，再运行 `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --features custom-protocol`。打包资源只匹配真实核心二进制，CI 也按此顺序准备资源。

测试输出放在 `.local/`；历史快照与过期安装包放在 `.local/archive/YYYY-MM-DD/`。这些文件不代表当前源码或可发布产物。

`verify-pets` 根据 `MTLCreateSystemDefaultDevice()` 检查图形能力。没有 Metal 设备的 CI 仅跳过需要 GPU 的菜单 symbol effect，仍运行桌宠关键帧、窗口生命周期、缩放与截图检查，并在日志明确标注。真实图形验收运行 `.build/verify-pets --require-metal-device`，缺少设备会失败；可用 `.build/verify-pets --without-metal-device` 验证无 GPU 分支。Intel 与 Apple Silicon 均纳入原生 CI。

跨语言契约维护：编辑 [core-contracts.json](core-contracts.json)，运行 `python3 scripts/sync-core-contracts.py`（需要 rustfmt）；`--check` 检查生成文件，已纳入 CI。可用 `target/debug/aieyes-core --call methods.manifest` 查看核心通道、失效事件和刷新键。
