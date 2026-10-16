# 全应用文案审查与实施记录

审查基线：`main@275625c`（v0.1.43）。范围包含 macOS 原生界面、Windows/Linux Web 界面、面板、菜单栏/托盘、桌宠、系统通知，以及核心和远程辅助程序回传到界面的提示。源码定位以文件和稳定状态/控件标识为准，避免后续增删行使清单失效。

## 文案规范

- 面向开发者使用简洁中文；保留 Agent、SSH、API Key、Cookie、Token、CLI、Base URL 等必要术语和服务商名称。
- 账户实体统一称“账户”；额度表示可用订阅量，限额用于上限，余额用于服务商余额；API 等价成本、额度估值及实际余额分别表达。
- 标题说明任务，按钮使用明确动词，步骤一次一个动作。默认说明保留操作必需信息，渠道特例与高级机制放在字段旁或折叠详情中。
- 状态标签与操作指引分开，更新时间独立显示。已保存、已登录、正在使用、查询成功、已部署、运行完成各有独立含义。
- 失败保留输入和重试入口。已知短错误保留具体原因；未知或过长错误使用操作摘要及可展开、可选择复制的详情。展示时遮蔽常见令牌、Cookie、密码和含凭据的 URL。
- 删除范围、切换影响的进程、远程清理残留、额度消耗等决定操作后果的信息保留在操作附近。
- 数字与单位间保留空格；设置说明使用“5 小时 / 7 天”，紧凑的服务商窗口标签保留 `5h` / `7d`。推理强度标签中文化，实际模型 ID 和参数值保持原样。

## 问题处置清单

| 编号 | 入口 / 状态 | 原问题 | 最终处置 | 验收定位 |
| --- | --- | --- | --- | --- |
| C01 | 全局账户入口、登录、切换、API | 账号 / 账户混用 | 统一为账户；用户名和密码使用独立名称 | 原生、Web、核心及回传脚本 |
| C02 | 额度刷新与分析 | 余额、限额、额度查询口径混杂 | 查询动作统一为刷新额度，保留窗口上限及余额的独立含义 | 刷新菜单、账户卡、查询设置 |
| C03 | 配置向导八目标 | 长问句与说明重复 | 任务标题、短步骤、操作入口、当前状态；独有补充折叠 | 向导八个稳定 goal ID |
| C04 | 向导验证区 | verification 重复步骤与状态 | 移除独立验证字段及区块；独有条件并入操作和更多说明 | 无 setup-verify；原生 Goal.help |
| C05 | 向导账户状态 | 配置或日志快照被误读为连接正常 | 使用明确状态标识，实时查询结果关联账户、机器与时间 | querySucceeded / pending / queryFailed |
| C06 | 向导错误缓存 | 长句混入旧数据解释 | 查询失败标签、短指引、独立数据时间、折叠说明 | queryFailed、updatedAt |
| C07 | 向导网络状态 | 未设置前置命令被推断为直连模式 | 未配置只描述保存事实；直连时无需填写的条件放入状态说明 | unconfigured / configured / reconfirm |
| C08 | 服务器采样与账户网络 | SSH、资源采样、账户查询混为验证结果 | 资源状态标为采样成功；账户查询单独验证 | sampled 及其状态说明 |
| C09 | 网络帮助 | 代理命令、SSH 跳板机制挤占主路径 | 本机与远端分别指导；命令示例折叠且不显示保存的命令内容 | network 帮助及场景切换 |
| C10 | 全局设置反馈 | 常驻解释不同保存机制 | 空闲移除提示；保存中、失败、草稿和成功时显示实际反馈 | settings-save-bar、原生设置头部 |
| C11 | 刷新字段校验 | 用中文错误文本匹配字段并跳转 | 根据内部字段 ID 定位，分别报告有效范围并保留输入 | settingsValidationField |
| C12 | 状态颜色及主题错误清除 | 依赖中文“延迟”或错误前缀 | 颜色读取新鲜度数据；主题错误按来源标识清除 | hasStaleUsage / hasStaleHosts、data-source |
| C13 | 目录配置 | 常驻文字跨页面指路 | 原文件保留的短说明；远程 Shell 与账户前置命令快捷入口 | directory-shortcuts、原生目录编辑器 |
| C14 | 目录多账户 | 跳转可能选错账户或机器 | 单账户直接打开，多账户先选择，目标机器保持一致 | data-directory-account、machineId |
| C15 | 会话监测 | 观察 Hook、接入等内部词 | 用户层统一为会话监测；保留远端 Python 3 条件及日志回退行为 | 实时监测开关、修复监测 |
| C16 | 总览与用量分析 | 无账户、无日志、无匹配数据说明混杂 | 保留不同空状态和相应动作；简化筛选与趋势范围解释 | empty、single、failure 场景 |
| C17 | 模型价格及映射 | 缺价、未计价及重算关系不清 | 明确缺价需补齐，映射为日志模型与价格模型的对应关系 | 模型价格、价格缺口、重新计价 |
| C18 | 登录与 API | 添加、保存、当前使用状态说明冗长 | 简化账户自动识别及 API 切换说明，保留进程关闭和凭据影响 | Codex、Antigravity 登录与切换 |
| C19 | 定时唤醒主说明 | 近零成本及可控制刷新时间的承诺 | 按设定时间调用订阅模型；费用取决于实际请求 | wakeup-benefit、原生任务头部 |
| C20 | 唤醒草稿与部署 | 草稿等于部署、部署等于执行的误解 | 独立状态、更新部署动作和运行记录入口 | draft、pendingSync、modified、deployed |
| C21 | 唤醒说明与参数 | 离线草稿自述常驻、Effort 未翻译 | 草稿提示移编辑器，执行规则折叠；字段显示推理强度 | 任务编辑器、执行规则 |
| C22 | 通知规则 | 非预期 / 自然重置难懂，检测条件常驻 | 提前 / 到期重置；检测条件折叠，退出和休眠条件保留 | 全局与账户提醒设置 |
| C23 | 测试通知 | 提交系统通知等同于用户已看到 | 显示“测试通知已提交，请检查系统显示” | 系统通知测试结果 |
| C24 | 估值与采样 | 估值可能被理解为可兑换余额 | 标明 API 等价价值，简化采样条件并保留影响样本的条件 | 自动估值、手动采样、credits |
| C25 | 桌宠与会话 | AGY、idle 等显示名及重复操作说明 | Antigravity、空闲动画预览；明确导入后立即使用与素材保留 | 桌宠库、会话列表及工具提示 |
| C26 | SSH 导入 | 机制长句挤占导入操作 | 沿用 SSH 别名、密钥和跳板配置；导入后可测试连接 | SSH 导入弹窗 |
| C27 | 更新与退出 | 提示长句、保存失败反馈重复 | 简化版本与重启提示；保留未保存草稿的明确处理动作 | 更新设置、更新弹窗及退出确认 |
| C28 | 错误与诊断 | 英文底层错误、长错误和可疑敏感值直接占据页面 | 共用错误摘要与详情展示，保留可定位信息并遮蔽常见凭据 | UserFacingError、AieyesUI.errorPresentation |
| C29 | 全局控件与无障碍 | 入口、提示和可访问名称术语不一致 | 同步菜单、按钮、提示、无障碍标签；保留准确的品牌和协议名 | 主窗口、面板、托盘与编辑器 |
| C30 | 面板重载 | 字体加载后滚动锚定偏移 2 像素 | 字体就绪后恢复持久化位置，保留原有精度校验 | workspace reload / scroll persistence |

