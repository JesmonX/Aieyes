# Headless 与 Aieyes 内部会话排除规则

依据：`docs/agent-auth-quota-research-2026-10-29.md` (c) 与 (h)。实现：`crates/core/src/session_events.rs` 的 `headless_argv` 与 `classify`、`session_processes.rs` 的原生进程关联，以及 `sessions.rs` 的 `non_interactive`。SSH 观察器 `scripts/session_remote.py` 使用相同规则，由共享用例 `scripts/fixtures/headless-argv.json` 保证 argv 两边一致。

## 分类

每个会话带一个 `classification`：

| 值 | 含义 | 是否计入实时会话 |
|---|---|---|
| `internal` | Aieyes 自己启动的进程（`AIEYES_INTERNAL=1`，或 kv `internal-session:{provider}:{id\|cwd}`） | 否，对所有 provider |
| `headless` | 日志字段或进程 argv 判定为非交互 | 否 |
| `interactive` | 明确的交互会话 | 是 |
| `unknown` | 没有任何信号 | **是**（与 Codex 的未知即排除不同） |

判定顺序：
1. `internal` 最先，且粘性：一旦标为 internal 不会被后续事件改回。
2. 日志里的显式字段：Claude `entrypoint`、Codex `source`（规则不变，见两份 session-sources 文档）。
3. Hook 或已关联原生进程的 `classification`：Cursor `is_background_agent=true` → `headless`；Hook 读取最近的同 provider CLI 祖先。原生进程只分类它能证明属于自己的会话。交互进程重新续接同一会话时可以恢复计入。
4. 都没有则为 `unknown`，计入。

## argv 规则表

程序识别：argv 前 4 个 token 中，basename（去掉 `.exe/.cmd/.js/.mjs/.cjs`）等于下列程序名，或路径包含 `node_modules/<包名>/`。支持当前 DSH、Kimi、CodeBuddy、Pi、OMP、OpenCode 的 npm 包入口。观察器自身以及含 `--aieyes-session-hook` 的 shell 不参与判定。Cursor、ZCode、CodeBuddy 的 Hook 不绑定任意 Node 祖先；原生日志仍可按确切会话证据绑定 PID。

选项解析会跳过模型、目录、prompt 等参数的值，并遵守 `--`。例如模型名 `acp`、提示词中的 `--print`、普通位置文本 `stream-json` 都不会凭空变成 headless 信号。

原生关联优先使用 PID 与启动时间、完整日志路径的打开句柄、明确的 session 参数；DSH 使用对应会话目录内的 `session.lock` 句柄。否则必须同时匹配真实工作目录、数据目录、启动时间，且双方只有一个候选。不会因为同 provider 有一个 headless 进程就排除该 provider 的全部会话。OpenCode 的 `serve --service` 与 Kilo 的共享 `serve` 不参与这种归因。

Pi/Kimi 设置 `process.title` 后会覆盖系统 argv。监控已观察到的启动分类、配置目录和 session 选择按 PID 与启动时间保留，进程退出后最多再保留 15 秒，以关联 OMP 等退出时才落盘的日志；会话获得的分类继续随游标保留。缓存只存规范化证据，不保留 prompt、原始 argv 或环境。新 PID/启动时间不会继承旧进程的分类。

