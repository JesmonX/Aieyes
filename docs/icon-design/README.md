# Aieyes 图标

保留眼睛轮廓，加入代表用量的仪表环、代表统计的三根柱形和绿色状态点。深蓝底与冰蓝主体用于突出监控工具的定位；绿色圆点是品牌图形，不表示实时状态。

`previous-icon.png` 为改版前图标；`aieyes-512.png` 为大图预览，其余尺寸用于检查缩小后的效果。

图形源码：`scripts/render-icon.swift`。在 macOS 上执行 `python3 scripts/generate-icons.py`，生成 PNG、包含 16–256 像素的 ICO、包含 16–1024 像素的 ICNS。无需第三方 Python 依赖，需要 Swift 和 AppKit。

资源用于 macOS 应用图标及 Tauri 打包图标，并生成应用内 `Brand.png` / `brand.png`、菜单栏 `BrandTemplate.png` 及托盘 `brand.rgba`。所有入口复用相同眼睛、仪表环和柱形；菜单栏为无底色模板，托盘实时状态在右上角独立叠加。生成脚本不修改已构建应用；正常构建时会复制最新资源。
