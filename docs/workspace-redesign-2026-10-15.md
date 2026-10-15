# 工作区 UI 改版 · 2026-10-15

本轮落实已确认的界面方案：以账户额度和会话状态为日常入口，用分类色组织 Agent 与服务器资源，将完整用量分析和设置放入统一主窗口。参考方向来自前期讨论的 DeepSeek Monitor、AgentPet 和 F50 Monitor。

macOS 继续使用 SwiftUI / AppKit，Windows / Linux 继续使用 Tauri 2。共享 Rust 核心的数据采集、账户认证、估值与部署流程沿用现有实现。

## 导航与信息组织

| 分组 | 主窗口页面 |
| --- | --- |
| 监测 | Agent 总览、用量分析、服务器 |
| 配置 | 接入与账户、机器与采集、模型价格、定时唤醒 |
| 偏好 | 外观、桌宠与会话、网络代理、刷新与历史、通知、更新与关于 |

macOS 设置并入主窗口；菜单栏、桌宠、设置深链接和 `⌘,` 都进入对应设置页，`⌘1` / `⌘2` / `⌘3` 切换三个监测页面。主窗口保留监测视图和已经访问的设置容器，切换页面时保留阅读位置、草稿和已有的保存流程。侧栏可以收起，短窗口中可独立滚动。

面板保持约 450 × 720，顶部保留 Agent / 服务器切换，底部操作固定，内容独立滚动。Agent 页提供总览及各 Agent 入口，按最后确认的顺序呈现会话摘要、今日用量、按 Agent 分组的账户限额与后台采样入口。趋势、模型分布和每日明细集中到主窗口的用量分析页，模型分布继续使用圆环图。

## 账户、服务器与会话

- 新账户卡片默认收起，已有的展开偏好继续生效。摘要保留账户名称、可用身份信息、订阅、额度、重置时间和余额；切换账户与账户级「查看分析」操作放在展开内容中。
- Antigravity 摘要保留全部模型组及各自的额度、时间窗口和重置时间。分组顺序跟随该组最靠前的手动排序账户，原有账户排序和面板显示账户选择继续有效。
- 分组标题显示账户数量与需留意数量。后台额度推送同步更新提醒，不重建分组标题；刷新保留展开状态、滚动位置和键盘焦点。
- 服务器以 CPU、内存色块和细进度条为主，GPU、存储、网络按类别分组。支持展开详情，缺失读数显示 `—`，零值仍显示为零；采样时间、延迟、失败和暂停状态明确区分。
- 会话先显示数量及状态分布，展开后显示 Agent、机器、状态和真实的最后更新时间。等待输入和执行失败更醒目；完全空闲时隐藏摘要，来源异常仍可见，不推测任务名称或持续时长。
- Token 细分保留点击入口；Web 使用顶层弹出层，限制在面板可见内容区内，避免被底栏裁切。macOS 继续使用原生 popover。

## 设置与状态隔离

接入设置按 Agent 提供四个独立分区：账户、API 连接、机器与目录、估值与采样。账户区首次默认展开，其余默认收起；展开偏好按 Agent 保存，深链接自动展开目标区。登录、账户切换确认、保存失败后的草稿、估值同意与后台采样流程保留。

| 范围 | 状态规则 |
| --- | --- |
| 面板 | 独立保存 Agent、账户及既有筛选偏好 |
| Agent 总览 | 与面板、用量分析分别保存筛选 |
| 用量分析 | 独立保存 Agent、账户、来源、模型及日期范围 |
| 显式「查看分析」 | 带入当前 Agent / 账户，清除分析页的来源和模型限制，保留分析页日期范围 |
| 菜单栏总量 | 始终显示全部账户的今日汇总 |

普通页面切换不传递筛选。Web 的新存储键为 `aieyes.view.v3.{panel,overview,analysis}`，原生为 `dashboard.filters.v2.{panel,overview,analysis}`；新键缺失时读取旧偏好。旧账户别名在设置加载后解析，账户或来源移除时统一校正三个范围。账户展开和面板账户选择继续使用原有偏好格式。

