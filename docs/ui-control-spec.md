# 三端控件与样式规格

本文件记录当前实现，配合 `ui-design-tokens.json`、`ui-phase-colors.json` 和 `ui-review-and-optimization-2026-10-26.md` 使用。macOS 使用 SwiftUI/AppKit；Windows/Linux 使用同一套 Web 组件。浏览器截图只验收 Web 内容，原生窗口、托盘、DPI、输入与辅助功能仍按 `manual-verification.md` 验收。

## 尺寸和文字

| 用途 | Windows/Linux 主窗口 | Web 紧凑面板 | macOS |
| --- | --- | --- | --- |
| 行内编辑、低频辅助、过滤按钮 | `control-sm`，24px | 24px | `.controlSize(.small)`，20pt |
| 分区工具栏、新增、表单、分段选项 | `control-md`，32px | 28px | 常规自定义按钮 24pt；原生输入控件保留系统尺寸 |
| 空状态主操作 | `control-lg`，40px | 40px | `.controlSize(.large)`，32pt |
| 正文／次要文字／说明 | 15／14／12px | 同左 | 共享令牌；原生菜单、分段控件仍有系统字号 |
| 最小点击高度 | `hit-target`，24px；复选框通过标签扩大命中区 | 同左 | 保留 AppKit/SwiftUI 原生命中行为 |

对话框保存／取消使用常规 32px，不随紧凑面板缩小。今日用量在主窗口同一行从左到右组合标题、Token、API 等价成本，分析入口靠右；窄内容区分两行，主数字 24px／semibold，紧凑面板 17px。

标题栏、导航、热力图单元格和桌宠按各自用途设尺寸，不计入内容按钮档位。Web 字号和圆角须来自令牌；允许 `inherit`、零、圆形 `50%` 和相对字号 `1em/100%`。胶囊及数据标记使用 `radius-pill/hairline/mark/chart`，普通控件、卡片、浮层分别使用 `radius-control/card/overlay`。平台窗口的 `window-radius` 与热力图的 `heat-cell-radius` 保留独立语义。

Windows/Linux Web 普通控件圆角为 4px，卡片和菜单容器为 12px；来自 `ui-design-tokens.json` 的 `web.dimensions`，不会改动 Swift 原生控件圆角。普通按钮用平底和细边框，工具栏及行内轻操作无边框。控件不再叠加亮边、内阴影和多重外阴影。菜单底色不透明；对话框填充与页面遮罩按最终选择维持原值。

## 操作与焦点

| 角色 | Web | macOS | 行为 |
| --- | --- | --- | --- |
| 默认主操作 | `button.primary` | `.aieyesPrimary` | 实心强调色；一个操作区一个主操作 |
| 强调的次要操作 | 普通按钮／`.tinted` | `.aieyesTinted` | Web 新增操作保持常规尺寸；macOS 保留原生染色样式 |
| 次要／轻操作 | 普通按钮／`.subtle` | `.aieyes`／无边框样式 | 保留清晰焦点环 |
| 危险操作 | `.danger` + `confirmDestructive` | destructive role + confirmation dialog | 确认文案描述删除或取消引用的后果 |
| 页面内返回 | `.back-link` | 原生返回控件 | 箭头和文字，位于内容起始位置 |

Web 选中项使用柔和强调色底，不再同时增加边框、阴影、渐变亮边或竖条；输入框与按钮保留单一清楚的键盘焦点轮廓。macOS 保留系统控件与焦点行为。

对话框页脚右对齐，macOS 的默认与取消操作使用相应键盘快捷键；Windows 的主操作通过布局顺序排在取消之前。输入对话框打开时聚焦首个可编辑字段。放弃未保存内容须有确认。

Web `details[data-menu]` 由 `menu.js` 管理，提供 menu/menuitem 语义、上下方向键、Home/End、首字母查找、Esc 返回触发器，以及 Tab、失焦和外部点击关闭。普通折叠区使用统一的前置右／下箭头。分段控件和常用 Agent 行只有一个 Tab 停靠点，方向键移动焦点，Enter/Space 选择。

Agent 页保留前四个常用入口，其余通过带搜索框的“更多 Agent”选择；窄窗口统一使用选择器。管理入口允许增减页面入口；移除暂停采集，保留数据源、机器选择、账户、凭据、历史与任务，重新添加恢复选择。

## 颜色、文案与时间

- 强调色文字用 `accent-text`，背景和进度条用 `accent`；实心主按钮用 `accent-ink`。系统默认强调色在 macOS 取 `controlAccentColor`，Web 使用预设默认色。
- 资源条保留类别色，达到 90% 时另加危险色数值与高负载图形。阶段色来自 `ui-phase-colors.json`，同时提供图形或文字。
- 状态使用稳定代码作为属性值，渲染时翻译标签。可变句子使用整句占位符，数量文案提供单数形式。
- 界面语言决定日期、时段和提示文字；显示时间的 12/24 小时规则来自系统偏好。macOS 从当前系统 `j` 模板读取规则，Windows 读取包含用户覆盖的 `GetLocaleInfoEx(LOCALE_ITIME)`；Linux 在 GNOME 系桌面读取 `clock-format`，其他桌面读取原生 `LC_TIME` 格式，无法读取时回退到 Intl。唤醒任务内部解析仍使用固定 24 小时表示。
- WebKit 的截图与布局检查覆盖 Linux 样式、浅深色、中英文、默认及最小尺寸；真实 GTK/WebKitGTK 验证与各桌面会话人工验收分别记录在 `manual-verification.md`。

## CSS 所有权与检查

全局层顺序为 `tokens, base, layout, components, platform, overrides`。基础布局先于组件，以便收起侧栏、面板和窄窗口的组件规则覆盖基础网格。

| 文件 | 所有权 |
| --- | --- |
| `tokens.css` | 字体、主题、生成的共享令牌与阶段色；生成区独立成块并保持同层级联顺序 |
| `base.css` | 元素重置、SVG、隐藏与基础文本行为 |
| `layout.css` | 主窗口网格、内容与侧栏基础布局 |
| `charts.css` | 趋势、热力图、图例与每日明细 |
| `controls.css` / `select.css` | 按钮、字段、选择器、确认、菜单和分段控件 |
| `style.css` / `workspace.css` | 业务卡片、导航、设置和工作区组合 |
| `panel.css` / `floating.css` | 紧凑面板与 Web 桌宠 |
| `windows.css` | Windows 控件与动效差异 |

平台选择器进入 `platform` 层；减少动效、减少透明度和强制颜色进入 `overrides` 层。同文件、同上下文的选择器与声明不得重复。按元素动态设置的 `--weeks` 和 `--provider-mask` 是有记录的变量入口；其他没有全局声明的变量必须有回退值。

`npm --prefix apps/desktop run lint:css` 检查分层、重复、字号／圆角令牌和未定义变量；`scripts/ci-checks.sh` 同时运行规则反例检查。首次运行共享检查前先执行 `npm ci --prefix apps/desktop --ignore-scripts`。

`npm --prefix apps/desktop run test:webkit` 输出 `.build/ui-webkit/manifest.json` 和 64 张截图。CI 安装 Chromium 与 WebKit 后运行两个通道。截图不代替 WebKitGTK、DWM 或 SwiftUI 的原生验收。
