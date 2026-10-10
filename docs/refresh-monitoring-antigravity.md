# 刷新、监测与 Antigravity 接入

本次配置格式升级到版本 5。现有账户、数据源 ID 和历史记录保持原有归属。

## 刷新设置

| 设置 | JSON 字段 | 新安装默认值 |
| --- | --- | --- |
| Agent 限额 | `refreshSeconds` | 300 秒 |
| Agent 记录 | `historyRefreshSeconds` | 300 秒 |
| 服务器前台 | `serverForegroundRefreshSeconds` | 2 秒 |
| 服务器后台 | `serverRefreshSeconds` | 10 秒 |

升级时，原 Agent 间隔同时作为限额和记录间隔；原服务器后台间隔保留。
前台表示任一可见的详情或悬浮窗口正在显示服务器页。服务器间隔不控制 Agent 记录同步。
估值采样在建立边界时仍会主动同步关联记录，以确保限额和用量对应。
记录同步到期时先执行，避免被频繁的限额到期检查延后。

刷新进度、失败原因与重试入口位于标题旁的固定大小按钮。查询期间继续显示上一结果；其范围和错误也可在此查看。
空的限额到期检查不会清除此前失败状态。
通用设置自动保存保留当前表单和代理检测结果；相同内容不重复写入，更新准备使用同一套草稿状态判断。
macOS 模型图例显示与柱形图一致的颜色；图表点击不再显示蓝色焦点框，左右键选择仍保留。
桌面端 Agents 按机器显示独立行、开关和目录设置入口。

## 本机与文件系统

`localMonitor` 独立保存本机的 `enabled`、`metrics`、`devices`、`details`，默认启用。
本机不作为 SSH 主机写入 `hosts`，不会在 Agents 机器列表中重复出现。
CPU、内存、Swap、磁盘、网络和运行时间通过 Rust `sysinfo` 获取；不枚举用户进程。
GPU 使用可用的 `nvidia-smi`，否则显示不可用原因。某些平台没有的计数器保持缺失，不生成虚构速率。

未指定文件系统设备时采用推荐列表：隐藏 snap、efi、run 和虚拟文件系统，以及 macOS 辅助系统卷。
设备发现保留完整列表，用户可以手动选中隐藏项。
`filesystems:__all__` 表示显式全选，`filesystems:__none__` 表示清空；具体挂载点选择优先于默认规则。

## Antigravity

读取单条记录或单个数据库失败时，其他有效记录继续导入。
同步结果提供 `partial`、`issues`（文件、步骤、原因）和 `failedFiles`；有警告的文件会继续重试。
数字模型 ID 仅在同一个数据库中存在唯一、明确的实际模型映射时用于恢复模型名称。
无法确认的模型使用 `antigravity-unknown-<ID>`，保留真实 Token，保持未定价，并允许之后修正模型而不重复计数。
缺文件或损坏记录会暂停相关估值采样，保留已确认的区间。

账户和连接机器界面提供“登录 / 检查”：先用 `/usage` 验证已有登录；需要授权时通过受控终端运行 agy 自身的浏览器登录流程，支持授权网址、授权码、验证、取消和超时。
此操作仅提交 `/usage`，不提交模型推理请求。CLI 使用原机器的凭据存储，授权码和终端输出不写入 Aieyes 数据库。
取消仅结束本次授权进程。现有登录不会注销。
额度继续来自 `/usage`；身份适配器读取 agy 自己保存的 Google 登录摘要，支持 macOS Keychain 的 `gemini` / `antigravity` 条目（`go-keyring-base64:` 包装或 JSON）和 SSH 用户主目录中的 CLI 凭据文件。它不从历史采集目录推断认证位置。
订阅使用当前 CLI 的 `GET https://aicode.googleapis.com/v1:fetchLicenses`，取第一项许可的 `tierDisplayName` / `userTier`。空许可显示未知，不推断为 Free，也不使用另一套 `loadCodeAssist` 权益降级订阅。

