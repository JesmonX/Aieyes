# 原生 Agent 验收记录

核实日期：2026-10-19。验收对象为现有非 Codex 客户端及新增 Kimi Code、DeepSeek Harness、Pi、Oh My Pi、CodeBuddy。以下区分真实客户端调用、协议回归和缺少服务凭据的项目。

2026-10-22 的核查、修复和新实测见[后续验收报告](agent-integration-audit-2026-10-22.md)。下文的版本、调用数字及开发包信息保留为此前验收证据。

## 安装与真实调用

本机为 macOS Intel，远端为已配置的 Linux SSH 测试主机。模型服务为用户提供的自建兼容端点，实际模型 ID 为 `Qwen3.8-27B`，无需 Key；原生 SDK 要求非空 Key 时使用 `not-required`。测试没有注入模型回复或 Token。用于观察状态的代理只延迟转发。

| 客户端 | 本机已核实版本 | 真实客户端验收 |
| --- | --- | --- |
| Claude Code | 2.1.301 | 自建服务的 Anthropic Messages 调用、原生历史与状态 |
| Antigravity | 1.4.2 | 现有登录的真实模型调用、历史和四个额度窗口；独立 Gemini API 仅协议端点测试 |
| Qoder | 1.1.73 | 自建模型、Aieyes 保存的默认配置、Read 工具；Linux 真实调用与 SSH 计量 |
| ZCode | App 3.15.3 / CLI 0.16.9 | 本机自建模型；Linux 工具后继续调用、原生 Hooks 与最终结算 |
| Grok | 1.0.62 | 默认自定义模型、读取文件工具；Linux 原生调用与 SSH 计量 |
| Cursor | 2026.10.16-e3cb207 | 官方安装、启动与帮助；按用户要求排除登录和依赖账号的调用 |
| OpenCode | 1.19.3 | Aieyes 原生 API 配置、bash 读取文件、实时状态；Linux 1.2.15 调用与 SSH 计量 |
| Kimi Code | 2.2.0 | 自建模型、Read 工具、Aieyes 原生配置；Linux 调用与 SSH 计量 |
| DeepSeek Harness | 0.2.0-rc.2 | 自建模型、bash 读取文件、Aieyes 原生配置；Linux 调用、v0/v4 历史与 SSH 计量 |
| CodeBuddy | 2.168.2 | 默认自定义模型、Read 工具、low 推理参数；Linux 当前版及旧 2.108.0 的真实调用 |
| Pi | `@earendil-works/pi-coding-agent` 1.3.0 | 本机和 Linux 均使用 Aieyes 保存的配置、low、bash 文件读取，核对 HTTP / 原生 / 导入用量 |
| Oh My Pi | `can1357/oh-my-pi` 18.12.2 | 官方预编译程序经安装器 SHA-256 校验；本机和 Linux 完成与 Pi 相同的实际验收 |

本机入口位于 `~/.local/bin`。Pi、CodeBuddy、Kimi、ZCode 的启动器使用已核实的 Node 绝对路径。Pi 采用当前官方包，未使用已弃用的 `@mariozechner/pi-coding-agent`。Linux 新版本安装在专用临时目录，保留系统旧版本。

十二个客户端均在各自空 HOME、仅含 `~/.local/bin:/usr/bin:/bin` 的精简 PATH 下通过版本启动检查，不依赖交互式 shell 的 Node 配置。报告：`final-minimal-path-clients-report.json`。

所有实验使用独立 Aieyes 数据库。Pi/OMP 和最终验证使用隔离客户端配置；没有替换 `/Applications/Aieyes.app`。较早的 Qoder 原生控制接口测试临时更改默认模型后已按原始字节恢复；其报告记录恢复结果。ZCode 的旧本机测试会话保留在原生统计目录。原有登录凭据未切换。

## 数据与 API 接入

