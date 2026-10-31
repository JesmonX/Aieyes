# Agent 登录、多账号、额度与 Headless 调研（2026-10-29）

范围：
1. Aieyes 已支持的全部 agent：Codex、Claude Code、Antigravity、Qoder、Kimi Code、ZCode、DeepSeek Harness、Cursor、Grok、OpenCode、Pi/omp、CodeBuddy（第 1–12 节）。
2. 已支持 agent 中尚未覆盖的形态：Codex Cloud 与 ChatGPT 桌面端 Work 模式、Claude Desktop/Web、Antigravity IDE、Qoder IDE/QoderWork/qoder.cn、Kimi Work、Cursor IDE、OpenCode Console、CodeBuddy IDE 插件等（并入各节“缺口”）。**不含 Grok Web。**
3. 新增调研的 agent：GitHub Copilot、AWS Kiro、Trae / Trae CN、Cline / Roo Code / Kilo Code、Tencent WorkBuddy（第 13–17 节）。

标记说明：
- **[已实现]**：仓库代码里已有。
- **[已验证]**：官方文档、本机安装包源码或本机只读检查能证实。
- **[未验证]**：来自社区逆向或模型知识，本轮没有复核。

本轮对 7 个关键疑点做了联网抽查（Claude extra_usage、DeepSeek get_user_summary、Qoder token、Kimi 额度、Cursor 端点、ChatGPT Work/Codex、Antigravity credits），来源均为搜索摘要与社区逆向，结果已回填到各节，标为 [已验证:社区] 的仍需实测。其余 [未验证] 项落地前都要先跑一次实测。

2026-10-29 第二轮补全（本机只读检查：`--help`、JS bundle/二进制 strings、本地会话文件与 DB schema、`app.asar` 源码；未登录、未安装、未打印密钥）：Trae 第 15 节重写；各 agent 的 Headless 行与 (c) 规则表补齐；DeepSeek 微信登录、Qoder token 落盘、Antigravity credits 端点、ChatGPT Work 与 Codex 的区分已定位。**总体结论：除 Claude、Codex、ZCode、WorkBuddy 外，其余 agent 的本地会话文件都没有显式的 headless 字段，只能靠进程 argv（含 hook 父进程 argv）判定；进程退出后的历史会话无法事后判定。**

---

## (a) 各 Agent 现状与缺口

### 1. Codex（基准）
| 维度 | 内容 |
|---|---|
| 登录 | ChatGPT OAuth（浏览器或 `--device-auth`）；API key |
| 存储 | `$CODEX_HOME/auth.json`；keyring 后端 `cli_auth_credentials_store` [未验证] |
| 切换 | 每个账号一个 CODEX_HOME 目录 [已实现]，可同时运行多个账号 |
| 额度 | 5h 和周窗口、`rateLimitsByLimitId`、credits [已实现：解析]；按 limitId 分桶在 UI 上是否展示 [未验证]；Cloud 任务额度：无本地来源 |
| Headless | `session_meta.payload.source`：`exec` 和 `mcp` 排除；`cli` 和 `vscode` 计入 [已实现]；`source` 为 `{"subagent":…}` 或 `thread_source=guardian_review` 的归入父会话或排除 [已验证:本机] |
| ChatGPT 桌面端 | 2026-07 起 Codex 并入 ChatGPT 桌面端，分 Chat/Work/Codex 三种模式；**Work 与 Codex 共用额度和 credits，Chat 单独计算** [已验证:OpenAI 定价与帮助文档]，额度层面不必区分。`/Applications/ChatGPT.app` 的 bundle id 就是 `com.openai.codex`（26.1023.50603），内带 `Resources/codex-cli/bin/codex`，数据目录 `~/Library/Application Support/Codex` [已验证:本机]。**会话归类可以区分** [已验证:app.asar 源码 + 本机 `~/.codex/state_5.sqlite`]：新建本地线程时 Work 模式写 `originator=codex_work_desktop`，Codex 模式为 `Codex Desktop`（`CODEX_INTERNAL_ORIGINATOR_OVERRIDE`）；两者 `source` 都是 `vscode`，**不能只看 source**。辅助信号：Work 会话 cwd 在 `~/Documents/Codex/<日期>/<slug>`，`cli_version` 带 `-alpha` 后缀，另有 `models_cache.codex_work_desktop.json`。本机统计：codex_work_desktop/vscode 36、Codex Desktop/vscode 18、Codex Desktop/cli 3、codex-tui/cli 256、codex-tui/vscode 189、codex_exec/exec 1225、guardian 子代理 151 |
| 缺口 | ① Cloud 任务未导入 ② source 枚举没有定期复核 ③ originator 未用于区分 ChatGPT Work（规则已明确：`codex_work_desktop`，只影响会话归类）④ API-key 档案没有余额（OpenAI 没有公开余额 API）⑤ 189 条 `codex-tui`+`vscode` 来自 VS Code 扩展还是旧版桌面端未区分 [未验证]；网页版 ChatGPT agent 不写 `~/.codex`，本地不可见 |

### 2. Claude Code
| 维度 | 内容 |
|---|---|
| 登录 | Claude.ai OAuth；`setup-token`（CLAUDE_CODE_OAUTH_TOKEN）；Console API key 或 apiKeyHelper；Bedrock；Vertex |
| 存储 | macOS 用 Keychain `Claude Code-credentials`；Linux/Windows 用 `~/.claude/.credentials.json`；账号元数据在 `~/.claude.json` 的 oauthAccount |
| 切换可行性 | Linux/Windows：每个账号一个 CLAUDE_CONFIG_DIR，可行。macOS：Keychain 条目是否按 config dir 加后缀 [未验证]；如果不加后缀，只能在没有 claude 进程运行时整体替换 Keychain 条目和 oauthAccount |
| 额度 | `/api/oauth/usage`：five_hour、seven_day、seven_day_sonnet、seven_day_opus、seven_day_oauth_apps [已实现]；`extra_usage`（is_enabled、utilization、used_credits（美分，除以 100）、monthly_limit（推测也是美分））未解析 [已验证:社区 issue，非官方]；同一响应另有 `limits[]`，`kind=="weekly_scoped"` 条目带按模型周限额（scope.model.display_name、percent、resets_at），与 seven_day_opus 等可能重叠或为新格式 [需实测]；Console 预付余额没有 API；Bedrock/Vertex 不覆盖 |
| Headless | `entrypoint` 为 sdk-cli、sdk-ts、sdk-py、mcp、claude-code-github-action 时排除 [已实现]；Desktop 和 Web 的 entrypoint 值 [未验证] |
| 缺口 | ① canSwitch 只对 codex 开放（account_targets.rs:118）② 没有托管 profile 和登录流程 ③ extra_usage 未接入 ExtraUsageWallet ④ CLAUDE_CODE_OAUTH_TOKEN 账号未识别 |

### 3. Antigravity（agy / IDE）
| 维度 | 内容 |
|---|---|
| 登录 | Google OAuth（agy 浏览器流程加粘贴 code）；IDE 内 Google 登录 |
| 存储 | agy 根目录下的 `antigravity-oauth-token`；数据目录 `~/.gemini/antigravity-cli/`（`conversations/<uuid>.db`、`history.jsonl`、`log/`、`cli.log`）[已验证:本机]；IDE 存储位置 [未验证] |
| 切换可行性 | 没有切换命令 [未验证]。可以替换 token 文件，或用隔离 HOME（agy_auth.rs 登录时已经这么做）。provider_paths 没有 config-dir 环境变量，同时运行多账号需要隔离 HOME |
| 额度 | agy `/usage`：按模型组（Gemini 和 Claude/GPT）的 5h 和周窗口 [已实现]；付费额度每 5h 刷新且受周上限约束（Ultra 是否有周上限有争议）[已验证:Google blog]；AI credits 超额机制（overage Always/Never，仅 Pro/Ultra 可购）[已验证]。**端点已定位** [已验证:agy 二进制 protobuf 描述符 + 本机日志，主机 `daily-cloudcode-pa.googleapis.com`]：① 额度窗口 `POST v1internal:retrieveUserQuotaSummary`（请求 `{project}`）→ `buckets[]/groups[]`，bucket 含 `bucketId、displayName、window、disabled、resetTime`、`remainingFraction|remainingAmount`；② **积分余额**在 `v1internal:loadCodeAssist` 响应的 `currentTier/paidTier.availableCredits[]`：`{creditType: GOOGLE_ONE_AI, creditAmount(int64), minimumCreditAmountForUsage}`；③ 每次调用扣减看 `streamGenerateContent` 响应的 `consumedCredits/remainingCredits`（请求带 `enabledCreditTypes[]`，本地开关 `enable_ai_credits/use_ai_credits`）；④ `OverageInfo{enabled}` 只出现在企业版 `businessaicode.googleapis.com` 的 `FetchQuotaStatus`。agy 用 `quota_manager.go doRefreshQuota` 节流刷新，`authMethod=consumer`。`bucketId/window` 实际取值、`creditAmount` 单位、正式版 `cloudcode-pa` 与 `daily-` 是否一致、额外请求头 [未验证]；Windsurf 时代 `availablePromptCredits` 等旧字段是否还有值 [未验证]；注意 Claude 4.6 系列 2026-11-02 从 Antigravity 下线 |
| Headless | argv：`-p`、`--print`、`--prompt`（`--print` 别名）、`--output-format json|stream-json`、`--input-format stream-json`、`--print-timeout`、`--json-schema`、`--disable-slash-commands` → headless；`-i/--prompt-interactive` 算交互；`agy remote-control`（daemon）和 `mic-serve` 不算会话 [已验证:`agy --help`]。`conversations/<uuid>.db` 的 `trajectory_meta(trajectory_type, source)` 本机 25 个交互会话全是 `4/17`，`-p` 下取值 [未验证，需跑一次对照]；`history.jsonl` 只有 `display/timestamp/workspace/type`，print 模式是否写入 [未验证]（若不写，“conversations 有、history 无”可作辅助信号 [推断]）；glog 日志无 print/headless 字样 [已验证]；hook（`~/.gemini/config/hooks.json`：SessionStart/PreInvocation/PostInvocation/PostToolUse/Stop）payload 是否带 mode [未验证]，建议 hook 读父进程 argv；子轨迹看 `steps.has_subtrajectory` 与 `parent_references` 表 [schema 已验证，语义未验证]；旧版 `gemini -p` 只能靠 argv |
| 缺口 | 切换、headless 过滤、IDE 原生历史 |

### 4. Qoder（国际版与国内版，CLI、IDE、QoderWork）
| 维度 | 内容 |
|---|---|
| 登录 | qodercli 浏览器登录（token 每 30 分钟后台刷新）；PAT：`/login` 粘贴或 `QODER_PERSONAL_ACCESS_TOKEN`（qoder.com/account/integrations 生成），`/login` 拿到的优先于环境变量；国内版 `qoderclicn` + `QODERCN_PERSONAL_ACCESS_TOKEN` [已验证:docs.qoder.com]；IDE/QoderWork 登录 |
| 存储 | QODER_CONFIG_DIR（默认 `~/.qoder`；另有 `QODER_USER_CONFIG_DIR`、`QODER_SITE`、`QODER_PAT`、`QODER_ENV_FILE`）[已验证:qodercli-1.1.79 strings]。**token 在 `~/.qoder/.auth/user`**：0600、约 1.4KB、base64 样式密文，不是 JSON，不在 Keychain [已验证:本机]；同目录 `machine_id`（UUID）和 `.credential-transaction` 锁；刷新先写 `user.<pid>.<ts>.<uuid>.tmp` 再原子 rename（扫描时忽略 tmp）[已验证]；密钥可能由 machine_id 派生 [推测]。`state.json.lastLoginMethod` 区分 PAT/浏览器 [已验证:键名]。国内版 `qoderclicn` 目录未确认 [未验证]。Qoder 桌面 App（`com.qoder.app` 0.4.8，Electron）：`~/Library/Application Support/com.qoder.app.stable/auth.v1.dat`、`auth-profile-overlays.v1.dat`（`v10` 头，safeStorage，Keychain “Qoder App Safe Storage”/“Qoder App Key”）、`auth.machine-id` [已验证:本机]。Qoder IDE（VS Code 分支）未装，按惯例在 `state.vscdb` secret + “Qoder Safe Storage” [未验证] |
| 切换可行性 | **高**：每个账号一个 QODER_CONFIG_DIR（只有环境变量，无 `--config-dir` 参数 [已验证:strings]），且已经在按目录查询额度；复制目录时 `.auth/user` 要连同 `machine_id` 一起 [未验证]；PAT 环境变量可做进程级多账号；桌面 App 可按账号快照 `auth*.dat` + `auth.machine-id`，退出后替换（同机有效）[未验证] |
| 额度 | 月度套餐 Credits、加购包、组织共享池 [已实现；付费和组织样本未验证]；qoder.cn 新体系未实现 |
| Headless | 本机全部 3395 条 entrypoint 都是 `cli`，用这个字段区分不了 [已验证：本机]；要靠 argv（`-p`、`--output-format`、`stream-json`）或 sessionstart-hook。**Aieyes 自己发起的 get_usage_info 控制会话必须排除** |
| 缺口 | 切换流程、qoder.cn API、headless 过滤、IDE 凭据 |

