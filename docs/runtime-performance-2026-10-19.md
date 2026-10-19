# 0.1.49 运行时性能修复与验证

本轮围绕真实 IPC、重复刷新、缓存失效、慢主机阻塞和会话读取实施修复。基线为提交 `133e7ac` 的 0.1.49 开发版，验证环境为 Intel macOS。所有负载使用隔离 SQLite、合成会话文件和模拟 SSH；这些数字不代表个人数据或真实网络的延迟。

已完成的改动：

- Swift 长寿命 IPC 读取循环逐轮释放 Foundation 临时对象，保留半行拼接、断线和重连行为。
- 主机采样由核心常驻调度器负责：3 个远程 worker 和独立本机 worker，配置与手动刷新立即返回；配置变更、暂停和退出取消旧任务，晚到结果不覆盖新配置，相同配置不重复发布。Swift 配置键使用稳定字段顺序。
- 设置按设置修订缓存，历史已删除账户按设置及用量修订缓存；价格和日聚合复用已解码对象。写事务不发布缓存，回滚及复用修订不会污染后续读取。
- 新增 `dashboard.status`。额度、估值和来源状态变化只合并状态；设置、用量、价格、日期和时区变化才使相关用量快照失效。估值显示使用索引选取每组最新记录及历史有效值，额度协调与自动采样只读取未完成记录。
- 通知复用已有运行时修订，不再独立每 5 秒排空通知队列。Web 刷新保留一次防抖，繁忙期间等待完成通知恢复处理。
- 会话文件确认 EOF 后不再打开；目录发现缓存元数据，并通过 inotify/kqueue 更新。远端复用 SSH/Python 观察进程，保留有界读取、半行续读、轮转和监听失败回退。
- HTTP 客户端按代理和连接策略复用，最多保留 16 个；构建和淘汰释放不持有缓存锁。macOS 直接读取并观察 SystemConfiguration，强制探测先刷新代理再计算缓存键。
- Windows 桌宠复用解码状态，按下一实际帧调度定时器，隐藏或减少动态效果时停止动画计时。

以下为本轮可重复对照，耗时为相同合成夹具下的中位数；SSH 列包含模拟进程启动与 IPC。

| 场景 | 修复前 | 修复后 |
| --- | ---: | ---: |
| 12,000 次 4 KB IPC 回复，第 1,000 至 12,000 次间 RSS 增长 | 241.8 MiB | 0.05 MiB |
| 慢主机采样 6 秒时，快速主机的后续刷新 | 6,119 ms | 193–221 ms；RPC 确认 0.53–0.83 ms |
| 10,000 条已完成采样，每次修改一条后的显示摘要 | 478.0 ms | 12.0 ms |
| 同一历史规模，额度读取使用本地固定结果，无网络 | 2,603.9 ms | 55.7 ms |
| 10,000 个文件，其中 256 个近期文件，未变化轮次 | 698.2 ms | 5.7 ms |
| 上述会话轮次的 `Path.stat` / `Path.open` 次数 | 20,261 / 256 | 258 / 0 |

IPC 内存对照是隔离管道测试，不能当作整应用内存收益。会话文件计数统计 Python 调用，不是全部内核系统调用，也不包括 SSH 传输。测试期间同机运行了构建及其他回归任务。

原生 Swift `AppModel` 使用真实核心、10,000 条用量事件验证：连续 5 次不变刷新没有仪表盘读取；5 次仅额度修订产生 5 次 `dashboard.status`，没有 `dashboard` 或 `dashboard.summary`；价格变更重新读取 2 个范围快照及 1 个菜单摘要。Web 对相同失效分类、防抖和忙碌后恢复也有回归。

40 分钟真实应用长测已完成，含末尾栈采样及清理共 2,404.93 秒，脚本退出码为 0。测试使用独立应用标识、100,000 条合成用量事件和固定代码快照，报告位于 `.local/runtime-ipc-_1puhsj5/`。最初窗口被遮挡，在预热期重新显示过；详细页面随后大部分时间处于遮挡状态，因此应按背景运行场景解读，不能声称完整分析页持续可见。后续补充的代理强制失效和额度协调层优化由最新构建的专项及完整回归覆盖，未包含在此长测快照中。

预热 10 分钟后共记录 180 个样本，以下起止值对应第 600.00 至 2,390.12 秒：

| 指标 | 起始 | 结束 | 峰值 | 增长 |
| --- | ---: | ---: | ---: | ---: |
| UI RSS | 113.04 MiB | 137.68 MiB | 137.73 MiB | 24.64 MiB |
| UI physical footprint | 66.07 MiB | 84.43 MiB | 88.99 MiB | 18.36 MiB |
| 12 个核心进程 RSS 合计 | 167.38 MiB | 176.96 MiB | 176.96 MiB | 9.58 MiB |