## 保留项与兼容边界

- 破坏性确认中保留：账户归档与删除的区别、历史和凭据的保留范围、切换需关闭的进程、强制删除后远端任务可能继续运行。
- 采样保留：纳入所有设备用量的确认、共享目录切换账户前需结束采样、缺价与跨边界用量的影响。金额仍按现有计算口径显示。
- 网络保留：127.0.0.1 在远端指服务器自身、SSH 跳板与账户代理的区别、前置命令的作用范围。原始前置命令不进入向导。
- 产品及协议术语保留：Agent、SSH、HTTP/HTTPS、SOCKS5、API Key、Cookie、Token、Base URL、Bank Reset、provider/model ID、CLI 路径及参数值。
- 源码控制的提示统一修改；第三方及操作系统返回的未知错误保留诊断信息，通过展示层提供摘要。开发工具日志及历史发布说明不作为应用文案改写。
- RPC、配置版本、provider ID、状态码、默认提示词、计价与调度行为保持兼容。渠道提示从 docs/core-contracts.json 生成，contractHash 随规范源同步更新。

## 全量源码覆盖记录

下表列出含中文文案的应用及运行时来源。行数为含中文的源码行数（包含动态拼接、注释与默认名称，不等同于独立文案条数）。无修改的来源已经检查，保留其准确短标签、必要错误条件或技术名称。生成资源沿用规范源，测试与开发工具不混入运行时覆盖表。

### Tauri 宿主

| 来源 | 含中文行数 | 处置 |
| --- | ---: | --- |
| [apps/desktop/src-tauri/src/desktop.rs](../apps/desktop/src-tauri/src/desktop.rs) | 60 | 已核对，保留 |
| [apps/desktop/src-tauri/src/main.rs](../apps/desktop/src-tauri/src/main.rs) | 16 | 已核对，保留 |
| [apps/desktop/src-tauri/src/native_pet.rs](../apps/desktop/src-tauri/src/native_pet.rs) | 14 | 已核对，保留 |
| [apps/desktop/src-tauri/src/notifications.rs](../apps/desktop/src-tauri/src/notifications.rs) | 10 | 已修订并核对 |
| [apps/desktop/src-tauri/src/passive_window.rs](../apps/desktop/src-tauri/src/passive_window.rs) | 1 | 已核对，保留 |
| [apps/desktop/src-tauri/src/pet_sprite.rs](../apps/desktop/src-tauri/src/pet_sprite.rs) | 9 | 已核对，保留 |
| [apps/desktop/src-tauri/src/updates.rs](../apps/desktop/src-tauri/src/updates.rs) | 36 | 已修订并核对 |

### Web 界面

