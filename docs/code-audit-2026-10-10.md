# 代码审计：死代码、残留功能、未同步结构与配置重复（2026-10-10，基线 v0.1.29）

本报告只记录证据可定位到 `文件:行` 的问题。标记含义：**【已确认】** 代码可直接证实；**【疑似】** 逻辑成立但需结合运行场景判断。未修改任何源码。

审计范围与分工：

| 范围 | 方法 |
|---|---|
| `crates/core`（Rust 核心） | 全量 grep 引用计数、设置键读写矩阵、SQL 触发器与缓存键核对 |
| `apps/desktop`（Tauri Rust + Web） | HTML 脚本加载链、JS 导出调用关系、三份 `tauri*.conf.json` 与两份 capability 比对、JSON 字段名 JS↔Rust 比对 |
| `apps/macos`（Swift） | 类型/属性/枚举引用计数；与核心及 Web 的相位色、定价、供应商、设置键、配额字段逐项比对 |
| `scripts`、`docs`、`.github` | 脚本孤儿扫描、workflow 对比、文档声明逐条回查代码 |
| 跨语言契约（本人） | 核心 RPC 方法表 ↔ Web/Swift/Python 调用方；`agy` 别名迁移；UI 持久化键；更新机制；发布版本号 |

---

## 一、问题总览（按建议处理顺序）

| # | 严重度 | 类别 | 问题 | 证据 |
|---|---|---|---|---|
| 1 | 高 | 未同步结构 → 数据错误 | 核心内 **5 份供应商白名单**互不一致：`custom` 在旧配置迁移时拿不到配额；唤醒任务拒绝 `deepseek`/`custom` | `models.rs:372`、`:381`、`store.rs:222`、`:227-234`、`wakeups/mod.rs:239` |
| 2 | 高 | 统计错误风险 | 用量事件 ID 把 `provider` 原样参与哈希，`agy`/`antigravity` 同一事件可能产生两个 ID → Token 与成本重复计数（靠两处补丁探测兜底） | `usage.rs:54-62`、`store.rs:456`、`:491-500` |
| 3 | 高 | 配置变更残留 | `githubRepository` 设置键在核心、Swift、Python 夹具中都保留，UI 早已删除该字段，测试甚至断言它不存在 | `models.rs:307/332`、`Models.swift:225`、`preview-quota-core.py:13`、`test-ui.cjs:263` |
| 4 | 高 | 发布流程 | macOS `CFBundleVersion`（当前 30）需人工递增，无脚本生成或校验"比上一发布大"，漏改则 Sparkle 不推送更新 | `Info.plist:7`、`docs/releases.md:7`、`release.py:98` |
| 5 | 中 | 死 RPC | `updates.check`、`updates.open`、`codexAuth.homes.list`、`sessions.list` 四个方法无任何生产调用方 | `lib.rs:473-495`、`:205`、`codex_auth/mod.rs:218` |
| 6 | 中 | 跨端显示不一致 | Swift 与 Web 对同一采样记录显示不同状态文案、Swift 丢弃 `reason`、日期按系统 12/24 小时制输出、估值窗口选择逻辑不同 | 见 §5.3 |
| 7 | 中 | CI 覆盖缺口 | `test-agent-settings-ui.cjs` 不在 `test:ui` 链也不在 CI；`release.yml` 的 JS 语法检查漏 `webview-guards.js`；相位色 `--check` 未进 CI | `package.json`、`release.yml:104-111`、`check-native-capsule.sh:8` |
| 8 | 中 | 冗余配置 | `tauri.conf.json` 的 `plugins.updater.pubkey: ""` 从不被读取（代码用环境变量常量覆盖）；`requireSignedVersion` 疑似无效键 | `tauri.conf.json:42,46`、`updates.rs:14,235-237` |
| 9 | 中 | 未同步结构 | 两张列定义完全相同的估值表 `quota_estimates`/`credit_estimates` 仅靠字符串 `kind` 路由；`Estimate` 结构承载两套互斥字段 | `store.rs:33-36`、`estimates.rs:9-60,80-81` |
| 10 | 中 | 未同步结构 | 同一仪表盘有两套聚合实现（缓存版与参考版），参考版是缓存出错时的静默回退路径 | `store.rs:767-768`、`:886`、`usage_cache.rs:192` |
| 11 | 低 | 版本漂移 | 远端辅助脚本向对端报告的客户端版本停留在 0.1.5 / 0.1.21 | `remote_wakeup.py:153`、`remote_codex_auth.py:502` |
| 12 | 低 | 死代码 | Swift 4 个未使用视图、3 个 Palette 常量、11 个只解码不读取的模型字段、`settingsStage` 等 4 个写后不读的状态；Web `#account-fields` CSS、`estimateLabel`、4 份重复 escape 函数；核心 `read_quota` | 见 §3 |
| 13 | 低 | 孤儿脚本 | `verify-models.swift` 零引用且断言 `settings.version == 2`（代码已迁移到 5） | `verify-models.swift:18`、`models.rs:468` |
| 14 | 低 | 文档漂移 | 7 处文档描述与代码不符（设置页标签数、"设置 → 数据源"、供应商列表缺 DeepSeek、"签名未配置"、配置版本 4） | 见 §6 |

