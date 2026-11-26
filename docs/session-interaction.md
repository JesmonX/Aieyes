# 桌宠会话交互契约

`sessions.interaction.*` 是桌宠弹窗与核心之间的接口。类型定义在
`crates/core/src/session_interaction/contract.rs`，各 Agent 适配器把自己的原生请求转换成这里的结构；
两端 UI 只按本页字段渲染，不需要识别具体 Agent。

安全规则只有一条：回答只送达它所针对的那个请求。无法核验的路径一律返回 `blocked`，不做猜测。

## 方法

| 方法 | 作用 |
| --- | --- |
| `sessions.interaction.status` | 读取开关、Hook 安装状态和 Codex 控制接口是否存在 |
| `sessions.interaction.configure` | 修改开关 |
| `sessions.interaction.inspect` | 读取某个会话当前的卡片 |
| `sessions.interaction.respond` | 回答卡片、选择审批决定或发送后续指令 |
| `sessions.interaction.handoff` | 把 Claude / Qoder Hook 挂起的请求交还给原终端 |

## inspect

参数：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `sessionId` | string，必填 | 会话标识 |
| `capabilities.localTerminal` | bool，可选 | 调用方能自己把 `terminalJob` 输入到原终端（目前只有 macOS 原生端）。缺省为 `false` |
| `requestId` | string，可选 | OpenCode：显示 `pendingRequests` 中的这一条；缺省或该请求已不存在时显示最早的一条 |

返回值的所有字段始终存在：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `sessionId`、`stateKey`、`promptHash` | string | 回答时原样带回。`stateKey` 标识会话当前的等待状态，`promptHash` 标识具体请求 |
| `kind` | string | `none`、`approval`、`questions`、`text`、`followup`、`readonly` |
| `adapter` | string | `claudeHook`、`codex`、`opencode`、`tmux`、`macosTerminal`、`focusOnly` |
| `transport` | string | `local` 或 `ssh` |
| `canRespond`、`canHandoff` | bool | 能否回答、能否交还终端 |
| `requestId` | string | Claude / Qoder Hook 请求或 OpenCode 请求的标识，其余为空 |
| `pendingCount` | number | 该会话排队中的请求数，含当前这一条；无请求时为 0 |
| `pendingRequests` | array | OpenCode：排队中的全部请求（最早的在前，最多 50 条），每项 `{requestId, kind, title}`，`kind` 为 `approval`、`questions` 或 `readonly`。把某项的 `requestId` 传给 inspect 即可切换到该请求。其余适配器为空数组 |
| `requestedAt` | number | Hook 请求登记的 Unix 秒；未知为 0 |
| `inputKind` | string | 会话在等什么：`approval`、`elicitation`、`question`、`notification` 或空 |
| `answered` | bool | 该请求刚刚已在弹窗回答，会话状态尚未跟上 |
| `title` | string | 会话标题 |
| `approval` | object 或 null | 审批卡片，见下 |
| `questions` | array | 问题列表，见下 |
| `form` | object 或 null | 表单级信息，见下 |
| `textPrompt`、`promptSource` | string | 提示文本及其来源：`question`（读取到的待答问题）、`lastMessage`（Agent 最后一条回复的结尾，最多 1600 字节）、`none` |
| `followUp` | object | `{available, adapter, mode, reason, canInterrupt}`，见“后续指令” |
| `needsClaudeHook`、`needsClaudeApprovals`、`quickEnable` | bool、bool、string | 开启对应 Hook 后该请求可在弹窗处理。`quickEnable` 为 `questions`、`approvals` 或空 |
| `readonlyReason` | string | 不能回答时的本地化原因 |

`kind` 的含义：

- `approval`：`approval` 非空。`decisions` 为空数组时只能查看，需回原窗口确认。
- `questions`：`questions` 非空。`canRespond` 为 `false` 时只读。
- `text`：等待自由文本，经 tmux 或 macOS 终端输入。
- `followup`：本轮已结束，可以发送下一条指令；`followUp.mode` 为 `queue` 时会话仍在执行，指令会排队。
- `readonly`：有等待但没有可回答的接口。
- `none`：没有等待。`answered` 为 `true` 时表示刚提交、等待原会话继续。

### approval

