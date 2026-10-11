# 代码审计处理记录（2026-10-10）

基线：[原审计](code-audit-2026-10-10.md)，应用版本 0.1.29。按当前源码和锁定依赖核实；原报告保留为历史证据。逐项核实与处理已完成，包括[独立验收](code-audit-2026-10-10-acceptance.md)发现的三项补充修复；不成立或存在冲突的建议在下方列出。

2026-10-11 后续补充[手动全量重扫](manual-history-rescan.md)，使已经读完的原始日志可由用户明确触发重读并修复历史重复；启动和定时同步保持增量。该功能的新增回归及开发版构建结果独立记录在该说明中。

## 原问题总览的逐项结果

| # | 当前结果 | 实现或证据 |
|---|---|---|
| 1 | 已修复供应商表、custom 迁移和显示名；唤醒限制保留 | `providers::PROVIDERS/known/requires_path/supports_wakeup`；迁移、来源验证和唤醒验证共用。DeepSeek/custom 没有订阅唤醒执行协议，不能仅扩大白名单 |
| 2 | 新 ID 规范化，条件迁移与去重；旧记录兼容保留 | `usage::event_id`；`Store::migrate_usage_ids` 使用事务、每批 512 条。旧 payload 曾 `#[serde(skip)] import_identity`，没有可逆身份，不能批量猜测；重读日志获得身份后，`Store::put_event` 在已验证的归属范围内原子合并全部匹配哈希，保留冻结价格、Token 最大值及所有来源关系。单个旧哈希再于下次打开时规范化；兼容哈希集中在 `providers::usage_ids` |
| 3 | 已移除 | Rust/Swift/夹具的 `githubRepository` 已删除；配置迁移到 6，旧 JSON 键被忽略 |
| 4 | 已修复 | `release.py` 校验正整数构建号并要求超过上一较低稳定版本标签；正式标签检查拒绝浅克隆，CI 获取完整历史；覆盖不递增、无效构建号、预发布/更新标签和浅克隆 |
| 5 | 已处理 | 移除 `updates.check/open`、`open_release_page`、`codexAuth.homes.list`、`read_quota`。`sessions.list` 保留并已有 Swift 生产调用；删除 Swift 本地日志监测重写，采用独立核心通道 |
| 6 | 已修复 | Swift pending 状态“需要确认”、原因提示、唤醒失败和延迟文案与 Web 对齐；日期使用 POSIX 24 小时制；估值窗口按组唯一选择，Claude 专属 ID 只对 Claude 生效 |
| 7 | 已修复 | 两份 workflow 使用 `ci-checks.sh`，检查所有 Web JS 和相位色；Desktop 的 push / pull_request 路径均包含契约与相位色单源 JSON；账户设置真实核心回归加入 `test:ui`；统一 Windows runner/证据名，提前准备核心二进制 |
| 8 | 原建议不成立，保留配置 | 锁定 `tauri-plugin-updater 2.13.1` 的 `config.rs` 明确声明 `require_signed_version`，`pubkey` 是必填反序列化字段。删除会降低校验或使插件启动失败；代码旁说明占位字段用途，并增加真实 Config 解码回归 |
| 9 | 已修复 | 单表 `estimates`，以 `(kind,id)` 为主键；`Estimate::Quota/Credit` 使用互斥字段。旧表事务迁移，保留原 payload/冻结价格；同 ID 跨类型、失败回滚、重复打开和 RPC 类型隔离均有回归；原生混合采样列表也使用类型加 ID 区分 |
| 10 | 已修复 | 仪表盘生产路径只使用缓存实现，错误正常返回；参考实现和其聚合助手仅在 `#[cfg(test)]` 下编译，继续作为结果对照 |
| 11 | 已修复 | 核心以 `CARGO_PKG_VERSION` 注入两份远端脚本及部署副本；独立运行脚本标记 development，不再伪报旧发布版本 |
| 12 | 已处理 | 删除四个 Swift 死视图、三个 Palette 常量、未读模型字段/设置状态和闭包；Web 转义集中到 `AieyesUI.escapeHTML`，删除死 CSS/estimateLabel；供应商别名只在边界/偏好迁移处理；裁剪未读显示字段，保留历史存储数据。截图专用入口保留，见下文 |
| 13 | 已接入并修正 | `verify-models.swift` 不再孤立；原生测试使用真实核心 dashboard/settings 输出，断言配置版本至少 6 |
| 14 | 已修正文档 | 更新 Agents/账户入口、六个标签、DeepSeek、发布状态、配置版本、13 项细分显示；七份历史文档添加后续记录链接；增加手动验证入口清单 |

