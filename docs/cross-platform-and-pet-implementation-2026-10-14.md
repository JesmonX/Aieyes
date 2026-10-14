# 三端交互一致性与桌宠改进验收记录

日期：2026-10-14。基线：`5fb1c1e`（0.1.37）。对应[已确认实施计划](cross-platform-and-pet-plan-2026-10-14.md)。本记录接续被中断的实现，保留原工作区修改；当前未提交或发布。

状态：已完成已确认的实现、自动化回归与本机视觉验收；Windows/Linux 实机边界见文末。

## 已实现行为

| 计划编号 | 实现与验收内容 |
| --- | --- |
| 01 | 条件空态先设为最近 365 天并清空自定义日期，再打开详情；保留其他筛选。 |
| 02、11、18 | 常规设置连续编辑排队保存，输入保持可编辑；成功就地显示，失败保留草稿并可重试。保存返回只确认已提交值，接受 core 规范化结果，保留期间的新编辑，包括改回保存前的原值。登录、切换和 API 操作后的配置刷新按字段及记录身份合并，保留无关设置和模型映射草稿；快捷主题保存也保留等待期间的新选择。 |
| 03、04、05、17 | 统一机器行与操作列，窄窗口按一致规则换行；菜单和多行输入使用有效主题背景。同名目录显示可区分路径，长路径可查看全文。折叠额度卡标题、数值与进度条分层，等宽列采用 12 逻辑像素间距。 |
| 06、13 | 自动选择每种 Agent 排序前五个可显示账户；手动模式不限数量、允许为空、保留明确选择，新账户不自动加入，选择失效不补选；提供恢复自动模式入口。 |
| 07、10 | 已关联且身份匹配的账户／目录可在离线或暂停时保存唤醒草稿；部署、探测和运行继续检查执行条件。登录、切换、额度、估值统一考虑 Agent 开关、机器选择、机器采集和目录绑定／启用状态；不可用时提供原因及设置入口。 |
| 08、09 | 唤醒列表先返回，各任务独立刷新；任务级操作只限制自身，成功只清理自身操作错误。请求版本防止旧列表、状态或错误覆盖新结果。账户删除与任务操作仍协调互斥。 |
| 12 | 前置命令按账户 × 机器保存，只在账户设备设置编辑，同机目录共享并由新关联目录继承。远程旧代理值不再应用，不转换成命令；本机代理保留。授权、切换、额度和唤醒使用相同命令。已有部署后台同步且保留启停状态，失败保持待同步并可重试。 |
| 14、15 | 保持已确认的不调整项：更改唤醒 CLI 路径仍清空模型；名称／提示的现有 UI 字符数与 core UTF-8 字节限制不变。 |
| 16、19、20 | 主面板导航几何居中；底栏顺序为主题、更新、详情、设置、更多，设置使用明确齿轮。机器菜单外部点击、Esc 和选择操作后关闭，同一时间只打开一个，并支持键盘导航及 Esc 焦点恢复。 |
| 21、22 | 桌宠菜单提供「设置」「桌宠设置」，后者直达常规→桌面显示；macOS 新增工作区边缘 48 逻辑像素吸附，Windows/Linux 保持已有吸附和重置位置。中断使用 idle 动画，文字保留「已中断」。 |

## 桌宠导入、库与动画

- 支持本地 ZIP／`.codex-pet.zip`／`pet.json`、ZIP 或 manifest 资源链接（不依赖 URL 后缀）、已适配的 Codex Pet／Petdex／CodexPets 页面及命令、npm 包与 scoped/version/tag 写法。仅下载并解析素材，不执行 npx、安装器或包脚本，不依赖 Node.js。
- 成功导入立即选择并保留旧桌宠；相同内容去重，同名不同内容并存。库显示名称、可取得的来源／作者、idle 预览和使用状态；选择持久化。导入期间的旧库请求不能覆盖新的导入／选择结果。
- 迁移仅扫描 Aieyes 自身缓存，恢复全部可辨识有效条目并保留当前选择；缺失资料使用历史桌宠名称和可辨识日期，损坏条目逐项报告。
- 图片先在临时目录完整写入，再发布资源并原子提交库记录与当前选择；失败或提交前取消保留原选择并清理未提交资源。下载、解码、归档和迁移检查取消，最终短提交阶段保持完整。
- 库元数据与图像按需读取分离，成功校验有文件指纹缓存；仅可见预览播放，关闭、滚出视野或减少动态效果时停止连续播放。慢下载不阻塞常规设置保存。
- 保持 v1/v2 PNG/WebP 图集格式，不扩展 SVG/GIF/APNG 或 v2 额外视线动作。三端和库预览共用逐帧时长，idle 一轮 1.1 秒、工作动作一轮 0.82 秒；使用单调时间轴，轮询和数量变化不重启动画，当前动作持续循环。

格式与操作详见[桌宠导入与选择](pets.md)；会话监测见[桌面宠物与会话状态](desktop-pets.md)。

## 迁移与同步回归

配置升级为 v10。旧目录命令无冲突时迁移到账户设备；同一账户 × 机器存在不同历史命令时清空并标记 `preCommandResetRequired`，由用户重新设置。迁移不连接远端，也不启动暂停任务。

