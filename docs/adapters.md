# 数据接入

## 已核对的本机格式

Codex CLI 0.160.0 的 `session_meta`、`turn_context`、`event_msg/token_count` 已用本机日志核对。数据库只保存身份、时间、模型与统计字段，不保存对话正文。

Codex 限额通过 `codex app-server`（默认 stdio）的 `account/rateLimits/read` 读取，协议由 CLI 的 `generate-json-schema --experimental` 生成。避免使用旧版 CLI 不支持的 `--stdio` 别名；若旧版接口明确拒绝对象参数并返回 `expected unit`，同一连接以空参数重试一次。客户端不发送 `account/rateLimitResetCredit/consume`。Bank Reset 使用 `rateLimitResetCredits.availableCount`，明细数组长度不代表可用次数。快照保留独立的 Bank Reset 更新时间。

查询失败会显示初始化／读取限额等阶段、SSH／本地位置、自然退出码或 RPC 错误码，并识别启动参数、SSH 认证、配置、订阅登录与网络错误。stderr 有限采集并持续排空，仅显示固定分类，不回显代理凭据、远端脚本输出或服务端原始错误内容。程序主动清理子进程不记为自然退出。

Claude Code 使用 `assistant.message.usage`，按会话、请求与消息 ID 合并流式响应；普通输入、缓存读取和缓存写入分别计数。错误消息和 synthetic 模型不计入用量。

Claude Code 实时查询读取来源目录下的 `.credentials.json`；默认 macOS 配置也可以读取 `Claude Code-credentials` Keychain 项目。HTTP 读取尚未在当前环境完成联网验证。

## Antigravity 与自定义用量

目前提供标准统计 JSONL 导入与自定义限额命令；尚未实现 Antigravity 原生会话数据库或语言服务器的自动发现。在添加数据源时选择「Antigravity」。

每行一个事件；`id` 必须在同一会话内稳定。时间支持 Unix 秒或 RFC 3339：

```json
{"id":"request-001","sessionId":"session-001","timestamp":"2026-10-03T08:00:00Z","model":"provider-model-name","tokens":{"input":1000,"output":300,"cacheRead":4000,"cacheWrite":0,"reasoning":100}}
```

`input` 是未命中缓存的输入；`cacheRead`、`cacheWrite` 与它互斥。`reasoning` 是 `output` 的子集。以上事件总量为 5300 Token。

同一账号、同一 Provider 下，复制到另一台机器的相同事件会合并；每个来源仍可单独筛选。不同账号使用不同账号标识。改变已有来源的账号关联仅影响新增记录；已导入历史保留原账户归属，移除来源后也保留历史。

## 账户与数据源

「设置 → 数据源」统一管理来源和账户。在编辑器开启「作为账户」后，新建或选择同一 Agent 的账户，设置账户名称、限额查询与优先查询位置。来源和新账户一次提交；取消及保存失败不留下空账户。

- 本机和服务器登录同一账户：创建一个账户，再让两个数据源都选择它。限额按账户显示一次，记录可按任一数据源筛选。
- 本机使用两个 Codex 账户：创建两个账户，为各自的 `CODEX_HOME` 目录添加数据源并分别关联。不同账户的相同事件标识也不会合并。
- 外接 API：关闭「作为账户」。统计模型、Token、缓存和成本，不显示订阅限额；无账户来源之间不自动去重。
- API 账户也可作为命名账户管理，并关闭「显示并查询账户限额」。

旧版配置自动按 `provider + accountId` 生成账户，保留原有关联；账户可以独立重命名。新安装自动发现时，明确使用 API key 的 Codex 目录不创建订阅账户。关闭「作为账户」或更换账户关联，保留旧账户、历史事件、价格快照和导入游标，新增记录使用当前归属。核心记住来源曾使用的账户命名空间，文件重扫或 SSH 全量读取时复用该来源的原事件，避免重复计算。无账户来源之间仍不自动去重。`dashboard.sources` 的 `accountIds` 包含来源的当前及历史账户归属，支持解除关联后的历史筛选；旧客户端可忽略该新增字段。未关联账户和归档账户在数据源页的折叠区域管理；已关联账户可在来源编辑器中归档。

限额优先从账户指定的位置查询，默认优先本机；失败后尝试同账户其他启用的来源，仍失败则保留上次成功的快照。可在来源中的账户设置关闭不需要的限额。所有 Agent 通过相同的添加数据源入口接入；agy 读取所选位置当前登录账户的限额。

## 自动限额查询

Codex 使用配置目录启动 App Server；Claude Code 本机读取登录信息，远程通过 Python 3 从来源目录的 `.credentials.json` 读取 OAuth 登录状态。远程只回传额度字段。界面无需填写限额查询命令。

旧配置的 `quotaCommand` 扩展接口仍兼容，最后一行标准输出返回包含 `windows` 的 JSON；新界面只提供查询前的环境准备。

## SSH 前置命令

新建服务器及未配置 shell 时默认使用 `/bin/bash`，支持 `source` 等 Bash 语法；已保存的 shell 配置保持不变。

使用 `source` 的命令选择 `/bin/bash`。例如：

```sh
source ~/proxy-environment
```

