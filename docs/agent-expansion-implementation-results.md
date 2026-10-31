# Agent 扩展实施结果

2026-10-31，续接 Claude 会话 `325887cd-cbd5-4e59-af4b-572929ffde04`。
功能分支验收在 `Aieyes-agents` / `feat/agent-auth-quota` 完成。下文保留合并前的验收范围，用户随后授权的本地合并见文末。
WP0–WP16 的实现、独立复核与最终验收已完成；依赖真实登录账户的项目按计划保留“未实测”。
该验收阶段未新增暂存、提交或推送，也未改动原 `Aieyes` 工作目录。

## 交付范围

按[实施计划](agent-expansion-implementation-plan.md) WP0–WP16 和
[用户决定](agent-auth-quota-research-2026-10-29.md#h-最终决定2026-10-29已批准实施)
接入单账号自动发现、历史账号与额度保留、统一计量项、会话分类及各 Agent 的原生格式。
Codex 保留已有账号切换；其他客户端读取当前登录，不新增并发多账号运行或替换客户端凭据。
Claude 首次钥匙串授权、Qoder 后台额度查询、Buddy 可选余额登录沿用已批准方案。

自动发现覆盖启动、原生凭据变化、新目录、新启用的 Agent、SQLite WAL 和 SSH 首次登录。
短暂写入失败保留已有状态，长期过期显示刷新提示；更换账号后保留旧账号和快照。
ChatGPT Work 的会话和用量可以单独选择，账户与额度统一显示为“Codex · ChatGPT Work”。
具备可靠身份的 OpenCode / Pi / OMP 上游订阅与原生客户端共享额度，不重复查询。

## 各 Agent 的交付与边界

“未实测”指本轮没有相应真实登录账户；解析器、身份隔离和错误处理的 fixture
通过不等于服务商真实额度或所有套餐已验证。自建 Qwen 调用只验证客户端、原生日志、
用量和会话状态。每种来源的细节见[订阅接入](agent-subscriptions.md)与
[原生验证记录](native-agent-verification.md)。

| Agent | 零配置来源 | 额度项 | 兜底 | 未实测或限制 |
|---|---|---|---|---|
| Codex | `CODEX_HOME/auth.json`、已管理档案 | 5h / 周与分组窗口、Credits，沿用 app-server | 原有登录、档案管理与切换 | 本轮未增加真实双账号或长期续期验收 |
| ChatGPT Work | macOS 的 ChatGPT.app 安装检测；Codex rollout 的 `originator`，缺字段的 fork 继承已知父来源 | 共享 Codex 快照 | 使用 Codex 登录 | 不创建独立账号；安装与已有来源标记经本机只读核对，继承与卸载后历史入口由回归验证 |
| Claude Code | `.claude.json` 身份、明文 credentials；macOS 首次请求 `Claude Code-credentials` | 官方 5h / 周、模型分项、extra_usage 钱包 | 拒绝授权时保留本地记录；可重新授权 | 不主动刷新 token；系统授权和付费接口不以模拟结果宣称实测 |
| Antigravity | CLI token；仅装 IDE 时读 `state.vscdb` 的 OAuth 投影 | 模型窗口、tier、availableCredits | 已有隔离 HOME 登录 | CLI 凭据存在时优先，不因失效偷偷换成 IDE 的另一个账号；真实额度未实测 |
| DeepSeek API | 已配置的官方 API Key 余额连接 | 总余额、赠送/充值余额、币种 | 保留显式 API Key 设置 | API 余额不代表任意自建模型服务；真实付费余额未实测 |
| DeepSeek Harness | `.credentials.yaml` 的原生 grant、`.env` / API Key | 账号 summary 余额或官方 API 余额 | 在 dsh 登录 | 自建 Qwen headless 已验证；官方账号余额未实测 |
| ZCode | `v2/credentials.json` 原生编码与当前 provider、provider 配置 | 地区对应的 5h / 周 / MCP 窗口 | 身份一致时才复用 coding-plan key；否则保留本地记录 | 安装包结构已核对；JWT、套餐和付费 Key 未实测 |
| Qoder | 对应目录的 `qodercli get_usage_info`；CLI / 桌面 / QoderWork 引导 | 套餐、加购、组织共享 Credits | 安装并登录 qodercli，支持官方 PAT 登录方式 | 后台至少 600 秒；新 `qoder.cn` 体系、QoderWork 登录及套餐覆盖未实测 |
| Grok | 原生 OIDC 多条目，未过期优先、按原生时间选择并保留手选 | 订阅 Credits；显式管理 Key 的 API 预付余额 | 原生登录与已有手动连接 | 自建 Qwen headless 已验证；后付费与真实订阅未实测 |
| Cursor | IDE `state.vscdb` / CLI `cli-config.json` 的可读登录 | api2 周期窗口、按需用量、团队池 | 打开 Cursor；历史可导出标准 JSONL | 不读第三方钥匙串或拼 Cookie；私有库 Token 未接入，账号操作按用户要求排除 |
| OpenCode | `auth.json`、配置及 v2 `credential` / `account_state` | Go、Zen；可识别的上游 Claude / Codex / Copilot | 使用原生 `/connect` | 自建 Qwen调用、SQLite与SSH投影已验证；付费池未实测 |
| Kimi Code / Kimi Work | Code 凭据与 region；Work `shareDir` 和私有 Code 内核日志 | Code usages / boosterWallet、Moonshot 余额；Work 同身份复用 | 原生 Kimi 登录 | Code Qwen交互/无头已验证；Work 登录与付费池未实测，不把 Work 网页 token 发给 Coding API |
| CodeBuddy Code / IDE | Code 原生日志；IDE `CodeBuddyExtension/Data/.../<uid>/history` | 本机请求 Credits 的本月估算；可选共享余额 | 可选“登录查看余额” | Code Qwen交互/无头已验证；IDE真实登录、企业余额与可选登录成功未实测 |
| Pi | `PI_CODING_AGENT_DIR` / `~/.pi/agent/auth.json` | Claude / Codex / Copilot / Gemini 上游额度，无统一 Pi 余额 | 使用原生登录或 API 配置 | Qwen交互/无头已验证；上游订阅未实测，Gemini 与 Antigravity 分池 |
| Oh My Pi | 原生目录与 Profile 的 `agent.db` 启用 OAuth 行，兼容旧 `auth.json` | 同 Pi，按服务端身份共享，Gemini 按用户及项目区分 | 使用原生登录或 API 配置 | 安装版18.12.2数据库格式已核实；Qwen无头及退出落盘关联已验证，上游订阅未实测 |
| WorkBuddy | 只读 `workbuddy.db` 的身份、会话和 request Credits | 本月本机估算；可选套餐、免费/付费包和企业余额 | 可选“登录查看余额” | 本机/SSH请求投影已验证；无每请求时间/模型，不能还原安装前精确月份或模型；真实登录未实测 |
| GitHub Copilot | 专用/通用环境变量、旧 apps/hosts 文件、CLI 明文 JSONC | Premium 请求 / AI Credits、超额、Free chat / completions | 有 Aieyes OAuth App 配置时 device flow，否则登录说明 | 1.0.105 BYOK交互/无头已验证；没有 Aieyes client id，不发布无效授权按钮；真实额度未实测 |
| Cline | CLI providers 配置与新/旧 task 格式 | 个人、组织 Credits；OpenRouter Key 限额 | 原生 Cline 登录或 BYOK | Qwen交互/无头已验证；真实个人/组织钱包未实测 |
| Kilo Code | `kilo/auth.json`、原生 SQLite session/message | 个人和组织美元余额；OpenRouter Key 限额 | 原生 Kilo 登录或 BYOK | profile 用户身份与组织 ID 分离；Qwen用量已验证，付费余额未实测 |
| Roo Code | VS Code globalStorage 的 tasks 与父任务元数据 | 无额度，只读历史和状态 | 在 Roo 中使用原有登录 | 不读取 SecretStorage；旧任务布局由 fixture 验证 |
| Kiro | CLI SQLite、AWS SSO 缓存和 IDE profile | 计划积分、超额、试用/赠送、加购与重置 | 仅本机 CLI token 过期时内部 `--v2 --no-interactive /usage` | CLI/IDE包与未登录行为已核对；真实账户、区域和企业套餐未实测 |
| Trae | IDE 可读 `iCubeAuthInfo`；CLI trajectories | 可读 token 时查官方用量 | 打开国际版官方用量页 | CLI Qwen 普通/工具调用已验证；加密 IDE 仅报告进程/会话数未知，IDE真实登录与额度未实测 |
| Trae CN | 独立 CN 目录、身份与后端 | CN 官方用量 | 打开中国版官方用量页 | 与国际版分离；中国版 IDE 真实登录与额度未实测 |
| 自定义 | 已配置的标准历史来源与命令 | 按用户配置的额度查询 | 保留原有配置入口 | 无统一原生账号协议，不参与具名客户端自动发现 |

## 影响结果准确性的修正

- 会话分类接入实际进程证据，按 PID / 启动时间、日志或锁句柄、目录与会话关系关联。
  修复进程标题覆盖 argv、参数值误当命令、OMP 退出才落盘、首次 hook 目录和 SSH 观察。
  证据不足时保留 unknown；不把一个无头进程的分类套到整个 Agent。
- Trae 轨迹在每步重写时可能短暂写空或截断，导致已知无头判定丢失。现在同一任务
  保留判定，合法新任务、实际删除和退出近期文件集合会清除旧判定；新未知会话仍计入。
  两项新增回归先失败后通过，最终真实普通／工具调用均无可见无头会话且用量一致。
- Work fork 从可核实的父记录继承 originator；普通与精确用量模式、SQL和缓存保持独立
  Codex / Work 筛选，明确的子来源优先。计费账户和父子 ID 不因显示分组改变。
  macOS 安装检测独立于 Codex CLI，并在 Web 与原生设置页显示结果；卸载应用仍保留历史入口。
- Copilot 按专用环境变量、通用环境变量、磁盘凭据的顺序选择来源；SSH 使用远端环境。
  GitHub 正整数用户 ID 确定身份，改名保留原账户。旧记录迁移需验证身份与原令牌指纹，
  半写配置保留最后快照；同路径换令牌不能把新账户的额度写进旧账户。
- Pi / OMP 增加 Gemini CLI OAuth 额度；Google 用户与项目共同确定身份，独立于
  Antigravity。OMP 读取启用的数据库凭据并监测 WAL，兼容配置根、数据根与 Profile 的
  旧 JSON。先选有效凭据，再按来源优先级去重；失效时保留已验证的历史身份和旧快照。
- Cline WorkOS token 使用自身 Bearer 规则；DSH 专用头只用于 DSH 账号模式。
  Cline / Kilo 的发现和钱包查询使用同一 profile 身份，组织 ID 不充当用户 ID。
- OpenRouter 周期限额使用同周期 usage，只有服务端明确要求才计入 BYOK；不把累计
  usage 当作当期已用。组织接口部分失败保留其他钱包，零余额不当作缺失。
- WorkBuddy 不把 `sessions.model` 写到历史请求；本机与 SSH 以稳定请求 ID 导入
  Credits，支持同时间戳修改、分页和数据库替换。月度值明确标为估算。
- Buddy 余额池按地区、用户和企业隔离；原 owner 停用后可转移唯一查询来源。
  退出 Aieyes 的余额登录仍保留旧快照，共享余额展示一次，各产品本月用量保留。
- Claude 未授权/拒绝钥匙串改为非错误提示，保留本地用量和已计算成本，不宣称存在
  未计算的剩余额度估算；钱包单独出现时也能回退到旧快照。快照年龄使用数据时间，
  不把刚失效当作刚更新。
- 只返回余额的 Gemini / Antigravity AI Credits 明确标记已用量未知。双端只显示
  服务端给出的剩余值，不把缺失用量显示为 0，也不绘制暗示全部未使用的进度条。
- 主窗口和独立设置窗口都会在自动发现新账户后重绘，保留尚未保存的输入。
  Antigravity 远端纯 IDE 登录监测当前/旧版数据库及 WAL 的元数据变化；这些库
  不进入通用 Cursor 数据库解析。Grok 在实际过期后记录时间，满 24 小时才提示。
- SSH 原生额度与上游订阅在凭据所在机器查询，共用本机的请求和解析规则；新额度
  传输只返回必要身份与响应，逐次核验选中条目。保留远端前置命令的代理环境，
  原生 Codex 仍使用自身目录的 app-server，上游 Codex 使用不刷新令牌的临时查询。
  既有账户发现仍通过私有 core 通道传输凭据文档或字段投影，没有随新额度传输一起重写。
- 通用契约覆盖审计实际调用会话分派和空 HOME 自动发现，补回曾遗漏的 WorkBuddy
  顶层入口。只读 SQLite 保留 WAL 可见性，读取与 SSH 输出不携带消息正文。

## 验证记录

此前分包与集成阶段已通过的定向检查：WorkBuddy Rust 7 项、Python远端历史14项；
Buddy 30项及历史快照展示3项；订阅解析/请求42项；全provider入口审计2项。
WP1/WP2的session70项、fork16项、真进程1项、hook4项、Work缓存回归，以及
Python observer39项、process7项、history30项通过。当前源码的整树结果另列于下表。

真实模型调用使用用户指定的 `http://10.98.36.99:11444`、`Qwen3.8-27B` 和隔离配置。
Pi、Kimi、CodeBuddy、OpenCode、Grok、DSH、OMP 七种 headless 调用均正常退出、
HTTP 200、用量与响应计数一致，实时会话为空；Pi、Kimi、CodeBuddy 的 PTY交互
能观察到活动与完成。Copilot、Cline、Kilo 的补充验证由相应实施包单独记录。

功能分支合并前验收结果：

| 检查 | 结果 |
|---|---|
| contracts / localizations 生成与一致性 | 130 RPC；3,513 条源码消息、3,863 条翻译 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | **755 通过、0 失败、4 忽略**，139.76 秒 |
| `cargo build --workspace --locked` | 通过 |
| `scripts/ci-checks.sh` | 通过，44.41 秒，含 SSH 额度、live 脚本离线回归和 47 项 Node 检查 |
| 完整 Web `npm run test:ui` | 通过，500.97 秒，含 Copilot 真实核心负例、显式 RPC 界面正例与 80 组双语布局 |
| Swift 构建与 `scripts/test-native.sh` | 通过，212.44 秒，含模型、原生窗口、额度布局和性能 |
| Work 原生设置截图 | 四图逐张复核通过：1080×800 / 760×480，浅色 / 深色，真实安装标志，0 账户、0 来源 |
| live integration / lifecycle | 两脚本退出码均为 0，分别 269.31 / 217.75 秒；各场景状态见下表 |
| 源码、差异与暂存区 | 665 个源码文件与冻结清单一致；diff 检查通过；继承的暂存内容未变 |

最终 Trae 修复后重跑了契约、文案、Core、CI 及两组真实调用。完整 Web / Native
在此修复前已通过；与对应清单相比仅 `crates/core/src/native_sessions.rs` 变化，
其余 664 个源码文件及原生应用二进制完全一致，因此保留这些未受影响的通过记录。
Work 四图使用最终 Core、已通过 Native 检查的生产 Swift 对象和真实 `MainWindowView`。
本地截图工具改用自身窗口的系统合成捕获，修正离屏截图丢失侧栏、暗色外观错误；
没有修改生产外观代码、系统权限或手工修图。

四个 Rust 忽略项中，三个是由其他测试启动的子进程 fixture 入口，一个是手动性能基准。
Copilot UI 的成功分支为已验证数值身份的 RPC 夹具，不能代替真实 GitHub 授权成功。

| 本轮真实客户端 | 已通过场景 | 保留限制 |
|---|---|---|
| Copilot、Kilo | 普通、工具、续聊；用量对账、无头排除和重复导入稳定 | 不证明官方付费额度 |
| Cline | 普通、工具；用量对账、无头排除和重复导入稳定 | 3.0.74 JSON 模式无已验证的续聊方法；ACP 三场景需要原生登录 |
| Trae CLI | 普通、工具；两项最大可见会话均为 0，用量一致、重复导入稳定 | 无可用的原生续聊方法；IDE 登录与额度未实测 |
| CodeBuddy、Kimi | 权限确认、子任务、取消；用量对账、内部会话排除和重复导入稳定 | 不证明官方账户计费 |
| Kilo 生命周期 | 权限确认、子任务 | 取消后原生未保存 usage：导入 0、HTTP 15,721；该项对账仍未实测 |
| WorkBuddy | 未实测 | 缺少隔离 BYOK 的桌面／已登录运行方法 |
| Roo、Kiro、Trae CN | 本轮脚本跳过 | runner 未找到可运行 CLI；已有安装包/schema 检查不等于客户端调用 |

这些场景使用指定 Qwen 端点与隔离 HOME，没有服务商已登录账户。Google/GitHub
订阅、Claude 系统同意与付费额度、Buddy 成功登录及余额、各 IDE 受保护登录与套餐等
边界仍按上方逐 Agent 表标为未实测；脚本退出码为 0 不覆盖这些限制。

本轮完整证据见[验收汇总](../.local/codex-resume/final/resumed-20261031/summary.json)、
[完成审计](../.local/codex-resume/final/completion-audit.md)、
[真实集成报告](../.local/codex-resume/final/resumed-20261031/live-integration/live-1793436419607117000/report.json)、
[生命周期报告](../.local/codex-resume/final/resumed-20261031/live-lifecycle/live-1793436688949600000/report.json)与
[Work 视觉复核](../.local/codex-resume/final/resumed-20261031/visual-review/work-native.json)。
合并记录、失败回归、修复差异与旧阶段日志保留在 `.local/codex-resume/`；
[workflow 记录](agent-expansion-workflow-status.md)说明续接与最终冻结依据。

## 2026-10-31 本地合并

用户追加授权后，将已验收的 Agent 扩展与最新 `main`（v0.1.67）、`Aieyes` 原有宣传脚本
及 workday 作品、调研文档一并合入本地 `Aieyes/main`。原有工作先保存为 `56df664`，
Agent 实现保存为 `079e5a2`；双方内容均保留。两处文本冲突分别按测试清单并集和
“原调研＋实施回填”处理。31 个仅来自主分支的文件保持原字节，10 个共同修改的文件合并后重新验收。

合并后的检查：Rust **755 通过、0 失败、4 忽略**；格式、严格 Clippy、构建、CI、
完整 Web（464.39 秒）及 Native（128.65 秒）通过。契约仍为 130 RPC；当前源码消息为
3,509 条，翻译为 3,863 条。Native 首次在活动窗口／编辑器焦点探针失败；源码和断言
未修改，单项复跑及完整序列复跑均通过，首次失败记录仍保留。

宣传脚本语法、五个时段的数据样例及原生渲染通过，生成 25 张 2× 截图；抽查了上午面板
和下午估值窗口。此次未重新生成完整视频，也没有把宣传样例当作真实账户或付费额度证据。
详细命令与日志见[本地合并检查](../.local/merge-main-20261031/checks.json)，
保留情况、提交和最终工作树校验见[合并记录](../.local/merge-main-20261031/result.json)。
本次仅在本地合并，未推送；前文真实登录／额度及受限场景的未实测边界保持不变。
