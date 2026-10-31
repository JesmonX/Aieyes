# Agent 扩展实施计划（依据 agent-auth-quota-research-2026-10-29.md (f)(g)(h)）

范围：(g) 全部 + (h) 三项决定：
- headless 规则：无信号默认计入，标 `classification=unknown`。
- ChatGPT Work 与 Codex 分开显示。
- 现有 agent 的零配置识别与额度补缺，包括现有 agent 尚未支持的形态：Qoder 桌面/QoderWork、Kimi Work、Cursor IDE、仅装 IDE 的 Antigravity、CodeBuddy IDE、Trae CN。
- 新 agent，按优先级：WorkBuddy > Copilot > Cline/Kilo（含 Roo 历史）> Kiro > Trae。
- Claude macOS 首次启动请求一次钥匙串（h1）。
- WorkBuddy/CodeBuddy 可选“登录查看余额”（h2）。
- Qoder 后台定时查询（h3）。

约定：
- 行号以 `feat/agent-auth-quota` 当前状态为准，实施时以函数名为准。
- 每个 WP 由一个实施 agent 完成，完成后必须通过 `scripts/ci-checks.sh`；改到 Swift 的 WP 还要跑 `scripts/native-verifiers.sh` / `sh scripts/test-native.sh`。
- 标注 **[装机]** 的条目需要在本机用 brew/npm 安装 agent 核实。本机没有任何账号，所以只能核实文件布局、argv、help、DB schema 和未登录行为。需要模型调用时，把 agent 的自定义 provider 指向 `http://10.98.36.99:11444`，模型 `Qwen3.8-27B`（OpenAI 兼容）。
- 有账号才能核实的额度响应，一律用 fixture 单测覆盖，并在文档中标为“未实测”。

## 0. 共享文件与串行约束

| 热点文件 | 涉及 WP | 规则 |
|---|---|---|
| `docs/core-contracts.json` + 生成物（`contracts.rs`、`web/contracts.js`、`Models.swift` 生成段） | WP0（集中加全部 provider/method），之后 WP6/7/11/15 只追加 method | WP0 先行；后续追加串行，每次都重跑 `sync-core-contracts.py` |
| `crates/core/src/account_scan.rs`、`scripts/remote_account_scan.py` | WP3 → WP6 → WP8 → WP9 → WP10 → WP11 → WP12 → WP13 → WP14 | **严格串行**（核心链） |
| `crates/core/src/subscriptions.rs`、`quota.rs` | WP6, WP7, WP8, WP9, WP10–WP15 | 跟随核心链串行 |
| `sessions.rs`、`session_events.rs`、`session_hooks.rs`、`scripts/session_remote.py` | WP1 → WP2 →（WP10–14 的会话部分） | 会话链串行 |
| `models.rs` QuotaSnapshot、`Models.swift` Quota、`Views.swift` QuotaCard、`web/app.js` quotaCard | WP5（唯一改模型的 WP），WP4 改设置 UI | WP5 先于所有需要新额度项的 WP |
| `locales/en.json` → `sync-localizations.py` | 几乎所有 WP | 改完各自重跑 sync；合并冲突时重新生成，不手工合并 |
| `web/agent-settings.js`、`AgentSettingsViews.swift` | WP4, WP6, WP7, WP11, WP15 | 串行 |

可并行的三条线：
- **会话线**：WP1 → WP2。
- **核心额度线**：WP3 → WP5 → WP6 …
- **UI 线**：WP4，在 WP3 之后。

新 agent 的 WP（10–14）同时要进会话线和核心线，因此在两条线都空闲后再做。

---

## WP0 契约骨架：新增 provider、method、图标与文案

**目标**：一次性登记全部新 id，后续 WP 不再改 provider 列表，避免冲突。

**文件：`docs/core-contracts.json` `providers[]`**，追加以下条目（`capabilities` 先只开实际会交付的能力，`quota` 在对应 WP 交付时再打开也可以，但为减少冲突建议此处一次写好）：

| id | 说明 |
|---|---|
| `chatgpt-work` | `agent:"codex"`，`defaultPath:"~/.codex"`，`discoverLocal:false`，capabilities `["quota","quotaReset"]`；额度复用 Codex 快照 |
| `workbuddy` | `defaultPath:"~/.workbuddy"`，capabilities `accounts, machines, quota` |
| `copilot` | `defaultPath:"~/.copilot"`，binary `copilot`，capabilities 加 `quotaReset` |
| `cline` | `~/.cline/data` |
| `kilo` | `~/.local/share/kilo` |
| `roo` | 只做历史，无 quota；VS Code globalStorage 路径在 WP12 解析 |
| `kiro` | 路径在 WP13 装机后确认 |
| `trae` | 路径在 WP14 装机后确认 |
| `trae-cn` | `agent:"trae"` |

`codebuddy` 的 capabilities 增加 `quota`。`kimi` 不新增 id：Kimi Work 复用快照，只在会话上区分，WP9 判断是否需要 `kimi-work` 别名，默认不加。

**`methods[]`** 预登记（lane、invalidates 与 `accounts.*` 同类）：
- `accounts.autoScan`
- `claudeAuth.keychain`、`claudeAuth.status`
- `copilotAuth.start`、`copilotAuth.status`、`copilotAuth.cancel`
- `buddyAuth.start`、`buddyAuth.status`、`buddyAuth.cancel`、`buddyAuth.logout`

`accounts.scan` 暂时保留，WP4 再删。

