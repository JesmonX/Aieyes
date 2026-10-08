# Codex 查询兼容性排查（2026-10-08）

报告来自 Windows Aieyes：SSH 来源显示「Codex 查询进程已退出」，本地来源显示限额查询失败。未取得故障机器及 SSH 目标的 CLI 版本和原始错误，因此以下是已复现的代码缺陷，不等同于现场根因已经确认。

## 已复现问题

通过 npm 官方 `@openai/codex` 包，在临时目录运行 0.114.0 的 macOS x64 CLI；未替换已有安装，未使用真实登录凭据。

1. `codex app-server --stdio --help` 返回退出码 2，错误为 `unexpected argument '--stdio' found`；`codex app-server --help` 成功，帮助标明 stdio 为默认传输。本机 0.160.1 接受该别名，不能据此假定其他机器或 SSH 目标也接受。
2. 0.114.0 接受当前 initialize 请求，但 `account/rateLimits/read` 的对象参数被拒绝，返回 RPC -32600、`Invalid request: invalid type: map, expected unit`。0.160.1 接受对象参数；在空配置目录返回要求账户认证的错误。
3. 原 Rust 查询将 stderr 重定向到空设备，断开时不读取退出码；RPC 错误则统一改写为「Codex 限额读取失败」，导致参数、账户和网络错误无法区分。

## 修改

- Rust 本地／SSH 查询与 Python 远程唤醒能力检查统一启动 `codex app-server`，沿用默认 stdio。
- 限额只在 -32600/-32602 且明确包含 `expected unit` 时，使用 `params: null` 重试一次；新版继续接收原有选项。认证、网络以及其他接口不做此重试。
- Rust 查询显示阶段、位置、自然退出码和 RPC 错误码。stderr 最多保留 64 KiB，同时持续排空管道；仅输出固定分类，不输出原始私密内容。关闭 stdin／终止进程属于清理步骤，不作为自然退出的证据。

## 验证与现场边界

跨平台 Rust 子进程回归覆盖：启动失败、SSH 认证失败、正常退出但无响应、初始化与限额 RPC 错误、旧协议重试、大量 stderr、超时和敏感信息不回显。子进程使用测试可执行文件模拟 CLI，不依赖真实账户或 SSH。

`cargo test -p aieyes-core`：81 项通过（1 个仅供子进程调用的 fixture 默认忽略）；`python3 scripts/test-wakeups.py`：11 项通过。新增回归在本轮 macOS 环境执行，尚未在 Windows 执行。

修复后的 `aieyes-core --call quotas.refresh` 分别调用真实 0.114.0 与 0.160.1，使用临时数据库、空 Codex 配置目录：两者均进入限额认证检查并显示「RPC -32600：需要 ChatGPT 订阅登录」，旧版不再停在启动参数／对象参数错误。没有验证已登录账户的联网限额查询。

现场仍需分别取得 Windows 本地与 SSH 目标的 `codex --version`、`codex app-server --help` 输出，以及更新后的具体错误。两个来源可能使用不同版本、配置目录、登录方式和代理。SSH 查询使用目标机器的环境及前置命令；本地查询使用 Windows 上的 CLI、所选配置目录和来源／应用代理。

官方文档：[Codex App Server](https://developers.openai.com/codex/app-server/) 说明 `codex app-server` 默认使用 stdio，并提供 `account/rateLimits/read` 接口。
