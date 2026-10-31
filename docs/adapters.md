# 数据接入

## 已核对的本机格式

Codex CLI 0.160.0 的 `session_meta`、`turn_context`、`event_msg/token_count` 已用本机日志核对。数据库只保存身份、时间、模型与统计字段，不保存对话正文。

Codex 限额通过 `codex app-server`（默认 stdio）的 `account/rateLimits/read` 读取，协议由 CLI 的 `generate-json-schema --experimental` 生成。避免使用旧版 CLI 不支持的 `--stdio` 别名；若旧版接口明确拒绝对象参数并返回 `expected unit`，同一连接以空参数重试一次。客户端不发送 `account/rateLimitResetCredit/consume`。Bank Reset 使用 `rateLimitResetCredits.availableCount`，明细数组长度不代表可用次数。快照保留独立的 Bank Reset 更新时间。

查询失败会显示初始化／读取限额等阶段、SSH／本地位置、自然退出码或 RPC 错误码，并识别启动参数、SSH 认证、配置、订阅登录与网络错误。stderr 有限采集并持续排空，仅显示固定分类，不回显代理凭据、远端脚本输出或服务端原始错误内容。程序主动清理子进程不记为自然退出。

Claude Code 使用 `assistant.message.usage`，按会话、请求与消息 ID 合并流式响应；普通输入、缓存读取和缓存写入分别计数。错误消息和 synthetic 模型不计入用量。

## Claude Code 官方额度

额度读自 `GET https://api.anthropic.com/api/oauth/usage`（Bearer，`anthropic-beta: oauth-2025-04-20`）：`five_hour`、`seven_day`，以及 `seven_day_sonnet`／`seven_day_opus`／`seven_day_oauth_apps` 的 `utilization`（0–100）。以下两项按社区实现解析，本机没有账户，只用 `scripts/fixtures/claude-oauth-usage.json` 单测覆盖，尚未实测：

- `extra_usage`：`used_credits`、`monthly_limit` 按美分换算为 USD，显示为额外用量钱包（本月扣费、月度上限、余额＝上限－已用）。未启用且未扣费时不显示。
- `limits[]`：`kind == "weekly_scoped"` 的条目按 `scope.model.display_name` 显示为该模型的 7d 窗口（`percent`，缺失时用 `utilization`）；与 `seven_day_opus` 等同名窗口重复时只保留一个，其他 `kind` 忽略。

SSH 来源由 `scripts/remote_quota.py` 在远端读取 `.credentials.json`，只回传上述字段，解析与本机相同。Linux、Windows 以及 macOS 上有 `.credentials.json` 的目录直接读取该文件，不碰钥匙串。令牌返回 401 时 Aieyes 不刷新令牌，显示上次额度并提示“打开 Claude Code 一次即可刷新”。

macOS 默认目录 `~/.claude` 的登录保存在钥匙串 `Claude Code-credentials` 项目中（设置 `CLAUDE_CONFIG_DIR` 时 Claude 使用带后缀的项目名，Aieyes 不读取）：

- 账户由 `~/.claude.json` 的 `oauthAccount` 识别，不读取任何凭据；没有该字段时显示“未登录或使用 API key”。
- 首次启动后询问一次，通过 `/usr/bin/security find-generic-password -s "Claude Code-credentials" -w` 读取。系统对话框里的“始终允许”授予的是 `security`，不是 Aieyes，所以 ad-hoc 签名的构建（未设置 `MACOS_SIGNING_IDENTITY`，以及 Tauri 版的 `signingIdentity "-"`）更新后仍然有效；请选“始终允许”，只选“允许”时之后每次启动都会再问。读取前先只查条目属性（不会弹窗）；条目不存在时记为“未登录或使用 API key”，下次启动再查。
- 设置只保存 `claudeKeychain {askedAt, granted, result}`，令牌只在核心进程内存中。界面保存设置不能改写这条记录。拒绝后不再自动询问，可在 Claude 账户区点“重新授权”；两分钟内未作答不算拒绝，下次启动再问。
- macOS 原生界面每个通道一个核心进程，所以 `claudeAuth.keychain`、`claudeAuth.status` 走额度通道，与 `quotas.refresh` 在同一进程；对话框打开期间，额度查询会等待。其他进程（采样估值、自动估值，以及重启后的核心）只在已记录同意时自行读取一次，读取失败后要等 Claude 改写 `~/.claude/.credentials.json` 或 `~/.claude.json`，且距上次重读满 24 小时才再读。
- 401 后先等 Claude 自己刷新：Claude 已改写上述文件且距上次重读满 24 小时，就重读一次并重试本次查询，否则只给出上面的提示。Claude 刷新令牌会重写钥匙串条目，系统可能因此再弹一次窗（尽力而为）。
- 没有令牌时额度卡片说明原因（未授权、已拒绝或未登录），用量统计照常来自本地记录。

