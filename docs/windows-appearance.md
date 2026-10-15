# Windows 外观对齐（2026-10-14）

Windows 使用本地鸿蒙黑体，内容尺寸以当前原生 macOS 界面为参照，保留独立的左侧导航。概览、服务器、全部设置、弹窗和展开的悬浮面板共用控件与图表样式；Linux 使用同一套内容规则。主窗口与展开的悬浮面板分别请求原生桌面材质；网页仅在对应窗口的原生调用成功后启用半透明表面。

## 字体与尺寸

| 项目 | Windows 逻辑像素 |
| --- | --- |
| 主窗口默认客户区 | 1294 × 840（1080 × 800 内容 + 214 侧栏 + 40 标题栏） |
| 主窗口最小尺寸 | 640 × 440，受显示器工作区约束 |
| 侧栏展开 / 收起 | 214 / 72，开关不调整窗口尺寸或字号 |
| 展开的悬浮面板 | 450 × 720，受工作区约束 |
| 正文 / 次级文字 / 说明文字 | 15 / 14 / 12 |
| 主标题 / 小节标题 | 22 / 16 |
| 面板概览 / 限额标题 | 17 / 20 |
| 主要数值 | 24 |
| 常规按钮 / 紧凑选择器 / 输入框 | 24 / 24 / 26 |
| 开关 / 滑块 | 42 × 20 / 16 × 16 |
| 悬浮面板内边距 / 统计卡片间距 | 12 / 10 |
| 悬浮面板顶部图标框 / 底部图标按钮 | 34 / 28 |
| 悬浮面板分段选择器可见宽度 | 120；原生布局容器宽 264，不是控件宽度 |
| 资源与限额进度条 | 4，实色填充，无凹槽阴影 |
| 资源圆环 | 48 / 72，描边 5 |
| 窗口 / 卡片 / 控件 / 弹出层圆角 | 8 / 16 / 6 / 12 |
| 账号切换按钮圆角 | 8 |

字体文件来自 OpenHarmony 的 `resources` 仓库，固定版本及 SHA-256 位于 [sources.json](../apps/desktop/web/fonts/sources.json)。Regular、Medium、Bold 三个原始 TTF 共约 23.5 MiB；没有子集化或修改。WebView 使用本地 `@font-face`，原生桌宠文字使用 GDI+ 私有字体集合，不需要用户安装字体。代码、路径仍使用等宽字体。

在「设置 → 常规 → 外观」展示字体署名和完整许可。打包前运行 `check-bundled-fonts.cjs` 检查文件尺寸、哈希及许可。正文和图表共用字体，字体加载完成后重绘图表。模型分布及每日模型明细的复制按钮已移除，文字仍可选中复制。

## 共享样式维护

[ui-design-tokens.json](ui-design-tokens.json) 是字号、间距、控件尺寸、圆角、颜色及图表几何的唯一生成源。运行：

```sh
python3 scripts/sync-ui-design-tokens.py
python3 scripts/sync-ui-design-tokens.py --check
```

脚本生成 [DesignTokens.swift](../apps/macos/Sources/Aieyes/DesignTokens.swift) 和 [tokens.css](../apps/desktop/web/tokens.css) 中的标记区域；`scripts/ci-checks.sh` 检查是否同步。不要直接修改生成区域。Swift 保留现有系统字体、字重和材质，Web 的常规、强调、粗体分别使用随包的 400、500、700 字体文件。原生已有常量只做等值替换。

Web 专用的控件表面、边缘阴影与设置卡片降级色在 `tokens.css` 的非生成区域维护；这些变量不改变 macOS 原生界面或共享尺寸。

[controls.css](../apps/desktop/web/controls.css) 管理共用按钮、输入框、开关、分段选择器、表单和弹出层；[panel.css](../apps/desktop/web/panel.css) 管理悬浮面板布局；[style.css](../apps/desktop/web/style.css) 管理页面及保留的 Windows 侧栏。内容断点以可用内容宽度为准，避免侧栏收起或 Linux 窗口尺寸变化时产生额外空列。悬浮面板保留固定头部、底部和单一内容滚动区。

弹窗采用各自的原生参考宽度：目录 570、主机 620、价格 520、API 530、账号选择与排序 440、估值管理 660、数据源自动估值 600、确认 460。所有宽度受当前客户区限制，内容可滚动，底部按钮保持可达。`showEditor` 的 `kind` 和 `quotaDialog` 的 `layout` 仅指定外观，不改变操作逻辑。

模型分布使用带间隙的 SVG 圆环；零数据保留空环，单模型闭合绘制，多模型保留中心空白。资源圆环使用固定 5 像素描边；趋势图、图例和热力图读取共享尺寸与现有主题配色。趋势图日期根据实际字体宽度限制在画布内，窄窗口或长日期范围下减少重叠刻度。图标统一由 [icons.js](../apps/desktop/web/icons.js) 提供，侧栏和窗口控制仍保留原来的设计。

主窗口普通材质与悬浮面板的表面并不相同；Web 不透明模式分别使用原生截图的中性底色。原生材质成功时仍使用各窗口的透明参数，减少透明度与高对比度规则优先。

## 控件与设置卡片（2026-10-15）