---

## 二、同作用却未同步的数据结构

### 2.1 供应商（provider）标识表 —【已确认】

同一概念在核心里有 5 份硬编码列表，在 UI 里还有 4 份：

| 位置 | 内容 | 缺失 |
|---|---|---|
| `crates/core/src/models.rs:372`（`version<2` 迁移，决定 `quota_enabled`） | `codex, claude, antigravity, agy, deepseek` | **缺 `custom`** |
| `crates/core/src/models.rs:381`（为每个 agent 建默认配置） | `codex, claude, antigravity, deepseek, custom` | 缺 `agy`（被 `canonical()` 掩盖） |
| `crates/core/src/store.rs:222`（免填目录的供应商） | `antigravity, agy, deepseek` | — |
| `crates/core/src/store.rs:227-234`（来源校验） | 全集 6 项 | — |
| `crates/core/src/wakeups/mod.rs:239`（唤醒支持） | `codex, claude, antigravity, agy` | **缺 `deepseek`、`custom`** |
| `apps/desktop/web/app.js:30` | 含 `agy:'Antigravity'` | 需在 `:457`、`:482`、`agent-settings.js:97` 三处手动过滤 `agy` |
| `apps/macos/Sources/Aieyes/Models.swift:399` | 同上 | — |
| `apps/macos/Sources/Aieyes/AgentSettingsViews.swift:5`、`SettingsView.swift:302` | 顺序列表 | — |
| `crates/core/src/providers.rs:18-33` `display_name()` | 只识别 `"claude code"` | **不识别 `"claude"`、`"custom"`**，返回原串 |

具体后果：
- 旧配置（`version<2`）里 `custom` 账户被迁移为 `quota_enabled=false`，但校验器明确接受 `custom`。
- `deepseek`/`custom` 来源创建唤醒任务时报"此 CLI 暂不支持订阅唤醒"，而账户页允许选择它们。
- 核心补全名称时 `display_name("claude")` 得到 `"claude"`，两端 UI 显示的是 `"Claude Code"`（`providers.rs:69,84` 调用处）。

### 2.2 `agy` → `antigravity` 遗留别名层 —【已确认】

重命名早已完成（`providers.rs:111` 的 `antigravity.migrated.v1` 迁移），但兼容层散落在 ≥18 处：核心 `providers::canonical`（`providers.rs:8`）、`normalize_value`（`:50-90`）、`store.rs:456`、`:491-500`、`wakeups/mod.rs:280,354`、`quota.rs::parse_agy`；Web `app.js:30,32,544,548,1072-1073,1249`、`ui-state.js:95,111`、`agent-settings.js:97`；Swift `Models.swift:19-34,399`、`QuotaTools.swift:133,194`、`Views.swift:739,753`、`AuditComponents.swift:15`。每条新逻辑都要记得同时接受两个名字，是典型的"同作用未同步"温床。

### 2.3 面板 UI 状态持久化：Web 与 macOS 各一套 —【已确认】

| 维度 | Web（`localStorage`） | macOS（`UserDefaults`） |
|---|---|---|
| 页面/筛选 | 单个 JSON `aieyes.panel.view`（page/provider/sourceId/accountKey/model/days/cost/serverFilter），`app.js` | 离散键 `panel.page`、`detail.page`、`panel.serverFilter`、`panel.trendExpanded`、`panel.sessionsExpanded`、`menu.showCount`（`Views.swift:72-77`） |
| 仪表盘筛选条件 | 随 `aieyes.panel.view` 持久化并恢复 | **从不持久化**（`EngineClient.swift:309` 的 `selectedAccount/provider/selectedSource/selectedModel/range` 无任何 `UserDefaults.set`） |
| 面板账户选择 | `aieyes.panel.accounts.v2`，v1→v2 迁移在 `ui-state.js:3-13` | `panel.accounts.v2`，迁移在 `Models.swift:12-13` 与 `QuotaTools.swift:113`、`Views.swift:147` 三处重复 |
| `agy` 别名迁移 | `ui-state.js:110-120`，标记键 `aieyes.antigravity.preferences` | `Models.swift:15-35`，标记键 `antigravity.preferences.v1` |
| 遗留迁移 | — | `Models.swift:34` 迁移 `panel.provider`/`detail.provider`，**当前没有任何代码写这两个键**（已失效的迁移） |
| 截图模式覆盖 | — | `AieyesApp.swift:33` 硬编码 8 个键的默认值列表，新增持久化键时必须同步修改，否则截图验证读到真实用户值 |

