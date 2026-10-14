# Windows / Linux 内容样式与 macOS 对齐

本轮以开始实施时的 macOS 工作区界面为基准，覆盖概览、服务器、设置、编辑器、菜单及展开的悬浮面板。Windows 214 / 72 侧栏、窗口外壳和随包 HarmonyOS 字体保留；Linux 复用内容样式。已有业务、RPC、持久化与独立桌宠动画保持原有实现。

状态：实施、本机自动化检查与视觉复核已完成。Windows/Linux 实机待验范围见文末。

## 实施结果

- 新增 `docs/ui-design-tokens.json` 与同步脚本，生成 Swift / CSS 常量并接入 CI。macOS 中的原值等价替换为引用。
- 控件统一为按钮 24、选择器 24、输入框 26、开关 42 × 20；卡片圆角 16，普通控件 6，弹出层 12。
- 面板保持 450 × 720，内容边距 12，两列统计卡片间距 10，统计卡片高 98。头部可见分段控件宽 120，底部按钮 28，间距 8。头尾固定，内容单独滚动。
- 卡片、账号徽标、进度条、资源圆环、模型圆环、图例及热力图使用原生参考尺寸。主窗口统计卡片铺满两列，缓存明细按可用宽度排列；清除旧的三列与 Linux 专用内容覆盖。
- 密集趋势图根据实际字体宽度约束日期标签位置，并在空间不足时减少刻度，避免最右侧日期被画布裁切或相邻日期重叠。用量数据和日期选择保持原有行为。
- 各类弹窗使用原生参考宽度，并限制在当前客户区内。顶部、底部和内容滚动区域采用共用样式；保留关闭、取消、错误恢复及焦点行为。
- 语义图标统一到 `icons.js`，由应用初始化静态控件，图标模块本身不访问 DOM。截图夹具向浏览器提供与原生 RPC 相同的账号状态、网络状态及扫描结果。

## 验证

| 检查 | 结果 |
| --- | --- |
| `npm --prefix apps/desktop run test:ui` | 通过；全部交互回归和双窗口主题同步 |
| `node scripts/test-windows-ui.cjs` | 通过；六场景、浅深色、设置、字体、DPI、材质降级、Windows/Linux 布局及 100 组趋势标签边界检查 |
| `sh scripts/ci-checks.sh` | 通过；Python 检查、生成文件同步、JS 语法、31 项 Node 测试及重置时间显示 |
| `swift build --package-path apps/macos --scratch-path .build/swift` | 通过 |
| `sh scripts/test-native.sh` | 通过；模型、配置、主题、账号、唤醒、服务器和桌宠等原生回归 |
| `sh scripts/test-ui-native-render.sh` | 通过；六场景 650 张截图、原生对比度、设置页和透明度验证 |

原生截图中 604 / 650 张逐像素相同，其余截图包含加载进度、倒计时、动画相位、系统滚动条与布局结算时机的差异，不能作为完全确定的像素基线。单独把所有新增 `DesignTokens` 引用展开回原值后，与原生基准源码一致；因此本轮没有改动原生字号、尺寸、颜色值或渲染逻辑。

接续原会话的收尾复核发现并修复了长日期范围下的趋势图标签裁切。新增回归直接记录实际 Canvas 字形边界，覆盖 Windows/Linux、640 / 900 / 1040 / 1294 / 1800 窗口宽度、0 / 1 / 7 / 100 / 365 天，以及 Token / USD 两种模式，共 100 组；修复前复现失败，修复后全部通过。补充的 16 组平台、数据、主题与指标组合及最小窗口长模型展开检查通过，图表截图位于 `after/final-review/`。

最终代码重新通过完整 Web 回归、Windows/Linux 布局套件与共享 CI 检查，日志为 `resumed-web-regressions.log`、`resumed-windows-ui.log`、`resumed-ci-checks.log`。原生构建、回归和截图沿用原会话的通过记录；本次再次验证 14 个 Swift 文件的常量展开等价性，结果保存为 `final-native-equivalence.json`。

本机基准保存在 `.local/ui-alignment-20261014/baseline/source/`，保留了实施前 183 个工作区源文件；验收截图和日志在同目录的 `after/` 与 `*.log`。图形对照页包含 108 组原生、Windows 调整前、Windows 调整后截图，支持按场景、主题、页面筛选和原始尺寸查看：

```sh
python3 scripts/compare-ui.py \
  --native-root .local/ui-alignment-20261014/baseline/native \
  --web-root .local/ui-alignment-20261014/after/windows \
  --before-web-root .local/ui-alignment-20261014/baseline/windows \
  --output .local/ui-alignment-20261014/comparison.html
```

截图为合成数据，不读取个人账号或凭据。macOS 截图验证在当前宿主原生运行；Windows/Linux 布局通过 Chromium 平台夹具验证，不能替代实际 Windows WebView2、Linux WebKitGTK、Windows 10 / 11 DWM 或混合 DPI 实机验收。本轮没有生成安装包或更改发布配置。

后续共同调整样式时，先修改 JSON，运行同步脚本，再按受影响界面执行浏览器和原生检查。平台字体、原生窗口材质、侧栏及桌宠仍由各自实现负责，避免从内容样式隐式继承新的尺寸。