`cargo test -p aieyes-core claude_keychain` 用注入的读取函数覆盖“只问一次、拒绝后不再问、401 后按 24 小时节流重读”；`cargo test -p aieyes-core --test claude_keychain` 覆盖账户识别、未同意时不读钥匙串、伪造同意无效，以及 `AIEYES_AUTO_SCAN=off`（界面测试默认设置）时启动不读钥匙串；`node scripts/test-claude-quota-ui.cjs` 覆盖两种窗口的额度卡片和设置中的授权状态。测试都不读取真实钥匙串。系统弹窗只出现一次需要在有 Claude 登录的 Mac 上手工确认，尚未验证。

## Antigravity 与自定义用量

在添加数据源时选择「Antigravity」，agy 是它的 CLI 接入方式。默认从 `~/.gemini/antigravity-cli/conversations` 的 SQLite 会话数据库读取调用时间、实际模型、输入、输出和缓存 Token；本机及 SSH 均支持，只回传统计字段。IDE 原生历史采集尚未实现。也可指定标准 JSONL 目录或使用自定义限额命令。

额度使用登录机器上现有 access token 只读查询 Antigravity 原生 REST 接口；默认刷新不启动 agy，也不刷新令牌。Gemini 与 Claude/GPT 组按实际模型分别计价和学习容量倍率；未知组或模型、缺失价格、跨采样边界会阻止输出不可靠的估值，并说明原因。每个账户同时保留一段额度采样。旧 `agy` 配置、账户、用量、排序和唤醒关联自动迁移为 `antigravity`，冲突账户保留为独立身份。

每行一个事件；`id` 必须在同一会话内稳定。时间支持 Unix 秒或 RFC 3339：

```json
{"id":"request-001","sessionId":"session-001","timestamp":"2026-10-03T08:00:00Z","model":"provider-model-name","tokens":{"input":1000,"output":300,"cacheRead":4000,"cacheWrite":0,"reasoning":100}}
```

`input` 是未命中缓存的输入；`cacheRead`、`cacheWrite` 与它互斥。`reasoning` 是 `output` 的子集。以上事件总量为 5300 Token。

同一账号、同一 Provider 下，复制到另一台机器的相同事件会合并；每个来源仍可单独筛选。不同账号使用不同账号标识。改变已有来源的账号关联仅影响新增记录；已导入历史保留原账户归属，移除来源后也保留历史。

## 账户与数据源

「设置 → Agent 接入」选择并启用 Agent，在同页添加实际登录账户、设置名称、限额查询与优先查询位置，并在「机器与目录」连接设备和管理数据目录。登录取消或失败不留下空账户。详见 [Agents 与账户设置](agents-and-accounts-settings.md)。

- 本机和服务器登录同一账户：创建一个账户，再让两个数据源都选择它。限额按账户显示一次，记录可按任一数据源筛选。
- 本机使用两个 Codex 账户：创建两个账户，为各自的 `CODEX_HOME` 目录添加数据源并分别关联。不同账户的相同事件标识也不会合并。
- 外接 API：关闭「作为账户」。统计模型、Token、缓存和成本，不显示订阅限额；无账户来源之间不自动去重。
- API 账户也可作为命名账户管理，并关闭「显示并查询账户限额」。

