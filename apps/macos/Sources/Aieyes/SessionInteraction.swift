import AppKit
import Combine
import Foundation

struct InteractionHookEntry: Decodable, Equatable, Identifiable {
    var sourceId: String
    var provider: String
    var path: String
    var mode: String
    /// Whether the MCP form (`Elicitation`) hook is installed for this source.
    var elicitation: Bool? = nil
    var id: String { sourceId + ":" + path }
}

/// Whether a local Codex source exposes its control socket; without it Codex cards are read-only.
struct InteractionCodexControl: Decodable, Equatable {
    var sourceId: String
    var available: Bool
}

struct SessionInteractionStatus: Decodable, Equatable {
    var enabled: Bool
    var autoPopup: Bool
    var claudeQuestions: Bool
    var claudeApprovals: Bool
    var allowTerminal: Bool
    var desktopAllowTerminal: Bool?
    var feishuPaired: Bool?
    var hooks: [InteractionHookEntry]?
    var codexControl: [InteractionCodexControl]? = nil
}

private extension KeyedDecodingContainer {
    /// A missing, null or mistyped field reads as `fallback`, so cards from an older core still decode.
    func value<T: Decodable>(_ key: Key, _ fallback: T) -> T {
        (try? decodeIfPresent(T.self, forKey: key)) ?? fallback
    }
}

/// One button of an approval card. `id` goes back to core unchanged; the card renders from
/// `label`, `style` and `description` and never interprets ids it does not know.
struct InteractionDecision: Decodable, Equatable, Identifiable {
    var id: String
    var label: String
    /// `primary` | `secondary` | `danger`.
    var style: String
    /// `allow` | `deny` | `cancel`.
    var behavior = ""
    /// `once` | `turn` | `session` | `always`.
    var scope = "once"
    /// Core accepts a `reason` with this decision, and only with such a decision.
    var acceptsReason = false
    /// The decision also stops the agent's current turn.
    var interrupts = false
    var description = ""
}

extension InteractionDecision {
    private enum CodingKeys: String, CodingKey { case id, label, style, behavior, scope, acceptsReason, interrupts, description }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        id = try row.decode(String.self, forKey: .id)
        label = try row.decode(String.self, forKey: .label)
        style = row.value(.style, "secondary")
        behavior = row.value(.behavior, "")
        scope = row.value(.scope, "once")
        acceptsReason = row.value(.acceptsReason, false)
        interrupts = row.value(.interrupts, false)
        description = row.value(.description, "")
    }
}

struct InteractionPlan: Decodable, Equatable {
    var markdown: String
    var truncated = false
}

extension InteractionPlan {
    private enum CodingKeys: String, CodingKey { case markdown, truncated }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        markdown = row.value(.markdown, "")
        truncated = row.value(.truncated, false)
    }
}

/// A file touched by the request. `diff` is a unified diff when the agent supplies one;
/// otherwise `oldText` / `newText` hold the replaced and the new content.
struct InteractionFileChange: Decodable, Equatable {
    var path: String
    /// `add` | `update` | `delete` | `move` | `unknown`.
    var kind = "unknown"
    var movePath = ""
    var diff = ""
    var oldText = ""
    var newText = ""
    var truncated = false
}

extension InteractionFileChange {
    private enum CodingKeys: String, CodingKey { case path, kind, movePath, diff, oldText, newText, truncated }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        path = row.value(.path, "")
        kind = row.value(.kind, "unknown")
        movePath = row.value(.movePath, "")
        diff = row.value(.diff, "")
        oldText = row.value(.oldText, "")
        newText = row.value(.newText, "")
        truncated = row.value(.truncated, false)
    }
}

struct InteractionApproval: Decodable, Equatable {
    var toolName: String
    var reason: String
    var command: String
    var details: String
    /// `command` | `fileChange` | `plan` | `permissions` | `mcpTool` | `tool` | `generic`.
    var kind = "generic"
    var title = ""
    var cwd = ""
    var plan: InteractionPlan? = nil
    var changes: [InteractionFileChange] = []
    var paths: [String] = []
    /// Empty when the request can only be confirmed in its own window.
    var decisions: [InteractionDecision]
}

extension InteractionApproval {
    private enum CodingKeys: String, CodingKey { case toolName, reason, command, details, kind, title, cwd, plan, changes, paths, decisions }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        toolName = row.value(.toolName, "")
        reason = row.value(.reason, "")
        command = row.value(.command, "")
        details = row.value(.details, "")
        kind = row.value(.kind, "generic")
        title = row.value(.title, "")
        cwd = row.value(.cwd, "")
        plan = row.value(.plan, InteractionPlan?.none)
        changes = row.value(.changes, [])
        paths = row.value(.paths, [])
        // A decision that does not decode must fail the card: dropping it could hide the only safe choice.
        decisions = try row.decodeIfPresent([InteractionDecision].self, forKey: .decisions) ?? []
    }
}

/// One selectable answer. Options are identified by their position in the question, never by label:
/// two options may share a label, and core may supply a separate `value` to send back.
struct InteractionOption: Decodable, Equatable {
    var label: String
    var description: String?
    var preview: String?
    /// Core's string `value`, when it supplies one. Non-string values only set `hasValue`.
    var value: String?
    /// Whether core attached any non-null `value`; such options resolve by label or string value in core.
    var hasValue: Bool

    init(label: String, description: String? = nil, preview: String? = nil, value: String? = nil, hasValue: Bool? = nil) {
        self.label = label
        self.description = description
        self.preview = preview
        self.value = value
        self.hasValue = hasValue ?? (value != nil)
    }

    private enum CodingKeys: String, CodingKey { case label, description, preview, value }

    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        label = try row.decode(String.self, forKey: .label)
        description = try row.decodeIfPresent(String.self, forKey: .description)
        preview = try row.decodeIfPresent(String.self, forKey: .preview)
        value = try? row.decodeIfPresent(String.self, forKey: .value)
        hasValue = row.contains(.value) && (try? row.decodeNil(forKey: .value)) == false
    }
}

/// Limits a form field declares. Core checks them again; the card uses them to explain a refusal early.
struct InteractionConstraints: Decodable, Equatable {
    var minimum: Double? = nil
    var maximum: Double? = nil
    var minLength: Int? = nil
    var maxLength: Int? = nil
    var minItems: Int? = nil
    var maxItems: Int? = nil
    var pattern: String? = nil
    var format: String? = nil
}

