# WP12：Cline / Kilo / Roo 核实记录（2026-10-31）

本记录对应 `agent-expansion-implementation-plan.md` 的 WP12，范围是只读账号识别、原生历史和会话分类。没有增加账号切换、并发多账号运行或 refresh token 刷新。测试环境为 macOS Intel（x86_64），Node `x64`。

## 安装与真实请求

通过 npm 包隔离安装 `cline@3.0.74` 和 `@kilocode/cli@7.8.18`，使用各自的 Darwin x64 二进制。Cline launcher 声明的 `@cline/core@0.2.0`、`@cline/shared@0.2.0` 也已解包核对。每次运行均使用独立 HOME、配置目录、数据目录和空项目；四次运行顺序执行。

批准的 BYOK 服务为 `http://10.98.36.99:11444/v1`，模型 `Qwen3.8-27B`，客户端需要密钥时使用无实际凭据的 `not-required`。本地代理只记录 HTTP 状态、模型、用量和数据库生命周期元数据。

| 实际入口 | HTTP | 输入 / 输出 Token | 原生记录 | Aieyes 只读读取得到的会话 |
|---|---|---|---|---|
| Cline `--json` | 200 | 4689 / 23 | `interactive=0`，running → completed | 0 |
| Cline `--tui`，PTY | 200 | 4689 / 24 | `interactive=1`，running → idle → completed | 1，complete |
| Kilo `run --format json` | 200 | 2537 / 15 | 根 session 的 question permission 为 deny | 0 |
| Kilo `serve` 后端，通过 `/session`、`/session/{id}/message` 发起 | 200 | 2540 / 24 | 根 session 的 permission 为 null | 1，complete |

四份落盘历史都成功导入，计数逐项与 HTTP usage 一致。Kilo `serve` 验证的是扩展使用的后端协议，没有冒充 VS Code 扩展 UI 的端到端验证。headless 排除只影响会话数，不会抹去真实请求用量。

本地原始研究材料在 `.local/wp12-tools/`，不进入交付代码：

- `live-cline-1793410021595468000/report.json`
- `live-cline-tui-1793411845896601000/report.json`
- `live-kilo-1793410078271943000/report.json`
- `live-kilo-serve-1793412645218603000/report.json`
- `reader-evidence.json`：以上四份数据经过当前历史和会话读取得到的无正文结果。

回归 fixture `crates/core/tests/fixtures/native-agents/cline-current.json` 保留真实 Token 与时间字段，替换会话 ID、请求 ID 和正文。没有收集日常账号凭据。

## 来源与数据语义