旧版配置自动按 `provider + accountId` 生成账户，保留原有关联；账户可以独立重命名。新安装自动发现时，明确使用 API key 的 Codex 目录不创建订阅账户。关闭「作为账户」或更换账户关联，保留旧账户、历史事件、价格快照和导入游标，新增记录使用当前归属。核心记住来源曾使用的账户命名空间，文件重扫或 SSH 全量读取时复用该来源的原事件，避免重复计算。无账户来源之间仍不自动去重。`dashboard.sources` 的 `accountIds` 包含来源的当前及历史账户归属，支持解除关联后的历史筛选；旧客户端可忽略该新增字段。归档与恢复在账户页管理。

限额优先从账户指定的位置查询，默认优先本机；失败后尝试同账户其他启用的来源，仍失败则保留上次成功的快照。可在账户设置关闭不需要的限额；Antigravity 读取所选位置当前登录账户的限额。

## 自动限额查询

Codex 使用配置目录启动 App Server；Claude Code 本机读取登录信息，远程通过 Python 3 从来源目录的 `.credentials.json` 读取 OAuth 登录状态。远程只回传额度字段。界面无需填写限额查询命令。

旧配置的 `quotaCommand` 扩展接口仍兼容，最后一行标准输出返回包含 `windows` 的 JSON；新界面只提供查询前的环境准备。

## SSH 前置命令

新建服务器及未配置 shell 时默认使用 `/bin/bash`，支持 `source` 等 Bash 语法；已保存的 shell 配置保持不变。

使用 `source` 的命令选择 `/bin/bash`。例如：

```sh
source ~/proxy-environment
```

在「账户 → 设备 → 前置命令」配置代理，例如 `export HTTPS_PROXY=http://127.0.0.1:7890`。命令用于该账户的授权、切换、额度查询和唤醒；显式留空不会继承主机命令。历史同步和指标采集不执行账户命令。

前置命令与后续查询处于同一 shell，导出的变量会传入后续 Python 或 Agent 进程。前置命令的标准输入与统计脚本分开，避免读取掉脚本内容。Linux 主机需要 Python 3；GPU 采集另外使用现有 `nvidia-smi`。

SSH 沿用系统配置、密钥、ssh-agent 与主机信任记录。Unix 使用短期 ControlMaster 连接复用，不更改服务器配置。未知主机先在终端通过正常 SSH 流程完成指纹确认。

## 计价

OpenRouter 请求路径为 `/api/v1/models`；读取 `pricing.prompt`、`completion`、`input_cache_read`、`input_cache_write`，归一化为 USD / Token。界面编辑价格的单位是 USD / 百万 Token。

模型自动匹配只接受完整 ID 或唯一、完全相同的模型名后缀。模型版本别名通过显式映射设置，不把未知型号替换成近似型号。

概览的成本卡片在计价覆盖不足 100% 时显示感叹号，点击进入价格设置，列出当前统计范围内每个未完全计价的模型、待计价 Token 和缺少的输入／输出／缓存读取／缓存写入维度。可同步 OpenRouter、映射已有模型，或直接填写该模型的单价；0 为明确免费，留空表示缺失。

价格列表支持按模型 ID 和名称搜索，忽略大小写；无匹配时显示空状态。保存价格或映射后自动补计缺项，已计价维度保留原单价。需要把整个历史改用当前价格时，使用「按当前价格重算」。


## Antigravity 与 DeepSeek

`provider: "antigravity"` 通过 Python 3 / curl 调用 `loadCodeAssist` 和 `retrieveUserQuotaSummary`，投影模型组窗口、重置时间和实际返回的 AI Credits。仅使用现有 access token，已知过期时不发 HTTP 请求；不执行 `/usage`，避免 CLI 自行刷新 OAuth。支持 SSH 和限额查询前置命令；`agyBinary` 继续用于显式登录流程。查询优先使用该机器的 CLI 登录；没有 CLI 凭据时读取默认 Antigravity IDE 的只读 SQLite 登录存储，纯 IDE 安装无需创建 CLI 历史目录。已有 CLI 凭据过期、为空或损坏时不会改查其他 IDE 账户；数据目录不用于切换身份。存储格式和验证边界见[原生账户审计](native-account-audit-2026-10-31.md)。