### 2.4 RPC 方法 → 引擎通道/事件 的分类表存在 4 份 —【已确认】

- 核心 `is_configuration_method()`（`settings.rs:7-22`，11 个方法）。
- Swift `configurationMethods`（`EngineClient.swift:18`）手工复制同一份 11 个方法。
- Tauri `update_events()`（`main.rs:142-175`）两个 `matches!` 列表 + `refresh_key()`。
- Web `app.js:1066-1067` 两条失效判断列表。

新增一个会修改配置的方法需要改 4 处、2 种语言；漏改一处则该端不刷新或走错引擎通道。

### 2.5 更新检查实现有 3 份，其中核心那份已死 —【已确认】

- 核心 `updates.check`/`updates.open`（`lib.rs:473-495`）：直接查 GitHub API 取 `tag_name`，无签名校验，**无调用方**，只有 `docs/adapters.md:95` 描述它、`test-ui.cjs:76-77,288` 在 mock 它。
- Tauri `updates.rs`：签名 `latest.json` 源（`:13`）、`version_message` 语义版本比较（`:146`）、`valid_download` 下载地址校验（`:129`）。
- macOS `AppUpdater.swift`：Sparkle appcast + 经核心 `--update-transport` 代理。

### 2.6 Swift 对核心逻辑的本地重写（当前一致，但无同步机制）—【已确认】

- `SessionActivity.swift:36-91` `SessionMonitor` 完整重写 `crates/core/src/sessions.rs`（30s 发现、3600s 过期、180s 陈旧、256KiB 尾读）。Tauri 端是进程内直接用核心 `sessions::SessionMonitor`（`desktop.rs:2,972`）。核心暴露的 `sessions.list` RPC 因此无人调用。
- `MonitorSelection.swift:3-41` 重写 `monitoring.rs` 的 `recommended_filesystem`/`filter_devices`（11 类排除类型、14 个排除路径）。
- `Models.swift:3-67` `PanelAccountPreference` 与 `ui-state.js:3-25` 语义一致。

### 2.7 核心内部的并行结构 —【已确认】

- **估值表**：`quota_estimates` 与 `credit_estimates`（`store.rs:33-36`）列、索引完全相同，仅靠 `Estimate.kind == "credits"`（`estimates.rs:80-81`）路由；所有读取需 `UNION ALL`（`estimates.rs:249-251,265`、`providers.rs:147`）。`Estimate` 结构（`estimates.rs:9-60`）同时携带配额专用字段（`weekly_*`、`segments`、`five_hour_value`…）与积分专用字段（`baseline_balance`、`value_per500/1000`…），每个消费点都要 `is_credit()` 分支。
- **仪表盘聚合**：缓存版 `usage_cache::read`（`usage_cache.rs:192`）与原始版 `Store::dashboard_usage_reference`（`store.rs:886`），后者是前者出错时的静默回退（`store.rs:767-768`），两份 SQL 投影只靠测试保证一致。
- **容量**：`capacity::Capacity`（`capacity.rs:9-14`，来自 `capacity_samples`）与估值侧 `weekly_checkpoint_percent`/`EstimateSegment.weekly_percent`（`estimates.rs:69,653,718`）是两套"周容量"数字，无对账。

### 2.8 Swift ↔ 核心 Settings 默认值差异 —【疑似】

- `Models.swift:224` `var version = 2`；核心默认 1、迁移后 5（`models.rs:318,468`）。若 Swift 在加载前用默认 `Settings()` 发起保存，核心会重跑 `v<4`/`v<5` 迁移。
- `Models.swift:189` `AgentSource.path = "~/.codex"` 作为所有供应商的默认目录；核心 `Source::default` 是空串。
- `AgentSettingsViews.swift:102-105` `saveGeneralConfiguration` 手工复制 9 个字段到完整对象；`SettingsView.swift:110` 又直接改 `model.settings.localMonitor` 再保存——同一字段两条写路径，新增通用设置时必须记得加进复制列表。

### 2.9 Web ↔ Rust 的重复常量 —【已确认】

| 常量 | Web | Rust |
|---|---|---|
| 默认数据目录 | `agent-settings.js:181` | `settings.rs:319-322` |
| 默认二进制名 `codex`/`agy` | `agent-settings.js:181,205` | `models.rs:105-106` |
| 5h/7d 窗口 300/10080 分钟 | `ui-state.js:76,99,103`、`app.js:546,548` | `capacity.rs:67,117,131-132`、`estimates.rs:135` |
| 供应商表 | `app.js:30`、`wakeups.js:17` | `providers.rs:9-24` |

---

## 三、死代码与残留功能

### 3.1 核心 Rust —【已确认】