| Provider | 程序 | headless | interactive | 无判定 |
|---|---|---|---|---|
| claude | `claude` | `-p/--print` | 其余 | — |
| codebuddy | `codebuddy`、`@tencent-ai/codebuddy-code` | `-p/--print`、`--acp` | 其余 | — |
| codex | `codex` | `exec`、`e`、`mcp-server` | — | 其余 |
| deepseek-harness | `dsh` | `headless`、`--profile headless\|sdk\|acp` | `tui`、`web`、`desktop` | 其余 |
| kimi | `kimi` | `-p/--prompt`、`acp`、`web`、`server`、`rc`、`remote` | 其余 | — |
| cursor | `cursor-agent`、`agent` | `-p/--print` | 其余 | — |
| opencode | `opencode` | `run`、`acp`、`api`、不带 `--service` 的 `serve` | 无子命令（TUI） | `serve --service`、其他子命令 |
| antigravity | `agy` | `-p/--print/--prompt`、`--output-format`、`--input-format`、`--print-timeout`、`--json-schema`、`remote-control`、`mic-serve` 子命令 | `-i/--prompt-interactive` | 其余 |
| qoder | `qodercli` | `-p/--print`、`-o/--output-format`、`--remote-control` | 其余 | — |
| zcode | `zcode` | `-p/--prompt`、`--target`、`app-server` | — | 其余 |
| grok | `grok` | `-p/--single`、`--output-format`、`--json-schema` | — | 其余 |
| pi / omp | `pi`、`omp` | `-p/--print`、`--mode json\|rpc` | — | 其余 |
| copilot | `copilot`、`@github/copilot` | `-p/--prompt`、`--acp` | — | 其余 |
| kiro | `kiro-cli` | `chat --no-interactive/--non-interactive` | — | 其余 |
| cline | `cline` | 位置 prompt 无 `-i`、`schedule`、`--json`、`-z/--zen`、`--oneshot`、`-y/--yolo` | `-i/--tui` | 无参数 |
| kilo | `kilo`、`kilocode` | `run`、`acp`、`--auto`、`--format json` | — | `serve`（按其中会话计）、其余 |
| trae / trae-cn | `trae-cli` | `run`、`--docker-*`、`--must-patch` | `interactive` | 其余 |

已用 `--help` 核实（2026-10-29，本机安装，未登录）：kimi、cursor-agent、opencode、agy、qodercli、zcode、dsh、grok、pi、omp、copilot（npm `@github/copilot`）、cline（npm）、kilo（npm `@kilocode/cli`）。后续 Kiro 与 Trae 的核实见各自补充；依赖真实账户的模型调用仍需单独验收。

与 (c) 表的差异：
- OpenCode `serve --service` 是 TUI 的后台服务，其中承载的是 TUI 会话，因此不做判定（交给 TUI 客户端），而不是直接排除。
- Grok 另加 `--single`（`-p` 的长名）、`--output-format`、`--json-schema`；Qoder 另加 `-o` 与 `--remote-control`（help 中为 headless worker）；Kilo 另加 `acp`。

## Aieyes 自身的进程

额度/登录 spawn 点注入 `AIEYES_INTERNAL=1`（SSH 端在远程脚本中 `export`）：Qoder `get_usage_info`、Codex `app-server`、Kiro 本机过期登录的 `/usage` 兜底，以及 `agy` 登录（`agy_auth`）。Antigravity 日常额度已改为只读已保存 access token 的 REST 请求，不再启动 `agy --print /usage`。Hook 观察器读到该环境变量时写入 `sessionSource=aieyes-internal`、`classification=internal`。

无法注入环境变量或 Hook 不触发的情况（WP7 Qoder、WP13 Kiro）用 `Store::mark_internal_session(provider, id|cwd)` 写 kv `internal-session:{provider}:{key}`；`sessions.list` 每次读取前载入，本机读取器按会话 id 或可靠元数据中的工作目录排除。

Qoder：`get_usage_info` 控制会话带 `--no-session-persistence`，正常不写 transcript（qodercli 1.1.79 与 npm 1.1.81 未登录实测：只写 `logs/sessions/` 调试日志，不建 `projects/`）。作为兜底，控制会话在 `system`/`assistant`/`result` 帧中报告的 `session_id` 与本机检查的工作目录 `~/.aieyes/qoder-probe` 一并写入内部会话表。写入发生在发起检查的那个核心进程（任一 lane：额度、设备状态、自动识别）每次调用结束时，而不是等 `sessions.list`，因为 macOS 的额度查询在独立进程中运行；`internal-session:qoder:*` 保留 2 天。验证：`cargo test -p aieyes-core --test qoder_internal_sessions`（一个进程查额度，另一个进程读会话）。