| Agent | 原生根目录 / 配置 | 已实现的处理 |
| --- | --- | --- |
| Kimi | `~/.kimi-code`，`KIMI_CODE_HOME`；`config.toml` | 逐请求 `usage.record`、实际模型、独立 agent 身份；跳过 fork 继承前缀。兼容旧 Python wire 统计 |
| Harness | `~/.dsh`，`DSH_HOME`；profile 的 `cordis.patch.yml` | v0/v4 JSONL 与连续 Zstd 帧；继承边界、代际选择、迁移去重、校验损坏重试；保存配置保留 `!!js` 与持久化 root |
| CodeBuddy | `~/.codebuddy`，`CODEBUDDY_CONFIG_DIR`；`settings.json`、`models.json` | 消息、工具及独立用量记录；并行工具、子会话镜像去重；旧父消息链关联；保留已有模型与允许列表 |
| Pi | `~/.pi/agent`，`PI_CODING_AGENT_DIR`；`models.json`、`settings.json` | assistant、独立 usage 和摘要用量；分叉去重；v1 升级后的身份迁移保留原账户和冻结价格 |
| OMP | `~/.omp/agent`；`models.yml/yaml`、`config.yml/yaml` | 与 Pi 共用原生条目解析；额外处理 profile、配置/数据目录分离和可变的 256 字节标题槽 |

OMP 路径解析按官方规则处理 `OMP_PROFILE`、旧 `PI_PROFILE`、`PI_CONFIG_DIR`、`PI_CODING_AGENT_DIR` 与已存在的 XDG 数据目录。新增/发现目录使用原生环境变量；已保存的明确路径不会静默改绑。SSH 使用远端明确路径或默认目录，不套用本机环境。

OpenCode 同时只读 v1 `message` 和 v2 `session_message` 统计，各表游标独立，以稳定身份去重；新表不会遮住旧表独立记录。ZCode 只读 `model_usage` 结算，实时状态来自原生 process Hooks，完成以 `turn_usage` 的持久结算为准。`ZCODE_DATA_BASE_DIR` 不移动 CLI 历史，不能用它推断统计路径。

Qoder 读取消息 Token、独立请求 Credits、折扣前值与 `billable`，不重复计算 Result 的累计值。Claude、Antigravity 保留现有原生适配。Cursor 状态接入官方 Hooks，私有库的请求级 Token 明确标为未支持；上下文占用不冒充累计用量，标准 JSONL 导出仍可读。

Web 和 Swift 原生界面提供十类本机 API 保存入口：Claude、Qoder、ZCode、Grok、OpenCode、Kimi、Harness、CodeBuddy、Pi、OMP。Claude 支持 API Key / Bearer 两种认证；Pi/OMP 支持上下文、最大输出和推理强度，OMP 已验证实际发送 `reasoning_effort=low`。这些保存入口面向本机；Linux 实测将 Aieyes 生成的隔离配置逐字节传至远端，未宣称存在直接编辑 SSH 配置的表单。

API 验证成功才保存。两文件配置分别检查版本、备份，写入失败回滚；保持其他 Provider 和设置，Key 不回显且不写入 Aieyes 设置。原生配置使用受限文件权限。Cursor 提供官方服务认证指引；Antigravity 提供 Gemini 原生配置指引，未把普通 Chat Completions 地址当作这两种客户端的服务端。

## 最后一次真实用量核对

| 场景 | 请求数 | HTTP 与导入总 Token | 结果 |
| --- | ---: | ---: | --- |
| Pi 本机 | 2 | 1,107 | working → tool → working → complete |
| OMP 本机 | 2 | 2,125 | tool → working → complete |
| Pi Linux SSH | 2 | 1,105 | working → tool → working → complete |
| OMP Linux SSH | 2 | 2,141 | tool → working → complete |
| ZCode Linux 继续一轮 | 3 | 44,738 | Read 后继续调用；Stop 不提前结束；最终 complete |

以上场景重复扫描和显式 rescan 均新增 0。Pi/OMP 还逐项对照原生日志计数；未传模型或端点覆盖参数，使用 Aieyes 保存的默认配置。OMP 第一次模型响应前可能尚未持久化日志，当前日志观察器不保证显示这个尚无记录的阶段。

