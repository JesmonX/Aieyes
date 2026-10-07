# 原生悬浮胶囊与跨端 UI 同步

2026-10-06。本轮保留 Windows 侧栏，统一内容和交互，macOS 继续使用原生 SwiftUI。

## 实现

- Windows 悬浮入口独立为 Win32/GDI+ 胶囊，144×44 逻辑像素。鼠标消息、拖动、原生状态提示和右键菜单在独立线程处理，不经过 WebView；操作队列与绘制线程分离。胶囊显示现有品牌图、状态和活动数。
- 点击打开／收起面板，悬停只看提示。菜单提供固定、重建面板（用于界面无响应）、主窗、设置、重置位置、隐藏和退出；托盘也能刷新。面板保持独立 WebView，默认 450×720，按工作区限制尺寸。
- 刷新销毁并重建面板，使用 generation 区分新旧就绪通知；保留胶囊、核心、后台采样和固定状态。面板页签、筛选、滚动和账户折叠使用本地展示状态恢复。创建失败或十秒未就绪时显示恢复失败，允许重试。
- Linux 保留 WebView 入口，使用相同胶囊外观及点击行为；位置算法支持矩形尺寸。macOS 保留菜单栏入口。
- 标题栏、侧栏和面板品牌图有固定尺寸。移除悬浮页依赖的内联初始化脚本，修复真实 CSP 下误判页面类型的问题；Windows 原生层关闭默认浏览器菜单，Web 层阻止浏览器快捷导航和文件拖入导航。
- 额度卡统一折叠，保留关键数值；新增 Web 采样管理入口、隐藏筛选提示及携带筛选打开详情。数据更新保留焦点和阅读位置，弹窗操作期间延后背景重绘。
- 剩余额度大于 30% 为主题色，10%～30% 橙色，低于 10% 红色；资源负载沿用高负载警示。余额和采样消耗保留两位小数，采用十进制四舍五入。入口为“估算 credit 价值”，结果仅展示 1000 credit 的 API 等价价值，待确认样本不当作有效结果展示。
- 估值算法和数据库格式没有改变，历史的两个估值字段继续保留。

新增桌面命令：desktop_detail（携带展示筛选打开详情）、desktop_panel_ready（generation 就绪确认）；desktop_action 增加刷新、切换面板和切换固定。原有 floating WebView label 和权限继续使用。

## 验证

- Web 回归涵盖真实 CSP、图标边界、主窗／面板、两种主题、额度阈值、十进制格式、采样生命周期和重建后的展示状态。
- Rust 覆盖核心回归及矩形定位、吸附、低工作区和跨屏边界；Swift 覆盖原生构建、现有交互回归及相同的小数格式用例。
- Windows 新增胶囊及桌面管理模块已在 macOS 上面向 Windows GNU 目标做编译和严格 Clippy 检查，使用真实 Tauri／Windows 库与隔离的核心类型替身。这是平台模块检查，不是完整 Windows 应用构建或运行。
- 完整 Windows 交叉构建受本机缺少 MinGW C 工具链限制。Windows CI 已增加正常执行的桌面 Rust 测试。

截图由隔离模拟数据生成：

- [主窗图标修正](../.local/ui-previews/overview-light.png)
- [浅色 credit 面板](../.local/ui-previews/credit-floating.html-light.png)
- [深色 credit 面板](../.local/ui-previews/credit-floating.html-dark.png)

## Windows 实机验收（本机未执行）

1. 在 Windows 10／11 检查胶囊浅深色、字体、提示、右键菜单、拖动和任务数；反复点击、取消拖动后仍能打开面板。
2. 在隔离测试账户启动应用，让面板渲染线程停止响应；确认原生胶囊仍可操作，并通过右键／托盘“重建面板（用于界面无响应）”恢复。验证采样未重复启动或结束。
3. 固定面板、选择筛选、折叠账户后刷新，核对状态恢复；刷新失败后可重试，主窗和胶囊仍可进入。
4. 检查 100%／150%／200% DPI、混合 DPI、多屏拔插和休眠恢复，胶囊和面板不得越出工作区。
5. 检查点击外部关闭、固定保持、菜单取消返回面板，以及输入框选择／复制／编辑；不得出现浏览器默认菜单或导航。

本轮没有发布安装包，也没有改动真实账户、服务器或定时任务配置。