**其他文件**：
- `crates/core/src/accounts.rs:9 default_name`：给每个新 id 加分支。
- `crates/core/src/provider_paths.rs:19`：新增环境变量映射，例如 `copilot → COPILOT_HOME`（是否存在待核实，没有就不加）；同步修改两张测试表 `provider_paths.rs:148`、`crates/core/tests/provider_paths.rs:7`。
- 图标：`apps/desktop/web/provider-{chatgpt-work,workbuddy,copilot,cline,kilo,roo,kiro,trae}.png`（144×144），并记录到 `docs/provider-icon-sources.json`、`docs/provider-icons.md`；同时更新 `scripts/test-desktop.mjs:73` 的 PNG 列表。
- `settings.rs:510/525/531` 的 `"deepseek"` 特判改为判断 `default_path == ""` 或 `discoverLocal:false`，让 `chatgpt-work` 不创建 Source。
- `locales/en.json`：新名称与 hint。

**测试**：
- `cargo test -p aieyes-core provider_paths settings`
- `node scripts/test-agent-settings-ui.cjs`、`test-workspace-ui.cjs`、`test-settings-navigation-ui.cjs`（agent 数量与顺序变了）
- `scripts/verify-selection.swift`

**验证**：
```
python3 scripts/sync-core-contracts.py && python3 scripts/sync-localizations.py && python3 scripts/check-localizations.py && scripts/ci-checks.sh
```

**依赖**：无。**冲突**：所有后续 WP。

---

## WP1 Headless 分类器 + argv + AIEYES_INTERNAL（P0）

**目标**：
- 把 (c) 的规则表落成按 provider 分派的分类器。
- 没有信号时计入，并标 `unknown`。
- 凡是 Aieyes 自己 spawn 的进程一律排除。

**文件**：
- `crates/core/src/session_events.rs`
  - 新增 `pub(crate) fn headless_argv(provider:&str, argv:&[String]) -> Option<&'static str>`，覆盖 (c) 表中全部行：dsh、kimi、cursor、opencode（`serve --service` 算不计会话）、antigravity、qoder、zcode、grok、pi/omp、copilot、kiro、cline、kilo、trae-cli。
  - `Event` 与 `Activity` 增加 `classification`（`interactive|headless|internal|unknown`），或复用 `session_source="argv:<rule>"`。新字段在 `Activity::apply` 中按粘性规则合并。
  - Cursor hook 的 `is_background_agent=true` 映射为 `headless`（`hook()` :41-75）。
- `crates/core/src/session_hooks.rs`
  - `ancestry` :578 改用 `ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always)`，并返回 argv。
  - cursor/zcode/codebuddy 不走 :580 的 pid 绑定，另开一条只取 argv 的路径。
  - `run_if_requested` :610 中，如果有 `AIEYES_INTERNAL` 环境变量，就设 `session_source="aieyes-internal"`。
- `crates/core/src/sessions.rs`
  - `non_interactive` :571 改为调用 `session_events::classify(provider,&Activity)`：`internal` 对所有 provider 排除；`headless` 排除；`unknown` 计入。
  - 进程采样时，把 pid 对应的 argv 判定写入 `Activity`。现有的 `activity.pid` 绑定点在 `consume` :662 附近。
  - 新增 Aieyes 内部会话过滤表：kv 中的 `internal-session:{provider}:{id|cwd}`，由 WP7（Qoder）、WP13（Kiro）写入，`consume` 读取后排除。
- `scripts/session_remote.py`
  - `ancestor` :160 改读 `/proc/<pid>/cmdline`。
  - `EVENT_KEYS` :17 加入新字段。
  - 复制 `headless_argv` 的同一张表，并用共享 fixture 保证两边一致。
- 新 fixture：`scripts/fixtures/headless-argv.json`（`provider`、`argv`、`expected`），Rust 与 Python 共用。
- quota 侧：所有 spawn 点注入 `AIEYES_INTERNAL=1`。包括 `subscriptions/qoder.rs` 的 `command` :104、`remote_script` :87，`quota.rs` 的 agy/codex app-server spawn，以及 `agy_auth.rs launch`（登录不算会话）。

**测试**：
- 在 `sessions.rs` 的测试 :3158、:3213、:3289 中扩展 argv、internal、unknown 三种情况。
- 在 `crates/core/tests/session_observer.rs` 中增加 argv 和 `AIEYES_INTERNAL` 的用例。
- `scripts/test-session-observers.py` 增加读取 fixture 的用例。
- 新增 Rust 单测 `headless_argv_table_matches_fixture`。

**验证**：
```
cargo test -p aieyes-core sessions session_observer && python3 scripts/test-session-observers.py
```

**[装机]**：用 `--help` 复核以下 agent 的 argv：kimi（`uv tool install kimi-cli` 或 brew）、opencode（`brew install sst/tap/opencode`）、cursor-agent、copilot（`npm i -g @github/copilot`）、cline（`npm i -g cline`）、kilo（`npm i -g @kilocode/cli`）、grok、pi（`npm i -g @mariozechner/pi-coding-agent`）。
- 用 Qwen 端点各跑一次 `-p`，确认会被分类为 headless；再跑一次交互模式，确认计入。

**文档**：`docs/codex-session-sources.md`、`docs/claude-session-sources.md`，新增 `docs/headless-rules.md`。

**依赖**：WP0（新 provider 的 id）。**冲突**：WP2、WP10–14 的会话部分。

---

## WP2 ChatGPT Work 独立显示（originator）

**目标**：`originator=codex_work_desktop` 的会话与用量以 `chatgpt-work` 显示；额度共享 Codex 快照；headless 判定仍只看 `source`。

**文件**：
- `session_events.rs`
  - `Event`/`Activity` 增加 `originator`。
  - `codex()` :123/:143 读取 `payload.originator`。
