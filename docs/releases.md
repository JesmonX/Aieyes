# 发布

Release 工作流推送稳定版本标签后运行四个原生构建任务：`macos-15-intel`、`macos-15`、`windows-2022`、`ubuntu-22.04`。产物共五个：macOS x64 / arm64 DMG、Windows x64 EXE、Linux x64 DEB / AppImage。

## 版本准备

同步修改以下版本，提交对应的锁文件，再创建标签：

- `crates/core/Cargo.toml` 和 `apps/desktop/src-tauri/Cargo.toml` 的 package version。
- `apps/desktop/src-tauri/tauri.conf.json` 的 version。
- `apps/macos/Info.plist` 的 CFBundleShortVersionString；CFBundleVersion 随构建递增。
- 根目录和 Tauri 目录下 Cargo.lock 中的本项目包版本。

前端版本从核心的 `hello` 接口读取；macOS 从应用 Bundle 读取。检查需要 Python 3.11+：

```sh
python3 scripts/release.py check --tag v0.1.0
python3 scripts/test-release.py
node --test scripts/test-desktop.mjs scripts/test-multiselect.mjs scripts/test-release-publish.mjs
```

将 `v0.1.0` 替换为本次版本。已有公开 Release 不覆盖，修复后发布新版本。当前仅支持稳定的三段版本号。

## 构建与发布行为

推送版本标签后自动构建。也可从 Actions 手动选择已存在的标签：默认仅验证构建，勾选 `publish` 才发布。手动模式验证输入确实指向对应 Git 标签。

Release 强制执行 Rust 核心测试、Python 采集器测试、JavaScript 交互测试；Windows/Linux 加测 Tauri Rust，Linux 加测 Chromium 页面交互。macOS 编译 SwiftUI 应用，运行原生会话与选择模型测试，核验主程序和内置核心架构、ad-hoc 签名及 DMG。

构建全部成功后统一收集安装包，验证文件集并生成 `SHA256SUMS`。上传先进入草稿，所有附件上传并核对名称、大小和状态后才公开。中途失败保留草稿，可重新运行失败任务；相同标签串行发布。构建任务只有仓库读取权限，发布任务使用 GitHub 自动提供的 `GITHUB_TOKEN` 获取 contents:write，不需要额外发布密钥。

附件名称为 `Aieyes-版本-平台-架构.扩展名`。发布页自动生成提交更新说明，并注明平台、架构及签名方式。

## macOS 本地 DMG

```sh
sh scripts/build-macos.sh release --dmg
sh scripts/test-native.sh
```

脚本按当前机器架构构建，部署目标保持 macOS 14。DMG 内含 Aieyes.app 和 Applications 快捷入口；Swift 主程序、Rust 核心保持相同架构。调试构建仍输出 `dist/Aieyes.app`。

本轮沿用 macOS ad-hoc 签名、Windows 未签名；Developer ID 公证、Windows 证书签名及自动下载安装更新不在当前流程中。系统材质、安装、升级及卸载体验仍需目标系统实机验证。