```json
{
  "toolName": "Bash",
  "reason": "Run the tests",
  "command": "cargo test",
  "details": "/repo",
  "kind": "command",
  "title": "",
  "cwd": "/repo",
  "plan": {"markdown": "# Plan\n…", "truncated": false},
  "changes": [{"path": "src/a.rs", "kind": "update", "movePath": "", "diff": "", "oldText": "…", "newText": "…", "truncated": false}],
  "paths": ["/repo/src"],
  "decisions": [
    {"id": "approve", "label": "允许本次", "labelKey": "允许本次", "style": "primary", "behavior": "allow", "scope": "once", "acceptsReason": false, "interrupts": false},
    {"id": "decline", "label": "拒绝", "labelKey": "拒绝", "style": "danger", "behavior": "deny", "scope": "once", "acceptsReason": true, "interrupts": false}
  ]
}
```

- `toolName`、`reason`、`command`、`details`、`kind`、`decisions` 始终存在。`title`、`cwd`、`plan`、`changes`、`paths` 为空时省略；`changes` 各项里的 `movePath`、`diff`、`oldText`、`newText` 为空时省略。
- `kind`：`command`、`fileChange`、`plan`、`permissions`、`mcpTool`、`tool`、`generic`。
- `plan.markdown` 是 Agent 原样写出的 Markdown，最长 12000 字节。
- `changes[].kind`：`add`、`update`、`delete`、`move`、`unknown`。单个文本字段最长 6000 字节，整张卡片合计 24000 字节，超出时 `truncated` 为 `true`。最多 40 个文件、50 条路径。
- `decisions[]`：
  - `id`：回答时作为 `action` 发送。约定的取值为 `approve`、`approveSession`、`approveAlways`、`decline`、`declineInterrupt`、`cancel`；适配器可以增加其他 id，UI 按 `label`、`style` 渲染即可。
  - `label` 已按应用语言本地化，`labelKey` 是简体中文源文案。
  - `style`：`primary`、`secondary`、`danger`。
  - `behavior`：`allow`、`deny`、`cancel`。
  - `scope`：`once`、`turn`、`session`、`always`。
  - `acceptsReason`：为 `true` 时可以随决定发送 `reason`。
  - `interrupts`：该决定同时中断当前轮次。
  - `description`：可选的补充说明。

只列出原请求确实提供、且适配器能原样回传的决定。每个决定在后台保存着要发回原会话的原生回复，UI 只发送 `id`。

Codex 的决定按请求类型生成（依据 codex-cli 0.172.0 生成的协议定义和实际请求）：

| 原生请求 | `kind` | 决定 |
| --- | --- | --- |
| `item/commandExecution/requestApproval` | `command` | `availableDecisions` 中的 `accept` → `approve`，`acceptForSession` → `approveSession`，`acceptWithExecpolicyAmendment` 对象 → `approveAlways`（`description` 显示将被免询问的命令前缀），`applyNetworkPolicyAmendment` 对象 → `approveHost` / `declineHost`（`description` 为主机名，同类多个时为 `approveHost.2` …），`cancel` → `cancel`（文案「拒绝并中断」，`interrupts: true`）。`decline` 始终提供。对象形式的决定原样回传；含控制字符、结构不认识或为空的对象不出现。未列出 `availableDecisions` 的旧版本只有 `approve`、`decline`。`kind` 为 `writeStdin`、`humanCommand` 的请求没有决定 |
| `item/fileChange/requestApproval` | `fileChange` | `approve`、`approveSession`、`decline`、`cancel`。`changes` 取自同一 `itemId` 的补丁（先用连接上收到的 `item/started`，否则读会话记录中 `call_id` 相同的那次调用）；找不到时不显示其他调用的文件。`grantRoot` 在 `paths` 中 |
| `item/permissions/requestApproval` | `permissions` | `approve`（`scope: "turn"`，文案「本轮内允许」）、`approveSession`、`decline`（授予空权限）。没有 `cancel` |
| `mcpServer/elicitation/request`，表单模式且没有字段（MCP 工具调用确认） | `mcpTool` | `approve`、`_meta.persist` 列出时的 `approveSession` / `approveAlways`、`decline`、`cancel` |
| `execCommandApproval`、`applyPatchApproval`（旧协议） | `command` / `fileChange` | `approve`、`approveSession`、`decline`（`acceptsReason: true`）、`cancel` |