## 与审计建议冲突或不成立的内容

- **DeepSeek/custom 唤醒**：核心 `wakeups::manifest` 及 `remote_wakeup.py` 只实现三种可验证 CLI 协议。DeepSeek 是 API 余额查询，自定义来源没有统一订阅 CLI。维持拒绝行为，能力判定已集中管理。
- **旧事件一次性重算并删除所有补丁**：旧稳定身份未保存，哈希不能逆推。新身份持久化；有证据的记录原子迁移，缺失身份的历史保留原 ID。重新导入与新增副本仍能去重；回归覆盖真实缺失身份的旧 payload。
- **空 pubkey / requireSignedVersion**：当前实际依赖与原报告所述不同，两项均保留。公钥占位用于解码，实际公钥在代码内覆盖；签名版本约束保持开启。
- **负时间戳缓存失效**：当前 SQL 已使用 `stamp - CASE WHEN stamp<0 THEN 86399 ELSE 0 END` 再除以 86400，与 `div_euclid` 一致，此条已不成立。
- **Swift 丢弃 reason**：模型原本已有 `reason`，详情和历史已读取；真正缺口是 `issueLabel` 对 pending 忽略原因，现已修正。
- **速率 >300 秒断档**：保留。两帧间隔很长时不把休眠期间的累计计数伪装成当前速率；CPU、网络、磁盘原始读数仍保留，下次有效采样恢复速率。
- **同秒容量观测**：保留。配额快照时间为秒，`observe_capacity` 已拒绝 `updated_at <= last.updated_at` 的重复/倒序观察，主键并非在有效观测后额外丢样本。
- **周容量两个数字**：估值已经持有 `capacity::Capacity` 快照；`weekly_checkpoint_percent` 和 segment 周消耗是特定采样段的实测消耗，capacity 是跨观测的倍率学习结果。两者语义不同，不能替换为同一个数字。
- **前端使用的设置字段**：`menuMetric`、历史/服务器刷新间隔均有生产消费方，保留。CLI `--call` 已用于真实核心原生模型验证，不是死入口。
- **截图专用代码**：`SessionPhase.idle`、可访问表面预览属于受支持的截图矩阵，保留。Windows 专用模块、签名验证工具、tomli、夹具和 UI helper 也保留。
- **Windows 与基础窗口尺寸**：当前 Windows 1294×840 与 Linux 原生装饰窗口的差异已由平台设计说明明确规定，不能直接按“漂移”统一。
- **MonitorSelection 改调“已有推荐 RPC”**：核心只有 `recommended_filesystem`/`filter_devices` 函数，没有该独立 RPC。Swift 需要同步重筛选保留的采样，采用共享生成规则和跨语言夹具，避免在视图计算时增加跨进程调用。
- **直接跳过序列化 Tokens.reasoning / 估值内部快照**：Tokens 同时用于事件与解析游标持久化；全局跳过会损失历史计数。只裁剪 Dashboard 显示投影；估值的边界、原价格和容量快照仍供修复与冻结算法使用，两类估值互斥字段已从类型上裁减。
- **缓存错误回退**：移除静默回退；现有界面错误路径保留先前读数并提示失败，避免隐蔽地换算法。

## 优化阶段覆盖

阶段 1 的六项已处理，包括上述有证据的例外。阶段 2 的八项已处理，更新配置删除建议按不成立记录；手动脚本入口见 [手动验证](manual-verification.md)。阶段 4 已清理重复 ignore、Python 缓存和三个 `.DS_Store`，旧源码/DMG 已移动到 `.local/archive/2026-10-10/`，保留构建缓存并在本地 README 说明。

阶段 3：

