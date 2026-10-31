# Codex 活动会话来源

活动会话的根项只接纳 `session_meta.payload.source` 严格等于 `cli` 或 `vscode` 的 Codex 日志。创建来源保存在内部生命周期状态及文件游标中，本地读取器和 SSH 观察器使用相同规则。

子代理来源对象提供 `subagent.thread_spawn.parent_thread_id`，且同一机器上存在已发现的父会话时，子任务状态归并到父会话。等待状态会影响主会话摘要及桌宠，列表和数量只计算一次，子任务失败不直接判定主会话失败；实际用量继续计入历史。

`exec`、无法连接到可见根会话的子代理、未知值或缺失来源的日志不会作为独立活动会话显示。`originator` 不参与是否显示的判断，普通对话调用 `exec_command` 也不受影响。`originator=codex_work_desktop` 的会话照常显示，但标为 ChatGPT Work（`chatgpt-work`）；会话 ID、父子归并和 hook 去重仍按 Codex 计算，没有 originator 的 Codex hook 以原生日志的归类为准。用量记录的 `provider` 仍为 `codex`，另带 `agent=chatgpt-work`；额度与 Codex 共用同一快照，不单独查询。远程会话与远程历史只额外传 `originator` 这一个标签。已读到明确来源的这类日志在未变化时不再读取，也不占用 256 个近期会话的读取名额；缺失来源的日志仍按未识别处理。

首次读取大日志时独立读取文件头，以免尾部读取遗漏创建来源；后续追加和未变化的文件保留游标中的来源，文件替换时清除旧状态并重新分类。Windows 使用卷与文件 ID 识别替换，避免 NTFS 同名文件沿用创建时间时继承旧来源。来源不暴露到前端，也不改动用量历史、配置格式或其他 Agent 的识别规则。

共享用例位于 `scripts/fixtures/codex-session-sources.json`。Rust 会话测试检查列表、数量、摘要、桌宠、增量缓存及本地/SSH 一致性；`scripts/test-session-observers.py` 检查 Python 投影及大日志、追加、替换行为。

Codex 的规则在 `session_events::classify` 中先于进程 argv 判定，argv 只作兜底（`exec`、`mcp-server`）。Aieyes 自己启动的 `codex app-server` 额度查询带 `AIEYES_INTERNAL=1`，其会话一律排除。其他 Agent 的规则见 `docs/headless-rules.md`。
