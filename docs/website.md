# Aieyes 官网维护

官网：https://aieyes.2h3ngy4n.space ，中文：https://aieyes.2h3ngy4n.space/zh-CN/ 。默认英语，语言由 URL 决定；切换语言保留章节，平台选择在当前浏览器会话中保留。

## 内容与构建

源码位于 `apps/website`：`content.mjs` 管理中英内容，`style.css` 和 `site.js` 管理外观及轻量交互，`public` 保存部署素材。静态生成器共用页面结构，无网站运行时依赖，不需要在服务器安装 Node。

在仓库根目录执行：

```sh
node scripts/website/build.mjs
node scripts/website/test.cjs
```

输出位于 `dist/website`。测试使用桌面项目已有的 Playwright，检查中英页面、全部图片、README 本地链接、平台切换、放大与焦点恢复、移动端菜单、FAQ、锚点和窄屏布局；视觉验收截图位于 `.local/website/qa`。可用本地静态服务器预览此目录。

部署后可使用 `AIEYES_WEBSITE_ORIGIN=https://aieyes.2h3ngy4n.space node scripts/website/test.cjs` 对正式域名运行同一套浏览器检查，覆盖线上资源与 CSP 下的交互。

## 重建展示图

```sh
node scripts/website/encode.cjs --pet
python3 scripts/website/render.py
node scripts/website/render-windows.cjs
node scripts/website/og.cjs
node scripts/website/build.mjs
node scripts/website/test.cjs
```

macOS 渲染要求本机图形会话、Xcode/Swift 和已有的 `.build/swift/debug/Sparkle.framework`。首次可运行项目现有 macOS 构建来准备依赖。Windows 展示使用项目实际前端在 Chromium 中渲染。原始 PNG、预览源码、临时应用及隔离数据只写入 `.local/website`；优化后的展示素材位于 `apps/website/public`。

`fixture.py` 提供固定的合成账户、用量、服务器、任务与会话数据，不读取个人配置，不联系模型服务商。渲染使用产品共用的英文词库与真实语言设置，不再替换预览副本中的中文。页面明确标注演示数据；Windows 图片不作为实机材质验收记录。仅保留总览、分析、服务器及面板四类截图，其他功能通过文字与动画呈现。

桌宠动画直接使用原有 8 × 9 帧图集，遵守减少动态效果的系统偏好。内置月薪喵素材来自项目既有资源，作者为 **kiffin**，页面保留来源署名。Provider 图标复用项目既有接入素材。分享图由实际产品界面与网页排版生成。

## 部署

目标为本机 SSH 别名 `proxyus`。脚本禁用此别名附带的端口转发，沿用已有 SSH 密钥和主机校验；首次部署安装 Ubuntu 仓库中的 Caddy。需要目标用户可使用非交互式 sudo，以及域名已指向主机、外部 TCP 80/443 可达。

```sh
sh scripts/website/deploy.sh
```

脚本先构建与验证，再上传静态输出到 `/var/www/aieyes/releases/<UTC 时间>`，验证 Caddy 配置后原子切换 `current` 链接。Caddy 通过 systemd 常驻，管理证书续期及 HTTP → HTTPS 跳转。站点独立配置为 `/etc/caddy/sites/aieyes.caddy`，通过 import 接入主配置，不覆盖其他站点。`/etc/caddy/aieyes-global-options` 将 TCP 443 的协议限定为 HTTP/1.1 与 HTTP/2，保留服务器已有的 Hysteria UDP 443；全局块通过 import 合并。配置备份仅保存在服务端 `/etc/caddy/aieyes-backups`，不对外提供。

部署后分别核对公网英文首页和中文页，保留最近三份版本。页面使用较短资源缓存，不引入统计脚本、网站账户或数据库。日志使用 systemd journal：

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
