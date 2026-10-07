# Aieyes 图标

保留眼睛轮廓，加入代表用量的仪表环、代表统计的三根柱形和绿色状态点。深蓝底与冰蓝主体用于突出监控工具的定位；绿色圆点是品牌图形，不表示实时状态。

`previous-icon.png` 为改版前图标；`aieyes-512.png` 为大图预览，其余尺寸用于检查缩小后的效果。

图形源码：`scripts/render-icon.swift`。在 macOS 上执行 `python3 scripts/generate-icons.py`，生成 PNG、包含 16–256 像素的 ICO、包含 16–1024 像素的 ICNS。无需第三方 Python 依赖，需要 Swift 和 AppKit。

同一份图形源码生成三种资源：安装图标保留深蓝圆角底；应用内 `Brand.png` / `brand.png` 和托盘、Windows 原生胶囊的 `brand.rgba` 只绘制透明底的彩色标志；菜单栏 `BrandTemplate.png` 为透明模板。主面板、菜单栏面板、悬浮面板及悬浮胶囊复用透明标志，不另加底色或阴影。彩色标志使用更清晰的蓝色轮廓，以适应浅深色界面；所有入口保留相同眼睛、仪表环、柱形及状态点的构图，模板省略状态点，实时状态独立叠加。

`brand-16.png` 至 `brand-256.png` 是透明标志预览。生成脚本不修改已构建应用；正常构建时会复制最新资源。
