import AppKit
import SwiftUI

/// Interactive card for one session inside the pet orbit popover: approvals, questions and forms,
/// text replies, follow-up instructions and read-only requests (docs/session-interaction.md).
/// It renders from the card core returns and never interprets an agent-specific id.
///
/// Keyboard: Return submits answers, replies and instructions (never while an input method is
/// composing), ⌘Return picks the primary approval decision, 1-9 select an option, and Esc first
/// collapses the decision note or leaves a text field before it closes the popover.
struct PetInteractionCardView: View {
    let session: LiveSession
    @ObservedObject var store: SessionInteractionStore
    /// Manual opens move the keyboard into the reply field; auto-popups never take focus.
    var autoFocus = false
    var onFocusSession: () -> Void
    var onOpenSessions: () -> Void = {}
    var onCompleted: () -> Void

    /// The decision whose optional note is being typed; nil when no note field is shown.
    @State private var reasonDecisionID: String?
    @State private var expandedPreviews: Set<String> = []
    @State private var questionPage = 0
    @State private var reasonExpanded = false
    @State private var promptExpanded = false
    @State private var changesExpanded = false
    @State private var confirmingInterrupt = false
    @State private var keyOwner = UUID()
    @FocusState private var focus: InteractionKeyboard.Field?
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"

    private var payload: InteractionPayload? { store.payloads[session.id] }
    private var loading: Bool { store.loadingSessionIDs.contains(session.id) }
    private var submitting: Bool { store.submittingSessionIDs.contains(session.id) }
    private var feedback: (status: String, message: String)? { store.feedback[session.id] }
    /// Answered from here and still waiting: the controls stay off so the answer cannot be sent twice.
    private var completed: Bool { payload.map(store.isCompleted) ?? false }
    private var locked: Bool { submitting || completed }
    private var promptKey: String? { payload.map(store.draftKey) }
    private var waiting: Bool { session.phase == .needsInput }

