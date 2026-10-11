# 手动验证与维护入口

从仓库根目录运行。Python 检查使用 Python 3.11+；浏览器检查需要 `npm ci --prefix apps/desktop` 和 Playwright Chromium。核心集成检查使用先构建的 `target/debug/aieyes-core`。原生检查需要 macOS、Swift 和已解析的 Sparkle 依赖。

自动化入口：`sh scripts/ci-checks.sh [--rust]`、`npm run test:ui --prefix apps/desktop`、`sh scripts/test-native.sh`。`test-agent-settings-ui.cjs` 已在 Web 测试链及两份 CI 中运行；`verify-models.swift` 已在原生测试链中使用真实核心输出运行。

以下工具保留为按需验证或维护入口；性能基准、真实联网下载及截图矩阵不作为每次 CI 的默认步骤。

| 工具 | 用途与入口 |
|---|---|
| [compare-ui.py](../scripts/compare-ui.py) | 对照已生成的两端截图，`python3 scripts/compare-ui.py`；先查看脚本定义的截图目录 |
| [generate-icons.py](../scripts/generate-icons.py) | 重新生成品牌图标，`python3 scripts/generate-icons.py`；会更新图标资产 |
| [test-native-performance.sh](../scripts/test-native-performance.sh) | `sh scripts/test-native-performance.sh`，链接真实 Swift debug 对象并测量视图性能 |
| [test-server-performance.sh](../scripts/test-server-performance.sh) | `sh scripts/test-server-performance.sh`；`--deadlines-only` 单独验证调度暂停、恢复和退出 |
| [test-streaming-core.py](../scripts/test-streaming-core.py) | `python3 scripts/test-streaming-core.py`，真实核心与临时 SSH 流式夹具；测试中需要本机进程及监听权限 |
| [test-incremental-history.py](../scripts/test-incremental-history.py) | `python3 scripts/test-incremental-history.py`，采集器增量游标、截断与兼容路径 |
| [test-update-transport.py](../scripts/test-update-transport.py) | `python3 scripts/test-update-transport.py`，真实核心子进程和本机录制代理，不访问公网 |
| [benchmark-performance.py](../scripts/benchmark-performance.py) | `python3 scripts/benchmark-performance.py --soak-seconds 3600`，默认使用 release 核心及临时合成库；可用 `--binary` 和 `--output` 指定路径 |
| [benchmark-remote-history.py](../scripts/benchmark-remote-history.py) | `python3 scripts/benchmark-remote-history.py`，全量与增量采集性能对照 |
| [verify-update-transfer.swift](../scripts/verify-update-transfer.swift) | 可选真实联网读取、下载和签名校验；需要包含 Sparkle 更新源、公钥的临时 plist，见 [传输说明](macos-performance-and-update-transport.md)；不安装更新 |
| [test-ui-native-render.sh](../scripts/test-ui-native-render.sh) | `sh scripts/test-ui-native-render.sh`，合成场景原生截图矩阵；可用 `AIEYES_RENDER_BINARY`、`AIEYES_RENDER_ROOT` 指定构建与输出 |

Tauri 宿主测试先运行 `node scripts/prepare-wakeup.cjs --debug`，再运行 `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --features custom-protocol`。打包资源只匹配真实核心二进制，CI 也按此顺序准备资源。

测试输出放在 `.local/`；历史快照与过期安装包放在 `.local/archive/YYYY-MM-DD/`。这些文件不代表当前源码或可发布产物。

跨语言契约维护：编辑 [core-contracts.json](core-contracts.json)，运行 `python3 scripts/sync-core-contracts.py`（需要 rustfmt）；`--check` 检查生成文件，已纳入 CI。可用 `target/debug/aieyes-core --call methods.manifest` 查看核心通道、失效事件和刷新键。