设置分组采用悬浮面板卡片的轻薄表面、16 像素圆角与细边框。在主窗口 Acrylic / Blur 成功时直接复用面板卡片的 `--panel` 底色；不透明模式使用浅灰 `#F1F2F4`、炭灰 `#303238`。设置侧栏透明，嵌套卡片通过分隔线和留白分组，避免重复叠加底色。窗口原生材质及其检测逻辑沿用现有实现。

复选框统一为 16 × 16，开关保持 42 × 20，标签间距为 8 像素并对齐首行。复选框保留原生输入、键盘、禁用及表单语义；高对比度模式恢复系统复选框外观。按钮、输入框和选择器共用轻边框、轻阴影及悬停、按下、焦点和禁用反馈。

外观页的四个下拉框共用 220 像素列宽；可用内容宽度不足时，标签与选择器上下排列。普通选择器保持 26 像素、紧凑筛选器保持 24 像素，原生单选菜单继续负责展开交互。多选按钮与单选框共用箭头，长标题省略时保留完整提示。多选弹层显示后按实际高度定位，短列表贴近按钮，窗口缩放时重新限制在客户区内。

`test-windows-ui.cjs` 增加 [control-appearance-checks.cjs](../scripts/control-appearance-checks.cjs) 验收：Windows/Linux 明暗设置、透明与不透明表面、最小窗口、复选框与首行文字几何、100% / 125% / 150% / 200% 缩放、原生表单值、键盘选择、多选菜单缩放及高对比度。控件截图输出到指定 `AIEYES_UI_OUT` 目录下的 `controls/`。这些是 Chromium 合成数据检查，不替代 Windows 原生材质和系统 DPI 实测。

## 原生材质

| 系统与设置 | 请求的材质 |
| --- | --- |
| Windows 11 build ≥ 22621 | DWM Desktop Acrylic（`DWMSBT_TRANSIENTWINDOW`） |
| Windows 10 build ≥ 17763、较早的 Windows 11 | 原生 Blur，带浅 / 深色底色 |
| 不支持、原生调用失败、关闭系统透明效果或高对比度 | 不透明 |

`desktop_info` 和 `desktop:status` 返回 `materials: {main, floating}`；原来的 `material` 保留为主窗口材质别名。悬浮面板不会根据主窗口的成功状态自行开启透明。主题和系统设置变化时重新计算两者；面板重建会清除旧 HWND 的材质缓存。

浅色 / 深色主窗口网页底色 alpha 分别为 .18 / .28，面板为 .12 / .20；卡片保留轻微底色，菜单和输入控件使用较强底色。实际桌面透出程度由系统材质与网页底色共同决定。减少透明度和强制颜色模式在网页侧同样恢复不透明底色。

Windows 11 使用系统圆角；Windows 10 使用按 DPI 缩放的窗口区域裁剪。最大化恢复方角。原生桌宠保留独立的点击、拖动与焦点逻辑。

## 验证与复现

```sh
node scripts/check-bundled-fonts.cjs
node --test scripts/test-desktop.mjs scripts/test-multiselect.mjs
AIEYES_UI_OUT=.local/windows-appearance/after node scripts/test-windows-ui.cjs
node scripts/test-agent-settings-ui.cjs
node scripts/test-theme-sync-ui.cjs
pwsh -File scripts/test-windows-panel.ps1
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --features custom-protocol
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --locked --features custom-protocol -- -D warnings
```

浏览器检查覆盖六类数据场景、浅深色、设置页、640 × 440、侧栏开关、100% / 125% / 150% / 200% 缩放、实际渲染字体、两窗口独立材质、减少透明度、高对比度及本地字体许可。它检查 CSS 的透明度与布局，不代表真实 DWM 毛玻璃验收。

`test-windows-ui.cjs` 同时调用 [ui-alignment-checks.cjs](../scripts/ui-alignment-checks.cjs)：验证 Windows/Linux 在 640–1800 宽度下的卡片布局、缓存列、八类弹窗、SVG 圆环空心区域，以及 450 × 720 / 360 × 480 面板中的控件、菜单和固定头尾。另以实际 Canvas 字形边界验证 0–365 天、Token / USD 两种模式的趋势日期，确保标签完整且不重叠。截图前等待实际字体加载。完整实施与本轮验证结果见 [2026-10-14 对齐记录](ui-alignment-2026-10-14.md)。

此前 Windows 外观工作已在 macOS 宿主验证桌面 / 多选 Node 测试、Tauri Rust 测试、Clippy 和 Windows 原生模块交叉类型检查。当前交互回归使用 Agents 页面及 `settings.patch`，服务器移除后检查焦点返回“添加主机”。本轮样式改动的验证单独记录在上述实施记录中。

Windows 实机尚未运行，本轮未生成 Windows 安装包。Windows 10 / 11 材质、混合 DPI、原生桌宠字体及安装流程仍需实机验收。已有交互测试增加了可选的桌面像素验证，在启用透明效果的交互桌面中运行：

```powershell
./scripts/build-desktop.ps1 release
./scripts/test-windows-panel.ps1 -VerifyMaterials
```

该选项交替显示两种纯色背景，分别采集主窗口与重建后的悬浮面板边缘像素，验证两者随桌面背景变化；截图和结果写入 `.local/windows-ui-sync/native/`。默认测试继续覆盖焦点保持、外部点击关闭、固定、重建及关闭后无残影。