2026-10-07：Linux 胶囊窗口与内容统一为 144×44；主窗最小尺寸统一为 640×440。计数默认关闭，可在通用设置的桌面显示中开启。阶段色来源为 `docs/ui-phase-colors.json`，运行 `python3 scripts/sync-phase-colors.py --check` 检查 CSS/Rust 与 macOS 系统色映射。Linux 胶囊聚焦后可用 Shift+F10 / ContextMenu 打开菜单；Windows 非激活胶囊不接收键盘焦点，使用系统托盘的键盘入口，原不可达 `WM_KEYDOWN` 分支已移除。

## 2026-10-07：Windows 关闭故障与 mac 视觉基准

### 窗口生命周期

- Windows 面板不再依靠 `Focused(false)` 自动收起：展开不激活窗口，胶囊消息线程仅在面板展开期间注册低级鼠标钩子；外部按下交给现有动作队列处理，原点击继续传递。胶囊、面板及其所属窗口不触发外部关闭，胶囊菜单期间暂停判定。
- 面板移除永久 `WS_EX_NOACTIVATE`，仍以 `SW_SHOWNOACTIVATE` 展开；用户点击面板时允许正常激活以进行键盘输入。
- 开关、切换、外部关闭及重建恢复均在主 UI 线程解析当前状态、完成原生操作后同步状态。显隐统一使用 `ShowWindow` 并检查 `IsWindowVisible`，避免原生显示与框架隐藏混用。
- 重建恢复核对 generation，并读取执行时的展开状态；重建期间的关闭请求会保留。Windows 收起时不再聚焦隐藏的 Web 胶囊。
- `desktop_action` 增加内部动作 `dismiss-panel`；其余命令及业务数据格式未改动。

### 视觉与提示

- 保留 Windows 侧栏、窗口按钮和原生胶囊；mac 原生展示代码不变。
- 正文 15px、次级文字 14px、分区 16px、主标题 22px 使用共同层级。Windows 中英文系统字体回退明确为 Segoe UI Variable／Segoe UI 与 Microsoft YaHei UI／Microsoft YaHei；代码字体使用 Cascadia Mono／Consolas。标题采用与正文相同的字体族，Canvas 坐标文字读取次级字号；原生胶囊优先使用 Microsoft YaHei UI。
- 按 mac 对照调整卡片底色与密度、筛选控件、设置分类分段控件、统计指标分段控件、账户标题与订阅标签、面板留白和图表高度；统一保存配置、账户标题、日期范围与每日明细等文案。共享 CSS 的调整同样作用于 Linux 桌面。
- 每日用量图只保留图下单一读数，不再写入 `title`；保留键盘选日、读数和精确明细。图例按 mac 折叠呈现。
- 通用提示在延迟开始前移除原生 `title`，使用 `data-hint` 持有文案；离开、滚动、失焦、元素被移除时取消提示，运行时更新文案也不会重新产生系统提示。

### 可重复验证

```sh
npm run test:ui --prefix apps/desktop
node scripts/test-windows-ui.cjs
node --test scripts/test-desktop.mjs scripts/test-multiselect.mjs
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked
sh scripts/check-native-capsule.sh
AIEYES_RENDER_ROOT="$PWD/.local/windows-ui-sync/macos" AIEYES_UI_STAMP=1791342000 sh scripts/test-ui-native-render.sh
python3 scripts/compare-ui.py
```

`preview-quota-core.py --fixture` 与 mac 原生渲染共用六类模拟数据。新增浏览器矩阵覆盖浅深色、所有设置分类、概览、服务器、面板、最小尺寸、1／1.5／2 DPR、单一图表读数及动态提示；通过 Chromium 的平台字体报告记录实际使用的字体。图库位于 `.local/windows-ui-sync/comparison.html`。

Windows CI 现在也运行浏览器集成，并在构建后运行：

```powershell
./scripts/test-windows-panel.ps1
```

该脚本以临时 `AIEYES_DATA_DIR` 启动真实应用及背景测试窗口，检查打开不抢焦点、面板交互可激活、外部关闭、关闭后的鼠标命中与桌面像素、重复切换、菜单取消、固定／取消固定以及重建后的关闭。截图与 `result.json` 上传到 `Windows-native-panel-results`。没有交互桌面时报告 `not-run`，不报告通过。

### 本轮验证边界

本机是 macOS。Web 回归、共享模拟数据截图、原生 mac 渲染、Rust 单元测试及 Windows 胶囊／显隐模块的交叉编译检查可以在本机运行；Windows 完整应用及 PowerShell 桌面回归需 Windows CI 执行。Windows CI 结果需在对应提交的运行记录中核对。浏览器 DPR 覆盖不能替代原生混合 DPI、多屏拔插或 Windows 中文字体实测；原生回归记录运行器实际 DPI，不修改运行器的系统缩放。
