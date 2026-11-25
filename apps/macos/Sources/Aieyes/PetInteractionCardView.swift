import AppKit
import SwiftUI

/// Interactive waiting-for-input card embedded inside the pet orbit popover.
struct PetInteractionCardView: View {
    let session: LiveSession
    @ObservedObject var store: SessionInteractionStore
    var onFocusSession: () -> Void
    var onCompleted: () -> Void

    @State private var showDeclineReason = false
    @State private var expandedPreviews: Set<String> = []
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"

    private var payload: InteractionPayload? { store.payloads[session.id] }
    private var loading: Bool { store.loadingSessionIDs.contains(session.id) }
    private var submitting: Bool { store.submittingSessionIDs.contains(session.id) }
    private var feedback: (status: String, message: String)? { store.feedback[session.id] }

    var body: some View {
        let _ = interfaceLanguage
        VStack(alignment: .leading, spacing: 8) {
            sessionHeader
            if loading && payload == nil {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(L("正在读取待处理内容…"))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 10)
            } else if let payload {
                switch payload.kind {
                case "approval":
                    if let approval = payload.approval {
                        approvalView(payload: payload, approval: approval)
                    }
                case "questions":
                    questionsView(payload: payload)
                case "text":
                    textReplyView(payload: payload)
                case "readonly":
                    readonlyView(payload: payload)
                default:
                    Text(L("当前没有需要处理的请求"))
                        .font(AppFont.caption)
                        .foregroundStyle(.secondary)
                        .padding(.vertical, 6)
                }
            }
            if let feedback {
                feedbackBanner(status: feedback.status, message: feedback.message)
            }
        }
        .padding(10)
        .background(Color.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 10))
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(Palette.warn.opacity(0.35), lineWidth: 1)
        )
        .onAppear {
            Task { await store.inspect(session: session) }
        }
        .onChange(of: session.id) { _, _ in
            showDeclineReason = false
            Task { await store.inspect(session: session) }
        }
        .onChange(of: session.inputStateKey) { _, _ in
            showDeclineReason = false
            Task { await store.inspect(session: session, force: true) }
        }
        .simultaneousGesture(TapGesture().onEnded {
            if !NSApp.isActive {
                NSApp.activate(ignoringOtherApps: true)
            }
        })
    }

    private var sessionHeader: some View {
        HStack(alignment: .center, spacing: 6) {
            ActivityIndicator(phase: session.phase)
            VStack(alignment: .leading, spacing: 1) {
                Text(session.project ?? Format.provider(session.provider))
                    .font(.system(size: DesignTokens.fontEntry, weight: .semibold))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text([session.machine, session.shortIdentity.map { "#" + $0 }]
                    .compactMap { $0 }
                    .joined(separator: " · "))
                    .font(AppFont.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 4)
            Button {
                Task { await store.inspect(session: session, force: true) }
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 11, weight: .medium))
            }
            .buttonStyle(.borderless)
            .help(L("刷新待处理内容"))
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
            }
        }
    }

    @ViewBuilder
    private func approvalView(payload: InteractionPayload, approval: InteractionApproval) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Text(approval.toolName)
                    .font(.system(size: 11, weight: .semibold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Palette.warn.opacity(0.18), in: Capsule())
                    .foregroundStyle(Palette.warn)
                Text(approval.reason)
                    .font(AppFont.caption)
                    .foregroundStyle(.primary)
                    .lineLimit(2)
            }
            if !approval.command.isEmpty {
                PetInteractionCodeBlock(text: approval.command)
            }
            if !approval.details.isEmpty {
                PetInteractionCodeBlock(text: approval.details, maxHeight: 72, muted: true)
            }
            if !payload.canRespond {
                if !payload.readonlyReason.isEmpty {
                    Text(payload.readonlyReason)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                HStack(spacing: 6) {
                    Spacer()
                    if session.focusable {
                        Button(L("跳转到原窗口")) {
                            onFocusSession()
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                    }
                }
            } else {
                if showDeclineReason && payload.adapter == "claudeHook" {
                    let reasonBinding = Binding<String>(
                        get: { store.draftBinding(for: payload).declineReason },
                        set: { value in store.updateDraft(for: payload) { $0.declineReason = value } }
                    )
                    HStack(spacing: 6) {
                        TextField(L("拒绝原因（可选）"), text: reasonBinding)
                            .textFieldStyle(.aieyes)
                            .font(AppFont.caption)
                        Button(L("确认拒绝")) {
                            submitApproval(payload: payload, action: "decline", reason: reasonBinding.wrappedValue)
                        }
                        .controlSize(.small)
                        .tint(Palette.danger)
                        .disabled(submitting)
                    }
                }
                HStack(spacing: 6) {
                    ForEach(approval.decisions) { decision in
                        if decision.style == "primary" {
                            Button(decision.label) {
                                submitApproval(payload: payload, action: decision.id, reason: nil)
                            }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                            .disabled(submitting)
                        } else if decision.style == "danger" {
                            Button(decision.label) {
                                if payload.adapter == "claudeHook" && !showDeclineReason {
                                    showDeclineReason = true
                                } else {
                                    let r = store.draftBinding(for: payload).declineReason
                                    submitApproval(payload: payload, action: decision.id, reason: r)
                                }
                            }
                            .buttonStyle(.bordered)
                            .tint(Palette.danger)
                            .controlSize(.small)
                            .disabled(submitting)
                        } else {
                            Button(decision.label) {
                                submitApproval(payload: payload, action: decision.id, reason: nil)
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                            .disabled(submitting)
                        }
                    }
                    Spacer(minLength: 4)
                    if payload.canHandoff {
                        Button(L("回原终端")) {
                            Task {
                                if await store.handoff(session: session, payload: payload) {
                                    onCompleted()
                                }
                            }
                        }
                        .controlSize(.small)
                        .help(L("释放弹窗拦截并在原终端显示确认框"))
                        .disabled(submitting)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func questionsView(payload: InteractionPayload) -> some View {
        let draft = store.draftBinding(for: payload)
        let isSingleDirect = payload.questions.count == 1
            && !payload.questions[0].multiSelect
            && !payload.questions[0].options.isEmpty
        VStack(alignment: .leading, spacing: 10) {
            ForEach(payload.questions) { q in
                VStack(alignment: .leading, spacing: 6) {
                    HStack(alignment: .firstTextBaseline, spacing: 4) {
                        Text(q.question)
                            .font(.system(size: DesignTokens.fontEntry, weight: .medium))
                            .fixedSize(horizontal: false, vertical: true)
                        if q.multiSelect {
                            Text(L("（可多选）"))
                                .font(AppFont.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    ForEach(q.options) { opt in
                        let selected = isOptionSelected(question: q, option: opt, draft: draft)
                        VStack(alignment: .leading, spacing: 4) {
                            Button {
                                handleOptionTap(payload: payload, question: q, option: opt, directSubmit: isSingleDirect)
                            } label: {
                                HStack(alignment: .top, spacing: 6) {
                                    Image(systemName: q.multiSelect
                                        ? (selected ? "checkmark.square.fill" : "square")
                                        : (selected ? "largecircle.fill.circle" : "circle"))
                                        .font(.system(size: 12))
                                        .foregroundStyle(selected ? Palette.accent : .secondary)
                                        .padding(.top, 1)
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
                            }
                            .buttonStyle(.plain)
                            .disabled(submitting || !payload.canRespond)

                            if let preview = opt.preview, !preview.isEmpty {
                                let pKey = "\(q.id):\(opt.label)"
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

                    if payload.canRespond && (q.allowCustom || q.options.isEmpty) {
                        let customBinding = Binding<String>(
                            get: { store.draftBinding(for: payload).customAnswers[q.id] ?? "" },
                            set: { value in
                                store.updateDraft(for: payload) {
                                    $0.customAnswers[q.id] = value
                                    if !q.multiSelect && !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                        $0.singleChoices.removeValue(forKey: q.id)
                                    }
                                }
                            }
                        )
                        HStack(spacing: 6) {
                            TextField(q.options.isEmpty ? L("输入回答…") : L("其他自定义回答…"), text: customBinding)
                                .textFieldStyle(.aieyes)
                                .font(AppFont.caption)
                                .onSubmit {
                                    if isSingleDirect {
                                        submitQuestions(payload: payload)
                                    }
                                }
                            if isSingleDirect {
                                Button(L("发送")) {
                                    submitQuestions(payload: payload)
                                }
                                .controlSize(.small)
                                .disabled(submitting || customBinding.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                            }
                        }
                    }
                }
            }
            if !payload.canRespond {
                if !payload.readonlyReason.isEmpty {
                    Text(payload.readonlyReason)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                HStack(spacing: 6) {
                    if payload.needsClaudeHook {
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
                    if session.focusable {
                        Button(L("跳转到原窗口")) {
                            onFocusSession()
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                    }
                }
            } else if !isSingleDirect || payload.canHandoff {
                HStack(spacing: 6) {
                    if !isSingleDirect {
                        Button(submitting ? L("提交中…") : L("提交回答")) {
                            submitQuestions(payload: payload)
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                        .disabled(submitting || !allQuestionsAnswered(payload: payload, draft: draft))
                    }
                    Spacer(minLength: 4)
                    if payload.canHandoff {
                        Button(L("回原终端")) {
                            Task {
                                if await store.handoff(session: session, payload: payload) {
                                    onCompleted()
                                }
                            }
                        }
                        .controlSize(.small)
                        .disabled(submitting)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func textReplyView(payload: InteractionPayload) -> some View {
        let textBinding = Binding<String>(
            get: { store.draftBinding(for: payload).textReply },
            set: { value in store.updateDraft(for: payload) { $0.textReply = value } }
        )
        VStack(alignment: .leading, spacing: 8) {
            if !payload.textPrompt.isEmpty {
                ScrollView {
                    Text(payload.textPrompt)
                        .font(AppFont.caption)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 120)
            }
            TextField(L("输入回复内容…"), text: textBinding, axis: .vertical)
                .textFieldStyle(.aieyes)
                .lineLimit(2...5)
                .font(AppFont.caption)
            HStack(spacing: 6) {
                Text(payload.adapter == "tmux" ? L("通过 tmux 发送") : L("通过原终端发送"))
                    .font(.system(size: 10.5))
                    .foregroundStyle(.secondary)
                Spacer()
                Button(submitting ? L("发送中…") : L("发送")) {
                    let body = textBinding.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !body.isEmpty else { return }
                    Task {
                        if await store.respond(session: session, payload: payload, action: "text", text: body) {
                            scheduleCompletion()
                        }
                    }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
                .disabled(submitting || textBinding.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }

    @ViewBuilder
    private func readonlyView(payload: InteractionPayload) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if !payload.textPrompt.isEmpty {
                ScrollView {
                    Text(payload.textPrompt)
                        .font(AppFont.caption)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 130)
            }
            if !payload.readonlyReason.isEmpty {
                Text(payload.readonlyReason)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 6) {
                if payload.needsClaudeHook {
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
                if session.focusable {
                    Button(L("跳转到原窗口")) {
                        onFocusSession()
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                }
            }
        }
    }

    @ViewBuilder
    private func feedbackBanner(status: String, message: String) -> some View {
        let color: Color = switch status {
        case "confirmed", "submitted": Palette.ok
        case "blocked", "uncertain": Palette.warn
        default: Palette.danger
        }
        HStack(spacing: 5) {
            Image(systemName: status == "confirmed" || status == "submitted" ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                .font(.system(size: 11))
                .foregroundStyle(color)
            Text(message)
                .font(.system(size: 11))
                .foregroundStyle(color)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.top, 2)
    }

    private func isOptionSelected(question: InteractionQuestion, option: InteractionOption, draft: InteractionDraft) -> Bool {
        if question.multiSelect {
            return draft.multiChoices[question.id]?.contains(option.label) == true
        }
        return draft.singleChoices[question.id] == option.label
    }

    private func handleOptionTap(payload: InteractionPayload, question: InteractionQuestion, option: InteractionOption, directSubmit: Bool) {
        if question.multiSelect {
            store.updateDraft(for: payload) { draft in
                var set = draft.multiChoices[question.id] ?? []
                if set.contains(option.label) { set.remove(option.label) }
                else { set.insert(option.label) }
                draft.multiChoices[question.id] = set
            }
            return
        }
        store.updateDraft(for: payload) { draft in
            draft.singleChoices[question.id] = option.label
            draft.customAnswers.removeValue(forKey: question.id)
        }
        if directSubmit {
            submitQuestions(payload: payload)
        }
    }

    private func allQuestionsAnswered(payload: InteractionPayload, draft: InteractionDraft) -> Bool {
        for q in payload.questions {
            let custom = (draft.customAnswers[q.id] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            if q.multiSelect {
                let picked = draft.multiChoices[q.id] ?? []
                if picked.isEmpty && custom.isEmpty { return false }
            } else {
                let single = draft.singleChoices[q.id] ?? ""
                if single.isEmpty && custom.isEmpty { return false }
            }
        }
        return !payload.questions.isEmpty
    }

    private func submitApproval(payload: InteractionPayload, action: String, reason: String?) {
        Task {
            if await store.respond(session: session, payload: payload, action: action, reason: reason) {
                showDeclineReason = false
                scheduleCompletion()
            }
        }
    }

    private func submitQuestions(payload: InteractionPayload) {
        let draft = store.draftBinding(for: payload)
        var answers: [String: Any] = [:]
        for q in payload.questions {
            let custom = (draft.customAnswers[q.id] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            if q.multiSelect {
                var items = q.options.map(\.label).filter { draft.multiChoices[q.id]?.contains($0) == true }
                if !custom.isEmpty { items.append(custom) }
                answers[q.id] = items
            } else if !custom.isEmpty {
                answers[q.id] = custom
            } else if let picked = draft.singleChoices[q.id] {
                answers[q.id] = picked
            }
        }
        Task {
            if await store.respond(session: session, payload: payload, action: "answer", answers: answers) {
                scheduleCompletion()
            }
        }
    }

    private func scheduleCompletion() {
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(700))
            onCompleted()
        }
    }
}

private struct PetInteractionCodeBlock: View {
    let text: String
    var maxHeight: CGFloat = 110
    var muted = false

    @State private var contentHeight: CGFloat?

    var body: some View {
        ScrollView {
            Text(verbatim: text)
                .font(.system(size: DesignTokens.fontCaption, design: .monospaced))
                .foregroundStyle(muted ? Color.secondary : Color.primary)
                .lineSpacing(2)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(DesignTokens.spaceControl)
                .onGeometryChange(for: CGFloat.self) { geometry in
                    min(geometry.size.height, maxHeight)
                } action: { height in
                    contentHeight = height
                }
        }
        .scrollIndicators(.hidden)
        .scrollBounceBehavior(.basedOnSize)
        .frame(height: contentHeight ?? maxHeight)
        .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: DesignTokens.radiusControl))
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
                    L("在弹窗中回答 Claude / Qoder 提问选项"),
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
                    L("在弹窗中审批 Claude / Qoder 工具权限"),
                    isOn: Binding(
                        get: { store.status.claudeApprovals },
                        set: { value in Task { await store.configure(claudeApprovals: value) } }
                    )
                )
                .disabled(store.saving)
                Text(L("开启后会拦截 Claude / Qoder CLI 工具授权并在弹窗显示「允许本次 / 拒绝 / 回原终端」；应用退出或超时将自动交还终端。"))
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
