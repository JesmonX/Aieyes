# 原生账户凭据审计（2026-10-31）

本次核实 ZCode、DeepSeek Harness、Grok 与 Antigravity 的原生账户读取和额度查询。修复依据来自本机官方发行包、当前凭据字段形状和官方 Antigravity IDE 安装包；协议回归使用隔离目录与模拟 HTTP。没有新建付费账户、切换既有登录、刷新 OAuth、写入原生凭据或启动 IDE 登录。

## 核实来源

| 客户端 | 版本 / 文件 | 证据范围 |
| --- | --- | --- |
| ZCode | App 3.15.3、CLI 0.16.9；`Resources/app.asar`、`Resources/glm/zcode.cjs` | v2 扁平键、AES-256-GCM 编码、地区与 profile 字段、Coding Plan Key、额度 Authorization 头 |
| DeepSeek Harness | 0.2.1-rc.2；`dsh-credentials-local`、`dsh-deepseek-account-platform`、`dsh-app-boot` | v1 grant YAML、默认记录、官方 issuer、Node `util.parseEnv`、账户 summary 协议 |
| Grok | 1.0.62；官方 `grok-native` | 原生 auth.json 的 key / expires_at、OIDC、令牌刷新符号；没有逐登录 last_used 字段 |
| agy | 本机现有 Go 二进制、Keychain / CLI 文件字段形状 | 当前存储为 `token`、`auth_method`、`id_token_email`；原生 REST / proto 字段；现有 access token 已过期 |
| Antigravity IDE | 官方 2.6.0 ARM DMG；CFBundleVersion 2.6.0、编辑器基线 1.107.0 | 只读挂载检查 `out/main.js`、`extensions/agy` 与内嵌 protobuf 描述符；未运行应用 |