部署同步沿用实际已部署任务，保留尚未部署的草稿。运行或启用前比较已部署与当前账户环境，并在需要时先重试同步；同步失败继续保留待同步标记。并发停用与命令保存、同步期间再次编辑、移除失败后的运行／启用均有回归，避免旧操作结果覆盖新配置或待同步状态。

关键测试位于 [wakeup_sync.rs](../crates/core/tests/wakeup_sync.rs)、[wakeups/mod.rs](../crates/core/src/wakeups/mod.rs)、[pet_import/acquire.rs](../crates/core/src/pet_import/acquire.rs)、[pet_import/library.rs](../crates/core/src/pet_import/library.rs)、[Web 任务隔离回归](../scripts/test-wakeup-isolation-ui.cjs)、[原生任务隔离回归](../scripts/verify-wakeups.swift)和[原生通道回归](../scripts/verify-configuration-lane.swift)。

## 自动化验证

| 检查 | 结果 |
| --- | --- |
| `cargo test --workspace --locked` | 287 通过，0 失败，4 忽略。忽略项为 3 个由其他测试调用的子进程 fixture 和 1 个手动性能基准。 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过。 |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --features custom-protocol` | macOS 宿主上 23 通过，0 失败；已使用最终 core 代码重新验证。 |
| Tauri `clippy --all-targets --locked --features custom-protocol -- -D warnings` | 通过。 |
| `cargo fmt --all -- --check` 与 Tauri workspace 格式检查 | 通过。 |
| `npm run test:ui --prefix apps/desktop` | 最终完整运行 23/23 个脚本通过，退出码 0；覆盖两套平台 fixture 的连续保存、任务隔离、宠物库、主题与界面回归。 |
| `sh scripts/test-native.sh` | 完整通过，包含任务隔离、保存队列、宠物库异步刷新、吸附几何与动画。末次保存合并／快捷主题修复后，`verify-configuration.swift` 与 `verify-macos-ui.swift` 再次通过，覆盖改回原值、可选字段撤销和新主题输入。 |
| macOS Swift 构建 | 最终代码构建通过。 |
| `sh scripts/ci-checks.sh` | 最终核对通过：Python 86 项、Node 31 项及语法、版本、契约、配色检查。 |
| `python3 scripts/sync-core-contracts.py --check` | 通过，103 个方法；Rust／Swift／JS 与规范一致。 |
| `git diff --check` 与修改文档的本地链接检查 | 通过；10 份文档的 56 个本地链接有效。 |

日志保存在 `.local/ui-review-20261014-implementation/`，其中 core 全量为 `core/workspace-tests.log`、Tauri 为 `core/tauri-tests.log` 与 `core/tauri-clippy.log`、共享检查为 `core/ci-checks.log`、原生为 `native/native-tests.log`、Swift 构建为 `native/swift-build.log`。原生定点验证与图像检查的证据索引为 `native/VALIDATION.md`。该目录被 Git 忽略，测试和用户文档保存在仓库中。

## 界面验证与范围

Web 55 张截图位于 `.local/ui-review-20261014-implementation/web/`，宠物库和 50%／100%／125%／150%／200% 缩放截图位于其 `pets/` 子目录；覆盖 Windows/Linux 平台 fixture、明暗主题、窄面板、长路径、机器菜单、桌宠库和状态动画。长路径登录页 `windows-login-long-path.png`、`linux-login-long-path.png` 和窄菜单均已目视核对，无横向溢出并保留完整路径提示。

macOS 离屏截图位于 `.local/ui-review-20261014-implementation/native/{empty,single,multi,long,failure,capacity}/`，覆盖六种 fixture 的明暗主题、窄窗口、长名称、多账户、机器列表及桌宠库。对比度、六场景面板透明度和设置快照检查通过；已抽查长机器名窄布局和同名桌宠库，未见遮挡或横向溢出。库布局使用八条合成元数据和内置图集，导入正确性另由隔离回归验证。

原生 90 张缩放截图位于 `native/pet-scales/`。其中 `0.5-row0-light.png` 和 `0.5-row0-dark.png` 在首轮 SwiftUI 更新稳定前捕获了前一个多行状态文字，仅用于几何参考；后续 0.5 倍场景及 1.0／2.0 倍样本显示预期动作文字，逐帧时序另由回归验证。

本次宿主为 Intel macOS。Chromium 的 Windows/Linux 平台 fixture 和宿主 Tauri 测试不能替代 Windows WebView2、Linux WebKitGTK、原生托盘或 Windows 原生桌宠窗口实测；本机缺少完整 Windows 交叉编译工具链，也未执行对应系统构建。真实多显示器、混合 DPI、系统级拖放／焦点和 Linux 桌面环境行为仍需在目标系统验收。macOS 吸附几何与可见预览裁剪已用确定性测试覆盖，离屏图像不作为真实鼠标操作证据。

验证使用临时数据库、模拟 HTTP／SSH／CLI 及 fixture，没有进行真实账号登录、SSH 部署、第三方配置修改或线上发布。
