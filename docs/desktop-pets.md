# 桌面宠物与会话状态

Windows、macOS、Linux 首次启动默认显示桌宠。首次使用通过统一文件导入器导入随应用分发的[月薪喵](https://codex-pet.org/zh/pets/yuexinmiao1/)（作者 kiffin），不需要联网。成功导入会启用新桌宠并保留旧桌宠，也可在库中切换；升级和重启保留当前选择。

在「设置 → 常规 → 桌面显示」导入本地 ZIP 或 `pet.json`，也可输入资源链接、已适配站点页面、npx 命令或 npm 素材包。只解析并下载素材，不执行安装器或包脚本。导入失败或提交前取消保持原桌宠；升级恢复 Aieyes 自身有效的历史缓存，损坏项单独提示。完整格式、桌宠库与恢复规则见[桌宠导入与选择](pets.md)。

## 公开文件格式

2026-10-13 使用三个站点的真实文件完成导入验证：

| 来源 | 验证样本 | 文件与图集 |
| --- | --- | --- |
| [CodexPets](https://codexpets.net/api/pets/pixel-coder/download) | Pixel Coder | ZIP 内嵌目录，`pet.json` + PNG，v1 8×9；包含 `__MACOSX` / AppleDouble 附带文件 |
| [Petdex](https://petdex.dev/api/install-pet/boba) | Boba | `pet.json` + WebP，声明 `spriteVersionNumber: 2`，v2 8×11 |
| [Codex Pet](https://codex-pet.org/zh/pets/yuexinmiao1/) | 月薪喵 | `.codex-pet.zip`，`pet.json` + WebP，v1 8×9 |

它们共用 manifest 和分格图集的基本结构，但封装、图片编码和图集版本不同。可直接输入资源链接，或下载文件后导入；本地 `pet.json` 的图集须在相同目录或其子目录。导入器支持 PNG / WebP、8×9 和 8×11，以及每格比例为 192:208 的等比例整数缩放。版本缺省按 v1 处理。

导入时忽略 macOS 附带文件，拒绝多宠物歧义包、路径穿越、符号链接、加密/损坏 ZIP、无法解码或尺寸不符的图集。上限为 64 MiB 压缩包、128 MiB 声明解压量、1,024 个文件、32 MiB 源图片及 1,600 万像素。统一转换成 RGBA PNG，写完文件后才提交当前桌宠指针。客户端按资源变化加载图片，不在每次会话轮询中传输图集。

## 桌面行为

- 显示方式仅有桌宠和系统托盘。旧 `auto` / `floating` 偏好迁移为桌宠，明确选择的托盘或隐藏偏好保留。没有可用托盘时恢复桌宠入口，避免应用无法打开。
- 单击打开面板；拖动移动并保留位置，松手后距工作区边缘 48 逻辑像素内吸附；屏幕变化时限制到可见区域。右键菜单提供「设置」「桌宠设置」及隐藏或恢复入口。macOS 同时保留菜单栏，不新增重置位置入口；Windows/Linux 保留原有重置位置功能。
- 三端使用相同的状态优先级及动画时间轴。动画持续循环当前会话状态对应的动作，与下方文字一致，直到真实状态变化；轮询不会重启动画，新的一轮工作会重新触发。遵循系统减少动态效果设置。
- 各动作使用共享的逐帧时长，保留尾帧停顿；idle 六帧一轮为 1.1 秒，工作动作一轮为 0.82 秒。中断使用 idle 动画，文字保留「已中断」。完整时间表见[实施计划](cross-platform-and-pet-plan-2026-10-14.md#动画节奏)。
- 多会话共用一个宠物，按状态优先级选择主动作，文字条同时显示各状态数量。数量变化不会重启动画；完成、失败、中断不计入活跃数。文字换行时窗口随之增高。
- 「设置 → 常规 → 桌面显示」提供桌宠缩放：50%–200%，每档 5%，默认 100%，可恢复默认。比例保存在当前设备，重启、导入其他宠物时保留；图片、圆角状态条、点击区域及窗口一起缩放；文字按最终字号排版，不放大已栅格化的文字层，并限制在屏幕可见区域。该比例独立于显示器 DPI 和面板尺寸。
- 图集先裁成独立帧再缩放，避免采样到相邻帧。Windows 复用绘制缓冲区，状态文字和圆角背景以三倍物理分辨率绘制、面积采样后缓存，使用预乘透明度合成；Web 使用透明 Canvas，macOS 使用裁切后的 CGImage。保留素材本身的描边，不添加窗口阴影。
- Windows 使用独立原生透明窗口与消息线程，面板 WebView 重建不影响桌宠和菜单。旧横条入口及相关渲染、选项、静态猫图资源已删除。

## Codex / Claude Code / AGY

监测范围跟随 Agent 接入中启用的来源和机器，包含本机及已配置 SSH 机器。同一机器/Agent/目录的账户连接不会重复计算会话；hooks 按实际配置目录去重。

会话状态使用独立刷新任务和查询通道，与 Agent 历史同步、限额查询分开。macOS 在配置读入后立即启动约每秒一次的状态刷新，保存设置和价格重算不会暂停它；准备应用更新时暂停，恢复后继续，退出时停止。Windows/Linux 使用独立监测线程。动画按共享逐帧时长的单调时间轴播放，不等待状态查询返回；Web 使用持续帧回调，Windows 与 macOS 高频检查帧号，仅帧变化时刷新图片，普通状态轮询不重建定时器；macOS 拖动采用非阻塞鼠标事件。新状态到达时同步切换动作和文字。

本机每秒读取已有文件，每 5 秒发现新文件。每台 SSH 机器使用独立后台任务和缓存，约每 5 秒读取一次，单次传输最多等待 12 秒。某台机器失联不会阻塞本机或其他机器。会话输出包含 Agent、机器、来源、轮次及每个来源的健康状态，不包含提示词、工具参数或回复。

- **Codex**：增量读取 JSONL 生命周期，跟踪轮次和并行输入等待；匹配的工具返回才解除该等待。
- **Claude Code**：自动合并 Aieyes 的观察 hooks，覆盖启动、提示提交、工具、权限/输入等待、子任务及结束。只读 JSONL 提供既有会话恢复。
- **AGY（Antigravity CLI）**：观察生命周期 hooks，同时只读查询 conversations SQLite（含 WAL）的步骤状态。`PostInvocation` 不是整轮完成，`Stop.fullyIdle=false` 表示后台任务仍在运行。完成的工具步骤不等于会话完成。不安装 AGY `PreToolUse` 权限拦截器。

状态优先级为：等待输入/权限 → 最终失败 → 执行/思考/工具 → 完成/中断 → 待确认 → 空闲。等待会话计入活跃数。单个工具失败可以恢复，不直接判定会话失败；其他来源故障也不会覆盖已知正常状态。

「设置 → Agent 接入 → 对应 Agent → 机器与目录」显示 hooks 接入状态，并提供**修复接入**和**停用**。配置操作只管理带 Aieyes 标记的处理器，保留其他配置和 hooks。自动接入尊重显式停用；修复可重新启用 Aieyes 自己的条目。配置 JSON 无效或指向符号链接时保留原文件并报告问题。无需为每个账户重复安装。

SSH 上 Codex 只读会话日志，无需部署 hook。Claude / AGY 在采集时通过 SSH 自动放置观察器并合并配置，需要远端 Python 3，不安装常驻服务。机器行复用会话健康缓存；尚未读到健康状态时显示“待确认”，暂停采集时显示“采集已暂停”。点击状态可查看每个目录并管理自动接入。

观察器只落盘有限的生命周期字段。Claude 观察器不输出授权决定；AGY 仅返回事件所需的无操作结果，停止事件返回 `{"decision":"allow"}`，不会自动批准工具或改变权限策略。标准输入/文件读取异常不会阻塞 Agent。辅助程序使用应用管理的稳定路径，不依赖安装目录中的临时位置。

状态是可观察证据的投影：未接入 hooks 的旧会话依赖日志；CLI 未发出事件的权限切换要等下一条日志或 hook 才能确认。没有可验证的进程身份且连续 3 分钟没有更新时转为待确认，完成/中断保留 90 秒，过期未确认记录会移除；有匹配进程身份的长任务继续保留运行状态。SSH 离线保留带故障提示的缓存。每个来源最多发现最近 24 小时的 256 个会话文件。

## 接口与验证

`pets.current`（`includeImage: true` 按需读取图集）、`pets.import({input})`（兼容 `path`）、`pets.list`、`pets.select({libraryId})`、`pets.asset({libraryId})`、`sessions.list`、`sessions.hooks.status`、`sessions.hooks.repair({sourceId})`、`sessions.hooks.remove({sourceId})` 的注册信息位于 `docs/core-contracts.json`。

验证入口：`cargo test --workspace --locked`、`scripts/ci-checks.sh`、`npm run test:ui --prefix apps/desktop`、`scripts/test-native.sh`、Windows `scripts/test-windows-panel.ps1`。真实桌面输入测试会记录运行或跳过原因，不把无交互桌面的结果当作鼠标/焦点验证。

实现参考了 [clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk) 的观察器管理和后台任务处理思路；实现代码由 Aieyes 独立编写，未复制其 AGPL 代码或素材。协议依据 [Claude hooks](https://code.claude.com/docs/en/hooks)、[AGY hooks](https://antigravity.google/docs/hooks) 和 [Petdex 图集定义](https://github.com/crafter-station/petdex/blob/main/src/lib/sprite-atlas.ts)。默认资源及原始校验值见 `crates/core/assets/README.md`。