- `sessions.rs` `consume` 构建 LiveSession 处（:725 附近）：把显示用的 `provider` 设为 `chatgpt-work`，`id` 仍用 codex 的 hash，以保证 `parent_id`（:743-751）一致。
- `usage.rs` `parse_codex` :117/:129-166
  - `ParseState.originator` 写入 UsageEvent 的 agent 归属（核对 `models.rs` UsageEvent 与 `codex_attribution.rs`）。
  - 同时提升 `parser_version("codex")` :6。
- 额度复用：`quota_queries` 和 UI 对 `chatgpt-work` 不单独查询，`EngineClient.swift quotaAccounts` 与 `app.js` 把 Codex 卡片标注为 “Codex · ChatGPT Work”，或者在 Work 下显示同一张卡。规则：同一 poolId 只显示一次。
- 检测：`/Applications/ChatGPT.app` 存在即视为有 Work 形态，`account_scan` 不新增账号。
- `scripts/session_remote.py` 的 `metadata` codex 段（:228）加入 `originator`。
- `scripts/remote_history.py` 的 codex 分支同步修改。

**测试**：
- 在 `scripts/fixtures/codex-session-sources.json` 中加入 `codex_work_desktop`、`Codex Desktop`（`source=vscode`，可见）以及 `guardian_review` 三个用例，并加上 `expectedProvider`。
- 修改 `sessions.rs` 测试 :3343、:3385，`crates/core/tests/fork_history.rs`（fork 继承 originator），`scripts/test-session-observers.py` :98/:111。

**验证**：
```
cargo test -p aieyes-core && python3 scripts/test-session-observers.py && python3 scripts/test-incremental-history.py
```

**[装机]**：如果本机已有 ChatGPT.app，抽一个 rollout 文件确认 `originator` 的取值；不安装新应用。

**依赖**：WP1。**冲突**：WP1、usage.rs（WP10/12/13/14 的历史解析）。

---

## WP3 自动识别基础设施（P0.5 核心）

**目标**：
- 启动时自动识别，凭据文件变化时自动识别。
- 去掉 pending 状态。
- 半写文件回退到上次快照。
- 统一 SQLite 只读规范。
- 记住历史账号。
- 跨来源按服务端身份去重。
- token 过期 24h 才提示。

**文件**：
- `account_scan.rs`
  - `scan_accounts` :1014：新增 `scope:"auto"`，即全部已启用 agent × 已启用机器，目标为空时不 `ensure!`。
  - 由 `accounts.autoScan` 暴露。
  - 在 engine 启动时调用（`lib.rs` Engine 初始化后的后台线程），之后在 `refresh_native_connections` :1659 中按凭据文件 mtime 和 size 指纹触发。新增 kv 键 `native-mtime:{conn}`。
  - :610 的新 candidate 直接设为 `observed`，删除 `pending`。
  - 解析失败（:1100）时保留上次连接和快照，不推送 `failed`；连续失败超过 24h 才标为 `stale`。
  - `expired` 的处理（:722-739）：先保持 `observed` 并记录 `token_expired_at`；超过 24h 改为新状态 `stale`，提示文案为“打开 {agent} 一次即可刷新”。删除所有“重新扫描”文案（:739、:1159、:1494）。
  - 身份变化时不报错，新建或复用 `Account`，旧连接保留为历史账号（`unavailable` 改名为 `remembered`）。
  - 新增 `upstream_identity: Option<String>` 字段，放在 `NativeConnection` 和 `Account`（`models.rs:218`）上，取值形如 `anthropic:{accountUuid}`、`openai:{user}\0{ws}`、`github:{id}`、`google:{sub}`。`quota_sources` :1383 按它去重，只保留原生来源。
  - Claude 扫描路径：删除 :446-474 的 `security` 调用（钥匙串交给 WP6）。没有身份时显示“未登录或使用 API key”。
- 新建 `crates/core/src/sqlite_ro.rs`：`open_ro(path)` 使用 `file:{path}?mode=ro&immutable=0`、`SQLITE_OPEN_READ_ONLY`、`PRAGMA query_only=1`，不执行 checkpoint，带超时。`opencode.rs`、`native_sessions.rs` 改用它。
- `quota_schedule.rs` / `accounts.rs`：历史账号只在 `has_quota_target` 为真时查询，否则显示最后快照。快照年龄字段由 WP5 提供，本 WP 只在 `QuotaSnapshot` 中先加 `stale_since`（与 WP5 协调：**WP5 定义字段，WP3 只写入**，因此 WP5 的模型部分先合入，见 WP5 说明）。
- `scripts/remote_account_scan.py`：解析容错与 Rust 一致。

**测试**：
- `crates/core/tests/account_scan.rs`
  - 重写 `defaults_are_fresh_only_and_scanning_is_explicit…` :73，改成“自动、幂等、私有、无 pending”。
  - 更新 :242/:286/:349/:393。
  - 新增：半写 JSON 回退、mtime 触发、身份切换后记住旧账号、upstream 去重、24h stale。
- 单测 :1969 的文案。
- `scripts/test-account-scan.py`。

**验证**：
```
cargo test -p aieyes-core account_scan accounts quota_schedule && python3 scripts/test-account-scan.py
```

**依赖**：WP0。**冲突**：核心链全部后续 WP；`models.rs` 与 WP5 相关（建议 WP5 的模型部分作为 WP3 的第一个提交，或 WP5 先做）。

---

## WP4 去掉扫描 UI 与 pending 文案（Web + Swift）

**目标**：界面上不再有扫描按钮和“待确认”；识别结果通过 settings revision 自动刷新；显示 `stale`/`remembered` 状态。

**文件**：

