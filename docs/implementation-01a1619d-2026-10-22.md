# 会话 01a1619d 实施与验收

对应方案：`01a1619d-848a-7a41-8018-af85b17ad1b4`，以及将精确计数改为所有 Agent 共用的补充要求。交付范围为代码、素材与本地验收；未发布 Release、提交 Git 或部署官网。原工作区改动保留，开始实施前的差异与源码副本位于 `.local/implementation-01a1619d-baseline/`。

## 共享精确计数

配置改为 `preciseCounting`，Web 与 Swift 的“刷新与历史”页使用同一个开关。新安装默认开启；已有安装通过 `counting.global-default.v1` 仅开启一次，此后保留用户选择。仍接受旧的 `codexPreciseCounting` 输入，同时出现时以新字段为准。

开启时排除有证据的继承用量；关闭时按会话恢复原始用量。两种模式都保留请求、重复导入与镜像记录的正常去重。独立子 Agent 的新请求继续计入。用量原文与首次价格保存在 `usage_inheritance`；切换模式重放有效投影，不拿当前价格重算旧价格。源游标包含开关和计数代次，旧任务不能覆盖快速切换后的结果。下次同步执行重算，已入库的继承记录在原文件缺失时仍可恢复。

首次历史修正使用独立的 `shared-counting-v1-*.sqlite` 备份，保留已有备份与价格。修正会更新相关估算和统计缓存。

已逐项检查全部 15 个 Provider 注册项：

| Provider | 可验证的继承依据或适用范围 |
|---|---|
| Codex | 有界 `history_base` 解析、父历史累计基线；未确认的首段累计保留诊断，后续有效增量继续统计 |
| Claude Code | 原始请求 ID、消息 ID、时间戳；跨会话复制使用相同来源身份 |
| Antigravity | 原生请求 ID 与起止时间；只有步骤下标时不推断跨会话复制 |
| DeepSeek | API 余额来源不产生请求 Token；请求导出可通过标准 JSONL 来源导入 |
| ZCode | 原生结算请求 ID 与完成时间 |
| Qoder | 请求/消息 ID 与原始时间；加密会话残差使用未扣除继承前的原始计量，避免把已排除的用量补回 |
| Grok | 原生完成事件 ID、模型与毫秒时间 |
| Cursor | 标准 JSONL 可携带继承证据；私有会话库的请求 Token 仍未接入 |
| OpenCode | v2 `fork_session_id`、`fork_boundary` 与 `seq`；正常 `parent_id` 委派不等于复制。稳定消息身份也可去重 |
| Kimi Code | 最后一个完整 `forked` 边界之前的前缀；继承和新请求各自编号，保留稳定身份 |
| DeepSeek Harness | v0 `seedLength` 与 v4 seed 边界；明文/连续 Zstd、代际重编号一致。继承与新请求的坐标相同时仍分开保存 |
| CodeBuddy | 原始消息 ID 与时间；独立保留既有请求/工具镜像和 Credits 去重 |
| Pi | 稳定条目 ID 标识来源，每个会话保存独立观察；旧身份迁移保留价格与来源关联 |
| Oh My Pi | 与 Pi 共用条目计量，保留标题槽、原生目录与游标规则 |
| 自定义 | 标准 JSONL 的可选 `inheritance: {origin, inherited}`；SSH 只传这两个字段，来源字符串有长度限制 |

OpenCode v1 的部分 fork 会重写消息 ID，且未保存原始身份或边界。此类记录不能可靠识别，仍计入；没有使用文本相似度或 Token 相等来猜测。无原生请求数据的来源不会因为共享开关而获得虚构计量。

计数回归覆盖反复开关、升级一次默认值、旧配置别名、冻结价格、来源镜像、独立账户、原始文件缺失、快速切换拒绝旧结果、本机与 SSH 投影、普通子任务，以及 Harness 继承/新请求坐标冲突。

## Qoder

本机和 SSH 同时尝试保留大小写与小写的项目路径哈希，由 AES-GCM 认证结果决定匹配项。控制协议保留 stdin，先等待匹配的 initialize 成功响应，再发送额度请求；仅接收匹配的账户 usage 响应，忽略会话统计和其他请求响应。