| 来源 | 含中文行数 | 处置 |
| --- | ---: | --- |
| [apps/desktop/web/account-targets.js](../apps/desktop/web/account-targets.js) | 2 | 已修订并核对 |
| [apps/desktop/web/agent-settings.js](../apps/desktop/web/agent-settings.js) | 90 | 已修订并核对 |
| [apps/desktop/web/analysis.js](../apps/desktop/web/analysis.js) | 6 | 已核对，保留 |
| [apps/desktop/web/app.js](../apps/desktop/web/app.js) | 206 | 已修订并核对 |
| [apps/desktop/web/automatic-estimates.js](../apps/desktop/web/automatic-estimates.js) | 20 | 已修订并核对 |
| [apps/desktop/web/chart-palettes.js](../apps/desktop/web/chart-palettes.js) | 5 | 已核对，保留 |
| [apps/desktop/web/codex-api.js](../apps/desktop/web/codex-api.js) | 32 | 已修订并核对 |
| [apps/desktop/web/codex-auth.js](../apps/desktop/web/codex-auth.js) | 25 | 已修订并核对 |
| [apps/desktop/web/contracts.js](../apps/desktop/web/contracts.js) | 12 | 已修订并核对 |
| [apps/desktop/web/desktop.js](../apps/desktop/web/desktop.js) | 7 | 已修订并核对 |
| [apps/desktop/web/floating.html](../apps/desktop/web/floating.html) | 17 | 已修订并核对 |
| [apps/desktop/web/floating.js](../apps/desktop/web/floating.js) | 8 | 已核对，保留 |
| [apps/desktop/web/index.html](../apps/desktop/web/index.html) | 6 | 已修订并核对 |
| [apps/desktop/web/multiselect.js](../apps/desktop/web/multiselect.js) | 6 | 已核对，保留 |
| [apps/desktop/web/network.js](../apps/desktop/web/network.js) | 4 | 已核对，保留 |
| [apps/desktop/web/notifications.js](../apps/desktop/web/notifications.js) | 5 | 已修订并核对 |
| [apps/desktop/web/pet-library.js](../apps/desktop/web/pet-library.js) | 20 | 已修订并核对 |
| [apps/desktop/web/pet.js](../apps/desktop/web/pet.js) | 2 | 已核对，保留 |
| [apps/desktop/web/server-renderer.js](../apps/desktop/web/server-renderer.js) | 9 | 已修订并核对 |
| [apps/desktop/web/sessions.js](../apps/desktop/web/sessions.js) | 7 | 已修订并核对 |
| [apps/desktop/web/setup-guide.js](../apps/desktop/web/setup-guide.js) | 61 | 已修订并核对 |
| [apps/desktop/web/single-select.js](../apps/desktop/web/single-select.js) | 2 | 已核对，保留 |
| [apps/desktop/web/ssh-config.js](../apps/desktop/web/ssh-config.js) | 5 | 已修订并核对 |
| [apps/desktop/web/theme.js](../apps/desktop/web/theme.js) | 2 | 已核对，保留 |
| [apps/desktop/web/ui-state.js](../apps/desktop/web/ui-state.js) | 25 | 已修订并核对 |
| [apps/desktop/web/updates.js](../apps/desktop/web/updates.js) | 21 | 已修订并核对 |
| [apps/desktop/web/wakeups.js](../apps/desktop/web/wakeups.js) | 18 | 已修订并核对 |

### macOS 原生

| 来源 | 含中文行数 | 处置 |
| --- | ---: | --- |
| [apps/macos/Sources/Aieyes/AccountTargetsView.swift](../apps/macos/Sources/Aieyes/AccountTargetsView.swift) | 5 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/AgentSettingsViews.swift](../apps/macos/Sources/Aieyes/AgentSettingsViews.swift) | 133 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/AieyesApp.swift](../apps/macos/Sources/Aieyes/AieyesApp.swift) | 34 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/AntigravityLoginView.swift](../apps/macos/Sources/Aieyes/AntigravityLoginView.swift) | 10 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/AppUpdater.swift](../apps/macos/Sources/Aieyes/AppUpdater.swift) | 41 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/AuditComponents.swift](../apps/macos/Sources/Aieyes/AuditComponents.swift) | 7 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/CodexAPIView.swift](../apps/macos/Sources/Aieyes/CodexAPIView.swift) | 44 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/CodexAccountsView.swift](../apps/macos/Sources/Aieyes/CodexAccountsView.swift) | 47 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/ConfigurationActions.swift](../apps/macos/Sources/Aieyes/ConfigurationActions.swift) | 12 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/EngineClient.swift](../apps/macos/Sources/Aieyes/EngineClient.swift) | 50 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/FloatingTokenCard.swift](../apps/macos/Sources/Aieyes/FloatingTokenCard.swift) | 4 | 已核对，保留 |
| [apps/macos/Sources/Aieyes/MainWindowView.swift](../apps/macos/Sources/Aieyes/MainWindowView.swift) | 12 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/Models.swift](../apps/macos/Sources/Aieyes/Models.swift) | 50 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/MultiSelectPicker.swift](../apps/macos/Sources/Aieyes/MultiSelectPicker.swift) | 9 | 已核对，保留 |
| [apps/macos/Sources/Aieyes/Palette.swift](../apps/macos/Sources/Aieyes/Palette.swift) | 5 | 已核对，保留 |
| [apps/macos/Sources/Aieyes/PetWindow.swift](../apps/macos/Sources/Aieyes/PetWindow.swift) | 18 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/QuotaNotificationSettingsView.swift](../apps/macos/Sources/Aieyes/QuotaNotificationSettingsView.swift) | 12 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/QuotaNotifications.swift](../apps/macos/Sources/Aieyes/QuotaNotifications.swift) | 13 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/QuotaTools.swift](../apps/macos/Sources/Aieyes/QuotaTools.swift) | 82 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/SSHConfigImportView.swift](../apps/macos/Sources/Aieyes/SSHConfigImportView.swift) | 11 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/SessionActivity.swift](../apps/macos/Sources/Aieyes/SessionActivity.swift) | 9 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/SettingsView.swift](../apps/macos/Sources/Aieyes/SettingsView.swift) | 147 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/SetupGuide.swift](../apps/macos/Sources/Aieyes/SetupGuide.swift) | 94 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/SetupGuideView.swift](../apps/macos/Sources/Aieyes/SetupGuideView.swift) | 22 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/UpdateTransport.swift](../apps/macos/Sources/Aieyes/UpdateTransport.swift) | 2 | 已核对，保留 |
| [apps/macos/Sources/Aieyes/Views.swift](../apps/macos/Sources/Aieyes/Views.swift) | 167 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/WakeupTime.swift](../apps/macos/Sources/Aieyes/WakeupTime.swift) | 4 | 已核对，保留 |
| [apps/macos/Sources/Aieyes/WakeupsStore.swift](../apps/macos/Sources/Aieyes/WakeupsStore.swift) | 4 | 已核对，保留 |
| [apps/macos/Sources/Aieyes/WakeupsView.swift](../apps/macos/Sources/Aieyes/WakeupsView.swift) | 48 | 已修订并核对 |
| [apps/macos/Sources/Aieyes/CopyNotice.swift](../apps/macos/Sources/Aieyes/CopyNotice.swift) | 2 | 已修订并核对 |