`decisions` 为空的审批卡片 `canRespond` 为 `false`，`readonlyReason` 为「此审批需要在原窗口确认」。除旧协议的 `decline` 外，Codex 的决定都不接受 `reason`。

### questions

```json
{
  "id": "1",
  "key": "",
  "header": "Database",
  "question": "Which database should I use?",
  "kind": "string",
  "multiSelect": false,
  "allowCustom": true,
  "secret": false,
  "required": true,
  "options": [{"label": "Postgres", "description": "…", "preview": "…", "value": "pg"}]
}
```

- `kind`：`string`、`multiselect`、`boolean`、`integer`、`number`。
- `allowCustom`：能否在选项之外自行填写。没有选项时恒为 `true`。
- `secret`：输入需要遮盖，且不应回显。
- `description`、`placeholder`、`default`、`constraints`（`minimum`、`maximum`、`minLength`、`maxLength`、`pattern`、`format`）仅在适配器提供时出现。
- 选项的 `description`、`preview`、`value` 仅在存在时出现。

### form

问题来自表单或 MCP 请求时非空：

```json
{"title": "", "description": "", "source": "mcp", "serverName": "github", "url": "", "canCancel": true, "canSkip": false}
```

`source` 为 `mcp`、`tool` 或 `agent`。只有 `canCancel` / `canSkip` 为 `true` 时，`cancel` / `skip` 才会被接受。

Codex 的 `mcpServer/elicitation/request` 表单（平铺字段）是 `kind: "questions"` 的卡片，`inputKind` 为 `elicitation`：字段按必填在前排列，`answer` 提交为 `{"action":"accept","content":{…}}`（枚举显示名换回取值，数字和布尔保持类型，多选为数组），`skip` 为拒绝，`cancel` 为取消。URL 模式、厂商自定义模式和带嵌套或条件字段的表单是 `kind: "readonly"` 的卡片，`form.url` 给出需要打开的地址，`readonlyReason` 为「此表单包含条件或外链字段，请回到原窗口处理」。Codex 的 `item/tool/requestUserInput` 问题带 `header`，`isOther` 对应 `allowCustom`，`isSecret` 对应 `secret`；多选的每个选项作为单独的答案提交，不合并成一个字符串。

## respond

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `sessionId` | string，必填 | |
| `stateKey` | string，必填 | inspect 返回的值。缺失或已变化时返回 `blocked` |
| `promptHash` | string | inspect 返回的值。除远程终端文本外，所有回答都要求与当前请求一致 |
| `action` | string | 见下。缺省为 `answer` |
| `decisionId` | string | 仅与 `action: "decide"` 一起使用 |
| `requestId` | string，可选 | 指定回答哪一条 Claude / Qoder Hook 请求或 OpenCode 请求；缺省为最早的一条。`promptHash` 必须是这一条的 |
| `answers` | object | `{问题 id: string 或 string[]}`。最多 16 题，每题最多 32 个值，每个值最长 2000 字节 |
| `text` | string | 自由文本，最长 8192 字节 |
| `reason` | string | 给 Agent 的说明，最长 2000 字节，仅当所选决定的 `acceptsReason` 为 `true` |
| `capabilities.localTerminal` | bool | 同 inspect |

`action` 的取值：

- `answer`：回答问题或自由文本。会话已结束时按后续指令处理（保持旧行为）。
- `followup`：发送后续指令。会话仍在等待回答时拒绝。
- `cancel`：审批卡片上等同于选择 `cancel` 决定；问题卡片上仅当 `form.canCancel`。
- `skip`：仅当 `form.canSkip`。
- `interrupt`：中断正在执行的本轮，仅当 `followUp.canInterrupt`。不需要 `promptHash`；会话在等待回答时也可使用，原会话会同时撤销未处理的请求。
- `decide` 加 `decisionId`，或直接把决定的 id 写在 `action` 里（`approve`、`decline` 等）。`accept`、`deny` 仍作为 `approve`、`decline` 的别名。

校验在送达前完成，失败时返回 `blocked` 且不改动任何状态：

- 决定必须出现在当前卡片的 `decisions` 中；不接受说明的决定带了 `reason` 会被拒绝。
- `answers` 的键必须是当前问题的 id，必答题不能缺；单选题只能有一个值；`allowCustom` 为 `false` 时值必须是某个选项的 `label` 或 `value`。
- 文本、说明和答案不能包含控制字符。