    var body: some View {
        let _ = interfaceLanguage
        VStack(alignment: .leading, spacing: 8) {
            sessionHeader
            if let payload {
                pendingBar(payload: payload)
                switch payload.kind {
                case "approval":
                    if let approval = payload.approval {
                        approvalView(payload: payload, approval: approval)
                    } else {
                        readonlyView(payload: payload)
                    }
                case "questions":
                    questionsView(payload: payload)
                case "text", "followup":
                    textReplyView(payload: payload)
                case "readonly":
                    readonlyView(payload: payload)
                default:
                    idleView(payload: payload)
                }
                if payload.followUp.canInterrupt && !completed {
                    interruptRow(payload: payload)
                }
                if completed {
                    Text(feedback?.status == "uncertain"
                        ? L("结果未确认，请先核对原窗口；确认未送达后可刷新重试")
                        : L("已提交，等待 Agent 继续…"))
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            } else if loading || feedback == nil {
                // Nothing loaded and nothing failed yet: the first inspect is about to start.
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(L("正在读取待处理内容…"))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 10)
            }
            if let feedback {
                feedbackBanner(status: feedback.status, message: feedback.message)
            }
        }
        .padding(10)
        .background(Color.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 10))
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(waiting ? Palette.warn.opacity(0.35) : Color.primary.opacity(0.14), lineWidth: 1)
        )
        .accessibilityElement(children: .contain)
        .accessibilityLabel(waiting ? L("待处理请求：{0}", cardTitle) : L("后续指令：{0}", cardTitle))
        .onAppear {
            registerKeys()
            Task { await store.inspect(session: session) }
            applyInitialFocus()
        }
        .onDisappear { store.unregisterKeyInterceptor(owner: keyOwner) }
        .onChange(of: session) { _, _ in registerKeys() }
        .onChange(of: session.id) { _, _ in
            resetPromptState()
            Task { await store.inspect(session: session) }
        }
        .onChange(of: session.inputStateKey) { _, _ in
            reasonDecisionID = nil
            Task { await store.inspect(session: session, force: true) }
        }
        // A card of a session that is not waiting describes that very phase and turn; core refuses
        // an instruction sent with the state of an earlier one.
        .onChange(of: [session.phase.rawValue, session.turnId]) { _, _ in
            confirmingInterrupt = false
            Task { await store.inspect(session: session, force: true, preserveFeedback: true) }
        }
        .onChange(of: promptKey) { _, _ in
            resetPromptState()
            applyInitialFocus()
        }
        .onChange(of: reasonDecisionID) { _, shown in
            if shown != nil { DispatchQueue.main.async { focus = .declineReason } }
        }
        .onChange(of: feedback?.message) { _, message in
            guard let message, !message.isEmpty else { return }
            NSAccessibility.post(element: NSApp as Any, notification: .announcementRequested, userInfo: [
                .announcement: message,
                .priority: NSAccessibilityPriorityLevel.high.rawValue,
            ])
        }
        .simultaneousGesture(TapGesture().onEnded {
            if !NSApp.isActive {
                NSApp.activate(ignoringOtherApps: true)
            }
        })
    }

    private var cardTitle: String {
        if let title = payload?.title, !title.isEmpty { return title }
        return session.project ?? Format.provider(session.provider)
    }

    private func resetPromptState() {
        reasonDecisionID = nil
        questionPage = 0
        reasonExpanded = false
        promptExpanded = false
        changesExpanded = false
        confirmingInterrupt = false
        expandedPreviews = []
    }

    private func applyInitialFocus() {
        guard autoFocus else { return }
        DispatchQueue.main.async {
            guard focus == nil, let payload, payload.canRespond, !locked else { return }
            switch payload.kind {
            case "text", "followup": focus = .reply
            case "questions":
                if let question = currentQuestion(payload), InteractionAnswers.options(for: question).isEmpty { focus = .custom(question.id) }
            default: break
            }
        }
    }

    // MARK: Keyboard

    private func registerKeys() {
        store.registerKeyInterceptor(owner: keyOwner) { event in handleKey(event) }
    }

    private func primaryDecision(_ payload: InteractionPayload?) -> InteractionDecision? {
        payload?.approval?.decisions.first(where: { $0.style == "primary" })
    }

    private func handleKey(_ event: NSEvent) -> InteractionKeyOutcome {
        let editor = event.window?.firstResponder as? NSTextView
        let payload = self.payload
        var context = InteractionKeyboard.Context(
            kind: payload?.kind ?? "none",
            interactive: payload.map { $0.canRespond && !locked } ?? false
        )
        context.showDeclineReason = reasonDecisionID != nil
        context.focus = focus
        context.editingText = editor != nil || focus != nil
        context.composing = editor?.hasMarkedText() == true
        context.hasPrimaryDecision = primaryDecision(payload) != nil
        if let payload, payload.kind == "questions", let question = currentQuestion(payload) {
            context.optionCount = InteractionAnswers.options(for: question).count
        }
        let action = InteractionKeyboard.action(
            keyCode: event.keyCode,
            characters: event.charactersIgnoringModifiers,
            modifiers: event.modifierFlags,
            context: context
        )
        switch action {
        case .passThrough:
            return .passThrough
        case .close:
            return .close
        case .collapseDecline:
            reasonDecisionID = nil
            focus = nil
            return .handled
        case .defocus:
            focus = nil
            event.window?.makeFirstResponder(nil)
            return .handled
        case .selectOption(let index):
            guard let payload, let question = currentQuestion(payload) else { return .passThrough }
            // A digit only selects; Return then submits, so one stray key cannot answer.
            selectOption(payload: payload, question: question, index: index, directSubmit: false)
            return .handled
        case .submit:
            guard let payload else { return .passThrough }
            if payload.kind == "questions" {
                // ⌘Return asks for the whole form to be sent; a plain Return only moves through it.
                let flags = event.modifierFlags.intersection([.command, .option, .control, .shift])
                submitOrAdvance(payload: payload, explicit: flags == .command)
            } else {
                sendText(payload: payload)
            }
            return .handled
        case .confirmDecline:
            guard let payload else { return .passThrough }
            confirmReason(payload: payload)
            return .handled
        case .approve:
            guard let payload, let decision = primaryDecision(payload) else { return .passThrough }
            submitDecision(payload: payload, decision: decision, reason: nil)
            return .handled
        }
    }

    // MARK: Header

    /// What the card is, when that is not obvious from its controls.
    private func badge(for payload: InteractionPayload) -> String? {
        guard waiting else { return nil }
        let asks = ["approval", "questions", "readonly"].contains(payload.kind)
        return asks && !payload.canRespond ? L("只读") : nil
    }

    private var sessionHeader: some View {
        HStack(alignment: .center, spacing: 6) {
            ActivityIndicator(phase: session.phase)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 5) {
                    Text(session.project ?? Format.provider(session.provider))
                        .font(.system(size: DesignTokens.fontEntry, weight: .semibold))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if let payload, let badge = badge(for: payload) {
                        Text(badge)
                            .font(.system(size: 10, weight: .medium))
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(Color.primary.opacity(0.10), in: Capsule())
                            .foregroundStyle(.secondary)
                            .help(payload.readonlyReason)
                    }
                }
                Text([session.machine, session.shortIdentity.map { "#" + $0 }, waiting ? nil : session.phase.label]
                    .compactMap { $0 }
                    .joined(separator: " · "))
                    .font(AppFont.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 4)
            Button {
                Task { await store.refresh(session: session) }
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 11, weight: .medium))
            }
            .buttonStyle(.borderless)
            .help(L("刷新待处理内容"))
            .accessibilityLabel(L("刷新待处理内容"))
            .disabled(loading || submitting)
            if session.focusable {
                Button {
                    onFocusSession()
                } label: {
                    Image(systemName: "arrow.up.forward.app")
                        .font(.system(size: 11, weight: .medium))
                }
                .buttonStyle(.borderless)
                .help(session.focusHelp)
                .accessibilityLabel(session.focusHelp)
            }
        }
    }

    /// How many requests the session holds and, where core lists them, a way to step through them.
    @ViewBuilder
    private func pendingBar(payload: InteractionPayload) -> some View {
        let queue = payload.pendingRequests
        let asked = payload.requestedAt > 0
            ? Date(timeIntervalSince1970: payload.requestedAt).formatted(Date.FormatStyle(date: .omitted, time: .shortened).locale(AieyesLocale.locale))
            : nil
        if queue.count > 1 {
            let index = queue.firstIndex(where: { $0.requestId == payload.requestId }) ?? 0
            HStack(spacing: 4) {
                Button {
                    Task { await store.selectRequest(session: session, requestID: queue[index - 1].requestId) }
                } label: {
                    Image(systemName: "chevron.left").font(.system(size: 10, weight: .semibold))
                }
                .buttonStyle(.borderless)
                .disabled(index == 0 || loading || submitting)
                .help(L("上一个请求"))
                .accessibilityLabel(L("上一个请求"))
                Text(L("第 {0} 个请求，共 {1} 个", index + 1, queue.count))
                    .font(AppFont.caption)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                Button {
                    Task { await store.selectRequest(session: session, requestID: queue[index + 1].requestId) }
                } label: {
                    Image(systemName: "chevron.right").font(.system(size: 10, weight: .semibold))
                }
                .buttonStyle(.borderless)
                .disabled(index >= queue.count - 1 || loading || submitting)
                .help(L("下一个请求"))
                .accessibilityLabel(L("下一个请求"))
                if !queue[index].title.isEmpty {
                    Text(queue[index].title)
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .help(queue[index].title)
                }
                Spacer(minLength: 0)
            }
        } else if payload.pendingCount > 1 || asked != nil {
            Text([
                payload.pendingCount > 1 ? L("此会话共有 {0} 个待处理请求，按顺序逐个处理", payload.pendingCount) : nil,
                asked.map { L("请求于 {0}", $0) },
            ].compactMap { $0 }.joined(separator: " · "))
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: Approval

    @ViewBuilder
    private func approvalView(payload: InteractionPayload, approval: InteractionApproval) -> some View {
        let answerable = payload.canRespond && !approval.decisions.isEmpty
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Text(approval.toolName.isEmpty ? L("等待审批") : approval.toolName)
                    .font(.system(size: 11, weight: .semibold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Palette.warn.opacity(0.18), in: Capsule())
                    .foregroundStyle(Palette.warn)
                if !approval.title.isEmpty && approval.title != approval.toolName {
                    Text(approval.title)
                        .font(.system(size: DesignTokens.fontCaption, weight: .medium))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .help(approval.title)
                }
            }
            if !approval.cwd.isEmpty {
                Label(approval.cwd, systemImage: "folder")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.head)
                    .help(L("工作目录：{0}", approval.cwd))
                    .accessibilityLabel(L("工作目录：{0}", approval.cwd))
            }
            if !approval.reason.isEmpty {
                expandableText(approval.reason, expanded: $reasonExpanded)
            }
            if let plan = approval.plan, !plan.markdown.isEmpty {
                PetInteractionCodeBlock(text: plan.markdown, maxHeight: 180, attributed: InteractionMarkdown.attributed(plan.markdown), proportional: true)
                    .accessibilityLabel(L("计划审批"))
                if plan.truncated { truncationNote }
            }
            if !approval.command.isEmpty && approval.command != approval.plan?.markdown {
                PetInteractionCodeBlock(text: approval.command)
            }
            changesView(approval.changes)
            if !approval.paths.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    Text(L("涉及路径"))
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                    PetInteractionCodeBlock(text: approval.paths.joined(separator: "\n"), maxHeight: 60, muted: true)
                }
            }
            if !approval.details.isEmpty {
                PetInteractionCodeBlock(text: approval.details, maxHeight: 72, muted: true)
            }
            originalPrompt(payload, shown: [approval.reason, approval.command, approval.details, approval.plan?.markdown ?? ""])
            if !answerable {
                readonlyRows(payload: payload)
            } else if !completed {
                decisionRows(payload: payload, approval: approval)
            }
        }
    }

    private var truncationNote: some View {
        Text(L("内容过长，已截断显示；完整内容请在原窗口查看"))
            .font(.system(size: 10.5))
            .foregroundStyle(Palette.warn)
            .fixedSize(horizontal: false, vertical: true)
    }

    static func changeKindLabel(_ kind: String) -> String {
        switch kind {
        case "add": L("新增")
        case "update": L("修改")
        case "delete": L("删除")
        case "move": L("移动")
        default: L("文件")
        }
    }

    @ViewBuilder
    private func changesView(_ changes: [InteractionFileChange]) -> some View {
        let limit = 3
        let shown = changesExpanded ? changes : Array(changes.prefix(limit))
        ForEach(Array(shown.enumerated()), id: \.offset) { _, change in
            let lines = InteractionDiff.lines(for: change)
            let summary = InteractionDiff.summary(lines)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 5) {
                    Text(Self.changeKindLabel(change.kind))
                        .font(.system(size: 10, weight: .medium))
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background((change.kind == "delete" ? Palette.danger : change.kind == "add" ? Palette.ok : Palette.accent).opacity(0.16), in: Capsule())
                    Text(verbatim: change.path)
                        .font(.system(size: 11, design: .monospaced))
                        .lineLimit(1)
                        .truncationMode(.head)
                        .help(change.path)
                        .textSelection(.enabled)
                    Spacer(minLength: 2)
                    if !summary.isEmpty {
                        Text(verbatim: summary)
                            .font(.system(size: 10.5, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .accessibilityHidden(true)
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(Self.changeKindLabel(change.kind) + " " + change.path)
                if !change.movePath.isEmpty {
                    Text(L("移动到 {0}", change.movePath))
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .truncationMode(.head)
                }
                if !lines.isEmpty {
                    PetInteractionCodeBlock(text: lines.map(\.text).joined(separator: "\n"), maxHeight: 120, attributed: InteractionDiff.attributed(lines))
                }
                if change.truncated { truncationNote }
            }
        }
        if changes.count > limit {
            Button(changesExpanded ? L("收起") : L("显示全部 {0} 个文件", changes.count)) {
                changesExpanded.toggle()
            }
            .buttonStyle(.plain)
            .font(.system(size: 10.5))
            .foregroundStyle(.secondary)
        }
    }

    /// A decision that takes a note for the agent asks for it first; allowing never needs a second step.
    private func asksForReason(_ decision: InteractionDecision) -> Bool {
        decision.acceptsReason && decision.behavior != "allow"
    }

    @ViewBuilder
    private func decisionRows(payload: InteractionPayload, approval: InteractionApproval) -> some View {
        let noting = approval.decisions.first(where: { $0.id == reasonDecisionID && asksForReason($0) })
        // The decision whose note is open is confirmed from the note row, so it is not listed twice.
        let listed = approval.decisions.filter { $0.id != noting?.id }
        let stacked = approval.decisions.count > 2 || approval.decisions.contains(where: { !$0.description.isEmpty || $0.interrupts || $0.label.count > 8 })
        if let noting {
            let reasonBinding = Binding<String>(
                get: { store.draftBinding(for: payload).declineReason },
                set: { value in store.updateDraft(for: payload) { $0.declineReason = value } }
            )
            let placeholder = approval.kind == "plan" ? L("希望如何修改计划（可选）")
                : noting.behavior == "deny" ? L("拒绝原因（可选）") : L("给 Agent 的说明（可选）")
            HStack(spacing: 6) {
                TextField(placeholder, text: reasonBinding)
                    .textFieldStyle(.aieyes)
                    .font(AppFont.caption)
                    .focused($focus, equals: .declineReason)
                    .onSubmit { confirmReason(payload: payload) }
                    .accessibilityLabel(placeholder)
                    .disabled(locked)
                Button(noting.label) {
                    confirmReason(payload: payload)
                }
                .controlSize(.small)
                .tint(noting.style == "danger" ? Palette.danger : nil)
                .help(L("按 Return 确认，Esc 收起"))
                .disabled(locked)
            }
        }
        if stacked {
            VStack(alignment: .leading, spacing: 5) {
                ForEach(listed) { decision in
                    decisionButton(payload: payload, decision: decision, wide: true)
                }
                if payload.canHandoff {
                    HStack {
                        Spacer(minLength: 0)
                        handoffButton(payload: payload)
                            .help(L("释放弹窗拦截并在原终端显示确认框"))
                    }
                }
            }
        } else {
            HStack(spacing: 6) {
                ForEach(listed) { decision in
                    decisionButton(payload: payload, decision: decision, wide: false)
                }
                Spacer(minLength: 4)
                if payload.canHandoff {
                    handoffButton(payload: payload)
                        .help(L("释放弹窗拦截并在原终端显示确认框"))
                }
            }
        }
    }

    /// One decision, rendered from core's label, style and description only.
    @ViewBuilder
    private func decisionButton(payload: InteractionPayload, decision: InteractionDecision, wide: Bool) -> some View {
        let primary = decision.id == primaryDecision(payload)?.id
        let notes = [decision.description, decision.interrupts ? L("同时中断本轮") : ""].filter { !$0.isEmpty }
        let label = VStack(alignment: .leading, spacing: 1) {
            Text(decision.label)
                .frame(maxWidth: wide ? .infinity : nil, alignment: .leading)
            if wide {
                ForEach(notes, id: \.self) { note in
                    Text(verbatim: note)
                        .font(.system(size: 10.5))
                        .opacity(0.75)
                        .lineLimit(3)
                        .multilineTextAlignment(.leading)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        let action = { tapDecision(payload: payload, decision: decision) }
        let hint = (notes + [primary ? L("按 ⌘ Return 确认") : ""]).filter { !$0.isEmpty }.joined(separator: "\n")
        Group {
            if decision.style == "primary" {
                Button(action: action) { label }.buttonStyle(.borderedProminent)
            } else if decision.style == "danger" {
                Button(action: action) { label }.buttonStyle(.bordered).tint(Palette.danger)
            } else {
                Button(action: action) { label }.buttonStyle(.bordered)
            }
        }
        .controlSize(.small)
        .help(hint)
        .accessibilityLabel(decision.label)
        .accessibilityHint(notes.joined(separator: " "))
        .disabled(locked)
    }

    // MARK: Questions

    @ViewBuilder
    private func formHeader(_ form: InteractionForm) -> some View {
        let origin = form.source == "mcp"
            ? (form.serverName.isEmpty ? L("MCP 表单") : L("来自 MCP 服务 {0}", form.serverName))
            : ""
        if !origin.isEmpty || !form.title.isEmpty || !form.description.isEmpty || !form.url.isEmpty {
            VStack(alignment: .leading, spacing: 3) {
                if !origin.isEmpty {
                    Text(origin)
                        .font(.system(size: 11, weight: .semibold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Palette.accent.opacity(0.16), in: Capsule())
                }
                if !form.title.isEmpty && form.title != form.serverName {
                    Text(form.title)
                        .font(.system(size: DesignTokens.fontEntry, weight: .semibold))
                        .fixedSize(horizontal: false, vertical: true)
                }
                if !form.description.isEmpty {
                    expandableText(form.description, expanded: $reasonExpanded)
                }
                if !form.url.isEmpty {
                    HStack(spacing: 6) {
                        Text(verbatim: form.url)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                            .help(form.url)
                        Spacer(minLength: 2)
                        // Only web links open from here, and only on an explicit click.
                        if let url = URL(string: form.url), ["http", "https"].contains(url.scheme?.lowercased() ?? "") {
                            Button(L("打开链接")) { NSWorkspace.shared.open(url) }
                                .controlSize(.small)
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func questionsView(payload: InteractionPayload) -> some View {
        let draft = store.draftBinding(for: payload)
        let count = payload.questions.count
        let page = min(max(questionPage, 0), max(count - 1, 0))
        let unanswered = InteractionAnswers.unansweredIndices(in: payload, draft: draft)
        let ready = InteractionAnswers.allAnswered(in: payload, draft: draft)
        VStack(alignment: .leading, spacing: 10) {
            if let form = payload.form { formHeader(form) }
            if count > 1 {
                HStack(spacing: 6) {
                    Text(L("问题 {0}/{1}", page + 1, count))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                    ForEach(0..<count, id: \.self) { index in
                        let question = payload.questions[index]
                        let answered = InteractionAnswers.isAnswered(question, in: draft) && !unanswered.contains(index)
                        let blocking = unanswered.contains(index)
                        Button {
                            questionPage = index
                        } label: {
                            Circle()
                                .fill(answered ? Palette.accent : Color.clear)
                                .overlay(Circle().strokeBorder(index == page ? Color.primary : (answered ? Palette.accent : (blocking ? Palette.warn : Color.secondary)), lineWidth: index == page ? 1.5 : 1))
                                .frame(width: 8, height: 8)
                                .padding(3)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(L("问题 {0}/{1}", index + 1, count))
                        .accessibilityValue(answered ? L("已回答") : (question.required ? L("未回答") : L("可选")))
                        .accessibilityAddTraits(index == page ? .isSelected : [])
                    }
                    Spacer(minLength: 4)
                    Text(L("已回答 {0}/{1}", InteractionAnswers.answeredCount(in: payload, draft: draft), count))
                        .font(AppFont.caption)
                        .monospacedDigit()
                        .foregroundStyle(unanswered.isEmpty ? Palette.ok : Palette.warn)
                }
            }
            if payload.questions.indices.contains(page) {
                questionBlock(payload: payload, question: payload.questions[page], draft: draft)
            }
            originalPrompt(payload, shown: payload.questions.map(\.question) + [payload.form?.description ?? ""])
            if !payload.canRespond {
                if count > 1 { HStack(spacing: 6) { pagingButtons(page: page, count: count) } }
                readonlyRows(payload: payload)
            } else if !completed {
                if count > 1 { HStack(spacing: 6) { pagingButtons(page: page, count: count); Spacer(minLength: 0) } }
                HStack(spacing: 6) {
                    // Core accepts these only where the original form offers them.
                    if payload.form?.canCancel == true {
                        Button(L("取消表单")) { dismissForm(payload: payload, action: "cancel") }
                            .controlSize(.small)
                            .help(L("关闭这个表单，不提交任何内容"))
                            .disabled(locked)
                    }
                    if payload.form?.canSkip == true {
                        Button(L("拒绝填写")) { dismissForm(payload: payload, action: "skip") }
                            .controlSize(.small)
                            .help(L("告诉 Agent 不提供这些信息"))
                            .disabled(locked)
                    }
                    Spacer(minLength: 4)
                    if payload.canHandoff {
                        handoffButton(payload: payload)
                    }
                    Button(submitting ? L("提交中…") : L("提交回答")) {
                        submitQuestions(payload: payload)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                    .disabled(locked || !ready)
                }
            }
        }
    }

    @ViewBuilder
    private func pagingButtons(page: Int, count: Int) -> some View {
        Button(L("上一步")) { questionPage = page - 1 }
            .controlSize(.small)
            .disabled(page == 0)
        Button(L("下一步")) { questionPage = page + 1 }
            .controlSize(.small)
            .disabled(page >= count - 1)
    }

    @ViewBuilder
    private func questionBlock(payload: InteractionPayload, question q: InteractionQuestion, draft: InteractionDraft) -> some View {
        let enabled = payload.canRespond && !locked
        let options = InteractionAnswers.options(for: q)
        let multi = InteractionAnswers.isMulti(q)
        VStack(alignment: .leading, spacing: 6) {
            if !q.header.isEmpty && q.header != q.question {
                Text(q.header)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                Text(q.question.isEmpty ? q.header : q.question)
                    .font(.system(size: DesignTokens.fontEntry, weight: .medium))
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                if multi {
                    Text(L("（可多选）"))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                }
                if !q.required {
                    Text(L("可选"))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                }
            }
            if !q.description.isEmpty && q.description != q.question {
                Text(q.description)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }
            ForEach(Array(options.enumerated()), id: \.offset) { index, opt in
                let selected = isOptionSelected(question: q, index: index, draft: draft)
                let sendable = InteractionAnswers.wireAnswer(for: index, in: q) != nil
                VStack(alignment: .leading, spacing: 4) {
                    Button {
                        selectOption(payload: payload, question: q, index: index, directSubmit: true)
                    } label: {
                        HStack(alignment: .top, spacing: 6) {
                            Image(systemName: multi
                                ? (selected ? "checkmark.square.fill" : "square")
                                : (selected ? "largecircle.fill.circle" : "circle"))
                                .font(.system(size: 12))
                                .foregroundStyle(selected ? Palette.accent : .secondary)
                                .padding(.top, 1)
                                .accessibilityHidden(true)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(opt.label)
                                    .font(AppFont.caption)
                                    .fontWeight(selected ? .semibold : .regular)
                                    .foregroundStyle(.primary)
                                    .multilineTextAlignment(.leading)
                                if let desc = opt.description, !desc.isEmpty {
                                    Text(desc)
                                        .font(.system(size: 11))
                                        .foregroundStyle(.secondary)
                                        .multilineTextAlignment(.leading)
                                }
                            }
                            Spacer(minLength: 0)
                            if enabled && sendable && index < 9 {
                                // The digit key that selects this option.
                                Text(verbatim: "\(index + 1)")
                                    .font(.system(size: 10, weight: .medium, design: .monospaced))
                                    .foregroundStyle(.tertiary)
                                    .accessibilityHidden(true)
                            }
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(
                            selected ? Palette.accent.opacity(0.14) : Color.primary.opacity(0.04),
                            in: RoundedRectangle(cornerRadius: 7)
                        )
                        .overlay(
                            RoundedRectangle(cornerRadius: 7)
                                .strokeBorder(selected ? Palette.accent.opacity(0.55) : Color.primary.opacity(0.08), lineWidth: 1)
                        )
                        .contentShape(RoundedRectangle(cornerRadius: 7))
                    }
                    .buttonStyle(.plain)
                    .disabled(!enabled || !sendable)
                    .help(sendable ? "" : L("此选项无法从弹窗准确提交，请在原窗口选择"))
                    .accessibilityLabel(opt.label)
                    .accessibilityValue(selected ? L("已选择") : "")
                    .accessibilityHint(sendable ? (opt.description ?? "") : L("此选项无法从弹窗准确提交，请在原窗口选择"))
                    .accessibilityAddTraits(selected ? .isSelected : [])

                    if let preview = opt.preview, !preview.isEmpty {
                        let pKey = "\(q.id)#\(index)"
                        let expanded = expandedPreviews.contains(pKey)
                        Button {
                            if expanded { expandedPreviews.remove(pKey) }
                            else { expandedPreviews.insert(pKey) }
                        } label: {
                            Label(expanded ? L("收起预览") : L("查看预览"), systemImage: expanded ? "chevron.down" : "chevron.right")
                                .font(.system(size: 10.5))
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                        .padding(.leading, 4)
                        if expanded {
                            PetInteractionCodeBlock(text: preview)
                        }
                    }
                }
            }

            if payload.canRespond && !completed && InteractionAnswers.acceptsTyped(q) {
                let customBinding = Binding<String>(
                    get: { store.draftBinding(for: payload).customAnswers[q.id] ?? "" },
                    set: { value in
                        store.updateDraft(for: payload) {
                            $0.customAnswers[q.id] = value
                            if !multi && !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                $0.singleChoices.removeValue(forKey: q.id)
                            }
                        }
                    }
                )
                let numeric = q.kind == "integer" || q.kind == "number"
                let placeholder = !q.placeholder.isEmpty ? q.placeholder
                    : numeric ? L("输入数字…")
                    : options.isEmpty ? L("输入回答…") : L("其他自定义回答…")
                let name = q.header.isEmpty ? q.question : q.header
                if q.secret {
                    // Masked on screen and never read back into any label or tooltip.
                    SecureField(placeholder, text: customBinding)
                        .textFieldStyle(.aieyes)
                        .font(AppFont.caption)
                        .focused($focus, equals: .custom(q.id))
                        .onSubmit { submitOrAdvance(payload: payload) }
                        .help(L("输入内容已隐藏"))
                        .accessibilityLabel(name)
                        .accessibilityHint(L("输入内容已隐藏"))
                        .disabled(locked)
                } else {
                    TextField(placeholder, text: customBinding)
                        .textFieldStyle(.aieyes)
                        .font(AppFont.caption)
                        .focused($focus, equals: .custom(q.id))
                        .onSubmit { submitOrAdvance(payload: payload) }
                        .accessibilityLabel(name)
                        .disabled(locked)
                }
            }
            if payload.canRespond, let problem = InteractionAnswers.problem(with: q, in: draft) {
                Text(L(problem))
                    .font(.system(size: 11))
                    .foregroundStyle(Palette.danger)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    // MARK: Text, follow-up, read-only

    /// The agent's words that give a reply its context, labelled by where they come from.
    @ViewBuilder
    private func promptContext(_ payload: InteractionPayload, maxHeight: CGFloat) -> some View {
        let prompt = payload.textPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        if !prompt.isEmpty {
            VStack(alignment: .leading, spacing: 3) {
                if payload.promptSource == "lastMessage" {
                    Text(L("Agent 最后一条回复"))
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                ScrollView {
                    Text(prompt)
                        .font(AppFont.caption)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: maxHeight)
            }
        } else if waiting {
            // A card must say what it is for even when the agent's prompt could not be read.
            Text(L("原会话正在等待输入，未读取到提示内容"))
                .font(AppFont.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// How the text will reach the session, so the user knows what Send does.
    private func deliveryNote(_ payload: InteractionPayload) -> String {
        if payload.kind == "followup" {
            switch payload.followUp.mode {
            case "queue": return L("会话仍在执行，指令将排队到本轮结束后执行")
            case "newTurn": return L("将在原会话开启下一轮")
            default: break
            }
        }
        switch payload.kind == "followup" && !payload.followUp.adapter.isEmpty ? payload.followUp.adapter : payload.adapter {
        case "tmux": return L("通过 tmux 发送")
        case "macosTerminal": return L("通过原终端发送")
        default: return L("发送到原会话")
        }
    }

    @ViewBuilder
    private func textReplyView(payload: InteractionPayload) -> some View {
        let followUp = payload.kind == "followup"
        let textBinding = Binding<String>(
            get: { store.draftBinding(for: payload).textReply },
            set: { value in store.updateDraft(for: payload) { $0.textReply = value } }
        )
        let placeholder = followUp ? L("输入后续指令…") : L("输入回复内容…")
        VStack(alignment: .leading, spacing: 8) {
            promptContext(payload, maxHeight: 120)
            if !payload.canRespond {
                readonlyRows(payload: payload)
            } else if !completed {
                TextField(placeholder, text: textBinding, axis: .vertical)
                    .textFieldStyle(.aieyes)
                    .lineLimit(2...5)
                    .font(AppFont.caption)
                    .focused($focus, equals: .reply)
                    .onSubmit { sendText(payload: payload) }
                    .help(L("按 Return 发送，Option Return 换行"))
                    .accessibilityLabel(placeholder)
                    .disabled(locked)
                HStack(spacing: 6) {
                    Text(deliveryNote(payload))
                        .font(.system(size: 10.5))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer()
                    Button(submitting ? L("发送中…") : L("发送")) {
                        sendText(payload: payload)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                    .disabled(locked || textBinding.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }

    /// What a read-only card is waiting for, in words, when it has no structured content of its own.
    private func waitLabel(_ payload: InteractionPayload) -> String {
        switch payload.inputKind {
        case "approval": L("等待审批")
        case "question": L("等待选择")
        case "elicitation": L("MCP 表单")
        default: L("等待输入")
        }
    }

    @ViewBuilder
    private func readonlyView(payload: InteractionPayload) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(waitLabel(payload))
                .font(.system(size: 11, weight: .semibold))
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Palette.warn.opacity(0.18), in: Capsule())
                .foregroundStyle(Palette.warn)
            if let form = payload.form { formHeader(form) }
            if payload.textPrompt.trimmingCharacters(in: .whitespacesAndNewlines) != (payload.form?.description ?? "").trimmingCharacters(in: .whitespacesAndNewlines) {
                promptContext(payload, maxHeight: 130)
            }
            readonlyRows(payload: payload)
        }
    }

    /// `kind: "none"`: nothing to answer. For a session that is not waiting this is where core says
    /// why an instruction cannot be sent yet.
    @ViewBuilder
    private func idleView(payload: InteractionPayload) -> some View {
        let reason = !payload.readonlyReason.isEmpty ? payload.readonlyReason
            : !waiting && !payload.followUp.reason.isEmpty ? payload.followUp.reason
            : L("当前没有需要处理的请求")
        VStack(alignment: .leading, spacing: 8) {
            if !waiting { promptContext(payload, maxHeight: 100) }
            Text(reason)
                .font(AppFont.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.vertical, waiting ? 6 : 0)
            if !waiting && !payload.answered {
                HStack(spacing: 6) {
                    Spacer()
                    jumpButton(prominent: false)
                }
            }
        }
    }

    @ViewBuilder
    private func jumpButton(prominent: Bool) -> some View {
        if session.focusable {
            if prominent {
                Button(L("跳转到原窗口")) { onFocusSession() }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                    .help(session.focusHelp)
            } else {
                Button(L("跳转到原窗口")) { onFocusSession() }
                    .controlSize(.small)
                    .help(session.focusHelp)
            }
        } else {
            // Nothing here can reach this session's window; the sessions page shows where it runs.
            Button(L("全部会话")) { onOpenSessions() }
                .controlSize(.small)
        }
    }

    /// Why the card cannot answer, and what the user can do about it: switch on the hook that would
    /// make it answerable, or go to the window that owns the request.
    @ViewBuilder
    private func readonlyRows(payload: InteractionPayload) -> some View {
        if !payload.readonlyReason.isEmpty {
            Text(payload.readonlyReason)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        if session.machineId != "local" {
            Text(L("远程会话无法从这里跳转，请在 {0} 的原终端处理", session.machine))
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        HStack(spacing: 6) {
            if payload.quickEnable == "approvals" || (payload.quickEnable.isEmpty && payload.needsClaudeApprovals) {
                Button(L("启用弹窗审批")) {
                    Task {
                        await store.configure(claudeApprovals: true)
                        // The request already on screen was raised before the hook existed.
                        if store.status.claudeApprovals { store.note(L("已启用弹窗审批，之后的工具授权会显示在这里"), for: session) }
                        await store.inspect(session: session, force: true, preserveFeedback: true)
                    }
                }
                .controlSize(.small)
                .disabled(store.saving)
            } else if payload.quickEnable == "questions" || (payload.quickEnable.isEmpty && payload.needsClaudeHook) {
                Button(L("启用弹窗回答")) {
                    Task {
                        await store.configure(claudeQuestions: true)
                        await store.inspect(session: session, force: true)
                    }
                }
                .controlSize(.small)
                .disabled(store.saving)
            }
            Spacer()
            jumpButton(prominent: true)
        }
    }

    /// Stops the running turn. Two clicks: the first only arms the button.
    @ViewBuilder
    private func interruptRow(payload: InteractionPayload) -> some View {
        HStack(spacing: 6) {
            Text(confirmingInterrupt ? L("中断会撤销本轮未完成的操作和待处理请求") : L("可以中断正在执行的本轮"))
                .font(.system(size: 10.5))
                .foregroundStyle(confirmingInterrupt ? Palette.danger : .secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 4)
            if confirmingInterrupt {
                Button(L("取消")) { confirmingInterrupt = false }
                    .controlSize(.small)
            }
            Button(confirmingInterrupt ? L("确认中断") : L("中断本轮")) {
                guard confirmingInterrupt else {
                    confirmingInterrupt = true
                    return
                }
                confirmingInterrupt = false
                Task { _ = await store.interrupt(session: session, payload: payload) }
            }
            .buttonStyle(.bordered)
            .tint(Palette.danger)
            .controlSize(.small)
            .disabled(submitting)
        }
    }

    private func handoffButton(payload: InteractionPayload) -> some View {
        Button(L("回原终端")) {
            Task {
                if await store.handoff(session: session, payload: payload) {
                    onCompleted()
                }
            }
        }
        .controlSize(.small)
        .disabled(locked)
    }

    /// Long text starts at three lines; the rest is one click away and never silently cut.
    @ViewBuilder
    private func expandableText(_ text: String, expanded: Binding<Bool>) -> some View {
        let long = text.count > 80 || text.contains("\n")
        VStack(alignment: .leading, spacing: 2) {
            Text(text)
                .font(AppFont.caption)
                .foregroundStyle(.primary)
                .lineLimit(expanded.wrappedValue || !long ? nil : 3)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
                .help(text)
            if long {
                Button(expanded.wrappedValue ? L("收起") : L("展开")) {
                    expanded.wrappedValue.toggle()
                }
                .buttonStyle(.plain)
                .font(.system(size: 10.5))
                .foregroundStyle(.secondary)
            }
        }
    }

    /// The agent's own prompt text, when it says more than the structured fields already on the card.
    @ViewBuilder
    private func originalPrompt(_ payload: InteractionPayload, shown: [String]) -> some View {
        let prompt = payload.textPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        if !prompt.isEmpty, !shown.contains(where: { $0.trimmingCharacters(in: .whitespacesAndNewlines) == prompt }) {
            Button {
                promptExpanded.toggle()
            } label: {
                Label(promptExpanded ? L("收起原始提示") : L("查看原始提示"), systemImage: promptExpanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 10.5))
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            if promptExpanded {
                PetInteractionCodeBlock(text: prompt, maxHeight: 96, muted: true)
            }
        }
    }

    @ViewBuilder
    private func feedbackBanner(status: String, message: String) -> some View {
        let color: Color = switch status {
        case "confirmed", "submitted": Palette.ok
        case "blocked", "uncertain", "note": Palette.warn
        default: Palette.danger
        }
        let symbol = switch status {
        case "confirmed", "submitted": "checkmark.circle.fill"
        case "note": "info.circle.fill"
        default: "exclamationmark.triangle.fill"
        }
        HStack(spacing: 5) {
            Image(systemName: symbol)
                .font(.system(size: 11))
                .foregroundStyle(color)
                .accessibilityHidden(true)
            Text(message)
                .font(.system(size: 11))
                .foregroundStyle(color)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
        }
        .padding(.top, 2)
    }

    // MARK: Actions

    private func currentQuestion(_ payload: InteractionPayload) -> InteractionQuestion? {
        guard !payload.questions.isEmpty else { return nil }
        return payload.questions[min(max(questionPage, 0), payload.questions.count - 1)]
    }

    private func isOptionSelected(question: InteractionQuestion, index: Int, draft: InteractionDraft) -> Bool {
        if InteractionAnswers.isMulti(question) {
            return draft.multiChoices[question.id]?.contains(index) == true
        }
        return draft.singleChoices[question.id] == index && InteractionAnswers.customAnswer(for: question, in: draft).isEmpty
    }

    private func selectOption(payload: InteractionPayload, question: InteractionQuestion, index: Int, directSubmit: Bool) {
        guard payload.canRespond, !locked, InteractionAnswers.wireAnswer(for: index, in: question) != nil else { return }
        if InteractionAnswers.isMulti(question) {
            store.updateDraft(for: payload) { draft in
                var set = draft.multiChoices[question.id] ?? []
                if set.contains(index) { set.remove(index) }
                else { set.insert(index) }
                draft.multiChoices[question.id] = set
            }
            return
        }
        store.updateDraft(for: payload) { draft in
            // Clicking the chosen option of an optional field clears it again.
            if !question.required, draft.singleChoices[question.id] == index, draft.customAnswers[question.id, default: ""].isEmpty {
                draft.singleChoices.removeValue(forKey: question.id)
            } else {
                draft.singleChoices[question.id] = index
            }
            draft.customAnswers.removeValue(forKey: question.id)
        }
        // A lone question answers on click. An option with a preview is only selected,
        // so it can be compared first and then sent with the submit button or Return.
        let options = InteractionAnswers.options(for: question)
        if directSubmit, InteractionAnswers.isSingleDirect(payload), (options[index].preview ?? "").isEmpty {
            submitQuestions(payload: payload)
        }
    }

    /// The popover moves on once a request is settled, unless more requests of this session follow.
    private func finish(_ payload: InteractionPayload) {
        guard payload.kind != "followup", payload.pendingCount <= 1 else { return }
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(700))
            onCompleted()
        }
    }

    private func tapDecision(payload: InteractionPayload, decision: InteractionDecision) {
        if asksForReason(decision) {
            // The first click opens the note; the decision is sent from the note row.
            reasonDecisionID = decision.id
            return
        }
        submitDecision(payload: payload, decision: decision, reason: nil)
    }

    private func submitDecision(payload: InteractionPayload, decision: InteractionDecision, reason: String?) {
        guard payload.canRespond, !locked else { return }
        Task {
            // Always `decide` plus the id as listed, so an adapter's own id is never read as a verb.
            if await store.respond(session: session, payload: payload, action: "decide", decisionId: decision.id, reason: decision.acceptsReason ? reason : nil) {
                reasonDecisionID = nil
                finish(payload)
            }
        }
    }

    private func confirmReason(payload: InteractionPayload) {
        guard let decision = payload.approval?.decisions.first(where: { $0.id == reasonDecisionID }) else { return }
        submitDecision(payload: payload, decision: decision, reason: store.draftBinding(for: payload).declineReason)
    }

    /// Return pages through a form and sends it from its last page; ⌘Return (`explicit`) sends it from
    /// any page. Either way an incomplete form moves to the first question that still needs an answer.
    private func submitOrAdvance(payload: InteractionPayload, explicit: Bool = false) {
        let page = min(max(questionPage, 0), max(payload.questions.count - 1, 0))
        switch InteractionAnswers.returnStep(in: payload, draft: store.draftBinding(for: payload), page: page, explicit: explicit) {
        case .page(let next):
            guard next != questionPage else { return }
            questionPage = next
            // Keep typing on the new page when it is a plain field, as on the first one.
            let question = payload.questions[next]
            if focus != nil {
                focus = InteractionAnswers.options(for: question).isEmpty ? .custom(question.id) : nil
            }
        case .submit:
            submitQuestions(payload: payload)
        }
    }

    private func submitQuestions(payload: InteractionPayload) {
        let draft = store.draftBinding(for: payload)
        guard payload.canRespond, !locked, InteractionAnswers.allAnswered(in: payload, draft: draft) else { return }
        let answers = InteractionAnswers.answers(for: payload, draft: draft)
        Task {
            if await store.respond(session: session, payload: payload, action: "answer", answers: answers) {
                finish(payload)
            }
        }
    }

    /// `cancel` closes the form, `skip` declines to fill it; both are offered only when the form allows.
    private func dismissForm(payload: InteractionPayload, action: String) {
        guard payload.canRespond, !locked, let form = payload.form, action == "cancel" ? form.canCancel : form.canSkip else { return }
        Task {
            if await store.respond(session: session, payload: payload, action: action) {
                finish(payload)
            }
        }
    }

    private func sendText(payload: InteractionPayload) {
        let body = store.draftBinding(for: payload).textReply.trimmingCharacters(in: .whitespacesAndNewlines)
        guard payload.canRespond, !locked, !body.isEmpty else { return }
        Task {
            if await store.respond(session: session, payload: payload, action: payload.kind == "followup" ? "followup" : "answer", text: body) {
                finish(payload)
            }
        }
    }
}

/// Monospaced block for a command, details or preview. Content taller than `maxHeight` scrolls with
/// visible indicators, shows a shaded lower edge, and can be expanded, so its tail is never hidden
/// without a cue.
struct PetInteractionCodeBlock: View {
    let text: String
    var maxHeight: CGFloat = 110
    var muted = false
    /// Styled content (a coloured diff, a rendered plan) shown in place of `text`.
    var attributed: AttributedString? = nil
    /// Prose rather than code: the body font instead of the monospaced one.
    var proportional = false

    static let expandedHeight: CGFloat = 320
    static func overflows(contentHeight: CGFloat, maxHeight: CGFloat) -> Bool { contentHeight > maxHeight + 1 }
    static func height(contentHeight: CGFloat?, maxHeight: CGFloat, expanded: Bool) -> CGFloat {
        min(contentHeight ?? maxHeight, expanded ? max(maxHeight, expandedHeight) : maxHeight)
    }

    @State private var contentHeight: CGFloat?
    @State private var expanded = false

    var body: some View {
        let overflowing = contentHeight.map { Self.overflows(contentHeight: $0, maxHeight: maxHeight) } ?? false
        let shape = RoundedRectangle(cornerRadius: DesignTokens.radiusControl)
        VStack(alignment: .leading, spacing: 3) {
            ScrollView {
                (attributed.map { Text($0) } ?? Text(verbatim: text))
                    .font(.system(size: DesignTokens.fontCaption, design: proportional ? .default : .monospaced))
                    .foregroundStyle(muted ? Color.secondary : Color.primary)
                    .lineSpacing(2)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(DesignTokens.spaceControl)
                    .onGeometryChange(for: CGFloat.self) { geometry in
                        geometry.size.height
                    } action: { height in
                        contentHeight = height
                    }
            }
            .scrollIndicators(.visible)
            .scrollIndicatorsFlash(onAppear: true)
            .scrollBounceBehavior(.basedOnSize)
            .frame(height: Self.height(contentHeight: contentHeight, maxHeight: maxHeight, expanded: expanded))
            .background(Color.primary.opacity(0.055), in: shape)
            .overlay(alignment: .bottom) {
                if overflowing && !expanded {
                    LinearGradient(colors: [Color.primary.opacity(0), Color.primary.opacity(0.16)], startPoint: .top, endPoint: .bottom)
                        .frame(height: 16)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .clipShape(shape)
            if overflowing {
                Button(expanded ? L("收起") : L("展开全部")) {
                    expanded.toggle()
                }
                .buttonStyle(.plain)
                .font(.system(size: 10.5))
                .foregroundStyle(.secondary)
            }
        }
    }
}

struct PetInteractionSettingsView: View {
    @ObservedObject var store: SessionInteractionStore
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"

    var body: some View {
        let _ = interfaceLanguage
        VStack(alignment: .leading, spacing: 8) {
            Toggle(
                L("启用桌宠等待输入交互弹窗"),
                isOn: Binding(
                    get: { store.status.enabled },
                    set: { value in Task { await store.configure(enabled: value) } }
                )
            )
            .disabled(store.saving)
            Text(L("点击等待输入的 Agent 徽章可直接审批、选择选项或回复，不依赖飞书渠道。"))
                .font(AppFont.secondary)
                .foregroundStyle(.secondary)

            if store.status.enabled {
                Toggle(
                    L("等待输入时自动浮出弹窗"),
                    isOn: Binding(
                        get: { store.status.autoPopup },
                        set: { value in Task { await store.configure(autoPopup: value) } }
                    )
                )
                .disabled(store.saving)
                Text(L("不抢占当前应用键盘焦点；全屏隐藏桌宠时不弹出，同一请求关闭后不重复弹出。"))
                    .font(AppFont.secondary)
                    .foregroundStyle(.secondary)

                Toggle(
                    L("在弹窗中回答 Claude / Qoder 提问和 MCP 表单"),
                    isOn: Binding(
                        get: { store.status.claudeQuestions || store.status.claudeApprovals },
                        set: { value in
                            Task {
                                await store.configure(
                                    claudeQuestions: value,
                                    claudeApprovals: value ? store.status.claudeApprovals : false
                                )
                            }
                        }
                    )
                )
                .disabled(store.saving)

                Toggle(
                    L("在弹窗中审批 Claude / Qoder / CodeBuddy / ZCode 工具权限"),
                    isOn: Binding(
                        get: { store.status.claudeApprovals },
                        set: { value in Task { await store.configure(claudeApprovals: value) } }
                    )
                )
                .disabled(store.saving)
                Text(L("开启后会在 Claude / Qoder / CodeBuddy / ZCode 的配置中安装授权 Hook，工具授权改在弹窗中决定。Claude / Qoder 可选允许本次、本会话或始终允许、接受编辑、拒绝或回原终端，应用退出或超时会自动交还终端；CodeBuddy / ZCode 只有允许本次和拒绝，等待期间不显示自己的授权提示，每次最长 120 秒，超时后交还终端。"))
                    .font(AppFont.secondary)
                    .foregroundStyle(.secondary)

                Toggle(
                    L("在弹窗中审批 Kiro CLI 工具权限"),
                    isOn: Binding(
                        get: { store.status.kiroApprovals == true },
                        set: { value in Task { await store.configure(kiroApprovals: value) } }
                    )
                )
                .disabled(store.saving)
                Text(L("适用于 Kiro CLI 3.4.0 及更新版本的新 V3 会话。可允许或拒绝本次操作，超时后交还原终端。"))
                    .font(AppFont.secondary)
                    .foregroundStyle(.secondary)

                HStack {
                    Toggle(
                        L("允许通过终端适配发送文本"),
                        isOn: Binding(
                            get: { store.status.desktopAllowTerminal ?? store.status.allowTerminal },
                            set: { value in Task { await store.configure(allowTerminal: value) } }
                        )
                    )
                    .disabled(store.saving)
                    if !SessionChannelAutomation.installed.isEmpty {
                        Button(L("授权终端自动化")) {
                            Task { await SessionChannelAutomation.authorize() }
                        }
                        .controlSize(.small)
                    }
                }
            }

            if let error = store.error {
                ErrorNotice(message: error)
            }
        }
        .onAppear {
            Task { await store.refreshStatus() }
        }
    }
}
