# Agent 订阅、原生用量与账户接入

2026-10-19 原生客户端安装、目录、真实模型调用、Kimi 额度与额外用量钱包、Harness 官方 API 余额、Qoder CLI Credits 及剩余验收项见[原生 Agent 验证记录](native-agent-verification.md)。下文套餐研究保留 2026-10-15 核实口径。

2026-10-31 的 ZCode、Grok、DSH 原生凭据格式和查询修正见[原生账户审计](native-account-audit-2026-10-31.md)。以下使用及接口说明反映该次修正。

核实日期：2026-10-15。价格用于说明计费边界，不作为运行时额度分母；账户返回的额度、周期和套餐名称优先。

## 套餐研究

| Agent / 计费方 | 套餐与内容 | 计量边界 |
| --- | --- | --- |
| ZCode / Z.ai、智谱 | GLM Coding Plan Lite、Pro、Max；官方当前列出的 5h Credits 为 2,000 / 12,000 / 28,000，周 Credits 为 10,000 / 60,000 / 140,000。ZCode 使用登录账户的套餐。地区、促销和购买周期价格以结算页为准。 | Credits 不等于 Token 或美元；国际版与中国版凭据分别选择。 |
| Qoder 国际版 | Free；Pro $20、Pro+ $60、Ultra $200/月；对应 2,000 / 6,000 / 20,000 Credits；付费版包含 Quest、Repo Wiki 等。新用户试用 14 天、300 Credits。 | 月订阅、共享 Credits、另购包不能当作 API 余额。 |
| Qoder CN 新体系 | Pro ¥59、Pro+ ¥169、Ultra ¥559/月；2,000 / 6,000 / 20,000 Credits，跨客户端共享。 | `qoder.cn` 新账户与旧 `qoder.com.cn` 接口分开，不能把旧接口支持描述为新 CN 支持。 |
| Grok / Cursor 新用户 | 2026-10-12 发布 X Subscription：Free、Plus $8、Premium $30、Super $100、Ultra $200/月。付费档包含不同量的共享 Grok 用量；Premium 起含 Grok Bot。 | Grok 与 Cursor 内 Grok 模型共享订阅池；Cursor 第三方模型另计费。同一订阅在两个 Agent 展示不代表两份额度。 |
| Cursor / SuperGrok 存量账户 | 官方允许保留旧套餐。旧 Cursor Pro $20、Pro+ $60、Ultra $200；Teams Standard $40、Premium $120/席位/月，Enterprise 定制。 | 旧 Cursor 模型池与第三方模型池分别显示；不强制迁移，不根据套餐价格推算额度。 |
| OpenCode | 客户端可接外部模型服务商；Go $10、Go Plus $40/月，提供所列模型的用量；Zen 为按量余额。 | Go 有 5h / 周 / 月限制，模型折算不同；Zen 与 Go 分开，外部 OAuth/API 账户不等于 Go。 |

官方依据：