Web：
- `web/account-scan.js`：删除 `button`/`hint`/`scan`/:47-49，保留 `list` 与解除全部引用，文案 :63 删除“重新扫描”的说法。
- `web/agent-settings.js`：删除 :331 的扫描按钮；修改 :21-34 的状态文案；`sourceActionsHTML` :370 只对 codex 显示切换按钮。
- `web/app.js`：删除 :1328 的主机扫描按钮。
- `scripts/desktop-preview/model.js:137`：加 `accounts.autoScan` 的 stub。

Swift：
- 删除 `AccountScanView.swift` 中的 `NativeScanButton`，以及 3 个调用点（`AgentSettingsViews.swift:424`、`SettingsView.swift:186`、`AccountTargetsView.swift:77`）。
- `NativeConnectionsSection` 改文案。
- `Models.swift:552 statusLabel`：`stale` → “打开 X 一次即可刷新”，`remembered` → “历史账号”。

契约与文案：
- 在 `core-contracts.json` 中删除 `accounts.scan` 并重新生成（同时改 `contracts.rs:45` 的生成物与 `crates/core/src/lib.rs:320` 的 dispatch）。
- 删除 (ui-web §4) 列出的字符串，并删除 `LocalizationCatalog.swift` 中对应的条目。

**测试**：
- 重写 `scripts/test-account-scan-ui.cjs`：启动后无需点击即出现账号；没有扫描按钮；不渲染私钥；解除引用的确认保留。
- 检查 `test-history-rescan-ui.cjs`、`test-refresh-monitoring-ui.cjs`、`test-credits-wakeups-ui.cjs`、`test-sampling-theme-ui.cjs` 中的文案。
- Swift：`verify-macos-ui.swift`、`verify-native-settings-snapshots.swift`。

**验证**：
```
cd apps/desktop && npm run test:ui; scripts/ci-checks.sh; scripts/native-verifiers.sh
```

**依赖**：WP3。**冲突**：WP6/7/11/15 的设置 UI。

---

## WP5 额度模型扩展：通用计量项、快照年龄、非错误提示

**目标**：给积分池、本月已用、超额、快照年龄和“打开 X 刷新”提供统一表示，后续 agent 不需要再加 struct。

**文件**：
- `models.rs:797-880` 的 `QuotaSnapshot` 增加：
  - `meters: Vec<QuotaMeter{id, group, label, used, total:Option, remaining:Option, overage:Option, unit("credits"|"requests"|"usd"|"cny"|"points"), resets_at, estimated:bool}>`
  - `snapshot_at`、`stale_since`
  - `notice: Option<String>`：非错误提示，例如“打开 X 一次即可刷新”或“本地估算”
- `quota.rs:194 normalize`：在 :243 的非空检查中把 `meters` 也算作非空。
- Swift `Models.swift:318`：`Quota` 增加对应的可选字段（带默认值，保持 memberwise init 兼容）。
- `Views.swift` QuotaCard：在 `balanceContent` :1193 中渲染 meters；补充无障碍摘要 :1246；空状态判断 :1382 也要计入 meters；新增 notice 与“上次 xx 前”标记。
- `EngineClient.swift:779` 的占位 Quota。
- Web `app.js` 的 `quotaCard` :1033：渲染 meters 与 notice。
  - `quotaNeedsAttention` :362 不把 notice 和 stale 当错误。
  - 分组逻辑 :1040 把 antigravity 特判改成“任意 provider 的 windows 带 groupName 即分组”，供 Claude/Cursor/Kiro 使用。
- `quota-card-layout.js:16/:33` 如有新别名（`chatgpt-work` → codex）也要处理。
- `scripts/remote_quota.py`：输出结构同步。

**测试**：
- `scripts/verify-models.swift`：加 meters 与 stale 的解码用例。
- `scripts/verify-quota-layout.swift`、`test-native-quota-layout.sh`：加只有 meters 的卡、stale 的卡、非 codex 的卡。
- `scripts/verify-cpu-optimization.swift`：重新编译。
- `scripts/test-quota-ui.cjs`、`test-quota-card-layout.cjs`。
- Rust：`normalize` 单测。

**验证**：
```
cargo test -p aieyes-core quota && cd apps/desktop && npm run test:ui && sh scripts/test-native.sh
```

**依赖**：WP0；建议在 WP3 之前或与 WP3 同时（模型部分先合入）。**冲突**：任何改 `QuotaSnapshot` 或 QuotaCard 的 WP。

---

## WP6 Claude：完整官方额度 + macOS 首次启动钥匙串（h1）

**目标**：
- 补齐 `extra_usage` 与 `limits[]`。
- macOS 首次启动只请求一次 `Claude Code-credentials`，token 只放内存。
- 401 时等待 Claude 自己刷新。
- 拒绝授权后显示本地估算，并提供重试入口。

**文件**：
- `quota.rs`
  - `normalize` 的 claude 分支 :226-246：`extra_usage` → `ExtraUsageWallet`；`limits[]` → 带 group 的 windows；按模型分项。
  - 本地 claude 的读取 :113-170：删除每次查询都调用 `security`，改为读 `claude_keychain::cached()`。
- 新建 `crates/core/src/claude_keychain.rs`
  - 进程内 `OnceLock<Mutex<Option<Token>>>`。
  - `request_once()` 调用一次 `/usr/bin/security find-generic-password -s "Claude Code-credentials" -w`。因为 app 是 ad-hoc 签名（`tauri.macos.conf.json signingIdentity "-"`），走 `security` 可以避免每次更新后“始终允许”失效；这一点写进文档。
  - 在 settings 中持久化 `claude_keychain: {asked_at, granted}`，不存 token。
  - 当 `~/.claude/.credentials.json` 或 `~/.claude.json` 的 mtime 变化，且缓存的 token 返回 401 时，最多每 24h 重新调用一次，以接受 Claude 重写条目后可能再弹窗的情况。