IDE 下载来自[官方下载页](https://antigravity.google/download)链接的更新服务。安装包执行版本为 `5526304964083712`，SHA-256 为 `7971e71ffaec88f9d9480ea2bcc80aebaee59c0ac06df6f0141ef59046b49c9c`。ARM 包仅用于源码检查，不能代替本机 Intel 或其他系统运行验收。

## ZCode

v2 credentials 的实际键是 `oauth:active_provider`、`zcodejwttoken`、`oauth:zai:user_info` 和 `oauth:bigmodel:user_info`，profile 本身是 JSON 字符串。`enc:v1` 由 base64url nonce、GCM tag、ciphertext 组成，密钥为原生环境 secret 或 home / username 回退值的 SHA-256。读取只投影必要的登录、profile 与手动 Coding Plan Key，不投影 OAuth refresh / access token。SSH 解密在远端使用其自身 home、用户名与环境。

活动登录地区决定查询区域。Z.ai 使用原生 user_id；BigModel 优先使用包含所选组织 / 项目范围的 profile id。身份哈希包含地区，避免跨地区合并。手动保存的 Coding Plan Key 保持独立账户。只有同目录、同地区且已确认相同 identity_key 的 Key 才能作为登录查询兜底；通用 providerRules Key 没有这种归属证据。

App 的 `createBigModelUsageHeaders` 原样发送 Authorization，现已移除自动添加的 Bearer 前缀，并用真实 HTTP 请求头回归验证。AES-GCM 另用 Node 产生的固定向量核对。没有有效付费登录进行联网额度验收；登录 JWT 能否直接取得各地区额度仍待真实账户验证。没有调用可能创建 / 交换业务凭据的接口。

## DeepSeek Harness

`.credentials.yaml` 必须是 v1 文档，账户登录取 `records["deepseek-account-platform/default"]` 中 `kind: grant` 的 v1 payload。设备记录与其他 grant 不视为当前账户；只对规范化后精确等于官方 `https://platform.deepseek.com` 的 issuer 启用账户查询。

原生账户 summary 使用 `x-dsh-auth-token`。只有顶层 `code: 0`、`data.biz_code: 0` 且普通与赠送钱包数组齐全时才接受余额；缺失字段不伪装为零余额。401 / 原生过期错误提示在 dsh 重新登录。`.env` 的引号、注释和最后值覆盖规则依据 Node `util.parseEnv`，不执行内容。官方 API Key 继续属于独立的 API 余额入口。

真实 grant 不包含可离线确认的稳定用户 ID。此次没有增加账户 profile 网络发现，也没有声称完成真实平台账户余额查询。

## Grok

只选择原生官方 OIDC scope `https://auth.x.ai::...` 或旧 `https://accounts.x.ai/sign-in`；拒绝相似恶意域名，refresh-only 条目不作为访问凭据。扫描保留每个账户，以真实 user_id / subject 建立身份，当前选择与已记住条目分开。

多条登录先排除已过期项，再使用明确的使用时间；缺少时使用 JWT iat、expires_at 判断顺序。当前发行包没有逐条 last_used，因此后两者只是凭据新旧的依据，不等于观察到最后一次模型使用。相同时间稳定选择；用户显式输入的令牌绕过自动选择。未找到本机真实 auth.json，未进行新登录或付费 billing 联网验收。

账户状态遵循统一的 24 小时约定：临近过期不提前标记，实际过期才记录 `token_expired_at`，持续过期满 24 小时才显示刷新提示；登录条目排序仍优先使用未过期凭据。

## Antigravity CLI 与 IDE

常规额度刷新不再执行 agy `/usage`，因为 CLI 自带 OAuth 刷新能力。Python helper 仅使用现有 access token，固定调用 Google userinfo、fetchLicenses、loadCodeAssist 和 retrieveUserQuotaSummary；Authorization 通过 curl stdin 传递，不使用重定向或 token endpoint。已知过期时不发 HTTP 请求。

新版 agy 的 id_token_email 只是邮箱提示。稳定身份来自已验证 Google userinfo ID；旧 ID token 格式仍兼容。查询前后重新读取凭据，账户变化时拒绝结果。已绑定账户若无法再确认身份，也拒绝更新额度并保留成功快照。只失败的套餐 / Credits 元数据不丢弃同一身份的有效模型组额度。

IDE 2.6.0 的 `oauthLifecycle` 将 base64 protobuf Topic 写入应用级 `ItemTable` 的 `antigravityUnifiedStateSync.oauthToken`。Topic 的 `oauthTokenInfoSentinelKey` 行含 base64 OAuthTokenInfo，字段 1 为 access token、2 为 token type、3 为 refresh token、4 为 Timestamp expiry；旧迁移行也可能是 JSON。读取器跳过 refresh token，只保留现有访问凭据和有效期。库以 `mode=ro`、`query_only` 打开，不提交写入或 checkpoint。

优先顺序为现有 CLI 存储，其次默认 IDE 存储。CLI 过期、为空、损坏或冲突不会让查询静默转向另一个 IDE 账户。IDE 目录按系统应用数据根下的 `Antigravity IDE/User/globalStorage/state.vscdb` 查找；当前目录不存在才兼容旧 `Antigravity` 目录。当前 IDE 已退出登录时，不复用旧 IDE 账户。自定义 `--user-data-dir` 和同时选择不同 CLI / IDE 账户未接入。

纯 IDE 安装没有 CLI 历史目录时，先只读探测上述存储；只有可解析、未过期的 consumer 凭据才进入身份查询。无存储时没有额度 / 身份 HTTP 请求。认证目录不创建为历史来源，正常凭据检查不创建 `.gemini`。身份确认失败时继续保留显式 `agyAuth` 登录 / 检查入口。

本机和 SSH 的自动发现均监测当前/旧版 IDE 数据库及 WAL 的元数据，即使尚无 CLI 目录或已绑定账户，首次 IDE 登录也会触发检查。SSH 指纹轮询不读取凭据内容，IDE 身份仍交给专用解码器；`pathsOnly` 请求保持明确路径范围。

现有 CLI access token 已过期，没有刷新。IDE 仅做安装包来源核查、存储投影和隔离自动发现 / 额度测试，没有启动 IDE 或验证真实新账号。Linux / Windows 路径与传输逻辑不等于这些系统上的实际 GUI 登录验收。

## 回归与交接

Python 回归覆盖原生 AES-GCM、远端解密失败脱敏、CLI / IDE 优先级、过期零 HTTP、IDE protobuf / JSON、只读 SQLite、稳定 Google 身份和元数据降级。Rust 回归覆盖 ZCode 原样 Authorization、DSH 严格钱包结构、Grok 多登录与过期排序、AGY 身份缺失拒绝更新及无 CLI 目录的 IDE-only 自动发现。

验证使用真实整合实现的临时快照，不加入缺失依赖的 stub。Rust gate 均设置 `AIEYES_AUTO_SCAN=off`；启动发现测试在自身隔离进程中按需启用。最终合入后的全工作区检查由整合分支再次执行。本报告不将模拟服务响应当作真实付费账户验收。