UI CPU 按进程累计 CPU 时间差计算，中位数为 0.31%，P95 为 3.39%（100% 表示占满一个逻辑核）。核心进程数量在整个正式观测阶段保持 12 个。第 1,930–1,940 秒出现一次 UI RSS 约 23.72 MiB 的阶跃，当时没有完整仪表盘或菜单用量摘要读取，也没有核心进程增长；之后约 7.5 分钟 RSS 在 137.58–137.73 MiB 间波动。现有计数器和末段 `vmmap` 无法确定该次分配的原因，因此此次长测不能证明整应用无泄漏。末尾 2 秒栈采样中，主线程等待系统事件，IPC 读取线程等待管道数据。

各正式阶段持续 10 分钟，RPC 按回复时间归类：

| 阶段 | 完整 `dashboard` | 菜单 `dashboard.summary` | 轻量 `dashboard.status` |
| --- | ---: | ---: | ---: |
| 用量每 5 秒变化 | 0 | 114 | 2 |
| 仅额度每 5 秒变化 | 0 | 0 | 122 |
| 空闲 | 0 | 0 | 4 |

仅额度变化和空闲阶段没有重读用量快照；来源定期扫描仍会推进状态修订。全程 3,484 次 RPC 回复没有错误，`hosts.monitor.configure` 和 `quotaResetNotifications.take` 各 1 次。会话状态读取仍约每秒一次，中位耗时 1.07 ms；后台修订检查也保留，修复并未取消所有轮询。唯一一次完整仪表盘冷读取耗时 2,997 ms；这些数据不代表持续可见分析页的交互性能。

测试结束后已确认应用、12 个 IPC 记录进程及 12 个核心进程全部退出，隔离偏好域已删除。报告保留原始场景描述与全部样本，另附实际遮挡情况、快照来源和清理核验信息。

验证与范围：

- 最新核心完整测试：427 项通过，4 个子进程夹具按既有规则忽略；生产 Clippy 通过。
- Tauri macOS 测试：26 项通过。会话观察脚本 21 项、流式核心及子进程退出 3 项通过。
- Web 完整测试列表分前段与后段执行；配置向导的模拟保存需同步模拟后端，自定义额度夹具需声明自定义查询命令，已修正后通过。前段日志保留了自定义额度夹具的失败，后段包含该项重跑及其余项目。
- macOS 原生完整回归通过，包含会话独立刷新、窗口显隐、价格缓存、跨日失效、配置并发、更新和桌宠时序；新增 Swift 仪表盘缓存回归已纳入原生测试脚本。
- Core/Web/Swift 的 115 个 RPC 契约一致。最新开发版产物为 `dist/Aieyes.app`；没有安装到 `/Applications` 或发布版本。
- Windows 桌宠共享帧时序测试通过，但本机未执行 Windows/Linux 原生构建或真实 SSH 长测；系统代理仅作只读发现及缓存验证，没有修改用户代理。

主要报告（`.local/` 为本机验证产物，不纳入版本库；汇总数据及复现脚本随代码保存）：

- [IPC 内存对照](../.local/ipc-memory.json)
- [最终热点与慢主机对照](../.local/runtime-regressions-final.json)
- [Swift 真实核心缓存验证](../.local/native-dashboard-cache-xje9q7kj/report.json)
- [真实应用长测](../.local/runtime-ipc-_1puhsj5/report.json)、[运行场景说明](../.local/runtime-ipc-_1puhsj5/notes.json)、[末尾栈采样](../.local/runtime-ipc-_1puhsj5/sample-final.txt)、[末段内存映射](../.local/runtime-ipc-_1puhsj5/vmmap-late-idle.txt)
- [完整核心回归](../.local/performance-core-tests-final.log)、[Clippy](../.local/performance-clippy-final.log)、[Tauri](../.local/performance-tauri-tests-final.log)
- [Web 前段](../.local/performance-web-suite-final.log)、[Web 后段](../.local/performance-web-suite-tail.log)、[原生回归](../.local/performance-native-tests-final.log)、[最新构建](../.local/performance-build-latest.log)

复现命令（先构建当前开发版；长测自动使用隔离数据）：

```sh
AIEYES_OFFLINE=1 sh scripts/build-macos.sh debug
python3 scripts/test-ipc-memory.py --compare-head
python3 scripts/benchmark-runtime-regressions.py --before <修复前核心路径>
python3 scripts/test-dashboard-cache.py
python3 scripts/soak-runtime-ipc.py --app dist/Aieyes.app --duration 2400 --warmup 600
```
