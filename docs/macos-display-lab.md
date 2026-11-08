# macOS 显示实验室

这个开发版把几种桌面显示放在「设置 → 显示实验室」中，支持同时开启、切换演示场景和恢复实时数据。应用名称与进程为 **Aieyes Lab / AieyesLab**，可与正式版并行运行。

| 显示方式 | 设计与用途 | 更新方式 |
| --- | --- | --- |
| 程序坞 · 灵动双眼 | 双眼与状态灯响应工作、思考、工具执行、等待、完成和失败；等待与失败数显示角标 | 状态变化立即更新，动效最多 8 fps |
| 程序坞 · 额度仪表 | 两圈显示首个有额度窗口的账户；优先取 5h / 7d，低额度变色 | 随账户额度更新 |
| 程序坞 · 桌宠图标 | 使用当前桌宠的图集及各状态帧时长 | 本机独立动画，不要求桌面桌宠同时开启 |
| 桌面实时卡片 | 小、中、大三种尺寸，显示今日全部用量、额度和会话 | 跟随应用的数据发布 |
| 轻量状态条 | 320 × 54 pt 的窄条，适合屏幕边缘 | 跟随应用的数据发布 |
| macOS 系统小组件 | WidgetKit 原生扩展，支持小、中、大号；可放桌面或通知中心，点击进入总览 | 应用发布快照并请求刷新，实际时间由 macOS 决定 |
| 菜单栏、独立桌宠、固定面板 | 沿用原有显示，可在「桌宠与会话」中组合使用 | 沿用现有监测与刷新设置 |

Dock 使用 [NSDockTile 的自定义内容和 display()](https://developer.apple.com/documentation/appkit/nsdocktile)。开启后，关闭全部窗口仍保留图标；关闭此显示后恢复原有的窗口驱动策略。动效开关、系统减少动态效果、低电量模式、屏幕休眠与会话锁定都会停止帧定时器。

WidgetKit 是快照界面，系统有刷新预算，不适合作为秒级会话指示器；实时观察可搭配 Dock 或桌面卡片。实现依据 [Apple 的小组件更新说明](https://developer.apple.com/documentation/widgetkit/keeping-a-widget-up-to-date)。

## 构建与启动

```sh
# 默认使用优化构建，便于评价实际运行开销
AIEYES_OFFLINE=1 sh scripts/build-macos-display-lab.sh
open "dist/Aieyes Lab.app"

# 开发调试构建
AIEYES_OFFLINE=1 sh scripts/build-macos-display-lab.sh debug
```

本次测评包也已安装到 `~/Applications/Aieyes Lab.app`。运行后按 `⌘4` 可回到显示实验室。

首次启动打开显示实验室，启用双眼 Dock 和中号桌面卡片，并进入明确标注的「进行中」演示场景。后续启动回到实时数据，显示开关、风格、尺寸和位置保留。也可指定场景：

```sh
open "dist/Aieyes Lab.app" --args --show-display-lab --display-demo needsInput
```

实验数据位于 `.local/display-lab/app/`。首次构建创建空的账户、机器和来源配置，不复制正式版凭据或定时任务。要试用真实数据，可在开发版的「接入与账户」中选择来源。所有新显示使用今日全量摘要，不跟随用量分析页的筛选。

开发版保持独立 Bundle ID `app.aieyes.displaylab`，不会安装正式版自动更新。普通 `scripts/build-macos.sh` 的输出仍为 `dist/Aieyes.app`。

## 建议的测评顺序

1. **切换场景。** 对比进行中、思考、执行工具、等待输入、完成、失败、额度不足与状态不可用。场景只修改显示快照，不写入用量、账户、会话或通知状态。「返回实时」立即恢复真实摘要。
2. **比较 Dock。** 依次选择双眼、额度仪表与桌宠，关闭主窗口观察图标是否保持；右键图标可返回显示实验室。关闭动效后，图标继续响应状态但不播放动画。
3. **摆放卡片。** 尝试不同尺寸和状态条。单击进入总览，拖动移动，右键隐藏或锁定。关闭「悬浮置顶」后留在桌面层；「重置卡片位置」可找回移出屏幕的卡片。
4. **添加原生小组件。** 桌面右键 → 编辑小组件 → 搜索 **Aieyes Lab** → 选择尺寸后添加。也可在通知中心编辑小组件。应用内「同步小组件」会更新快照，并显示系统报告的已添加数量。
5. **检查恢复。** 退出并重新打开开发版，确认布局保留、演示已退出。只想保留原有入口时，点击「收起新增显示」；系统小组件通过 macOS 自己的移除菜单管理。

比较时可重点关注：小图标辨识度、等待提示是否清楚、卡片是否挡住工作内容、置顶与桌面层哪个更合适，以及系统小组件的快照频率是否满足需求。

## 数据与原生扩展

小组件快照仅包含汇总数、状态计数、服务商名称、额度和时间，不包含账户 ID、邮箱、会话文本、路径或凭据。应用原子写入快照；损坏或不兼容的数据回退为空状态。

本机实验包没有 Apple 开发团队证书，扩展使用 ad-hoc 签名和仅限 `~/Library/Application Support/Aieyes Display Lab/` 的只读沙盒例外。主应用写入 `widget-snapshot.json`，扩展不能修改它。该路径由构建脚本写入两个 bundle，适用于构建它的本机用户。正式分发应改用团队签名与 App Group，参见 [Apple 的 App Group 配置](https://developer.apple.com/documentation/xcode/configuring-app-groups)。

扩展与 SwiftPM 主应用共享 `DisplaySnapshot.swift`、`DisplayCard.swift` 和生成的词库。扩展使用与 Xcode App Extension 产品类型一致的 `_NSExtensionMain` 链接入口；仅使用 Swift 可执行文件的默认 `main` 会在注册 Widget 后直接退出，无法响应系统的扩展查询。

退出应用后快照标记为已退出；若应用意外结束，时间线在快照超过两分钟后标记待更新，不继续声称会话正在运行。跨日后不把昨天的 Token、成本与缓存率显示成今天的数据。额度重置时间已过时显示待更新，等待新的账户查询。

## 验证

```sh
sh scripts/test-display-lab.sh
AIEYES_SKIP_BUILD=1 sh scripts/test-native-navigation.sh
python3 scripts/test-release.py
```

显示实验室检查覆盖：全量摘要与页面筛选隔离、演示与真实模型隔离、状态优先级、文件格式与权限、跨日和过期语义、Dock 生命周期、实际帧推进及静态定时器清理、多屏坐标约束，以及真实卡片宿主的点击、拖动、锁定和位置保存。渲染输出位于 `.local/display-lab-verification/`，包含双语与浅深色小组件、Dock 各状态和窄窗口预览，共 39 张。

本机已在系统小组件图库中验证三种尺寸，添加中号小组件，并确认扩展从安装目录运行、读取应用发布的场景快照。若开发过程中移动了已注册的 `.app`，macOS 可能暂时沿用旧扩展路径；日常测评建议固定使用上述安装位置。

本次开发与系统集成验证环境为 Intel macOS 26.7.1、Xcode 26.5。最低部署目标保持 macOS 14；旧系统及 Apple Silicon 的实机表现未在这台机器上验证。