| 项 | 位置 | 说明 |
|---|---|---|
| `codex_auth::read_quota` | `codex_auth/mod.rs:94` | 全仓库仅定义；活跃入口是 `read_quota_at`（`lib.rs:95`） |
| RPC `updates.check` / `updates.open` | `lib.rs:473-495` | 无调用方；仓库名硬编码 `"JesmonX/Aieyes"`（`:479`） |
| RPC `codexAuth.homes.list` | `codex_auth/mod.rs:218` | 无调用方 |
| RPC `sessions.list` | `lib.rs:205` | 无生产调用方，只有 `test-*.cjs`、`preview-quota-core.py` 的 mock |
| CLI `--call` | `main.rs:45` | 无脚本/测试/文档使用 —【疑似】调试用途 |
| 设置键 `github_repository` | `models.rs:307,332` | 核心、Swift（`Models.swift:225`）均不读；仅 `preview-quota-core.py:13`、`test-ui.cjs:34` 夹具填值；`test-ui.cjs:263` 断言 UI 字段 `#field-githubRepository` 不存在 |
| 设置键 `menu_metric` | `models.rs:306,331` | 核心不读，仅 Swift 菜单栏用（`EngineClient.swift:806`）；Web 无对应 UI |
| 设置键 `history_refresh_seconds`、`server_refresh_seconds`、`server_foreground_refresh_seconds` | `models.rs:327-329`、校验 `store.rs:274-283` | 核心只校验不消费，全部由前端使用；只有 `refresh_seconds` 被核心用于调度（`quota_schedule.rs:45`） |
| `settings.patch` 静默丢弃两个键 | `settings.rs:117-118` | `deletedAccounts`、`accountAliases` 总被服务端当前值覆盖，客户端传入无效且无报错 |

### 3.2 Tauri 桌面端与 Web —【已确认】

- **`verify-update.rs` 不是死文件**：它是 `[[bin]] verify-update`（`Cargo.toml:32-35`，`required-features=["release-tools"]`），由 `sign-desktop-updates.py:26`、`test-update-signatures.py:25` 构建调用。建议在 `main.rs` 加一行注释避免误判。
- `capsule.rs`、`material.rs`、`passive_window.rs` 为 Windows 专用（`main.rs:2-8` cfg 门控），非死代码。
- Web 18 个 JS 文件均被 `index.html`/`floating.html` 加载（`theme.js` 在 `<head>`），无孤儿文件。
- 死 CSS：`#account-fields` 三条规则（`style.css:221,539,540`），无任何元素或 JS 生成该 id。
- 死导出：`AieyesUI.estimateLabel`（`ui-state.js:121`），零调用。
- 重复 HTML 转义函数 4 份（`app.js:6`、`desktop.js:4`、`updates.js:2`、`sessions.js:10`）+ 2 个别名（`codex-auth.js:3`、`agent-settings.js:3`）；修复 XSS 转义需改 4 处。
- 序列化但 Web 从不读取的字段：`Dashboard.days`、`Dashboard.priceUpdatedAt`（热力图用 `d.heatmap`，`app.js:464`）、`Tokens.reasoning`（`models.rs:11`，`total()` 有意排除）、`desktop_info` 的 `generation`/`dark`/`trayAvailable`（`desktop.rs:972` 附近）。估值记录中 20 余个内部字段（`baselinePercent`、`intervalEvidence`、`lastCumulative` 等）也随 payload 发出 —【疑似】可裁剪。

### 3.3 macOS Swift —【已确认】

| 类别 | 项 |
|---|---|
| 未使用视图 | `AttentionNotice`（`Views.swift:45`）、`TokenBreakdownCard`（`:429`）、`ModelKey`（`:504`）、`EstimateSummary`（`AuditComponents.swift:79`） |
| 未使用 Palette | `radiusOverlay`（`Palette.swift:27`）、`colors`（`:34`）、`radiusControl`（`:28`，仅被死视图引用） |
| 写后不读的状态 | `settingsStage`（`EngineClient.swift:266`，在 `:610,:620,:677,:679` 写入但无视图显示）、`settingsStartedAt`（`:267`）、`settingsChangeCount`（`:277-291`）、`showDetail` 闭包（`:354`，`AieyesApp.swift:50` 赋值，无人调用；真实入口是 `showDetailPage`） |
| 只解码不读的模型字段 | `githubRepository`、`events`（`Models.swift:75`）、`ResetCredit.grantedAt`（`:88`）、`subscriptionCheckedAt`（`:98`）、`newEvents`/`malformedLines`（`:111`）、`attemptedAt`（`:140`）、`pendingName`（`:148`）、`calculationVersion`（`:405`）、`valuePer500`（`:413`）、`WakeModel.defaultEffort`/`referenceCost`（`WakeupsView.swift:11`；Web 会显示 `referenceCost`，`wakeups.js:32`） |
| 仅截图工具可达 | `SessionPhase.idle`（`SessionActivity.swift:6`，运行时 `phase(in:)` 永不产生）、`previewAccessibleSurfaces`（`Views.swift:19-30`） |

