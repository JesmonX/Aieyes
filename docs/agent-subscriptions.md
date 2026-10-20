# ZCode、Qoder、Grok、Cursor、OpenCode、Kimi Code、Harness 接入

2026-10-19 原生客户端安装、目录、真实模型调用、Kimi 额度与额外用量钱包、Harness 官方 API 余额、Qoder CLI Credits 及剩余验收项见[原生 Agent 验证记录](native-agent-verification.md)。下文套餐研究保留 2026-10-15 核实口径。

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

1. 在设置启用对应 Agent，点击“添加账户”。选择订阅类型/地区。
2. “打开官方登录 / 用量页”在系统浏览器打开对应地区网站；登录由服务商完成。Aieyes 不接收密码、不自动购买套餐。
3. ZCode 输入所选地区 API Key；Qoder/Cursor 输入用量页请求的 Cookie 头；Grok 输入 CLI 登录令牌；OpenCode Go 输入 API Key。Grok 和 OpenCode Go 可明确勾选“读取本机已登录凭据”，分别读取 `GROK_HOME/auth.json`（默认 `~/.grok/auth.json`）和 `XDG_DATA_HOME/opencode/auth.json`（默认 `~/.local/share/opencode/auth.json`）。Grok 首次登录使用 `grok login`，OpenCode 使用 `/connect`。
4. Grok API 余额需 **Management API Key** 和 Team ID，不是推理 API Key；OpenCode Console 需 Cookie 和 Workspace ID。
5. 验证成功才创建账户；失败保留表单供重试。后续在账户详情更新订阅登录。凭据存在系统凭据库，配置只包含不透明引用；导出配置到另一台机器后需重新登录。

Qoder 也可选择“本机 Qoder CLI 登录”（默认选项），无需填写 Cookie。先在原生 CLI 运行 `qodercli login`；自定义 `--config-dir` 时填写同一个配置目录。留空时跟随 Aieyes 进程继承的 `QODER_CONFIG_DIR`，未设置则使用 `~/.qoder`。Aieyes 发送无模型提示的 SDK 控制请求，独立显示套餐、加购和组织共享 Credits，并绑定服务端账户身份的哈希；客户端切换账户后拒绝更新原账户的额度。本模式不复制令牌到凭据库。0 / 0 Credits 显示无可用容量，缺失或仅有会话 Credits 时显示查询不可用。协议见 [Qoder 官方额度文档](https://docs.qoder.com/cli/sdk/cost-usage)。

Qoder 的“机器与目录”显示各本机 / SSH 目录的当前登录，可以点击“刷新账户状态”重新读取。检查在对应机器运行 `get_usage_info`，使用该来源的实际配置目录；`~/` 在远端展开，账户前置命令在对应机器执行。身份取自当前响应的 `userId`，不读取历史日志或加密凭据内容。账户标识显示其哈希前 12 位，同一账户在多台机器显示相同标识；账户设备页另行标明是否与已绑定账户一致。身份识别不依赖 Credits 数值，失败保留带“上次检查”提示的旧结果。相同配置目录与执行环境在一次批量检查中共用一次 CLI 查询。

目录列表和计数只包含有路径的会话来源，额度连接保留在账户设置中。读取机器身份不创建额外目录，也不改变原有历史归属或已有账户绑定。

本机登录导入是一次快照，客户端后续切换账户不会让监测对象跟着变。Grok 多个 OIDC 登录存在时拒绝猜测，要求显式输入所需令牌。令牌到期需要重新登录/连接；不刷新或改写其他客户端的凭据。凭据验证有效不代表已确认客户端当前账户身份。

## 实现及接口边界

| 模式 | 只读请求 | 结果 |
| --- | --- | --- |
| ZCode global / cn | `GET https://api.z.ai/api/monitor/usage/quota/limit` / `https://open.bigmodel.cn/api/monitor/usage/quota/limit`，Bearer | Credits / Token 窗口、MCP、重置时间、可用套餐名 |
| Qoder global / cn-legacy | `GET https://qoder.com/api/v2/me/usages/big_model_credits` / `https://qoder.com.cn/api/v2/me/usages/big_model_credits`，Cookie | totalQuota + sharedQuota，实际重置时间 |
| Cursor session | `GET https://cursor.com/api/usage-summary`，Cookie | 独立模型池、个人上限或团队共享池；实际账期 |
| Grok subscription | `GET https://cli-chat-proxy.grok.com/v1/billing?format=credits`，Bearer + `x-xai-token-auth` | 服务端 creditUsagePercent、实际周期与套餐 |
| Grok api | `GET https://management-api.x.ai/v1/billing/teams/{team_id}/prepaid/balance` | 有符号 USD cents 转美元，与订阅分开 |
| OpenCode go | `GET https://opencode.ai/zen/go/v1/usage`，Bearer | rolling / weekly / monthly 百分比；0.5 是 0.5% |
| OpenCode console | `GET /console/api/go/status`、`GET /console/api/billing/status`，Cookie + `x-org-id` | Go meters 与 Zen 有符号 micro-cents 余额，比例与余额分开 |

除 [xAI Management API](https://docs.x.ai/developers/rest-api-reference/management/billing) 外，表中用量接口主要是客户端/控制台协议，不承诺稳定。协议字段参考其实际开源实现：[CodexBar 各 provider 文档](https://github.com/steipete/CodexBar/tree/main/docs)、[OpenCode Go 路由源码](https://github.com/anomalyco/opencode/blob/dev/packages/console/app/src/routes/zen/go/v1/usage.ts)。独立实现解析器；未复制第三方实现代码。

HTTP 仅访问代码中的固定 HTTPS 端点，禁用重定向，不跨区域重试凭据；响应大小有界，错误不包含响应正文或凭据。缺字段、未知协议、未知余额均报不可用，不能伪造为 0%。Console 余额查询失败不覆盖有效 Go 额度。只使用服务端实际返回的套餐标签，不从金额猜套餐。

新 X Subscription 如返回尚未识别的 Cursor 协议，会显示不可用，应从 Grok 登录查询共享池或查看官方页面；当前没有跨 Agent 身份合并或共享池自动去重。Grok 缺少 `creditUsagePercent` 时不把 on-demand 消费上限当订阅额度。Qoder 新 `qoder.cn` API、企业组织切换、旧 Cursor 请求计数套餐尚未实现；可配置标准限额命令。

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

## 验证

`cargo test --workspace --locked` 覆盖解析、缺失字段、比例单位、地区隔离、登录过期/多账户歧义、HTTP 重定向/错误脱敏、只读 SQLite、重复导入与失败建账。`node scripts/test-subscriptions-ui.cjs` 使用真实核心配置和模拟服务凭据，验证七种表单（含 Kimi Code 与 Harness）、官网打开、失败重试、取消和成功保存。该浏览器测试已纳入现有 Windows/Linux CI，现有 Core/macOS CI 检查三平台 Rust 和 Swift 编译。

Qoder 本机现有登录已完成真实额度和身份查询（套餐余额为 0 Credits）；其余付费接口的验收边界见[验证记录](native-agent-verification.md)。接口变动需要脱敏样例和真实账户回归，不能仅凭合成测试宣称兼容全部套餐。