- RPC：`claudeAuth.keychain`（重试或关闭）、`claudeAuth.status`。在 `lib.rs` dispatch 中注册。
- 启动时机：engine 启动后，只在 macOS、`~/.claude` 存在且从未询问过时执行。Tauri 侧在 `boot` 之后调用 `claudeAuth.keychain`（`app.js` 中 `loadDashboard` 旁边）；SwiftUI 在 `EngineClient` 启动流程中调用。**只执行一次**，以 settings 中的 `asked_at` 为准。
- UI：在 Claude 账号区显示一行状态（“已获授权 / 已拒绝，显示本地估算 [重新授权]”）。Web 改 `agent-settings.js`，Swift 改 `AgentSettingsViews.swift`。
- `scripts/remote_quota.py`：同步 `extra_usage` 与 `limits[]` 的解析。

**测试**：
- `quota.rs` 增加 claude normalize 的 fixture 单测（含 `extra_usage` 与 `limits`）。
- `claude_keychain` 用注入的命令 runner 做单测：只询问一次、拒绝后不再询问、401 后在 24h 节流下重新询问。
- `scripts/test-kimi-wallet-ui.cjs` 复制为 `test-claude-quota-ui.cjs` 并加入 `package.json` 的 `test:ui`。

**验证**：
```
cargo test -p aieyes-core quota claude_keychain && npm run test:ui
```
然后手工：删除 settings 中的 `asked_at`，启动 app，确认只弹窗一次。

**[装机]**：Claude Code 已安装（`~/.claude` 存在）；没有账号，只验证“未登录”路径以及 `security` 找不到条目时的行为。

**依赖**：WP3、WP5。**冲突**：`quota.rs`、`agent-settings.js`、`AgentSettingsViews.swift`。

---

## WP7 Qoder：后台定时查询（h3）+ 限频 + 一键安装 CLI + 桌面/QoderWork 形态

**目标**：Qoder 后台定时查询，间隔不少于 10 分钟；不论面板是否打开都查；spawn 时带 `AIEYES_INTERNAL`；只装桌面版的用户有一个按钮。

**文件**：
- `quota_schedule.rs:14 interval()`：增加 provider 下限表 `fn floor(provider)`，取值为 `qoder 600`、`copilot 300`、`trae 600`、`kiro 300`。
- `quota_queries.rs:302-310` 改为调用 `quota_schedule::interval`。
- `subscriptions/qoder.rs`
  - `command` :104 和 `remote_script` :87 注入 `AIEYES_INTERNAL=1`。如果 WP1 已经做了，这里只复核。
  - 记录这次 control session 的 id 和 cwd，写入 WP1 的内部会话表。
- 桌面/QoderWork 形态：检测 `/Applications/Qoder.app`、QoderWork.app。如果只有桌面版、没有 `qodercli`，Qoder 账号区显示按钮“安装 qodercli 并登录”，内容为复制命令 `npm i -g @qoder-ai/qodercli && qodercli login`（核实包名），并提供第二选项 PAT（使用现有 subscription 模式）。桌面的 `auth.dat` 不读。
- UI：Web `agent-settings.js`（Qoder 区），Swift `AgentSettingsViews.swift:809-821`。刷新间隔提示 `ConfigurationActions.swift:173` 上显示“最少 10 分钟”。

**测试**：
- `quota_schedule.rs` 测试（:301+）增加 floor 用例。
- `quota_queries/tests.rs`。
- `subscriptions/qoder.rs` 测试检查 env 中有 `AIEYES_INTERNAL`。
- `scripts/test-subscriptions-ui.cjs` 增加安装按钮用例。

**验证**：`cargo test -p aieyes-core quota_schedule qoder`，以及 `scripts/verify-qoder-sync-live.py`（本机有 qodercli 时运行）。

**[装机]**：用 `npm i -g` 安装 qodercli，确认未登录时 `get_usage_info` 的输出，以及在 `AIEYES_INTERNAL=1` 下产生的 transcript 能被过滤。

**依赖**：WP1、WP3。**冲突**：`quota_schedule.rs`（WP11/13/14 只引用 floor 表，不改它）。

---

## WP8 现有 agent 补缺 A：ZCode、DeepSeek、Antigravity、Grok

**目标**：这四个 agent 零配置可读身份与额度。

**文件**：
- **ZCode**
  - `account_scan.rs` 的 `spec` :145 与 `extra_paths` :210 加入 `${ZCODE_DATA_BASE_DIR:-~}/.zcode/v2/credentials.json`。
  - `extract` 增加分支：从 JWT 与 `userInfo` 生成 `login` 行；区域按登录 provider 推断。
  - `quota_sources` :1393 加入 `zcode`，`SubscriptionConfig.mode` 设为推断出的 global/cn。
  - `subscriptions.rs` 中 zcode 的 `request` 先用 JWT，失败时改用 `providerRules` 里 coding-plan key 的 api 行，零操作。
  - **注意不要改 `provider_paths.rs`。**
- **DeepSeek**
  - `spec` 加入 `$DSH_HOME/.credentials.yaml` 与 `.env`。
  - `extract` 解析 records/refs，账号 token 调 `get_user_summary`（新 endpoint，加入 `subscriptions.rs:150` 的允许列表）。
  - 401 的 notice 文案为“在 dsh 中重新登录”。
- **Antigravity**
  - `quota.rs` 的 `parse_agy` :673 增加 `loadCodeAssist.availableCredits` 与 tier，写成 meters 和 plan。
  - token 过期时不使用 refresh token。
  - 仅装 IDE 的形态：**[装机]** 用 `brew install --cask antigravity`，确认 IDE 是否写 `antigravity-oauth-token` 或其他文件。如果都没有，保留 `agy_auth` 兜底。
