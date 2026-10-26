<p align="center"><img src="apps/website/public/brand.png" width="76" alt="Aieyes"></p>

<h1 align="center">Aieyes</h1>
<p align="center"><strong>所有 AI Agent，一眼看清。</strong></p>
<p align="center">面向 AI 编程工作流的桌面工作台。<br>用量、额度、账户、会话与远程机器，一处掌握。</p>

<p align="center"><a href="https://aieyes.2h3ngy4n.space/zh-CN/">产品官网</a> · <a href="https://github.com/JesmonX/Aieyes/releases/latest">下载安装</a> · <a href="README.md">English</a></p>
<p align="center">macOS · Windows · Linux</p>

![Aieyes macOS 原生工作台，展示账户额度与 Token 用量。使用演示数据及产品英文界面。](apps/website/public/screenshots/macos-overview.webp)

Agent 分散在不同工具里，额度、用量和远程会话也一样。Aieyes 给这些状态一个清晰的入口；只想快速看一眼时，打开紧凑面板就够了。

## 看清整个工作流

- **下一次提问前，心里有数。** 在今日 Token 用量旁，查看支持的账户额度、重置时间、Credits 和余额。
- **每一份用量，都有迹可循。** 从每日趋势到模型分布、缓存效率和活动热力图，按 Agent、账户或机器筛选。
- **多个账户，集中管理。** 组织 Agent 及关联设备，配置支持的 API；在已保存的 Codex 账户和 API 连接之间切换，并保留共用的对话历史。
- **需要你的时候，抬眼就知道。** 查看本机与 SSH 会话的工作、工具执行、等待输入和完成状态，选择桌宠、菜单栏或系统托盘。
- **远程机器，也能近在眼前。** 通过 SSH 监控 Linux 主机的 CPU、内存、GPU、文件系统、磁盘 I/O 和网络，按需选择指标。
- **给日常工作一点节奏。** 在本机或 SSH Linux 主机配置定时模型唤醒，每天多个时刻，并查看部署状态与运行结果。

## 融入你的桌面

| macOS 原生界面 | Windows |
| --- | --- |
| ![macOS 原生用量分析，使用演示数据与产品英文界面。](apps/website/public/screenshots/macos-analytics.webp) | ![基于实际前端渲染的 Windows 用量分析，使用演示数据与产品英文界面。](apps/website/public/screenshots/windows-analytics.webp) |

macOS 采用 SwiftUI/AppKit 原生界面，Windows 与 Linux 使用 Tauri 桌面界面，共享 Rust 核心。想要更轻巧的入口，可以固定紧凑面板，或让桌宠显示 Agent 状态。支持从文件、已适配资源链接和 npm 素材包导入桌宠，并在自己的库中切换。

[查看六组主要功能的 macOS 与 Windows 展示图 →](https://aieyes.2h3ngy4n.space/zh-CN/#overview)

<sub>展示图使用合成数据与产品英文界面。应用支持简体中文与 English，可在外观设置或面板语言按钮中切换。Windows 图片由实际前端渲染，不代表 Windows 实机桌面材质。展示桌宠：[月薪喵，作者 kiffin](https://codex-pet.org/zh/pets/yuexinmiao1/)。</sub>

## 接入你已经在用的工具

Codex · Claude Code · Antigravity · DeepSeek · ZCode · Qoder · Grok · Cursor · OpenCode · Kimi · Harness · CodeBuddy · Pi · Oh My Pi

历史读取、额度查询、API 配置与会话状态能力随接入而异。例如，Cursor 私有数据库中的请求级 Token 尚未作为支持的历史来源。具体能力见[接入说明](docs/agent-subscriptions.md)与[原生 Agent 验证记录](docs/native-agent-verification.md)。

## 开始使用

1. **[下载最新正式版本](https://github.com/JesmonX/Aieyes/releases/latest)**，选择对应系统的安装包。
2. **跟随配置向导**，启用 Agent，选择账户与数据来源。
3. **布置你的桌面**：连接 SSH 主机，选择面板，再挑一个桌宠。

| 平台 | 安装包 |
| --- | --- |
| macOS 14+ | Apple Silicon 与 Intel DMG |
| Windows x64 | NSIS 安装包 |
| Linux x64 | AppImage、DEB、RPM。Linux 各发行版的实测较少，目前仅为理论支持。如发现任何问题或影响体验的细节，欢迎[反馈](https://github.com/JesmonX/Aieyes/issues)。 |

发布页提供 `SHA256SUMS` 校验和。当前 macOS 构建使用 ad-hoc 签名，Windows 暂无发布者证书，首次打开可能需要在系统中允许。具体见[安装与发布说明](docs/releases.md)。

## 几点实用说明

用量历史保存在本地 SQLite 中。额度查询与 SSH 连接使用你配置的来源。API 等价成本用于估算使用价值，订阅费用和服务商实际账单另行计算。服务器监控展示当前采样，Token 历史持续保留。定时模型调用可能消耗额度，目标设备需要保持可用。

## 继续了解

- [产品官网](https://aieyes.2h3ngy4n.space/zh-CN/) · [发布记录](https://github.com/JesmonX/Aieyes/releases) · [反馈问题](https://github.com/JesmonX/Aieyes/issues)
- [Agent 与账户](docs/agents-and-accounts-settings.md) · [桌宠](docs/pets.md) · [Codex API 接入](docs/codex-api.md)
- [开发与验证](docs/development.md) · [技术设计](docs/architecture.md) · [官网维护](docs/website.md)

欢迎贡献代码、反馈问题或提供接入样例。分享诊断信息前，请移除凭据及私人账户、服务器信息。
