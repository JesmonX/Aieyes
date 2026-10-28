# Claude 会话状态来源

Claude 非交互调用不进入实时会话列表、状态计数或汇总，因此也不会驱动宠物状态和等待输入／完成提示音。Token、费用等历史用量仍由原有导入流程统计。

来源依据为 Claude 原生日志的 `entrypoint`，以及 Hook 进程继承的 `CLAUDE_CODE_ENTRYPOINT`。本机 Rust 和 SSH Python 观察器均只保留该来源标记，不保留提示词或工具参数。

排除的来源：

- `sdk-cli`：`claude -p` / `claude --print`。
- `sdk-ts`、`sdk-py`：TypeScript / Python SDK。
- `mcp`、`claude-code-github-action`：MCP 服务及 GitHub Action。

普通终端和 IDE 会话继续参与状态统计。缺少来源标记的旧记录保留兼容；若同一会话的另一条日志已确认属于非交互调用，其旧 Hook 投影及关联子会话也一并排除。增量读取和无变化刷新保留已识别的来源。

验证入口：`cargo test -p aieyes-core --lib claude_source_tests`、`cargo test -p aieyes-core --test session_observer`、`python3 scripts/test-session-observers.py`。覆盖本机／SSH 元数据一致性、大日志首次读取、追加及缓存、旧 Hook 重复记录、子会话和状态汇总。
