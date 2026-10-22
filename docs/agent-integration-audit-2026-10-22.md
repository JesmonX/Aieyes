# Agent 接入核查与修复：2026-10-22

实施依据：Codex 会话 `01a160fa-62b8-7730-bc20-48d1b121ee4a` 的“全量 Agent 接入核查与修复方案”。本轮检查全部 Agent，以 Codex 为基准，并用真实 CodeBuddy、Kimi 子任务检查会话归并。

## 已实施

- OpenCode 同时读取 v1 `message` 和 v2 `session_message`，各表游标独立，以稳定身份去重；补齐压缩摘要用量、内嵌工具和 `idle` 结果。本机 **2.1.1** 的新记录已实测，旧格式保留回归。
- CodeBuddy 解析规范化 `usage`、`rawUsage`、OpenAI/Responses/Anthropic 缓存分类，接入原生 Hooks。Web、Swift 设置页提供监测开关和修复入口，保留其他 Hook 及停用选择。
- Kimi 以 `usage.record.model` 为准，按交互 ID 解除等待；客户端记录的模型别名原样保留。Grok 接入实际工具和权限事件，无调用 ID 的工具使用有界计数。
- 历史游标验证文件身份、内容锚点和修改信息。同路径替换后重读；坏行、超大记录、压缩损坏、Harness 不支持的代际或编码冲突保留诊断及重试能力，其他会话继续导入。
- 状态读取保留完整记录，覆盖 320 KiB 冷启动末行、600 KiB 分段追加及 16 MiB 上限。原生事件时间决定过期，毫秒时间及轮次决定新旧，标题更新不使旧会话重新活跃。
- 有明确关系的子会话归并到父会话。等待状态汇总，子任务失败不使主会话失败；列表、数量、摘要和桌宠共用归并结果，子任务实际用量保留。
- 升级受影响解析器及游标。新测量原子替换旧分类，旧解析器不能覆盖纠正结果；保留原账户、来源和有效冻结价格，模型改变时重新匹配价格，缺失则保持未定价。

真实 CodeBuddy 测试还发现：中间说明文本会标记 `completed`；子日志用独立 UUID，而 Hook 用 `父 UUID:agent ID`；取消可能写入 assistant 或 tool-result 的 `incomplete` 记录，之后仍发送通用 Stop。现在分别处理这些记录，依据已发现的父文件及原生 `subagents/agent-*.jsonl` 位置关联身份，保留明确的中断结果。

## 新的真实模型调用

端点为 `http://10.98.36.99:11444`，请求模型为 `Qwen3.8-27B`。使用全新客户端配置目录和 Aieyes 数据库。代理延迟转发以观察状态，响应正文和用量不作修改；配置验证请求与客户端会话请求分开统计。

下表合计普通回答、工具后继续、续聊；Harness headless 场景为前两项。八类客户端的主请求用量均与原生日志导入一致，重复扫描、核心进程重启及全量重扫均未增加重复记录。

| Agent | 入库主请求/轮次 Token | HTTP 总 Token | 原生日志之外的辅助调用 Token | 状态结果 |
| --- | ---: | ---: | ---: | --- |
| Claude Code | 12,339 | 12,339 | 0 | 普通、工具、续聊完成 |
| CodeBuddy | 30,033 | 30,033 | 0 | working → tool → working → complete |
| Kimi Code | 82,839 | 82,839 | 0 | thinking → tool → thinking → complete |
| Grok | 12,575 | 14,713 | 2,138 | 工具、结束及续聊正常 |
| OpenCode 2.1.1 | 5,386 | 6,510 | 1,124 | 工具、续聊及 v2 结束正常 |
| DeepSeek Harness | 18,238 | 19,111 | 873 | 工具后继续并完成 |
| Pi | 1,919 | 1,919 | 0 | 普通、工具、续聊完成 |
| Oh My Pi | 2,953 | 2,953 | 0 | 工具后继续并完成 |

对于存在差额的 Harness、OpenCode、Grok，辅助调用依据本次受控调用的工具签名区分：主请求始终携带对应工具，标题请求使用 `session_title` 或不携带工具。差额保留在 HTTP 记录中，没有补造客户端未持久化的使用记录；已持久化的 OpenCode 压缩摘要仍正常导入。Grok 的原生计量粒度为轮次，不能把入库行数当作 HTTP 请求数。本轮服务没有返回缓存命中，缓存分类由协议回归覆盖。

OpenCode 首次工具测试将工作目录选为仓库根目录，未读到隔离目录中的文件；原始失败记录保留。显式传入测试目录及文件绝对路径后补跑，工具成功读取、随后回答及续聊全部通过，表中采用补跑结果。

## 权限、子任务、失败与取消

通过真实 ACP 协议将权限问题保留三秒后答复，子任务使用原生 Agent 工具，取消使用 `session/cancel`。

