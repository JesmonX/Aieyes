# ZCode、Qoder、Grok、Cursor、OpenCode 接入

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

## 历史数据

五种 Agent 均可使用 [标准 JSONL](adapters.md) 导入 Token；在机器/采集设置指定导出目录。订阅来源本身不扫描历史，也不把 Cookie/Key 文件当日志。

OpenCode 额外支持本机 `opencode.db`：v1 `message` 和 v2 `session_message`。只取已完成 assistant 的模型、时间、Token 和稳定 ID，排除正文、未完成消息；v2 表存在时只读取 v2，防止迁移重复。OpenCode reasoning 独立于 output，转为 Aieyes 的 output 包含 reasoning 口径；输入、缓存读取和缓存写入互斥。重复扫描幂等。当前按 256 条分页重读已完成统计，后续可增加基于 WAL 的增量游标。自定义 XDG 目录需在采集设置指定。

原生 SQLite 读取仅支持本机。SSH 可导出标准 JSONL，凭据不复制到 SSH。历史默认属于设备来源，不能由本机日志推断整个账户的剩余额度。ZCode/Qoder/Cursor/Grok 的私有会话库未声明为已支持。

参考：[OpenCode v2 SQL](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/session/sql.ts)、[消息协议](https://github.com/anomalyco/opencode/blob/dev/packages/schema/src/session-message.ts)、[Token 口径](https://github.com/anomalyco/opencode/blob/dev/packages/schema/src/token-usage.ts)。

## 验证

`cargo test --workspace --locked` 覆盖解析、缺失字段、比例单位、地区隔离、登录过期/多账户歧义、HTTP 重定向/错误脱敏、只读 SQLite、重复导入与失败建账。`node scripts/test-subscriptions-ui.cjs` 使用真实核心配置和模拟服务凭据，验证五种表单、官网打开、失败重试、取消和成功保存。该浏览器测试已纳入现有 Windows/Linux CI，现有 Core/macOS CI 检查三平台 Rust 和 Swift 编译。

当前环境没有这五类服务的付费测试凭据；不声称已完成真实账户联网验收。接口变动需要脱敏样例和真实账户回归，不能仅凭合成测试宣称兼容全部套餐。