extension InteractionConstraints {
    private enum CodingKeys: String, CodingKey { case minimum, maximum, minLength, maxLength, minItems, maxItems, pattern, format }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        func count(_ key: CodingKeys) -> Int? {
            row.value(key, Double?.none).flatMap { $0.isFinite && $0 >= 0 && $0 < 1e9 ? Int($0) : nil }
        }
        minimum = row.value(.minimum, Double?.none)
        maximum = row.value(.maximum, Double?.none)
        minLength = count(.minLength)
        maxLength = count(.maxLength)
        minItems = count(.minItems)
        maxItems = count(.maxItems)
        pattern = row.value(.pattern, String?.none)
        format = row.value(.format, String?.none)
    }
}

struct InteractionQuestion: Decodable, Equatable, Identifiable {
    var id: String
    var key: String?
    var question: String
    /// `string` | `multiselect` | `boolean` | `integer` | `number`.
    var kind: String?
    var multiSelect: Bool
    var allowCustom: Bool
    var options: [InteractionOption]
    /// Short label above the question (a form field's title or the tool's header).
    var header = ""
    /// The input must be masked and never echoed.
    var secret = false
    var required = true
    var description = ""
    var placeholder = ""
    /// The field's default, as the text the user would type or the option they would pick.
    var defaultValue: String? = nil
    var constraints: InteractionConstraints? = nil
}

extension InteractionQuestion {
    private enum CodingKeys: String, CodingKey {
        case id, key, question, kind, multiSelect, allowCustom, options, header, secret, required, description, placeholder, constraints
        case defaultValue = "default"
    }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        id = try row.decode(String.self, forKey: .id)
        key = row.value(.key, String?.none)
        question = row.value(.question, "")
        kind = row.value(.kind, String?.none)
        multiSelect = row.value(.multiSelect, false)
        options = try row.decodeIfPresent([InteractionOption].self, forKey: .options) ?? []
        allowCustom = row.value(.allowCustom, options.isEmpty)
        header = row.value(.header, "")
        secret = row.value(.secret, false)
        required = row.value(.required, true)
        description = row.value(.description, "")
        placeholder = row.value(.placeholder, "")
        constraints = row.value(.constraints, InteractionConstraints?.none)
        if let text = try? row.decodeIfPresent(String.self, forKey: .defaultValue) {
            defaultValue = text
        } else if let flag = try? row.decodeIfPresent(Bool.self, forKey: .defaultValue) {
            defaultValue = flag ? "true" : "false"
        } else if let number = try? row.decodeIfPresent(Double.self, forKey: .defaultValue), number.isFinite {
            defaultValue = number == number.rounded() && abs(number) < 9e15 ? String(Int64(number)) : String(number)
        }
    }
}

/// Form-level information of a question card (MCP elicitation or an agent's own form).
struct InteractionForm: Decodable, Equatable {
    var title = ""
    var description = ""
    /// `mcp` | `tool` | `agent`.
    var source = ""
    var serverName = ""
    var url = ""
    /// Core accepts `cancel` / `skip` only when these are true.
    var canCancel = false
    var canSkip = false
}

extension InteractionForm {
    private enum CodingKeys: String, CodingKey { case title, description, source, serverName, url, canCancel, canSkip }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        title = row.value(.title, "")
        description = row.value(.description, "")
        source = row.value(.source, "")
        serverName = row.value(.serverName, "")
        url = row.value(.url, "")
        canCancel = row.value(.canCancel, false)
        canSkip = row.value(.canSkip, false)
    }
}

/// Whether an instruction can be sent to a session that is not waiting for an answer.
struct InteractionFollowUp: Decodable, Equatable {
    var available = false
    var adapter = ""
    /// `newTurn` | `queue` | `terminal` | empty when unavailable.
    var mode = ""
    /// Why it is unavailable, already localized.
    var reason = ""
    /// The `interrupt` action can stop the session's running turn.
    var canInterrupt = false
}

extension InteractionFollowUp {
    private enum CodingKeys: String, CodingKey { case available, adapter, mode, reason, canInterrupt }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        available = row.value(.available, false)
        adapter = row.value(.adapter, "")
        mode = row.value(.mode, "")
        reason = row.value(.reason, "")
        canInterrupt = row.value(.canInterrupt, false)
    }
}

/// One queued request of a session that lists its queue (OpenCode); passing `requestId` to inspect shows it.
struct InteractionPendingRequest: Decodable, Equatable {
    var requestId: String
    var kind = ""
    var title = ""
}

extension InteractionPendingRequest {
    private enum CodingKeys: String, CodingKey { case requestId, kind, title }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        requestId = row.value(.requestId, "")
        kind = row.value(.kind, "")
        title = row.value(.title, "")
    }
}

/// `sessions.interaction.inspect` result (docs/session-interaction.md).
struct InteractionPayload: Decodable, Equatable {
    var sessionId: String
    var stateKey: String
    var promptHash: String
    /// `none` | `approval` | `questions` | `text` | `followup` | `readonly`.
    var kind: String
    var adapter: String
    var canRespond: Bool
    var canHandoff: Bool
    var needsClaudeHook: Bool
    var requestId: String
    var title: String
    var approval: InteractionApproval?
    var questions: [InteractionQuestion]
    var textPrompt: String
    var readonlyReason: String
    /// `local` | `ssh`.
    var transport = "local"
    /// Requests queued on the session including the shown one; 0 when none.
    var pendingCount = 0
    var pendingRequests: [InteractionPendingRequest] = []
    /// Unix seconds a hook request was registered; 0 when unknown.
    var requestedAt: Double = 0
    /// `approval` | `elicitation` | `question` | `notification` | empty.
    var inputKind = ""
    /// Just answered from the popup; the session has not caught up yet.
    var answered = false
    /// `question` | `lastMessage` | `none`: what `textPrompt` holds.
    var promptSource = "none"
    var form: InteractionForm? = nil
    var followUp = InteractionFollowUp()
    var needsClaudeApprovals = false
    /// `questions` | `approvals` | empty: the hook switch that would make this request answerable here.
    var quickEnable = ""
}