| 来源 | 固定版本 / commit | 核对内容 |
|---|---|---|
| [Cline npm 包](https://www.npmjs.com/package/cline/v/3.0.74) | CLI 3.0.74；core/shared 0.2.0 | 当前 sessions.db、消息 envelope、companion manifest、配置覆盖 |
| [Cline legacy 源码](https://github.com/cline/cline/tree/0e7e0099cdcf5349bebc2cf86b9b5fd64650b736) | v3.72.0 / `0e7e0099cdcf5349bebc2cf86b9b5fd64650b736` | ui_messages、请求配对、压缩汇总、subagent 批次 |
| [Kilo 源码](https://github.com/Kilo-Org/kilocode/tree/b81761530650f8ec7b071441f4f54d2514f1a244) | v7.8.18 / `b81761530650f8ec7b071441f4f54d2514f1a244` | auth/config、SQLite schema、run permissions、profile |
| [Roo 源码](https://github.com/RooCodeInc/Roo-Code/tree/27001b2b5aa47b65e8a6ba1914e0f4216be0ebb0) | v3.54.0 / `27001b2b5aa47b65e8a6ba1914e0f4216be0ebb0` | history_item、任务持久化与 parentTaskId |

Cline 当前 `sessions.db` 包含 `session_id, source, pid, status, interactive, parent_session_id, messages_path, updated_at` 等字段。历史文件为 `<sessions>/<root>/<name>.messages.json`，同目录可包含根会话、agent 和 `agent__teamTask`；不能只按目录名寻找一份 transcript。assistant 消息的 `metrics` 保存 input/output/cache 计数，`modelInfo.id` 才是该请求的模型。原生消息 ID 和时间组成复制历史的去重证明。

Cline 的 `pending` 对应工具确认，映射为 PermissionRequest；`idle` 表示一轮结束后等待输入，映射为 complete。file-only 模式优先读取 companion manifest。超过 24 小时或排在最近 256 项之外的父 manifest 仍可用于 ancestry，且不会因此变成新的可见活动。metadata 索引有数量限制，祖先链最多 32 层；找不到父节点时保留 unknown，会话仍计入。

Kilo 当前安装的数据库为 `$XDG_DATA_HOME/kilo/kilo.db`，实际同时存在 `message` 与 `session_message`；本次真实调用写入 `message`，后一张表为空。子 agent 必须沿 `parent_id` 找根 session 再检查 question deny，不能用子 agent 自己限制提问的规则排除正常交互主会话。

Roo `history_item.json` 有累计用量与 parent/root task ID，没有模型 ID。`apiConfigName` 是配置名称，因此历史模型显示 unknown。legacy Cline/Kilo/Roo 同一目录同时有 `history_item.json` 与 `ui_messages.json` 时，只读取摘要，避免同一任务导入两次。

## Legacy 汇总与混合布局

[Cline SubagentRunner](https://github.com/cline/cline/blob/0e7e0099cdcf5349bebc2cf86b9b5fd64650b736/src/core/task/tools/subagent/SubagentRunner.ts) 在内存中保存子会话，工具 callback 的 `say` 被替换为空操作；[SubagentToolHandler](https://github.com/cline/cline/blob/0e7e0099cdcf5349bebc2cf86b9b5fd64650b736/src/core/task/tools/handlers/SubagentToolHandler.ts) 汇总一次已结束批次，写入 `source: "subagents"` 的 `subagent_usage`。该版本本身不为这些子调用持久化独立任务文件。

导入器仍处理旧/新文件并存：只有独立记录的原生 parent 元数据明确指向 legacy task 时，才停止叠加该 task 中无子请求 ID 的 `subagent_usage`，保留父请求与独立后代，来源标为 `partial / overlappingSubagentSummary`。不会按相同用量或接近的时间猜测重复。

旧请求的 started/finished 配对和 `deleted_api_reqs` 被归为一个稳定的 task 累计事件，压缩不会增加第二份旧用量。`clineTaskUsageVersion` 只适用于 `provider=cline` 且 `id=task:<sessionId>` 的规范 task 行；独立后代稍后出现时，版本 3 可以向下修正旧父汇总。它不改变子消息、其他 provider 或其他未知账号 source 的 namespace。混合布局缺少批次级映射，所得总量明确标为不完整。

## 账号与路径

Cline 支持 providers 对象中的 `settings.apiKey` 和 `settings.auth.apiKey`、账号 token、自定义 `CLINE_PROVIDER_SETTINGS_PATH` 文件名；指定该配置覆盖时不再扫描默认 provider 文件。`CLINE_DIR` 在没有 `CLINE_DATA_DIR` 时决定 data 根目录。SQLite 中的绝对 `messages_path` 可跟随单独的 transcript 目录，手选 session 目录也可用 companion manifest 分类。

Kilo 支持原生 `auth.json`、五种官方 config 文件名、`KILO_CONFIG` / `KILO_CONFIG_DIR`，本机还支持 `KILO_AUTH_CONTENT` / `KILO_CONFIG_CONTENT` 的内存文档。`KILO_DB` 可为绝对路径，或相对于 Kilo data 目录。OAuth `accountId` 是选中的组织 ID，不被当作用户身份；缺少可用 access token 的 refresh-only 项不创建登录连接。

付费身份由只读 Cline `/users/me` 或 Kilo `/profile` 提供，并按凭据指纹缓存。凭据变化时不继承旧账号身份；后续余额响应仍检查身份是否一致。个人/组织余额与 OpenRouter key 额度由共享 prepaid 适配器处理。Roo 仅发现历史，不读取 SecretStorage。

自动发现还扫描已有编辑器的 globalStorage extension ID，以及 vscode-server 目录中的 tasks；发现历史目录本身不是登录证明。限定目标的自动扫描不受另一个 provider 的无效路径覆盖影响。

## 验证与边界

- `cargo test -p aieyes-core --locked`：通过完整 core suite，含 446 个 lib tests 和集成测试。
- `cargo test -p aieyes-core --test task_json`：6 项通过，覆盖原生请求、复制、重放、压缩、损坏文件、混合汇总修正及 namespace。
- `python3 scripts/test-task-remote.py`：8 项通过，覆盖 SSH 历史、增量、manifest、旧/溢出祖先、隐私和混合布局。
- `cargo clippy -p aieyes-core --all-targets --locked -- -D warnings`、修改文件 rustfmt 检查、Python py_compile：通过。
- `sh scripts/ci-checks.sh`：通过，含 Python observers / native database / account scan、生成物与本地化校验、CSS lint 和 Web tests；日志 `.local/wp12-ci.log`。
- 本机真实原生数据经 SSH Python 读取得到上述四组相同计数和会话分类；没有进行真实 SSH 主机连接测试。
- 付费 Cline/Kilo 个人/组织余额、登录 profile 与 OpenRouter 额度只通过 fixture 验证，**未实测**付费账号。
- Roo 依据固定源码与 fixture 验证；legacy Kilo 是兼容 fixture，未把找不到的历史发行包标为装机验证。
- `CLINE_DB_DATA_DIR` / `CLINE_SESSION_DATA_DIR` 单独覆盖的任意目录尚未自动加入所有默认发现入口；明确选择目录或从 sessions.db 的 messages_path 可读取。Kilo 任意 channel DB 的自动枚举尚未覆盖，可明确指定 `KILO_DB`。
- 复制消息的稳定 ID/时间去重已覆盖；只有 fork seed 而没有原始请求可比较的孤立历史，尚未按 Cline `metadata.fork` 额外剔除。

共享 WP1/WP2 测试契约与 prepaid 文件作为本工作树验证依赖，记录在 `.local/wp12-dependency-overlay.json`，不进入 WP12 合并增量。逐文件哈希见本地 handoff。
