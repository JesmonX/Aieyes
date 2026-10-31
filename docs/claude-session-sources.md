# Claude 会话状态来源

Claude 非交互调用不进入实时会话列表、状态计数或汇总，因此也不会驱动宠物状态和等待输入／完成提示音。Token、费用等历史用量仍由原有导入流程统计。

来源依据为 Claude 原生日志的 `entrypoint`，以及 Hook 进程继承的 `CLAUDE_CODE_ENTRYPOINT`。本机 Rust 和 SSH Python 观察器均只保留该来源标记，不保留提示词或工具参数。

排除的来源：

- `sdk-cli`：`claude -p` / `claude --print`。
- `sdk-ts`、`sdk-py`：TypeScript / Python SDK。
- `mcp`、`claude-code-github-action`：MCP 服务及 GitHub Action。

普通终端和 IDE 会话继续参与状态统计。缺少来源标记的旧记录保留兼容；若同一会话的另一条日志已确认属于非交互调用，其旧 Hook 投影及关联子会话也一并排除。增量读取和无变化刷新保留已识别的来源。

已识别的非交互日志在文件未变化时不再读取，也不占用每个来源 256 个近期会话的读取名额。大量 `claude -p` 因此不会挤掉交互会话，也不会让来源提示“近期会话过多”、让桌宠显示“状态不全”。首次刷新时，被这类日志占用的名额可以抵扣尚未读取的文件，所以不报超限；文件追加或改为交互续接后重新分类。

验证入口：`cargo test -p aieyes-core --lib claude_source_tests`、`cargo test -p aieyes-core --test session_observer`、`python3 scripts/test-session-observers.py`。覆盖本机／SSH 元数据一致性、大日志首次读取、追加及缓存、旧 Hook 重复记录、子会话、状态汇总和近期会话上限。

`entrypoint` 优先于进程 argv；Hook 只在缺少来源时用祖先进程的 `-p/--print` 兜底。`AIEYES_INTERNAL=1` 下的会话一律排除。通用规则见 `docs/headless-rules.md`。