### 5. Kimi Code
| 维度 | 内容 |
|---|---|
| 登录 | OAuth（kimi.com / moonshot.cn）；config.toml 中 provider 的 api_key |
| 存储 | `KIMI_CODE_HOME/oauth/*.json`、`credentials/*.json` [文件名未验证]；区域由 `~/.kimi-code/region` 决定（国际 host `api.kimi.ai`）[已验证]；刷新 `POST auth.kimi.com/api/oauth/token` 会轮换 refresh token，**Aieyes 不得自行刷新** [已验证:社区 PR] |
| 切换可行性 | **高**：每个账号一个 KIMI_CODE_HOME |
| 额度 | `GET api.kimi.com/coding/v1/usages`：`limit_5h`、`limit_7d`、`limit_month_total`、`limit_month_code`（各带 used_ratio、reset_time）、`boosterWallet`；无 managed 视图时返回通用 `limits[]`；本地 Server API `GET /api/v1/oauth/usage` 有官方文档 [已验证]。managed-usage 周期窗口、boosterWallet [已实现；付费账号未验证]；Moonshot 平台余额 `/v1/users/me/balance` 未实现 |
| Headless | argv：`-p/--prompt`（配 `--output-format`）、子命令 `acp`（stdio ACP server）、`web`、`rc|remote` 判为非交互；`server` 已废弃 [已验证:`kimi --help` v2.2.0]。bundle 里有 `uiMode: promptMode ? "print" : "shell"`，但只进遥测，不写 `state.json`/`wire.jsonl` [已验证:dist/main.mjs]；`SessionStart` hook 输入为 `{source, sessionTitle, model, profile, cwd, sessionId}`，**不带 ui_mode**，hook 本身分不出 `-p`，需读父进程 argv [已验证]。本机无 `~/.kimi-code` 样本 |
| 缺口 | 切换、headless、Kimi Work、平台余额；本机未安装 |

### 6. ZCode（Z.ai / 智谱 GLM Coding Plan）
| 维度 | 内容 |
|---|---|
| 登录 | `zcode login zai|bigmodel`（OAuth）；`/login *-coding-plan-api-key` |
| 存储 | `${ZCODE_DATA_BASE_DIR:-$HOME}/.zcode/v2/credentials.json`（zcodeJwtToken、zaiUserInfo）[已验证：bundle 源码]；API key 在 `provider_config.json` 的 providerRules[].config.access.apiKey |
| 切换可行性 | 每个账号一个 ZCODE_DATA_BASE_DIR，或给每个进程设置 ZCODE_PERSONAL_PROVIDER_CONFIG_FILE。会话 DB `~/.zcode/cli/db` 被所有账号共用，且没有账号 ID |
| 额度 | `/api/monitor/usage/quota/limit`（z.ai 和 bigmodel）：5h、周、MCP 月度 [已实现：API key]；**用 OAuth JWT 能否调用 [未验证]**；model-usage 和 tool-usage [未验证]；按量余额没有公开 API |
| Headless | `-p/--prompt`、`--target`、`app-server`。DB 的 `session.task_type` 有 interactive、fork、selection_side_chat、workflow_*、subagent_child、nested_workflow_child；`parent_id` 表示子会话 [已验证：schema]。`-p` 是否会写入不同的 task_type [未验证] |
| 缺口 | ① 没导入 OAuth JWT ② **导入器没有 join task_type/parent_id，子会话会被计成会话** ③ 区域需要手选，可以按登录 provider 推断 ④ 没有切换 |

### 7. DeepSeek Harness（dsh）与 DeepSeek API
| 维度 | 内容 |
|---|---|
| 登录 | Platform 网页 `platform.deepseek.com/sign_in` 支持 +86 手机号短信验证码（自动注册）和**微信扫码**，`/sign_up` 可邮箱注册；不同方式注册的是独立账号，需手动绑定 [已验证:第三方教程，非官方]。dsh 本身不实现任何登录方式（包内无 wechat/qrcode/sms 字符串），统一走“系统浏览器 → `/dsh/authorize` → 本地 `/oauth/callback` → S256 PKCE `auth_init`/`auth_exchange`” [已验证:`@deepseek-ai/dsh@0.2.1-rc.2` 源码]，授权页继承网页登录方式，所以微信扫码应可用 [未验证：需登录实测]；API key（DEEPSEEK_API_KEY、.env、凭据库 `refs.DEEPSEEK_API_KEY`） |
| 存储 | `$DSH_HOME/.credentials.yaml`：`records.client-connection/browser-session`、`deepseek-account-platform/device`（随机 UUID）、`deepseek-account-platform/default{version, token, issuer}`、`refs.DEEPSEEK_API_KEY` [已验证：本机键名和 README]；token 无过期字段，401 或 40003 时清除并发 `deepseek-account/session-expired`；`issuer` 与 platformOrigin 不一致时删授权保留 key 和 device；桌面端数据 `~/Library/Application Support/@deepseek-ai/dsh-desktop` |
| 切换可行性 | 每个账号一个 DSH_HOME（各自保留 device id），或替换 `deepseek-account-platform/default` 记录；README 写明凭据变化会让进行中的查询失效、bonus 不会误确认到另一账号，说明 dsh 监听凭据变化 [已验证:README]；运行中进程能否热加载 [未验证] |
| 额度 | `api.deepseek.com/user/balance`（总额、赠送、充值）[已验证、已实现]；账号登录余额 `GET platform.deepseek.com/api/v0/users/get_user_summary` [已验证:社区，非官方]：Platform 网页用 `Authorization: Bearer <userToken>`；**DSH 桌面账号用 `x-dsh-auth-token: <token>`（无 Bearer）加 x-client-platform/version/locale/timezone-offset/bundle-id 五个头**；API key 不能调；另有 `/api/v0/usage/by_api_key/amount`、`/cost`（start、end、tz）；get_unnotified_bonuses |
| Headless | `dsh headless "<task>"` 或 `--profile headless`（完成即退出；`--json` 输出 NDJSON，`--session-id` 沿用持久化会话，`-` 读 stdin）[已验证:help]；交互入口是 `dsh tui`、`dsh web`、desktop。会话文件 `~/.dsh/sessions/<cwd编码>/session-<uuid>/session.v4.jsonl.zstd`，首行 `{type:session, version:4, id, createdAt, cwd, isSeeded, delegationDepth, agentPreset}`，**没有记录启动 profile 的字段** [已验证:本机 6 个会话，无 headless 样本]；`delegationDepth>0` 是子代理，不等于无头。headless 是否写入同一目录、`agentPreset`/`permission/preset` 有无特征值 [未验证]。唯一可靠信号是 argv，pid 绑定 `session.lock` 所在目录 |
| 缺口 | ① 没读账号 grant，也不调用 get_user_summary ② .credentials.yaml 和 .env 里的 key 没导入 ③ headless/sdk/acp 没过滤 ④ 没有切换和身份绑定 |

### 8. Cursor（IDE 与 cursor-agent）
| 维度 | 内容 |
|---|---|
| 登录 | IDE 浏览器登录（WorkOS、SSO）；cursor-agent login；CURSOR_API_KEY；团队 Admin API key |
| 存储 | IDE：`state.vscdb` 中的 cursorAuth/accessToken、refreshToken、cachedEmail、stripeMembershipType [键名仍未知，未验证]；CLI：Keychain cursor-access-token [未验证]；CURSOR_CONFIG_DIR |
| 切换可行性 | IDE 只能分开用 `--user-data-dir`，在关闭状态下改 sqlite 很脆弱。CLI 每个进程用一个 API key 最稳 |
| 额度 | 旧 `GET cursor.com/api/usage-summary`（WorkosCursorSessionToken Cookie；个人池、个人上限、团队池）[已实现，未实测]，可能返回过期月周期和 0%；新 `POST cursor.com/api/dashboard/get-current-period-usage`（body `{}`，给出周窗口），等价 `api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage`（Bearer）[已验证:社区]；可能 400 "Usage summary is not enabled"。**建议新端点优先、旧端点回退，`billingCycleStart` 按字符串 epoch ms 容错**；旧版 500 次快速请求、Admin API spend、会员类型 未实现 |
| Headless | `cursor-agent -p/--print`（配 `--output-format text|json|stream-json`、`--stream-partial-output`，常带 `-f/--force/--yolo`、`--trust`、`--approve-mcps`）[已验证:help 2026.10.16]。hook payload **确认没有交互标志** [已验证:bundle]：`sessionStart` 为 `{conversation_id, generation_id, model, is_background_agent:false, composer_mode}`（只在新会话触发），`sessionEnd` 加 `reason, duration_ms, final_status`；bundle 内 `isHeadless/is_headless` 只进遥测。所以 hook 进程需读父进程（`cursor-agent`/`agent`）argv 查 `-p/--print`；`is_background_agent:true`（带 `session_id/repo_urls/repos`）是云端代理，排除；IDE agent 无 print 模式，`composer_mode` 只区分 agent/ask。本机 `~/.cursor` 无 chats/projects，CLI transcript 有无标记 [未验证] |
| 缺口 | 没从 state.vscdb 自动导入、没有切换、没有 headless 过滤、Grok 共享池没去重、未切到新用量端点 |

### 9. Grok CLI
| 维度 | 内容 |
|---|---|
| 登录 | `grok login`（OIDC）；XAI_API_KEY；Management key |
| 存储 | `GROK_HOME/auth.json`，可存多个 OIDC 条目 |
| 切换可行性 | 每个账号一个 GROK_HOME [是否所有路径都遵守未验证]；有多个条目时目前要求用户手动选 |
| 额度 | `cli-chat-proxy.grok.com/v1/billing?format=credits` [已实现，私有协议]；API 预付余额 [已实现，官方]；postpaid 消费上限未实现；滚动速率限制（grok.com/rest/rate-limits）[未验证] |
| Headless | `-p` [未验证]；目前不分类 |
| 缺口 | headless、按 GROK_HOME 切换、导入只是一次性快照 |

### 10. OpenCode
| 维度 | 内容 |
|---|---|
| 登录 | `/connect` 或 `auth login`，按 provider 走 API key 或 OAuth（Anthropic、Copilot、ChatGPT、Zen/Go）；Console 登录；环境变量 |
| 存储 | 旧版 `~/.local/share/opencode/auth.json`；新版 opencode.db 的 `credential`（label、active）、`account`/`control_account`、`account_state`（active_account_id、active_org_id）[已验证：仅列名] |
| 切换可行性 | credential.active 加 label 看起来原生支持多凭据切换 [语义未验证]；XDG_DATA_HOME 隔离 |
| 额度 | Go 的 5h、周、月 [已实现]；Zen 余额 [已实现]，自动充值和月限额 [未验证]；通过 OpenCode OAuth 查询上游（Claude、Copilot、Codex）额度 未实现 |
| Headless | `opencode run`（`--format json`、`--auto`、`-s`、`--fork`…）、`acp`、`api`、用户直接执行的 `serve` 判为非交互 [已验证:help 2.1.0]。**陷阱：2.x 交互 TUI 默认会拉起 `serve --service`，它不是无头信号** [已验证:`~/.local/share/opencode/log/opencode.log` 的 `cli starting … args=` 行]。opencode.db 的 `session`/`session_v2` **没有 source/client 字段**，`metadata` 本机全 NULL [已验证:PRAGMA]；`parent_id` 表示子代理。`time_viewed IS NULL` 可能代表 run/acp 创建的会话 [推测，样本不足]；日志 `created id=ses_…` 与 `cli starting` 行的关联方式 [未验证] |
| 缺口 | headless 过滤、没读新版 DB 的凭据、没有切换、没复用上游额度 |

### 11. Pi / Oh My Pi
| 维度 | 内容 |
|---|---|
| 登录 | `/login` OAuth（Claude、ChatGPT、Copilot、Gemini）加 API key |
| 存储 | `~/.pi/agent/auth.json`（本机为空）；`~/.omp/agent`；覆盖目录的环境变量 PI_CODING_AGENT_DIR [未验证] |
| 切换可行性 | 换 agent 目录，或用 omp profile |
| 额度 | 本身没有额度；可以用 Pi 存的 OAuth 复用 Claude 和 Codex 的额度代码 [未实现] |
| Headless | `-p`、`--mode json|rpc`、SDK；`--no-session` 时不写文件 [未验证] |
| 缺口 | headless、身份、复用上游额度 |

### 12. CodeBuddy
| 维度 | 内容 |
|---|---|
| 登录 | Web/OAuth（国际版和国内版，含微信、QQ、企业微信、短信、SSO/iOA，见第 17 节 WorkBuddy 的 asar 字符串）[已验证:本机 WorkBuddy asar]；CODEBUDDY_API_KEY、AUTH_TOKEN、BASE_URL |
| 存储 | 本机没有明文 token 文件；可能在 Keychain，或在 `local_storage/entry_*.info` 的 gzip blob 里 [未验证] |
| 切换可行性 | 每个账号一个 CODEBUDDY_CONFIG_DIR（前提是 token 存在目录里）[未验证]；API key 用环境变量即时切换 |
| 额度 | 只有日志里的单次 credits [已实现]；与 WorkBuddy 共用积分池，接口为 `billing/meter/get-user-resource*`（见第 17 节）[端点已验证，字段未验证] |
| Headless | `-p`、SDK、ACP；是否走 Claude 风格的 entrypoint 过滤 [未确认] |
| 缺口 | 额度接口、登录读取、切换、确认 headless 过滤 |