### 3.4 脚本 —【已确认】

- **真孤儿**：`scripts/verify-models.swift`，零引用；且 `:18` 断言 `settings.version == 2`，代码已迁移到 5，即使接回也会失败。
- **只在文档出现、无自动化**（12 个）：`compare-ui.py`、`generate-icons.py`、`test-agent-settings-ui.cjs`、`test-native-performance.sh`、`test-server-performance.sh`、`test-streaming-core.py`、`test-incremental-history.py`、`test-update-transport.py`、`benchmark-performance.py`、`benchmark-remote-history.py`、`verify-update-transfer.swift`、`test-ui-native-render.sh`。其中 `test-agent-settings-ui.cjs` 是完整 Playwright 测试（真实核心 + 临时库）。
- 三个脚本重复覆盖 `remote_history.py` 增量游标语义（`test-incremental-history.py:40-127`、`test-streaming-core.py:64-108`、`benchmark-remote-history.py:29-40`）。
- `Format.subscription` 在 `verify-reset-format.swift:40-41` 与 `verify-macos-ui.swift:36` 重复断言。
- `scripts/vendor/tomli` **仍需要**：被 `codex_auth/remote.rs:8-11` 以 `include_str!` 嵌入远端助手，供 Python < 3.11 主机使用。

---

## 四、配置重复、缺失与冗余

### 4.1 Tauri 配置 —【已确认】

- `plugins.updater.pubkey: ""`（`tauri.conf.json:42`）从不被读取：`updates.rs:14` 用 `option_env!("AIEYES_UPDATER_PUBLIC_KEY")`，`:235` `.pubkey(PUBLIC_KEY)`，`:237` 端点硬编码 `FEED`。配置与代码各管一半，JSON 里的空公钥纯属误导。`requireSignedVersion`（`:46`）不是 `tauri-plugin-updater` v2 的已知键 —【疑似】被静默忽略。
- `app.windows` 块在 `tauri.windows.conf.json:3` 整体重述（Tauri 覆盖数组不做字段合并）：`label/title/minWidth/minHeight` 手工同步，`width/height` 已漂移（1120×800 vs 1294×840）。`tauri.linux.conf.json` 无 `app.windows` → Linux 与 macOS 共用 1120×800 带装饰窗口，与 `docs/windows-ui-refresh.md:9` 的描述一致，但属于隐式继承而非显式配置。
- `bundle.resources: ["binaries/*"]` 会把 `binaries/README.txt` 一并打进安装包。
- 两份 capability（`main.json`、`main-window.json`）互补、无缺口；`core:window:allow-start-dragging` —【疑似】可能已包含在 `core:default`。

### 4.2 版本与发布 —【已确认】

- 6 处版本号一致为 0.1.29，`release.py:23-38` 强制校验。
- `CFBundleVersion`（`Info.plist:7`，值 30）无生成脚本，`docs/releases.md:7` 要求人工递增；`release.py:98` 只校验 plist 与 Sparkle appcast 相等，不校验"比上一次大"。
- Python 远端助手上报的客户端版本硬编码：`remote_wakeup.py:153` `'0.1.5'`、`remote_codex_auth.py:502` `'0.1.21'`。
- `AIEYES_UPDATER_PUBLIC_KEY` 在 `release.yml:60` 注入、`:89` 校验非空，正常。

### 4.3 CI 与测试运行器 —【已确认】

- `apps/desktop/package.json` `test:ui` 链 16 个脚本 / 仓库 21 个 `test-*.cjs|mjs`；CI 另跑一组（`desktop.yml:42-67`、`release.yml:37-136`）。`test-agent-settings-ui.cjs` 两边都没有。
- `desktop.yml:50-58` 语法检查 9 个 JS，`release.yml:104-111` 只有 8 个，漏 `webview-guards.js` → 发布流水线不检查它。
- `desktop.yml:24` `windows-latest` vs `release.yml:50` `windows-2022`；同一证据产物名 `Windows-native-panel-results`（`desktop.yml:99`）vs `windows-ui-evidence`（`release.yml:168`）。
- 大段 job 在两份 workflow 中重复定义（浏览器集成、`node --test`、`test-wakeups.py`、`test-collectors.py`、Rust 测试）。
- 相位色一致性 `sync-phase-colors.py --check` 唯一调用方是 `check-native-capsule.sh:8`，而该脚本只在文档中出现。
- 无根 `package.json`；所有 `scripts/test-*.cjs` 以相对路径 `require('../apps/desktop/node_modules/playwright')` 取依赖。

### 4.4 设置键 —【已确认】