返回值：

```json
{"status": "confirmed", "message": "…", "receipt": {"adapter": "codex"}}
```

`status`：

- `confirmed`：原会话已确认收到。
- `submitted`：已提交，尚未确认。
- `blocked`：未提交，`message` 说明原因。
- `uncertain`：可能已提交，需要回原窗口核对；不要重发。
- `needsLocalTerminal`：附带 `terminalJob: {text, session}`，由调用方输入到原终端。只会发给声明了 `capabilities.localTerminal` 的调用方；其他调用方得到 `blocked`。

## 终端输入的限制

向原终端输入“文本加回车”只用于自由文本，inspect 和 respond 使用同一组条件：

- 会话处于等待输入或本轮已结束；执行中或状态未知时拒绝。
- `inputKind` 为 `approval` 或 `elicitation` 时拒绝。权限对话框和 MCP 表单不能用文本回答，这类等待显示为没有按钮的审批卡片或只读卡片。
- `inputKind` 为 `question` 时拒绝：提问工具在各 Agent 的终端里都是选项菜单，读不到选项时卡片只读并说明原因。
- 会话记录里有未回答的选项题或审批时拒绝。
- Qoder 桌面版没有终端，拒绝。
- 需要开启终端输入，会话状态在 20 秒内更新过，等待不是来自子 Agent，并且能唯一定位到 tmux 分屏或 macOS 终端标签页。

本机文本卡片的 `promptHash` 绑定当时显示的问题，问题变化后旧卡片的回答被拒绝。SSH 会话由远程脚本做同样的检查。

本机 Kiro CLI 3.4.0 起可在桌宠快捷交互设置里单独开启「在弹窗中审批 Kiro CLI 工具权限」。新开的 V3 会话通过 Kiro 的 `PermissionRequest` Hook 显示审批，浮窗支持允许或拒绝本次操作。选择回原终端、应用退出或等待 120 秒后，Kiro 继续显示原审批菜单。长期规则与 Kiro 限制由用户亲自处理的审批在原终端操作。

未接入 Hook 的 Kiro 会话，以及 DeepSeek Harness、Antigravity、Qoder 桌面版、Kimi、Grok、Copilot CLI、CodeBuddy、WorkBuddy 的选项题和审批只读展示，开启终端输入也不会变成可回答：
各终端界面的选项菜单按键不同（实测 Kimi 的审批用回车和 Esc、提问菜单用数字和方向键，Grok 的审批是四项单选菜单），而且终端画面无法和记录中的请求逐一对应，不能确认按键会落在预期的请求上。
这些卡片的 `adapter` 为 `focusOnly`，`canRespond` 为 `false`，`readonlyReason` 说明需要回原窗口，跳转由客户端的聚焦功能完成。

只读卡片的内容来自各 Agent 自己的会话记录，`inputKind` 随读到的内容设为 `approval` 或 `question`：

- 审批：`approval.toolName`、`reason`、`command`（命令、文件路径或地址）、`details`（写入内容或修改前后）、`kind`（`command`、`fileChange` 或 `tool`）、`cwd`（记录里有时），`decisions` 为空。
- 提问：`questions` 中每题带 `question`、`options`（`label`、可选的 `description`）和 `multiSelect`。

SSH 会话由远程脚本用同样的规则读取（Kimi、Grok、Copilot CLI、CodeBuddy、WorkBuddy），以 `structuredApproval` / `structuredQuestions` 返回，卡片与本机一致。各 Agent 的记录位置和核对过的版本见 [chat-channels.md](chat-channels.md)。

## 后续指令

`followUp`：

```json
{"available": true, "adapter": "codex", "mode": "newTurn", "reason": "", "canInterrupt": false}
```

- `mode`：`newTurn`（原生接口在同一会话开启下一轮）、`queue`（会话仍在执行，指令排队到本轮结束后执行）或 `terminal`（输入到原终端）。
- `canInterrupt`：可以用 `action: "interrupt"` 中断本轮。目前只有执行中或等待中的本机 OpenCode 会话为 `true`。
- 除 OpenCode 外，本轮结束后才可用：执行中的会话 `available` 为 `false`，`reason` 说明需等待本轮结束。
- 可用时 inspect 返回 `kind: "followup"`、`canRespond: true`，`textPrompt` 为最后一条回复的结尾。
- 各适配器：Codex / ChatGPT Work 需要控制接口；OpenCode 需要会话所在的服务可连接（见“OpenCode”）；其余 Agent 走终端输入条件；SSH 上的 Codex 由远程脚本核验。