---

## (a2) 新增 Agent（尚未支持）

### 13. GitHub Copilot（VS Code 扩展 / Copilot CLI / SDK）
> 本机只读检查：已装 VS Code 的 `github.copilot-chat`，存在 `~/.copilot`；PATH 中没有 `copilot` 和 `gh`。

**形态**
| 形态 | 说明 | 标记 |
|---|---|---|
| VS Code 扩展 | `github.copilot-chat`（已合并补全），数据在 `~/Library/Application Support/Code/User/globalStorage/github.copilot-chat` | [已验证:本机] |
| Copilot CLI（`copilot`） | npm 或 brew 安装，新版带 Rust 原生 runtime；VS Code 扩展自带 shim `globalStorage/github.copilot-chat/copilotCli/copilot` | [已验证:本机] |
| JetBrains / VS / Xcode / Eclipse 插件 | 旧插件 OAuth 写 `~/.config/github-copilot/apps.json`（旧 `hosts.json`），本机没有 | [未验证:社区逆向] |
| Copilot SDK | JSON-RPC，提供 `account.getQuota` | [已验证:docs] |
| 独立桌面 App | 没查到 | [未验证] |

**登录**
| 方式 | 细节 | 标记 |
|---|---|---|
| OAuth 浏览器 | `copilot login` 或 `/login`，loopback 回调，桌面默认 | [已验证:docs] |
| OAuth 设备码 | `--device-code`；SSH、Codespaces、CI 下默认 | [已验证:docs] |
| GHE.com 数据驻留 | `copilot login --host xxx.ghe.com` | [已验证:docs] |
| 环境变量 | `COPILOT_GITHUB_TOKEN` > `GH_TOKEN` > `GITHUB_TOKEN` > keychain OAuth > `gh auth token`；环境变量会静默覆盖已存 OAuth | [已验证:docs] |
| Token 类型 | 支持 `gho_`、`github_pat_`（需 "Copilot Requests" 权限）、`ghu_`；不支持经典 `ghp_` | [已验证:docs] |
| BYOK / `COPILOT_OFFLINE=true` | 不需要登录 GitHub | [已验证:docs] |
| VS Code | 内置 GitHub Authentication Provider，可登多个账户但扩展同时只用一个 | [未验证] |

**凭据存储与刷新**
- CLI 存 macOS 钥匙串 service **`copilot-cli`**；无 keychain 时询问是否明文写 `~/.copilot/config.json` [已验证:docs]。本机查不到该项（未登录）；config.json 是 JSONC，只有 `firstLaunchAt` [已验证:本机]。
- 长期 `gho_` 不过期；调用前换短期 Copilot token：`GET api.github.com/copilot_internal/v2/token`（约 30 分钟，带 `refresh_in`）[未验证:社区逆向]。
- VS Code 会话在 SecretStorage（"Code Safe Storage" 加密，密文在 `state.vscdb`），Aieyes 不宜直接读 [未验证]。

**账户切换**
- CLI 自带多账户：`/user list`、`/user switch`，再 `copilot login` 即新增 [已验证:docs]。
- `COPILOT_HOME` 换配置目录 [已验证:docs]；但 keychain service 固定，目录间是否共享账户列表不清楚 [未验证]。
- **进程级切换**：每个进程注入不同 `COPILOT_GITHUB_TOKEN`，可并发多账户，是即时切换 + 并发的最可靠办法 [已验证:优先级规则]。
- VS Code：每窗口一个账户，切换要手动 UI [未验证]。

**额度**：2026-06-01 起按用量计费，premium requests 被 **GitHub AI Credits** 取代（1 credit = $0.01，按 token 计价）[已验证:github.blog]。补全仍不限量；Pro/Pro+ $10/$39 对应等额额度；Business 每人 1,900、Enterprise 每人 3,900 credits；年付 Pro/Pro+ 合约内仍按旧 PRU；组织默认允许超额，用完可能直接停 [已验证]。个人方案 credits 数量第三方说法矛盾 [未验证]。

| 项目 | 端点与认证 | 字段 | 标记 |
|---|---|---|---|
| 用户配额快照（未公开） | `GET https://api.github.com/copilot_internal/user`，`Authorization: token <gho_/github_pat_>` + `Editor-Version`、`Editor-Plugin-Version` | `copilot_plan`、`access_type_sku`、`quota_reset_date_utc`、`token_based_billing`；`quota_snapshots.{chat,completions,premium_interactions}` 下 `entitlement`、`remaining`（可负）、`quota_remaining`、`percent_remaining`、`unlimited`、`overage_permitted`、`overage_count`、`has_quota`、`credits_used` | [已验证:社区 issue] |
| 用量计算 | `unlimited:false`：已用 = `entitlement - quota_remaining`；`unlimited:true`：用 `premium_interactions.credits_used` | — | [已验证:社区 issue] |
| SDK RPC | `account.getQuota` → `quotaSnapshots`：`entitlementRequests`（-1 不限）、`usedRequests`、`remainingPercentage`、`resetDate` | — | [已验证:docs] |
| 会话内用量 | CLI `/usage` | 本会话 AI Credits 与各模型 token | [已验证:docs] |
| 组织/企业预算 | REST billing usage（需管理员） | — | [未验证] |

**本地会话**
- CLI：`~/.copilot/logs/process-<epoch_ms>-<pid>.log` [已验证:本机 13 个]；会话事件 `~/.copilot/session-state/<uuid>/events.jsonl`（旧 `history-session-state/*.json`），本机还没有 [未验证]；`~/.copilot/ide/<uuid>.lock` 表示 CLI 连着 IDE [已验证:本机]。
- VS Code 聊天：`workspaceStorage/<hash>/chatSessions/<uuid>.jsonl` [已验证:本机]。
- 内嵌 CLI 会话库：`globalStorage/github.copilot-chat/session-store.db`（SQLite + WAL），表 `sessions`（含 **host_type**、cwd、repository、branch、agent_name…）、`turns`、`checkpoints`、`session_files`、`session_refs`、FTS `search_index` [已验证:本机 schema]；本机暂无会话。

**Headless 信号**：argv 含 `-p/--prompt`（常配 `--allow-all-tools`、`--agent`）、`--acp`、JSON-RPC server 模式 [已验证:docs]；`~/.copilot/logs/process-*.log` 出现 "Standalone JSON-RPC … Standalone server does not support host call" 表示 standalone/SDK 宿主模式 [已验证:本机日志；作为 headless 信号属推断]；`-p` 模式的日志特征、`events.jsonl` 的 `session.start` 是否带 `producer` [未验证]；非 TTY 或有 `CI`/`GITHUB_ACTIONS` [未验证]；认证来自环境变量 [未验证]；`sessions.host_type` 值待核对 [未验证]；云端 coding agent 跑在 Actions 上，无本地日志 [已验证]。

**Aieyes 建议**
1. 账户层：每账户一份 token 存 Aieyes 自己的 keychain 项；查额度直接调 `copilot_internal/user`，启动 CLI 注入 `COPILOT_GITHUB_TOKEN`。无需换文件即可即时切换和并发。遇到 `ghp_` 提示。
2. 额度层：展示 `premium_interactions`（AI Credits）已用/剩余、`quota_reset_date_utc` 倒计时、`overage_permitted`，处理 `unlimited` 和负 remaining；`chat`/`completions` 只在 Free 显示；`token_based_billing` 作新计费开关。轮询 ≥5 分钟；接口未公开，备选 SDK `account.getQuota`。
3. 会话层：`session-state/*/events.jsonl`（待样本）、`logs`（进程存活）、VS Code `chatSessions/*.jsonl` + `session-store.db`（只读带 WAL）；live 用 `ide/*.lock` + pid。
4. 排除 `-p/--prompt`、无 TTY、CI 环境的会话。

