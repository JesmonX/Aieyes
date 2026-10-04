# Windows 界面刷新

2026-10-04 对 Tauri 桌面界面做了一次大范围视觉与动效更新，目标是半透明、圆润的桌面观感。改动全部位于 `apps/desktop/web/`，Windows 与 Linux 共用同一套资源，Rust 侧未改动。

## 设计语言

- **一层窗口面板决定轮廓。** `body::before` 绘制整块窗口表面：圆角 `--window-radius`（默认 14px）、1px 内描边、柔和的角部高光与浅色渐变。Windows 使用无边框透明窗口，因此面板之外就是 Mica 或桌面本身；Linux 保留原生窗口装饰，面板改为不透明底色。
- **Mica 透出而非叠加。** 材质为 `mica` 时面板改用半透明着色（`--window-tint`），由 Windows 11 的 Mica 提供模糊；其它情况回退为 `--window-base` 实色渐变，避免在 Win10 上把未滤镜的桌面直接透出来。
- **三级表面。** `--panel`（卡片）、`--surface-strong`（控件、弹窗、气泡）、`--surface-soft`／`--surface-sunken`（悬停、分区底色）。卡片、统计块、弹窗、多选面板共用同一套圆角与投影，圆角半径为 7／10／13／17／21／26px 的等比阶梯。
- **字体只用系统字体。** 优先 `Segoe UI Variable Text`／`Display`，依次回退 `Segoe UI`、`system-ui`、`SF Pro`、`PingFang SC`、`Microsoft YaHei UI`。CSP 未开放 `font-src`，且应用需要离线可用，因此不引入联网字体；标题使用字重与负字距拉开层次，数值统一 `tabular-nums`。

## 动效

- 只对 `transform` 与 `opacity` 做过渡，避免滚动和窗口缩放时重排。
- `#content` 的直接子元素带 320ms 的错峰入场（`rise`），页面切换与数据刷新都会重放；订阅控件、进度条、圆环和热力图单元格用宽度／`stroke-dasharray`／背景色过渡，而不是逐帧脚本。
- 入场动画一律使用 `animation-fill-mode: backwards`，动画结束后不再持有属性值，因此卡片悬停的位移不会被动画覆盖。
- 悬浮球改为接近不透明的球面渐变（88px 的透明窗口无法获得真实模糊），活跃时眼睛与光晕同步呼吸。
- `prefers-reduced-motion: reduce` 下关闭全部过渡与动画。

## 提示气泡

`tooltip.js` 以事件委托读取控件上已有的 `title`，悬停约 520ms 或键盘聚焦后显示统一气泡，并暂时移除原生 `title` 以免两套提示重叠。气泡用 `position: fixed` 定位并限制在视口内，因此不会撑宽页面，也不会被滚动容器裁剪；鼠标按下、滚动、失焦或 Esc 立即关闭。`app.js` 里为新增的按钮补充了说明文字。

## 无障碍

- 浅色与深色各一套令牌，`color-scheme` 同步。
- `forced-colors: active`：表面改为 `Canvas`，圆环与进度条改用 `Highlight`，窗口面板关闭。
- `prefers-reduced-transparency: reduce`：面板与卡片改为不透明，关闭弹窗背板、多选面板和气泡的 `backdrop-filter`。
- 焦点环统一为 2px 强调色；输入框改为强调色描边加 3px 柔光环。

## 已知限制

- 窗口圆角由网页面板绘制。Windows 11 下圆角之外是 Mica 背景，观感接近但不等于 DWM 的窗口圆角；若要与系统完全一致，需要原生侧调用 DWM 圆角偏好，本轮未做。
- 未改动 Rust 侧窗口配置，因此未重新验证 Windows 原生构建、真实 Mica 效果、混合 DPI 与多显示器行为。
- 悬浮球的动效与配色仅在浏览器中按 88×88 视口检查，未在真实桌面点击、拖动或右键。

## 验证

```sh
node --test scripts/test-desktop.mjs scripts/test-multiselect.mjs
cd apps/desktop && npx playwright install chromium && npm run test:ui
```

本轮结果：9 项桌面与多选单元测试通过；浏览器集成检查通过，覆盖浅深色概览、服务器、价格、通用、数据源编辑、最小尺寸 840×600、强制高对比、设备缩放 1.5／2、多选面板与弹窗几何，截图输出到 `.local/ui-previews/`；`scripts/test-release-publish.mjs` 3 项与 `scripts/test-collectors.py` 4 项通过。另用临时脚本单独核对了提示气泡的定位与悬浮球两种相位的渲染。

发布前使用 `python3.12` 运行版本一致性检查与 `scripts/test-release.py`（3 项通过），`cargo test --workspace --locked` 共 36 项通过；桌面 JavaScript、发布上传和 Python 采集器检查、浏览器集成检查再次通过。系统默认 Python 3.8 不支持 `tomllib`，发布脚本需使用 Python 3.11+。
