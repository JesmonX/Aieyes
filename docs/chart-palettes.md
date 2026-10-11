# 图表预置配色

在「设置 → 通用 → 外观」选择「图表配色」和「图表对比度」。选择自动保存，主窗口和菜单栏／悬浮面板同步；每套配色均有浅色高对比、浅色低对比、深色高对比、深色低对比四个版本。设置同时展示明暗背景的柱形预览。

| 预置 | 颜色数 | 风格与来源 |
| --- | --- | --- |
| 经典 | 8 | Aieyes 原有经典色列 |
| Tableau · 经典分析 | 10 | [Tableau 10，Vega 收录](https://vega.github.io/vega/docs/schemes/#tableau10)，均衡的分类分析配色 |
| Paul Tol · 明快 | 7 | [Paul Tol Bright](https://sronpersonalpages.nl/~pault/)，参考其考虑色觉差异的分类配色 |
| ColorBrewer · 柔彩 | 8 | [ColorBrewer Set2](https://colorbrewer2.org/?n=8&scheme=Set2&type=qualitative)，柔和的分类色系 |
| Nord · 北欧 | 8 | [Nord Frost 与 Aurora](https://www.nordtheme.com/docs/colors-and-palettes/)，偏冷的蓝绿搭配暖色 |

高对比版本保留原色，在必要时调整亮度，针对浅色 `#e1e8f6`、深色 `#2e3a52` 的保守图表背景达到至少 3:1 的柱体／背景对比度（生成目标 3.2:1，留出舍入余量）。低对比版本在白色和 `#29292e` 背景上约为 2.2:1，适合较大的柱体和柔和展示。这里的高低对比指柱体与背景，不表示任意两个类别之间的对比度；图例、精确读数、隐藏模型功能仍可辅助区分。

明暗与对比度版本调整过原始色值，因此不声称保留原方案完整的色觉无障碍性质。分类色用来区分模型，数值大小由柱高表示。超过预置颜色数后复用颜色，需结合模型名及明细；年度热力图继续使用强调色表达数值强弱。

旧配置兼容：缺失 `appearance.chartPalette` / `appearance.chartContrast` 时使用 `classic` / `high`。经典预置使用原有经典色列代替此前按模型名生成的任意色相。模型按名称哈希选择颜色槽，在色系未用满时避开已分配槽；筛选、隐藏模型、切换主题或对比度不改变已分配槽。

色值单源为 `docs/chart-palettes.json`。运行 `python3 scripts/sync-chart-palettes.py` 生成 Web 和 Swift 色表；CI 的 `--check` 防止副本漂移。验证入口：`node scripts/test-theme-sync-ui.cjs`、`cargo test --test chart_appearance --locked` 和 `scripts/verify-palette.swift`（由 `scripts/test-native.sh` 调用）。
