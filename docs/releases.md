# 发布与应用内更新

Release 工作流对稳定版本标签构建 macOS Intel / Apple Silicon、Windows x64、Linux x64。保留五个安装包：两个 DMG、NSIS EXE、DEB 和 AppImage；另发布三个 Tauri 包签名、两个已签名的 Sparkle appcast、`latest.json` 和 `SHA256SUMS`。

## 版本准备

同步修改 Rust 核心、Tauri Cargo 包、Tauri 配置及 macOS `CFBundleShortVersionString`，更新两个 Cargo.lock。macOS `CFBundleVersion` 必须为正整数，每次发布递增；Sparkle 按构建号判断是否有更新。已有公开 Release 不覆盖，修复须使用新版本标签。

Python 工具要求 Python 3.11+。Tauri CLI 固定为 2.12.1，Sparkle 固定为 2.9.6；提交 SwiftPM 的 `Package.resolved`。

```sh
python3 scripts/release.py check --tag v0.1.6
python3 scripts/test-release.py
node --test scripts/test-desktop.mjs scripts/test-multiselect.mjs scripts/test-release-publish.mjs
```

示例标签必须替换为实际待发布版本。旧客户端没有安装器，需要先手动安装一次首个支持应用内更新的版本；之后才能一键升级。不能通过修改 Release 说明给已安装的旧程序补上更新器。

## 一次性配置更新签名

更新签名不需要 Apple Developer 或 Windows 证书，但需要两组长期密钥。公钥内置应用；私钥仅用于发布签名。使用仓库 Variables/Secrets，不能把测试密钥用于正式更新，不能每次发布重新生成密钥。

| GitHub 配置 | 用途 |
| --- | --- |
| Variable `SPARKLE_PUBLIC_KEY` | Sparkle Ed25519 公钥 |
| Secret `SPARKLE_PRIVATE_KEY` | Sparkle 导出的 32 字节 seed 的 base64 文本 |
| Variable `AIEYES_UPDATER_PUBLIC_KEY` | Tauri `.pub` 文件的完整内容 |
| Secret `TAURI_SIGNING_PRIVATE_KEY` | Tauri 私钥文件的完整内容 |
| Secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Tauri 私钥密码；无密码时留空 |

在安全的开发机器上生成并备份密钥。例如，先准备一个仓库外的私有目录，再运行：

```sh
sh scripts/prepare-sparkle.sh
.build/sparkle-tools/bin/generate_keys --account aieyes
.build/sparkle-tools/bin/generate_keys --account aieyes -x /安全目录/sparkle.key
npm install --global @tauri-apps/cli@2.12.1
tauri signer generate -w /安全目录/tauri.key
```

Sparkle 工具会打印公钥，Tauri 公钥在 `.pub` 文件中。私钥填写到对应 Secret，并在独立安全位置备份。Sparkle 私钥丢失且没有 Developer ID 时，现有客户端不能通过换一个公钥无缝继续更新。

正式 Release 缺少公钥或签名密钥、密钥不匹配、签名验证失败时停止发布。本地调试和 Desktop 工作流可不配置更新密钥，页面会明确提示该构建尚未配置应用内更新。

## 客户端行为

- 设置页显示当前版本；手动检查无新版时显示“当前已是最新版本”。本地版本更高时明确标注，不降级。
- 新版弹窗显示当前/目标版本和说明。点击“更新并重启”后下载、验证并安装；失败显示原因，不虚报成功。
- 默认启动时及每 24 小时检查，可关闭。选择“稍后”后同一版本 24 小时不自动重复提示；后台发现新版，等用户回到应用再展示。
- macOS 使用 Sparkle 的签名 appcast 和 EdDSA 包签名；安装器保留用户数据。更新网络使用 macOS 系统网络设置。
- Windows 使用用户级 NSIS 更新；Linux AppImage 原位更新。DEB 验签后通过系统 `pkexec` 授权调用 apt 安装；取消授权即停止，不收集密码。无 polkit、无 apt 或包管理器忙时显示失败，修复系统条件后重试。
- Windows/Linux 更新请求沿用应用代理设置；更新下载独立于业务引擎。安装前等待正在进行的数据操作结束，并保护未保存的设置。

## 构建、签名与发布

```sh
sh scripts/build-macos.sh release --dmg
sh scripts/test-native.sh
```

macOS 部署目标为 14；主程序与 Rust 核心匹配目标架构。打包嵌入 Sparkle framework，并从内部辅助程序到整个 app 逐层签名。设置 `SPARKLE_PUBLIC_KEY` 可在本地打包时内置公钥；`AIEYES_REQUIRE_UPDATER=1` 强制要求公钥。

Release 中每个平台在安装包最终命名后签名。Tauri 签名绑定版本号，客户端启用 `requireSignedVersion`，防止更新清单把旧包伪装成新版本。macOS 在最终签名、公证完成后对 DMG 和 appcast 签名。收集任务验证附件全集、版本、目标平台、签名和尺寸，生成静态更新清单及校验和。

更新源位于 `https://github.com/JesmonX/Aieyes/releases/latest/download/`：`latest.json`、`appcast-macos-x64.xml`、`appcast-macos-arm64.xml`。清单中的安装包 URL 固定指向对应版本标签，不能混用 latest 下载链接。

所有附件先上传到草稿并核对，完成后才公开；中断保留草稿，重跑替换草稿附件。发布只需要 GitHub 自动的 `GITHUB_TOKEN`，安装包签名仍需要上述 Secrets。发布说明按实际配置标注操作系统签名情况。

## macOS 安全拦截与可选公证

当前没有付费 Apple Developer 账号时，继续使用 ad-hoc 签名。更新包签名用于防止更新内容被篡改，不能替代 Apple 的 Developer ID 和公证。首次下载后系统仍可能要求用户在系统设置中允许打开；不移除 quarantine、不关闭 Gatekeeper。

准备好账号后配置以下项，工作流会切换到 Developer ID、Hardened Runtime、Apple 公证和 app/DMG stapling：

| GitHub 配置 | 用途 |
| --- | --- |
| Variable `MACOS_SIGNING_IDENTITY` | 完整 Developer ID Application 身份名称 |
| Secrets `MACOS_CERTIFICATE_P12` / `MACOS_CERTIFICATE_PASSWORD` | 证书与私钥的 P12 base64 / 密码 |
| Secrets `APPLE_ID` / `APPLE_TEAM_ID` / `APPLE_APP_PASSWORD` | 公证身份、团队和应用专用密码 |

CI 在临时钥匙串导入证书，使用后清理。只配置部分签名/公证凭据会报错，不静默降级成未公证发布。Windows 当前没有 Authenticode 证书，首次安装仍可能有系统信誉提示。

## 验证

除 Rust、原生模型和浏览器测试外，可使用一次性测试密钥验证真实签名流程：

```sh
sh scripts/prepare-sparkle.sh  # 仅 macOS 需要
python3 scripts/test-update-signatures.py
```

测试不会访问发布私钥；覆盖包内容篡改、公钥错误、版本号不匹配，以及 Sparkle appcast 篡改。浏览器更新测试包含新版/同版/本地较新版、未保存设置、进度、签名失败及自动检查设置。

发布前仍应在干净机器上用两个测试版本验证 macOS 双架构、Windows NSIS、Linux AppImage/DEB 的实际升级与重启，检查配置、凭据和采样记录保留，并覆盖拒绝授权、只读目录与包管理器占用。签名公证分支须在配置真实 Developer ID 后实机验证。