## 局限

DeepSeek、Kimi、Cursor、OpenCode、Antigravity、Trae 的会话文件/DB 没有统一的 headless 字段。监控启动前已退出、或原 argv 已被进程标题覆盖而没有其他证据的历史会话，无法事后恢复启动模式。进程关联含歧义、无权限读取或 SSH 平台不支持 `/proc` 时同样保留 `unknown` 并计入；不会按目录名猜测工作目录。运行期间确认的分类在进程退出和静默轮询后仍保留。

Copilot CLI 例外：`copilot -p` 在 `session-state/<id>/events.jsonl` 写入固定的 `system.message`（“You are running in non-interactive mode…”），本机与 SSH 读取器都据此判为 `headless`。2026-10-31 用 Copilot CLI 1.0.105（BYOK 指向 Qwen 端点，未登录 GitHub）核实：`-p` 会话被排除；在伪终端里驱动的交互 TUI 会话不带这条消息，按 `unknown` 计入，`sessions.list` 中正常显示。`session-store.db` 的 `sessions.host_type` 是仓库托管方（`github` 或空），不是无头信号。

## 验证

```
cargo test -p aieyes-core --lib session
cargo test -p aieyes-core --test native_session_processes --test session_observer
python3 scripts/test-session-observers.py
python3 scripts/test-session-processes.py
```
覆盖共享 argv fixture、真实进程到原生日志到公开快照的整条链路、不同项目/数据目录与歧义候选、PID 复用、标题改写、退出落盘、首个 Hook 目录创建和静默轮询。公开快照及 SSH 事件不包含提示词、原始参数或环境。新增 Python suite 已登记在 `scripts/ci-checks.sh`。

2026-10-31 使用隔离配置和工作目录，把本机 Pi、Kimi、CodeBuddy、OpenCode、Grok、DeepSeek Harness、OMP 的模型请求送至授权的 Qwen3.8-27B 端点：七种 headless 调用均成功退出，HTTP 200，用量与响应计数一致，实时会话均未出现。Pi、Kimi、CodeBuddy 的真实 PTY 交互调用均显示活动与完成状态。此验证不涉及真实账户登录或服务商额度；SSH 由相同 argv、原生格式、模拟 `/proc` 与嵌入脚本回归覆盖。


Kiro 补充：kiro-cli 2.36.0 的 `chat --no-interactive` 和 `chat --output-format stream-json` 排除；Aieyes 的 `/usage` 使用 `--v2` 并标记为 internal。V2 会话旁的 `.lock` 允许关联 CLI/ACP worker 与 TUI 父进程；TUI 优先于仅存在 KIRO_API_KEY 的辅助信号。V3/IDE 会话无进程信号时计入，subagent 按 parentSessionId 归属。SSH 同步该规则；本次监控已确认的 headless 判定不会因进程退出而丢失。

Trae 补充（2026-10-31）：已核实安装的 `trae-agent` CLI 与 `TrajectoryRecorder` 源码。只有有效的 `trajectories/*.json` 生成会话；`run` 排除，`interactive` 计入。显式 `-t/--trajectory-file` 按完整路径关联进程，默认轨迹名还必须匹配初始工作目录；Trae 在应用 `--working-dir` 之前确定轨迹路径，因此不按 basename 或时间单独匹配。已确认的 headless/internal 判定在同一任务的短暂写空、截断或元数据未写完时仍保留；有效的新任务标识、实际删除或退出近期文件集合会清除旧判定，删除后重建不会继承。SSH 导入只发送 Token、模型和时间字段，Linux 进程分类与本机保持一致；无法取得进程证据的轨迹按 unknown 计入。

Trae IDE 会话保存在加密存储中，不构造会话行。按版本和机器报告主进程是否运行，并显示“会话数量未知”；Electron helper 不算主进程，该状态不增加会话计数，也不产生读取失败提示。