- 冗余：`githubRepository`（无人读）、`menuMetric`（仅 macOS）。
- 四个全局轮询间隔 + 每账户 `quota_refresh_seconds`（`quota_schedule.rs:9`）覆盖 `refresh_seconds`，命名易混且核心只用其一。
- `deletedAccounts`、`accountAliases` 名义上在 `Settings` 里，实际 `settings.patch` 不可写（`settings.rs:117-118`）。
- Swift 通用设置页对间隔字段无前端校验（`SettingsView.swift:196-199`），Web 有（`app.js:353`）；核心 `store.rs:270-283` 兜底报错，仅体验不一致。

---

## 五、可能导致使用或统计出错的点

### 5.1 核心统计 —【已确认除注明外】

1. `usage::event_id`（`usage.rs:54-62`）哈希 `"{provider}:{namespace}:{identity}"`，供应商字符串翻转（`agy`↔`antigravity`）会生成不同 ID；`store.rs:456` 的 `(provider=?2 OR (?2='antigravity' AND provider='agy'))` 与 `:491-500` 的旧命名空间探测是事后补丁。任一分支漏判即重复计数。
2. 【疑似】`usage_day_revision` 触发器用 SQLite 整数除法截断（`usage_cache.rs:78`），读取端用 `div_euclid` 向下取整（`:174-178`）；正时间戳一致，负时间戳会让版本串对不上而复用过期缓存。
3. `metrics.rs:8` 两次采样间隔 >300s 时丢弃全部速率/利用率字段（休眠、慢 SSH 后首帧无数值），需确认是有意设计。
4. `capacity.rs:142` 容量样本主键 `(account_key, stamp)` 精确到秒，同秒两次观测只留一条。
5. 价格重算 **会** 正确触发缓存失效：`usage_events_update` 触发器的 `WHEN` 条件含 `cost/priced_tokens/price`（`usage_cache.rs:70-80`），且 `prices.save` 后立即 `reprice`（`lib.rs:465-471`）。

### 5.2 供应商表导致的功能错误 —【已确认】

见 §2.1：`custom` 账户迁移后无配额、`deepseek`/`custom` 不能建唤醒、`display_name("claude")` 小写。

### 5.3 Swift 与 Web 对同一数据的不同呈现 —【已确认除注明外】

| 项 | Swift | Web |
|---|---|---|
| 采样状态文案 | `Models.swift:422` `pending→"待确认"` | `app.js:1151` `pending→"需要确认"` |
| 采样问题原因 | `Models.swift:408` 丢弃核心给的 `reason` | `ui-state.js:32` `record.reason \|\| '待确认'` |
| 唤醒失败文案 | `WakeupsView.swift:16` "…模型和代理" | `wakeups.js:3` "…模型及代理" |
| 服务器延迟标签 | `Views.swift:884` "数据延迟" | `app.js:583-589` "数据已延迟" |
| 【疑似】估值窗口选择 | `QuotaTools.swift:131-140` 任何供应商都匹配 `five_hour`/`seven_day` | `ui-state.js:93-105` 仅 `claude` 才匹配 |
| 【疑似】Antigravity 估值窗口 | 返回所有 300 分钟窗口 | 每组只取单个 5h 否则单个 7d |
| 【疑似】日期格式 | `Models.swift:390-392` 跟随系统 12/24 小时制 | `app.js:29`、`ui-state.js:81-82` 固定 `h23`/`zh-CN`；Swift 内部 `Format.quotaReset`（`:311-315`）又固定了 `MM/dd HH:mm`，自身不一致 |
| 【疑似】积分金额舍入 | `Models.swift:333-346` `Decimal` + `.plain` | `app.js:9-20` `BigInt` 半进位 |

### 5.4 发布 —【已确认】

`CFBundleVersion` 漏递增 → Sparkle 认为无新版本；`release.py` 不会拦截。

---

## 六、文档漂移 —【已确认】

| 文档:行 | 声明 | 代码事实 |
|---|---|---|
| `docs/adapters.md:95` | `updates.check` 返回版本、`updates.open` 打开发布页 | 两方法无调用方（§3.1） |
| `docs/adapters.md:33`、`docs/windows-linux.md:15` | "设置 → 数据源" | 标签已改名 "Agents"（`app.js:708`） |
| `docs/windows-linux.md:23` | "数据源、服务器、价格、通用四个标签" | 实际 6 个：Agents/服务器/价格/账户/定时唤醒/通用 |
| `docs/architecture.md:27` | `providers/ Codex / Claude Code / Antigravity` | 缺 DeepSeek（`accounts.rs:13`、`providers.rs:23`） |
| `docs/architecture.md:96` | "签名、公证及正式 GitHub Release 发布尚未配置" | `release.yml:139-234` 已完整配置 |
| `docs/product-spec.md:9` | 首批接入三家 | DeepSeek 已上线（README:56） |
| `docs/agents-and-accounts-settings.md:24` | "配置版本 4" | `models.rs:468` 已到 5；`docs/refresh-monitoring-antigravity.md:3` 写的是 5 |
| 【疑似】`README.md:39` | "12 项细分显示" | `Host.details` 默认 13 项（`models.rs:233-245`） |

