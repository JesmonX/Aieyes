# Qoder / Qoder CN 接入与用量统计

两者是独立 Agent，复用同一组账户、额度和历史解析器。产品配置集中在 `crates/core/src/qoder_family.rs`：

| Agent | CLI | 原生目录 | macOS 桌面 App / 数据目录 |
| --- | --- | --- | --- |
| `qoder` | `qodercli` | `~/.qoder` / `QODER_CONFIG_DIR` | Qoder / `~/Library/Application Support/com.qoder.app.stable` |
| `qoder-cn` | `qoderclicn` | `~/.qoder-cn` / `QODERCN_CONFIG_DIR` | Qoder CN / `~/Library/Application Support/com.qodercn.app.stable` |

- 两者均复用原生 PAT 或浏览器登录。Aieyes 不提供另一套登录流程，不复制原生登录凭据；CLI 自行读取和处理现有认证。界面分别展示 CLI/桌面客户端，以及 CLI 最近记录的认证方式。
- 身份与额度通过 SDK v2 的 `daemon.initialize`、`daemon.account_info`、`daemon.get_usage_info` 获取；初始化使用 `auth.type=qodercli`，不创建模型会话。SDK 模式不能带旧版 print-only 参数，例如 `--no-session-persistence`。查询有超时、有限重试和至少 600 秒的后台轮询间隔。
- 账户身份哈希包含 Agent 标识。两地同名、相同 email 或相同上游 ID 不合并；原有 `qoder` 数据、global Cookie 与 `cn-legacy`（qoder.com.cn）连接保持原归属，旧中国站不映射到 `qoder-cn`。
- `.qoder-app-status.json` 仅用作桌面登录摘要。它不能证明与 CLI 是同一账户，因此不会自动关联 CLI 账户或以此查询额度。桌面库以只读方式打开并读取 WAL；当前版本从消息状态补充会话生命周期，不将 UI 上下文占用当作 Token 用量。
- 桌面 App 的真实请求同样写入各自原生目录的 `projects/**/*.jsonl` 和会话 `state.json`，由下述共享解析器采集；存在原生 daemon transcript 表时也可导入其中的标准用量记录。事件身份负责去重，界面消息正文不写入 Aieyes 数据库。

- 历史 JSONL 的 Token 可能为零；会话状态恢复的是累计总量。补偿量减去已采集的请求计数，真实计数补齐后自动缩减或删除，重复扫描不重复累计。
- 会话主链首个有效助手模型保存在采集游标中，零计数及缺少 usage 的记录仍提供模型线索。会话补偿量和同一记录流中的辅助调用归到主模型，包含辅助模型消耗；这不是底层模型的精确拆分。
- 解析版本升级会自动重读本地及 SSH 历史，已有状态文件无需变化。缺少主模型线索时保留总量，显示“Qoder（主模型待识别）”。
- dfmodel 显示为 DeepSeek-Flash，qfmodel 显示为 Qwen3.8-Flash；名称依据 Qoder 会话启动元数据。内部模型键保留，用于筛选与计价。Dashboard 的 modelDisplayNames 是可选展示映射，旧客户端仍可读取原模型键。
- 面板和用量分析均不显示 Qoder 原生 Credits 记录及说明；后台计量数据保留。

`QODER_EXPOSE_TOKEN_USAGE=true`、`QODERCN_EXPOSE_TOKEN_USAGE=true` 分别作用于两地客户端，只影响带此环境变量启动的新进程，不会改写旧日志。macOS/Linux 的环境配置同时维护两个变量；Windows 需在系统环境变量中设置。历史累计量仍通过状态恢复。

本机验证（2026-10-31）：Qoder CLI 1.1.79 的 PAT 登录、Qoder CN CLI 1.1.81 的浏览器登录均完成原生身份和额度查询；Qoder App 0.4.8、Qoder CN App 0.5.1 各完成一轮最小会话。两地历史导入均未报告解密缺失。其他版本若未提供 SDK v2 或已改变本地数据库结构，会保留明确的查询/采集错误；Windows/Linux 桌面路径未进行本机实测。
