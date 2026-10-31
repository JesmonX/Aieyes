# Agent 扩展 workflow 续接状态

2026-10-31，继续 Claude 会话 `325887cd-cbd5-4e59-af4b-572929ffde04`。WP0–WP16 的代码、脚本、独立审查和最终验收已完成，G1–G5 已关闭；真实账户限制仍按计划保留。

## 功能分支验收阶段的范围与流程

继续执行 [实施计划](agent-expansion-implementation-plan.md) 的 WP0–WP16，以及调研文档 (f)(g)(h) 的用户决定。单账号自动识别优先；记住历史账号及额度；无头与 Aieyes 内部运行不计会话；保留已批准的 Claude 钥匙串、Qoder 后台查询和 Buddy 可选登录方案。

功能分支验收在 `../Aieyes-agents` / `feat/agent-auth-quota` 完成。该阶段沿用分组实现、独立验证、合并和总门禁，没有提交或推送，也未改动当时另有工作的 main 工作目录；随后用户授权的本地合并见文末。

原 Claude parallel workflow 最终 `result=null`，包含实现、验证、合并及总门禁的 429 失败，不能视为完成。7 个 WP worktree 的基线树相同，改动全部保留，续接从实际文件开始。

## 已接回及修正

- 已将 WP6、WP7、WP8、WP9、WP10/WP15、WP11、WP12、WP13 和 WP14（含修复 delta）接回集成目录；最终验收见下文。WP8按原始base、WP12按捕获的checkpoint逐文件合并，冻结哈希均已核对。
- 删除中断遗留的 Kiro 编译占位代码，接入真实的 CLI/IDE 会话和凭据读取。Kiro 查询只把自身目录产生的会话标为内部会话，避免误隐藏同时新建的用户会话；远端过期账号不会调用本机 CLI。
- 修复 Trae 的 macOS 官方用量页入口，开启已接入新 agent 的自动发现。
- 无 Aieyes GitHub OAuth 应用配置时，Copilot 入口改为登录说明，不显示 OAuth 授权按钮。
- Swift 设置模型保留上游账号标识、原生连接来源和过期时间，并添加序列化往返检查，防止保存设置丢失识别信息。
- Buddy 已补可选官方轮询登录、Aieyes 自有系统凭据存储、退出登录和账单解析，以及 Web/Swift 界面。已有本月用量与可选余额分开处理；无账号时的真实接口探测没有取得成功认证响应，线上登录及账单仍未实测。
- ChatGPT Work 的 Web/Swift 用量筛选保留独立 provider，账户与额度继续复用 Codex；统一显示“Codex · ChatGPT Work”，不增加重复额度请求。补齐 ChatGPT.app 的安装检测和双端设置提示，原生截图模式也读取实际 hello 状态；安装标志不控制历史入口。
- Copilot 补齐环境变量优先级和 GitHub 数值身份；同 ID 改名保留账户，旧身份迁移必须有同一凭据的验证证据。半写配置保留历史与快照，换令牌不能借旧用户名或路径归属额度。本机和 SSH 的完整扫描链已有定向回归与独立审查。
- Pi/OMP 接入 Gemini CLI OAuth 额度，按 Google 用户和项目共享，独立于 Antigravity。OMP 读取启用的数据库凭据、监测 WAL，兼容配置/数据/Profile/XDG 根；有效凭据先于数据库来源优先级。过期身份保留、换行/换库和异常数值坏行隔离经过修复与独立复核。
- 只有剩余额度的 AI Credits 用可选 usedKnown 字段表示已用量未知，双端不再显示虚构的“已用 0”或进度条。原生数值格式化避免 Int64 最大值转 Double 后触发整数转换崩溃。
- 实际 Pi/Kimi/CodeBuddy 无头运行暴露了规则与进程关联脱节；选项位于子命令前也会漏判。已补全按会话关联、进程标题改写、DSH 锁句柄和 OMP 退出落盘识别。OpenCode 的 `serve --service` 是 TUI 自己启动的服务，已核对真实源码，保留其交互豁免。
- 自动发现复查发现 macOS 空账户列表会跳过首次登录检查，已修正并补原生回归。通配目录、非默认目录、新启用 agent、SQLite WAL 与首次远端登录原先均有漏识别，已修复；远端用每主机 300 秒的元数据指纹批次探测变化，配置变化立即重查。
- Cline 的 WorkOS token 原先误发为 DSH 请求头，已按 provider 收紧。OpenRouter 周期额度已改用相应日/周/月计数，并按服务端标志计入 BYOK。Cline/Kilo 的发现身份与余额身份已统一，Kilo 的组织 ID 不再当作用户身份。
- Buddy 生命周期复查新增 3 个先红回归，覆盖地区/企业池覆盖、停用 owner 重登录，以及退出后历史余额消失；修复后 30 项 Buddy 回归及 3 项历史额度展示回归通过。共享余额只显示一份，各产品的本月用量保留。
- WorkBuddy 的本机/SSH 用量投影已统一：按请求导入 Credits、保留首次观察时间、支持同时间戳更新和分页；模型元数据不足时显示未知，不用会话当前模型代替历史模型。
- WP0–WP5复查确认Claude拒绝钥匙串会误显示错误和不存在的估算、钱包单独出现时不能正确回退，已修复并通过2项公开RPC回归。旧快照年龄已改用数据时间，主窗口与独立设置窗口会随自动发现重绘并保留输入草稿，Web/Swift定向回归通过。远端 Antigravity 纯 IDE 首次登录也已补数据库/WAL指纹监测，13项Python回归通过。
- SSH 原生额度和上游订阅传输已补齐：新增额度助手在凭据所在主机使用令牌，逐次核验条目和身份，共用本机请求与解析规则。既有账户发现仍通过私有 core 通道传输凭据文档或投影，本次没有重写该流程。首轮传输审查的10项问题已有修复；后续 Copilot/Gemini 扩充了优先级、稳定身份、到期保留和坏行隔离回归。协议样本通过不代表付费账户实测。
- WP16按自己的checkpoint合入三文件delta，哈希逐项验证，并把14项离线脚本回归加入CI。Copilot普通/工具/续聊、Trae普通/工具、Kimi取消、Kilo权限/子任务已完成真实Qwen验证；Cline需要原生登录的ACP、JSON续聊及Kilo取消后原生未保存Token的场景保留明确限制。
- 最终真实 Trae 工具调用发现轨迹重写时丢失无头判定，已修复。两项先红回归验证同任务写空／截断、writer 退出、新任务及删除重建；19 项相关测试与最终真实普通／工具调用通过。未知新会话仍计入，未放宽 live 断言。
- 合并前备份、逐文件合并记录和日志保存在 `.local/codex-resume/`；完整恢复审计保存在 `.local/codex-workflow-audit.md`。