- **Grok**：`subscriptions.rs:100` 遇到多个 OIDC 条目时不再报错，改为选 `last_used` 或 mtime 最新的条目；保留手选覆盖；每个条目各自作为一个记住的账号。
- `scripts/remote_account_scan.py` 同步以上全部改动。

**测试**：
- `account_scan.rs` 单测 :1880 的 fixture 增加 zcode `credentials.json`、dsh yaml/.env。
- `subscriptions.rs` 测试增加 zcode JWT、DeepSeek summary、grok 多条目。
- `quota.rs` 增加 agy credits 用例。

**验证**：`cargo test -p aieyes-core account_scan subscriptions quota`。

**[装机]**：
- zcode：安装 ZCode 后确认 `v2/credentials.json` 的结构（未登录时）。
- dsh：`npm`/`brew` 安装后确认 `.credentials.yaml` 的位置。
- grok CLI。

**依赖**：WP3、WP5。**冲突**：核心链。

---

## WP9 现有 agent 补缺 B：OpenCode、Pi/OMP、Cursor、Kimi(+Work)、CodeBuddy IDE

**文件**：
- **OpenCode**
  - 通过 `sqlite_ro` 读 `opencode.db` 的 credential 表，`active=1` 的为当前凭据。
  - **[装机]** 核实 `value` 是否为明文，以及 `account_state` 的列名。
  - 如果是 Anthropic/OpenAI/Copilot 凭据，生成对应 `upstream_identity` 的 api/login 行，复用上游适配器；按 WP3 的规则去重。
- **Pi/OMP**：`auth.json` 解析失败时当作未登录；其中的上游凭据同样复用上游适配器。
- **Cursor**
  - IDE 的 `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`（`sqlite_ro`）读取 `cursorAuth/accessToken`、`cachedEmail`、`stripeMembershipType`（**[装机]** 核实键名）。
  - `subscriptions.rs` 的 cursor 改为 Bearer 调 `api2 GetCurrentPeriodUsage`，并**删除 `session` cookie 模式**（同时修改 `core-contracts.json` 中 cursor 的 `subscription.modes`，并重新生成）。
  - 团队池按 teamId 去重。
  - 读不到时 notice 为“打开 Cursor 一次”。
  - UI：`AccountConnectionView` 与 `agent-settings` 的 cursor cookie 表单删除。
- **Kimi**
  - 不调用刷新接口。
  - `config.toml` 中的 `api_key` 增加 Moonshot 余额（`balances`）。
  - Kimi Work 通过进程或应用（**[装机]** 确认 Kimi Work 的形态与目录）复用快照，只在会话上标注。
- **CodeBuddy**
  - IDE 形态：**[装机]** 确认 CodeBuddy IDE 的目录。
  - 现有的“本月已用”（`usage.nativeCredits`）也作为 meters 的一项（`estimated:true`）进入 QuotaSnapshot，让额度卡可见（capability 已在 WP0 打开）。
  - 核实内置 CLI 是否有 usage 命令。

**测试**：
- `crates/core/tests/opencode.rs` 增加 credential 表 fixture。
- `account_scan.rs` 单测增加 Cursor state.vscdb fixture（建一个临时 sqlite）。
- `subscriptions.rs` 增加 cursor api2 用例。
- `scripts/test-native-api-ui.cjs` 与 `test-subscriptions-ui.cjs`（删除 cookie 模式）。
- `scripts/test-native-credits-ui.cjs codebuddy`。

**验证**：`cargo test -p aieyes-core`，`npm run test:ui`。

**[装机]**：`brew install --cask cursor`、opencode、pi、kimi、codebuddy。

**依赖**：WP8。**冲突**：核心链，契约（cursor modes）。

---

## WP10 新 agent：WorkBuddy（会话 + 本月已用 + 身份）

**文件**：
- 识别：`account_scan.rs` 的 `spec("workbuddy")` 用 `sqlite_ro` 读 `~/.workbuddy/workbuddy.db`，取 `user_id` 与个人/团队信息。不碰 `edge-sync-mapping-v4.db` 和 `device-id`。`remote_account_scan.py` 同步。
- 会话：在 `native_sessions.rs::refresh` 中增加 workbuddy 的 SQLite 会话。排除条件：`is_background_automation=1`、`is_playground=1`，以及出现在 `automation_runs` 中的会话（接入 WP1 的分类器，判为 `internal`/`headless`）。同时修改 `sessions.rs:1048` 的 root 映射。
- 用量/积分：`session_usage.credit_json` 按月汇总，在 `usage.rs` 中走 `native_credits` 门控（:422，加 workbuddy）。如果需要历史导入，在 `import.rs:90` 处理。
- 额度：在 `quota.rs` 中为 workbuddy 增加一个分支，生成本地 meter“本月已用”（`estimated:true`）。可选登录的余额见 WP15。
- UI：`app.js:889` 的 native credits 已经是通用的；`agent-settings.js:205` 与 `AgentSettingsViews.swift:203/211/223` 加入 workbuddy。

**测试**：
- 新 fixture `crates/core/tests/fixtures/workbuddy/workbuddy.db`（用脚本生成，不要提交真实数据）。
- 测试覆盖：会话排除、按月汇总。
- `test-native-credits-ui.cjs` 加 workbuddy 分支，并加入 `package.json`。

**[装机]**：安装 WorkBuddy（官网 dmg），未登录时确认 db schema 与 `local_storage` 是否有未加密的计费缓存。如果有，零配置读余额。

**依赖**：WP1、WP3、WP5、WP9。**冲突**：会话链、核心链。

---

## WP11 新 agent：GitHub Copilot（识别 + 额度 + device-code + 会话）