`provider: "deepseek"` 通过官方 [Get User Balance](https://api-docs.deepseek.com/api/get-user-balance) 的 `GET https://api.deepseek.com/user/balance` 查询，使用 Bearer API Key。`balances` 保存币种、总额、赠送余额和充值余额，金额以原始十进制字符串保留，`isAvailable` 反映 API 可用状态。余额不换算为百分比，不跨币种合计。

界面的 API Key 输入框只写不回显；`credentials.save` 保存到应用数据目录中的凭据文件，配置只引用路径。Unix 下目录权限为 0700、文件为 0600，文件内容未加密。也可在数据源高级设置指定现有纯文本 Key 文件；路径为空时读取 `DEEPSEEK_API_KEY`。DeepSeek 查询在本机执行并遵循数据源代理设置。DeepSeek 来源只用于余额查询，不扫描密钥文件为用量历史；Antigravity 的会话历史接入见上文。

## 服务器设备和细分显示

`hosts.discover` 接收待编辑的主机配置并读取全部设备，不要求先保存。`devices` 按 `filesystems:/data`、`network:eth0` 等标识筛选；没有某组前缀代表该组全部显示，界面取消整组所有设备时使用 `组名:__none__`。CPU 总览始终保留，核心可单独选择。`details` 控制 CPU 时间分布、内存缓存、Swap、可用空间、文件系统类型、inode、IOPS、磁盘忙碌率、累计流量、网络错误、显存和温度功耗。

监控使用独立核心连接，多主机并行采样，与日志和限额读取分开。首次取两组相隔 250ms 的计数，后续按连续计数差计算速率；SSH 连接复用继续生效。网页刷新保留已展开的指标组。超过 10 秒的旧采样会显示延迟标识，失败保留上次成功结果并显示错误。

## 连接与更新

「设置 → 常规 → 网络代理」配置应用代理；本机来源可覆盖代理或跟随应用。指定代理可选择 HTTP、HTTPS、SOCKS5、SOCKS5H，填写 Host 与端口，自动生成地址；IPv6 自动补方括号。自定义 URL 保留高级地址输入。配置继续保存为 `mode` 与 `url`，旧地址自动回填。

更新由各平台外壳处理：macOS 使用签名 Sparkle appcast，Windows/Linux 使用签名 Tauri 更新源，支持安装与重启。核心只提供更新传输代理，不提供旧的 `updates.check` / `updates.open` RPC。

## 界面统计与状态

概览的输入是非缓存输入，缓存是读取与写入之和，与输出互不重复。缓存命中率仍按缓存读取 / 全部输入计算。概览不显示记录数量、计价覆盖文字或数据来源板块；来源筛选继续可用，导入错误在 Agent 的目录设置查看。

服务器顶部显示 CPU、内存和各选中 GPU 圆环，设备容量、CPU 核心、Swap 与磁盘忙碌率使用状态条。70% 起为提醒色，90% 起为高占用色；缺失值显示「—」，零容量不产生百分比。网络与磁盘吞吐直接显示速率。暂停、等待采样、正常、延迟、连接失败及部分采集失败以短状态标记。

## Codex 多账号

同一 home 可启用共享历史和独立账号额度。在「设置 → Agent 接入 → Codex」登录、关联和切换；详情与兼容要求见 [Codex 账号管理](codex-accounts.md)。

Antigravity 身份元数据由 `scripts/agy_identity.py` 在登录机器上读取。旧 ID token 取经过字段校验的 Google subject；新版仅保存邮箱提示时，通过 Google userinfo 的已验证 ID 确认身份。订阅优先使用 `aicode.googleapis.com/v1:fetchLicenses`，与模型组额度查询独立。身份绑定、错误降级和测试入口见 [刷新、监控与 Antigravity](refresh-monitoring-antigravity.md)。

## 更多 Agent

ZCode、Qoder、Grok、Cursor、OpenCode 的订阅类型、登录方式、计量单位和已验证范围见 [接入研究](agent-subscriptions.md)。五类均支持标准 JSONL；OpenCode 额外支持本机 SQLite v1/v2。