## Claude / Qoder Hook 请求

- 同一会话有多条挂起请求时按登记顺序处理，最早的先显示，与终端里对话框的顺序一致。`pendingCount` 给出总数。
- Hook 超过 5 秒没有心跳的请求不再显示，也不能回答。
- 没有观察 Hook 的 CLI（例如 Qoder）在工具审批期间仍记录为执行中。只要交互 Hook 已收下请求，会话会显示为等待输入，`inputKind` 为 `approval`、`question` 或 `elicitation`。Hook 进程退出但请求尚未超时的几秒内，这个等待可能多显示一会儿。
- Hook 取走答复时立即清除该请求的工具输入、附带信息和答案，只保留标识用于回执；未被取走的请求由定时清理兜底。
- 终端里的对话框与弹窗同时有效。Hook 跟随会话记录，请求在终端里被处理、会话开始新一轮或 Agent 进程退出后，Hook 自行退出，卡片随之消失。子 Agent 发起的请求不在主会话记录中，只能等到答复、交还或超时。
- Hook 输入上限 32 MiB。超过 256 KiB 的工具输入在卡片中截短显示（单个字符串最多 16 KiB，`changes[].truncated` 为 `true`），批准时仍使用完整的原始输入。
- 无人值守运行（`-p` / `--print`）不接管；会话记录路径、会话标识与来源目录对不上的请求不接管。

### 审批卡片

`adapter` 为 `claudeHook`。决定来自 CLI 在 `permission_suggestions` 中给出的建议，只保留可核对的三类，最多三条：

| 决定 `id` | 含义 | 出现条件 |
| --- | --- | --- |
| `approve` | 允许本次；计划卡片上为「批准计划」，并把会话切到 `default` 模式（见下） | 始终 |
| `approveAlways` | 允许并写入建议的允许规则（用户、项目或本地设置） | 建议为 `addRules` 且 `behavior` 为 `allow`，目标不是本会话 |
| `approveSession` | 同上，仅本会话 | 目标为本会话 |
| `approveAcceptEdits` | 允许并把会话切到自动接受编辑（`acceptEdits`） | 建议为 `setMode` `acceptEdits`、目标为本会话；计划卡片始终提供 |
| `approveDirectories` | 允许并加入建议的工作目录 | 建议为 `addDirectories` |
| `decline` | 拒绝，可附说明；计划卡片上为「继续修改计划」 | 始终 |
| `declineInterrupt` | 拒绝并中断本轮 | Claude、Qoder |

同类建议有多条时 `id` 带 `.2`、`.3` 后缀。提升为 `bypassPermissions` 等其他模式、拒绝规则、删除规则的建议不展示。决定的原生载荷由核心按已知字段重建，界面只回传 `id`。

弹窗的任何决定能设置的权限模式只有 `default` 和 `acceptEdits` 两种，且只作用于本会话；`bypassPermissions`、`auto`、`dontAsk`、`plan` 以及写入设置文件的模式变更都不会发出（单元测试 `no_decision_switches_to_a_mode_without_permission_checks` 覆盖所有提供方和工具）。

- 计划（`ExitPlanMode`）：`kind` 为 `plan`，正文是计划全文，`allowedPrompts` 列在详情中。批准时回传原始输入，因为 CLI 不接受不带 `updatedInput` 的批准。
  - Claude / Qoder 的两个批准决定都带 `setMode`：`approve` 为 `default`（`description` 为「后续操作仍需逐项确认」），`approveAcceptEdits` 为 `acceptEdits`。不带 `setMode` 的批准会让 CLI 回到进入计划模式之前的模式；终端对话框会写明这个模式（例如「Yes, and use auto mode」），卡片无法得知，所以不提供。实测 Claude Code 2.1.332：从 auto 模式进入计划的会话，不带 `setMode` 批准后回到 auto，后续命令不再询问；带 `setMode: default` 后每条命令照常弹出审批。
  - 计划文件没有写到 CLI 指定的路径时，CLI 发来的 `tool_input` 为空对象，终端只问是否退出计划模式。此时卡片没有 `plan`，`reason` 为「请求退出计划模式，未附计划内容」，批准时回传 `updatedInput: {}`。
  - `decline` 的说明作为工具结果交给 Agent，会话留在计划模式。
