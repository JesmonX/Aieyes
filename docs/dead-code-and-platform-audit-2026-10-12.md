# 死代码、测试与跨平台一致性优化（2026-10-12）

基线：`7f57c9a`，应用版本 `0.1.32`。范围：Rust 核心、SwiftUI/AppKit、Tauri/Web、测试入口和 CI。先核对生产调用、RPC/生成契约、平台条件编译及测试调用，再决定删除；历史数据兼容、系统委托回调、Windows 专用模块和截图入口不以文本引用数判死。

## 方案与证据

| 优先级 | 发现 | 实施方案 | 验证方式 |
|---|---|---|---|
| P2 | `settings::is_configuration_method` 无调用；Antigravity `read_database` 和不带类型的 `estimate` / `finish_estimate` / `repair_estimate` 只被测试调用，生产已使用详细结果和带类型接口 | 删除冗余入口，现有回归改走生产接口；删除只验证废弃接口的断言 | 核心全量测试、Clippy、同 ID 的额度/credit 隔离回归 |
| P2 | `AgentsSettingsView.editSource` 从不调用，导致外层 `sourceEditor` 弹窗不可达；`confirmDiscardConfig` 无置真入口；`QuotaWindow.supportsEstimate` 无使用方 | 删除不可达状态、回调、弹窗和重复计算属性，保留目录页面实际使用的 `SourceEditor` | Swift 完整构建、原生回归 |
| P2 | Tauri 更新成功分支在不返回的 `app.restart()` 后继续构造返回值，产生不可达代码和未使用变量；原 CI 只编译，未阻止警告 | 删除重启后的返回值，Desktop CI 同时执行严格 Clippy 和宿主测试 | Tauri 严格静态检查、宿主回归；不触发真实更新安装 |
| P1 | Tauri `local_path` 优先 `HOME` 且不识别 `~\\`；核心在 Windows 使用 `USERPROFILE` 并支持反斜杠。同一配置的“检查/选择路径”和实际采集可指向不同目录 | Tauri 直接使用核心 `store::expand`，删除独立展开逻辑 | 路径回归、Tauri 宿主测试；Windows 条件用例交由原生 CI |
| P2 | 面板账户偏好对未知 mode、缺失字段和无效结构的回退不同：Web 按自动选择或报错，Swift 可变成空选择 | 在两端规范化偏好，未知值回退自动；保留明确的空选择、顺序、去重与每 Agent 五账户上限 | 扩展现有跨语言共享夹具，运行 Node 与原生测试 |
| P2 | 流式传输测试用 400ms 包含外部 Python 启动；基线出现首条数据前超时，无法验证其声称的流/批处理行为 | 将短时序测试改为自包含 Rust 子进程夹具，保留接收超时、慢消费者、错误和清理断言；可移植部分在 Windows 也运行 | 独立重现、核心全量回归 |
| P2 | 共享检查默认选系统 Python，但 `release.py` 无条件使用 Python 3.11 才有的 `tomllib` 和 `hashlib.file_digest`；本机 Python 3.9.6 失败 | 复用已有 vendored Tomli，校验值按 1 MiB 分块计算；既有测试逐个检查真实文件摘要 | Python 3.9 与已安装的较新 Python 检查 |
| P2 | 性能 UI 测试通过截取并 `eval` 另一测试的源码获取夹具，修改缩进或声明即破坏测试；重置格式纯 VM 测试混在浏览器链中 | 将夹具提为显式模块；纯 VM 回归放共享检查；复用两端相同的重置样例，去掉重复、与生产无关的断言 | 共享检查、完整浏览器回归、原生格式回归 |
| P2 | 部分离线 Python 回归和真实核心主题同步未接默认检查；Linux Tauri 测试默认不执行 | 将有效回归接入对应入口；按真实平台能力标明 POSIX 测试边界；Windows/Linux 默认执行宿主测试 | 本机完整检查、工作流静态核验 |

## 实施与验收

上述清理已在独立工作区实施，且通过本机验证。相关旧接口已无引用；测试现在使用生产的详细数据库结果、带类型估值接口和实际路径检查命令。截图入口、操作系统委托回调、旧数据迁移及缓存参考算法保留。

面板偏好新增九组共享恢复场景，覆盖缺字段、未知 mode/版本、损坏 JSON、无效结构及部分有效配置。基线实测：`{"version":2,"providers":{"codex":{"mode":"custom"}}}` 在 Web 将账户全部隐藏；修复后恢复默认顺序。明确保存的 `keys: []` 继续表示不显示账户。macOS 新增不查询额度的账户时，也不再将其写入面板自定义选择。

流式回归由 Rust 子进程产生数据，跨平台可运行；持续流长于 1 秒接收期限、每批处理 1.1 秒、停滞后的超时和特定错误原因分别验证。子进程挂起 30 秒，清理须在 15 秒内返回，以区分正常清理和等待自然结束；短于平台停止进程开销的旧时限已移除。父测试实际执行子进程夹具，夹具自身的 `ignore` 并非跳过功能验证。

| 检查 | 本机结果 |
|---|---|
| `cargo test --workspace --locked --offline` | 185 通过；3 个显式忽略入口为两个由父测试调用的子进程夹具和一个手动性能基准 |
| Core / Tauri `cargo clippy --all-targets … -- -D warnings` | 均通过；Tauri 含 `custom-protocol` |
| `sh scripts/ci-checks.sh` | Python 3.9：57 项；Node：29 项；重置格式、JS 语法、三类生成文件一致性均通过 |
| Python 3.14 `scripts/test-release.py` | 7 项通过，包含逐个核对文件摘要 |
| `npm run test:ui --prefix apps/desktop` | 18 组浏览器回归通过，含真实核心、多窗口主题同步 |
| `swift build --package-path apps/macos --scratch-path .build/swift --disable-sandbox` | 通过 |
| `sh scripts/test-native.sh` | 整链通过，含共享偏好、重置格式、真实核心模型和原生交互模型 |
| Tauri `cargo test --locked --offline --features custom-protocol` | 23 项通过，含路径检查隔离子进程 |

日志保存到 `.local/audit-cc65Sr/phase1/logs/`。首次复制的 Swift 模块缓存包含旧路径，清除隔离工作区自己的模块缓存后重新构建通过；这是工作区迁移问题。生产安装、真实账户切换和远端服务器未用于本次测试。

本节是当前清理的独立验收。其后对其他 Codex 产物的取舍、逻辑合并和最终三平台 CI，见[整合审查与下一步方案](consolidated-audit-plan-2026-10-12.md)。

## 工作区说明

初始工作区只有用户未跟踪文件 `codex-audit-retry.py`。检测到多个独立 Codex 进程共用项目且基线日志混写，因此在独立 Git worktree 中实施和验证，合入时逐项检查当前工作区，保留其他任务及用户改动。混写的首次核心日志只用作线索，不作为最终验收证据。