较早的 Linux 真实调用还覆盖 Qoder 2,981、Kimi 20,391、Harness 5,675、Grok 3,009 Token，以及 CodeBuddy 当前版 6,368 / 旧版 6,673 Token。OpenCode 1.2.15 为 2,381 Token。均核对原生主请求、正式 SSH 导入和重复扫描。Grok 自动标题等未落入原生日志的辅助调用单列，不虚构本地未保存的请求。

最新核心另重新运行 OpenCode 1.19.3 与 Harness 的实际 bash 文件读取，观察工具和完成状态。真实 SSH 读取也覆盖现有 Claude / Antigravity 历史，以及 Harness 的 14 个 v0 压缩会话、806 条请求；未修复或改写原始客户端数据库。

主要报告均位于 `.local/agent-integration/`：`pi-omp-managed/report.json`、`linux-pi-omp-managed/report.json`、`linux-zcode-continuation/report.json`、`opencode-live-report.json`、`harness-live-report.json`、`linux-qoder-managed/report.json`、`linux-extra-managed/report.json`、`linux-codebuddy-managed/report.json`。

## 额度验收边界

- Qoder 现有真实登录的额度和账户身份查询成功：套餐剩余 0 Credits，正确显示无可用容量；会话 Credits 不代替账户余额。报告：`qoder-native-live-report.json`。
- Antigravity 现有登录的真实限额查询成功，返回四个窗口。Claude 未找到可用于本次订阅查询的登录信息。报告：`existing-quota-report.json`。
- Kimi 已接入中国 / 国际官方 usages 及 `boosterWallet`。钱包定点单位、普通分、十进制金额、零/负/缺失余额、月度上限、错误缓存、Web/Swift 展示均通过协议回归；尚缺真实付费凭据进行钱包联网验收。
- Harness 已接入 DeepSeek 官方 API 余额：总余额、赠送/充值余额、币种和可调用状态。它属于模型后端，不能代表自建模型或 Harness 的所有服务；尚缺对应官方 Key 和身份信息进行真实联网验收。
- CodeBuddy 已接入请求 Credits；该部分包含协议样本验证。当前官方公开资料中未找到可核实的服务账户剩余额度入口，不能把请求 Credits、本地 Gateway 密码状态或自建模型 Token 充当余额。
- Pi/OMP 没有统一客户端余额；额度属于实际模型服务商。其他订阅账户、Antigravity 独立 Gemini Key 的联网验收仍缺相应有效凭据。Cursor 登录及依赖账号的调用按用户要求排除。

这些缺口没有被标记为真实付费账户验收通过。自建服务成功与付费额度成功分别记录。

## 性能与损坏恢复

本机及 SSH 的 OpenCode/ZCode 历史读取使用 rowid 游标，额外检查最多 256 条未完成/待重试记录；队列保存在 Aieyes 的索引表中，传给远端的队列始终有界。写入停止后仍会完成剩余队列扫描，不漏掉较老请求的迟到结算。显式 rescan 用于重放已结算历史的人工修改。

SQLite 状态冷启动读取元数据，之后只读取新增行和最多 256 个可见会话的最新行；指纹不变时不再次查询。Harness 历史和状态从上次已校验的压缩 EOF 继续，固定前缀及尾部锚点验证文件身份，保留 v0/v4 继承状态。解压和校验在写锁外完成，统计投影使用 512 KiB 内存暂存、超出后落临时文件；提交批次有界。截断、损坏、并发写入不推进失败游标，状态保留上次有效结果。

真实 SSH 性能验收使用合成元数据，专门测量读取规模；不是实际付费消费证据：

| 已完成历史 | OpenCode 新增 + 迟到完成 | ZCode 新增 + 迟到完成 | Harness 追加一条请求 | 无变化刷新 |
| ---: | --- | --- | --- | --- |
| 10 条 | 2 行，106 ms | 2 行，93 ms | 新帧 176 字节，280 ms | 0 行 / 0 字节，约 100–114 ms |
| 10,000 条 | 2 行，110 ms | 2 行，108 ms | 新帧 181 字节，303 ms | 0 行 / 0 字节，约 101–123 ms |