`agyAuth.*`、`accounts.status.*` 和额度快照提供可选 `identity`（稳定 key、邮箱、订阅、检查时间、stale）和 `metadataError`；额度仍填入 `plan`。首次可靠识别绑定现有账户，默认名称替换成邮箱，自定义名称、账户 ID、历史与连接均保留。之后发现其他身份时显示实际邮箱和冲突，不把该身份额度写入原账户。不会自动合并账户或切换凭据。

订阅查询失败不影响额度刷新；仅当身份一致时保留上次订阅并标记待更新。未知身份不继承旧身份摘要。

登录会话返回成功前，将已验证的公开身份写入对应账户设备缓存，并通过 `accountStatus` 返回该状态；首次身份绑定与缓存写入在同一事务完成。成功提交同时更新认证版本，授权期间启动的旧查询不能覆盖新状态。界面直接接收该状态，再独立刷新设置和额度；关闭登录窗口不取消后续刷新，额度查询失败保留已确认身份并显示可重试的错误。`agyAuth.inspect` 仍只读取现有登录，不写入状态。

`workspaceConfirmationRequired` 明确表示 CLI 正在等待临时目录确认。终端提示增量解析，支持跨读取边界的控制字符与提示文本；提交确认后消费旧提示，同一临时目录的终端重绘不再请求确认。写入失败保留待确认状态，取消清除该状态。不会用一般的工具执行提示触发目录确认。现有轮询负责后续刷新。

适配器在凭据所在机器运行，需要 Python 3 与 curl；SSH Linux 的系统凭据读取可使用 `secret-tool`，无图形会话优先使用 CLI 文件存储。凭据、授权头与原始 API 响应不写入 Aieyes 配置、日志、数据库或 RPC；curl 的授权头通过标准输入传递。macOS 本机和配置中的 SSH 已只读验证邮箱与 Google AI Pro；其他登录模式、凭据存储或无法确定的多个身份明确显示未知。Windows 的本机凭据适配尚未验证。
本机实测既有登录有效；在隔离数据目录中读取到 17 条记录，无同步警告。

## SSH Python 兼容

原 Python 3.11 限制来自 TOML 标准库导入。现在优先使用 `tomllib`，旧版 Python 使用随助手打包的 Tomli 2.0.2；远程唤醒助手也携带同一解析器。
不要求在服务器上额外安装包，原凭据存储策略检查仍然生效。
已在配置中的 SSH 服务器 Python 3.8.10 上验证解析成功。
实机历史检查读取 64 个 Antigravity 数据库、6649 条记录，无文件读取失败；6 条记录模型身份待确认。

## 更新重启

更新准备暂停新轮询，等待当前同步、查询、采样、配置写入和授权操作。
等待超过 30 秒后列出任务及已等待时长，提供继续等待或取消。
未保存编辑提供保存、放弃、取消；失败保留输入和错误信息。取消或更新失败后恢复轮询。
覆盖目录、服务器、价格、账户连接与设置、定时唤醒、账户顺序及面板账户选择；嵌套编辑器先处理内层草稿。
桌面安装阶段保护核心、服务器、网络、配置和账户五条执行通道，保留原签名校验和系统安装授权。

## 验证入口

- `cargo test -p aieyes-core`
- `python3 scripts/test-codex-auth.py`
- `python3 scripts/test-agy-identity.py`
- `python3 scripts/test-collectors.py`
- `node scripts/test-refresh-monitoring-ui.cjs`
- `node scripts/test-updates-ui.cjs`
- `npm run test:ui --prefix apps/desktop`
- `sh scripts/test-native.sh`
- `sh scripts/build-macos.sh debug`

授权交互测试使用隔离的假 CLI；实机只验证既有登录和读取历史。
Windows/Linux 的平台原生采样和 OAuth 浏览器回调仍需在相应系统实机验证。