总预算为 45 秒，临时断流、超时和额度暂不可用最多重试一次。登录或身份错误不重试。失败仅更新错误状态，保留上次成功的额度与时间。

2026-10-22 使用新核心与隔离数据库实测；原 Aieyes 设置只读，未切换登录或修改原生配置：

| 位置 | 成功解密状态 | 重复同步 | 连续额度查询 |
|---|---:|---|---|
| 本机 | 13 | 事件数与 Token 均不变 | 3/3 成功，约 4.97–5.04 秒/次 |
| SSH `144` | 63，含此前失败的全部 10 份 | 事件数与 Token 均不变 | 3/3 成功，约 5.16–5.56 秒/次 |

`144` 另有一行原始 JSONL 无法解析。收集器保留失败位置重试，其他文件和加密状态正常入库；此来源仍显示部分完成，没有修改或跳过原文件来隐藏错误。

9 项额度协议回归与 10 项状态回归覆盖慢初始化、stdin 生命周期、匹配响应、断流、总超时、临时失败、仅会话响应、零余额、身份不符、登录错误不重试、失败保留旧值/时间、两种哈希、损坏认证数据，以及残差不恢复继承重复。

可复验命令：

```sh
python3 scripts/verify-qoder-sync-live.py \
  --settings-db "$HOME/Library/Application Support/Aieyes/aieyes.sqlite" --ssh 144
```

该脚本仅对已有本机/指定 SSH 来源进行读操作，另建 Aieyes 数据库；报告不输出凭据、身份或会话内容。本次报告位于 `.local/qoder-sync-live/1792655889359841000/report.json`，`passed` 限定为解密/重复同步/额度检查，`historyComplete: false` 如实反映损坏的 JSONL。

## App 与官网对照

| 修改前 | 修改后 |
|---|---|
| 部分英文导航、账户操作与设备状态比中文多折行 | 共用词库缩短操作及关键指标；导航和数字采用单行规则，保留说明正文自然换行 |
| 原生紧凑内容固定 450px，在 360px 容器两侧被裁切 | 内容跟随父容器宽度，常规弹出面板仍由宿主指定 450px |
| 360px 原生面板的英文额度刷新按钮折为两行，高 38px | 结合所在的额度区将文案简化为“Refresh”，明暗主题均为单行 24px，与中文一致 |
| 官网只有一套英文展示图 | 两种语言 × macOS/Windows × 四个页面，共 16 张；无语言路径保留英文副本 |
| 官网字体和字号分散，部分深色图标不清楚 | Comic Sans MS 优先，Comic Neue/霞鹜文楷自托管回退；四级字号与深色图标处理 |
| 桌宠按图集全部列数播放 | 按共用时长表播放 6/8/5 个有效帧，无空白帧；减少动态效果时静止 |
| 平台下载入口先去 Release 页面 | 六个安装包各有动态直接下载地址，首屏/导航先提供包选择 |

App 未改字体。Web 80 组语言对照覆盖 Windows/Linux、明暗主题、760/1120/1440px 主窗口和 360/450px 面板，含长名称、错误提示、设置页、按钮行数及裁切。原生总览、分析、服务器与面板使用相同尺寸/主题矩阵，22 组语言对照（44 个真实 SwiftUI 视图）通过控件边界、可见控件重叠与短文案行数检查；另目视检查了窄窗口、面板及最终展示图。

原生 PNG 与无障碍几何快照位于 `.local/website/native/{en,zh-CN}/audit/`，结果为 `.local/website/native/layout-report.json`。采集使用独立预览进程；屏外虚拟节点保留身份并标记为无几何信息，滚动条的滑块尺寸不作为按钮文字行数比较。检查器拒绝空树或缺少足够可见对照项的结果。360px 刷新按钮修正前的截图与失败记录保存在 `.local/implementation-01a1619d/native-panel-refresh-before.png`、`native-layout-before-final-fix.json`，最终检查日志为 `native-layout-final.log`。