### 核心与规范源

| 来源 | 含中文行数 | 处置 |
| --- | ---: | --- |
| [crates/core/src/account_targets.rs](../crates/core/src/account_targets.rs) | 6 | 已修订并核对 |
| [crates/core/src/accounts.rs](../crates/core/src/accounts.rs) | 39 | 已修订并核对 |
| [crates/core/src/agy_auth.rs](../crates/core/src/agy_auth.rs) | 36 | 已核对，保留 |
| [crates/core/src/agy_identity.rs](../crates/core/src/agy_identity.rs) | 3 | 已核对，保留 |
| [crates/core/src/antigravity.rs](../crates/core/src/antigravity.rs) | 15 | 已核对，保留 |
| [crates/core/src/automatic.rs](../crates/core/src/automatic.rs) | 35 | 已修订并核对 |
| [crates/core/src/codex_api/direct.rs](../crates/core/src/codex_api/direct.rs) | 31 | 已修订并核对 |
| [crates/core/src/codex_api/gateway.rs](../crates/core/src/codex_api/gateway.rs) | 3 | 已核对，保留 |
| [crates/core/src/codex_api/mod.rs](../crates/core/src/codex_api/mod.rs) | 51 | 已修订并核对 |
| [crates/core/src/codex_api/scheduler.rs](../crates/core/src/codex_api/scheduler.rs) | 3 | 已核对，保留 |
| [crates/core/src/codex_api/usage.rs](../crates/core/src/codex_api/usage.rs) | 5 | 已核对，保留 |
| [crates/core/src/codex_auth/files.rs](../crates/core/src/codex_auth/files.rs) | 20 | 已修订并核对 |
| [crates/core/src/codex_auth/local.rs](../crates/core/src/codex_auth/local.rs) | 60 | 已修订并核对 |
| [crates/core/src/codex_auth/mod.rs](../crates/core/src/codex_auth/mod.rs) | 41 | 已修订并核对 |
| [crates/core/src/codex_auth/processes.rs](../crates/core/src/codex_auth/processes.rs) | 18 | 已修订并核对 |
| [crates/core/src/codex_auth/processes/windows.rs](../crates/core/src/codex_auth/processes/windows.rs) | 21 | 已修订并核对 |
| [crates/core/src/codex_auth/remote.rs](../crates/core/src/codex_auth/remote.rs) | 4 | 已修订并核对 |
| [crates/core/src/codex_auth/rpc.rs](../crates/core/src/codex_auth/rpc.rs) | 35 | 已修订并核对 |
| [crates/core/src/codex_history.rs](../crates/core/src/codex_history.rs) | 11 | 已核对，保留 |
| [crates/core/src/codex_routes.rs](../crates/core/src/codex_routes.rs) | 1 | 已核对，保留 |
| [crates/core/src/contracts.rs](../crates/core/src/contracts.rs) | 1 | 已修订并核对 |
| [crates/core/src/credentials.rs](../crates/core/src/credentials.rs) | 8 | 已核对，保留 |
| [crates/core/src/estimates.rs](../crates/core/src/estimates.rs) | 84 | 已修订并核对 |
| [crates/core/src/history_corrections.rs](../crates/core/src/history_corrections.rs) | 2 | 已核对，保留 |
| [crates/core/src/host_sampling.rs](../crates/core/src/host_sampling.rs) | 1 | 已核对，保留 |
| [crates/core/src/import.rs](../crates/core/src/import.rs) | 13 | 已核对，保留 |
| [crates/core/src/lib.rs](../crates/core/src/lib.rs) | 26 | 已核对，保留 |
| [crates/core/src/main.rs](../crates/core/src/main.rs) | 2 | 已核对，保留 |
| [crates/core/src/models.rs](../crates/core/src/models.rs) | 3 | 已修订并核对 |
| [crates/core/src/monitoring.rs](../crates/core/src/monitoring.rs) | 2 | 已核对，保留 |
| [crates/core/src/network.rs](../crates/core/src/network.rs) | 6 | 已核对，保留 |
| [crates/core/src/opencode.rs](../crates/core/src/opencode.rs) | 10 | 已核对，保留 |
| [crates/core/src/operations.rs](../crates/core/src/operations.rs) | 2 | 已核对，保留 |
| [crates/core/src/pet_import.rs](../crates/core/src/pet_import.rs) | 30 | 已核对，保留 |
| [crates/core/src/pet_import/acquire.rs](../crates/core/src/pet_import/acquire.rs) | 50 | 已核对，保留 |
| [crates/core/src/pet_import/library.rs](../crates/core/src/pet_import/library.rs) | 28 | 已核对，保留 |
| [crates/core/src/pets.rs](../crates/core/src/pets.rs) | 13 | 已核对，保留 |
| [crates/core/src/process.rs](../crates/core/src/process.rs) | 30 | 已核对，保留 |
| [crates/core/src/quota.rs](../crates/core/src/quota.rs) | 86 | 已修订并核对 |
| [crates/core/src/quota_queries.rs](../crates/core/src/quota_queries.rs) | 15 | 已修订并核对 |
| [crates/core/src/quota_schedule.rs](../crates/core/src/quota_schedule.rs) | 1 | 已核对，保留 |
| [crates/core/src/remote_history.rs](../crates/core/src/remote_history.rs) | 12 | 已核对，保留 |
| [crates/core/src/session_hooks.rs](../crates/core/src/session_hooks.rs) | 13 | 已核对，保留 |
| [crates/core/src/sessions.rs](../crates/core/src/sessions.rs) | 26 | 已修订并核对 |
| [crates/core/src/settings.rs](../crates/core/src/settings.rs) | 35 | 已核对，保留 |
| [crates/core/src/ssh.rs](../crates/core/src/ssh.rs) | 7 | 已核对，保留 |
| [crates/core/src/ssh_config.rs](../crates/core/src/ssh_config.rs) | 9 | 已核对，保留 |
| [crates/core/src/store.rs](../crates/core/src/store.rs) | 47 | 已修订并核对 |
| [crates/core/src/subscriptions.rs](../crates/core/src/subscriptions.rs) | 59 | 已修订并核对 |
| [crates/core/src/update_transport.rs](../crates/core/src/update_transport.rs) | 9 | 已核对，保留 |
| [crates/core/src/usage.rs](../crates/core/src/usage.rs) | 1 | 已核对，保留 |
| [crates/core/src/usage_cache.rs](../crates/core/src/usage_cache.rs) | 3 | 已核对，保留 |
| [crates/core/src/wakeups/mod.rs](../crates/core/src/wakeups/mod.rs) | 61 | 已修订并核对 |
| [crates/core/src/wakeups/runner.rs](../crates/core/src/wakeups/runner.rs) | 29 | 已修订并核对 |
| [crates/core/src/wakeups/scheduler.rs](../crates/core/src/wakeups/scheduler.rs) | 9 | 已核对，保留 |
| [docs/core-contracts.json](../docs/core-contracts.json) | 12 | 已修订并核对 |