- 文件修改（`Edit`、`Write`、`MultiEdit`、`NotebookEdit`）：`kind` 为 `fileChange`，标题为「工具 · 文件名」，`changes` 给出路径、新增或修改以及差异。
- MCP 工具：`kind` 为 `mcpTool`，标题为「服务器 · 工具」，详情逐行列出参数。
- 卡片带 `cwd`。回执带 `decisionId`。
- 带 `mcp__` 前缀的工具由 CLI 决定是否采用 Hook 的允许，这类 `approve` 的回执 `status` 为 `submitted` 而不是 `confirmed`。除计划和提问外，需要用户在终端交互的内置工具不能从桌宠批准。
- 用户已处理的卡片在短时间内不再重复弹出；该标记绑定当时的 `promptHash` / `requestId`，会话离开该等待状态后即清除，新的请求不受影响。
- Claude 本地审批的 Hook 已写出有效答复时，下一次会话轮询会按来源、原生会话和工具调用清除旧等待，即使工具仍未返回。交还终端、可能被 CLI 忽略的 MCP 允许，以及晚于答复的新审批仍由原终端状态决定。

### 实测记录

以下在 Claude Code 2.1.332 的交互会话中逐项验证（临时配置目录和数据目录，经 `sessions.interaction.inspect` / `respond` 驱动，Hook 的输入输出由包装脚本记录）：

| 请求 | 弹窗操作 | Hook 输出的 `decision` | CLI 的行为 |
| --- | --- | --- | --- |
| `AskUserQuestion` | `answer` | `allow` + `updatedInput`（原输入加 `answers`） | 工具结果为所选答案，终端菜单关闭 |
| `Bash` | `approve` | `allow` | 命令执行 |
| `Bash` | `decline` 附说明 | `deny` + `message` | 命令未执行，说明原样成为工具结果 |
| `Bash` | `approveAlways` | `allow` + `updatedPermissions`（`addRules`，`localSettings`） | 命令执行，规则写入项目的 `.claude/settings.local.json`，同一命令再次执行不再询问 |
| `Bash` | `declineInterrupt` | `deny` + `interrupt: true` | 命令未执行，本轮中断 |
| `ExitPlanMode` | `approve` | `allow` + `updatedInput` + `setMode: default` | 计划获批，模式变为 `default`，后续写文件和命令照常询问 |
| `ExitPlanMode` | `approveAcceptEdits` | `allow` + `updatedInput` + `setMode: acceptEdits` | 计划获批，写文件不再询问，命令仍询问 |
| `ExitPlanMode` | `decline` 附说明 | `deny` + `message` | 说明成为工具结果，会话留在计划模式 |
| MCP 工具及其表单 | `approve`，然后 `answer` | `allow`；`action: "accept"` + `content` | 工具执行，MCP 服务器收到填写内容 |

Hook 输入里没有 `tool_use_id`，调用标识取自会话记录；计划模式下的请求不带 `permission_suggestions`。Qoder、CodeBuddy、ZCode 没有做同样的实测。

### MCP 表单

开启提问 Hook 后，Claude / Qoder CLI 的 `Elicitation` Hook 一并安装。表单显示为问题卡片，`method` 为 `aieyes/claudeElicitation`，`form` 为 `{title, description, source: "mcp", serverName, canCancel: true, canSkip: true}`，字段顺序与 MCP 服务器给出的一致。每题带 `key`、`kind`（`string`、`number`、`integer`、`boolean`、`multiselect`）、`required`、`default`、`constraints`，选项带 `label` 和 `value`。

- `answer` 提交填写内容（accept），核心按类型和约束校验后转换；`skip` 表示拒绝提供（decline）；`cancel` 表示关闭对话框（cancel）。
- 只接管表单模式且字段为字符串、枚举、数字、布尔或枚举多选的平铺结构，最多 12 个字段、每个字段 50 个选项、结构 64 KiB。URL 模式、嵌套对象等留在终端，卡片只读。
- 未安装 Hook 时 inspect 返回 `needsClaudeHook` 和 `quickEnable: "questions"`。
- Hook 运行期间 CLI 不显示自己的表单，因此每次最多接管 120 秒（Hook 超时 135 秒），之后交还终端。