已被后续文档取代、仍保留的历史记录：`ui-reform-2026-10-06.md`、`ui-audit-and-redesign-2026-10-06.md`、`ui-review-and-optimization-2026-10-07.md`（各有同名 `-implementation.md`）、`windows-ui-refresh.md`（自述为历史）、`windows-ui-review.md`（引用的行号已失效）、`performance-audit-2026-10-10.md`（自述被 implementation 取代）、`verification.md`（过时测试日志）。

---

## 七、仓库卫生（全部已 gitignore，不影响提交，但占用空间并干扰检索）

- `.local/` 2.5 GB、约 90 个子目录；其中 `.local/windows-appearance/baseline-src/` 是 1.1 MB 的旧源码副本（含旧版 `app.js`、`remote_codex_auth.py`、`verify-macos-ui.swift`），grep 时容易命中过期代码。
- `.build/` 1.4 GB；`dist/` 39 MB 含 `Aieyes-0.1.12`、`0.1.23` 两个旧 DMG 与 `Aieyes.app`。
- `scripts/__pycache__/` 同时存在 Python 3.12 与 3.14 的 `.pyc`。
- 根目录、`docs/`、`dist/` 下有 `.DS_Store`。
- `.gitignore` 中 `/target/` 与 `**/target/`、`/.build/` 与 `**/.build/` 重复。

---

## 八、优化方案

### 阶段 1：纠正会产生错误数据或错误行为的问题（小改动，优先）

1. **统一供应商表**：在 `providers.rs` 定义 `pub const PROVIDERS: &[&str]` 与 `supports_wakeup()`/`requires_path()` 等谓词，替换 `models.rs:372,381`、`store.rs:222,227-234`、`wakeups/mod.rs:239` 五处；比较一律经 `canonical()`。`display_name()` 补 `"claude"→"Claude Code"`、`"custom"→"自定义"`。
2. **事件 ID 先规范化供应商**：`usage::event_id` 内部对 `provider` 调 `canonical()`，之后可删除 `store.rs:456,491-500` 的补丁探测（需一次性迁移校验：对历史事件重算 ID 并去重）。
3. **删除 `githubRepository`**：核心结构、Swift 模型、`preview-quota-core.py:13`、`test-ui.cjs:34` 与 `:263` 断言一并清理；`models.rs` 升级 `version` 到 6 并在迁移中忽略该键。
4. **对齐 Swift 与 Web 的呈现**：把 §5.3 四条已确认文案改成一致；`Models.swift:408` 使用 `reason`；`Models.swift:390-392` 固定 `en_US_POSIX` + `HH:mm`；`QuotaTools.swift:131-140` 对非 Claude 供应商加 provider 门槛。
5. **删除死 RPC 与死函数**：`updates.check`/`updates.open`/`network::open_release_page`、`codexAuth.homes.list`、`sessions.list`、`codex_auth::read_quota`；同步删除 `docs/adapters.md:95` 与 `test-ui.cjs:76-77,288` 的 mock/断言。若希望保留 `sessions.list` 给 macOS 用，则反过来让 Swift 删除 `SessionMonitor` 改调 RPC（见阶段 3）。
6. **文档七处漂移**按 §6 逐条修正。

### 阶段 2：配置与 CI 收敛

1. `release.py` 增加 `CFBundleVersion` 校验：读取上一个 tag 的 plist（或 `git show <prev-tag>:apps/macos/Info.plist`）要求严格递增；或改为由 tag 派生构建号并写回 plist。
2. 删除 `tauri.conf.json:42` 空 `pubkey` 与 `:46` `requireSignedVersion`，并在 `updates.rs:13-17` 旁注明"更新源与公钥由代码管理"。
3. 把 JS 语法检查、`node --test`、Python 测试等重复步骤抽成 `scripts/ci-checks.sh`，两份 workflow 共用；修正 `release.yml:104-111` 漏掉的 `webview-guards.js`；统一 Windows runner 与产物名。
4. `package.json` `test:ui` 加入 `test-agent-settings-ui.cjs`；CI 加 `python3 scripts/sync-phase-colors.py --check`。
5. `remote_wakeup.py`、`remote_codex_auth.py` 的 `clientInfo.version` 改为由核心在部署时注入（已有 `include_str!` 嵌入机制，可同样嵌入 `CARGO_PKG_VERSION`），或至少改为读取 `hello` 返回值。
6. 删除 `verify-models.swift`，或接入 `test-native.sh` 并把断言改为 `version >= 5`；在 `docs/verification.md` 或 README 单独列出"手动验证脚本"清单，避免 12 个脚本处于既非 CI 又无人知道的状态。
7. `bundle.resources` 改为显式列出核心二进制，排除 `README.txt`。
8. `settings.patch` 对 `deletedAccounts`/`accountAliases` 传入非当前值时返回明确错误，或从 `Settings` 公开结构中移到只读字段。