| 建议 | 进度 |
|---|---|
| 1 估值单表与类型收敛 | 完成；两类记录分字段/算法，存储、列表查询和修复共享；迁移失败原子回滚 |
| 2 仪表盘生产单实现 | 完成，参考算法仅供测试 |
| 3 容量单源 | 已核实已有 Capacity 快照；采样消耗与倍率不同，保留 |
| 4 agy 边界规范化与 UI 清理 | 完成；Web API、Swift EngineClient 的 payload 边界和偏好迁移规范化；UI 展示表、过滤与分支只使用 antigravity；Web 唤醒来源直接读取 `AieyesContract.wakeupProviders`；旧输入夹具继续验证迁移 |
| 5 RPC 方法清单单源 | 完成；`docs/core-contracts.json` 生成核心 METHODS、Web 失效表与 Swift 通道表；Tauri 通道/失效/刷新键直接使用核心表，保留条件事件；提供 methods.manifest 自省；CI 校验生成输出及客户端静态方法；hello 返回契约指纹，Web/Swift 在启动时检查匹配 |
| 6 Swift 复用与共享夹具 | 完成；会话使用核心 RPC，账户选择/估值窗口共用夹具；文件系统排除表单源生成，并由 Rust/Swift/Web 共用推荐夹具，Rust/Swift 共用设备筛选夹具 |
| 7 面板筛选持久化与截图键集中 | 完成；macOS 保存 Agent/来源/账户/模型/天数及面板/详情成本显示；恢复时校验范围、规范化别名和已删除选择；NativePanelKey 集中固定键/动态前缀，截图遍历，删除旧 provider 键迁移 |
| 8 跨语言默认常量单源 | 完成；同一契约生成 5h/7d 分钟数、供应商名称/顺序、唤醒能力、默认目录/二进制；核心自动发现、Agent 创建、窗口判断及两端设置使用生成值；空 Antigravity 二进制也回退为 agy |
| 9 Web 清理和 payload 裁剪 | 完成；Dashboard 不发送 days/priceUpdatedAt，显示响应中的 tokens 去掉 reasoning，desktop_info 去掉未读 generation/dark/trayAvailable；Swift 删除对应模型字段；事件/解析状态 reasoning 和估值修复快照保留 |
| 10 Swift 死视图/常量/状态 | 完成；截图支持入口保留 |

## 已完成的验证

- `cargo test --workspace --locked --offline`：168 项通过，2 项按原定义忽略（子进程夹具、手动性能基准）；包括新增三项历史去重回归，覆盖两个旧哈希均无身份的真实日志重放、新增副本、别名冲突、冻结价格、完整 Token/来源关系、账号与无账户来源隔离及合并失败回滚。原有来源解绑/重新关联回放回归也通过。首次沙箱执行的进程/回环监听限制已在沙箱外复验排除。与 Swift 编译并发时出现过进程清理测试的 3 秒上限失败；同环境单独复验及最终完整回归均通过，未放宽断言。
- `sh scripts/ci-checks.sh`：发布 7 项、采集器 4 项、唤醒 11 项、Node 28 项及所有 JS 语法/相位色及跨语言契约生成校验通过；新增 Web 唤醒来源回归验证能力表收缩/扩展及来源启用、关联、主机状态过滤。契约生成还验证了 UTF-8/CRLF 输入兼容，CI 显式安装 rustfmt。
- `npm run test:ui --prefix apps/desktop`：16 个浏览器集成脚本整链通过，含新增真实核心账户设置回归。
- `sh scripts/test-native.sh`：整链通过，含真实核心模型解码及新的会话 RPC 验证；账户/窗口夹具与共享推荐/筛选规则通过；偏好保存/恢复、截图覆盖与旧身份边界另经重新编译的 `verify-selection` 和 Node 验证通过。
- Swift 应用完整 debug 编译：最终供应商边界、共享契约、筛选持久化和估值解码修改后通过。
- Tauri 宿主验证：准备真实核心二进制后 22 项通过，包含 updater 配置解码回归。

补充修复后的核心、共享检查、浏览器及 Tauri 复验全部通过，最新日志为 `.local/code-audit-fix-core-tests.log`、`.local/code-audit-fix-ci-checks.log`、`.local/code-audit-fix-ui-tests.log` 和 `.local/code-audit-fix-tauri-tests.log`；完整结论与首次发现的证据见[验收记录](code-audit-2026-10-10-acceptance.md)。原有证据日志保存在 `.local/code-audit-*-tests.log`、`.local/code-audit-shared-checks.log`、`.local/code-audit-shared-native-fixtures.log` 和 `.local/code-audit-swift-build.log`。本机测试不能代表 Windows/Linux 实机安装或正式发布；此轮未发布、提交或推送。