**文件**：
- 识别：依次读取环境变量 `COPILOT_GITHUB_TOKEN`/`GH_TOKEN`/`GITHUB_TOKEN`，再读 `~/.config/github-copilot/apps.json`（放在 `extra_paths`）；`~/.copilot/config.json` 做容错解析。**不运行 `gh auth token`，也不读第三方 Keychain。**
- 身份：`GET api.github.com/user`，`upstream_identity=github:{id}`。
- 额度：`subscriptions.rs` 增加 copilot endpoint `api.github.com/copilot_internal/user`。解析 plan、重置日期、premium 的 total/remaining/overage（写成 meters），以及 Free 的 chat/completions。轮询下限 300s（使用 WP7 的 floor 表）。
- device-code：新建 `crates/core/src/copilot_auth.rs`，提供 `copilotAuth.start`/`status`/`cancel`，走 GitHub 官方 device flow（client_id 使用 Copilot CLI 公开的 id 还是 Aieyes 自己注册的 OAuth App 需要确认，**默认用 Aieyes 自己的 OAuth App**）。
  - token 存到 `credentials.rs`。为此要泛化 `entry(service, ref)`，新增 service `app.aieyes.agents`；`credentials.rs:82` 的测试要更新。
- 会话：Copilot CLI 的会话目录 `~/.copilot/session-state`（**[装机]** 核实），解析器放在 `native_agents.rs` 或新模块，判断 `host_type`；argv 规则用 WP1 的。`session_remote.py`/`remote_history.py` 同步。
- UI：Copilot 账号区在没有 token 时显示“GitHub 授权”按钮，打开对话框（参照 `openAntigravityLogin` 的模式）。Web 改 `agent-settings.js`，Swift 新建 `CopilotAuthView.swift`。

**测试**：
- `subscriptions.rs` copilot 解析（Pro、Free、超额）。
- `copilot_auth` 用 mock HTTP 测试。
- `credentials.rs` 测试 service 泛化。
- `native_agents` 加 copilot fixture。
- 新增 `scripts/test-copilot-auth-ui.cjs`。

**[装机]**：`npm i -g @github/copilot`，确认会话目录、`config.json` 格式、未登录行为；用 Qwen 端点（如果支持 BYOK）跑一次会话。

**依赖**：WP1、WP3、WP5、WP7、WP10。**冲突**：核心链、会话链、`credentials.rs`（与 WP15 共用）。

---

## WP12 新 agent：Cline / Kilo（+ Roo 历史）

**文件**：
- Cline：读 `~/.cline/data/settings/providers.json`（明文，只读）。额度为个人与组织的余额（balances），以及 OpenRouter key 的 `/api/v1/key`（限额与剩余）。会话来自 `~/.cline/data/tasks/*`（**[装机]** 核实）。
- Kilo：读 `~/.local/share/kilo/auth.json`。额度为 profile 加每个组织的余额。会话来自 `kilo` 的数据目录；`serve` 的会话计入。
- Roo：只做历史，VS Code globalStorage `rooveterinaryinc.roo-cline/tasks`。`parentTaskId` 归入父会话。不做额度。
- 新增 endpoint 加入 `subscriptions.rs`；`spec`/`extract`、`quota_sources` 加入 cline/kilo；`import.rs`/`usage.rs`/`native_agents.rs` 增加解析器，并提升 `parser_version`。
- remote 脚本同步。

**测试**：解析 fixture；余额解析单测；`test-session-observers.py`。

**[装机]**：`npm i -g cline @kilocode/cli`，配置 OpenAI 兼容 provider 指向 Qwen 端点，各跑一次交互会话和一次 headless 会话，确认目录与分类。

**依赖**：WP11。**冲突**：核心链、会话链。

---

## WP13 新 agent：Kiro

**文件**：
- 识别：用 `sqlite_ro` 读 kiro-cli 的 `data.sqlite3` 中的 `auth_kv` 与 profile ARN；IDE 读 `~/.aws/sso/cache/kiro-auth-token.json`。取最新且未过期的 token，不刷新。
- 额度：`GetUsageLimits` 生成 meters（计划积分、超额、bonus）与重置日。只有在 token 过期时才兜底执行 `kiro-cli chat --no-interactive /usage`，带 `AIEYES_INTERNAL`，并把产生的会话写入内部会话表，同时限频（floor 300s）。
- 会话：kiro-cli 的会话和 IDE 的会话（**[装机]** 核实目录）。argv 规则见 WP1；`KIRO_API_KEY` 作为辅助信号。

**测试**：`GetUsageLimits` fixture；兜底命令的 env 与过滤；会话 fixture。

**[装机]**：`brew install --cask kiro` 以及 kiro-cli 安装脚本；未登录时只核实文件布局。

**依赖**：WP12。**冲突**：核心链、会话链。

---

## WP14 新 agent：Trae / Trae CN（+ trae-cli）

**文件**：
- **[装机]** 优先：安装 Trae 与 Trae CN，检查 Local Storage `iCubeAuthInfo` 或 `state.vscdb` 是否被 safeStorage 加密。
  - 未加密：只读 token，低频（≥600s）调用 `ide_user_ent_usage`，生成 meters。
  - 加密：只做会话与进程监控，并提供“打开官方用量页”链接（契约中的 `subscription.loginUrl`，使用 `subscriptions.openLogin`）。
- 会话：IDE 的会话存储（待核实）。trae-cli（`pip`/`uv` 安装 `trae-agent`）的 `run` 判为 headless，`interactive` 计入。
- 不写认证，不改机器 ID。

**测试**：两种分支各有 fixture；`test-subscriptions-ui.cjs` 加链接按钮用例。

**依赖**：WP13。**冲突**：核心链、会话链。

---