来源：[Authenticate Copilot CLI](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli) · [Use Copilot CLI](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/use-copilot-cli) · [Copilot SDK usage and billing](https://docs.github.com/en/copilot/how-tos/copilot-sdk/features/usage-and-billing) · [GitHub Blog: usage-based billing](https://github.blog/news-insights/company-news/github-copilot-is-moving-to-usage-based-billing/) · [tokscale#912](https://github.com/junhoyeo/tokscale/issues/912) · [opencode-quota#300](https://github.com/slkiser/opencode-quota/issues/300) · [copilot-powerline#55](https://github.com/xpepper/copilot-powerline/issues/55) · [opencode-statusline-plugins#138](https://github.com/lnwu/opencode-statusline-plugins/issues/138) · [Visual Studio Magazine](https://visualstudiomagazine.com/articles/2026/04/27/devs-sound-off-on-usage-based-copilot-pricing-change-you-will-get-less-but-pay-the-same-price.aspx)

### 14. AWS Kiro（Kiro IDE / kiro-cli / Kiro Web）
> 本机**没有安装**任何 Kiro 组件（~/.kiro、~/.aws/sso/cache、~/Library/Application Support/kiro-cli 均不存在，PATH 无 kiro-cli/kiro/q）[已验证:本机]。本地文件结论来自官方文档和第三方开源实现。jwadow/kiro-gateway README 抓取失败，IDE token 字段与刷新端点只有搜索摘要。

**形态**：Kiro IDE（VS Code 系，GovCloud 最低 0.9.2）；kiro-cli（Amazon Q Developer CLI 后继，GovCloud 最低 1.25.0，旧 Q CLI 数据库 `~/.local/share/amazon-q/data.sqlite3`）；Kiro Web/Mobile/app.kiro.dev（API Key 在此生成）[已验证:kiro.dev]。

**登录**
| 方式 | IDE | CLI | Web | Device flow | 备注 |
|---|---|---|---|---|---|
| GitHub（kirodotdev） | ✓ | ✓ | ✓ | ✓ | 个人订阅 [已验证] |
| Google | ✓ | ✓ | ✓ | ✓ | 个人订阅 [已验证] |
| AWS Builder ID | ✓ | ✓ | ✓ | ✓ | 个人订阅 [已验证] |
| IAM Identity Center | ✓ | ✓ | ✓ | ✓ | 需 Start URL 和 Region [已验证] |
| 外部 IdP（Okta、Entra、OIDC） | ✓ | ✓ | ✓ | ✗ | [已验证] |
| API Key `ksk_...`（`KIRO_API_KEY`） | — | ✓ | — | n/a | Pro 及以上；仅非交互；扣订阅 credits [已验证] |

- GovCloud 只支持 IdC 和外部 IdP（Start URL 含 `us-gov-home`）[已验证]。API 区域由 profile ARN 决定：us-east-1 → `codewhisperer.us-east-1.amazonaws.com`，eu-central-1 → `q.eu-central-1.amazonaws.com` [已验证:CodexBar]。
- CLI：`kiro-cli login`、`kiro-cli whoami`；凭据优先级 已登录会话 > KIRO_API_KEY > 提示登录 [已验证]；logout 命令名 [未验证]。

**凭据存储**
| 项 | 路径/格式 | 标注 |
|---|---|---|
| CLI DB（macOS） | `~/Library/Application Support/kiro-cli/data.sqlite3` | [已验证:CodexBar] |
| CLI DB（Linux） | `~/.local/share/kiro-cli/data.sqlite3` | [已验证:kiro-meter] |
| OIDC token | 表 `auth_kv`，key `kirocli:odic:token`（拼写就是 odic） | [已验证:CodexBar；其余字段来自摘要] |
| 社交登录 token | `auth_kv` key `kirocli:social:token` | [未验证] |
| OIDC 客户端注册 | `kirocli:odic:device-registration`；旧前缀 `codewhisperer:odic:*` | [未验证] |
| Profile | 表 `state`，key `api.codewhisperer.profile` 的 `arn` | [已验证:CodexBar] |
| IDE token | `~/.aws/sso/cache/kiro-auth-token.json`（HackerOne 3630605 称全局可读） | [未验证] |
| Keychain | 无证据 | [未验证] |

- **关键风险**：SSO refresh token 每次刷新都轮换，外部工具刷新并写回会让 CLI 当前会话失效；第三方工具普遍只读打开 DB，过期时提示运行 kiro-cli [已验证]。社交登录刷新端点未核实 [未验证]。

**账号切换**：官方无多账号命令，只能 logout/login [未验证]；kiro-cli 不运行时替换整个 `data.sqlite3` 或 `auth_kv`/`state` 行（切走前先保存最新轮换后的行）[未验证:推断]；无 KIRO_HOME，可试按账号隔离 `HOME`/`XDG_DATA_HOME` [未验证]；非交互用 `KIRO_API_KEY` 按进程指定账号，但已登录会话优先级更高会覆盖 [已验证]；IDE 换 `kiro-auth-token.json` 后可能需重启 [未验证]。

**额度**
| 项 | 周期/重置 | 标注 |
|---|---|---|
| 套餐 credits：Free 50 / Pro 1,000 / Pro+ 2,000 / Pro Max 5,000 / Power 10,000 | 每计费周期刷新 | [已验证:kiro.dev/docs/billing] |
| 超额：currentOverages、overageCap(WithPrecision)、overageRate、overageCharges | 套餐内先用 | [已验证:CodexBar] |
| Bonus credits：`bonuses[]`，有过期；首次升级送 $20 | 按包到期 | [已验证] |
| 重置：`nextDateReset` | — | [已验证] |

```
POST https://codewhisperer.us-east-1.amazonaws.com/   （欧洲区: https://q.eu-central-1.amazonaws.com/）
X-Amz-Target: AmazonCodeWhispererService.GetUsageLimits
Content-Type: application/x-amz-json-1.0
Authorization: Bearer <access_token>
{"profileArn": "<state.api.codewhisperer.profile.arn>"}
```
- 未公开，与 CLI `/usage` 同一调用，不消耗 credits [已验证:CodexBar]；GET 写法 [未验证]。响应：currentUsageWithPrecision、usageLimitWithPrecision、currentOverages、overage*、nextDateReset、bonuses[]。
- 兜底：`kiro-cli chat --no-interactive "/usage"` 解析去 ANSI 文本 [已验证]，这次运行本身是 headless，必须排除。

**本地会话**：CLI `~/.kiro/sessions/cli/*.json`，每轮 credits 在 `session_state.conversation_metadata.user_turn_metadatas[*].metering_usage` [已验证:kiro-meter]；cwd/model 字段名 [未验证]。IDE 日志位置不同，推测在 `~/Library/Application Support/Kiro/` [未验证]。

**Headless 信号**：argv 含 `kiro-cli chat --no-interactive`（也可能写作 `--non-interactive`），常配 `--trust-all-tools`、stdin 管道 [已验证]；IDE 中由 `.kiro/hooks/*.kiro.hook`（`when.type` fileEdited/fileCreated/userTriggered，`then.type` askAgent/runCommand）触发的 agent 执行和 spec autopilot 应归为 automation 排除 [文档；IDE 存储字段名未验证，需装机查 `~/Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/`]；环境有 `KIRO_API_KEY` [已验证]；会话只有一轮且 prompt 为 `/usage`、`/context`（监控探测）[未验证:推断]；会话 JSON 中的 interactive / trust-all-tools 标记 [未验证]。

**Aieyes 建议**
1. 额度：`mode=ro` 打开 `data.sqlite3`，读 `kirocli:odic:token`（兜底 social）和 profile ARN，按区域调 GetUsageLimits；**绝不刷新、绝不写回**，过期提示运行 kiro-cli；IDE token 作第二来源，取最新未过期者。
2. 兜底 `/usage`：超时 20 秒，有输出后空闲 4 秒截断，标为 Aieyes 内部 headless。
3. 多账号：快照 `auth_kv`/`state` 行；切换前确认 kiro-cli 和 IDE 未运行，回写当前账号最新行再写入目标；平时用各账号已存 token 并行查额度，过期标“需重新登录”。
4. 会话：解析 `~/.kiro/sessions/cli/*.json`，按上面信号排除 headless。
5. 风险：格式和接口未公开，做版本探测，出错保留上次正常数据。

来源：[kiro.dev 认证](https://kiro.dev/docs/getting-started/authentication/) · [kiro.dev billing](https://kiro.dev/docs/billing/) · [CodexBar kiro.md](https://github.com/steipete/CodexBar/blob/main/docs/kiro.md) · [kiro-meter](https://github.com/Seuqram78/kiro-meter) · [codex-widget](https://github.com/fberbert/codex-widget) · [kiro-gateway（仅摘要）](https://github.com/jwadow/kiro-gateway) · [HackerOne 3630605（仅摘要）](https://hackerone.com/reports/3630605) · [omarchy#11557](https://github.com/omacom/omarchy/pull/11557)

### 15. ByteDance Trae（国际版）/ Trae CN（2026-10-29 补全）
> 调研条件：本机没装 Trae、Trae CN、SOLO 客户端和 trae-cli（`/Applications/Trae*` 和 `~/Library/Application Support/Trae*` 都不存在），**没法做本地检查**。去取第三方换号工具（Yang-505/Trae-Account-Manager）源码的那次请求被平台安全机制拦截，所以下面关于凭据写入细节的内容仍然只来自 README。改机器 ID、伪造请求头、签名这类绕过官方客户端的做法不写进本文，也不应进入 Aieyes。

**15.1 产品线**
| 产品 | 结论 | 状态 |
|---|---|---|
| Trae IDE 国际版（新文档叫 **TraeCode**） | 2026-10-08 发布 v3.6.0，新增“桌面陪伴机器人”，主窗口不打开也能发起任务、看进度；Builder 并入 Agent，SOLO Coder 改名 SOLO Agent | [已验证:docs.trae.ai/ide/changelog] |
| Trae CN（trae.cn） | 独立账号体系和后端（api.trae.cn），按 credits 计费 | [已验证:traework2api-panel README] |
| SOLO 独立版 | 2026-03-31 报道内测，有桌面端和 Web 端，分 Code 和 MTC 两种模式，要邀请码 | [已验证:第三方报道，经搜索] |
| TRAE Work | Web 端，有签到、积分（credits）、按会话统计用量；社区工具 TraeTools 已于 2026-10-23 归档 | [已验证:star620/TraeTools README] |
| trae-agent CLI | 开源，用自己的 LLM Key，跟 Trae 账号无关 | [已验证:bytedance/trae-agent] |
| VS Code / JetBrains 插件（原 MarsCode） | 存在，细节未查到 | [未验证] |

**15.2 登录方式（按区域）**
- **CN（trae.cn）**：手机号加验证码（会自动注册）、抖音扫码；老教程里还有掘金账号 [已验证:CSDN 教程]。客户端打开浏览器授权页 `www.trae.cn/authorization`，再回调回 IDE [已验证:README]。
- **国际版（trae.ai）**：要海外账号，从中国大陆登录大概率失败 [已验证:CSDN]。Google、GitHub、邮箱这几种方式**没找到官方原文** [未验证]。官方文档里的 GitHub 是插件市场里的“应用授权”，用来连仓库，不是登录方式 [已验证:docs.trae.cn/ide_connect-trae-to-github]。
- **TRAE Work**：第三方工具用内嵌 WebView2 走 Web 登录 [已验证:TraeTools README]。
- **trae-agent**：不需要登录。Key 来自 `trae_config.yaml`（YAML，缩进不能用 Tab）或环境变量 / `.env`：`OPENAI_/ANTHROPIC_/GOOGLE_/OPENROUTER_/DOUBAO_` 加 `API_KEY`、`BASE_URL`。优先级是 命令行 > 配置文件 > 环境变量 > 默认值 [已验证:README]。

**15.3 凭据、刷新和设备绑定**
| 项 | 结论 | 状态 |
|---|---|---|
| IDE 内的令牌位置 | Webview Local Storage 里含 `iCubeAuthInfo` 的键，值里有 `token` 字段 | [已验证:Trae-Account-Manager README] |
| 磁盘路径 | 推测 `~/Library/Application Support/Trae/`（CN 是 `Trae CN/`）下的 `User/globalStorage/storage.json`、`state.vscdb`、`Local Storage/leveldb`，这是 VS Code 系的常规布局 | [未验证]，要装机确认 |
| Web 会话（Work/Web） | JWT 有效期约 **8 小时**；另有 Cookie `X-Cloudide-Session`，约 **14 天**，能用它静默换新 JWT，换不了就要重新登录 | [已验证:TraeTools README] |
| IDE access token | 约 14 天，刷新时会轮换 | [已验证:README]，接口 [未验证] |
| 设备绑定 | 每个账号对应一个 `TRAE_DEVICE_ID`；同一设备上账号多了会触发风控提示“当前参与用户太多” | [已验证:TraeTools FAQ] |
| trae-agent | 明文 YAML / `.env` | [已验证] |

**15.4 换号和多开**
- 官方没有换号命令 [未验证，文档里没找到]。
- 社区工具的做法是冷切换：关 IDE → 清登录态 → 写入目标账号 → 重启。TraeTools 会把每个账号的登录态备份进一个 “vault”，切换失败自动回滚 [已验证:README]。这种做法涉及设备 ID 和封号风险，Aieyes **不做**。
- **可行做法**：用 `--user-data-dir` 给每个账号开独立实例，可以并行（cockpit-tools 实测只覆盖原版 Trae）[已验证:cockpit-tools]。Trae CN 和 SOLO 独立客户端能不能这样做 [未验证]。

**15.5 额度**
| 时期 / 项 | 内容 | 状态 |
|---|---|---|
| 旧模式（2025 到 2026-02） | Free：Premium Fast 10、Slow 50、Advanced 1000、补全 5000，按月重置。Pro $10：Fast 600+，Slow 和补全不限；Max Mode 按 token 折算 Fast Request（约 $0.017 = 1 次） | [已验证:trae.ai blog 和多家第三方] |
| 活动加量 | 国际版一周年：Free 送 600 Fast（到 2026-02-14），Pro 送 800（到 2026-03-14） | [已验证:zeeklog] |
| 新模式（2026-02 下旬起） | 按 token 计费，**以美元计**的 usage 余额从月度额度里扣；Pro 含 $20 Basic usage 加 Bonus；Pro+ 是 Pro 的 3.5 倍；Ultra 是 20 倍；有 Lite；额度用完可开 pay-as-you-go；支持 BYOK | [已验证:多家第三方]，官方页面没取到 |
| 2026-09 调价 | 有报道说 Pro 涨到 $20、Pro+ 到 $60；另一处 2026-08 的资料还写 $10 | 有冲突，[未验证] |
| CN 和 Work | 用 credits（积分）；有剩余积分、趋势、即将过期提醒；按会话列出模型、输入/输出 token、缓存命中、消耗积分 | [已验证:TraeTools、traework2api README] |
| 接口 | intl `ug-normal.trae.ai/trae/api/v1/pay/ide_user_ent_usage`（host 可能随账号 `api_host`）；CN `api.trae.cn/trae/api/v2/pay/ide_user_ent_usage`；按会话用量 `POST /trae/api/v1/pay/query_user_usage_group_by_session` | [已验证:README]，非公开接口 |
| 已知字段 | `is_dollar_usage_billing`、`is_credits_billing`；`user_entitlement_pack_list[].entitlement_base_info.quota.{credits_limit,basic_usage_limit,premium_model_fast_request_limit,…}`；`usage.{credits_amount,basic_usage_amount}`；`display_desc` | [已验证:README] |
| 响应字段变化 | 2026-02 改版后，Fast/Slow 计数改成美元余额，旧字段可能已经失效；重置时间字段名、鉴权请求头 | [未验证]，要有账号才能实测 |

**15.6 本地会话日志**
- **trae-agent**：默认写 `trajectories/trajectory_YYYYMMDD_HHMMSS.json`，相对当前工作目录，可以用 `--trajectory-file` 改 [已验证]。字段 [已验证:docs/TRAJECTORY_RECORDING.md]：
  - 顶层：`task, start_time, end_time, provider, model, max_steps, llm_interactions[], agent_steps[], success, final_result, execution_time`
  - `llm_interactions[]`：`timestamp, provider, model, input_messages[{role,content}], response{content,model,finish_reason,usage,tool_calls}, tools_available`
  - `usage`：`input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens, reasoning_tokens`
  - `agent_steps[]`：`step_number, timestamp, state(thinking|calling_tool|reflecting|completed|error), llm_messages, llm_response, tool_calls[{call_id,name,arguments}], tool_results[{call_id,success,result,error}], reflection, error`
  - 注意：用 Ollama 时 `provider` 会记成 `"openai"`。
- **IDE / SOLO**：可能在 `workspaceStorage/*/state.vscdb` 或 globalStorage 里，也可能在云端 [未验证]。

**15.7 Headless 模式和识别信号**
- `trae-cli run "<task>"` 是一次性非交互运行，标 headless；`trae-cli interactive` 算会话 [已验证]。
- 带 `--docker-image`、`--docker-container-id`、`--docker-image-file`、`--must-patch` 的，大概率是批处理或评测 [已验证 flag 存在，用途属推断]。
- 官方 roadmap 把 “Headless Interface（程序化 API）” 列为**计划中**，还没发布 [已验证:docs/roadmap.md]。
- IDE 和 SOLO：没有公开的 headless 或 CLI。v3.6 的桌面机器人能在不开主窗口时发起任务，这种任务**仍算交互会话**，但进程形态可能不同 [已验证 功能；进程信号 未验证]。有第三方评测说 SOLO 不是一直在后台跑的 agent。
- 识别规则：进程名 `trae-cli` 且 argv 里有 `run` 就判为 headless；Python 进程跑 `trae_agent` 模块且没有 tty 也是 headless；轨迹文件本身不记交互还是非交互，要配合 argv 判断。

**15.8 没有账号或装机就确定不了的**
1. IDE、CN、SOLO 实际的磁盘路径、键名和加密方式（`state.vscdb` 或 safeStorage）。
2. 2026-02 改为美元计费后 `ide_user_ent_usage` 的响应 schema，以及 CN credits 的字段。
3. 国际版具体有哪些登录方式（Google、GitHub、邮箱）。
4. SOLO 独立版、Work 的本地日志在哪，是否只存在云端。
5. 桌面机器人和 SOLO 后台任务对应的进程和 argv 特征。
6. 2026-09 起的当前价格（官方定价页没取到）。

**Aieyes 建议**
1. 第一步只读监控：识别 Trae/Trae CN/SOLO 进程，解析 trae-agent 轨迹；`trae-cli run` 标 headless。
2. 额度：装机后确认令牌落盘位置，只用用户本人令牌低频（≥5–10 分钟）调 `ide_user_ent_usage`，按 `is_dollar_usage_billing`/`is_credits_billing` 分别显示美元余额或 credits；失败引导到官方 Usage 页。
3. 多账号：**不做“写回认证 + 改机器 ID”切换**（设备绑定，有违反条款和封号风险）；推荐每账号一个 `--user-data-dir` 实例，Aieyes 只负责启动和汇总额度。
4. 不借鉴反向代理项目的伪造请求头和多账号轮换做法。

来源：[bytedance/trae-agent](https://github.com/bytedance/trae-agent) · [TRAJECTORY_RECORDING.md](https://github.com/bytedance/trae-agent/blob/main/docs/TRAJECTORY_RECORDING.md) · [roadmap](https://github.com/bytedance/trae-agent/blob/main/docs/roadmap.md) · [TRAE Changelog](https://docs.trae.ai/ide/changelog) · [TRAE 入门](https://docs.trae.ai/ide/set-up-trae?_lang=en) · [连接 GitHub](https://docs.trae.cn/ide_connect-trae-to-github) · [TraeTools](https://github.com/star620/TraeTools) · [Trae-Account-Manager](https://github.com/Yang-505/Trae-Account-Manager) · [CSDN 教程](https://blog.csdn.net/weixin_43525722/article/details/146078172) · [一周年活动](https://zeeklog.com/aizi-yuan-bai-piao-traeguo-ji-ban-yi-zhou-nian-fu-li-mian-fei-yong-yi-ge-yue-600ci-kuai-su-qing-qiu-6) · [vibecoding.gallery 定价](https://vibecoding.gallery/en/tools/trae/) · [CreditsPlan](https://www.creditsplan.com/plans/trae.pro/) · [diyai 评测](https://diyai.io/ai-tools/code-generation/reviews/trae-ai-review/) · [Max Mode in TRAE](https://www.trae.ai/blog/trae_update_0902) · [cockpit-tools](https://github.com/jlcodes99/cockpit-tools/blob/main/README.en.md) · [traework2api-panel](https://github.com/swysgh/traework2api-panel) · [TraeIDE-Switcher](https://github.com/WanJiu-LF/TraeIDE-Switcher)

### 16. Cline / Roo Code / Kilo Code
> 本机三者均未安装（各 IDE globalStorage 下无对应扩展目录，无 `~/.cline`、`~/.kilocode`、`~/.local/share/kilo`，无 `cline`/`kilo` 命令）[已验证:本机]。以下来自 GitHub 主分支源码和官方文档。

**形态**
| 产品 | 形态 | 最新版本 | 状态 |
|---|---|---|---|
| Cline | VS Code 扩展 `saoudrizwan.claude-dev`、Cline CLI、cline-hub、SDK；JetBrains [未验证] | v4.1.26（2026-10-26） | 活跃 [已验证] |
| Roo Code | VS Code 扩展 `rooveterinaryinc.roo-cline`，另有 `apps/cli` | v3.54.0（2026-05-15） | **仓库已 archived** [已验证] |
| Kilo Code | VS Code 扩展 `kilocode.kilo-code`、JetBrains 插件、Kilo CLI（opencode fork） | v7.8.18（2026-10-26） | 活跃；扩展以 `kilo serve --port 0` 启动 CLI 后端 [已验证] |

建议：Roo Code 只做只读历史兼容，不投入额度功能。

**登录与凭据**
| 产品 | 登录 | 存储 | 刷新 |
|---|---|---|---|
| Cline（新版扩展 + CLI 共用） | Cline 账号 OAuth（WorkOS，`workos:` 前缀）；OCA；OpenAI Codex OAuth；40 多个 BYO key | `~/.cline/data/settings/providers.json` 明文 JSON（`auth.accessToken/refreshToken/expiresAt(ms)/accountId`），可用 `CLINE_PROVIDER_SETTINGS_PATH`、`CLINE_DATA_DIR`、`CLINE_DIR` 覆盖 [已验证:源码] | 有 refreshToken；多进程刷新加锁（`oauth-refresh-lock.ts`）[已验证] |
| Cline 旧版扩展 | 同上（旧版 Firebase） | VS Code SecretStorage，键 `clineApiKey`、`clineAccountId`、`openRouterApiKey` 等 [已验证]；物理在 `state.vscdb` 的 `secret://…` [未验证:推断] | — |
| Roo Code | Roo Code Cloud（Clerk）[未验证]；BYO key | SecretStorage；CLI 用 vscode-shim 文件模拟 [路径未验证] | [未验证] |
| Kilo CLI / 新扩展 | 设备码：`POST {KILO_API}/api/device-auth/codes`，轮询 `GET .../codes/{code}` [已验证]；opencode 系 oauth/api/wellknown 凭据；BYO key | `$XDG_DATA_HOME/kilo/auth.json`（macOS 默认 `~/.local/share/kilo/auth.json`，0600）[已验证]；旧 `~/.kilocode/cli/config.json` 的 `kilocodeToken` 自动迁移 [已验证] | 长期 Bearer，有无 refresh [未验证] |
| 地区 | Cline 生产 `api.cline.bot`/`app.cline.bot`；Kilo `KILO_API_URL`（默认 `https://api.kilo.ai`），网关 `https://ai-gateway.kilo.ai/api/v1` [已验证]；无中国区登录 [未验证] | | |

**切换**
| 产品 | 机制 | 即时切换 / 并发 |
|---|---|---|
| Cline CLI | `--config`/`CLINE_DIR`、`--data-dir`/`CLINE_DATA_DIR` 隔离；`cline auth -p -k -m` 非交互登录；单次 `-P/-k/-m` [已验证:cli-reference] | **可行**：每账号一个 `CLINE_DIR` 并发；也可原子替换 `providers.json`，注意刷新锁 |
| Cline VS Code | 新版也读 `providers.json`，热加载 [未验证]，保险做法是 reload | 单实例一个账号；组织在 UI 切换 |
| Roo Code | API Configuration Profiles [未验证] | 同时只用一个 profile |
| Kilo | `KILO_CONFIG_DIR`；`XDG_DATA_HOME` 隔离；扩展有 `AccountSwitcher`，JetBrains 有会话级账号选择 [已验证:文件存在]；组织用 `x-kilocode-organizationid` 头 | **可行**：XDG 隔离并发 |

**额度 / 余额**
| 来源 | 端点 | 字段 |
|---|---|---|
| Cline 个人 | `GET https://api.cline.bot/api/v1/users/{uid}/balance`（uid 从 `/api/v1/users/me`），Bearer | BalanceResponse（预付，无周期）[已验证；字段名未验证] |
| Cline 用量/支付 | `/users/{uid}/usages`、`/payments` | `{items:[UsageTransaction]}` [已验证] |
| Cline 组织 | `/organizations/{orgId}/balance`、`/members/{memberId}/usages`；`POST /users/me/budget/request` | [已验证] |
| Cline Pass | provider `cline-pass` [已验证]；额度端点 [未验证] | |
| Kilo 余额 | `GET https://api.kilo.ai/api/profile/balance`（团队加 `x-kilocode-organizationid`） | `{balance}`（美元）[已验证] |
| Kilo 资料 | `GET /api/profile` | email/name、organizations[]、selectedOrganizationId、hasPersonalAccount [已验证] |
| Kilo Pass | `GET /api/trpc/kiloPass.getState` | [字段未验证] |
| OpenRouter key | `GET https://openrouter.ai/api/v1/key` | `limit`、`limit_remaining`、`limit_reset`、`usage_daily/weekly/monthly`、`budgets[]`(`reset_interval`,`resets_at`)、`is_free_tier`、`free_model_daily_requests` [已验证:官方] |
| OpenRouter 账户 | `GET /api/v1/credits` → `total_credits,total_usage` | 鉴权 [未验证]；Roo 用 `/key` 的 `limit - usage` [已验证] |
| 其他 BYO | Requesty（Roo 有组件）；Anthropic/OpenAI 无公开余额端点 | |

**会话**
- Cline 旧版：`<globalStorage>/saoudrizwan.claude-dev/tasks/<taskId>/`（`api_conversation_history.json`、`ui_messages.json` 带 tokensIn/Out、cost，`task_metadata.json`）[文件名已验证，字段未验证]。
- Cline 新版：`~/.cline/data/sessions`，`~/.cline/data/db/`（`sessions.db`、`tasks.db`、`cron.db`、`connectors.db`），`~/.cline/schedules`，可用 `CLINE_SESSION_DATA_DIR` 等覆盖 [已验证:paths.ts；表结构未验证]。
- Roo：`tasks/<id>/history_item.json` + `tasks/_index.json`，字段 tokensIn/Out、totalCost、workspace、mode、cache* [已验证]。
- Kilo：`~/.local/share/kilo/kilo.db`（`KILO_DB`，按 channel `kilo-<channel>.db`），日志 `~/.local/share/kilo/log` [已验证]；表沿用 opencode 的 session/message/part [未验证]；旧扩展 Roo 格式 [未验证]。

**Headless**
- Cline CLI：位置 prompt 或 stdin 管道即非交互，`-i/--tui` 才交互；`--json`（NDJSON）、`--auto-approve`、`-z/--zen`、`--timeout`；`cline schedule`/cron、kanban/hub 会话也是非交互 [已验证:cli-reference；字段级未验证]。
- Kilo：`kilo run`、`--auto`、`--format json` 非交互 [文档，未复核]；session 表 `parent_id` 表示子 agent（同 opencode）[未验证]；`kilo serve` 是扩展后端，不算会话，按其中 session 计 [已验证]；扩展启动带 `KILO_DISABLE_CHANNEL_DB`、`KILO_EXPERIMENTAL_DISABLE_FILEWATCHER` 环境变量可识别 IDE 后端 [已验证]。
- Cline 扩展 tasks 文件和 `taskHistory.json` 都没有 headless 字段；CLI 任务存 `~/.cline/data/tasks/`，可按存储路径区分扩展与 CLI [未验证]。
- 扩展类：无 headless，全部算交互；Roo subtask（parentTaskId）合并到父会话。

**Aieyes 建议**
1. 发现：扫 `~/.cline/data`（及 `CLINE_DIR`）、`~/.local/share/kilo`（及 `XDG_DATA_HOME`）、各 VS Code 系 IDE 的三个扩展 `tasks` 目录。
2. 会话：Roo/旧 Cline 读 json；新 Cline `sessions.db`、Kilo `kilo.db` 只读（`mode=ro`，注意 WAL）。
3. 额度：Cline 用 providers.json 的 accessToken 调 `/users/me` → `/balance`，**过期不自行刷新**（避免和 refresh-lock 冲突）；Kilo 调 `/api/profile` 和 `/balance`，组织逐个加头查；OpenRouter 调 `/api/v1/key`。token 只放内存。
4. 切换：Cline 每账号一个 `CLINE_DIR`，或原子替换 `providers.json`（临时文件 + rename，保持 0600）；Kilo 每账号一个 `XDG_DATA_HOME`，或替换 `auth.json` 的 `kilo` 条目；两者都能并发。扩展热加载先实测。
5. Headless：Cline `--json`、位置 prompt 无 `-i`、schedule/cron 不计；`kilo run` 不计，`kilo serve` 本身不算会话但其中 IDE 发起的 session 计入。
6. 装机后只读抽样核对 sessions.db/kilo.db 表结构和 Cline balance 字段。

### 17. Tencent WorkBuddy（含 CodeBuddy 积分）
> 本机已装 WorkBuddy.app 5.5.6，只读检查，未打印任何密钥值。

**形态**：Electron 桌面 Agent，内嵌 CodeBuddy Code CLI 运行时，会话 meta 有 `acpConnectionId`、`codebuddy.ai/hostKind`（ACP 驱动）[已验证:本机]；版本 5.5.6 [已验证:本机]；国内版（workbuddy.cn / codebuddy.cn / copilot.tencent.com）与国际版（workbuddy.ai / codebuddy.ai）[已验证:本机]；**与 CodeBuddy 共用账号和积分池**，订阅按账号计；2026-10-29 起 Buddy One 积分可在腾讯文档、ima、腾讯网盘通用 [已验证:web]。

**登录**
| 方式 | 证据 |
|---|---|
| 微信扫码（`/api/user/login/wxqrcode`、`/v2/user/wechat/auth/switch`） | [已验证:本机 asar] |
| 企业微信（`/api/account/wecom/login`） | [已验证:本机] |
| QQ（`/api/user/oauth/qq/login` 等） | [已验证:本机] |
| 手机短信（`/v1/auth/sms/code/{init,send,verify}`） | [已验证:本机] |
| 企业 SSO / iOA | [已验证:本机] |
| 国际版浏览器跳转 `https://www.workbuddy.ai/login?platform=workbuddy-ai&state=…`，客户端轮询，未完成返回 `code=11217` | [已验证:web helper] |
| API Key（`CODEBUDDY_API_KEY` / `CODEBUDDY_AUTH_TOKEN`） | [已验证:本机 asar] |
| 多账号选择（`/v1/auth/accounts`、`/v1/auth/select_account`、`/v2/accounts`） | [已验证:本机]，用法 [未验证] |

**凭据存储**：`~/.workbuddy/local_storage/`、`wb_entry_*.info` 中可识别的只是 gzip+base64 的 product config，其余 entry 是不透明短串 [已验证:本机]；按 accessToken/refresh_token 搜 `~/.workbuddy` JSON 无明文 [已验证]；login keychain 无相关条目 [已验证]；`~/Library/Application Support/WorkBuddy` 与 `com.tencent.workbuddy.mac` 为空 [已验证]；推测 token 加密在 local_storage 不透明 entry 或 `app/session` 的 Cookies/LevelDB [未验证]；CodeBuddy 用 `~/.codebuddy/local_storage/entry_*.info`，结构相同 [已验证]。刷新：`/v2/auth/token/refresh`、`https://copilot.tencent.com/oauth2/token` [已验证:asar]，社区先读 JWT `exp` 再回退 `expiresIn` [已验证:web]。

**切换**：GUI 退出再登录；select_account/handshake 可在同一身份的个人与团队账号间切换（本机有 `storage/user-<uid>` 和 `user-<uid>-personal`）[已验证:本机]；`CODEBUDDY_CONFIG_DIR` 用于 CLI 运行时 [已验证]，GUI 能否换 `~/.workbuddy` [未验证]；token 加密不透明，换文件不可行 [未验证]；可行路径是 Aieyes 自行浏览器 OAuth 持有 token（workbuddy-helper 做法），或每账号一份 API Key + `CODEBUDDY_CONFIG_DIR`；桌面端单实例（`SingletonLock`）[已验证]。**风险**：有封号报道，批量注册、号池会被风控 [已验证:web]。

**额度 / 积分**
| 项 | 周期 | 端点 | 标注 |
|---|---|---|---|
| 订阅月度积分：体验 500 / 标准 4000 / 高级 9000 / 旗舰 50000 | 按月 | `/billing/meter/get-user-resource-summary`、`…-paid-packages` | [已验证:web 定价、本机端点] |
| 免费/赠送包（新人约 3000，90 天；签到） | 按包到期 | `/billing/meter/get-user-resource-free-packages` | [已验证] |
| 国际版汇总 | — | `https://www.codebuddy.ai/v2/billing/meter/get-user-resource`（可能分页，每包 balance/used/total） | [已验证:web helper] |
| 企业（每人每月 2000，团队共享） | 按月 | `/billing/meter/get-enterprise-user-usage`、`/console/as/teams/me/quota` | [已验证] |
| Cloud Agent 配额 | [未验证] | `/v2/user/cloudagent/quota` | [端点已验证] |
| 单会话上下文与积分 | 按会话 | 本地 `workbuddy.db.session_usage(used,size,credit_json)` | [已验证:本机] |

- 鉴权推测 `Authorization: Bearer` + `X-User-Id`、`X-Domain`、`X-Product: SaaS`、`X-IDE-Name: WorkBuddy` [未验证]；国内 host、方法和响应字段需抓包确认 [未验证]。

**会话**：`~/.workbuddy/workbuddy.db`（SQLite），表 `sessions`、`session_usage`、`automations`、`automation_runs`、`workspaces`；`sessions` 含 cwd、user_id、status、last_activity_at、`source_mode`、`is_background_automation`、mode、model、permission_mode 等 [已验证:本机]。对话在 `~/.workbuddy/projects/<cwd 中 / 换成 ->/<sessionId>.jsonl`，另有 `.meta.json`、`.file-rollback.ndjson` [已验证]。日志 `~/.workbuddy/logs/`、`traces/`、`audit-log/`；CodeBuddy 日志 `~/.codebuddy/logs/` [已验证]。

**Headless**：`is_background_automation=1` 及出现在 `automation_runs` 的会话不计 [已验证:schema]；`source_mode` 本机样本 `craft` 为交互，其他取值 [未验证]；`is_playground` [已验证:schema]；内嵌 CLI 的 `-p` 和 `CODEBUDDY_*` 启动进程参考 CodeBuddy 检测 [未验证]。

**Aieyes 建议**
1. 会话：只读打开 `workbuddy.db`（`mode=ro` + busy_timeout），用 status 和 last_activity_at 判断实时；过滤 `is_background_automation=1`、`is_playground=1`、`deleted_at` 非空；用 projects 下 jsonl 补内容和 token。
2. 账号：从 `sessions.user_id` 和 `storage/user-<uid>` 推断，不解密桌面端 token。
3. 额度：Aieyes 自行 workbuddy.ai / .cn 浏览器 OAuth，token 加密存 Keychain，调 `billing/meter/get-user-resource*` 显示各包 balance/used/total 和到期；与 CodeBuddy 共池只显示一份（按账号 poolId 去重）。
4. 切换：GUI 即时切换不支持，界面标注；多账号只做额度看板，附风控提示，不做批量操作。
5. 实现前抓包确认国内版 host、方法和字段。

来源：[workbuddy-helper](https://github.com/autumnsentiment/workbuddy-helper) · [WorkBuddy 定价](https://www.workbuddy.cn/docs/workbuddy/Pricing) · [CodeBuddy 定价](https://www.codebuddy.cn/docs/ide/Account/pricing) · [积分共享说明](https://iqilian.com/learn/workbuddy-codebuddy-tongyi-huiyuan/) · [Buddy One](https://news.qq.com/rain/a/20261029A05PM400) · [封号报道](https://blog.liurb.org/posts/2026/10/20/workbuddy-connect/)

### 联网核实来源（回填到第 1–9 节）
[MMoMM-org/the-custom-startup#129](https://github.com/MMoMM-org/the-custom-startup/issues/129) · [Anthropic extra usage](https://support.anthropic.com/en/articles/12005970-additional-usage-for-claude-for-work-team-and-enterprise-plans) · [CodexBar deepseek.md](https://github.com/steipete/CodexBar/blob/main/docs/deepseek.md) · [DeepSeek widget #157](https://github.com/MeteorNOX/DeepSeek-Balance-Whale-Widget/issues/157) · [deepseek-platform-widget](https://github.com/MCwasd/deepseek-platform-widget) · [Qoder CLI 认证](https://docs.qoder.com/cli/authentication) · [Kimi Server API](https://www.kimi.com/code/docs/en/kimi-code-cli/reference/server-api.html) · [kimi-code PR #4373](https://github.com/MoonshotAI/kimi-code/pull/4373) · [cozyphi PR #30](https://github.com/alvnukov/cozyphi/pull/30) · [orca #31804](https://github.com/stablyai/orca/issues/31804) · [Claude-Code-Usage-Monitor #186](https://github.com/CodeZeno/Claude-Code-Usage-Monitor/issues/186) / [PR #190](https://github.com/CodeZeno/Claude-Code-Usage-Monitor/pull/190) · [Cursor 论坛](https://forum.cursor.com/t/i-cant-get-my-usage-info/155469) · [ChatGPT 定价](https://learn.chatgpt.com/docs/pricing) · [OpenAI Help 20001516](https://help.openai.com/en/articles/20001516-managing-usage-with-gpt-6-astra-in-work-and-codex) · [Google blog Antigravity](https://blog.google/feed/new-antigravity-rate-limits-pro-ultra-subsribers/) · [SmartScope](https://smartscope.blog/en/blog/antigravity-claude-quota-five-hour-weekly-limits-2026/) · [agentpedia](https://agentpedia.codes/blog/antigravity-credits-pricing-explained)

---

## (b) 统一的类 Codex 账号切换与多账号额度模型

> **2026-10-29 修订**：本节已按 (g) 单账号优先方案修订，冲突处以 (g) 为准。

### 数据模型
```
Account { id, provider, region, identity{userId/email/org}, authKind, profileRef, quotaSources[] }
Profile { kind: ConfigDir | FileSwap | Keychain | Cookie | ApiKey | DbRow, path/env, isolatedConcurrent: bool }
QuotaSnapshot { accountId, windows[{name, usedPct|used/limit, resetsAt, period}], wallets[{kind: plan|addon|shared|granted|paid|overage, amount, currency}], fetchedAt, source, verified }
```
身份统一用服务端返回的 userId 或 email 来去重（Codex 已经这么做），不靠文件路径。

### 按类别的切换策略
| 类别 | 适用 Agent | 做法 | 能否同时多开 | 风险 |
|---|---|---|---|---|
| A. 配置目录覆盖（首选） | Codex(CODEX_HOME)、Qoder(QODER_CONFIG_DIR)、Kimi(KIMI_CODE_HOME)、ZCode(ZCODE_DATA_BASE_DIR)、DeepSeek(DSH_HOME)、Grok(GROK_HOME)、OpenCode(XDG_DATA_HOME)、Pi/omp、CodeBuddy(CODEBUDDY_CONFIG_DIR)、Claude(Linux)、Copilot CLI(COPILOT_HOME，keychain 共享待确认)、Cline(CLINE_DIR)、Kilo(XDG_DATA_HOME)、Kiro(隔离 HOME，未验证)、Trae/Trae CN(`--user-data-dir` 实例) | 每个账号建一个托管目录；“当前账号”只是一个默认指针（shell 环境变量、wrapper、软链）；登录在隔离目录里完成 | 能 | 历史分散在各目录，导入时要多根扫描；共享 DB（ZCode）没有账号归属 |
| B. 凭据文件替换 | Antigravity token、ZCode credentials.json、Pi auth.json、Grok auth.json、Cline providers.json、Kilo auth.json、Kiro data.sqlite3 的 auth_kv/state 行（refresh token 轮换，必须回写） | Vault 保存副本，确认没有进程运行（或关闭进程）后原子替换 | 不能 | refresh token 轮换：每次替换前先把运行中的最新文件**回写**到 Vault |
| C. Keychain | Claude（macOS）、cursor-agent、Copilot CLI（service `copilot-cli`，但优先用 E 类） | 替换 Keychain item 并同步元数据文件 | 不能（除非 item 按目录区分） | 需要 Keychain 授权，轮换问题同上 |
| D. Cookie / Web 会话 / 自持 OAuth | Cursor 用量、OpenCode Console、Qoder Web、WorkBuddy/CodeBuddy（Aieyes 自行 OAuth 持 token，只看额度）、Trae（只用用户令牌低频查额度） | 只用于查询额度，不做切换；按账号保存 Cookie，过期提示 | 只读查询可以 | Cookie 过期，私有接口可能改动 |
| E. API key / 进程级 token | 所有支持 key 的 agent；Copilot（`COPILOT_GITHUB_TOKEN`，首选）、Kiro（`KIRO_API_KEY`，仅非交互，会被已登录会话覆盖）、Qoder（PAT 环境变量）、Cline（`-P/-k/-m`）、CodeBuddy/WorkBuddy CLI（`CODEBUDDY_API_KEY`） | 按进程注入环境变量；Vault 存 Keychain | 能 | 余额接口各不相同，有的没有 |
| F. DB 行切换 | OpenCode credential.active、Cursor state.vscdb | 只在 agent 关闭时修改 | 不能 | schema 变化；放最后做 |
| G. 不支持切换 | WorkBuddy GUI（单实例 + 加密 token）、Trae 写回认证+改机器 ID（设备绑定、封号风险）、Copilot/Cline VS Code 扩展（UI 手动） | 只做多账号额度看板 | — | 条款与风控 |

### Credential Vault
- 秘密放进 macOS Keychain（Aieyes 自己的 service），元数据放 SQLite；UI 和日志里不显示明文。
- 每个账号记录 `lastRefreshAt`。切走前先回收（harvest）当前活跃凭据，防止旧的 refresh token 覆盖新的。
- 切换流程沿用 Codex：探测进程 → 提示关闭 → 回收 → 替换或改指针 → 校验身份 → 刷新额度。

### 多账号额度
- 每个账号独立轮询，后台账号用低频率，当前账号用高频率；共享池（Qoder org、Cursor team、Grok 和 Cursor 的 X 订阅、WorkBuddy 与 CodeBuddy 共用积分池、ChatGPT Work 与 Codex 共用额度）按 poolId 去重。
- **绝不自行刷新会轮换的 refresh token**：Kiro、Kimi、Trae、Cline（有 refresh-lock）。过期只标“需重新登录/等待刷新”。
- 钱包类型统一为 plan、addon、shared、granted、paid、overage（Claude extra_usage、Kimi booster、Copilot overage、Kiro bonuses/overage、WorkBuddy 免费包接到 ExtraUsageWallet；Cline/Kilo/OpenRouter 预付余额归 paid）。

---

## (c) 统一的 Headless 排除规则（按优先级）
1. **日志里的显式字段**（最可靠）：Claude 的 `entrypoint`（白名单 cli、claude-vscode）；Codex 的 `source`（白名单 cli、vscode；`subagent`/`guardian_review` 归父或排除），`originator=codex_work_desktop` 归为 ChatGPT Work（计入，单独标注）；ZCode 的 `task_type`（只有 interactive 计入，`parent_id` 非空则归入父会话）；OpenCode 和 CodeBuddy 的 `parent_id`/sidechain 归入父会话；WorkBuddy `is_background_automation=1`、`is_playground=1`、出现在 `automation_runs` 中的排除；Cursor hook `is_background_agent=true` 排除（云端代理）；Copilot `sessions.host_type`（取值待核对）；Roo subtask（parentTaskId）归入父会话。
2. **Hook 打标**：Aieyes 安装的 SessionStart hook 读取父进程 argv（`ps -o args= -p $PPID`）和 TTY，写入 `.aieyes` 标记文件，按 sessionId 关联。已确认 hook payload 本身**不带**交互标志的有 Kimi、Cursor（`is_headless` 只进遥测）；Antigravity 待核对。
3. **进程 argv 规则表**（进程采样时判定，标记 pid，再用现有 `activity.pid` 绑定会话）：

| Agent | 判为 headless / 非交互 | 不算会话 | 算交互 | 证据 |
|---|---|---|---|---|
| Claude | `-p/--print`（兜底，主信号是 entrypoint） | — | 其余 | [已实现] |
| Codex | `exec`、`mcp-server`（兜底，主信号是 source） | — | — | [已实现] |
| DeepSeek dsh | `headless`、`--profile headless|sdk|acp` | — | `tui`、`web`、desktop | [已验证:help] |
| Kimi | `-p/--prompt`、`acp`、`web`、`rc|remote` | — | 其余 | [已验证:help] |
| Cursor | `cursor-agent|agent` 带 `-p/--print` | `is_background_agent=true` | 其余 | [已验证:help+bundle] |
| OpenCode | `run`、`acp`、`api`、不带 `--service` 的 `serve` | `serve --service`（TUI 后台） | TUI | [已验证:help+日志] |
| Antigravity | `-p/--print/--prompt`、`--output-format`、`--input-format stream-json`、`--print-timeout`、`--json-schema` | `remote-control`、`mic-serve` | `-i/--prompt-interactive` | [已验证:help] |
| Qoder | `-p`、`--output-format`、`stream-json` | Aieyes 的 get_usage_info | 其余 | [已验证:本机] |
| ZCode | `-p/--prompt`、`--target` | `app-server` | — | [已验证] |
| Grok | `-p` | — | — | [未验证] |
| Pi/omp | `-p`、`--mode json|rpc` | — | — | [未验证] |
| Copilot CLI | `-p/--prompt`、`--acp`、JSON-RPC server（日志 “Standalone JSON-RPC”） | — | — | [docs；日志信号推断] |
| Kiro | `chat --no-interactive`/`--non-interactive`、`KIRO_API_KEY`；IDE hook/autopilot 触发归 automation | — | — | [文档，未复核] |
| Cline | 位置 prompt 无 `-i`、stdin 管道、`--json`、`-z/--zen`、`schedule`/cron、`--oneshot`、`-y` | — | `-i/--tui` | [文档] |
| Kilo | `run`、`--auto`、`--format json` | `serve`（扩展后端，按其中 session 计） | — | [文档] |
| trae-cli | `run`；Python 跑 `trae_agent` 且无 tty；`--docker-*`、`--must-patch` 视为批处理 | — | `interactive` | [已验证:README] |
| Trae IDE/SOLO | 无公开 headless；桌面机器人任务仍算交互 | — | 全部 | [未验证进程信号] |

   辅助信号：环境变量 `CI`/`GITHUB_ACTIONS`、`KIRO_API_KEY`、`COPILOT_GITHUB_TOKEN`；无 TTY。
4. **Aieyes 自身发起的会话**：自己做额度查询或登录检查时 spawn 的进程（qodercli get_usage_info、agy /usage、`kiro-cli chat --no-interactive "/usage"`）注入 `AIEYES_INTERNAL=1`，或使用专用的配置目录，一律排除。
5. **没有任何信号时**：默认计入，但标为 `classification=unknown`。不能像 Codex 那样未知就排除，否则交互会话会大量丢失（与 Codex 的策略不同，需要用户确认）。**注意**：DeepSeek、Kimi、Cursor、OpenCode、Antigravity、Copilot、Trae 的会话文件/DB 都没有可靠字段，进程退出后的历史会话无法事后判定，只能对 Aieyes 运行期间采样到的会话分类。

实现上要把 `sessions.rs:571 non_interactive()` 改成按 provider 分派的 classifier trait，并补齐测试。

---

## (d) 实施阶段与风险

> **2026-10-29 修订**：本节已按 (g) 单账号优先方案修订，冲突处以 (g) 为准。

| 阶段 | 内容 | 依赖 |
|---|---|---|
| P0 Headless | classifier trait；ZCode task_type 和 parent_id；Codex originator（Work）与 guardian 子代理；Aieyes 内部会话标记；hook 打父进程 argv/TTY 标；按 (c) 表实现 DeepSeek、Qoder、OpenCode（排除 `serve --service`）、Cursor（排除 is_background_agent）、Kimi、Antigravity、Grok、Pi 的 argv 规则 | 无 |
| P0.5 新 agent 只读接入 | WorkBuddy（workbuddy.db + projects jsonl，本机可直接做）；Copilot（VS Code chatSessions、session-store.db、~/.copilot）；Cline/Kilo/Roo 会话（只读 DB/json）；Kiro CLI 会话 json；trae-agent 轨迹 | 需装机样本（Kiro、Cline、Kilo、Trae） |
| P1 额度补全（低风险，只读） | Claude extra_usage；Antigravity `retrieveUserQuotaSummary` + `loadCodeAssist.availableCredits`；ZCode OAuth JWT 查询并推断区域；DeepSeek 账号 get_user_summary 和 .credentials.yaml/.env 中的 key；Moonshot 平台余额；Grok postpaid；OpenCode 新版 DB 凭据；Pi/OpenCode 复用上游额度；Cursor 切新用量端点；Copilot `copilot_internal/user`；Kiro GetUsageLimits；Cline/Kilo/OpenRouter 余额；WorkBuddy 积分（需 Aieyes 自行 OAuth）；Trae `ide_user_ent_usage`（装机后） | 实测确认 schema |
| P2 配置目录类切换 | 把 canSwitch 推广到 A 类：Qoder、Kimi、ZCode、DeepSeek、Grok、OpenCode、Pi、CodeBuddy（需先确认 token 在目录里）；Claude Linux；Cline CLI、Kilo；Copilot CLI 进程级 token 注入 | Vault |
| P3 文件替换和 Keychain 切换 | Claude macOS、Antigravity、Pi、Kiro（sqlite 行快照） | 回收和轮换机制 |
| P4 Cookie、DB 类 | Cursor 自动导入 Cookie 和 IDE 切换；OpenCode active 切换；Qoder.cn；Trae 多实例启动器（不做改机器 ID） | 风险最高 |

风险：
- refresh token 轮换导致账号被登出（Kiro、Kimi、Trae、Cline 尤其明显）。
- 私有接口变化：Cursor、Grok、Qoder Web、DeepSeek platform、Copilot `copilot_internal`、Kiro GetUsageLimits、Trae、WorkBuddy billing。
- 风控与封号：WorkBuddy（号池、批量操作）、Trae（设备绑定、改机器 ID）。
- 计费模型变化：Copilot 2026-06 改 AI Credits；Antigravity 2026-11-02 下线 Claude 4.6。
- 研究缺口：Trae 已补全公开资料，但凭据写入细节的源码请求被拦截；Kiro、Cline、Kilo、Trae 本机未安装；各 agent 的 headless 实际落盘样本需登录后跑一次对照。
- Keychain 授权弹窗。
- 读取 IDE 的 sqlite 时与运行中的应用并发冲突。
- 共享 DB 的历史无法归属到账号。
- 部分平台条款可能禁止自动化切换账号。

---

## (e) 需要用户确认的问题

> **2026-10-29 修订**：本节已按 (g) 单账号优先方案修订，冲突处以 (g) 为准。

1. 没有信号的会话，默认计入还是排除？
2. ChatGPT Work 已可识别（`originator=codex_work_desktop`，source 同为 vscode）：要不要在 UI 上和 Codex 分开展示？（额度共用，只影响会话归类）
3. macOS 上 Claude 的切换能否接受“必须先关闭所有 claude 进程”？
4. 是否允许 Aieyes 读取 IDE 的 state.vscdb 或 Keychain 来自动导入 Cookie 和 token（Cursor、CodeBuddy）？
5. 多账号同时运行，是靠 wrapper 或 shell 环境变量注入，还是只做“全局默认账号”切换？
6. 是否需要 Codex Cloud 任务和 Claude Web/Desktop 会话？
7. 后台账号的额度轮询频率和网络预算是多少？
8. 能否提供各家付费或组织账号做实测（Qoder org、Kimi booster、Claude extra_usage、Cursor、Copilot Pro/Business、Kiro Pro、WorkBuddy 付费）？
9. Copilot：是否接受用 Aieyes 自存 token 注入 `COPILOT_GITHUB_TOKEN` 作为主切换方式（不碰 VS Code 账户）？
10. WorkBuddy：是否接受 Aieyes 自行做一次浏览器 OAuth 来读积分（桌面端 token 不解密）？GUI 账号切换标为不支持是否可以？
11. Trae：是否同意不做“写回认证 + 改机器 ID”切换，只做多 `--user-data-dir` 实例和额度看板？
12. Roo Code 已 archived：是否只做历史只读兼容？
13. 新 agent 优先级：建议 WorkBuddy（本机已装）> Copilot > Cline/Kilo > Kiro > Trae，是否同意？
14. 是否需要在本机安装 Kiro、Cline、Kilo、Trae 以核对 [未验证] 的存储格式？
15. 是否允许用测试账号各跑一次 headless（`agy -p`、`dsh headless`、`kimi -p`、`cursor-agent -p`、`opencode run`、`copilot -p`）做落盘对照？会产生会话和少量额度消耗。

## (f) 用户决定（2026-10-29）

1. 无任何 headless 信号的会话：**计入**。
2. 不做多账号并发运行；按现有 Codex 方式切换当前账号，并对所有账号查询限额、余额等。
3. ChatGPT Work 与 Codex **分开**显示（按 `originator=codex_work_desktop` 区分）。
4. Copilot 切换方式：待澄清。
5. WorkBuddy 积分读取方式：待澄清（用户询问不自行 OAuth 是否可读）。
6. Trae 切换方式：待澄清。
7. 新 agent 优先级同意：WorkBuddy > Copilot > Cline/Kilo > Kiro > Trae。
8. 可在本机安装 agent 做核对；暂无账号；需要模型调用时用 `http://10.98.36.99:11444` 的 `Qwen3.8-27B`。

## (g) 单账号优先方案（2026-10-29）

### 原则
1. 主要用户每个 agent 只有**一个账号**。第一优先级是**零配置、无感**：Aieyes 自动识别 agent 本地已有的登录，显示身份和全部额度/余额项。用户不粘贴 key/cookie，不点“添加账号/扫描”，不出现 Keychain 弹窗，也不对 agent 产生副作用。
2. 被动识别**完全只读**：不刷新会轮换的 token，不写 agent 的文件，SQLite 一律用 `mode=ro` 打开且不触发 WAL checkpoint。读到 agent 写到一半的文件（解析失败）时，退回上一次快照，不报错。
3. 无法自动识别时，最多给**一个按钮**。不设计拿不到结果的无效按钮。
4. 切换账号是尽力而为：只保留已经实现、成本低的 Codex 切换。其余都不做。
5. 多账号的含义改为：**记住历史上见过的账号**（用户在 agent 内部换了登录，Aieyes 自动发现），并显示它们最后一次已知的额度。不做并发运行。
6. 通用改动：
   - `account_scan.rs:1014 scan_accounts` 改成启动时自动运行，并在凭据文件变化时自动运行。去掉扫描按钮，也去掉 candidate 的 pending 确认（`:610`）。失效提示（`:739`、`:1159`）不再要求“重新扫描”。
   - token 过期时不立即提示。改为监视凭据文件的 mtime，agent 自己刷新后 Aieyes 自动重新查询。超过 24h 仍未刷新，才显示“打开 X 一次即可刷新”。
   - 唯一的写入例外：用户**主动**点 Codex 切换时，写入 `$CODEX_HOME/.aieyes/accounts/*` 和 `.aieyes/state/home.json`（`account_scan.rs:104-106`）。
   - 唯一的“借用 agent 进程”例外：Qoder（以及 Kiro 的兜底）通过 spawn agent 自己的 CLI 查额度。该进程可能刷新 token，这等同于用户在使用 agent，属于被接受的副作用。调用要标记 `AIEYES_INTERNAL`，并限频。
   - 来自多个来源的同一账号（OpenCode、Pi 里存的 Anthropic/ChatGPT/Copilot 凭据，与原生 agent 的同一账号）按服务端身份（userId/email/accountUuid）去重。

### 各 Agent

| Agent | 自动识别 | 无配置可读额度 | 兜底操作 | 切换：保留/放弃及原因 |
|---|---|---|---|---|
| Codex | 已有：`$CODEX_HOME/auth.json`（默认 `~/.codex`，明文，无弹窗），从 id_token 解出 email/plan/account_id；改为首次启动自动绑定 | 5h、周窗口、rateLimitsByLimitId、credits。**沿用现有 app-server 路径**（`codex_auth/rpc.rs:225-243` 推迟刷新，`account/read` 用 `refreshToken:false`，`:289`），不另写 HTTP 查询。API key 登录只显示身份 | 没有 `~/.codex` 时：一个“运行 codex login”按钮 | **保留**：已实现、已验证、成本低。自动记住 auth.json 中出现过的每个 account_id |
| ChatGPT Work | 与 Codex 同一账号；按 `/Applications/ChatGPT.app` 与 `originator=codex_work_desktop` 识别 | 复用 Codex 快照（同一 poolId），不重复查询 | 无 | **放弃**：没有独立账号 |
| Claude Code | 自动扫描**只读** `~/.claude.json` 的 oauthAccount（Linux/Windows 另读 `~/.claude/.credentials.json`）。没有身份时显示“未登录或使用 API key”，不报错。macOS 自动扫描**不再调用** `security`（修改 `account_scan.rs:446-474`）。不读 “Claude Safe Storage”（那是桌面版的 Electron 密钥） | 有 token 时：`/api/oauth/usage`（五小时、七天、各模型分项）+ extra_usage 钱包 + limits[] 周分项。macOS 无 token 时，**主路径是本地 transcript 估算**。401 时等待 Claude 自行刷新，不主动刷新 | macOS：用户点“显示官方额度”时，才读一次 Keychain（`Claude Code-credentials`），token 只缓存在内存。依赖用户选“始终允许”；Claude 刷新 token 会重写条目，所以**可能偶尔再弹一次**（尽力而为） | **放弃**：macOS 切换要关闭全部 claude 进程并会多次弹窗；Linux CLAUDE_CONFIG_DIR 切换未实现，也不做。自动记住变化过的 accountUuid |
| Antigravity | 只读 `antigravity-oauth-token`（已扫描）；身份来自 loadCodeAssist。**本机未找到该文件，仅装 IDE 的用户能否零配置待装机验证** | retrieveUserQuotaSummary（按模型组的 5h/周）+ loadCodeAssist 的 availableCredits 与 tier。过期时不使用 refresh token | 找不到 token 时：保留现有 `agy_auth.rs` 隔离 HOME 登录作为一键兜底 | **放弃**：token 文件替换要关进程，价值低 |
| Qoder（CLI/CN/桌面/QoderWork） | `~/.qoder/.auth/user` 是加密的（依赖 `machine_id`），不解密。用现有 CLI 模式，在用户的 QODER_CONFIG_DIR 下 spawn `qodercli` 的 get_usage_info（`account_scan.rs:1418`）。桌面端 auth.dat 用 safeStorage，不读 | 月度积分、加油包、组织共享池（已实现）。这是“借用 CLI 自身刷新”的例外：本机有 17 个 `user.*.tmp` 残留，说明每个进程都会改写 user 文件。所以**只在面板打开时查询，并且间隔不少于 10 分钟**。qoder.cn 新体系暂不做 | 只装桌面版：首选一个“安装 qodercli 并登录”按钮（复用 CLI 路径），第二选项是粘贴 PAT | **可选保留**：已有多个 QODER_CONFIG_DIR 的用户，沿用按目录查询；不新建登录流程。放弃桌面 auth.dat 快照替换 |
| ZCode | **新增**读取 `${ZCODE_DATA_BASE_DIR:-~}/.zcode/v2/credentials.json`（JWT、userInfo）；现有 spec（`:145`）只有 provider_config.json。区域按登录 provider 推断 | `/api/monitor/usage/quota/limit`：5h、周、MCP 月度。先用 JWT；识别到 provider_config 里的 coding-plan key 时**直接使用，零操作**。JWT 可用性待验证（本机未装） | 无；两者都不可用时，只显示身份和本地统计 | **放弃**：共享 DB 没有账号归属。记住身份 |
| Cursor | **新增** IDE `state.vscdb`（只读 `mode=ro`）：cursorAuth/accessToken、cachedEmail、stripeMembershipType，键名待装机核实。CLI 读 `cli-config.json`（现有 spec `:155`）。**完全不读** Keychain `cursor-access-token` | Bearer 调 api2 的 GetCurrentPeriodUsage（周期与周窗口）+ 会员类型。**不拼 WorkosCursorSessionToken cookie**（等于伪造会话）。团队池去重 | 读不到时只显示“打开 Cursor 一次”的提示，以及会话统计；不做浏览器登录按钮（拿不到 cookie，是无效操作） | **放弃**：IDE 切换要在关闭时改 sqlite，太脆弱。记住 cachedEmail |
| CodeBuddy | 本地 session/user_id 元数据；环境变量或设置中的 CODEBUDDY_API_KEY | 默认显示日志中按次积分汇总的“本月已用”（已实现，零操作）。账户积分池需要 token，暂无。待验证：内置 CLI 是否提供 usage 命令（可按 Qoder 方式 spawn） | 可选进阶：Aieyes 自持浏览器登录读取积分池（与 WorkBuddy 共用）。**默认不提供** | **放弃**。记住身份 |
| WorkBuddy | 只读 `~/.workbuddy/workbuddy.db`（`mode=ro`，不触发 checkpoint；不碰 `edge-sync-mapping-v4.db` 和 `device-id`）：user_id、个人/团队 | session_usage.credit_json 汇总的“本月已用”（零操作）。套餐、免费包、付费余额需要 billing token，**不自行登录就读不到**（除非 local_storage 有未加密缓存，待装机核实） | 同 CodeBuddy：可选进阶 OAuth，默认关闭 | **放弃**：GUI 单实例，token 加密，有风控风险 |
| Kimi Code（+Kimi Work） | 只读 `KIMI_CODE_HOME`（`~/.kimi-code`）的 oauth/credentials，区域取 `region` 文件。不调用会轮换 token 的刷新接口 | `/coding/v1/usages`：5h、7d、月度、boosterWallet（已实现）；config.toml 中有 api_key 时加 Moonshot 余额。Kimi Work 复用快照 | “运行 kimi login” | **放弃**：KIMI_CODE_HOME 切换未实现，本机未装，成本未知。记住身份 |
| Grok CLI | `GROK_HOME/auth.json` 的 OIDC 条目；多条目时**自动选最近使用的条目，同时保留手动覆盖**（不直接删掉现有手选 UI）。环境变量 XAI_API_KEY | billing credits + API 预付余额（已实现）；后付费额度以后再做。不刷新 OIDC | 无 | **放弃** GROK_HOME 切换（未实现）。多个条目都作为已记住账号，只在各自 token 有效时查询 |
| OpenCode | 现有 spec（`:127`）只读 `auth.json` 与 `opencode.json(c)`。**新增**只读 `opencode.db` 的 credential 表（id, integration_id, label, value, connector_id, method_id, active, time_created, time_updated），按 `active` 选当前凭据；account_state 的列名待核对；value 是否明文待检查 | Go 5h/周/月、Zen 余额（已实现）。若 value 是明文，就用其中的 Anthropic/ChatGPT/Copilot 凭据复用上游适配器（只读，OpenCode 自己负责刷新），并**与原生 Claude/Codex/Copilot 按服务端身份去重** | 无 | **放弃** |
| DeepSeek（dsh + API） | **新增**只读 `$DSH_HOME/.credentials.yaml`（records/refs/version）**和 `.env`**；环境变量 DEEPSEEK_API_KEY | 账号 token：get_user_summary（身份 + 余额，token 不过期也不刷新）；API key：`/user/balance`（已实现）。401 时提示在 dsh 重新登录 | 无 | **放弃**：替换记录会打断 dsh 中正在进行的查询。记住身份 |
| Pi / Oh My Pi | 只读 `~/.pi/agent/auth.json`、`~/.omp/agent/auth.json`、PI_CODING_AGENT_DIR。**解析失败（本机即为空或非 JSON）当作未登录** | 没有自有额度；复用 Claude/Codex/Copilot/Gemini 适配器，按服务端身份去重 | 无 | **放弃** |
| GitHub Copilot | 依次读：环境变量 COPILOT_GITHUB_TOKEN/GH_TOKEN/GITHUB_TOKEN → `~/.config/github-copilot/apps.json`（旧版明文）→ `~/.copilot/config.json`（本机不是严格 JSON，且没有 token 键，不能依赖）。**不运行 `gh auth token`，也不读 Keychain `copilot-cli`**（会弹窗，还要起外部进程）。VS Code SecretStorage 不读 | `copilot_internal/user`：plan、重置日期、premium 配额/剩余/超额、Free 的 chat/completions。token 长期有效、不刷新。轮询间隔不少于 5 分钟 | 没有文件 token 时（新版 CLI 与只用 VS Code 的用户，是常见情况）：**一个“GitHub 授权”按钮**，走 Aieyes 自己的官方 device-code 流程，token 存进 Aieyes 自己的 Keychain 条目（不触发第三方弹窗） | **放弃**：不做切换。记住出现过的每个 GitHub login。CLI 自带的 `/user` 切换交给用户自己使用 |
| Kiro | 只读 kiro-cli `data.sqlite3` 的 auth_kv（odic/social token）与 profile ARN；IDE 读 `~/.aws/sso/cache/kiro-auth-token.json`。取最新且未过期的 token；不刷新（会轮换） | GetUsageLimits：计划积分、超额、bonus、重置日。**仅在 token 过期时**用 `kiro-cli chat --no-interactive /usage`（AIEYES_INTERNAL）兜底 | 无 | **放弃**：auth_kv 行替换和轮换回收成本高 |
| Trae / Trae CN | 待装机验证：Local Storage 的 iCubeAuthInfo 或 state.vscdb。只在不是 safeStorage 加密时只读 | 可读时：低频调用 `ide_user_ent_usage`（不少于 10 分钟）；读不到时只做会话/进程监控 | 一个打开官方用量页的链接按钮（不粘贴 cookie） | **全部放弃**：不写回认证，不改机器 ID，也不做多 `--user-data-dir` 实例 |
| Cline / Roo / Kilo | Cline `~/.cline/data/settings/providers.json`（明文，只读，不刷新）；Kilo `~/.local/share/kilo/auth.json`；Roo 用 SecretStorage，只读历史；配置中的 OpenRouter key | Cline 个人与组织余额；Kilo profile 与每个组织的余额；OpenRouter `/api/v1/key` 限额与剩余 | 无；Roo 只显示会话统计 | **放弃**：切换便宜但未实现，单账号用户用不上 |

### 对 (b) 的修订
- 切换策略只保留 A 类中的 **Codex**（已实现），以及 **Qoder 现有的多 QODER_CONFIG_DIR 查询**（不新建登录流程）。B、C、F 类全部取消；D 类只保留为可选的只读额度查询（CodeBuddy/WorkBuddy 进阶 OAuth），不做 Cookie 导入；E 类只保留“识别到 key 就直接用来查额度”，不做进程注入切换。
- Credential Vault 缩小范围：只存 Aieyes 自己持有的 token（Copilot device-code token、可选的 WorkBuddy token），以及各账号的最后快照。不再回收或替换 agent 的凭据。
- 多账号额度：当前账号按正常频率查询；已记住的历史账号显示最后快照，只有在它的 token 仍可只读获取时才查询。

### 对 (d) 的修订
| 阶段 | 内容 |
|---|---|
| P0 Headless | 不变 |
| P0.5 自动识别 | 自动扫描（启动时和凭据文件变化时）、去掉扫描按钮和 pending 确认；Claude macOS 扫描不再调用 `security`；解析容错（处理半写文件和非 JSON）；SQLite 只读规范；账号记忆和最后快照；跨来源身份去重 |
| P1 零配置额度 | Claude extra_usage 与 limits[]；Antigravity 配额与 credits；ZCode credentials.json 与 JWT/key；DeepSeek .credentials.yaml/.env；OpenCode opencode.db；Pi/OpenCode 复用上游额度；Cursor state.vscdb + api2；Copilot 文件/环境变量 token + copilot_internal；Kiro GetUsageLimits；Cline/Kilo/OpenRouter；WorkBuddy/CodeBuddy 本月已用；Qoder 查询限频 |
| P2 一键兜底 | Copilot device-code；Claude macOS“显示官方额度”时读一次 Keychain；Qoder“安装 qodercli 并登录”；Antigravity 隔离 HOME 登录（已有） |
| P3 可选 | WorkBuddy/CodeBuddy 进阶 OAuth；Trae 装机核实后接入 |
| 取消 | 原 P2 配置目录切换推广、P3 文件/Keychain 切换、P4 Cookie/DB 切换 |

### 对 (e)/(f) 未决问题的处理
- f4 Copilot：**已按原则决定**，不做切换。零配置读取文件或环境变量中的 token；读不到时提供一键 device-code 授权。
- f5 WorkBuddy：**已回答**。不自行登录时只能读到“本月已用”，余额和套餐读不到（除非装机后发现 local_storage 有未加密缓存）。是否提供可选的进阶登录，见下面的问题 2。
- f6 Trae：**已按原则决定**，完全不做切换，只做零配置额度（若可读）和会话监控。
- e3、e5、e9、e11 已因“不做切换”而失效；e4 按原则改为默认允许只读读取 IDE 的 sqlite，但不读 Keychain。

剩余需要用户选择：
1. Claude Code（macOS）：要显示官方额度就必须读 Keychain，首次会弹一次窗，之后偶尔可能再弹。选项：A 默认只显示本地估算，用户点按钮才读（推荐）；B 首次启动就请求一次；C 永不读 Keychain。
2. WorkBuddy/CodeBuddy 的余额：A 只显示本地“本月已用”（推荐，零风险）；B 另外提供可选的“登录以查看余额”（需要 Aieyes 自建登录，有私有接口和风控风险）。
3. Qoder：查额度会多起一个 qodercli 进程，可能改写它的登录文件。A 只在面板打开时查，间隔不少于 10 分钟（推荐）；B 后台定时查；C 不查，只显示会话统计。

## (h) 最终决定（2026-10-29，已批准实施）

1. Claude Code（macOS）：首次启动弹一次钥匙串授权读取官方额度（选项 B）。
2. WorkBuddy / CodeBuddy：除本月已用外，提供可选“登录查看余额”（选项 B）。
3. Qoder：后台定时查询额度（选项 B）。