在「数据源 → 限额查询前置命令」配置代理，例如 `export HTTPS_PROXY=http://127.0.0.1:7890`。填写后只覆盖本来源的限额查询环境；留空继承主机前置命令。主机前置命令仍用于历史和指标采集。

前置命令与后续查询处于同一 shell，导出的变量会传入后续 Python 或 Agent 进程。前置命令的标准输入与统计脚本分开，避免读取掉脚本内容。Linux 主机需要 Python 3；GPU 采集另外使用现有 `nvidia-smi`。

SSH 沿用系统配置、密钥、ssh-agent 与主机信任记录。Unix 使用短期 ControlMaster 连接复用，不更改服务器配置。未知主机先在终端通过正常 SSH 流程完成指纹确认。

## 计价

OpenRouter 请求路径为 `/api/v1/models`；读取 `pricing.prompt`、`completion`、`input_cache_read`、`input_cache_write`，归一化为 USD / Token。界面编辑价格的单位是 USD / 百万 Token。

模型自动匹配只接受完整 ID 或唯一、完全相同的模型名后缀。模型版本别名通过显式映射设置，不把未知型号替换成近似型号。

概览的成本卡片在计价覆盖不足 100% 时显示感叹号，点击进入价格设置，列出当前统计范围内每个未完全计价的模型、待计价 Token 和缺少的输入／输出／缓存读取／缓存写入维度。可同步 OpenRouter、映射已有模型，或直接填写该模型的单价；0 为明确免费，留空表示缺失。

价格列表支持按模型 ID 和名称搜索，忽略大小写；无匹配时显示空状态。保存价格或映射后自动补计缺项，已计价维度保留原单价。需要把整个历史改用当前价格时，使用「按当前价格重算」。


## agy 与 DeepSeek

`provider: "agy"` 使用 `agy --print /usage --output-format json --print-timeout 30s`，读取 `command.data.groups[].buckets[]` 的 `remaining_fraction`、`window` 和 `reset_time`。实测返回 Gemini 与 Claude/GPT 两组的 5h、7d 窗口，`num_turns` 和 Token 用量均为 0。可指定 `agyBinary`，支持 SSH 及限额查询前置命令。CLI 查询使用该位置当前登录身份，不根据数据目录切换账户；不同远程用户可配置为不同位置。

`provider: "deepseek"` 通过官方 [Get User Balance](https://api-docs.deepseek.com/api/get-user-balance) 的 `GET https://api.deepseek.com/user/balance` 查询，使用 Bearer API Key。`balances` 保存币种、总额、赠送余额和充值余额，金额以原始十进制字符串保留，`isAvailable` 反映 API 可用状态。余额不换算为百分比，不跨币种合计。

界面的 API Key 输入框只写不回显；`credentials.save` 保存到应用数据目录中的凭据文件，配置只引用路径。Unix 下目录权限为 0700、文件为 0600，文件内容未加密。也可在数据源高级设置指定现有纯文本 Key 文件；路径为空时读取 `DEEPSEEK_API_KEY`。DeepSeek 查询在本机执行并遵循数据源代理设置。agy 和 DeepSeek 来源只用于查询，不扫描登录目录或密钥文件为用量历史。

## 服务器设备和细分显示

`hosts.discover` 接收待编辑的主机配置并读取全部设备，不要求先保存。`devices` 按 `filesystems:/data`、`network:eth0` 等标识筛选；没有某组前缀代表该组全部显示，界面取消整组所有设备时使用 `组名:__none__`。CPU 总览始终保留，核心可单独选择。`details` 控制 CPU 时间分布、内存缓存、Swap、可用空间、文件系统类型、inode、IOPS、磁盘忙碌率、累计流量、网络错误、显存和温度功耗。

监控使用独立核心连接，多主机并行采样，与日志和限额读取分开。首次取两组相隔 250ms 的计数，后续按连续计数差计算速率；SSH 连接复用继续生效。网页刷新保留已展开的指标组。超过 10 秒的旧采样会显示延迟标识，失败保留上次成功结果并显示错误。

## 连接与更新

「设置 → 通用 → 连接」配置应用代理；本机来源可覆盖代理或跟随应用。指定代理可选择 HTTP、HTTPS、SOCKS5、SOCKS5H，填写 Host 与端口，自动生成地址；IPv6 自动补方括号。自定义 URL 保留高级地址输入。配置继续保存为 `mode` 与 `url`，旧地址自动回填。

检查更新固定查询 `JesmonX/Aieyes` 的最新 Release，不再填写仓库。`updates.check` 返回版本及发布页，`updates.open` 请求系统浏览器打开固定项目的最新发布页；不执行安装。

## 界面统计与状态

概览的输入是非缓存输入，缓存是读取与写入之和，与输出互不重复。缓存命中率仍按缓存读取 / 全部输入计算。概览不显示记录数量、计价覆盖文字或数据来源板块；来源筛选继续可用，导入错误在数据源页查看。

服务器顶部显示 CPU、内存和各选中 GPU 圆环，设备容量、CPU 核心、Swap 与磁盘忙碌率使用状态条。70% 起为提醒色，90% 起为高占用色；缺失值显示「—」，零容量不产生百分比。网络与磁盘吞吐直接显示速率。暂停、等待采样、正常、延迟、连接失败及部分采集失败以短状态标记。
