# Aieyes 官网维护

官网：https://aieyes.2h3ngy4n.space ，中文：https://aieyes.2h3ngy4n.space/zh-CN/index.html 。默认英语，语言由 URL 决定；切换语言保留章节，平台选择在当前浏览器会话中保留。

## 内容与构建

源码位于 `apps/website`：`content.mjs` 管理中英内容，`style.css` 和 `site.js` 管理外观及轻量交互，`public` 保存部署素材。静态生成器共用页面结构，无网站运行时依赖，不需要在服务器安装 Node。

在仓库根目录执行：

```sh
node scripts/website/build.mjs
node scripts/website/test.cjs
```

输出位于 `dist/website`。测试使用桌面项目已有的 Playwright，检查中英页面、全部图片、README 本地链接、平台切换、放大与焦点恢复、移动端菜单、FAQ、锚点和窄屏布局；视觉验收截图位于 `.local/website/qa`。可用本地静态服务器预览此目录。

触控检查覆盖 320、360、390、430px 手机宽度、768px 平板及 844 × 390 横屏，验证至少 44px 的操作区域、功能标签可见性、桌宠与说明不重叠，以及图片弹窗和菜单在矮屏内完整显示。安装 Playwright 的 WebKit 后，可运行 `AIEYES_WEBSITE_BROWSER=webkit node scripts/website/test.cjs` 复核另一浏览器引擎。

部署后可使用 `AIEYES_WEBSITE_ORIGIN=https://aieyes.2h3ngy4n.space node scripts/website/test.cjs` 对正式域名运行同一套浏览器检查，覆盖线上资源与 CSP 下的交互。

## 重建展示图

```sh
node scripts/website/encode.cjs --pet
python3 scripts/website/render.py --language all --audit-layout
python3 scripts/test-native-bilingual-layout.py
node scripts/website/render-windows.cjs --language all
node scripts/website/og.cjs
node scripts/website/build.mjs
node scripts/website/test.cjs
```

macOS 渲染要求本机图形会话、Xcode/Swift 和已有的 `.build/swift/debug/Sparkle.framework`。首次可运行项目现有 macOS 构建来准备依赖。Windows 展示使用项目实际前端在 Chromium 中渲染。原始 PNG、预览源码、临时应用及隔离数据只写入 `.local/website`；优化后的展示素材位于 `apps/website/public`。

`fixture.py` 提供固定的合成账户、用量、服务器、任务与会话数据，不读取个人配置，不联系模型服务商。渲染使用产品共用词库与真实语言设置。两个渲染脚本均支持 `--language all|en|zh-CN`，默认生成中英文 × macOS/Windows × 总览、分析、服务器、面板，共 16 张图片。素材位于 `public/screenshots/{en,zh-CN}/`，旧的无语言路径继续指向英文副本。官网按当前语言与平台选择图片。Windows 图片由实际前端渲染，不作为 Windows 实机材质验收记录。

`--audit-layout` 额外输出 macOS 两种语言、明暗主题、760/1120/1440px 主窗口及 360/450px 面板的图片与控件几何信息，位于 `.local/website/native/{en,zh-CN}/audit/`。每个几何快照使用独立预览进程，避免 SwiftUI 无障碍树跨窗口失效。随后运行 `python3 scripts/test-native-bilingual-layout.py` 检查 22 组语言对照的控件边界、重叠及短文案行数；PNG 还需检查字形截断与材质效果。Web 界面对应检查通过 `node scripts/test-bilingual-layout.cjs` 运行。

Codex 展示账户使用 5h $7.10、7d $43.20、每 1000 Credits $40.00 的合成估值；这些数字只用于官网渲染。

桌宠动画使用原有 8 × 9 帧图集，由 `docs/core-contracts.json` 的帧时长生成 CSS：空闲 6 帧、工作 8 帧、完成 5 帧，仅播放有效帧。减少动态效果时保持静态展示。内置月薪喵素材来自项目既有资源，作者为 **kiffin**，页脚保留来源署名。Provider 图标优先使用深色背景版本，单色图标呈浅色，彩色图标保留原色。分享图由实际产品界面与网页排版生成。