## WP15 WorkBuddy/CodeBuddy 可选“登录查看余额”（h2）

**目标**：默认关闭。用户主动点“登录查看余额（可选，使用非公开接口）”后，Aieyes 自持 token，读取套餐、免费包、付费余额以及 CodeBuddy 积分池。

**文件**：
- 新建 `crates/core/src/buddy_auth.rs`，提供 `buddyAuth.start`/`status`/`cancel`/`logout`。流程为浏览器登录加本地回调，或 device 流程（**[装机]** 抓包确认官方 Web 登录方式；无账号时只能实现到跳转与回调框架，接口解析用 fixture）。
- token 存到 `credentials.rs` 的 `app.aieyes.agents`（WP11 已泛化）。
- `subscriptions.rs` 增加 workbuddy/codebuddy 的 billing endpoint 与 parse，生成 meters 和 balances。与本地“本月已用”同卡显示。
- UI：Web 在 `agent-settings.js` 的 `detailsHTML` :381 中加按钮与对话框；Swift 在 `AgentSettingsViews.swift` 中加。文案必须说明“可选、非公开接口、可随时退出登录”。

**测试**：mock 登录流程；billing parse fixture；新增 `test-buddy-auth-ui.cjs`。

**依赖**：WP10、WP11（`credentials.rs`）、WP9（CodeBuddy）。**冲突**：`credentials.rs`、`subscriptions.rs`、设置 UI。

---

## WP16 文档、实测脚本与总门禁

**文件**：
- `docs/agents-and-accounts-settings.md`、`docs/codex-accounts.md`、`docs/agent-subscriptions.md`、`docs/native-agent-verification.md`、`docs/desktop-pets.md`。
- 每个 agent 一行“零配置来源 / 额度项 / 兜底 / 未实测项”。
- `scripts/verify-agent-integration-live.py`、`verify-agent-lifecycle-live.py`：增加新 agent，在已安装时跑，未安装时跳过。
- `crates/core/tests/agent_integration_audit.rs`：检查每个契约 provider 都在 `default_name`、`spec`、session root 等处有分支，防止漏项。
- 汇总装机核实结果，回填到调研文档的 (a)/(a2) 节。

**验证**：
```
scripts/ci-checks.sh && scripts/native-verifiers.sh && cd apps/desktop && npm run test:ui && python3 scripts/verify-agent-integration-live.py
```

**依赖**：全部。

---

## 风险与待定

1. ad-hoc 签名下的钥匙串“始终允许”：采用 `/usr/bin/security`，签名稳定，但授权范围更宽。文档说明这一点；长期方案是正式签名。
2. Copilot device-code 的 client_id：是否注册 Aieyes 自己的 GitHub OAuth App，需要维护者提供。没有的话 WP11 的按钮不发布（遵循“不设计无效按钮”）。
3. WP15 的登录接口没有账号无法实测，只能交付框架加 fixture，并默认隐藏在“可选”下。
4. Trae 与 Antigravity 仅装 IDE 的形态：零配置是否可行取决于装机结果；不可行时退化为会话监控加链接。

## 汇总

```json
[
 {"wp":"WP0","title":"契约骨架: 新provider/method/图标/文案","deps":[],"lane":"contract","install":false},
 {"wp":"WP1","title":"Headless分类器+argv+AIEYES_INTERNAL","deps":["WP0"],"lane":"sessions","install":true},
 {"wp":"WP2","title":"ChatGPT Work按originator独立显示","deps":["WP1"],"lane":"sessions","install":"optional"},
 {"wp":"WP3","title":"自动识别/去pending/容错/记住账号/upstream去重/sqlite_ro","deps":["WP0","WP5(model)"],"lane":"core","install":false},
 {"wp":"WP4","title":"去扫描UI与pending文案(Web+Swift)","deps":["WP3"],"lane":"ui","install":false},
 {"wp":"WP5","title":"额度模型: meters/snapshot_at/stale/notice","deps":["WP0"],"lane":"core+ui","install":false},
 {"wp":"WP6","title":"Claude extra_usage/limits + macOS首次启动钥匙串","deps":["WP3","WP5"],"lane":"core+ui","install":true},
 {"wp":"WP7","title":"Qoder后台定时(>=10min)+floor表+安装qodercli按钮","deps":["WP1","WP3"],"lane":"core+ui","install":true},
 {"wp":"WP8","title":"补缺A: ZCode/DeepSeek/Antigravity/Grok","deps":["WP3","WP5","WP6"],"lane":"core","install":true},
 {"wp":"WP9","title":"补缺B: OpenCode/Pi/Cursor/Kimi(+Work)/CodeBuddy IDE","deps":["WP8"],"lane":"core+ui","install":true},
 {"wp":"WP10","title":"WorkBuddy 会话+本月已用+身份","deps":["WP1","WP9"],"lane":"core+sessions","install":true},
 {"wp":"WP11","title":"Copilot 识别+额度+device-code+会话","deps":["WP7","WP10"],"lane":"core+sessions+ui","install":true},
 {"wp":"WP12","title":"Cline/Kilo(+Roo历史)","deps":["WP11"],"lane":"core+sessions","install":true},
 {"wp":"WP13","title":"Kiro","deps":["WP12"],"lane":"core+sessions","install":true},
 {"wp":"WP14","title":"Trae/Trae CN/trae-cli","deps":["WP13"],"lane":"core+sessions+ui","install":true},
 {"wp":"WP15","title":"WorkBuddy/CodeBuddy 可选登录查看余额","deps":["WP9","WP10","WP11"],"lane":"core+ui","install":true},
 {"wp":"WP16","title":"文档/实测脚本/总门禁","deps":["ALL"],"lane":"docs","install":true}
]
```