- [ZCode 模型与套餐](https://zcode.z.ai/en/docs/configuration)、[GLM Coding Plan](https://docs.z.ai/devpack/overview)
- [Qoder 国际定价](https://docs.qoder.com/account/pricing)、[Qoder CN 账号与订阅](https://docs.qoder.cn/product-overview/account-and-subscription)
- [X Subscription 发布](https://x.ai/news/x-subscription)、[当前价格与存量套餐说明](https://docs.x.ai/x-subscription/pricing)、[Cursor 旧套餐](https://prod.cursor.com/help/account-and-billing/pricing)
- [OpenCode Go](https://opencode.ai/v2/docs/console/go/)、[Zen](https://docs.opencode.ai/docs/zen/)

## 使用

1. 在设置启用对应 Agent，已有原生登录会自动识别。下面的添加账户/订阅表单用于显式配置或原生登录不可读时的兜底；只显示对应 Agent 已接入的方式。
2. “打开官方登录 / 用量页”在系统浏览器打开对应地区网站；登录由服务商完成。Aieyes 不接收密码、不自动购买套餐。
3. ZCode 输入所选地区 API Key；Qoder 输入用量页请求的 Cookie 头（Cursor 不再接受 Cookie，额度只来自本机 IDE / CLI 登录，见下文）；Grok 输入 CLI 登录令牌；OpenCode Go 输入 API Key。Grok 和 OpenCode Go 可明确勾选“读取本机已登录凭据”，分别读取 `GROK_HOME/auth.json`（默认 `~/.grok/auth.json`）和 `XDG_DATA_HOME/opencode/auth.json`（默认 `~/.local/share/opencode/auth.json`）。Grok 首次登录使用 `grok login`，OpenCode 使用 `/connect`。
4. Grok API 余额需 **Management API Key** 和 Team ID，不是推理 API Key；OpenCode Console 需 Cookie 和 Workspace ID。
5. 验证成功才创建账户；失败保留表单供重试。后续在账户详情更新订阅登录。凭据存在系统凭据库，配置只包含不透明引用；导出配置到另一台机器后需重新登录。

Qoder 也可选择“本机 Qoder CLI 登录”（默认选项），无需填写 Cookie。先在原生 CLI 运行 `qodercli login`；自定义 `--config-dir` 时填写同一个配置目录。留空时跟随 Aieyes 进程继承的 `QODER_CONFIG_DIR`，未设置则使用 `~/.qoder`。Aieyes 发送无模型提示的 SDK 控制请求，独立显示套餐、加购和组织共享 Credits，并绑定服务端账户身份的哈希；客户端切换账户后拒绝更新原账户的额度。本模式不复制令牌到凭据库。0 / 0 Credits 显示无可用容量，缺失或仅有会话 Credits 时显示查询不可用。协议见 [Qoder 官方额度文档](https://docs.qoder.com/cli/sdk/cost-usage)。

Qoder 的“机器与目录”显示各本机 / SSH 目录的当前登录，可以点击“刷新账户状态”重新读取。检查在对应机器运行 `get_usage_info`，使用该来源的实际配置目录；`~/` 在远端展开，账户前置命令在对应机器执行。身份取自当前响应的 `userId`，不读取历史日志或加密凭据内容。账户标识显示其哈希前 12 位，同一账户在多台机器显示相同标识；账户设备页另行标明是否与已绑定账户一致。身份识别不依赖 Credits 数值，失败保留带“上次检查”提示的旧结果。相同配置目录与执行环境在一次批量检查中共用一次 CLI 查询。

Qoder 额度在后台定时查询：Tauri 的后台轮询线程和 macOS 的调度器在面板关闭时同样运行。每次查询都会启动一次 qodercli（它可能刷新自己的登录文件，这是被接受的副作用），因此 Qoder 的额度查询、设备状态检查和原生登录复查都至少间隔 10 分钟；账户或全局间隔更短时按下限执行，设置页的查询间隔旁会提示。下限表是 `quota_schedule::floor`：Qoder、Trae / Trae CN 600 秒，Copilot、Kiro 300 秒。手动刷新不受下限限制。

只装了 Qoder 桌面版、Qoder IDE 或 QoderWork（macOS 的 `/Applications` 或 `~/Applications`）且还没有 Qoder 登录时，Qoder 账户区显示一个“安装 qodercli 并登录”按钮（已装 qodercli 但未登录时为“登录 qodercli”）。对话框给出可复制的官方安装命令 `curl -fsSL https://qoder.com/install | bash && ~/.local/bin/qodercli login`；登录时也可在 CLI 中选择 Personal Access Token（在 qoder.com/account/integrations 生成后粘贴），之后仍走本机 CLI 额度模式。“重新检测”只扫描本机的 Qoder CLI 登录。桌面端的 `auth.v1.dat` 等 safeStorage 加密登录不读取。npm 包 `@qoder-ai/qodercli` 已核实存在（1.1.81，bin `qodercli`），但官方文档已不推荐 npm 安装，而且 nvm 下的全局安装不在图形界面应用使用的 PATH 中；官方安装脚本把入口放在 `~/.local/bin/qodercli`，Aieyes 的本机查询无需登录 shell 就能找到。未登录的 qodercli 会先输出 `error: authentication_failed` 的 assistant 帧再退出，Aieyes 直接提示运行 `qodercli login`，不再重试。未实测：QoderWork 的应用名（本机未安装）、已登录时的额度响应（无账号，fixture 覆盖），以及 Windows/Linux 上的桌面版检测（目前只检测 macOS）。

目录列表和计数只包含有路径的会话来源，额度连接保留在账户设置中。读取机器身份不创建额外目录，也不改变原有历史归属或已有账户绑定。

手动导入本机登录是一次快照，客户端后续切换账户不会让监测对象跟着变。Grok 多个 OIDC 登录先选仍有效的条目；优先使用明确的使用时间，没有时依次使用 JWT `iat` 和原生 `expires_at` 判断顺序。Grok 1.0.62 没有逐条 `last_used` 字段；相同时间采用稳定顺序，显式输入令牌始终优先。原生扫描保留各账户，其余条目标为已记住；不会从同一个文件的 mtime 虚构逐账户使用时间。令牌到期需要在 Grok 重新登录，不自动刷新或改写凭据。

ZCode 扫描读取 `v2/credentials.json` 的扁平加密键（`oauth:active_provider`、`zcodejwttoken` 与对应地区 user_info），兼容旧格式；本机使用原生 AES-GCM 密钥规则，SSH 在凭据所在机器解密。地区和原生账户 / 组织 ID 共同确定身份，refresh token 不离开原生存储。保存的手动 Coding Plan Key 作为独立 API 账户。未经身份确认的同地区 Key 不作为另一个 OAuth 账户的额度兜底。

## 实现及接口边界

| 模式 | 只读请求 | 结果 |
| --- | --- | --- |
| ZCode global / cn | `GET https://api.z.ai/api/monitor/usage/quota/limit` / `https://open.bigmodel.cn/api/monitor/usage/quota/limit`，原样 `Authorization: <key>` | Credits / Token 窗口、MCP、重置时间、可用套餐名 |
| Harness account（原生连接） | `GET https://platform.deepseek.com/api/v0/users/get_user_summary`，`x-dsh-auth-token` | 普通 / 赠送钱包，CNY 与 USD 分开 |
| Qoder global / cn-legacy | `GET https://qoder.com/api/v2/me/usages/big_model_credits` / `https://qoder.com.cn/api/v2/me/usages/big_model_credits`，Cookie | totalQuota + sharedQuota，实际重置时间 |
| Cursor api2（原生连接） | `POST https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage`，Connect JSON `{}`，Bearer（IDE `state.vscdb` 或 CLI `cli-config.json` 的登录令牌） | Cursor 模型 / 其他模型百分比或套餐额度窗口；按需用量与团队共享池（美元，团队池按 teamId 只显示一次）；实际账期；IDE 缓存的 `stripeMembershipType` 作为套餐名 |
| Grok subscription | `GET https://cli-chat-proxy.grok.com/v1/billing?format=credits`，Bearer + `x-xai-token-auth` | 服务端 creditUsagePercent、实际周期与套餐 |
| Grok api | `GET https://management-api.x.ai/v1/billing/teams/{team_id}/prepaid/balance` | 有符号 USD cents 转美元，与订阅分开 |
| OpenCode go | `GET https://opencode.ai/zen/go/v1/usage`，Bearer | rolling / weekly / monthly 百分比；0.5 是 0.5% |
| OpenCode console | `GET /console/api/go/status`、`GET /console/api/billing/status`，Cookie + `x-org-id` | Go meters 与 Zen 有符号 micro-cents 余额，比例与余额分开 |
| Cline account（原生连接） | `GET https://api.cline.bot/api/v1/users/me` → `/users/{id}/balance`、`/organizations/{orgId}/balance`，Bearer `workos:` 令牌 | 个人与每个组织的预付 Credits（单位按 Credits 显示，未换算美元） |
| Kilo profile（原生连接） | `GET https://api.kilo.ai/api/profile`、`/api/profile/balance`（组织加 `x-kilocode-organizationid`），Bearer | 个人（`hasPersonalAccount=false` 时跳过）与每个组织的美元余额 |
| OpenRouter key（Cline / Kilo 配置中的 Key） | `GET https://openrouter.ai/api/v1/key`，Bearer | Key 的 limit / limit_remaining，按 limit_reset 选取当日、当周、当月或累计已用；无限额时只显示已用 |

除 [xAI Management API](https://docs.x.ai/developers/rest-api-reference/management/billing) 外，表中用量接口主要是客户端/控制台协议，不承诺稳定。协议字段参考其实际开源实现：[CodexBar 各 provider 文档](https://github.com/steipete/CodexBar/tree/main/docs)、[OpenCode Go 路由源码](https://github.com/anomalyco/opencode/blob/dev/packages/console/app/src/routes/zen/go/v1/usage.ts)。独立实现解析器；未复制第三方实现代码。

HTTP 仅访问代码中的固定 HTTPS 端点，禁用重定向，不跨区域重试凭据；响应大小有界，错误不包含响应正文或凭据。缺字段、未知协议、未知余额均报不可用，不能伪造为 0%。Console 余额查询失败不覆盖有效 Go 额度。只使用服务端实际返回的套餐标签，不从金额猜套餐。

Harness 账户令牌仅从 `.credentials.yaml` 的 v1 `records.deepseek-account-platform/default` 中读取 `kind: grant` 的 v1 payload；issuer 必须是官方平台。`refs` 和 `.env` 中的官方 API Key 仍查询独立 API 余额，`.env` 按 Node `util.parseEnv` 的引号、注释规则读取，不展开或执行。账户余额必须包含成功代码及普通 / 赠送钱包数组；拒绝或过期时提示在 dsh 重新登录。此头仅适用于 DSH 账户，其他客户端的 account 模式保持其自己的认证协议。

新 X Subscription 如返回尚未识别的 Cursor 协议，会显示不可用，应从 Grok 登录查询共享池或查看官方页面；Grok 与 Cursor 共享池尚未跨 Agent 去重（OpenCode / Pi 中的上游登录按服务端身份去重，见下文）。Grok 缺少 `creditUsagePercent` 时不把 on-demand 消费上限当订阅额度。Qoder 新 `qoder.cn` API、企业组织切换、旧 Cursor 请求计数套餐尚未实现；可配置标准限额命令。

Cline 令牌读自 `~/.cline/data/settings/providers.json`（`CLINE_DATA_DIR`），Kilo 令牌读自 `$XDG_DATA_HOME/kilo/auth.json` 的 `kilo` 条目；查询时从原文件读取，只在内存中使用，不复制、不刷新（Cline 自带刷新锁）。过期超过一天的 Cline 登录标记为“打开 Cline 一次即可刷新”。组织最多查询 8 个，单个组织失败只提示，不丢弃其他钱包。字段名依据 Cline CLI 3.0.74 与 Kilo CLI 7.8.18 的发行包核对；联网结果缺少对应账户，尚未实测。

OpenRouter 的 `usage` 为累计用量，周期限额分别使用 `usage_daily`、`usage_weekly`、`usage_monthly`；仅当 `include_byok_in_limit=true` 时计入同周期的 `byok_usage_*`。旧响应缺少周期计数时，可从服务端明确给出的限额与剩余额度计算已用，不能用累计值冒充当期值。字段语义依据 [OpenRouter 官方 SDK](https://github.com/OpenRouterTeam/typescript-sdk/blob/main/docs/models/operations/getcurrentkeydata.mdx)，由 fixture 覆盖；这些 Key 限额不等于账户总余额。

## Claude Code 官方额度与本地用量

macOS 在发现默认 Claude 目录后的首次启动请求 `Claude Code-credentials` 钥匙串
授权，设置只保存是否询问及授权结果，token 仅在核心进程内存中。使用
`/usr/bin/security` 读取，避免 ad-hoc 应用更新后自身签名改变导致每次失去授权；
“始终允许”授权的是这个系统工具，范围比专门签名的 Aieyes 更宽。Claude 重写条目后
系统仍可能再次询问。拒绝后不自动重问，设置页保留“重新授权”。

有可用 token 时只读官方 5h / 周窗口、模型分项和 extra_usage 钱包，不调用刷新接口。
缺少或拒绝钥匙串授权时，额度卡显示非错误提示“仍可查看本地用量”，保留历史 Token
和已计算的 API 等价成本；不会凭 transcript Token 伪造套餐剩余比例。已有官方快照
在暂时读取失败时继续显示，即使只有钱包而没有窗口也保留，标记旧数据并显示原始
快照年龄。失效时间不当作数据刚刚更新。

## GitHub Copilot

零配置识别只读、不刷新、不写回，依次为：

- 环境变量：按 Copilot CLI 自身的优先级取第一个非空的 `COPILOT_GITHUB_TOKEN`、`GH_TOKEN`、`GITHUB_TOKEN`。通用的 `GH_TOKEN` / `GITHUB_TOKEN` 只在 Copilot 已在本机运行过（`COPILOT_HOME`，默认 `~/.copilot` 存在）时计入，避免把给其他 GitHub 工具准备的令牌当成 Copilot 账户。
- 明文文件：旧版 IDE 插件的 `$XDG_CONFIG_HOME/github-copilot/apps.json`（键为 `github.com:<app id>`）与旧 `hosts.json`；`gh` 以明文保存时的 `$GH_CONFIG_DIR/hosts.yml`（默认 `~/.config/gh`）。只取 `github.com` 条目。
- `$COPILOT_HOME/config.json`：JSONC。只有 CLI 在没有系统钥匙串、经用户同意以明文保存时才有 `copilot_tokens`；多条时取 `last_logged_in_user`。解析失败或没有令牌即视为未登录。
- 不运行 `gh auth token`，不读取 `copilot-cli` 钥匙串条目或 VS Code SecretStorage。SSH 使用远端机器自己的环境变量、HOME 和明文文件，并在该机器上查询身份和额度；Aieyes 自持授权令牌只在本机使用。

身份由 `GET https://api.github.com/user` 的正整数 `id` 确认，文件中的登录名只作为显示信息。账户身份键为 `copilot:{id}`，跨来源身份为 `github:{id}`；同一服务端 ID 改名仍复用原账户，从不同来源发现时只查询一次。同身份的环境变量来源优先于磁盘，上游客户端中的 Copilot 登录继续让原生来源优先。[GitHub 认证优先级](https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli)、[用户身份接口](https://docs.github.com/en/rest/users/users?apiVersion=2022-11-28#get-the-authenticated-user)。

旧登录名哈希记录只有在原凭据指纹仍匹配时才迁移到数值 ID，并保留原账户、名称和历史快照；换令牌后不能仅凭相同登录名或文件路径合并身份。接口不可用时可保留同一凭据已验证的身份；首次未确认的令牌保持可见但不创建账户。半写 JSON/JSONC 沿用上次状态，持续失败超过 24 小时才提示；合法空配置按退出登录保留历史。经典 `ghp_` 令牌不能读取 Copilot 额度，只提示，不建账户。

| 请求 | 结果 |
| --- | --- |
| `GET https://api.github.com/copilot_internal/user`，`Authorization: token …` 加 `Editor-Version` / `Editor-Plugin-Version`，禁用重定向；轮询间隔不少于 300 秒 | `copilot_plan` 套餐名；`quota_reset_date_utc`（缺少时用 `quota_reset_date`，Free 用 `limited_user_reset_date`）作为重置时间；`quota_snapshots` 各池转为计量项 |

- `premium_interactions`：已用 = `entitlement − quota_remaining`（缺少时用 `remaining`）。剩余为负时按 0 显示，超出部分记为超额（取 `overage_count` 与负剩余中的较大值）。`token_based_billing=true` 时这一池显示为 AI Credits（单位 credits），否则为 Premium 请求。
- `unlimited=true` 的池没有上限。只有 premium 池带 `credits_used` 时，显示已用 AI Credits（不显示上限和剩余）。
- `chat`、`completions` 只在有限额时显示（实际只有 Free）；旧版 Free 响应的 `monthly_quotas` / `limited_user_quotas` 同样转为计量项。
- 所有池都不限量时显示“当前套餐无计量额度”，不是错误。

没有可归属账户的令牌时，Copilot 账户区显示一个入口：已配置 Aieyes OAuth App 时为“GitHub 授权”，否则为“GitHub 登录说明”。

- 只有构建时提供 Aieyes 自己的 GitHub OAuth App client id（`AIEYES_GITHUB_CLIENT_ID`）才启用官方 device flow（`copilotAuth.start` / `status` / `cancel`）。令牌只存在系统凭据库的 `app.aieyes.agents` / `copilot-github`，界面只看到授权码和 GitHub 页面地址。**不借用任何其他应用（Copilot CLI、VS Code、gh 等）的 client id。**
- 当前发布构建没有这个 client id：对话框引导在终端运行 `copilot login` 或 `gh auth login`，然后点“重新检测”。macOS 上两者默认把令牌存进各自的钥匙串条目，Aieyes 不读取，因此只有 CLI 以明文保存（没有钥匙串时 Copilot CLI 会询问；`gh auth login --insecure-storage`）或设置了上面的环境变量时才能识别。

未实测（本机没有 GitHub 账户）：`copilot_internal/user` 是未公开接口，字段依据社区实现（见调研文档 (a2) 第 13 节）；Pro、Free、超额、AI Credits、不限量加 `credits_used` 的解析只用 fixture 覆盖；`GET /user` 身份查询与 device flow 只用模拟 HTTP 覆盖；Aieyes 自有 OAuth App 签发的令牌能否调用 `copilot_internal/user`，要等维护者注册 App 后核实。

会话（Copilot CLI 1.0.105，本机 npm 安装，BYOK 指向自建 OpenAI 兼容端点实测）：

- 读取 `$COPILOT_HOME/session-state/<id>/events.jsonl`；同目录的 `workspace.yaml`、`checkpoints/` 等不读取。`user.message` 为开始，`assistant.turn_start` 为思考，`tool.execution_start` / `tool.execution_complete` 为工具，不带工具请求的 `assistant.message` 为完成。
- `copilot -p` 会写入固定的 `system.message`（“You are running in non-interactive mode…”），据此判为 headless 并排除；交互 TUI 会话没有这条消息，照常计入（两者都已用真实 CLI 端到端核实）。argv 规则见 [Headless 规则](headless-rules.md)。
- 未登录时运行 `copilot` 只留下没有 `events.jsonl` 的会话目录（只有 `workspace.yaml`），不计为会话；`config.json` 只有 `firstLaunchAt`，没有令牌。
- 用量：每次模型调用一条 `session.usage_record`，`inputTokens` 视为包含缓存读写；BYOK 调用同样计入本地 Token。
- `session-store.db` 的 `sessions.host_type` 是仓库托管方（有 GitHub 远端时为 `github`，没有远端时为空），不表示宿主或无头运行，因此不参与判定。
- SSH 观察器与远端历史只传生命周期和计量字段（`scripts/session_remote.py`、`scripts/remote_history.py`），与本机解析共用同一份 fixture 测试。
- 没有识别到登录时，在 Agent 设置里为 Copilot 选择机器即可读取会话。

## 零配置补缺：OpenCode、Pi、Cursor、Kimi、CodeBuddy（2026-10-31）

以下凭据读取均为只读，不刷新会轮换的令牌，不写回客户端文件；设置与界面只保存文件定位、身份哈希和状态。SSH 账户扫描使用同一 Rust 解析器，Cursor/OpenCode 数据库仅传输凭据字段投影，不复制数据库。新增上游订阅、Cursor、Moonshot 与 Kimi Code 额度同时支持本机和 SSH：查询在凭据所在机器执行，共用请求与响应解析。本节依据安装包源码与 schema 核对；本轮没有对应服务账号，依赖账号的响应仅用协议样本覆盖，标为未实测。

SSH 额度助手只返回必要身份字段、指纹和额度响应；访问令牌留在远端，接口禁用重定向，响应大小有界。每次请求前后重新读取选中条目，凭据或身份在请求期间变化时丢弃结果，多步个人/组织钱包查询同样校验。Cline、Kilo、Copilot 和 Kimi 的首次远端发现也通过这条路径确认服务端身份。原生 Codex 沿用已有自身目录的 app-server 查询，并在前后核验身份；OpenCode/Pi/OMP 中的 Codex 登录使用临时内存凭据查询，不刷新令牌。Qoder 与 Antigravity 沿用各自的远端查询路径；Kiro 的过期 CLI 兜底仅限本机。这一传输边界只适用于新增额度助手；既有账户发现仍通过私有 core 通道传输凭据文档或字段投影，本次没有重写该流程。

**OpenCode 2 凭据表。** 只读打开 `$XDG_DATA_HOME/opencode/opencode.db`（`mode=ro`，不触发 checkpoint）的 `credential(id, integration_id, label, value, connector_id, method_id, active, time_created, time_updated)`。每个 integration 取 OpenCode 自己的当前凭据：`active=1` 的行，否则最新的行（从 auth.json 迁移来的行没有 active 标记）；`integration_id` 为空的行 OpenCode 也不使用，同样忽略。`value` 是明文 JSON：`{type:"oauth",access,refresh,expires(ms),metadata}`、`{type:"key",key}` 或不含密钥的 `{type:"external"}`。schema 与 `account_state(id, active_account_id, active_org_id)` 列名已用本机 OpenCode 2.1.0 数据库核对，值格式依据 OpenCode 源码（`packages/schema/src/credential.ts`、迁移 `import_legacy_credentials`），真实登录行未实测。

**OpenCode / Pi / Oh My Pi 中的上游订阅登录。** `anthropic`、`openai`（Pi 另有 `openai-codex`）、`github-copilot` 三类 OAuth 条目复用对应服务的额度适配器，显示在该客户端自己的账户卡上：

| 条目 | 令牌 | 身份 | 额度 |
| --- | --- | --- | --- |
| `anthropic` | `access`；`expires` 为毫秒 | `GET api.anthropic.com/api/oauth/profile` 的 `account.uuid`（与 Claude Code 的 `anthropic:` 身份相同） | `/api/oauth/usage`，与 Claude Code 同一解析 |
| `openai` / `openai-codex` | `access` | JWT 中的 ChatGPT 用户与工作区（与 Codex 的 `openai:` 身份相同），不发请求 | 在凭据所在机器使用 Codex CLI app-server，令牌以临时内存凭据（`ephemeral`）传入 Aieyes 私有目录，刷新请求一律推迟给原客户端；该机器没有 Codex CLI 时显示不可用 |
| `github-copilot` | GitHub 令牌：Pi 与旧版 OpenCode 在 `refresh`，OpenCode 2 两个字段相同；`expires` 属于派生的 Copilot 令牌，不算过期 | `GET api.github.com/user` 的数值 `id`（与 Copilot 自身引用相同的 `github:{id}` 身份） | `copilot_internal/user`；GitHub Enterprise 条目只列出，不发送 |

需请求确认的身份在令牌不变时复用（进程内按令牌指纹缓存，其次是上次识别的引用），所以繁忙的 OpenCode 数据库反复触发识别不会重复请求；识别失败时保留原账户但不更新指纹，下次读取重新确认，无法确认的新令牌不会把额度写入旧账户。同一服务端账户只查询一次：有可用且未过期的原生登录（Claude Code、Codex、Copilot）并已开启其账户额度时，优先查询原生来源；否则由持有该登录的客户端查询。OpenCode/Pi 的 ChatGPT `accountId` 或 `metadata.accountID` 选定工作区同时用于身份与 app-server 查询。Pi 与 Oh My Pi 因此开放额度能力（`quota`、`quotaReset`）。真实上游订阅与付费响应未实测。

**Pi / OMP 的 Gemini CLI 登录。** 额外支持 `google-gemini-cli` OAuth 条目，只读现有 `access`、`expires` 与 `projectId`。通过 Google userinfo 的字符串 `id` 和 Code Assist 的项目确认身份，再请求 `retrieveUserQuota`；不刷新 OAuth、不执行 onboarding 或模型调用。每个模型与 token 类别分别显示剩余比例及服务端重置时间，AI Credits 只显示报告的剩余积分，不编造已用量、总量或窗口长度。相同 Google 用户与项目在 Pi/OMP 间共享一次查询；Gemini CLI 与 Antigravity 使用独立额度池。协议依据：[OMP 18.12.2 额度适配器](https://github.com/can1357/oh-my-pi/blob/v18.12.2/packages/ai/src/usage/gemini.ts)、[Google Code Assist 类型](https://github.com/google-gemini/gemini-cli/blob/6814e00d6b148c3a1d57244d5d7040f7c541009d/packages/core/src/code_assist/types.ts)。

**OMP 当前数据库。** 18.12.2 的登录池位于原生数据目录的 `agent.db.auth_credentials`，兼容旧 `auth.json`。只读启用的受支持 OAuth 行，忽略禁用记录与其他服务的私有数据，监测数据库及 WAL 变化。原生池没有单一 current 行，Aieyes 保留每条稳定引用，再按服务端身份合并；同身份优先有效凭据，其次选择数据库来源。禁用记录、替换数据库或复用行 ID 后，不能把新凭据的额度写入旧账号。数据库格式已用安装版内嵌源码核实，本机/SSH 查询沿用同一请求与解析；真实 Google 登录与付费额度未实测。[OMP 18.12.2 凭据存储](https://github.com/can1357/oh-my-pi/blob/v18.12.2/packages/ai/src/auth/sqlite-credential-store.ts)。

**Cursor。** IDE `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`（Windows `%APPDATA%`、Linux `$XDG_CONFIG_HOME`）的 `ItemTable` 键 `cursorAuth/accessToken`、`cursorAuth/cachedEmail`、`cursorAuth/stripeMembershipType` 已用 Cursor 3.27.22 安装包核对。Cursor 3 可把访问令牌用系统钥匙串加密保存（`cursor.storage.authTokenEncryptionAtRest`，值以 `cursor-safe-storage:v1:` 开头）；该密钥从不读取，这种登录只显示邮箱，提示在 Cursor 中查看额度。会话 Cookie 模式已删除：额度只用登录令牌调用 api2 `GetCurrentPeriodUsage`（字段与 3.27.22 的 protobuf 描述一致），读不到或被拒绝时提示“打开 Cursor 一次即可刷新”。同一 `teamId` 的共享积分池在总览只显示一次，个人窗口仍各自保留。

**Kimi Code 与 Kimi Work。** `kimi login` 的令牌位于 `$KIMI_CODE_HOME/credentials/<槽位>.json`（默认槽位 `kimi-code`，国际版或自定义环境为 `kimi-code-env-<hash>`），字段为 `access_token / refresh_token / expires_at(秒)`，已用 kimi-code 2.2.0 安装包核对；Aieyes 从不调用会轮换 refresh token 的刷新接口。地区按 CLI 的顺序判断：`config.toml` 中 `managed:kimi-code` 的 `base_url` / `oauth.oauth_host`（`kimi.ai` 为国际版），其次安装渠道的 `region` 标记，默认中国版；登录令牌据此查询对应地区的 `/coding/v1/usages`，账户由同地区 `/coding/v1/me` 的 `user_id` 确认。`config.toml` 中 Moonshot 开放平台 Key 另查余额（见上表）。

Kimi Work 是 Kimi 桌面端（3.2.23，`com.moonshot.kimichat`）的工作模式。默认共享目录为 Electron 用户数据目录下的 `Kimi/daimon-share`，macOS 即 `~/Library/Application Support/Kimi/daimon-share`；`Kimi/daimon-storage.json` 的绝对 `shareDir` 可重定位。另兼容显式 `KIMI_SHARE_DIR` 与旧 `~/.kimi`。本机和 SSH 均发现其中的 `daimon/runtime/kimi-code/home`，使用同一 `kimi` provider 导入内核的 `sessions/**/wire.jsonl`，来源显示为 Kimi Work；打包内核已确认使用标准 wire 记录。`daimon/config.json` 的 `credentials.kimiWeb.userId` 仅用于身份，与同用户的 Kimi Code 账户共享快照；Work 网页令牌不发送到 Coding API，Work 内嵌 Key 也不假定能查询 Coding 额度。仅安装 Work 且没有可用 Kimi Code 登录时不会伪造额度。已验证目录重定位与身份映射，尚未用真实登录完成 Work 会话及付费额度验收。

**CodeBuddy IDE。** 国际版（4.12.1，`com.tencent.codebuddy`）与国内版（4.12.2，`com.tencent.codebuddycn`）的 Agent 扩展配置与 CLI 共用 `~/.codebuddy`（遵循 `CODEBUDDY_CONFIG_DIR`），但 IDE 历史在独立的 `CodeBuddyExtension`：macOS 为 `~/Library/Application Support/CodeBuddyExtension`，Windows 为 `~/AppData/Local/CodeBuddyExtension`，Linux 为 `~/.local/share/CodeBuddyExtension`。4.12.x 的完整布局为 `Data/<user>/<IDE>/<uid>/history/<workspace-hash>/<conversation>/index.json`。本机和 SSH 按历史所属 uid 发现来源与账户，只读 `requests` 中的聚合 Token、Credits、时间与完成状态，不读消息文件；不向 IDE 历史目录安装 CLI Hooks。`inputTokens` 含缓存读写，导入时分离；`lastTokens` 是上下文占用，不相加。每个用户请求可能含多次模型调用，缺少逐调用模型时保留 `unknown`，不推断模型成本。相同请求 ID 与原始时间的复制记录默认只计一次，累计重写原位更新，损坏文件不阻断其他有效计量。

CodeBuddy Code 2.168.2 只有 `/cost`、`/stats`、`/status`，没有账户积分查询命令，因此不按 Qoder 方式启动 CLI。额度卡显示本地请求 Credits 汇总的“本月已用”（估算）；积分池余额需要可选登录。IDE 格式、缓存口径与状态来自安装包源码，已通过本机/SSH协议回归，尚无真实 IDE 登录会话验收。

## Kimi 额外用量钱包

中国 / 国际额度查询同时读取官方 `boosterWallet`，依据[官方 CLI 协议源码](https://github.com/MoonshotAI/kimi-code/blob/main/packages/oauth/src/managed-usage.ts)。钱包 `balance.amount` / `amountLeft` 是每分 1,000,000 个定点单位；月度 `priceInCents` 是普通分。Aieyes 将两种单位分别转换为主要货币单位的精确十进制字符串，显示钱包总额、可用余额、本月扣费和月度上限。零余额保留，未提供的余额 / 上限状态显示未提供，不推断为 0 或已关闭。

钱包独立于订阅窗口百分比、会话 Token 和请求 Credits；余额为 0、月度上限为 0 不代表订阅额度不可用。不设置或修改服务端扣费上限。无效钱包不丢弃有效订阅窗口；有效钱包也可在没有订阅窗口时保存和展示。整次刷新失败时保留带失败提示的上次结果。未知类型、金额格式或矛盾币种不转换为可用余额。

`node scripts/test-kimi-wallet-ui.cjs` 使用协议样本快照、真实核心缓存 / dashboard 和两种 Web 窗口验证展示；Rust 回归另外核对原始协议、单位、精度及缓存，Swift 回归核对旧 / 新快照解码。这些测试和原生截图使用独立测试账户，真实 Kimi 付费账户的联网钱包查询尚缺对应凭据，不能用自建模型的 Token 代替。

## 历史数据

五种 Agent 均可使用 [标准 JSONL](adapters.md) 导入 Token；在机器/采集设置指定导出目录。订阅来源本身不扫描历史，也不把 Cookie/Key 文件当日志。

Qoder 原生 assistant 的 `usage.credits` 作为独立请求计量读取，保留折扣前值和 `billable` 标记；即使 Token 为 0 也保留该请求。总览、按模型和每日明细分别显示原生 Credits，区分可计费、不计费及状态未提供，并标明折扣前数值的覆盖请求数。Credits 不折算成 Token 或 API 等价成本，也不用于推算账户余额；只有 Credits 的记录不会显示虚假的零成本。Result 的累计 Credits 不重复相加。字段口径见 [Qoder 官方说明](https://docs.qoder.com/cli/sdk/cost-usage)。

OpenCode 额外支持本机 `opencode.db`：v1 `message` 和 v2 `session_message`。只取已完成 assistant 的模型、时间、Token 和稳定 ID，排除正文、未完成消息；v2 有真实数据时优先读取 v2，预建空表不会遮住仍在使用的 v1 记录。OpenCode reasoning 独立于 output，转为 Aieyes 的 output 包含 reasoning 口径；输入、缓存读取和缓存写入互斥。重复扫描幂等。本机与 SSH 使用 rowid 增量游标，每轮额外检查最多 256 条未完成请求，无变化时不读取旧行；已结算记录的人工修改通过显式 rescan 重放。新增来源遵循 `XDG_DATA_HOME/opencode`，已有来源保留明确路径。

Pi / Oh My Pi 已接入原生日志、生命周期与本机 API 配置保存，分别使用 `~/.pi/agent`、`~/.omp/agent`（支持原生环境目录和 OMP Profile）。分叉与旧会话升级保持请求身份，缓存与推理 Token 按原生口径处理。二者没有统一客户端额度，额度属于实际模型服务商；本地用量和预估费用不代表余额。真实调用、SSH 与性能结果见[原生 Agent 验收记录](native-agent-verification.md)。

OpenCode / ZCode 的 SSH SQLite 统计通过只读元数据投影读取；其他来源也可导出标准 JSONL，凭据不复制到 SSH。历史默认属于设备来源，不能由本机日志推断整个账户的剩余额度。Cursor 私有会话库未声明为已支持。新增原生路径与 SSH 实测边界见[验证记录](native-agent-verification.md)。

Cursor 新来源默认指向 `~/.cursor`，可在用户或项目的 `.cursor/hooks.json` 安装、停用或修复只读状态观察器。该路径用于生命周期事件，CLI 的 `CURSOR_CONFIG_DIR` 不迁移用户 Hooks；私有数据库中的上下文 Token 不计入请求用量。发现私有库时明确提示 Token 读取尚未接入；标准 JSONL 可放在 `~/.cursor/imports`，也可继续选择原有导出文件或目录。用户没有 Cursor 账号，已明确排除登录及依赖账号的真实事件触发验收。

参考：[OpenCode v2 SQL](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/session/sql.ts)、[消息协议](https://github.com/anomalyco/opencode/blob/dev/packages/schema/src/session-message.ts)、[Token 口径](https://github.com/anomalyco/opencode/blob/dev/packages/schema/src/token-usage.ts)。

## Trae / Trae CN 补充（2026-10-31）

已核实 macOS Trae 3.6.4 与 Trae CN 3.4.11 的应用包、客户端实现和未登录数据目录；两版的主程序均为各自 `.app/Contents/MacOS/Electron`。登录位于 `User/globalStorage/storage.json` 的 `iCubeAuthInfo://icube.cloudide` 项。Aieyes 不解密私有认证或会话存储，不刷新令牌、不写回认证，也不修改机器 ID。若现有登录以明文保存，只读取得令牌后查询该版本的 `ide_user_ent_usage`，间隔至少 600 秒；错误响应不能作为空额度接受。国际版与中国版分别打开各自官方用量页，不提供 Cookie/令牌粘贴表单。

加密 IDE 来源只报告主进程运行状态，并明确显示会话数量未知。添加 trae-cli 项目目录后，本机和 SSH 都可读取 `trajectories/*.json` 的任务状态与模型调用计量；无效轨迹会被拒绝，历史导入保留同一文件中先前已导入的任务。SSH 传输不包含提示词、回复或工具输入。

尚无真实 Trae 账户：额度响应与明文/加密登录分支由 fixture 验证，不能据此宣称已经完成真实订阅、所有套餐或 BYOK 模型调用验收。浏览器测试覆盖两个版本的官方用量按钮、无凭据表单和进程状态行。

## 验证

`cargo test --workspace --locked` 覆盖解析、缺失字段、比例单位、地区隔离、登录过期/多账户歧义、HTTP 重定向/错误脱敏、只读 SQLite、重复导入与失败建账。`node scripts/test-subscriptions-ui.cjs` 使用真实核心配置和模拟服务凭据，验证七种表单（含 Kimi Code 与 Harness）、官网打开、失败重试、取消和成功保存。该浏览器测试已纳入现有 Windows/Linux CI，现有 Core/macOS CI 检查三平台 Rust 和 Swift 编译。

Qoder 本机现有登录已完成真实额度和身份查询（套餐余额为 0 Credits）；其余付费接口的验收边界见[验证记录](native-agent-verification.md)。接口变动需要脱敏样例和真实账户回归，不能仅凭合成测试宣称兼容全部套餐。


### Kiro（CLI 与 IDE，2026-10-31 核对）

- 自动识别 `kiro-cli/data.sqlite3` 的 `auth_kv`、`state`，以及 `~/.aws/sso/cache/kiro-auth-token.json`。macOS 的 CLI DB 在 `~/Library/Application Support`；Linux 使用 `XDG_DATA_HOME` 或 `~/.local/share`。IDE 的 IAM Identity Center profile 从 `Kiro/User/globalStorage/kiro.kiroagent/profile.json` 只读补全。数据库以只读事务访问；不读 device registration secret，不保存/使用 refresh token。CLI 的 OIDC/social 两条记录选择最新有效项。
- 使用 `GetUsageLimits` 展示套餐积分、超额、有效试用/赠送、加购包及各自重置/到期时间；同一 profile 的 CLI/IDE 去重。接口失效不主动刷新 token。真实账户响应、企业订阅及区域覆盖尚未实测，解析以官方客户端消费的字段和合成 fixture 核对。
- 仅本机 CLI 持有的 token 过期时，兜底运行 `kiro-cli chat --v2 --no-interactive /usage`，每 300 秒最多一次。`--v2` 用于让 `/usage` 留在 CLI 本地命令路径；不会把它作为 V3 模型提示。进程标记 `AIEYES_INTERNAL=1`，关闭自动更新，关闭 stdin，20 秒超时，在 Aieyes 的独立目录运行，并记录本次目录下新产生的会话 ID。同期用户新建的会话不会被排除。IDE-only/SSH 登录过期时等待对应客户端刷新，不启动本机的另一个账户。
- 会话读取 CLI V2 `~/.kiro/sessions/cli/*.json`、CLI V3/IDE `~/.kiro/sessions/*/sess_*/session.json`。`KIRO_HOME` 支持绝对路径。只读元数据，消息文件只取修改时间；不会扫描 `.kiro/extensions`。CLI lock-holder 及父进程的 argv/AIEYES_INTERNAL/KIRO_API_KEY 参与无头判断；明确的 TUI 仍计入，缺乏信号默认计入。已观察到的无头判定在同一次监控生命周期中保留。
- SSH 支持相同两类会话元数据及 CLI/IDE 凭据只读识别；Linux 进程信号来自 `/proc`。本机安装核对了 kiro-cli **2.36.0** 的版本、help、无登录 `/usage`（返回登录提示，不产生会话），以及其 DB `auth_kv(key,value)` / `state(key,value)`。Kiro IDE **1.4.62** 的文件和用量字段以下载应用包核对。没有真实账号；未把 fixture 结果记为在线额度验证。未见过运行状态的历史 headless 会话没有可靠信号时仍按用户决定计入。

### Buddy 可选余额登录（WP15）

WorkBuddy 与 CodeBuddy 的账户页在“登录查看余额（可选）”内提供独立授权。
打开设置或对话框不会发起登录；用户选择中国大陆 / 国际版并点击登录后，
按官方桌面客户端的 `/v2/auth/state` → 浏览器 → `/v2/auth/token` →
`/v2/login/account` 流程获取 Aieyes 自持 token。不会读取、替换或刷新客户端的
加密登录。凭据只在系统凭据库 `app.aieyes.agents`，settings / RPC / UI 中没有 token。

登录之后只读套餐积分汇总、分页的付费包和免费包（包括当期切片）、企业积分；
共享余额只取汇总，不把汇总与明细相加。CodeBuddy 与 WorkBuddy 同地区、同用户、
同企业只保留一个余额来源，本机“本月已用”仍在原账户卡内。历史登录会保留到用户
选择“退出所有余额登录”；该操作清理 Aieyes 的登录凭据和余额来源，保留账户与
历史记录，不操作官方客户端。token 过期时提示重新登录，不轮换其他应用的 token。
后台额度查询下限 300 秒。

UID 相同但地区或企业不同的登录分别保存，不覆盖旧池的账户和余额。若原余额所属
产品已停用，或账户已归档/停止查询，从另一产品重新登录会把唯一的余额来源转移
过去；恢复原产品不会增加重复请求。各账户的最后快照仍在本地，退出余额登录后
也继续显示并标记为旧快照；同一页面只展示一份共享余额，各自本月用量仍保留。

协议依据：本机 WorkBuddy 5.5.6 `app.asar` 的 `ExternalLinkAuthenticationProvider`、
`parseResourceSummary`、`resolveResourceCapacity`、`DesktopAccountRepo.billing`。
已验证本机产品配置的中国大陆 endpoint 为 `https://copilot.tencent.com`，
platform 为 `workbuddy`。测试响应为合成数据，见 `scripts/fixtures/buddy-billing.json`。
**尚无账号实测**：本次未登录端点探测返回 HTTP 404，不能据 fixture 声称浏览器登录或
付费 / 企业额度已在线验证。国际版 endpoint、真实认证成功及服务端字段兼容仍需
账号确认。任何响应失败都不制造成功登录或零余额；页面保留错误并可重试。

WorkBuddy 本机积分从只读 `workbuddy.db.session_usage.credit_json` 的 request-id →
credits 映射提取，并按 `sessions.user_id` 隔离身份。该表只有整行 `updated_at`，
没有每个请求时间；“本月已用”是按本月更新的会话估算，跨月继续旧会话可能包含旧
请求。历史导入能记住首次观察请求的时间，但无法还原安装 Aieyes 前每笔积分的月份。
本机与 SSH 都能增量导入这些请求 Credits，重复读取同一请求不会重复累计；历史
用量包含自动化调用，但自动化会话不出现在会话监控中。`sessions.model` 仅表示
会话当前选择，不能证明旧请求的模型，因此历史 Credits 的模型显示为“未知”。
SSH 导入支持同时间戳更新、分页和数据库替换；本月额度项仍明确标为本机数据库估算。