extension InteractionPayload {
    private enum CodingKeys: String, CodingKey {
        case sessionId, stateKey, promptHash, kind, adapter, canRespond, canHandoff, needsClaudeHook, requestId, title, approval, questions
        case textPrompt, readonlyReason, transport, pendingCount, pendingRequests, requestedAt, inputKind, answered, promptSource, form, followUp
        case needsClaudeApprovals, quickEnable
    }
    init(from decoder: Decoder) throws {
        let row = try decoder.container(keyedBy: CodingKeys.self)
        // What identifies the request and what may be answered is never defaulted.
        sessionId = try row.decode(String.self, forKey: .sessionId)
        stateKey = try row.decode(String.self, forKey: .stateKey)
        promptHash = try row.decode(String.self, forKey: .promptHash)
        kind = try row.decode(String.self, forKey: .kind)
        adapter = try row.decode(String.self, forKey: .adapter)
        canRespond = try row.decode(Bool.self, forKey: .canRespond)
        canHandoff = row.value(.canHandoff, false)
        needsClaudeHook = row.value(.needsClaudeHook, false)
        requestId = row.value(.requestId, "")
        title = row.value(.title, "")
        approval = try row.decodeIfPresent(InteractionApproval.self, forKey: .approval)
        questions = try row.decodeIfPresent([InteractionQuestion].self, forKey: .questions) ?? []
        textPrompt = row.value(.textPrompt, "")
        readonlyReason = row.value(.readonlyReason, "")
        transport = row.value(.transport, "local")
        pendingCount = Int(min(max(row.value(.pendingCount, 0.0), 0), 10_000))
        pendingRequests = row.value(.pendingRequests, [])
        requestedAt = row.value(.requestedAt, 0.0)
        inputKind = row.value(.inputKind, "")
        answered = row.value(.answered, false)
        promptSource = row.value(.promptSource, "none")
        form = row.value(.form, InteractionForm?.none)
        followUp = row.value(.followUp, InteractionFollowUp())
        needsClaudeApprovals = row.value(.needsClaudeApprovals, false)
        quickEnable = row.value(.quickEnable, "")
    }
}

struct InteractionTerminalJob: Decodable {
    var text: String
    var session: LiveSession
}

struct InteractionRespondResult: Decodable {
    var status: String
    var message: String
    var terminalJob: InteractionTerminalJob?
}

struct InteractionHandoffResult: Decodable {
    var handedOff: Bool
    var message: String
    /// False while the hand-back marker is written but the hook has not picked it up yet.
    var confirmed: Bool?
    var requestId: String?
}