官网字体顺序为 Comic Sans MS、Comic Neue、霞鹜文楷。Comic Sans MS 使用访问者已有字体；两个开源回退字体及 OFL 许可自托管于 `public/fonts/`。`scripts/website/fonts.py` 从官方源下载并按当前网站文字生成 WOFF2 子集，新增中文文案后需重建字体。依赖为 Python `fonttools`、`brotli`；字体来源版本记录在 `public/fonts/sources.json`。页面共用四级字号，桌面正文/控件 18px、辅助 16px，移动正文 16px。

## 部署

目标为本机 SSH 别名 `proxyus`。脚本禁用此别名附带的端口转发，沿用已有 SSH 密钥和主机校验；首次部署安装 Ubuntu 仓库中的 Caddy。需要目标用户可使用非交互式 sudo，以及域名已指向主机、外部 TCP 80/443 可达。

```sh
sh scripts/website/deploy.sh
```

脚本先构建与验证，再上传静态输出到 `/var/www/aieyes/releases/<UTC 时间>`，验证 Caddy 配置后原子切换 `current` 链接。Caddy 通过 systemd 常驻，管理证书续期及 HTTP → HTTPS 跳转。站点独立配置为 `/etc/caddy/sites/aieyes.caddy`，通过 import 接入主配置，不覆盖其他站点。`/etc/caddy/aieyes-global-options` 将 TCP 443 的协议限定为 HTTP/1.1 与 HTTP/2，保留服务器已有的 Hysteria UDP 443；全局块通过 import 合并。配置备份仅保存在服务端 `/etc/caddy/aieyes-backups`，不对外提供。

部署后分别核对公网英文首页和中文页，保留最近三份版本。页面不引入客户端统计脚本；外跳点击由 Caddy 服务端记录，不依赖 Cookie 或第三方服务。

GitHub 与下载按钮通过站点自身的 `/go/*` 端点做 302 跳转，Caddy 将访问日志写入 `/var/log/caddy/aieyes-access.log`（50 MiB 轮转，保留 10 份，最长 90 天）。查看最近 30 天的点击：

```sh
node scripts/website/stats.mjs
node scripts/website/stats.mjs --days 7
node scripts/website/stats.mjs --days 7 --details
```

六个安装包端点为：

| 平台 | 端点 |
|---|---|
| macOS Apple Silicon DMG | `/go/download/macos/arm64/dmg` |
| macOS Intel DMG | `/go/download/macos/x64/dmg` |
| Windows x64 EXE | `/go/download/windows/x64/exe` |
| Linux x64 AppImage | `/go/download/linux/x64/appimage` |
| Linux x64 DEB | `/go/download/linux/x64/deb` |
| Linux x64 RPM | `/go/download/linux/x64/rpm` |

Caddy 在请求时读取 GitHub `releases/latest` 的正式版本标签，再重定向到现有命名规范的对应安装包。发布新版无需重建官网；下载路由使用 `Cache-Control: no-store, max-age=0`。上游异常或无法解析版本时返回 502 页面，提供原地址重试及 Release 入口，禁用 JavaScript 时也可使用。导航与首屏先进入安装包选择区，具体包按钮直接下载。

旧 `/go/download/macos`、`/go/download/linux` 转到对应平台选择区，`/go/download/windows` 转到 x64 EXE，`/go/download/latest` 与 `/go/github` 保留。统计按六种包细分，也继续识别旧端点。统计以独立 IP 近似访客数，NAT 与共享网络可能合并访客；点击总数包含自动化请求，脚本会单独给出非机器人点击数。

`CADDY_BIN=/path/to/caddy python3 scripts/website/test-downloads.py` 启动临时本地 Caddy 与上游夹具，验证版本更新、六包映射、异常与重试、旧入口及日志统计，不修改生产配置或部署网站。

服务运行日志仍可通过 journalctl 查看：

```sh
ssh -o ClearAllForwardings=yes proxyus 'sudo journalctl -u caddy --since "20 minutes ago" --no-pager'
```

## 回滚

部署健康检查失败会恢复先前的站点链接及 Caddy 配置。手动回滚时，先列出 `/var/www/aieyes/releases` 并选择已验证版本，在服务端执行：

```sh
sudo ln -s releases/<已验证版本> /var/www/aieyes/.rollback-next
sudo mv -Tf /var/www/aieyes/.rollback-next /var/www/aieyes/current
```

单纯更换静态目录无需重启 Caddy。若要恢复 Web 服务配置，从对应 `/etc/caddy/aieyes-backups/<版本>` 恢复并运行 `caddy validate`，通过后 reload。完成后再次验证两个语言路径、图片与 HTTP 跳转。