### 阶段 3：结构收敛（中等改动，分别可独立进行）

1. **估值表合一**：单表 `estimates(kind, …)`，`Estimate` 改为 `enum { Quota(QuotaEstimate), Credit(CreditEstimate) }`，删除所有 `is_credit()` 分支与 `UNION ALL`。
2. **仪表盘单实现**：`dashboard_usage_reference` 降级为 `#[cfg(test)]` 的测试预言机，生产路径只走缓存版并在出错时返回错误而非静默换算法。
3. **容量单源**：估值记录引用 `capacity::Capacity` 快照而非自行计算周容量。
4. **`agy` 别名收口**：所有入口（设置加载、RPC 参数、日志解析）在边界调 `canonical()`，UI 的 `providers` 映射删除 `agy` 键及三处过滤，`['antigravity','agy'].includes` 改为 `=== 'antigravity'`；Swift 同理。保留 `providers.rs` 一处迁移逻辑即可。
5. **RPC 方法清单单源**：核心新增 `methods.manifest` 返回 `{name, lane, invalidates}`，Tauri `update_events`/`engine_for`、Swift `configurationMethods`、Web 失效列表都从它派生；或至少在核心用一个 `const METHODS: &[(name, Lane, Events)]` 表驱动并在 Swift/Web 加启动期一致性断言。
6. **Swift 复用核心**：`SessionMonitor` 改调 `sessions.list`；`MonitorSelection` 改调核心已有的推荐接口；`PanelAccountPreference` 若要保留本地实现，补一份与 `ui-state.js` 共用的测试夹具。
7. **面板状态持久化对齐**：决定 macOS 是否持久化仪表盘筛选；删除 `Models.swift:34` 失效迁移；`AieyesApp.swift:33` 截图覆盖列表改为遍历一个集中定义的键枚举。
8. **常量单源**：300/10080、默认目录、默认二进制通过 `hello` 或 `settings.get` 附带 `defaults` 下发，或与相位色一样由脚本生成到 JS/Swift。
9. Web 四份转义函数合并为 `app.js` 一份；删除 `#account-fields` CSS 与 `estimateLabel`；核心用 `#[serde(skip_serializing)]` 裁掉前端不读的 `Dashboard.days`、`priceUpdatedAt`、`Tokens.reasoning`、`desktop_info` 多余键（先确认 Swift 也不读）。
10. Swift 死视图/常量/状态按 §3.3 删除；`settingsStage` 若有价值则在 `SettingsView` 显示。

### 阶段 4：仓库卫生

- 清理 `.local/windows-appearance/baseline-src`、`dist/` 旧 DMG、`scripts/__pycache__`；为 `.local` 增加按日期归档或 README 说明。
- 给 7 份被取代的文档加 "已由 … 取代" 标头或移入 `docs/archive/`。
- `.gitignore` 去重。

---

## 九、已验证无问题（无需处理）

- 相位色四处副本一致：`docs/ui-phase-colors.json` ⇄ `tokens.css:184-199`（生成）⇄ `phase_colors.rs`（生成）⇄ `SessionActivity.swift`（校验），`sync-phase-colors.py --check` 通过。
- 应用版本 0.1.29 在 Cargo.toml ×2、Cargo.lock ×2、tauri.conf.json、Info.plist 一致；`release.py` 强制。
- Web 与 Swift 都用 `settings.patch` + `base` 乐观合并（`app.js:314,1015`、`agent-settings.js:112`、`EngineClient.swift:679`），无"整对象覆盖"路径。
- `store.settings()`/`prices()` 每次读库，不缓存；Tauri 6 个引擎通道间不存在设置/价格陈旧问题；用量缓存按 `usage_day_revision` 版本失效，价格重算会触发。
- 记录新鲜度阈值一致：`EngineClient.swift:470` 与 `ui-state.js:54` 均为 `max(60, historyInterval×2)`。
- Swift ↔ 核心的设置键名与数值默认、配额 JSON 字段、成本/缓存命中率公式、`compact`/`compactMoney` 阈值一致。
- `agyAuth.*` 全部 7 个动作、`codexAuth.enable/adopt/inspect/openUrl/login.*/profiles.*/switch.*`、`wakeups.*` 全部 11 个动作均有调用方。
- `hello.protocolVersion` 在核心、`codex_auth/local.rs`、`remote_codex_auth.py` 一致为 1。
- Capability 与运行时窗口标签（`main`、`floating`）匹配；`floating.html` 的脚本依赖（`AieyesTheme.loadFonts`、`AieyesApp.openPanelAccounts`）均已加载。
- `scripts/vendor/tomli`、`scripts/fixtures/codex-process-auth.json`、`scripts/ui-test-helpers.cjs` 均在使用。