struct InteractionDraft: Equatable {
    /// Question id to the selected option's index in `InteractionAnswers.options(for:)`.
    var singleChoices: [String: Int] = [:]
    var multiChoices: [String: Set<Int>] = [:]
    /// Typed answers: free text, numbers, secrets and the "other" field next to options.
    var customAnswers: [String: String] = [:]
    /// Reply of a text card or instruction of a follow-up card.
    var textReply: String = ""
    /// Note sent with a decision that accepts one.
    var declineReason: String = ""
    var hasUserInput: Bool {
        !singleChoices.isEmpty
            || multiChoices.values.contains(where: { !$0.isEmpty })
            || customAnswers.values.contains(where: { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
            || !textReply.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !declineReason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

/// Pure answer logic for question cards, shared by the card view and its native verifier.
enum InteractionAnswers {
    /// ASCII-only case folding, matching core's `eq_ignore_ascii_case`.
    private static func folded(_ text: String) -> [UInt8] {
        text.utf8.map { $0 >= 65 && $0 <= 90 ? $0 + 32 : $0 }
    }

    static func isMulti(_ question: InteractionQuestion) -> Bool {
        question.multiSelect || question.kind == "multiselect"
    }

    /// A yes/no field that arrives without options is shown as two; its answer is sent as a boolean.
    static func isBareBoolean(_ question: InteractionQuestion) -> Bool {
        question.kind == "boolean" && question.options.isEmpty
    }

    /// The options the card shows and that draft indices refer to.
    static func options(for question: InteractionQuestion) -> [InteractionOption] {
        isBareBoolean(question)
            ? [InteractionOption(label: L("是"), value: "true"), InteractionOption(label: L("否"), value: "false")]
            : question.options
    }

    /// Whether the card shows a typed field for this question.
    static func acceptsTyped(_ question: InteractionQuestion) -> Bool {
        !isBareBoolean(question) && (question.allowCustom || question.options.isEmpty)
    }

    /// The option core picks for an answer string: the first whose label matches ignoring ASCII case
    /// or whose string value equals it exactly.
    static func resolvedIndex(of answer: String, in options: [InteractionOption]) -> Int? {
        let key = folded(answer)
        return options.firstIndex { folded($0.label) == key || $0.value == answer }
    }

    /// The string to send for the option at `index`, or nil when no string selects exactly that option.
    /// A nil result must be refused in the UI: sending the label would answer with a different option.
    static func wireAnswer(for index: Int, in question: InteractionQuestion) -> String? {
        let options = options(for: question)
        guard options.indices.contains(index) else { return nil }
        let option = options[index]
        if isBareBoolean(question) { return option.value }
        // Without core values the label is the answer itself, so equal labels are the same answer.
        guard options.contains(where: \.hasValue) else { return option.label }
        for candidate in [option.label, option.value].compactMap({ $0 }) where resolvedIndex(of: candidate, in: options) == index {
            return candidate
        }
        return nil
    }

    static func customAnswer(for question: InteractionQuestion, in draft: InteractionDraft) -> String {
        (draft.customAnswers[question.id] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The draft a card starts from: each field's declared default, so an untouched form sends what
    /// the original dialog would. Secrets never start filled.
    static func initialDraft(for payload: InteractionPayload) -> InteractionDraft {
        var draft = InteractionDraft()
        guard payload.kind == "questions", payload.canRespond else { return draft }
        for question in payload.questions where !question.secret {
            guard let value = question.defaultValue?.trimmingCharacters(in: .whitespacesAndNewlines), !value.isEmpty else { continue }
            let options = options(for: question)
            if options.isEmpty {
                draft.customAnswers[question.id] = value
            } else if !isMulti(question),
                      let index = options.firstIndex(where: { $0.value == value }) ?? options.firstIndex(where: { folded($0.label) == folded(value) }),
                      wireAnswer(for: index, in: question) != nil {
                draft.singleChoices[question.id] = index
            }
        }
        return draft
    }

    /// Why the current input of this question would be refused, as a catalogue key; nil when acceptable
    /// or empty. Core validates again, so this only saves a round trip.
    static func problem(with question: InteractionQuestion, in draft: InteractionDraft) -> String? {
        let limits = question.constraints
        if isMulti(question) {
            let picked = (draft.multiChoices[question.id] ?? []).count + (customAnswer(for: question, in: draft).isEmpty ? 0 : 1)
            guard picked > 0 else { return nil }
            if let low = limits?.minItems, picked < low { return "选择的数量不符合要求" }
            if let high = limits?.maxItems, picked > high { return "选择的数量不符合要求" }
            return nil
        }
        let typed = customAnswer(for: question, in: draft)
        guard !typed.isEmpty, acceptsTyped(question) else { return nil }
        // Core refuses control characters; a pasted secret or note may carry them invisibly.
        if typed.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) && $0 != "\n" && $0 != "\t" }) { return "输入内容无效或过长" }
        if typed.utf8.count > 2000 { return "输入内容无效或过长" }
        switch question.kind {
        case "integer", "number":
            let number: Double?
            if question.kind == "integer" {
                guard let whole = Int64(typed) else { return "请输入有效的整数" }
                number = Double(whole)
            } else {
                number = Double(typed).flatMap { $0.isFinite ? $0 : nil }
            }
            guard let number else { return "请输入有效的数字" }
            if let low = limits?.minimum, number < low { return "数值超出允许范围" }
            if let high = limits?.maximum, number > high { return "数值超出允许范围" }
        default:
            guard options(for: question).isEmpty else { return nil }
            if let low = limits?.minLength, typed.count < low { return "文本长度不符合要求" }
            if let high = limits?.maxLength, typed.count > high { return "文本长度不符合要求" }
        }
        return nil
    }

    static func isAnswered(_ question: InteractionQuestion, in draft: InteractionDraft) -> Bool {
        if acceptsTyped(question), !customAnswer(for: question, in: draft).isEmpty { return true }
        if isMulti(question) {
            return (draft.multiChoices[question.id] ?? []).contains { wireAnswer(for: $0, in: question) != nil }
        }
        return draft.singleChoices[question.id].flatMap { wireAnswer(for: $0, in: question) } != nil
    }

    /// Questions that keep the card from being submitted: required ones without an answer and
    /// any whose input would be refused.
    static func unansweredIndices(in payload: InteractionPayload, draft: InteractionDraft) -> [Int] {
        payload.questions.indices.filter { index in
            let question = payload.questions[index]
            if problem(with: question, in: draft) != nil { return true }
            return question.required && !isAnswered(question, in: draft)
        }
    }

    static func answeredCount(in payload: InteractionPayload, draft: InteractionDraft) -> Int {
        payload.questions.filter { isAnswered($0, in: draft) && problem(with: $0, in: draft) == nil }.count
    }

    /// Every required question is answered, nothing would be refused, and there is something to send:
    /// a form of optional fields still needs one answer.
    static func allAnswered(in payload: InteractionPayload, draft: InteractionDraft) -> Bool {
        !payload.questions.isEmpty && unansweredIndices(in: payload, draft: draft).isEmpty && answeredCount(in: payload, draft: draft) > 0
    }

    /// A lone single-select question answers on click, without a separate submit step. Forms never do:
    /// they offer cancel and skip, and a filled field deserves a look before it is sent.
    static func isSingleDirect(_ payload: InteractionPayload) -> Bool {
        guard payload.questions.count == 1, payload.form == nil, let question = payload.questions.first else { return false }
        return !isMulti(question) && !question.options.isEmpty && !question.secret
    }

    /// `sessions.interaction.respond` answers: question id to a string, to an array for multi-select,
    /// or to a boolean for a yes/no field without options. Unanswered optional questions are left out,
    /// because core refuses keys it cannot match to an answer.
    static func answers(for payload: InteractionPayload, draft: InteractionDraft) -> [String: Any] {
        var answers: [String: Any] = [:]
        for question in payload.questions {
            let custom = acceptsTyped(question) ? customAnswer(for: question, in: draft) : ""
            if isMulti(question) {
                let picked = draft.multiChoices[question.id] ?? []
                var items = options(for: question).indices.filter(picked.contains).compactMap { wireAnswer(for: $0, in: question) }
                if !custom.isEmpty { items.append(custom) }
                if !items.isEmpty { answers[question.id] = items }
            } else if !custom.isEmpty {
                answers[question.id] = custom
            } else if let index = draft.singleChoices[question.id], let answer = wireAnswer(for: index, in: question) {
                answers[question.id] = isBareBoolean(question) ? (answer == "true") as Any : answer as Any
            }
        }
        return answers
    }

    enum ReturnStep: Equatable { case page(Int), submit }

    /// What Return does on a questions card showing `page`. A plain Return (`explicit` false) on a
    /// paged form only moves on: later pages may hold optional or pre-filled fields the user has not
    /// seen, so the form is sent from its last page, by ⌘Return or by the submit button.
    static func returnStep(in payload: InteractionPayload, draft: InteractionDraft, page: Int, explicit: Bool) -> ReturnStep {
        let blocking = unansweredIndices(in: payload, draft: draft)
        if !explicit, page < payload.questions.count - 1 {
            // A page that still needs its answer, or holds a refused value, keeps the user on it.
            return .page(blocking.contains(page) ? page : page + 1)
        }
        if let first = blocking.first { return .page(first) }
        return .submit
    }

    /// Plain digit keys 1-9 select the matching option; anything else is not an option key.
    static func optionIndex(forKey characters: String?, optionCount: Int) -> Int? {
        guard let characters, characters.count == 1, let digit = Int(characters), digit >= 1, digit <= optionCount else { return nil }
        return digit - 1
    }
}

/// What the orbit popover should do with a key press after the visible card has seen it.
enum InteractionKeyOutcome: Equatable { case passThrough, handled, close }

/// When the orbit popover closes, and whether closing counts as the user dismissing its requests.
enum InteractionPopoverPolicy {
    enum Cause { case explicit, otherAppClick, ownWindowClick, appDeactivated, petDrag, panelOpened, resolved }

    /// An auto-opened card appears while the user works elsewhere, so clicks in other apps and
    /// deactivation leave it up. Unsent input survives every outside click.
    ///
    /// `.resolved` is a session update: an auto-opened popover whose agent has no request waiting
    /// any more (answered in the terminal, session ended) has nothing left to ask. Nobody opened it
    /// by hand and outside clicks do not close it, so it goes away by itself; otherwise it would
    /// stay over other apps and hold back every later auto-popup. A popover the user opened stays.
    static func shouldClose(_ cause: Cause, autoOpened: Bool, hasDraft: Bool, hasWaiting: Bool = true) -> Bool {
        switch cause {
        case .explicit, .petDrag, .panelOpened: true
        case .otherAppClick, .appDeactivated: !autoOpened && !hasDraft
        case .ownWindowClick: !hasDraft
        case .resolved: autoOpened && !hasWaiting && !hasDraft
        }
    }

    /// A follow-up card belongs to the agent whose popover picked it. Showing another agent's
    /// popover drops the choice (the typed text stays with its session), so a draft that is not on
    /// screen never holds that popover open.
    static func keepsFollowUp(shownProvider: String?, nextProvider: String) -> Bool {
        shownProvider == nextProvider
    }

    /// Only an explicit close (Esc, clicking the badge again) dismisses the provider's waiting
    /// requests; requests that queued up behind the popover still get their own auto-popup.
    static func marksDismissed(_ cause: Cause) -> Bool { cause == .explicit }
}

/// Keyboard rules of the interaction card, kept free of view state so the native verifier covers them.
enum InteractionKeyboard {
    enum Field: Hashable { case declineReason, reply, custom(String) }
    enum Action: Equatable {
        case passThrough, close, collapseDecline, defocus
        case selectOption(Int), submit, confirmDecline, approve
    }
    struct Context: Equatable {
        var kind: String
        /// Whether the card accepts answers now: respondable, not submitting and not already answered.
        var interactive: Bool
        var showDeclineReason = false
        /// Options of the question that digit keys address; 0 when there is none.
        var optionCount = 0
        var hasPrimaryDecision = false
        var focus: Field?
        /// A text view owns the keyboard, whether or not the card knows which field it is.
        var editingText = false
        /// An input method holds uncommitted text; every key belongs to it.
        var composing = false
    }

    static func action(keyCode: UInt16, characters: String?, modifiers: NSEvent.ModifierFlags, context: Context) -> Action {
        // Return and Esc confirm or cancel an input-method candidate first.
        if context.composing { return .passThrough }
        let flags = modifiers.intersection([.command, .option, .control, .shift])
        if keyCode == 53 {
            guard flags.isEmpty else { return .passThrough }
            if context.showDeclineReason { return .collapseDecline }
            // The first Esc leaves the field and keeps its draft; the second closes the popover.
            return context.editingText ? .defocus : .close
        }
        guard context.interactive else { return .passThrough }
        if keyCode == 36 || keyCode == 76 {
            // A field of another card owns the keyboard: its ⌘Return must not approve or submit here.
            if context.editingText && context.focus == nil { return .passThrough }
            // Typing a note: Return and ⌘Return both send the noted decision, never the primary one.
            if context.showDeclineReason && context.focus == .declineReason {
                return flags.isEmpty || flags == .command ? .confirmDecline : .passThrough
            }
            if flags == .command {
                switch context.kind {
                // Approving needs the modifier: a stray Return must never allow a command. It is also
                // off while a note is open or a text field owns the keyboard.
                case "approval":
                    return context.hasPrimaryDecision && !context.showDeclineReason && !context.editingText ? .approve : .passThrough
                case "questions", "text", "followup": return .submit
                default: return .passThrough
                }
            }
            guard flags.isEmpty else { return .passThrough }
            if context.focus == .declineReason { return .confirmDecline }
            switch context.kind {
            case "questions", "text": return .submit
            // An instruction to a session that is not asking is only sent from its own field.
            case "followup": return context.focus == .reply ? .submit : .passThrough
            default: return .passThrough
            }
        }
        if flags.isEmpty, !context.editingText, context.kind == "questions",
           let index = InteractionAnswers.optionIndex(forKey: characters, optionCount: context.optionCount) {
            return .selectOption(index)
        }
        return .passThrough
    }
}

/// Manages desktop waiting-for-input interactions, drafts, and auto-popup deduplication.
@MainActor final class SessionInteractionStore: ObservableObject {
    @Published private(set) var status = SessionInteractionStatus(
        enabled: true,
        autoPopup: false,
        claudeQuestions: false,
        claudeApprovals: false,
        allowTerminal: false,
        desktopAllowTerminal: false,
        feishuPaired: false,
        hooks: []
    )
    @Published private(set) var saving = false
    @Published private(set) var error: String?
    @Published private(set) var payloads: [String: InteractionPayload] = [:]
    @Published private(set) var loadingSessionIDs: Set<String> = []
    @Published private(set) var submittingSessionIDs: Set<String> = []
    @Published private(set) var feedback: [String: (status: String, message: String)] = [:]
    @Published var drafts: [String: InteractionDraft] = [:]
    /// The session whose follow-up card the open popover shows (not waiting, picked by the user); nil when none.
    @Published var followUpSessionID: String?
    /// Draft keys of prompts already answered from here. Their cards stay locked until the prompt changes
    /// or the user refreshes, so a second click cannot answer whatever replaced the prompt.
    @Published private(set) var completedKeys: Set<String> = []

    /// Called when a newly observed `needsInput` session should auto-popup its orbit agent card.
    /// Returns whether the card was shown; a refused request stays queued and is offered again later.
    var onAutoPopupRequested: ((LiveSession) -> Bool)?
    /// Called after every session update, once stale cards and drafts are pruned and before queued
    /// requests are offered. The popover host closes an auto-opened popover that has nothing left to ask.
    var onSessionsObserved: (() -> Void)?
    /// Key handlers of the visible cards, in the order they appeared; removed when a card disappears.
    private var keyInterceptors: [(owner: UUID, handle: (NSEvent) -> InteractionKeyOutcome)] = []
    /// Delivers a `terminalJob` to the session's own terminal. Replaced by the native verifier.
    var deliverToTerminal: (LiveSession, String, EngineClient) async -> SessionChannelAutomation.Outcome = { session, text, client in
        await SessionChannelAutomation.deliver(session, text: text, client: client)
    }
    /// Brings the session's window forward after a hand-back. Replaced by the native verifier.
    var focusSession: (LiveSession) async -> Void = { session in
        _ = await SessionFocus.focus(session, engine: SessionFocus.engine)
    }
    /// Only this client can type a `terminalJob` into the user's terminal, so it says so on every call.
    static let capabilities: [String: Any] = ["localTerminal": true]

    private let client = EngineClient(configurationOnly: true)
    private var subscriptions = Set<AnyCancellable>()
    private var dismissedKeys = Set<String>()
    private var knownWaitingKeys = Set<String>()
    /// Dedup keys waiting for their turn to auto-popup, oldest first.
    private var pendingAutoPopup: [String] = []
    private var latestWaiting: [LiveSession] = []
    /// Lock key to its session and whether that session was waiting when the lock was set.
    private var completedSessions: [String: (id: String, waiting: Bool)] = [:]
    /// Session id to whether the session was waiting when its card was read.
    private var payloadWaiting: [String: Bool] = [:]
    /// Session id to the queued request the user chose to look at (sessions that list their queue).
    private var selectedRequests: [String: String] = [:]
    private var queuedInspects = Set<String>()
    /// Session id to the input state key the running inspect was asked for.
    private var inflightStateKeys: [String: String] = [:]
    /// Session id to whether the session was waiting in the latest call that reached the running inspect.
    private var inflightWaiting: [String: Bool] = [:]
    private var preservedFeedback = Set<String>()
    /// Session id to the input state key a recovery inspect was already issued for.
    private var recoveredStateKeys: [String: String] = [:]
    private var seeded = false
    private weak var model: AppModel?

    func start(_ model: AppModel) {
        self.model = model
        Task { await refreshStatus() }
        // `sessions` starts empty; the baseline must be the first list core actually returned,
        // or everything already waiting at launch would count as newly waiting.
        model.live.$sessions.combineLatest(model.live.$loaded)
            .filter { $0.1 }
            .map { $0.0 }
            .receive(on: DispatchQueue.main)
            .sink { [weak self] sessions in
                self?.observeSessions(sessions)
            }
            .store(in: &subscriptions)
        // A request refused while the pet was hidden (fullscreen, pet turned off) is offered again
        // when the pet is back; an unchanged session list would never offer it.
        model.live.$petVisible
            .removeDuplicates()
            .filter { $0 }
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.drainAutoPopup() }
            .store(in: &subscriptions)
    }

    func stop() {
        subscriptions.removeAll()
        client.stop()
    }

    func refreshStatus() async {
        do {
            let next: SessionInteractionStatus = try await client.call("sessions.interaction.status")
            if next != status { status = next }
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    func configure(
        enabled: Bool? = nil,
        autoPopup: Bool? = nil,
        claudeQuestions: Bool? = nil,
        claudeApprovals: Bool? = nil,
        allowTerminal: Bool? = nil
    ) async {
        guard !saving else { return }
        saving = true
        defer { saving = false }
        var params: [String: Any] = [:]
        if let enabled { params["enabled"] = enabled }
        if let autoPopup { params["autoPopup"] = autoPopup }
        if let claudeQuestions { params["claudeQuestions"] = claudeQuestions }
        if let claudeApprovals { params["claudeApprovals"] = claudeApprovals }
        if let allowTerminal { params["allowTerminal"] = allowTerminal }
        do {
            let next: SessionInteractionStatus = try await client.call("sessions.interaction.configure", params: params)
            status = next
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    static func dedupKey(for session: LiveSession) -> String {
        let state = session.inputStateKey ?? "\(session.turnId):\(Int(session.updatedAt.timeIntervalSince1970))"
        return "\(session.id)|\(state)"
    }

    /// Records that these waiting sessions were shown and closed, so they do not auto-popup again.
    func markDismissed(sessions: [LiveSession]) {
        for session in sessions where session.phase == .needsInput {
            dismissedKeys.insert(Self.dedupKey(for: session))
        }
        pendingAutoPopup.removeAll(where: dismissedKeys.contains)
    }

    static let followUpSuffix = "|followup"

    /// Identifies the prompt a draft or an answered-lock belongs to. A follow-up is addressed to the
    /// session rather than to a request, so its unsent text survives the turn ending underneath it.
    func draftKey(for payload: InteractionPayload) -> String {
        payload.kind == "followup"
            ? payload.sessionId + Self.followUpSuffix
            : "\(payload.sessionId)|\(payload.stateKey)|\(payload.promptHash)"
    }

    func hasActiveDraft(for sessionID: String) -> Bool {
        guard let payload = payloads[sessionID], !isCompleted(payload) else { return false }
        // Declared defaults are not something the user typed.
        guard let draft = drafts[draftKey(for: payload)], draft != InteractionAnswers.initialDraft(for: payload) else { return false }
        return draft.hasUserInput
    }

    /// Whether any of these sessions holds unsent input; such a popover must not close on an outside click.
    func hasActiveDraft(in sessions: [LiveSession]) -> Bool {
        sessions.contains { hasActiveDraft(for: $0.id) }
    }

    func isCompleted(_ payload: InteractionPayload) -> Bool {
        completedKeys.contains(draftKey(for: payload))
    }

    /// `keepDraft` is for an unconfirmed delivery: the card locks, but the text stays for a retry after refresh.
    private func markCompleted(_ payload: InteractionPayload, session: LiveSession, keepDraft: Bool = false) {
        let key = draftKey(for: payload)
        completedKeys.insert(key)
        completedSessions[key] = (session.id, session.phase == .needsInput)
        if !keepDraft { drafts.removeValue(forKey: key) }
        dismissedKeys.insert(Self.dedupKey(for: session))
        pendingAutoPopup.removeAll(where: dismissedKeys.contains)
    }

    /// A neutral line under the card, for something that worked but changes nothing on screen yet.
    func note(_ message: String, for session: LiveSession) {
        feedback[session.id] = ("note", message)
        preservedFeedback.insert(session.id)
    }

    /// Manual refresh: unlocks a card that was answered but is still waiting, then reloads it.
    func refresh(session: LiveSession) async {
        for (key, lock) in completedSessions where lock.id == session.id {
            completedKeys.remove(key)
            completedSessions.removeValue(forKey: key)
        }
        feedback.removeValue(forKey: session.id)
        await inspect(session: session, force: true)
    }

    func registerKeyInterceptor(owner: UUID, _ handle: @escaping (NSEvent) -> InteractionKeyOutcome) {
        if let index = keyInterceptors.firstIndex(where: { $0.owner == owner }) {
            keyInterceptors[index].handle = handle
        } else {
            keyInterceptors.append((owner, handle))
        }
    }

    func unregisterKeyInterceptor(owner: UUID) {
        keyInterceptors.removeAll { $0.owner == owner }
    }

    /// Routes a key press in the orbit popover to the visible cards; the first one that acts on it wins,
    /// so one key never answers two cards. Without a visible card, Esc closes the popover.
    func handleKey(_ event: NSEvent) -> InteractionKeyOutcome {
        guard !keyInterceptors.isEmpty else { return event.keyCode == 53 ? .close : .passThrough }
        for interceptor in keyInterceptors {
            let outcome = interceptor.handle(event)
            if outcome != .passThrough { return outcome }
        }
        return .passThrough
    }

    func draftBinding(for payload: InteractionPayload) -> InteractionDraft {
        drafts[draftKey(for: payload)] ?? InteractionAnswers.initialDraft(for: payload)
    }

    func updateDraft(for payload: InteractionPayload, _ mutate: (inout InteractionDraft) -> Void) {
        let key = draftKey(for: payload)
        var current = drafts[key] ?? InteractionAnswers.initialDraft(for: payload)
        mutate(&current)
        drafts[key] = current
    }

    /// Internal rather than private so the native verifier can feed session lists directly.
    func observeSessions(_ sessions: [LiveSession]) {
        let activeWaiting = sessions.filter { $0.phase == .needsInput }
        let currentKeys = Set(activeWaiting.map(Self.dedupKey))
        latestWaiting = activeWaiting
        if !seeded {
            seeded = true
            knownWaitingKeys = currentKeys
            return
        }
        // Prune state of sessions that left, and cards read while the session was in the other state:
        // a waiting card must not outlive its wait, nor a follow-up card the start of a new wait.
        let waitingIDs = Set(activeWaiting.map(\.id))
        let knownIDs = Set(sessions.map(\.id))
        for id in Array(payloads.keys) where !knownIDs.contains(id) || payloadWaiting[id] != waitingIDs.contains(id) {
            payloads.removeValue(forKey: id)
            payloadWaiting.removeValue(forKey: id)
            feedback.removeValue(forKey: id)
            selectedRequests.removeValue(forKey: id)
        }
        for id in Array(feedback.keys) where !knownIDs.contains(id) { feedback.removeValue(forKey: id) }
        for (key, lock) in completedSessions where !knownIDs.contains(lock.id) || lock.waiting != waitingIDs.contains(lock.id) {
            completedKeys.remove(key)
            completedSessions.removeValue(forKey: key)
        }
        for id in Array(recoveredStateKeys.keys) where !waitingIDs.contains(id) {
            recoveredStateKeys.removeValue(forKey: id)
        }
        // Drafts outlive a closed popover, but not the wait they were written for. An unsent
        // follow-up belongs to its session and lasts as long as the session is listed.
        for key in Array(drafts.keys) {
            let keep = key.hasSuffix(Self.followUpSuffix)
                ? knownIDs.contains(String(key.dropLast(Self.followUpSuffix.count)))
                : waitingIDs.contains(where: { key.hasPrefix($0 + "|") })
            if !keep { drafts.removeValue(forKey: key) }
        }
        let newlyWaiting = activeWaiting.filter { session in
            let key = Self.dedupKey(for: session)
            return !knownWaitingKeys.contains(key) && !dismissedKeys.contains(key)
        }
        knownWaitingKeys = currentKeys
        // Keep dismissed set bounded to active keys.
        dismissedKeys = dismissedKeys.intersection(currentKeys)
        pendingAutoPopup.removeAll { !currentKeys.contains($0) || dismissedKeys.contains($0) }
        if status.enabled && status.autoPopup {
            for key in newlyWaiting.map(Self.dedupKey) where !pendingAutoPopup.contains(key) {
                pendingAutoPopup.append(key)
            }
        }
        recoverStalePayloads(activeWaiting)
        onSessionsObserved?()
        drainAutoPopup()
    }

    /// Offers the oldest queued request to the popover host. At most one card opens per call; a refused
    /// request (another popover open, pet hidden or dragged) keeps its place and is not marked dismissed.
    /// The host calls this again whenever a popover closes or a pet drag ends; the store does when
    /// the pet becomes visible again.
    func drainAutoPopup() {
        guard status.enabled && status.autoPopup else {
            pendingAutoPopup.removeAll()
            return
        }
        while let key = pendingAutoPopup.first {
            guard !dismissedKeys.contains(key),
                  let session = latestWaiting.first(where: { Self.dedupKey(for: $0) == key }) else {
                pendingAutoPopup.removeFirst()
                continue
            }
            guard let onAutoPopupRequested, onAutoPopupRequested(session) else { return }
            pendingAutoPopup.removeAll { $0 == key }
            dismissedKeys.insert(key)
            return
        }
    }

    /// A loaded card whose state key no longer matches its session shows an old prompt. Reload it once
    /// per new key; the per-key record keeps a lagging session list from causing a reload loop.
    private func recoverStalePayloads(_ waiting: [LiveSession]) {
        for session in waiting {
            guard let key = session.inputStateKey,
                  let payload = payloads[session.id],
                  payload.stateKey != key,
                  recoveredStateKeys[session.id] != key,
                  !loadingSessionIDs.contains(session.id) else { continue }
            recoveredStateKeys[session.id] = key
            Task { await inspect(session: session, force: true) }
        }
    }

    /// Shows another queued request of a session that lists its queue (`pendingRequests`).
    func selectRequest(session: LiveSession, requestID: String) async {
        guard selectedRequests[session.id] != requestID else { return }
        selectedRequests[session.id] = requestID
        feedback.removeValue(forKey: session.id)
        await inspect(session: session, force: true)
    }

    /// `preserveFeedback` keeps the current banner, for the reload that follows a blocked response.
    /// Sessions that are not waiting are inspected too: their card says whether a follow-up
    /// instruction or an interrupt is possible.
    func inspect(session: LiveSession, force: Bool = false, preserveFeedback: Bool = false) async {
        let waiting = session.phase == .needsInput
        let expectedKey = session.inputStateKey
        if !force,
           let existing = payloads[session.id],
           payloadWaiting[session.id] == waiting,
           !waiting || expectedKey == nil || existing.stateKey == expectedKey,
           existing.kind != "none" {
            return
        }
        if preserveFeedback { preservedFeedback.insert(session.id) }
        if loadingSessionIDs.contains(session.id) {
            // The running request may return the prompt from before this call; ask again once it lands.
            // A call for the state key already being fetched adds nothing. A session that started or
            // stopped waiting meanwhile has another card, so that is fetched again even without force.
            let flipped = inflightWaiting[session.id] != waiting
            if flipped || (force && (expectedKey == nil || inflightStateKeys[session.id] != expectedKey)) {
                queuedInspects.insert(session.id)
                inflightStateKeys[session.id] = expectedKey
                inflightWaiting[session.id] = waiting
            }
            return
        }
        loadingSessionIDs.insert(session.id)
        inflightStateKeys[session.id] = expectedKey
        inflightWaiting[session.id] = waiting
        if let expectedKey { recoveredStateKeys[session.id] = expectedKey }
        defer {
            loadingSessionIDs.remove(session.id)
            queuedInspects.remove(session.id)
            inflightStateKeys.removeValue(forKey: session.id)
            inflightWaiting.removeValue(forKey: session.id)
            preservedFeedback.remove(session.id)
        }
        var rounds = 0
        repeat {
            queuedInspects.remove(session.id)
            rounds += 1
            do {
                var params: [String: Any] = ["sessionId": session.id, "capabilities": Self.capabilities]
                if let requestID = selectedRequests[session.id] { params["requestId"] = requestID }
                let result: InteractionPayload = try await client.call("sessions.interaction.inspect", params: params)
                // Core falls back to the oldest request when the chosen one is gone; follow it.
                if let chosen = selectedRequests[session.id], result.requestId != chosen { selectedRequests.removeValue(forKey: session.id) }
                payloads[session.id] = result
                // The state of the latest caller, not of the one that started this run: a card
                // recorded under the old state would be pruned by the next session update.
                payloadWaiting[session.id] = inflightWaiting[session.id] ?? waiting
                if result.kind != "none" && !preservedFeedback.contains(session.id) {
                    feedback.removeValue(forKey: session.id)
                }
            } catch {
                feedback[session.id] = ("error", error.localizedDescription)
            }
        } while queuedInspects.contains(session.id) && rounds < 3
    }

    /// Sends one reply for the shown card. `action` is a verb (`answer`, `followup`, `cancel`, `skip`,
    /// `interrupt`) or `decide` with the `decisionId` of one of the card's decisions.
    /// Returns true only when the reply was delivered (`confirmed` or `submitted`).
    func respond(
        session: LiveSession,
        payload: InteractionPayload,
        action: String,
        decisionId: String? = nil,
        answers: [String: Any]? = nil,
        text: String? = nil,
        reason: String? = nil
    ) async -> Bool {
        guard !submittingSessionIDs.contains(session.id), !isCompleted(payload) else { return false }
        // A card is only ever answered for the session it was read from.
        guard payload.sessionId == session.id, !payload.stateKey.isEmpty else {
            feedback[session.id] = ("blocked", L("会话状态或问题已变化，请刷新后再操作"))
            return false
        }
        submittingSessionIDs.insert(session.id)
        defer { submittingSessionIDs.remove(session.id) }
        var params: [String: Any] = [
            "sessionId": payload.sessionId,
            "stateKey": payload.stateKey,
            "promptHash": payload.promptHash,
            "action": action,
            "capabilities": Self.capabilities,
        ]
        if let decisionId { params["decisionId"] = decisionId }
        // Names the request the card showed, so the reply cannot land on another queued one.
        if !payload.requestId.isEmpty, action != "followup", action != "interrupt" { params["requestId"] = payload.requestId }
        if let answers { params["answers"] = answers }
        if let text { params["text"] = text }
        if let reason, !reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            params["reason"] = reason
        }
        do {
            let result: InteractionRespondResult = try await client.call("sessions.interaction.respond", params: params)
            var status = result.status, message = result.message
            if status == "needsLocalTerminal" {
                // Core hands the text back for this client to type; the job must be for this very session.
                guard let job = result.terminalJob, job.session.id == payload.sessionId else {
                    feedback[session.id] = ("blocked", L("无法准确定位原终端，指令已保留"))
                    return false
                }
                let delivered = await deliverToTerminal(job.session, job.text, client)
                status = delivered.status
                message = L(delivered.reason)
            }
            return await settle(status: status, message: message, session: session, payload: payload, action: action)
        } catch {
            feedback[session.id] = ("error", error.localizedDescription)
            return false
        }
    }

    /// Applies a respond status to the card. Every status has one outcome:
    /// `confirmed` / `submitted` lock the answered card, `uncertain` locks it without claiming success
    /// (it must not be sent twice), `blocked` reloads it, anything else is shown as an error.
    private func settle(status: String, message: String, session: LiveSession, payload: InteractionPayload, action: String) async -> Bool {
        let free = payload.kind == "followup" || action == "interrupt"
        switch status {
        case "confirmed", "submitted":
            feedback[session.id] = (status, message)
            if free {
                // Not an answer to a request: the card stays usable and shows the session's new state.
                if action != "interrupt" { drafts.removeValue(forKey: draftKey(for: payload)) }
                await inspect(session: session, force: true, preserveFeedback: true)
            } else {
                markCompleted(payload, session: session)
                // More requests are queued behind this one; show the next instead of a locked card.
                if payload.pendingCount > 1 { await inspect(session: session, force: true, preserveFeedback: true) }
            }
            return true
        case "uncertain":
            feedback[session.id] = (status, message)
            if action != "interrupt" { markCompleted(payload, session: session, keepDraft: true) }
            return false
        case "blocked":
            feedback[session.id] = (status, message)
            // Core refuses when the prompt moved on; show the current one under the refusal.
            await inspect(session: session, force: true, preserveFeedback: true)
            return false
        default:
            feedback[session.id] = ("error", message.isEmpty ? status : message)
            return false
        }
    }

    /// Stops the session's running turn where `followUp.canInterrupt` says the adapter can.
    func interrupt(session: LiveSession, payload: InteractionPayload) async -> Bool {
        guard payload.followUp.canInterrupt else { return false }
        return await respond(session: session, payload: payload, action: "interrupt")
    }

    func handoff(session: LiveSession, payload: InteractionPayload) async -> Bool {
        guard !submittingSessionIDs.contains(session.id), !isCompleted(payload), payload.sessionId == session.id else { return false }
        submittingSessionIDs.insert(session.id)
        defer { submittingSessionIDs.remove(session.id) }
        do {
            var params: [String: Any] = ["sessionId": payload.sessionId]
            if !payload.requestId.isEmpty { params["requestId"] = payload.requestId }
            let result: InteractionHandoffResult = try await client.call("sessions.interaction.handoff", params: params)
            // `confirmed: false` means the hook has not picked the hand-back up yet.
            feedback[session.id] = (result.handedOff ? (result.confirmed == false ? "submitted" : "confirmed") : "blocked", result.message)
            if result.handedOff {
                markCompleted(payload, session: session)
                await focusSession(session)
            } else {
                await inspect(session: session, force: true, preserveFeedback: true)
            }
            return result.handedOff
        } catch {
            feedback[session.id] = ("error", error.localizedDescription)
            return false
        }
    }
}