语言与状态回归验证用户名称/路径不被翻译，切换后保留草稿、独立筛选与滚动状态；原生回归还验证多窗口配置合并、同字段冲突与自动保存。修改前 Web 几何结果为 `.local/bilingual-layout/before.json`，当前为 `report.json`。三张修改前的已跟踪 App 预览保存在 `.local/implementation-01a1619d/before-*.webp`；当前素材位于 `apps/website/public/screenshots/`。

官网文案突出单 Agent 多账户、多 Agent 多账户，以及账户/API 切换和继续会话。移除重复限制、截图实现说明与重复署名；桌宠署名保留在页脚。Comic Neue 与霞鹜文楷的来源、版本和 OFL 许可位于 `apps/website/public/fonts/`，没有分发微软字体。分享图也使用相同的英文字体及最终 App 总览重建。

Chromium 与 WebKit 已在本轮最终素材构建后通过验收，包括字体加载、图标、双语/平台切换、全部截图、无 JavaScript、正常动画逐帧非空白、减少动态效果，以及 320/360/390/430/768px 与横屏触控布局。首屏、账户、下载及 Provider 图标区的最终截图也已目视检查，位于 `.local/website/qa/`。

`.local/implementation-01a1619d/final-assets.json` 记录 16 张展示图的大小与 SHA-256；逐一核对了源码素材与 `dist/website` 内容一致，8 个旧英文路径及分享图也一致。两个浏览器的最终日志分别为 `website-chromium-final.log`、`website-webkit-final.log`。

## 六包下载

`scripts/website/Caddyfile` 在请求时解析 GitHub 最新正式标签，不写死版本、不要求新 Release 重建网站。所有包路由禁止缓存，保留旧入口与现有访问日志；统计增加架构/格式。失败返回带原地址重试及 Release 链接的 502 页面，无 JavaScript 也可用。

实际本地 Caddy 验证了六包、两个模拟版本、四类上游错误、恢复重试、五个旧入口和真实访问日志统计。2026-10-22 通过 GitHub 官方 Release 元数据核对，[v0.1.54](https://github.com/JesmonX/Aieyes/releases/tag/v0.1.54) 的六个命名目标均存在。

| 路由后缀（`/go/download/`） | 安装包后缀 |
|---|---|
| `macos/arm64/dmg` | `macos-arm64.dmg` |
| `macos/x64/dmg` | `macos-x64.dmg` |
| `windows/x64/exe` | `windows-x64.exe` |
| `linux/x64/appimage` | `linux-x64.AppImage` |
| `linux/x64/deb` | `linux-x64.deb` |
| `linux/x64/rpm` | `linux-x64.rpm` |

报告：`.local/website/download-test.json`、`latest-release-assets.json`。这验证了本地路由与公开安装包，未声称生产官网已部署新路由。维护与重建步骤见 [website.md](website.md)。

## 回归记录

- `cargo test -p aieyes-core --locked --no-fail-fast`：510 通过，4 个手动基准按原配置忽略。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo fmt --all -- --check`：通过。
- 本地化同步、3260 条源文案覆盖与 3313 条词库占位符检查：通过。
- `test-i18n.cjs`、`test-bilingual-layout.cjs`、`test-agent-settings-ui.cjs`、`test-history-rescan-ui.cjs`：通过。
- `python3 scripts/test-native-bilingual-layout.py`：22 组语言对照通过；修正真实检测出的 360px 英文刷新按钮额外折行后，重新生成英文全矩阵并验收。
- Swift `verify-localization`、`verify-configuration`、`verify-macos-ui`、`verify-sessions`：通过；真实核心的默认开启和开关保存也通过。
- Python 增量历史 25 项、原生数据库 9 项、Harness 6 项、Qoder 环境 3 项：通过。
- 官网 Chromium/WebKit、真实 Caddy 下载与实际 Qoder 读取：通过，保留上文列明的原始坏行及不支持的历史格式限制。

核心与静态检查日志位于 `.local/implementation-01a1619d/`。本轮没有改动原工作区已有的 Linux 桌面实现、发布脚本或先前 Agent 实测报告。
