# 代码审计实施验收（2026-10-10）

验收对象：[原审计](code-audit-2026-10-10.md)、[实施记录](code-audit-2026-10-10-implementation.md)。验证基于 `f102fe9` 上当前未提交工作区，应用版本 0.1.29。

**结论：修复后通过本机代码验收。** 首次验收发现一个 P1 历史统计问题及两项 P2 缺口；三项修复现已完成，并补充回归。下方保留首次发现的证据，原问题表已更新为修复后的结果；复验记录见文末。

## 首次验收发现（均已修复）

### P1：缺失稳定身份的历史双哈希记录，重读日志后仍重复计数

- 对应原问题 #2，以及实施记录关于“重读日志补齐身份后再次迁移”“重新导入与新增副本仍能去重”的声明。
- 触发条件：旧库已同时保存同一账户、同一事件的 `agy` 与 `antigravity` 哈希，两条旧 payload 都没有 `importIdentity`。这是此次修复应处理的历史重复记录组合。
- 首次验收时，`Store::put_event` 在找到第一条已观察的候选 ID 后立即退出；`providers::usage_ids` 将规范 ID 放在候选首位。重放只补齐规范记录，旧哈希记录仍没有身份，后续迁移继续跳过它。
- 使用真实核心、临时 SQLite 库及一条 100 Token JSONL 记录复现：迁移前总量 200；删除采集游标、完整重读、再次打开核心后，总量仍为 **200**，数据库仍有两条事件，其中旧哈希的身份为 null。预期是一个事件、总量 **100**。
- 现有 [code_audit.rs:203](../crates/core/tests/code_audit.rs#L203) 只覆盖“一个旧哈希、没有规范副本”；另一个去重测试的重复 payload 都带身份。两者没有覆盖上述组合。
- 修复要求：在重读获得稳定身份时，在已验证的账户/来源范围内检查全部匹配哈希并原子合并，保留冻结价格、完整 Token 与来源成员关系；加入“两种哈希均无身份”的回归，并复验别名冲突、无账户来源和不同账户隔离。

首次复现脚本与输出保存在 `.local/code-audit-acceptance-repro-duplicate.py` / `.log`。脚本只操作自动清理的临时库，其断言针对修复前的缺陷；修复后的预期由 `code_audit.rs` 新增回归验证。

### P2：Web 唤醒页仍使用独立供应商白名单

- 对应阶段 3 的第 4、8 项。实施记录声称 UI 过滤只使用规范供应商且两端使用生成的唤醒能力。
- 首次验收时，[wakeups.js](../apps/desktop/web/wakeups.js) 使用 `['codex','claude','antigravity','agy']`，没有读取已生成的 `AieyesContract.wakeupProviders`；Swift 已使用 `CoreContract.wakeupProviders`。
- 当时支持集合相同，所以常规 UI 回归通过。补充 VM 验证将契约能力设为仅支持 Codex，Web 仍打开 Antigravity 唤醒编辑器，证实该过滤不会跟随契约变化。
- 修复要求：Web 来源选择使用生成的能力表，移除业务过滤中的 `agy`；增加能力表变化影响来源选择的回归。

复现脚本与输出保存在 `.local/code-audit-acceptance-repro-wakeup.cjs` / `.log`。

### P2：相位色单源文件的修改不会触发 PR 检查

- 对应原问题 #7 的 CI 覆盖收敛。首次验收时，`ci-checks.sh` 已加入相位色检查，但 [desktop.yml](../.github/workflows/desktop.yml) 的 push / pull_request 路径过滤仅显式加入 `docs/core-contracts.json`，遗漏 `docs/ui-phase-colors.json`。
- 只修改相位色 JSON、遗漏生成输出的 PR 不会触发 Desktop workflow。仓库另一份 Release workflow 仅在标签或手动触发时运行，不能补上该 PR 检查。
- 修复要求：将 `docs/ui-phase-colors.json` 加入两个路径过滤，确保单源文件发生变更时执行一致性校验。

## 修复内容

- [Store::put_event](../crates/core/src/store.rs)：在历史来源归属或当前账户范围内收集全部候选哈希，校验数据库中的供应商和账户；优先保留规范记录的冻结价格，合并 Token 最大值及计费元数据，转移全部来源成员后删除重复事件。直接写入开启事务，日志导入复用调用方事务，失败回滚。
- [Web 唤醒来源](../apps/desktop/web/wakeups.js)：改用生成的 `AieyesContract.wakeupProviders`；新回归实际调用新建任务入口，验证能力表增减、规范供应商及启用/关联/主机条件。
- [Desktop workflow](../.github/workflows/desktop.yml)：push / pull_request 两处路径均加入 `docs/ui-phase-colors.json`，使只修改相位色源文件的 PR 也能执行生成一致性检查。

## 原问题逐项结果（修复后）

| # | 验收结果 | 核验摘要 |
|---|---|---|
| 1 | 通过 | 核心供应商能力集中；custom 迁移和显示名修复；DeepSeek/custom 唤醒限制有执行协议依据 |
| 2 | 通过 | 无身份历史双哈希重放立即去重；冻结价格、完整 Token/来源关系、别名与账号隔离、失败回滚均通过 |
| 3 | 通过 | 无用配置键从生产模型移除；旧键忽略，迁移版本为 6 |
| 4 | 通过 | 正整数构建号、上一较低稳定标签和正式发布的完整历史校验均有实现与回归 |
| 5 | 通过 | 死 RPC/函数清除；sessions.list 已有真实 Swift 生产调用与独立通道 |
| 6 | 通过 | 所列状态、原因、日期与窗口选择已对齐；共享窗口夹具通过 |
| 7 | 通过 | 共享检查、所有 Web JS、真实核心账户 UI 回归已接入；两处触发路径包含相位色源文件 |
| 8 | 通过（原建议更正） | 锁定 updater 2.13.1 的真实 Config 必须有 pubkey 且支持 requireSignedVersion，保留配置合理 |
| 9 | 通过 | 单表、类型互斥、跨类型同 ID、迁移回滚及 RPC 隔离均通过 |
| 10 | 通过 | 生产只使用缓存聚合；参考实现及辅助聚合仅供测试 |
| 11 | 通过 | 核心向远端辅助脚本和部署脚本注入应用版本 |
| 12 | 通过 | 所列清理、转义集中及显示投影裁剪通过；Web 唤醒过滤共用生成的能力表，移除独立白名单和业务别名 |
| 13 | 通过 | 原生模型验证接入测试链，使用真实核心输出 |
| 14 | 通过 | 所列文档漂移、历史文档后续链接和手动验证入口已处理 |

容量快照与采样段消耗语义不同、长采样间隔不输出当前速率、同秒/倒序容量观察被拒绝、持久化 reasoning 不可全局裁剪等例外，有源码依据，可接受。原报告应继续作为历史基线保留。

## 首次独立验收的验证

| 检查 | 结果 | 日志 |
|---|---|---|
| `cargo test --workspace --locked --offline` | 165 通过、2 按原定义忽略 | `.local/code-audit-acceptance-core-tests.log` |
| `sh scripts/ci-checks.sh` | 发布 7、采集器 4、唤醒 11、Node 27 通过；全部 JS 语法、相位色、81 方法契约校验通过 | `.local/code-audit-acceptance-ci-checks.log` |
| `npm run test:ui --prefix apps/desktop` | 16 个浏览器集成脚本整链通过 | `.local/code-audit-acceptance-ui-tests.log` |
| `sh scripts/test-native.sh` | 整链通过，包括真实核心会话 RPC、模型解码和偏好夹具 | `.local/code-audit-acceptance-native-tests.log` |
| Tauri 宿主（先 prepare-wakeup，再 locked/offline/custom-protocol 测试） | 22 通过，包含 updater Config 解码 | `.local/code-audit-acceptance-tauri-tests.log` |
| Swift 完整 debug 构建 | 通过 | `.local/code-audit-acceptance-swift-build.log` |
| `git diff --check` | 通过 | 无空白错误输出 |

Rust 首轮的进程夹具/回环监听受沙箱限制，在沙箱外完整复验通过；浏览器集成同样在沙箱外运行。SwiftPM 首轮的嵌套沙箱失败，使用仓库构建方式 `--disable-sandbox` 和工作区缓存路径后通过。这些环境失败未计为产品缺陷。

验证限于本机 macOS。未进行 Windows/Linux 实机安装、正式发布、提交或推送；这些不属于本次代码修复验收结论。

## 修复后复验

| 检查 | 结果 | 日志 |
|---|---|---|
| `cargo test --workspace --locked --offline` | 168 通过、2 按原定义忽略；新增三项历史去重回归全部通过 | `.local/code-audit-fix-core-tests.log` |
| `sh scripts/ci-checks.sh` | 发布 7、采集器 4、唤醒 11、Node 28 通过；JS 语法、相位色及 81 方法契约检查通过 | `.local/code-audit-fix-ci-checks.log` |
| `npm run test:ui --prefix apps/desktop` | 16 个浏览器集成脚本整链通过，含账户、Antigravity 与唤醒流程 | `.local/code-audit-fix-ui-tests.log` |
| Tauri 宿主 locked/offline/custom-protocol 测试 | 准备修复后的真实核心二进制后，22 项通过 | `.local/code-audit-fix-prepare.log`、`.local/code-audit-fix-tauri-tests.log` |
| Desktop CI 路径核验 | push / pull_request 两处均覆盖 `docs/ui-phase-colors.json`，并执行共享一致性检查 | `.github/workflows/desktop.yml` |
| `git diff --check` | 通过 | 无空白错误输出 |

双哈希回归使用真实 JSONL 导入路径：两个无身份旧事件的总量从 200 修复为 100，当次重放即去重；新增日志副本及重新打开数据库后仍为一个事件、两个来源关系，成本保持原价格计算的 1.0。其他回归验证 Token/计费元数据合并、账号别名冲突和无账户来源隔离，以及删除重复事件失败时统计、身份及来源关系一起回滚。来源解绑/重新关联的现有完整回放回归也通过。

Web 回归修改生成能力表，验证移除和新增供应商都会影响实际新建任务来源选择；禁用、未关联和主机不可用的来源继续排除。实施记录已同步上述修复及测试结果。