## 功能分支合并前最终验证

日志集中在 `.local/codex-resume/final/resumed-20261031/`。
最终 [summary.json](../.local/codex-resume/final/resumed-20261031/summary.json)
确认 12 项门禁均有成功记录，最终汇总检查也已通过，未发现源码漂移，继承的暂存区内容未变。
Rust 工作区为 **755 通过、0 失败、4 忽略**；格式、严格全工作区 Clippy、构建和
CI 均通过。契约为 130 RPC，文案为 3,513 条源码消息、3,863 条翻译。
完整 Web 500.97 秒、Native 212.44 秒均通过，Work 的浅／深色及两种窗口尺寸
四图已由实施者与根任务逐张查看，提示、换行、侧栏及零账户状态正确。

本轮修正两处测试冗余 Default、一条缺失英文消息，以及 Copilot 浏览器测试
“离线缓存用户名等于已验证身份”的旧假设。失败与修复分别保留于
`clippy-initial/`、`ci-initial/`、`web-initial/` 和对应 `*-fix.json`。
生产身份要求未降低，Copilot UI 成功分支明确使用数值身份 RPC 夹具。

最后的 Trae 修复保留于 `trae-headless/`，最终 Core、CI 和真实调用全部重跑。
`source-manifest.json` 记录 665 个代码、脚本和生成物；与已通过 Web／Native 时相比，
仅 `crates/core/src/native_sessions.rs` 变化，其余 664 个源码文件及原生应用二进制不变。
因此保留未受影响的完整 Web／Native 结果，差异与复跑依据见 `trae-fix.json`。
Work 本地截图工具改用生产 Swift 对象和自身窗口的系统合成截图，单独对最终 Core 验证，
无新增系统权限、生产外观代码修改或图片修补；精确命令、哈希及视觉记录均保留。

最终 integration 269.31 秒、lifecycle 217.75 秒均以 0 退出；逐场景结果和未实测原因见
[实施结果](agent-expansion-implementation-results.md)。Trae 普通／工具两项用量相符、
最大可见会话为 0；CodeBuddy／Kimi 生命周期通过。Cline 登录／续聊限制、Kilo
取消后没有原生 usage 等均未提升为通过。

G1 Work 安装提示、G2 Pi/OMP Gemini、G3 Copilot 来源优先、G4 GitHub 数值身份及
G5 最终记录均已关闭。[完成审计](../.local/codex-resume/final/completion-audit.md)
逐项保留 WP0–WP16 与用户 (f)(g)(h) 的证据、修正和真实账户边界。

## 历史证据与实测边界

早期 619 项、pre-ACP 715 项和 post-ACP 716 项等计数只属于各自阶段。早期 Web/Native、各工作包定向日志和合并记录继续保留，不能代替当前源码的检查。旧状态文档已存入 `.local/codex-resume/final/resumed-20261031/docs-before-final/`，初次缺口审计保存在 `.local/codex-resume/final/completion-audit-pre-gaps.md`。

指定自建 Qwen 端点用于隔离的客户端、落盘、用量和会话验证。真实 Google/GitHub 等订阅账号、Buddy 成功登录及付费余额仍按批准计划标为“未实测”；CLI 需要原生登录、缺少原生 resume 能力或取消后没有持久化 usage 的场景单独记明。脚本总退出码为 0 也不代表每个 provider/case 都已实测。

上述验收保留了当时继承的暂存内容。用户随后明确授权将 Agent 扩展及 `Aieyes` 中的现有改动一并合入 `main`。

## 本地 main 合并完成

最新主分支 v0.1.67、宣传脚本／workday 作品和 Agent 扩展已合入 `Aieyes/main`。
原有八个改动文件保存于 `56df664`，Agent 功能保存于 `079e5a2`，合并保留双方内容。
测试清单与同名调研文档的两处文本冲突已处理；主分支独有 31 个文件逐字节保留。

合并源码重新通过格式、严格 Clippy、Rust 755/0/4、构建、CI、完整 Web 与 Native。
首次 Native 焦点等待失败后，未改源码或断言，单项和完整序列复跑均通过。
宣传脚本通过语法／五时段样例检查，原生渲染生成 25 张截图；完整视频未重制。
合并后的文案计数为 3,509 条源码消息与 3,863 条翻译，契约为 130 RPC。

上述合并前验收与真实调用报告继续作为历史证据，不重写其当时的 SHA、计数或范围。
本次[合并检查](../.local/merge-main-20261031/checks.json)及
[最终合并记录](../.local/merge-main-20261031/result.json)单独保存。
这是本地合并，未推送；真实服务商登录与付费额度等未实测边界继续保留。