### CodeBuddy 与 ZCode

开启弹窗审批后，为本机 CodeBuddy 和 ZCode 来源安装 `PermissionRequest` Hook（ZCode 写入 `cli/config.json` 的 `hooks.events`，且仅在其 `hooks.enabled` 为 `true` 时安装）。

- 只有 `approve` 和 `decline`（可附说明）；两者的 hook 输出都不支持中断本轮，因此不提供 `declineInterrupt`。没有「始终允许」、计划回填、提问和 MCP 表单。
- 这两个 CLI 在 Hook 运行期间不显示自己的对话框，所以每次最多等待 120 秒，超时后交还终端。
- 两者的 Hook 字段根据已安装的程序核对并由单元测试覆盖，没有在真实会话中验收。

### 到不了 Hook 的提示

以下提示不触发 `PermissionRequest`，只能在原窗口处理，卡片只读：沙箱网络访问确认、工作区信任、登录与 MCP 服务器授权、安装 Hook 之前已经弹出的对话框。

### handoff

参数 `sessionId`（必填）和 `requestId`（可选，缺省为最早的一条）。只交还属于该会话且仍在等待的请求。

```json
{"handedOff": true, "confirmed": false, "requestId": "…", "message": "…"}
```

`confirmed` 为 `false` 表示已写入交还标记，Hook 尚未取走（通常一秒内完成）。交互关闭、会话不存在或请求不属于该会话时 `handedOff` 为 `false`。

## status

在原有字段之外增加 `codexControl: [{sourceId, available}]`：每个本机 Codex 来源目录下是否存在
`app-server-control/app-server-control.sock`。只检查路径，不建立连接。没有该接口时 Codex 的问题和审批只读。

## 会话列表

`sessions.list` 的每个会话在等待输入时带 `inputKind`（取值同上，未知时省略）。只包含类别，不包含提示内容。

## 适配器现状

| 会话 | 审批决定 | 问题 | 后续指令 |
| --- | --- | --- | --- |
| Claude / Qoder CLI（本机，Hook） | `approve`、CLI 建议的 `approveAlways` / `approveSession` / `approveAcceptEdits` / `approveDirectories`、`decline`（可附说明）、`declineInterrupt`；计划、文件修改、MCP 工具有各自的 `kind` | 单选、多选、自定义；MCP 表单可填写、拒绝或取消 | 终端 |
| CodeBuddy / ZCode（本机，Hook） | `approve`、`decline`（可附说明） | 只读，或自由文本经终端 | 终端 |
| Codex / ChatGPT Work（本机） | 见上文 Codex 决定表：`approve`、`approveSession`、`approveAlways`、`approveHost` / `declineHost`、`decline`、`cancel`；命令、文件修改、权限、MCP 工具各有 `kind` | 结构化问题（多选按列表提交）；MCP 表单可填写、拒绝或取消 | `newTurn` |
| Codex（SSH） | 与本机相同 | 与本机相同；异步提问仍按文本回答 | 远程脚本核验 |
| OpenCode（本机服务） | `approve`、`approveAlways`（请求带可保存规则时）、`decline`（可附说明）；命令和文件修改有各自的 `kind`，文件修改带差异 | 表单可填写或取消；可选字段可留空 | `newTurn`；执行中为 `queue`，并可 `interrupt` |
| Claude / Qoder（SSH） | 只读 | 选项题只读，自由文本经 tmux | tmux |
| Kimi、Grok、Copilot CLI、CodeBuddy（提问）、WorkBuddy（本机和 SSH） | 只读，显示工具和命令 | 选项题只读并列出选项；无提问工具的自由文本经终端 | 终端（WorkBuddy 无） |
| Kiro CLI（本机） | 3.4.0 起可单独启用 V3 Hook，允许或拒绝本次；其他审批显示工具和对象 | 选项题只读并列出选项 | 终端 |
| DeepSeek Harness、Antigravity、Qoder 桌面版（本机） | 只读，显示工具和命令 | 选项题只读并列出选项 | 终端（Qoder 桌面版无） |
| OpenCode `--standalone` 或 SSH | 只读，无内容 | 只读，无内容 | 终端 |
| 其他 Agent（Cursor、Gemini、Cline、Kilo、Pi、OMP、ZCode 的提问等） | 只读，无内容 | 提问工具只读且无选项内容；普通文本提问显示最后一条回复，经终端回答 | 终端 |