报告：`native-ssh-performance/report.json`。本机 10/3,000 条历史的追加、状态和压缩读取回归也通过；700 条 pending 的测试验证每轮最多 256 行以及停写后的完整检查。新增热路径不重复扫描已完成请求；首次导入、目录发现/核对和用户触发的全量 rescan 仍与文件或记录数相关，不能把整个采集过程称为绝对 O(1)。

## 完整回归与开发包

最终源代码检查：

- `cargo test --workspace --locked`：411 passed，4 ignored（已有手动性能/环境测试）。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：通过。
- `scripts/ci-checks.sh`：通过，使用 Python 3.12 / Node 24；含 18 项 SSH 历史、17 项状态观察、5 项 Harness 压缩投影、6 项 SQLite 增量回归及契约检查。
- `npm run test:ui`：全部通过；原生 API 套件覆盖十类客户端、Claude 两种认证，共 11 个场景。
- `scripts/test-native.sh`：全部通过；含真实核心会话、独立查询通道、原生配置/金额解码、SwiftUI 和宠物窗口检查。
- `git diff --check`：通过。

日志分别为 `acceptance-workspace-final.log`、`acceptance-clippy-final.log`、`acceptance-shared-ci-final.log`、`acceptance-full-ui-final.log`、`acceptance-native-swift-final.log`。浏览器中的 Windows/Linux 场景不替代对应系统的图形客户端实机验证。

接入验收时的 macOS Intel 开发包已生成到 `dist/Aieyes.app`，版本 `0.1.48`。`codesign --verify --deep --strict` 通过，包内核心与该次编译核心的 Mach-O UUID 一致，`hello.contractHash` 与当前 `docs/core-contracts.json` 的 SHA-256 一致。没有替换 `/Applications` 中的日常应用。正式发布版本递增为 `0.1.49`、macOS 构建号 `50`；同步核心与桌面端锁文件，由 GitHub CI 验证最终提交并构建发布安装包。

使用包内核心和全新隔离数据库重放 Claude、Kimi、Harness、CodeBuddy、Grok、OpenCode、Pi、OMP 八类真实测试历史：首次导入均有有效用量，重复扫描及 rescan 均新增 0；Pi/OMP 分别为 1,107 / 2,125 Token，与对应 HTTP 计数一致。实际打包应用生成 136 张原生截图，已人工查看 Pi/OMP 浅色、深色设置页，文字和布局正常。截图覆盖设置入口，本轮没有进行原生 API sheet 的完整鼠标/键盘保存交互。

包内核心 SHA-256：`eaa3b5f146d16a879e9f3696c3949017d2ee846f1926d9a4e5534f4e2caa0dbb`。包验证报告、全部哈希及截图位置见 `final-native-bundle/report.json`；日志为 `acceptance-native-bundle-final.log`，最终汇总为 `final-acceptance-report.json`，均位于 `.local/agent-integration/`。

资料：[Pi 会话格式](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/session.md)、[Pi 模型配置](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/models-config.md)、[OMP 会话格式](https://github.com/can1357/oh-my-pi/blob/main/docs/session.md)、[OMP 路径规则](https://github.com/can1357/oh-my-pi/blob/main/packages/utils/src/dirs.ts)、[Harness 持久化格式](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/session/session-persistence-jsonl/README.md)、[Qoder 额度协议](https://docs.qoder.com/cli/sdk/cost-usage)、[Kimi 钱包协议](https://github.com/MoonshotAI/kimi-code/blob/main/packages/oauth/src/managed-usage.ts)、[DeepSeek 余额协议](https://api-docs.deepseek.com/api/get-user-balance/)。此前逐步排查记录保留在 `.local/agent-integration/native-agent-verification-before-final.md`。