### 运行时辅助脚本

| 来源 | 含中文行数 | 处置 |
| --- | ---: | --- |
| [scripts/agy_identity.py](../scripts/agy_identity.py) | 16 | 已核对，保留 |
| [scripts/remote_codex_api.py](../scripts/remote_codex_api.py) | 49 | 已修订并核对 |
| [scripts/remote_codex_auth.py](../scripts/remote_codex_auth.py) | 86 | 已修订并核对 |
| [scripts/remote_history.py](../scripts/remote_history.py) | 15 | 已核对，保留 |
| [scripts/remote_quota.py](../scripts/remote_quota.py) | 3 | 已修订并核对 |
| [scripts/remote_wakeup.py](../scripts/remote_wakeup.py) | 48 | 已修订并核对 |

## 统一替换明细

以下按原文与目标文案合并重复项；结构调整、折叠迁移及状态拆分见问题清单。完整发生位置由上方来源表与对应源码差异定位。

| 原文 / 片段 | 目标文案 / 片段 | 涉及运行时来源数 |
| --- | --- | ---: |
| 账号 | 账户 | 33 |
| 非预期重置 | 提前重置 | 6 |
| 自然重置 | 到期重置 | 6 |
| 限额查询 | 额度查询 | 9 |
| 账号密码 | 用户名和密码 | 2 |
| 刷新限额 | 刷新额度 | 7 |
| 账户限额 | 账户额度 | 7 |
| 限额间隔 | 额度查询间隔 | 2 |
| Agent 限额 | 额度查询 | 3 |
| 所有限额面板 | 所有额度面板 | 2 |
| credits估值（当前1000credits≈ | credits 估值（1000 credits ≈  | 2 |
| 远程 shell | 远程 Shell | 2 |
| 自动估值5h/7d限额和credits的价值 | 估算 5 小时 / 7 天额度及 credits 的 API 等价价值 | 2 |
| 自动估值5h/7d限额的价值 | 估算 5 小时 / 7 天额度的 API 等价价值 | 2 |
| Codex / Claude / AGY | Codex / Claude Code / Antigravity | 3 |
| 限额数据 | 额度数据 | 3 |
| Codex、Claude、AGY | Codex、Claude Code、Antigravity | 1 |
| 限额与余额 | 额度与余额 | 1 |
| 请在 Agents 重新选择机器并保存 | 请在接入与账户中重新选择机器并保存 | 1 |
| 订阅额度重置提醒已连接系统通知。 | 收到此通知表示系统通知可正常显示。 | 2 |
| 当前已是最新版本 | 已是最新版本 | 2 |
| 通过 Codex 会话日志读取状态，无需部署 Hook。 | 从 Codex 日志读取会话状态。 | 2 |
| 采集时自动接入观察 Hook；停用后继续读取日志。 | 采集时自动监测会话；停用实时监测后仍读取日志。 | 2 |
| 通过 SSH 自动接入观察 Hook，需要远端 Python 3；停用后继续读取日志。 | 通过 SSH 监测会话，需要远端 Python 3。停用实时监测后仍读取日志。 | 2 |
| 会话状态接入 | 会话监测 | 2 |
| 修复接入 | 修复监测 | 3 |
| 等待自动接入 | 等待监测安装 | 3 |
| 会话已接入 | 会话监测已启用 | 2 |
| 会话未接入 | 会话监测未启用 | 2 |
| 接入已停用 | 实时监测已停用 | 2 |
| 已登录表示保留登录凭据，使用中表示当前选用此账户 | 已登录：已保存凭据；使用中：机器当前选用的账户 | 1 |
| 部分设备状态待确认，离线设备以上次检查为准 | 部分机器状态待确认；离线时显示上次检查结果 | 1 |
| 选择项即时保存；文本离开输入框保存。 | 移除常驻段落，必要条件在编辑器或执行规则显示 | 1 |
| 复用此机器的现有登录，或完成浏览器授权。登录后显示邮箱与订阅，登录凭据保存在原机器。 | 读取机器现有登录，或通过浏览器授权。凭据保存在登录机器。 | 1 |
| 当前机器没有可用目录，请选择机器、启用采集或配置目录。 | 请启用此机器的采集并配置日志目录。 | 1 |
| 没有可用位置。请先在 Agent 接入选择机器；同一目录只能连接一个当前登录账户。 | 请在“接入与账户”选择机器和采集目录。每个目录连接一个当前登录账户。 | 1 |
| 更换目录将解除旧位置的账户连接；原文件和凭证保留，请在账户中读取新目录的登录信息。 | 更换目录会解除旧账户连接，保留原文件和凭据。之后请读取新目录的登录。 | 1 |
| 账户可单独覆盖这些默认设置。额度一直满额时不提醒；仅在实时查询确认重置后通知。关闭窗口后仍会检测，退出应用或电脑休眠时暂停。 | 账户可单独设置提醒规则。退出应用或电脑休眠时暂停检测。 | 2 |
| 到期重置时也提醒 | 到期重置时提醒 | 2 |
| 选择项即时保存；文本修改后离开输入框保存。 | 修改后自动保存。 | 1 |
| 配色随明暗主题调整，适用于模型柱形图与分布图。高对比度适合小色块；低对比度适合柔和展示，可结合图例与明细读取。 | 图表配色随明暗主题调整。高对比度适合小色块，低对比度更柔和。 | 2 |
| 默认隐藏已删除账户；历史用量仍计入总统计。开启后仅显示有历史记录的已删除账户。 | 在筛选中显示有历史记录的已删除账户；其用量始终计入总统计。 | 2 |
| 手动从头读取所有已启用本机及服务器数据源的完整日志，修复可确认的历史重复。保留已有历史和价格快照；已丢失的日志无法恢复或核实。 | 重新读取已启用来源的完整日志，修正可确认的重复记录。已有历史和价格快照保留。 | 2 |
| 接入账户后，这里将显示可用额度与余额。 | 添加账户后查看额度与余额。 | 2 |
| 可在实时账户区查看余额与额度；接入日志后即可分析用量。 | 连接账户查看额度与余额；添加日志目录后分析用量。 | 1 |
| 把日志里的模型名映射到 OpenRouter 模型 ID | 将日志模型名对应到 OpenRouter 模型 ID | 1 |
| 按本次模型组合的 API 等价成本与额度消耗比例估算，不是可兑换余额。 | 按样本成本与额度消耗比例估算 API 等价价值，金额不可兑换。 | 2 |
| 当前账户正在采样其他模型组；每个账户同时保留一段采样。 | 此账户正在采样其他模型组，请先结束当前采样。 | 2 |
| 建议开始后新建会话。可靠的正向消耗会立即输出估值，小样本会提示；跨采样边界的累计用量会使本次结果不可用。 | 开始后请新建会话。有可确认的额度消耗且价格完整时显示估值；跨采样边界的累计用量不计入。 | 1 |
| 共享目录只计算本次采样时段。采样期间请只使用此账户；切换账户前结束采样，切换后重新开始。 | 共享目录仅统计本次采样时段。切换账户前请结束采样，切换后重新开始。 | 2 |
| 关闭窗口后采样继续。只有结束采样才会停止记录。 | 关闭窗口后继续采样，点击“结束采样”停止。 | 2 |
| 手动选择不限数量，也可不选。自动模式显示每类前五个账户。仅影响本机面板。 | 自动显示每类前 5 个账户；手动选择可留空、不限数量。仅影响本机面板。 | 1 |
| 拖动账户，或使用上下按钮。顺序应用于所有额度面板。 | 拖动账户或使用上下按钮排序，应用于所有额度面板。 | 2 |
| 保存 API 连接后，可在本机账户与 API 之间切换。沿用 Codex 当前的模型、推理强度和网络设置。 | 添加 API 连接后可切换登录方式，沿用 Codex 的模型、推理强度和网络设置。 | 2 |
| 支持官方 OpenAI 与兼容 Responses API 的服务。保存后可通过切换按钮启用。 | 支持 OpenAI 和兼容 Responses API 的服务。保存后可切换使用。 | 2 |
| 此设备尚未保存账户，请在账户入口读取当前登录或登录。 | 请先在此机器读取当前登录或登录账户。 | 2 |
| 登录或读取成功后自动添加账户，以邮箱命名；已添加的同一账户会自动识别。 | 登录成功后自动添加账户，默认以邮箱命名；已有账户会自动识别。 | 2 |
| 登录后自动以邮箱命名，可在账户设置中修改名称。 | 默认以邮箱命名，可在账户设置中修改。 | 2 |
| 当前登录与目标账户不一致，可能已被其他程序修改，请重新检查。 | 当前登录与目标账户不一致，请重新检查。 | 2 |
| Z.ai / 智谱 Coding Plan。先在官网登录并复制所属地区 API Key。 | 从对应地区的 Z.ai / 智谱账户获取 Coding Plan API Key。 | 3 |
| 先在对应站点登录，复制用量请求的 Cookie。中国旧站点与 qoder.cn 新账户不互通。 | 复制对应站点的用量 Cookie。旧中国站与 qoder.cn 新账户不互通。 | 3 |
| 订阅：运行 grok login 后从 ~/.grok/auth.json 复制所选账户的 key。API 余额需 Management API Key 和 Team ID。 | 订阅：运行 grok login，从 ~/.grok/auth.json 复制账户 key。API 余额：填写 Management API Key 和 Team ID。 | 3 |
| 在 Cursor 用量页登录，复制请求 Cookie（包含 WorkosCursorSessionToken）。 | 从 Cursor 用量页复制含 WorkosCursorSessionToken 的请求 Cookie。 | 3 |
| 使用 API Key；Console 使用会话 Cookie 和 Workspace ID，可查询 Go 与 Zen 余额。外部模型订阅请连接对应服务商。 | 使用 API Key，或 Console Cookie 与 Workspace ID 查询 Go / Zen 余额。外部订阅请连接对应服务商。 | 3 |
| 桌宠单击打开面板，悬停查看状态，拖动松手后自动贴边；隐藏后可从托盘菜单恢复，显示偏好会保留。 | 单击打开面板，悬停查看状态，拖动松手后贴边。隐藏后可从托盘菜单恢复。 | 1 |
| 显示活跃会话计数（本机及 SSH，立即生效） | 显示活跃会话数（本机与 SSH） | 2 |
| 应用通知接口可用；实际显示由系统通知设置控制 | 通知接口可用，请发送测试通知检查显示 | 1 |
| 通知的显示与声音遵循系统通知和勿扰模式设置。 | 通知显示和声音由系统通知及勿扰设置控制。 | 1 |
| 自动识别 ZIP、pet.json、支持的桌宠网页、npx 命令和 npm 包名。导入成功后立即使用，已有桌宠会保留。 | 支持 ZIP、pet.json、桌宠链接、npx 命令和 npm 包名。导入后立即使用，已有桌宠保留。 | 1 |
|  idle 预览 | 空闲动画预览 | 1 |
| 同一 Agent 可添加多个账户并连接不同机器；在账户页区分“已登录”和“正在使用”，按需切换。 | 添加多个账户，管理各机器的登录与当前使用账户。 | 2 |
| 需要通过 API 使用 Codex 时，在 Codex 的 API 连接区域配置；与订阅账户分开管理。 | 在 Codex 中添加 API 连接，按需切换登录方式。 | 2 |
| 配置桌宠资源、大小和会话展示；要看账户额度，请先选择面板账户。 | 选择桌宠、调整大小和会话展示。 | 2 |
| 保留原 SSH 别名，密钥、跳板机等连接选项继续跟随 SSH 配置。地址为预览，未测试连接。 | 沿用 SSH 别名、密钥和跳板配置。导入后可测试连接。 | 2 |
| 未找到可导入的 SSH 别名，仍可手动添加主机。 | 未找到 SSH 别名，可手动添加服务器。 | 2 |
| 可以以约0成本帮助您控制5h/7d限额的刷新时间 | 按设定时间调用订阅模型。 | 2 |
| 部署到本机或 SSH Linux 服务器，使用目标机器已登录的订阅 CLI。每天按目标机器时区执行；错过时刻不补发，退出 Aieyes 后继续运行。 | 使用本机或 SSH Linux 服务器的订阅登录，按目标机器时区运行。 | 1 |
| 离线或暂停时也可保存已关联账户的草稿；部署和运行时检查执行能力。修改草稿后需更新部署，本机任务在用户登录期间执行。 | 移除常驻段落，必要条件在编辑器或执行规则显示 | 1 |
| 模型默认 effort | 默认推理强度 | 2 |
| 模型默认（不传参数） | 模型默认 | 2 |
| 读取模型与执行能力 | 读取可用模型 | 2 |
| 读取执行能力后显示目标时区与本地时间换算 | 读取模型后显示目标时区及本机时间 | 1 |
| 读取后按已知 API 价格推荐低价模型；保存草稿不会安装任务。 | 按已有 API 价格推荐模型。保存后需部署任务。 | 1 |
| 已部署任务的运行位置已锁定；迁移前请先移除旧部署。 | 更换运行位置前请移除旧部署。 | 1 |
| 请先添加并保存已关联订阅账户的本机或服务器数据源 | 请先连接订阅账户与运行机器 | 1 |
| 有尚未部署的更改 | 草稿待部署 | 2 |
| 系统任务存在 | 系统任务已安装 | 2 |
| 没有可用位置。请启用 Agent、选择机器并开启目录采集；同一目录只能连接一个当前登录账户。 | 请启用 Agent、选择机器并配置采集目录。每个目录连接一个当前登录账户。 | 1 |
| 复用此机器的现有登录，或完成浏览器授权。登录后显示邮箱与订阅，登录凭据由 agy 保存在原机器。 | 读取机器现有登录，或通过浏览器授权。凭据保存在登录机器。 | 1 |
| 正在等待应用退出；如取消过退出，可重试重启。 | 正在等待应用退出；若已取消退出，请重试重启。 | 1 |
| 保存失败时保留输入，并回到编辑器显示具体错误。 | 保存失败时保留输入，并返回对应编辑器。 | 1 |
| 此位置已暂停或未绑定，请在机器与采集设置中恢复。 | 请在“机器与采集”恢复此位置的采集或绑定。 | 1 |
| 此位置已暂停或未绑定，请在机器与采集设置中恢复后重新检查。 | 请在“机器与采集”恢复采集或绑定后重新检查。 | 1 |
| 只下载与解析素材，不运行 npx 或安装脚本。导入后立即启用，已添加的桌宠保留在下方。 | 导入后立即使用，已有桌宠保留。 | 1 |
| 通知的显示与声音遵循系统通知和专注模式设置。 | 通知显示和声音由系统通知及专注模式设置控制。 | 1 |
| 按实际 credits 扣款与样本 API 等价成本，估算 credit 的 API 等价价值，不是可兑换余额。 | 按 credits 消耗与样本成本估算 API 等价价值，金额不可兑换。 | 1 |
| 建议开始后新建会话。出现可确认的 credits 消耗且价格完整后即显示估值；小样本会有提示。充值或包含额度恢复后需要开始新一段。 | 开始后请新建会话。有可确认的 credits 消耗且价格完整时显示估值；充值或订阅额度恢复后请重新采样。 | 1 |
| 建议开始后新建会话。出现可确认的额度消耗且价格完整后即显示估值；小样本会有提示。跨起始边界的累计用量不参与计算。 | 开始后请新建会话。有可确认的额度消耗且价格完整时显示估值；跨采样边界的累计用量不计入。 | 1 |
| 自动显示每类前 5 个账户；手动选择不限数量，也可以不选。仅影响本机面板。 | 自动显示每类前 5 个账户；手动选择可留空、不限数量。仅影响本机面板。 | 1 |
| 更换目录将解除旧位置的账户连接；原文件和凭证保留。请在账户中读取新目录的登录信息。 | 更换目录会解除旧账户连接，保留原文件和凭据。之后请读取新目录的登录。 | 1 |
| 支持 Codex v1 / v2 PNG、WebP 图集。桌宠跟随已启用的本机和 SSH Codex、Claude Code、Antigravity 会话；单击打开面板，拖动移动。 | 桌宠展示本机和 SSH 的 Codex、Claude Code、Antigravity 会话。单击打开面板，拖动移动。 | 1 |
| 菜单栏口径：今日全部数据，不受面板和详情筛选影响。 | 菜单栏显示今日全部数据，独立于页面筛选。 | 1 |
| 日期、来源、模型筛选作用于用量分析；今日视图的趋势仍为近 7 天，热力图为过去 365 天。 | 用量分析遵循当前筛选；今日总览的趋势为近 7 天，热力图为过去 365 天。 | 1 |
| 查看实时余额与额度；接入日志后即可分析用量。 | 连接账户查看额度与余额；添加日志目录后分析用量。 | 1 |
| 计价覆盖未达 100%，部分模型缺少价格。请设置模型价格以补齐成本。 | 部分模型缺少价格，补齐后可计算完整成本。 | 1 |
| 部署到本机或 SSH Linux 服务器，使用目标机器已登录的订阅 CLI。按目标机器时区执行，错过时刻不补发；退出 Aieyes 后继续运行。 | 使用本机或 SSH Linux 服务器的订阅登录，按目标机器时区运行。 | 1 |
| 离线或暂停采集时仍可保存已关联账户的任务草稿；部署与运行时检查目标机器。 | 移除常驻段落，必要条件在编辑器或执行规则显示 | 1 |
| 使用已关联的账户和目录。离线时可先保存草稿；切换运行位置前需要移除旧部署。 | 离线时可先保存草稿。更换运行位置前请移除旧部署。 | 1 |
| 订阅凭据有效；不会据此推断客户端当前账户 | 已验证查询凭据；客户端当前账户待确认 | 1 |
| 此 Agent 按目录连接；当前账户状态需由 Agent 确认 | 已连接目录；当前账户需在 Agent 中确认 | 1 |
| 请通过 codexAuth.switch.prepare / commit 检查后切换账户或 API | 请在账户设置中检查并确认切换 | 1 |
| 数据源中填写 DeepSeek API Key | 账户中添加 DeepSeek API Key | 1 |
| 订阅响应不是有效 JSON，请检查登录是否过期 | 订阅响应格式无效，请检查登录状态 | 1 |
| 未返回可识别的额度，请查看官方用量页；未知用量不代表零消耗 | 未返回可识别的额度，请在官方用量页查看 | 1 |
| CLI 未提供可核验的模型列表，请填写模型 ID；可用性将在实际运行时验证 | CLI 未提供模型列表，请填写模型 ID；运行时检查可用性 | 1 |
| 按 1000 输入 + 100 输出 Token 的 API 等价价格推荐；部署后固定模型 | 参考 1000 输入 + 100 输出 Token 的 API 等价成本推荐模型；部署后使用所选模型 | 1 |
| 当前 CLI 不支持 effort，请选择模型默认值 | 当前 CLI 不支持推理强度参数，请选择模型默认值 | 2 |
| 等待有效的实时订阅限额 | 等待实时订阅额度数据 | 1 |
| 限额连接失败 | 额度查询连接失败 | 1 |
| 读取限额 | 读取额度 | 2 |
| 需要新的实时限额快照 | 请刷新订阅额度后重试 | 1 |
| 读取实时限额 | 读取实时额度 | 1 |
| 远程额度查询连接失败，请检查前置命令中的代理设置 | 远程额度查询失败，请检查登录状态和网络 | 1 |