同一 Codex 会话有多个待处理请求时按到达顺序逐个显示（与原窗口一致），`pendingCount` 为总数。MCP 请求和旧协议审批可以不带 turn，仍只匹配同一 thread。

SSH 上的 Codex：inspect 时远程脚本返回原始请求（文件修改另带同一调用的补丁），卡片由后台用与本机相同的代码生成。respond 时后台重新读取远程请求、核对 `promptHash`，据此生成原生回复后交给远程脚本；脚本再次找到该请求、核对指纹和回复形状（决定必须是请求列出的，权限只能是请求的或空，表单字段只能是请求里的），然后原样发送。任何一步不符都返回 `blocked`。

## OpenCode

按 OpenCode 2.1.0 的服务接口实现，真实响应样本在 `crates/core/tests/fixtures/opencode-v2.1.0/`。

- 服务来源：该安装的 `state/opencode/service.json`（`opencode serve --service`）；此外查找当前用户名下监听本机回环地址的 `opencode serve` 进程（`ps` 加 `lsof`，仅 Unix，结果缓存 15 秒），口令只取 Aieyes 自身环境变量 `OPENCODE_PASSWORD`，不读取其他进程的环境。口令只发往 `127.0.0.1`、`localhost`、`::1` 的明文 HTTP 地址。服务必须确认拥有该会话才会被使用；用随机口令手动启动的服务无法连接，保持只读。
- 排队：同一会话的权限请求和表单按创建顺序合并，最早的先显示，`pendingCount` 为总数，`pendingRequests` 列出全部。respond 时重新读取队列、按 `requestId` 找到请求并核对 `promptHash`；请求已消失或内容不同返回 `blocked`，不会落到另一条请求上。
- 审批：`shell` 显示为 `toolName: "Shell"`、`kind: "command"`；`edit` 为 `"Edit"`、`kind: "fileChange"`，`changes` 带路径、`add` / `update` / `delete` 和差异；其他动作用原动作名，`kind: "tool"`。决定为 `approve`（`once`）、`approveAlways`（`always`，仅当请求带 `save`，`description` 逐行列出将免询问的规则）、`decline`（`reject`，`reason` 作为 `message` 交给 Agent）。
- 表单：`form` 为 `{title, description: "", source: "tool" 或 "agent", serverName: "", url, canCancel: true, canSkip: false}`。问题带 `header`（表单标题）、`required`（字段声明必填时才为 `true`）、`placeholder`、`default` 和 `constraints`（`minimum`、`maximum`、`minLength`、`maxLength`、`minItems`、`maxItems`、`pattern`、`format`）。布尔字段的选项按界面语言显示“是 / 否”，`value` 为 `true` / `false`。可选字段留空时不提交该字段，至少要回答一题。`cancel` 撤销表单（Agent 收到“已取消”）；OpenCode 没有跳过，`skip` 被拒绝。带显示条件的字段和外链表单只读。
- 服务拒绝回答（400）时 `message` 带上服务给出的原因；口令失效（401）单独提示。
- 后续指令：本轮结束后为 `newTurn`；执行中为 `queue`（以 `delivery: "queue"` 提交，回执带 `delivery: "queue"`），本轮结束后执行。不使用会插入当前轮次的 `steer`。
- 中断：`action: "interrupt"` 调用服务的 interrupt；服务回答没有正在执行的任务时返回 `blocked`。
- 仍每次查询 `permission` 与 `form` 两个接口，没有改用 `/api/event` 事件流：事件流需要常驻连接、断线重连和与轮询结果对账，收益不足以抵消风险。

终端里的 Codex（codex-cli 0.172.0 的 TUI）由它自己启动的 app-server 托管，同样提供来源目录下的控制接口，因此审批和问题走上面的路径，不需要也不安装 Codex 的 `PermissionRequest` hook。没有控制接口的旧版本保持只读。