Web 的请求按范围保存缓存并拒绝过期结果；原生对相同筛选复用查询结果，在各范围查询完成后一起发布，避免把不同筛选的数据混在同一轮显示中。

## 视觉规范

[ui-design-tokens.json](ui-design-tokens.json) 继续作为 Swift / CSS 共用常量源，生成文件由 CI 检查同步。正文 15、说明 12、主要数字 24，卡片圆角 16、普通控件圆角 8。macOS 使用系统字体，Windows 保留随包的 HarmonyOS Sans。

卡片采用半透明底、分类渐变、边框和轻阴影；资源类别色与额度警告色分别表达类型和状态。系统减少透明度、增强对比度及强制颜色模式有对应降级；交互动画尊重减少动态效果设置。手动选择明暗主题时，单色 Agent 图标跟随应用主题，避免受系统主题影响而失去对比度。

## 验证记录

| 检查 | 本轮结果 |
| --- | --- |
| `npm run test:ui --prefix apps/desktop` | 完整通过，含新增工作区回归、账户 / 认证、估值、服务器、草稿、更新、Windows 控件与双窗口主题同步 |
| `node scripts/test-windows-ui.cjs` | 通过；六场景浅深色、Windows/Linux 布局、趋势标签、模型圆环、字体、材质降级、最小窗口和分数 DPI |
| `sh scripts/ci-checks.sh` | 通过；共享 Python 检查、生成文件同步、JS 语法、32 项 Node 测试及重置时间显示 |
| `sh scripts/test-native.sh` | 通过；新增筛选隔离、旧偏好迁移、上下文跳转、移除账户校正及缺失网络计数回归 |
| Tauri `cargo test --locked --features custom-protocol` 与 Clippy | 25 项通过，含详情目的页与可选筛选字段转发；Clippy `-D warnings` 通过 |
| `AIEYES_OFFLINE=1 sh scripts/build-macos.sh debug` | 最终原生构建通过，产物 `dist/Aieyes.app` |
| 最后样式调整后的工作区与审计用例 | 通过；面板密度、Token 弹出层边界、强制颜色及焦点 / 滚动保留 |
| 原生截图 | 最终构建的 24 张矩阵通过生成与视觉复核 |

完整 Web 回归中的账户列表性能用例 P95 为 6.5 ms，挂载 9 行，整页设置重建 0 次。该结果来自当前宿主的合成数据用例，不代表目标平台所有数据规模的性能。

新增 [test-workspace-ui.cjs](../scripts/test-workspace-ui.cjs) 已加入默认 Web 测试链，使用 [workspace-ui-fixture.cjs](../scripts/workspace-ui-fixture.cjs) 合成数据。原生模型回归位于 [verify-macos-ui.swift](../scripts/verify-macos-ui.swift)。

预览和日志位于 `.local/ui-redesign-20261015/{native,web}/` 与 `.local/ui-redesign-*.log`。原生矩阵覆盖主窗口总览 / 分析 / 服务器 / 接入设置的常规和最小尺寸、720 / 440 高度面板、会话展开、无障碍表面，均含浅深色。Web 预览覆盖对应工作区、短面板、最小设置页、强制颜色与 Token 细分。

重新生成本轮原生预览：

```sh
AIEYES_CORE_PATH="$PWD/scripts/preview-quota-core.py" \
AIEYES_DATA_DIR="$PWD/.local/ui-redesign-20261015/native-data" \
AIEYES_UI_SCENARIO=workspace AIEYES_UI_WORKSPACE_ONLY=1 \
dist/Aieyes.app/Contents/MacOS/Aieyes --render \
"$PWD/.local/ui-redesign-20261015/native"
```

截图使用合成账户和服务器数据。macOS 完成原生构建、模型回归和离屏视觉检查；完整的原生鼠标 / 键盘工作流仍需桌面验收。Windows / Linux 本轮在 macOS 宿主通过 Chromium 平台夹具和 Tauri 宿主测试验证，尚未覆盖真实 WebView2 / WebKitGTK、DWM 材质、托盘、多屏混合 DPI、安装包和窗口管理器行为。
