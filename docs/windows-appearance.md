# Windows 外观对齐（2026-10-09）

Windows 使用本地鸿蒙黑体，内容尺寸以原生 macOS 界面为参照，左侧导航单独占宽。主窗口与展开的悬浮面板分别请求原生桌面材质；网页仅在对应窗口的原生调用成功后启用半透明表面。

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
| 常规按钮 / 紧凑选择器 / 输入框 | 28 / 26 / 32 |
| 资源与限额进度条 | 4，实色填充，无凹槽阴影 |
| 资源圆环 | 48 / 72，描边 5 |
| 窗口 / 卡片 / 控件圆角 | 8 / 16 / 9 |

字体文件来自 OpenHarmony 的 `resources` 仓库，固定版本及 SHA-256 位于 [sources.json](../apps/desktop/web/fonts/sources.json)。Regular、Medium、Bold 三个原始 TTF 共约 23.5 MiB；没有子集化或修改。WebView 使用本地 `@font-face`，原生悬浮胶囊使用 GDI+ 私有字体集合，不需要用户安装字体。代码、路径仍使用等宽字体。

在「设置 → 通用 → 外观」展示字体署名和完整许可。打包前运行 `check-bundled-fonts.cjs` 检查文件尺寸、哈希及许可。正文和图表共用字体，字体加载完成后重绘图表。模型分布及每日模型明细的复制按钮已移除，文字仍可选中复制。

## 原生材质

| 系统与设置 | 请求的材质 |
| --- | --- |
| Windows 11 build ≥ 22621 | DWM Desktop Acrylic（`DWMSBT_TRANSIENTWINDOW`） |
| Windows 10 build ≥ 17763、较早的 Windows 11 | 原生 Blur，带浅 / 深色底色 |
| 不支持、原生调用失败、关闭系统透明效果或高对比度 | 不透明 |

`desktop_info` 和 `desktop:status` 返回 `materials: {main, floating}`；原来的 `material` 保留为主窗口材质别名。悬浮面板不会根据主窗口的成功状态自行开启透明。主题和系统设置变化时重新计算两者；面板重建会清除旧 HWND 的材质缓存。

浅色 / 深色主窗口网页底色 alpha 分别为 .18 / .28，面板为 .12 / .20；卡片保留轻微底色，菜单和输入控件使用较强底色。实际桌面透出程度由系统材质与网页底色共同决定。减少透明度和强制颜色模式在网页侧同样恢复不透明底色。

Windows 11 使用系统圆角；Windows 10 使用按 DPI 缩放的窗口区域裁剪。最大化恢复方角。原生胶囊继续保留独立的点击、拖动与焦点逻辑。

## 验证与复现

```sh
node scripts/check-bundled-fonts.cjs
node --test scripts/test-desktop.mjs scripts/test-multiselect.mjs
AIEYES_UI_OUT=.local/windows-appearance/after node scripts/test-windows-ui.cjs
node scripts/test-agent-settings-ui.cjs
node scripts/test-theme-sync-ui.cjs
sh scripts/check-native-capsule.sh
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --features custom-protocol
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --locked --features custom-protocol -- -D warnings
```

浏览器检查覆盖六类数据场景、浅深色、设置页、640 × 440、侧栏开关、100% / 125% / 150% / 200% 缩放、实际渲染字体、两窗口独立材质、减少透明度、高对比度及本地字体许可。它检查 CSS 的透明度与布局，不代表真实 DWM 毛玻璃验收。

本轮 macOS 宿主上的 21 项桌面 / 多选 Node 测试、21 项 Tauri Rust 测试、Clippy、Windows 原生模块交叉类型检查、当前 Agents / 账户与主题同步检查通过。发布准备时更新了 UI 交互回归测试：启用入口验证当前 Agents 页面，服务器保存使用新版 `settings.patch`，移除最后一个服务器后验证焦点返回“添加主机”，不再调用会主动移走焦点的旧草稿保存辅助函数。服务器保存测试同样补齐了 `settings.patch` 的测试返回值。

Windows 实机尚未运行，本轮未生成 Windows 安装包。Windows 10 / 11 材质、混合 DPI、原生胶囊字体及安装流程仍需实机验收。已有交互测试增加了可选的桌面像素验证，在启用透明效果的交互桌面中运行：

```powershell
./scripts/build-desktop.ps1 release
./scripts/test-windows-panel.ps1 -VerifyMaterials
```

该选项交替显示两种纯色背景，分别采集主窗口与重建后的悬浮面板边缘像素，验证两者随桌面背景变化；截图和结果写入 `.local/windows-ui-sync/native/`。默认测试继续覆盖焦点保持、外部点击关闭、固定、重建及关闭后无残影。