## 验证记录

- `npm --prefix apps/desktop run test:ui`：完整通过，覆盖 Windows/Linux 路由、向导状态、目录快捷入口、错误详情、账户登录/API、通知、采样、定时唤醒、更新、主题与最小窗口布局。
- `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --offline`：25 项通过。
- `PYTHON=/opt/local/bin/python3.14 sh scripts/ci-checks.sh --rust`：完整通过，包含 317 项 Rust 测试（原有 4 项手工/基准测试忽略）、33 项 Node 测试、Python 回归、生成契约及设计资源同步检查。
- `sh scripts/test-native.sh`：完整通过，覆盖模型、字段校验与输入保留、账户/配置并发、登录、额度通知、向导状态、唤醒、更新、桌宠及后台通道。
- 原生调试构建通过，构建缓存位于 `.build/swift`；新增共用错误视图为 `CopyNotice.swift`。
- `AIEYES_RENDER_ROOT=.local/app-copy-final/macos sh scripts/test-ui-native-render.sh`：完整通过，生成 839 张 PNG，覆盖 empty / single / multi / long / failure / capacity 六场景、浅深色、最小窗口、设置页面、降低透明度及高对比度；内容识别、边框与透明度检查通过，并人工抽查向导与通知布局。
- 补充错误展示回归通过：JSON Cookie、Cookie 请求头、Bearer / API Key、URL 凭据与查询令牌遮蔽；未知诊断使用中文摘要，详情转义后可展开与复制。
- 自动验证使用隔离配置、假 CLI 和模拟账户；不为真实账户部署任务或调用付费模型。
- Windows/Linux 原生系统菜单、真实通知送达及真实 SSH 尚未实机验收；Windows/Linux 浏览器模拟与 Tauri 宿主单元测试均已通过。