| 场景 | 真实证据及结果 |
| --- | --- |
| Kimi 权限等待 | needsInput → tool → thinking → complete；40,841 主请求 Token，与 HTTP 一致 |
| Kimi 子任务 | `state.json` 记录 `agent-0.parentAgentId=main`；等待汇总到主会话，最多显示一条；67,455 Token，与 HTTP 一致 |
| Kimi 取消 | ACP 返回 `cancelled`，监测显示 interrupted；已完成请求 21,429 Token，与 HTTP 一致 |
| CodeBuddy 权限等待 | 捕获原生 PermissionRequest，显示 needsInput；45,797 主请求 Token，与 HTTP 一致 |
| CodeBuddy 子任务 | 真实子日志及 SubagentStart/Stop；修复后最多显示一条，最后 complete；58,277 Token，与 HTTP 一致 |
| CodeBuddy 取消 | ACP 返回 `cancelled`，working → tool → interrupted，最多显示一条；22,882 主请求 Token 与 HTTP 一致，另有 578 个未持久化的标题 Token；assistant/tool-result 两种中断记录均纳入回归 |
| 实际模型失败 | Pi 续聊时关闭全部工具，服务返回 HTTP 400；监测显示 failed，未补造零用量请求。保持工具定义后的正常续聊通过 |

CodeBuddy 经典 Hooks 没有单独的“权限已获准”事件；批准后工具尚未返回时，等待标记可能保持到工具结果。OMP 在首次模型响应前可能尚未写日志，不能凭日志提前显示 working。

## 其余 Agent 与既有历史

所有导入均使用隔离 Aieyes 数据库；重复扫描及 rescan 不重复计数。

| Agent | 本轮真实历史验证 | 新调用或能力边界 |
| --- | --- | --- |
| Codex | 用户指定会话：26 条用量记录 | 生命周期、父子关系、列表/计数/桌宠、继承及 SSH 回归；本轮没有新增 Codex 模型调用 |
| Antigravity | 15 个会话、522 条用量记录 | 保留原生时间、状态及额度适配；没有用 Chat Completions 端点替代其原生服务 |
| Qoder | 13 个会话、502 条用量记录 | 隔离目录的新 API 配置要求先登录；没有复制日常凭据，未计为新调用通过 |
| ZCode | 3 个会话、4 条用量记录 | 数据库结算、原生 Hook 与 SSH 回归；本轮新模型调用未覆盖 |
| Cursor | 官方 Hook、旧代际停止、隐私投影及标准导出回归 | 私有库请求级 Token 仍明确未支持，上下文占用不当作累计用量 |
| DeepSeek 余额 | 余额与 Harness 会话用量保持独立 | 自建模型成功不代表官方余额或付费额度验证 |

另对 Claude、Kimi、Harness、CodeBuddy、Grok、OpenCode、Pi、OMP 的既有真实历史完成重新导入、重复扫描和全量重扫。账户、订阅和原生配置的边界沿用[此前验收记录](native-agent-verification.md)。

## 回归与复现

- 核心新增 16 项专项回归，位于 `crates/core/tests/agent_integration_audit.rs`，覆盖原计划及真实调用新增发现。
- 最终 `cargo test --workspace --locked`：**495 项通过、0 项失败、4 项忽略**；格式检查、Clippy `-D warnings`、核心构建及共享 CI 全部通过。
- SSH 协议：历史 25 项、状态观察 28 项、Harness 6 项、原生 SQLite 8 项；使用本机临时数据运行，没有宣称新的 SSH 远程实机调用。
- Web：CodeBuddy 安装/停用/恢复监测、保留外部 Hook 的真实核心测试和桌宠测试通过；已检查监测弹窗截图。
- Swift：`scripts/test-native.sh` 全部通过，含真实核心子会话归并、桌宠、独立轮询及失败恢复；最后再次执行 `.build/verify-sessions`。
- 最终结果、原始报告路径及未通过场景的保留证据见 `.local/agent-audit-2026-10-22/verification-summary.json`。核心、Clippy、共享 CI、Swift 会话日志分别为同目录的 `rust-tests-latest.log`、`clippy-latest.log`、`ci-checks-latest.log`、`swift-sessions-latest.log`。

```sh
python3 scripts/verify-agent-integration-live.py \
  --endpoint http://10.98.36.99:11444 --model Qwen3.8-27B
python3 scripts/verify-agent-lifecycle-live.py \
  --endpoint http://10.98.36.99:11444 --model Qwen3.8-27B
python3 scripts/verify-native-agent-history.py PROVIDER=/path/to/native/history
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
sh scripts/ci-checks.sh
sh scripts/test-native.sh
```

实测脚本会创建新测试目录并产生真实模型调用。模型、客户端版本和网络可用性会影响复现；原始报告保留首次失败与修复后的结果，没有覆盖失败证据。
