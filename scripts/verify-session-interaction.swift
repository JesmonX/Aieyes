import AppKit
import Foundation
import SwiftUI
@testable import Aieyes

/// Native checks for the pet interaction card: card decoding, answer and keyboard rules, diff and plan
/// rendering, popover close policy, and SessionInteractionStore against a stub core (auto-popup queue,
/// inspect, every respond status, local terminal delivery, follow-up, interrupt, request queue, handoff).
@main struct VerifySessionInteraction {
    static func option(_ label: String, value: String? = nil, hasValue: Bool? = nil, preview: String? = nil) -> InteractionOption {
        InteractionOption(label: label, preview: preview, value: value, hasValue: hasValue)
    }
    static func question(_ id: String, _ options: [InteractionOption], multi: Bool = false, custom: Bool = false) -> InteractionQuestion {
        InteractionQuestion(id: id, key: id, question: "Question " + id, kind: "string", multiSelect: multi, allowCustom: custom, options: options)
    }
    static func payload(_ questions: [InteractionQuestion], kind: String = "questions", state: String = "s1") -> InteractionPayload {
        InteractionPayload(sessionId: "a", stateKey: state, promptHash: "h", kind: kind, adapter: "claudeHook", canRespond: true, canHandoff: false,
                           needsClaudeHook: false, requestId: "r", title: "", approval: nil, questions: questions, textPrompt: "", readonlyReason: "")
    }
    static func waiting(_ id: String, state: String? = "s1", provider: String = "claude") -> LiveSession {
        var session = LiveSession(id: id, source: "fixture", phase: .needsInput, updatedAt: Date(timeIntervalSince1970: 100))
        session.provider = provider; session.machineId = "remote"; session.machine = "build-host"; session.inputStateKey = state
        return session
    }

    static func verifyAnswers() throws {
        // Options decode with or without core values; a non-string value is noted but never sent.
        let decoded = try JSONDecoder().decode([InteractionOption].self, from: Data(#"[{"label":"Yes","value":true},{"label":"Fast","value":"fast","description":"d"},{"label":"Plain"},{"label":"Null","value":null}]"#.utf8))
        precondition(decoded[0].hasValue && decoded[0].value == nil)
        precondition(decoded[1].hasValue && decoded[1].value == "fast" && decoded[1].description == "d")
        precondition(!decoded[2].hasValue && !decoded[3].hasValue)

        // Without core values the label is the answer, including for repeated labels.
        let plain = question("q", [option("A"), option("B"), option("A")])
        precondition(InteractionAnswers.wireAnswer(for: 0, in: plain) == "A" && InteractionAnswers.wireAnswer(for: 2, in: plain) == "A")
        precondition(InteractionAnswers.wireAnswer(for: 3, in: plain) == nil && InteractionAnswers.wireAnswer(for: -1, in: plain) == nil)

        // With core values, a repeated label is sent as the value that selects exactly that option.
        let valued = question("q", [option("Same", value: "one"), option("Same", value: "two"), option("Other", value: "three")])
        precondition(InteractionAnswers.wireAnswer(for: 0, in: valued) == "Same")
        precondition(InteractionAnswers.wireAnswer(for: 1, in: valued) == "two", "The second option must not be sent as the first one's label")
        precondition(InteractionAnswers.wireAnswer(for: 2, in: valued) == "Other")
        // Core matches labels ignoring ASCII case, and an earlier option's value can shadow a later label.
        let shadowed = question("q", [option("yes", value: "no"), option("YES", hasValue: true), option("no", hasValue: true)])
        precondition(InteractionAnswers.wireAnswer(for: 0, in: shadowed) == "yes")
        precondition(InteractionAnswers.wireAnswer(for: 1, in: shadowed) == nil, "An option no string selects must be refused, not guessed")
        precondition(InteractionAnswers.wireAnswer(for: 2, in: shadowed) == nil)
        precondition(InteractionAnswers.resolvedIndex(of: "nope", in: shadowed.options) == nil)

        // Selection is by index: equal labels are not selected together.
        var draft = InteractionDraft()
        draft.multiChoices["m"] = [1]
        let multi = question("m", [option("Same", value: "one"), option("Same", value: "two")], multi: true, custom: true)
        let single = question("s", [option("A"), option("B")], custom: true)
        let free = question("f", [])
        let form = payload([multi, single, free])
        precondition(InteractionAnswers.unansweredIndices(in: form, draft: draft) == [1, 2] && !InteractionAnswers.allAnswered(in: form, draft: draft))
        draft.singleChoices["s"] = 1
        draft.customAnswers["f"] = "  typed  "
        draft.customAnswers["m"] = " extra "
        precondition(InteractionAnswers.allAnswered(in: form, draft: draft))
        var answers = InteractionAnswers.answers(for: form, draft: draft)
        precondition(answers["m"] as? [String] == ["two", "extra"], "Multi-select sends option order, then the custom answer")
        precondition(answers["s"] as? String == "B" && answers["f"] as? String == "typed")
        // A typed custom answer replaces the single choice.
        draft.customAnswers["s"] = "mine"
        answers = InteractionAnswers.answers(for: form, draft: draft)
        precondition(answers["s"] as? String == "mine")
        // A selection that cannot be sent does not count as an answer.
        var refused = InteractionDraft()
        refused.singleChoices["q"] = 1
        precondition(!InteractionAnswers.isAnswered(shadowed, in: refused) && InteractionAnswers.answers(for: payload([shadowed]), draft: refused).isEmpty)
        precondition(!InteractionAnswers.allAnswered(in: payload([]), draft: draft))
        precondition(draft.hasUserInput && !InteractionDraft().hasUserInput)

        precondition(InteractionAnswers.isSingleDirect(payload([plain])))
        precondition(!InteractionAnswers.isSingleDirect(payload([multi])) && !InteractionAnswers.isSingleDirect(payload([free])) && !InteractionAnswers.isSingleDirect(payload([plain, single])))
        precondition(InteractionAnswers.optionIndex(forKey: "1", optionCount: 3) == 0 && InteractionAnswers.optionIndex(forKey: "3", optionCount: 3) == 2)
        for key in ["0", "4", "a", "12", "", "１"] { precondition(InteractionAnswers.optionIndex(forKey: key, optionCount: 3) == nil, key) }
        precondition(InteractionAnswers.optionIndex(forKey: nil, optionCount: 3) == nil)
    }

    /// Cards decode in full, and a card from a core that predates a field still decodes.
    static func verifyContract() throws {
        let minimal = try JSONDecoder().decode(InteractionPayload.self, from: Data(#"{"sessionId":"a","stateKey":"s","promptHash":"h","kind":"text","adapter":"tmux","canRespond":true}"#.utf8))
        precondition(minimal.canRespond && !minimal.canHandoff && minimal.pendingCount == 0 && minimal.pendingRequests.isEmpty && minimal.requestId.isEmpty)
        // What identifies the request or allows an answer is never defaulted.
        for broken in [#"{"sessionId":"a","promptHash":"h","kind":"text","adapter":"tmux","canRespond":true}"#, #"{"sessionId":"a","stateKey":"s","promptHash":"h","kind":"text","adapter":"tmux"}"#] {
            precondition((try? JSONDecoder().decode(InteractionPayload.self, from: Data(broken.utf8))) == nil, broken)
        }
        precondition(minimal.form == nil && !minimal.followUp.available && !minimal.followUp.canInterrupt && minimal.quickEnable.isEmpty)
        let full = try JSONDecoder().decode(InteractionPayload.self, from: Data(#"""
        {"sessionId":"a","stateKey":"s","promptHash":"h","kind":"approval","adapter":"codexControl","transport":"appServer","canRespond":true,"canHandoff":false,
         "requestId":"r2","title":"Edit","pendingCount":3,"pendingRequests":[{"requestId":"r1","kind":"approval","title":"Run"},{"requestId":"r2","kind":"approval","title":"Edit"}],
         "requestedAt":1700000000.5,"inputKind":"approval","promptSource":"lastMessage","needsClaudeApprovals":true,"quickEnable":"approvals",
         "followUp":{"available":true,"adapter":"opencode","mode":"queue","reason":"","canInterrupt":true},
         "approval":{"toolName":"apply_patch","kind":"fileChange","title":"Edit files","cwd":"/work","reason":"why","command":"","details":"",
           "plan":{"markdown":"# Plan","truncated":true},"paths":["/work/a.txt"],
           "changes":[{"path":"/work/a.txt","kind":"update","diff":"@@ -1 +1 @@\n-old\n+new","truncated":false},{"path":"/work/b.txt","kind":"move","movePath":"/work/c.txt","oldText":"x","newText":"y"}],
           "decisions":[{"id":"accept","label":"允许本次","style":"primary","behavior":"allow","scope":"once"},
                        {"id":"acceptForSession","label":"本会话内允许","style":"secondary","behavior":"allow","scope":"session","description":"d"},
                        {"id":"abort","label":"拒绝并中断","style":"danger","behavior":"deny","acceptsReason":true,"interrupts":true}]},
         "form":{"title":"Login","description":"desc","source":"mcp","serverName":"github","url":"https://example.com/x","canCancel":true,"canSkip":true},
         "questions":[{"id":"token","question":"Token?","header":"Token","kind":"string","secret":true,"required":false,"description":"d","placeholder":"p",
                       "default":5,"constraints":{"minimum":1,"maximum":9,"minLength":2,"maxLength":4,"minItems":1,"maxItems":2,"pattern":"x","format":"email"}}]}
        """#.utf8))
        precondition(full.transport == "appServer" && full.pendingCount == 3 && full.pendingRequests.map(\.requestId) == ["r1", "r2"] && full.pendingRequests[1].title == "Edit")
        precondition(full.requestedAt == 1700000000.5 && full.inputKind == "approval" && full.promptSource == "lastMessage" && full.needsClaudeApprovals && full.quickEnable == "approvals")
        precondition(full.followUp.available && full.followUp.mode == "queue" && full.followUp.canInterrupt)
        let approval = full.approval!
        precondition(approval.kind == "fileChange" && approval.title == "Edit files" && approval.cwd == "/work" && approval.plan?.truncated == true && approval.paths == ["/work/a.txt"])
        precondition(approval.changes.count == 2 && approval.changes[1].movePath == "/work/c.txt" && approval.changes[1].newText == "y")
        precondition(approval.decisions.map(\.id) == ["accept", "acceptForSession", "abort"] && approval.decisions[1].scope == "session" && approval.decisions[1].description == "d")
        precondition(approval.decisions[2].acceptsReason && approval.decisions[2].interrupts && approval.decisions[2].behavior == "deny" && !approval.decisions[0].acceptsReason)
        precondition(full.form?.serverName == "github" && full.form?.canCancel == true && full.form?.canSkip == true && full.form?.url == "https://example.com/x")
        let field = full.questions[0]
        precondition(field.secret && !field.required && field.header == "Token" && field.placeholder == "p" && field.defaultValue == "5")
        precondition(field.constraints?.maximum == 9 && field.constraints?.maxLength == 4 && field.constraints?.maxItems == 2)
        // A decision without a style is still listed; an approval without decisions has none to press.
        let bare = try JSONDecoder().decode(InteractionApproval.self, from: Data(#"{"toolName":"Bash","decisions":[{"id":"x","label":"X"}]}"#.utf8))
        precondition(bare.decisions.count == 1 && bare.changes.isEmpty && bare.plan == nil)
        let handoff = try JSONDecoder().decode(InteractionHandoffResult.self, from: Data(#"{"handedOff":true,"message":"m","confirmed":false,"requestId":"r"}"#.utf8))
        precondition(handoff.confirmed == false && handoff.requestId == "r")
        let status = try JSONDecoder().decode(SessionInteractionStatus.self, from: Data(#"{"enabled":true,"autoPopup":false,"claudeQuestions":true,"claudeApprovals":false,"allowTerminal":false}"#.utf8))
        precondition(status.enabled && status.claudeQuestions && status.hooks == nil && status.codexControl == nil)
    }

    /// Form fields: typed kinds, optional and secret fields, defaults and constraint problems.
    static func verifyForms() throws {
        func field(_ json: String) throws -> InteractionQuestion { try JSONDecoder().decode(InteractionQuestion.self, from: Data(json.utf8)) }
        let flag = try field(#"{"id":"ok","question":"OK?","kind":"boolean"}"#)
        let count = try field(#"{"id":"n","question":"N?","kind":"integer","constraints":{"minimum":1,"maximum":5}}"#)
        let ratio = try field(#"{"id":"r","question":"R?","kind":"number","required":false}"#)
        let secret = try field(#"{"id":"s","question":"S?","kind":"string","secret":true,"default":"hunter2","constraints":{"minLength":3,"maxLength":6}}"#)
        let picks = try field(#"{"id":"m","question":"M?","kind":"multiselect","options":[{"label":"a"},{"label":"b"},{"label":"c"}],"constraints":{"minItems":1,"maxItems":2}}"#)
        let preset = try field(#"{"id":"p","question":"P?","kind":"string","allowCustom":false,"options":[{"label":"One","value":"1"},{"label":"Two","value":"2"}],"default":"2"}"#)
        precondition(InteractionAnswers.isBareBoolean(flag) && InteractionAnswers.options(for: flag).map(\.value) == ["true", "false"])
        precondition(!InteractionAnswers.acceptsTyped(flag) && InteractionAnswers.acceptsTyped(count) && InteractionAnswers.acceptsTyped(secret) && !InteractionAnswers.acceptsTyped(preset))
        precondition(InteractionAnswers.isMulti(picks) && !InteractionAnswers.isMulti(preset))
        var form = payload([flag, count, ratio, secret, picks, preset])
        form.form = InteractionForm(title: "t", description: "", source: "mcp", serverName: "srv", url: "", canCancel: true, canSkip: false)
        // Defaults are prefilled, except a secret, which is never put on screen.
        var draft = InteractionAnswers.initialDraft(for: form)
        precondition(draft.customAnswers["s"] == nil && draft.singleChoices["p"] == 1, "\(draft)")
        precondition(InteractionAnswers.unansweredIndices(in: form, draft: draft) == [0, 1, 3, 4], "The optional number may stay empty")
        draft.singleChoices["ok"] = 1
        draft.customAnswers["n"] = "7"
        precondition(InteractionAnswers.problem(with: count, in: draft) == "数值超出允许范围")
        draft.customAnswers["n"] = "2.5"
        precondition(InteractionAnswers.problem(with: count, in: draft) == "请输入有效的整数")
        draft.customAnswers["n"] = "3"
        precondition(InteractionAnswers.problem(with: count, in: draft) == nil)
        draft.customAnswers["r"] = "abc"
        precondition(InteractionAnswers.problem(with: ratio, in: draft) == "请输入有效的数字" && InteractionAnswers.unansweredIndices(in: form, draft: draft).contains(2), "A wrong optional value still blocks")
        draft.customAnswers["r"] = ""
        draft.customAnswers["s"] = "ab"
        precondition(InteractionAnswers.problem(with: secret, in: draft) == "文本长度不符合要求")
        draft.customAnswers["s"] = "abcd"
        draft.multiChoices["m"] = [0, 1, 2]
        precondition(InteractionAnswers.problem(with: picks, in: draft) == "选择的数量不符合要求")
        draft.multiChoices["m"] = [2, 0]
        precondition(InteractionAnswers.unansweredIndices(in: form, draft: draft).isEmpty && InteractionAnswers.allAnswered(in: form, draft: draft))
        precondition(InteractionAnswers.answeredCount(in: form, draft: draft) == 5)
        let answers = InteractionAnswers.answers(for: form, draft: draft)
        precondition(answers["ok"] as? Bool == false, "A bare boolean is sent typed: \(answers)")
        precondition(answers["n"] as? String == "3" && answers["s"] as? String == "abcd" && answers["m"] as? [String] == ["a", "c"] && answers["p"] as? String == "Two")
        precondition(answers["r"] == nil && answers.count == 5, "An unanswered optional field is left out, as core rejects unknown or empty answers")
        // A form, or a secret, is never answered by a single click.
        precondition(!InteractionAnswers.isSingleDirect(form))
        var lone = payload([preset]); lone.form = form.form
        precondition(!InteractionAnswers.isSingleDirect(lone) && InteractionAnswers.isSingleDirect(payload([preset])))
        // A form of optional fields only still needs one answer before it can be submitted.
        let optional = payload([ratio])
        precondition(InteractionAnswers.unansweredIndices(in: optional, draft: InteractionDraft()).isEmpty && !InteractionAnswers.allAnswered(in: optional, draft: InteractionDraft()))

        // A plain Return pages through a form and only sends it from the last page; pages the user
        // has not reached may hold optional or pre-filled fields.
        typealias Step = InteractionAnswers.ReturnStep
        func step(_ payload: InteractionPayload, _ draft: InteractionDraft, _ page: Int, explicit: Bool = false) -> Step {
            InteractionAnswers.returnStep(in: payload, draft: draft, page: page, explicit: explicit)
        }
        var paged = payload([count, ratio, preset]); paged.form = form.form
        var filling = InteractionAnswers.initialDraft(for: paged)
        precondition(step(paged, filling, 0) == .page(0), "A required field without an answer keeps its page")
        filling.customAnswers["n"] = "3"
        precondition(InteractionAnswers.allAnswered(in: paged, draft: filling))
        precondition(step(paged, filling, 0) == .page(1), "Return on the first page must not send the unseen pages")
        precondition(step(paged, filling, 1) == .page(2) && step(paged, filling, 2) == .submit)
        precondition(step(paged, filling, 0, explicit: true) == .submit, "Command Return sends a complete form from any page")
        filling.customAnswers["r"] = "abc"
        precondition(step(paged, filling, 1) == .page(1) && step(paged, filling, 2) == .page(1) && step(paged, filling, 0, explicit: true) == .page(1), "A refused value is shown, not sent")
        precondition(step(payload([preset]), InteractionAnswers.initialDraft(for: payload([preset])), 0) == .submit, "A single question is answered by Return")
    }

    /// File changes and plans render from the agent's own text, with nothing that differs left out.
    static func verifyContent() {
        typealias Line = InteractionDiff.Line
        let unified = InteractionDiff.unified("--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n keep\n-old\r\n+new\n")
        precondition(unified.map(\.kind) == [.meta, .meta, .hunk, .context, .removed, .added], "\(unified)")
        precondition(unified[4].text == "-old" && InteractionDiff.summary(unified) == "+1 −1")
        let replaced = InteractionDiff.replaced(old: "a\nb\nc\nOLD\nx\ny\nz", new: "a\nb\nc\nNEW\nMORE\nx\ny\nz")
        precondition(replaced == [Line(kind: .context, text: "  b"), Line(kind: .context, text: "  c"), Line(kind: .removed, text: "- OLD"),
                                  Line(kind: .added, text: "+ NEW"), Line(kind: .added, text: "+ MORE"), Line(kind: .context, text: "  x"), Line(kind: .context, text: "  y")], "\(replaced)")
        precondition(InteractionDiff.replaced(old: "", new: "one\ntwo").map(\.kind) == [.added, .added])
        precondition(InteractionDiff.replaced(old: "gone", new: "").map(\.text) == ["- gone"])
        precondition(InteractionDiff.replaced(old: "same", new: "same").allSatisfy { $0.kind == .context } && InteractionDiff.summary([]) == "")
        let change = InteractionFileChange(path: "/p", kind: "update", movePath: "", diff: "+x", oldText: "ignored", newText: "ignored", truncated: false)
        precondition(InteractionDiff.lines(for: change) == [Line(kind: .added, text: "+x")], "The agent's diff wins over the raw texts")
        precondition(String(InteractionDiff.attributed(unified).characters) == unified.map(\.text).joined(separator: "\n"))

        let blocks = InteractionMarkdown.blocks("# Title\n\n\nStep **one**\n```sh\n# not a heading\n```\n#hash\n")
        precondition(blocks == [.heading(level: 1, text: "Title"), .blank, .text("Step **one**"), .code("# not a heading"), .text("#hash")], "\(blocks)")
        let plan = InteractionMarkdown.attributed("## Plan\n- see [docs](https://example.com) and **bold**\n```\nrm *x*\n```")
        let text = String(plan.characters)
        precondition(text.contains("Plan") && text.contains("docs") && text.contains("bold") && !text.contains("**") && text.contains("rm *x*"), text)
        precondition(plan.runs.allSatisfy { $0.link == nil }, "A plan never carries a clickable link")
        precondition(PetInteractionCardView.changeKindLabel("add") == "新增" && PetInteractionCardView.changeKindLabel("unknown") == "文件")
    }

    static func verifyKeyboard() {
        typealias Context = InteractionKeyboard.Context
        func action(_ keyCode: UInt16, _ characters: String? = nil, _ modifiers: NSEvent.ModifierFlags = [], _ context: Context) -> InteractionKeyboard.Action {
            InteractionKeyboard.action(keyCode: keyCode, characters: characters, modifiers: modifiers, context: context)
        }
        let esc: UInt16 = 53, ret: UInt16 = 36, enter: UInt16 = 76
        var approval = Context(kind: "approval", interactive: true)
        approval.hasPrimaryDecision = true
        // Esc: the decline reason collapses first, a text field is left second, then the popover closes.
        precondition(action(esc, nil, [], approval) == .close)
        var declining = approval
        declining.showDeclineReason = true; declining.focus = .declineReason; declining.editingText = true
        precondition(action(esc, nil, [], declining) == .collapseDecline)
        precondition(action(ret, "\r", [], declining) == .confirmDecline)
        // Command Return in the note field sends the noted decision; it must never approve.
        precondition(action(ret, "\r", [.command], declining) == .confirmDecline && action(enter, "\r", [.command], declining) == .confirmDecline)
        precondition(action(ret, "\r", [.option], declining) == .passThrough && action(ret, "\r", [.command, .shift], declining) == .passThrough)
        var noteOpen = declining
        noteOpen.focus = nil; noteOpen.editingText = false
        precondition(action(ret, "\r", [.command], noteOpen) == .passThrough, "An open note blocks the approve shortcut wherever the focus is")
        precondition(action(ret, "\r", [], noteOpen) == .passThrough, "Return outside the note field does not confirm the decline")
        noteOpen.editingText = true
        precondition(action(ret, "\r", [.command], noteOpen) == .passThrough)
        var typing = Context(kind: "text", interactive: true)
        typing.focus = .reply; typing.editingText = true
        precondition(action(esc, nil, [], typing) == .defocus, "Esc while typing keeps the draft and the popover")
        precondition(action(ret, "\r", [], typing) == .submit && action(enter, "\r", [], typing) == .submit)
        precondition(action(ret, "\r", [.option], typing) == .passThrough, "Option Return inserts a line break")
        precondition(action(ret, "\r", [.shift], typing) == .passThrough)
        precondition(action(ret, "\r", [.command], typing) == .submit)
        // An input method owns Return and Esc while it composes.
        typing.composing = true
        precondition(action(ret, "\r", [], typing) == .passThrough && action(esc, nil, [], typing) == .passThrough)
        precondition(action(ret, "\r", [.command], typing) == .passThrough)
        declining.composing = true
        precondition(action(ret, "\r", [], declining) == .passThrough && action(esc, nil, [], declining) == .passThrough)

        // A bare Return never approves; the primary decision needs the modifier.
        precondition(action(ret, "\r", [], approval) == .passThrough)
        precondition(action(ret, "\r", [.command], approval) == .approve)
        approval.hasPrimaryDecision = false
        precondition(action(ret, "\r", [.command], approval) == .passThrough)
        precondition(action(18, "1", [], approval) == .passThrough, "Digits never pick an approval decision")

        var questions = Context(kind: "questions", interactive: true)
        questions.optionCount = 3
        precondition(action(18, "1", [], questions) == .selectOption(0) && action(20, "3", [], questions) == .selectOption(2))
        precondition(action(21, "4", [], questions) == .passThrough && action(18, "1", [.command], questions) == .passThrough)
        precondition(action(ret, "\r", [], questions) == .submit)
        questions.editingText = true; questions.focus = .custom("q")
        precondition(action(18, "1", [], questions) == .passThrough, "Digits typed in a field are text")
        precondition(action(ret, "\r", [], questions) == .submit)
        questions.focus = nil
        precondition(action(ret, "\r", [], questions) == .passThrough, "An unidentified text view keeps its own Return")

        // A locked or read-only card only reacts to Esc.
        var locked = Context(kind: "questions", interactive: false)
        locked.optionCount = 3
        precondition(action(18, "1", [], locked) == .passThrough && action(ret, "\r", [], locked) == .passThrough && action(ret, "\r", [.command], locked) == .passThrough)
        precondition(action(esc, nil, [], locked) == .close)
        precondition(action(ret, "\r", [.command], Context(kind: "readonly", interactive: true)) == .passThrough)

        // An instruction to a session that is not asking is only sent from its own field.
        var followUp = Context(kind: "followup", interactive: true)
        precondition(action(ret, "\r", [], followUp) == .passThrough && action(18, "1", [], followUp) == .passThrough)
        precondition(action(ret, "\r", [.command], followUp) == .submit)
        followUp.focus = .reply; followUp.editingText = true
        precondition(action(ret, "\r", [], followUp) == .submit && action(ret, "\r", [.option], followUp) == .passThrough)
        followUp.composing = true
        precondition(action(ret, "\r", [], followUp) == .passThrough)
        // While another card's field owns the keyboard, this card neither approves nor submits.
        var foreign = Context(kind: "approval", interactive: true)
        foreign.hasPrimaryDecision = true; foreign.editingText = true
        precondition(action(ret, "\r", [.command], foreign) == .passThrough)
        foreign.kind = "text"
        precondition(action(ret, "\r", [], foreign) == .passThrough && action(ret, "\r", [.command], foreign) == .passThrough)
    }

    static func verifyPolicy() {
        typealias Policy = InteractionPopoverPolicy
        // An auto-opened card survives clicks in the app the user is working in.
        precondition(!Policy.shouldClose(.otherAppClick, autoOpened: true, hasDraft: false))
        precondition(!Policy.shouldClose(.appDeactivated, autoOpened: true, hasDraft: false))
        precondition(Policy.shouldClose(.otherAppClick, autoOpened: false, hasDraft: false))
        precondition(Policy.shouldClose(.appDeactivated, autoOpened: false, hasDraft: false))
        precondition(Policy.shouldClose(.ownWindowClick, autoOpened: true, hasDraft: false))
        // Unsent input survives every outside click.
        for cause in [Policy.Cause.otherAppClick, .ownWindowClick, .appDeactivated] {
            precondition(!Policy.shouldClose(cause, autoOpened: false, hasDraft: true))
        }
        for cause in [Policy.Cause.explicit, .petDrag, .panelOpened] {
            precondition(Policy.shouldClose(cause, autoOpened: true, hasDraft: true))
        }
        // An auto-opened popover whose request was resolved elsewhere closes on the session update.
        // One that still has a request waiting, holds unsent input, or was opened by hand stays.
        precondition(Policy.shouldClose(.resolved, autoOpened: true, hasDraft: false, hasWaiting: false))
        precondition(!Policy.shouldClose(.resolved, autoOpened: true, hasDraft: false, hasWaiting: true))
        precondition(!Policy.shouldClose(.resolved, autoOpened: true, hasDraft: true, hasWaiting: false))
        precondition(!Policy.shouldClose(.resolved, autoOpened: false, hasDraft: false, hasWaiting: false))
        precondition(!Policy.shouldClose(.resolved, autoOpened: true, hasDraft: false), "A session update closes nothing unless told the agent stopped waiting")
        // The waiting state changes no other cause.
        for asking in [true, false] {
            precondition(!Policy.shouldClose(.otherAppClick, autoOpened: true, hasDraft: false, hasWaiting: asking))
            precondition(!Policy.shouldClose(.appDeactivated, autoOpened: true, hasDraft: false, hasWaiting: asking))
            precondition(!Policy.shouldClose(.ownWindowClick, autoOpened: true, hasDraft: true, hasWaiting: asking))
            precondition(Policy.shouldClose(.explicit, autoOpened: false, hasDraft: true, hasWaiting: asking))
        }
        // A follow-up card is dropped when another agent's popover is shown, so its draft cannot hold that one open.
        precondition(Policy.keepsFollowUp(shownProvider: "claude", nextProvider: "claude"))
        precondition(!Policy.keepsFollowUp(shownProvider: "claude", nextProvider: "codex") && !Policy.keepsFollowUp(shownProvider: nil, nextProvider: "codex"))
        // The follow-up button is not offered while the agent's waiting card owns the popover.
        var busy = waiting("w"); busy.phase = .working
        precondition(PetAgentSessionPopoverView.offersFollowUp(enabled: true, session: busy, agentWaiting: false))
        precondition(!PetAgentSessionPopoverView.offersFollowUp(enabled: true, session: busy, agentWaiting: true))
        precondition(!PetAgentSessionPopoverView.offersFollowUp(enabled: false, session: busy, agentWaiting: false))
        precondition(!PetAgentSessionPopoverView.offersFollowUp(enabled: true, session: waiting("w"), agentWaiting: false))
        precondition(Policy.marksDismissed(.explicit))
        for cause in [Policy.Cause.otherAppClick, .ownWindowClick, .appDeactivated, .petDrag, .panelOpened, .resolved] {
            precondition(!Policy.marksDismissed(cause), "Only an explicit close dismisses queued requests")
        }

        // A long command scrolls inside its cap, expands to a larger cap, and short text is never padded.
        precondition(!PetInteractionCodeBlock.overflows(contentHeight: 110, maxHeight: 110) && PetInteractionCodeBlock.overflows(contentHeight: 400, maxHeight: 110))
        precondition(PetInteractionCodeBlock.height(contentHeight: 40, maxHeight: 110, expanded: false) == 40)
        precondition(PetInteractionCodeBlock.height(contentHeight: 400, maxHeight: 110, expanded: false) == 110)
        precondition(PetInteractionCodeBlock.height(contentHeight: 400, maxHeight: 110, expanded: true) == PetInteractionCodeBlock.expandedHeight)
        precondition(PetInteractionCodeBlock.height(contentHeight: 200, maxHeight: 110, expanded: true) == 200)
        precondition(PetInteractionCodeBlock.height(contentHeight: nil, maxHeight: 72, expanded: false) == 72)

        // Waiting sessions that share a project name are told apart by their short identity.
        var first = waiting("a"), second = waiting("b"), third = waiting("c")
        first.workspace = "/work/app"; first.nativeSession = "session-11111111"
        second.workspace = "/other/app"; second.nativeSession = "session-22222222"
        third.workspace = "/work/tool"; third.nativeSession = "session-33333333"
        let all = [first, second, third]
        precondition(PetAgentSessionPopoverView.chipTitle(for: first, among: all) == "app #11111111")
        precondition(PetAgentSessionPopoverView.chipTitle(for: second, among: all) == "app #22222222")
        precondition(PetAgentSessionPopoverView.chipTitle(for: third, among: all) == "tool")
        let agent = PetOrbitAgent(provider: "claude", displayProvider: "claude", phase: .needsInput, activeCount: 2)
        precondition(agent.accessibilitySummary == "Claude Code · 等待输入 · 2 个进行中", agent.accessibilitySummary)
        precondition(!PetOrbitAgent(provider: "codex", displayProvider: "codex", phase: .working, activeCount: 1).accessibilitySummary.contains("等待输入"))
    }

    static let stub = #"""
#!/usr/bin/env python3
import json,pathlib,sys,time
root=pathlib.Path(__file__).parent
def load(name,default):
    path=root/name
    return json.loads(path.read_text()) if path.exists() else default
for line in sys.stdin:
    request=json.loads(line);method=request['method']
    with (root/'calls').open('a') as log:log.write(method+'\t'+json.dumps(request.get('params') or {},sort_keys=True)+'\n')
    if (root/'blocked').exists() and (root/'blocked').read_text()==method:
        (root/'started').touch()
        deadline=time.monotonic()+30
        while not (root/'release').exists():
            if time.monotonic()>deadline:raise RuntimeError('test request was not released')
            time.sleep(.01)
        (root/'blocked').unlink()
    if method=='sessions.interaction.status':result=load('status.json',{})
    elif method=='sessions.interaction.inspect':
        chosen=(request.get('params') or {}).get('requestId')
        result=load('payload-'+chosen+'.json',None) if chosen else None
        if result is None:result=load('payload.json',{})
    elif method=='sessions.interaction.respond':result=load('respond.json',{})
    elif method=='sessions.interaction.handoff':result=load('handoff.json',{})
    else:result={}
    if isinstance(result,dict) and 'rpcError' in result:
        print(json.dumps({'id':request['id'],'error':{'message':result['rpcError']}}),flush=True)
        continue
    print(json.dumps({'id':request['id'],'result':result}),flush=True)
"""#

    /// Every respond status has exactly one outcome, and text only ever goes to the session it was written for.
    @MainActor static func verifyStatuses(
        store: SessionInteractionStore,
        root: URL,
        calls: (String) -> [String],
        write: (String, Any) throws -> Void,
        typed: () -> [(session: String, text: String)],
        setOutcome: (SessionChannelAutomation.Outcome) -> Void
    ) async throws {
        func card(_ kind: String, _ state: String, _ extra: [String: Any] = [:]) -> [String: Any] {
            var value: [String: Any] = ["sessionId": "a", "stateKey": state, "promptHash": "h-" + state, "kind": kind, "adapter": "macosTerminal", "canRespond": true,
                                        "requestId": "", "title": "Fixture", "textPrompt": "Continue?"]
            for (key, item) in extra { value[key] = item }
            return value
        }
        func job(_ session: String, _ text: String) -> [String: Any] {
            ["status": "needsLocalTerminal", "message": "", "terminalJob": ["text": text, "session": ["id": session, "source": "fixture", "phase": "needsInput", "updatedAt": 100]]]
        }
        let other = waiting("b", provider: "codex")
        var session = waiting("a", state: "t1")
        store.observeSessions([other, session])

        // A text card answers with `answer`; `needsLocalTerminal` is typed here, into the job's own session.
        try write("payload.json", card("text", "t1"))
        await store.refresh(session: session)
        var current = store.payloads["a"]!
        store.updateDraft(for: current) { $0.textReply = "go on" }
        try write("respond.json", job("a", "go on"))
        var ok = await store.respond(session: session, payload: current, action: "answer", text: "go on")
        var last = calls("sessions.interaction.respond").last ?? ""
        precondition(ok && typed().count == 1 && typed()[0].session == "a" && typed()[0].text == "go on", "\(typed())")
        precondition(last.contains("\"action\": \"answer\"") && last.contains("\"text\": \"go on\"") && !last.contains("requestId") && last.contains("\"localTerminal\": true"), last)
        precondition(store.feedback["a"]?.status == "submitted" && store.isCompleted(current) && store.drafts.isEmpty)

        // A job for another session is refused: nothing is typed, the text stays, the card stays answerable.
        await store.refresh(session: session)
        store.updateDraft(for: current) { $0.textReply = "keep me" }
        try write("respond.json", job("b", "keep me"))
        ok = await store.respond(session: session, payload: current, action: "answer", text: "keep me")
        precondition(!ok && typed().count == 1 && store.feedback["a"]?.status == "blocked" && !store.isCompleted(current), "Text must never reach another session's terminal")
        precondition(store.draftBinding(for: current).textReply == "keep me")
        // The same for a job that is missing altogether.
        try write("respond.json", ["status": "needsLocalTerminal", "message": ""])
        ok = await store.respond(session: session, payload: current, action: "answer", text: "keep me")
        precondition(!ok && typed().count == 1 && store.feedback["a"]?.status == "blocked" && !store.isCompleted(current))
        // A card is never answered on behalf of another session, and never without a state key.
        let before = calls("sessions.interaction.respond").count
        ok = await store.respond(session: other, payload: current, action: "answer", text: "x")
        precondition(!ok && store.feedback["b"]?.status == "blocked" && calls("sessions.interaction.respond").count == before)
        var keyless = current; keyless.stateKey = ""
        ok = await store.respond(session: session, payload: keyless, action: "answer", text: "x")
        precondition(!ok && calls("sessions.interaction.respond").count == before, "A card without a state key is not sent")

        // Terminal delivery that could not be verified: locked so it is not typed twice, text kept, no success.
        setOutcome(.init(status: "uncertain", reason: "已输入但未能确认送达"))
        try write("respond.json", job("a", "keep me"))
        ok = await store.respond(session: session, payload: current, action: "answer", text: "keep me")
        precondition(!ok && typed().count == 2 && store.feedback["a"]?.status == "uncertain" && store.isCompleted(current))
        precondition(store.draftBinding(for: current).textReply == "keep me", "An unconfirmed text is kept for a retry")
        ok = await store.respond(session: session, payload: current, action: "answer", text: "keep me")
        precondition(!ok && typed().count == 2 && calls("sessions.interaction.respond").count == before + 1, "An unconfirmed answer is not sent again without a refresh")
        // Terminal delivery that was refused leaves the card answerable.
        await store.refresh(session: session)
        setOutcome(.init(status: "blocked", reason: "无法准确定位原终端，指令已保留"))
        ok = await store.respond(session: session, payload: current, action: "answer", text: "keep me")
        precondition(!ok && store.feedback["a"]?.status == "blocked" && !store.isCompleted(store.payloads["a"]!) && store.draftBinding(for: current).textReply == "keep me")
        setOutcome(.init(status: "submitted", reason: "已发送到原终端"))
        // `uncertain` straight from core behaves the same; an unknown status is an error and locks nothing.
        try write("respond.json", ["status": "uncertain", "message": "未能确认"])
        ok = await store.respond(session: session, payload: current, action: "answer", text: "keep me")
        precondition(!ok && store.feedback["a"]?.status == "uncertain" && store.isCompleted(current))
        await store.refresh(session: session)
        try write("respond.json", ["status": "surprise", "message": ""])
        ok = await store.respond(session: session, payload: current, action: "answer", text: "keep me")
        precondition(!ok && store.feedback["a"]?.status == "error" && store.feedback["a"]?.message == "surprise" && !store.isCompleted(current))

        // Queue: the chosen request is inspected and answered by id; afterwards the next one is shown.
        session = waiting("a", state: "p1")
        let queue: [[String: Any]] = [["requestId": "r1", "kind": "approval", "title": "Bash"], ["requestId": "r2", "kind": "questions", "title": "Pick"]]
        let decisions: [[String: Any]] = [["id": "once", "label": "允许本次", "style": "primary"], ["id": "always", "label": "始终允许", "style": "secondary"]]
        try write("payload.json", card("approval", "p1", ["requestId": "r1", "pendingCount": 2, "pendingRequests": queue, "approval": ["toolName": "Bash", "decisions": decisions]]))
        try write("payload-r2.json", card("questions", "p1", ["requestId": "r2", "pendingCount": 2, "pendingRequests": queue, "questions": [["id": "q", "question": "Pick", "options": [["label": "A"], ["label": "B"]]]]]))
        store.observeSessions([other, session])
        await store.refresh(session: session)
        precondition(store.payloads["a"]?.requestId == "r1" && store.payloads["a"]?.pendingRequests.count == 2)
        await store.selectRequest(session: session, requestID: "r2")
        precondition(store.payloads["a"]?.requestId == "r2" && store.payloads["a"]?.kind == "questions")
        precondition(calls("sessions.interaction.inspect").last?.contains("\"requestId\": \"r2\"") == true)
        current = store.payloads["a"]!
        // Once answered, core reports the remaining request; the card follows instead of staying locked.
        try FileManager.default.removeItem(at: root.appendingPathComponent("payload-r2.json"))
        try write("payload.json", card("approval", "p2", ["requestId": "r1", "pendingCount": 1, "approval": ["toolName": "Bash", "decisions": decisions]]))
        try write("respond.json", ["status": "confirmed", "message": "已提交"])
        ok = await store.respond(session: session, payload: current, action: "answer", answers: ["q": "B"])
        last = calls("sessions.interaction.respond").last ?? ""
        precondition(ok && last.contains("\"requestId\": \"r2\"") && last.contains("\"answers\": {\"q\": \"B\"}"), last)
        precondition(store.payloads["a"]?.requestId == "r1" && !store.isCompleted(store.payloads["a"]!) && store.feedback["a"]?.status == "confirmed", "The next queued request is shown, answerable")
        // The chosen request vanished: the card follows core's fallback and stops asking for it.
        await store.inspect(session: session, force: true)
        precondition(calls("sessions.interaction.inspect").last?.contains("requestId") == false)
        // An extended decision is sent as `decide` with its id, never as a verb of its own.
        current = store.payloads["a"]!
        ok = await store.respond(session: session, payload: current, action: "decide", decisionId: "always")
        last = calls("sessions.interaction.respond").last ?? ""
        precondition(ok && last.contains("\"action\": \"decide\"") && last.contains("\"decisionId\": \"always\"") && last.contains("\"requestId\": \"r1\"") && store.isCompleted(current), last)
        // Cancelling or skipping a form goes out as its own verb for the shown request.
        await store.refresh(session: session)
        ok = await store.respond(session: session, payload: current, action: "cancel")
        last = calls("sessions.interaction.respond").last ?? ""
        precondition(ok && last.contains("\"action\": \"cancel\"") && !last.contains("answers") && !last.contains("decisionId"), last)

        // Follow-up: a session that is not waiting takes an instruction; the card is not locked afterwards.
        var running = LiveSession(id: "a", source: "fixture", phase: .working, updatedAt: Date(timeIntervalSince1970: 100))
        running.provider = "opencode"; running.machineId = "local"; running.machine = "mac"
        try write("payload.json", card("followup", "run1", ["adapter": "opencode", "followUp": ["available": true, "adapter": "opencode", "mode": "queue", "canInterrupt": true]]))
        store.observeSessions([other, running])
        precondition(store.payloads["a"] == nil && store.completedKeys.isEmpty, "A waiting card does not outlive its wait")
        await store.inspect(session: running)
        current = store.payloads["a"]!
        precondition(current.kind == "followup" && current.followUp.canInterrupt && store.draftKey(for: current) == "a" + SessionInteractionStore.followUpSuffix)
        store.updateDraft(for: current) { $0.textReply = "also add tests" }
        precondition(store.hasActiveDraft(for: "a"))
        // The draft survives the turn moving on underneath it.
        try write("payload.json", card("followup", "run2", ["adapter": "opencode", "followUp": ["available": true, "adapter": "opencode", "mode": "newTurn", "canInterrupt": false]]))
        await store.inspect(session: running, force: true)
        current = store.payloads["a"]!
        precondition(current.stateKey == "run2" && store.draftBinding(for: current).textReply == "also add tests")
        try write("respond.json", ["status": "submitted", "message": "已发送后续指令"])
        ok = await store.respond(session: running, payload: current, action: "followup", text: "also add tests")
        last = calls("sessions.interaction.respond").last ?? ""
        precondition(ok && last.contains("\"action\": \"followup\"") && last.contains("\"stateKey\": \"run2\"") && last.contains("\"text\": \"also add tests\""), last)
        precondition(!store.isCompleted(store.payloads["a"]!) && store.drafts.isEmpty && store.feedback["a"]?.status == "submitted", "Another instruction can follow")
        // Interrupt is only sent where core says it is possible, and never locks the card or drops a draft.
        let noInterrupt = await store.interrupt(session: running, payload: store.payloads["a"]!)
        precondition(!noInterrupt && calls("sessions.interaction.respond").last == last, "No interrupt without canInterrupt")
        try write("payload.json", card("followup", "run3", ["adapter": "opencode", "followUp": ["available": true, "adapter": "opencode", "mode": "queue", "canInterrupt": true]]))
        await store.inspect(session: running, force: true)
        current = store.payloads["a"]!
        store.updateDraft(for: current) { $0.textReply = "after the stop" }
        try write("respond.json", ["status": "confirmed", "message": "已中断本轮"])
        ok = await store.interrupt(session: running, payload: current)
        last = calls("sessions.interaction.respond").last ?? ""
        precondition(ok && last.contains("\"action\": \"interrupt\"") && last.contains("\"stateKey\": \"run3\"") && !last.contains("requestId") && !last.contains("\"text\""), last)
        precondition(!store.isCompleted(store.payloads["a"]!) && store.draftBinding(for: current).textReply == "after the stop")
        // A follow-up refused because the session moved on reloads the card and keeps the text.
        try write("respond.json", ["status": "blocked", "message": "会话状态已变化"])
        ok = await store.respond(session: running, payload: current, action: "followup", text: "after the stop")
        precondition(!ok && store.feedback["a"]?.status == "blocked" && store.draftBinding(for: current).textReply == "after the stop")
        // When the session starts asking, the follow-up card is dropped; its unsent text stays with the session.
        store.observeSessions([other, waiting("a", state: "w9")])
        precondition(store.payloads["a"] == nil && store.drafts["a" + SessionInteractionStore.followUpSuffix]?.textReply == "after the stop")
        store.observeSessions([other])
        precondition(store.drafts.isEmpty, "Drafts of a session that left are dropped")

        // Two visible cards: the first one that takes the key wins, and a removed card no longer sees keys.
        let one = UUID(), two = UUID()
        var seen: [String] = []
        store.registerKeyInterceptor(owner: one) { _ in seen.append("one"); return .passThrough }
        store.registerKeyInterceptor(owner: two) { _ in seen.append("two"); return .handled }
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36)!
        precondition(store.handleKey(event) == .handled && seen == ["one", "two"])
        store.unregisterKeyInterceptor(owner: two)
        precondition(store.handleKey(event) == .passThrough && seen == ["one", "two", "one"])
        store.unregisterKeyInterceptor(owner: one)
        precondition(store.handleKey(event) == .passThrough && seen.count == 3)
        store.observeSessions([other, waiting("a", state: "s4")])
    }

    @MainActor static func main() async throws {
        try verifyAnswers()
        try verifyContract()
        try verifyForms()
        verifyContent()
        verifyKeyboard()
        verifyPolicy()

        let root = FileManager.default.temporaryDirectory.appendingPathComponent("aieyes-interaction-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let script = root.appendingPathComponent("core.py")
        try stub.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
        let previous = ProcessInfo.processInfo.environment["AIEYES_CORE_PATH"]
        setenv("AIEYES_CORE_PATH", script.path, 1)
        defer { if let previous { setenv("AIEYES_CORE_PATH", previous, 1) } else { unsetenv("AIEYES_CORE_PATH") } }

        func write(_ name: String, _ value: Any) throws {
            try JSONSerialization.data(withJSONObject: value).write(to: root.appendingPathComponent(name), options: .atomic)
        }
        func calls(_ method: String) -> [String] {
            ((try? String(contentsOf: root.appendingPathComponent("calls"), encoding: .utf8)) ?? "").split(separator: "\n")
                .filter { $0.hasPrefix(method + "\t") }.map { String($0.dropFirst(method.count + 1)) }
        }
        func exists(_ name: String) -> Bool { FileManager.default.fileExists(atPath: root.appendingPathComponent(name).path) }
        func waitFor(_ message: String, _ condition: () -> Bool) async throws {
            let deadline = ContinuousClock.now.advanced(by: .seconds(5))
            while !condition() {
                guard ContinuousClock.now < deadline else { throw ClientError.message(message) }
                try await Task.sleep(for: .milliseconds(20))
            }
        }
        func block(_ method: String) throws {
            for name in ["started", "release"] { try? FileManager.default.removeItem(at: root.appendingPathComponent(name)) }
            try method.write(to: root.appendingPathComponent("blocked"), atomically: true, encoding: .utf8)
        }
        func release() throws { try Data().write(to: root.appendingPathComponent("release")) }
        func card(kind: String, state: String = "s1", hash: String = "h1", extra: [String: Any] = [:]) -> [String: Any] {
            var value: [String: Any] = [
                "sessionId": "a", "stateKey": state, "promptHash": hash, "kind": kind, "adapter": "claudeHook", "canRespond": true, "canHandoff": true,
                "needsClaudeHook": false, "requestId": "req-1", "title": "Fixture", "questions": [], "textPrompt": "", "readonlyReason": "",
            ]
            for (key, item) in extra { value[key] = item }
            return value
        }
        let approval: [String: Any] = ["toolName": "Bash", "reason": "Run the test suite", "command": (1...40).map { "echo line \($0)" }.joined(separator: "\n"), "details": "cwd: /work",
                                       "decisions": [["id": "approve", "label": "允许本次", "style": "primary", "behavior": "allow", "scope": "once"],
                                                     ["id": "approveSession", "label": "本会话内允许", "style": "secondary", "behavior": "allow", "scope": "session", "description": "之后同类命令不再询问"],
                                                     ["id": "decline", "label": "拒绝", "style": "danger", "behavior": "deny", "acceptsReason": true],
                                                     ["id": "abort", "label": "拒绝并中断", "style": "danger", "behavior": "deny", "interrupts": true]]]
        try write("status.json", ["enabled": true, "autoPopup": true, "claudeQuestions": true, "claudeApprovals": true, "allowTerminal": false])
        try write("payload.json", card(kind: "approval", extra: ["approval": approval]))

        let store = SessionInteractionStore()
        defer { store.stop() }
        // Nothing in this run may touch a real terminal or window.
        var typed: [(session: String, text: String)] = []
        var terminalOutcome = SessionChannelAutomation.Outcome(status: "submitted", reason: "已发送到原终端")
        store.deliverToTerminal = { session, text, _ in typed.append((session.id, text)); return terminalOutcome }
        var focused: [String] = []
        store.focusSession = { session in focused.append(session.id) }
        await store.refreshStatus()
        precondition(store.status.autoPopup && store.error == nil, store.error ?? "status")

        // Auto-popup: the first list is the baseline, later requests queue, and one card opens at a time.
        var requested: [String] = []
        var accept = false
        store.onAutoPopupRequested = { session in requested.append(session.id); return accept }
        let a = waiting("a"), b = waiting("b", provider: "codex"), c = waiting("c")
        store.observeSessions([a])
        precondition(requested.isEmpty, "Sessions already waiting when monitoring starts never pop up")
        store.observeSessions([a, b, c])
        precondition(requested == ["b"], "A refused request is offered, not dropped")
        store.observeSessions([a, b, c])
        precondition(requested == ["b", "b"], "A refused request keeps its place and is offered again")
        accept = true
        store.drainAutoPopup()
        precondition(requested == ["b", "b", "b"], "Only one card opens per offer")
        store.drainAutoPopup()
        precondition(requested == ["b", "b", "b", "c"], "The next queued request opens after the first closes")
        store.drainAutoPopup(); store.observeSessions([a, b, c])
        precondition(requested.count == 4, "A shown request never pops up twice")
        // A request the user dismissed while it was queued is not shown; a new prompt of the same session is.
        accept = false
        let d = waiting("d"), e = waiting("e")
        store.observeSessions([a, b, c, d, e])
        precondition(requested.last == "d" && requested.count == 5)
        store.markDismissed(sessions: [d])
        accept = true
        store.drainAutoPopup()
        precondition(requested.last == "e" && requested.count == 6)
        store.observeSessions([a, b, c, waiting("d", state: "s2"), e])
        precondition(requested.last == "d" && requested.count == 7, "A new prompt of a dismissed session pops up")
        // A queued request that stops waiting is discarded.
        accept = false
        store.observeSessions([a, waiting("f")])
        store.observeSessions([a])
        accept = true
        store.drainAutoPopup()
        precondition(requested.filter { $0 == "f" }.count == 1 && requested.last == "f")
        // An auto-opened popover is resolved elsewhere (gResolved in the terminal). The host, modelled
        // here as AppDelegate does it, closes it on that session update without dismissing anything,
        // and the request queued behind it opens in the same update.
        var shown: (provider: String, auto: Bool)?
        var closedResolved = 0, dismissedOnClose = 0
        var listed: [LiveSession] = []
        func hostWaiting() -> [LiveSession] { listed.filter { $0.phase == .needsInput && $0.provider == shown?.provider } }
        store.onAutoPopupRequested = { session in
            requested.append(session.id)
            guard shown == nil else { return false }
            shown = (session.provider, true)
            return true
        }
        store.onSessionsObserved = {
            guard let open = shown else { return }
            let waiting = hostWaiting()
            guard InteractionPopoverPolicy.shouldClose(.resolved, autoOpened: open.auto, hasDraft: store.hasActiveDraft(in: waiting), hasWaiting: !waiting.isEmpty) else { return }
            if InteractionPopoverPolicy.marksDismissed(.resolved) { dismissedOnClose += 1; store.markDismissed(sessions: waiting) }
            shown = nil
            closedResolved += 1
        }
        func observe(_ sessions: [LiveSession]) { listed = sessions; store.observeSessions(sessions) }
        let g = waiting("g", provider: "gemini"), h = waiting("h", provider: "codex")
        observe([a, g])
        precondition(shown?.provider == "gemini" && requested.last == "g", "The new request opens its card")
        observe([a, g, h])
        precondition(shown?.provider == "gemini" && requested.last == "h" && closedResolved == 0, "A request behind an open card waits, and a waiting card stays open")
        let asked = requested.count
        var gResolved = g; gResolved.phase = .working
        observe([a, gResolved, h])
        precondition(closedResolved == 1 && dismissedOnClose == 0, "A card resolved elsewhere closes without being dismissed")
        precondition(shown?.provider == "codex" && requested.count == asked + 1 && requested.last == "h", "The queued request opens once the resolved card is gone: \(requested)")
        // A new prompt of the session whose card was closed that way still pops up.
        var done = h; done.phase = .working
        observe([a, gResolved, done, waiting("g", state: "s2", provider: "gemini")])
        precondition(closedResolved == 2 && shown?.provider == "gemini" && requested.last == "g", "A later prompt of the resolved session pops up again")
        // A popover the user opened by hand is not closed by a session update, and keeps holding the queue.
        observe([a, gResolved, done])
        precondition(closedResolved == 3 && shown == nil)
        shown = ("gemini", false)
        observe([a, gResolved, done, waiting("i", provider: "codex")])
        precondition(closedResolved == 3 && shown?.provider == "gemini" && requested.last == "i", "A hand-opened popover stays")
        shown = nil
        store.drainAutoPopup()
        precondition(shown?.provider == "codex" && shown?.auto == true)
        // The agent of the open card still has another request waiting: it stays.
        observe([a, gResolved, done, waiting("i", provider: "codex"), waiting("j", provider: "codex")])
        var iDone = waiting("i", provider: "codex"); iDone.phase = .working
        observe([a, gResolved, done, iDone, waiting("j", provider: "codex")])
        precondition(closedResolved == 3 && shown?.provider == "codex", "A card stays while its agent has a request waiting")
        observe([a, gResolved, done, iDone])
        precondition(closedResolved == 4 && shown == nil, "The card closes with the agent's last waiting session")
        // The update that ends a wait drops its draft first, so a resolved card is never held open by one.
        try write("payload.json", card(kind: "text", state: "k1", extra: ["sessionId": "k"]))
        let k = waiting("k", state: "k1", provider: "gemini")
        observe([a, k])
        precondition(shown?.provider == "gemini")
        await store.inspect(session: k)
        guard let typing = store.payloads["k"] else { preconditionFailure("The text card did not load") }
        store.updateDraft(for: typing) { $0.textReply = "half typed" }
        precondition(store.hasActiveDraft(for: "k"), "Typed text counts as a draft")
        observe([a, k])
        precondition(closedResolved == 4 && shown != nil, "A waiting card with a draft stays")
        var kDone = k; kDone.phase = .working
        observe([a, kDone])
        precondition(closedResolved == 5 && shown == nil && !store.hasActiveDraft(for: "k"), "A resolved card closes; its draft ended with the wait")
        try write("payload.json", card(kind: "approval", extra: ["approval": approval]))
        observe([a])
        // The checks below count core calls from here.
        try FileManager.default.removeItem(at: root.appendingPathComponent("calls"))
        store.onSessionsObserved = nil
        store.onAutoPopupRequested = nil

        // Inspect caches by state key and reloads on force.
        await store.inspect(session: a)
        precondition(store.payloads["a"]?.kind == "approval" && store.payloads["a"]?.approval?.command.contains("line 40") == true)
        await store.inspect(session: a)
        precondition(calls("sessions.interaction.inspect").count == 1, "A loaded prompt is not fetched again")
        precondition(calls("sessions.interaction.inspect")[0].contains("\"capabilities\": {\"localTerminal\": true}"), "Inspect declares local terminal delivery")
        precondition(store.payloads["a"]?.approval?.decisions.map(\.id) == ["approve", "approveSession", "decline", "abort"])
        await store.inspect(session: a, force: true)
        precondition(calls("sessions.interaction.inspect").count == 2)

        // A forced inspect that arrives while another is in flight is run afterwards, not dropped.
        try block("sessions.interaction.inspect")
        let slow = Task { await store.inspect(session: a, force: true) }
        try await waitFor("The blocked inspect did not start") { exists("started") }
        let a2 = waiting("a", state: "s2")
        try write("payload.json", card(kind: "approval", state: "s2", hash: "h2", extra: ["approval": approval]))
        await store.inspect(session: a2, force: true)
        await store.inspect(session: a2, force: true)
        precondition(store.loadingSessionIDs.contains("a") && calls("sessions.interaction.inspect").count == 3)
        try release()
        await slow.value
        precondition(store.payloads["a"]?.stateKey == "s2", "The card shows the prompt the dropped call asked for")
        precondition(calls("sessions.interaction.inspect").count == 4 && store.loadingSessionIDs.isEmpty, "Repeated calls for one state key add one fetch")

        // A stale card is reloaded once when the session list reports a newer state key.
        try write("payload.json", card(kind: "approval", state: "s3", hash: "h3", extra: ["approval": approval]))
        let a3 = waiting("a", state: "s3")
        store.observeSessions([a3])
        try await waitFor("A stale card was not reloaded") { store.payloads["a"]?.stateKey == "s3" }
        let reloads = calls("sessions.interaction.inspect").count
        // Core lagging behind the list must not cause a reload loop.
        try write("payload.json", card(kind: "approval", state: "s3", hash: "h3", extra: ["approval": approval]))
        let a4 = waiting("a", state: "s4")
        store.observeSessions([a4])
        try await waitFor("The lagging reload did not run") { calls("sessions.interaction.inspect").count == reloads + 1 && store.loadingSessionIDs.isEmpty }
        store.observeSessions([a4]); store.observeSessions([a4])
        try await Task.sleep(for: .milliseconds(150))
        precondition(calls("sessions.interaction.inspect").count == reloads + 1, "One recovery reload per state key")

        // A blocked response reloads the card and keeps the refusal visible.
        var current = store.payloads["a"]!
        try write("payload.json", card(kind: "approval", state: "s4", hash: "h4", extra: ["approval": approval]))
        try write("respond.json", ["status": "blocked", "message": "审批请求已变化，请刷新后再操作"])
        let blocked = await store.respond(session: a4, payload: current, action: "decide", decisionId: "approveSession")
        precondition(!blocked && store.feedback["a"]?.status == "blocked" && store.feedback["a"]?.message == "审批请求已变化，请刷新后再操作")
        precondition(store.payloads["a"]?.promptHash == "h4" && !store.isCompleted(store.payloads["a"]!), "The card now shows the current prompt")
        let first = calls("sessions.interaction.respond").last ?? ""
        precondition(first.contains("\"promptHash\": \"h3\"") && first.contains("\"action\": \"decide\"") && first.contains("\"decisionId\": \"approveSession\""), first)
        precondition(first.contains("\"requestId\": \"req-1\"") && first.contains("\"capabilities\": {\"localTerminal\": true}"), first)

        // After a confirmed answer the card stays locked: a second click sends nothing.
        current = store.payloads["a"]!
        store.updateDraft(for: current) { $0.declineReason = "not now" }
        precondition(store.hasActiveDraft(for: "a") && store.hasActiveDraft(in: [b, a4]) && !store.hasActiveDraft(in: [b]))
        try write("respond.json", ["status": "confirmed", "message": "已拒绝本次操作"])
        let declined = await store.respond(session: a4, payload: current, action: "decide", decisionId: "decline", reason: "not now")
        precondition(declined && store.isCompleted(current) && store.feedback["a"]?.status == "confirmed")
        precondition(store.drafts.isEmpty && !store.hasActiveDraft(for: "a") && store.submittingSessionIDs.isEmpty)
        let sent = calls("sessions.interaction.respond")
        precondition(sent.last?.contains("\"decisionId\": \"decline\"") == true && sent.last?.contains("\"reason\": \"not now\"") == true && sent.last?.contains("\"stateKey\": \"s4\"") == true)
        let again = await store.respond(session: a4, payload: current, action: "decide", decisionId: "approve")
        let handedBack = await store.handoff(session: a4, payload: current)
        precondition(!again && !handedBack, "An answered prompt accepts no second answer")
        precondition(calls("sessions.interaction.respond").count == sent.count && calls("sessions.interaction.handoff").isEmpty)
        precondition(store.feedback["a"]?.status == "confirmed", "The success message is not replaced")
        // An inspect of the same prompt keeps it locked; a manual refresh unlocks it.
        await store.inspect(session: a4, force: true)
        precondition(store.isCompleted(store.payloads["a"]!))
        await store.refresh(session: a4)
        precondition(!store.isCompleted(store.payloads["a"]!) && store.feedback["a"] == nil)

        // Handoff locks the card as well; a refused handoff does not.
        try write("handoff.json", ["handedOff": false, "message": "当前没有可交还终端的挂起请求"])
        let refusedHandoff = await store.handoff(session: a4, payload: current)
        precondition(!refusedHandoff && store.feedback["a"]?.status == "blocked" && !store.isCompleted(current))
        try write("handoff.json", ["handedOff": true, "message": "已交还给原终端确认"])
        let handed = await store.handoff(session: a4, payload: current)
        precondition(handed && store.isCompleted(current) && calls("sessions.interaction.handoff").last?.contains("\"requestId\": \"req-1\"") == true)
        precondition(focused == ["a"] && store.feedback["a"]?.status == "confirmed", "A hand-back brings the session's window forward")
        // An unconfirmed hand-back is not reported as done.
        await store.refresh(session: a4)
        try write("handoff.json", ["handedOff": true, "confirmed": false, "message": "已请求交还，等待原终端接管"])
        let pendingHandoff = await store.handoff(session: a4, payload: current)
        precondition(pendingHandoff && store.feedback["a"]?.status == "submitted")
        // The lock ends with the wait; state of sessions that left is pruned.
        store.updateDraft(for: current) { $0.textReply = "left behind" }
        store.observeSessions([b])
        precondition(store.payloads["a"] == nil && store.feedback["a"] == nil && store.completedKeys.isEmpty)
        precondition(store.drafts.isEmpty, "Drafts of a finished wait are dropped")

        // Question answers go out in the shape core expects.
        let questions: [[String: Any]] = [
            ["id": "1", "key": "mode", "question": "Mode?", "kind": "string", "multiSelect": false, "allowCustom": false,
             "options": [["label": "Same", "value": "one"], ["label": "Same", "value": "two", "preview": "diff"]]],
            ["id": "2", "key": "tags", "question": "Tags?", "kind": "string", "multiSelect": true, "allowCustom": true, "options": [["label": "x"], ["label": "y"]]],
        ]
        try write("payload.json", card(kind: "questions", state: "q1", hash: "hq", extra: ["questions": questions, "adapter": "opencode"]))
        let q = waiting("a", state: "q1")
        store.observeSessions([b, q])
        await store.inspect(session: q)
        let form = store.payloads["a"]!
        precondition(form.questions.count == 2 && form.questions[0].options[1].value == "two" && form.questions[0].options[1].preview == "diff")
        store.updateDraft(for: form) { $0.singleChoices["1"] = 1; $0.multiChoices["2"] = [1, 0] }
        try write("respond.json", ["status": "submitted", "message": "已提交"])
        let answered = await store.respond(session: q, payload: form, action: "answer", answers: InteractionAnswers.answers(for: form, draft: store.draftBinding(for: form)))
        precondition(answered && calls("sessions.interaction.respond").last?.contains("\"answers\": {\"1\": \"two\", \"2\": [\"x\", \"y\"]}") == true, calls("sessions.interaction.respond").last ?? "")
        // A core error is reported and leaves the card answerable.
        try write("respond.json", ["rpcError": "core failed"])
        await store.refresh(session: q)
        let failed = await store.respond(session: q, payload: form, action: "answer", answers: [:])
        precondition(!failed && store.feedback["a"]?.status == "error" && store.feedback["a"]?.message.contains("core failed") == true && !store.isCompleted(form))

        try await verifyStatuses(store: store, root: root, calls: calls, write: write, typed: { typed }, setOutcome: { terminalOutcome = $0 })

        // Every card kind lays out, including the answered and the not-yet-loaded states.
        _ = NSApplication.shared
        func fitted(_ session: LiveSession) -> NSSize {
            let host = NSHostingView(rootView: PetInteractionCardView(session: session, store: store, onFocusSession: {}, onCompleted: {}).frame(width: 340))
            host.layoutSubtreeIfNeeded()
            return host.fittingSize
        }
        var heights: [String: CGFloat] = [:]
        let kinds: [(String, [String: Any])] = [
            ("approval", ["approval": approval, "textPrompt": "Allow Bash?"]),
            ("questions", ["questions": questions]),
            ("text", ["adapter": "tmux", "textPrompt": "Continue?"]),
            ("readonly", ["canRespond": false, "textPrompt": "Pick one", "inputKind": "question", "quickEnable": "questions", "readonlyReason": "此终端没有可核验的审批接口，请在原窗口确认"]),
            ("none", ["canRespond": false, "readonlyReason": "当前会话已结束或不可用"]),
        ]
        for (kind, extra) in kinds {
            try write("payload.json", card(kind: kind, state: "k-" + kind, hash: kind, extra: extra))
            let session = waiting("a", state: "k-" + kind)
            await store.refresh(session: session)
            precondition(store.payloads["a"]?.kind == kind, kind)
            let size = fitted(session)
            precondition(size.width >= 340 && size.height > 40, "\(kind) card did not lay out: \(size)")
            heights[kind] = size.height
        }
        precondition(heights["approval"]! > heights["none"]! && heights["questions"]! > heights["none"]!, "\(heights)")
        // The newer cards: plan, file changes, MCP form, generic text without a prompt, read-only approval, queue.
        let planApproval: [String: Any] = ["toolName": "ExitPlanMode", "kind": "plan", "title": "计划审批", "plan": ["markdown": "# Plan\n1. Read\n2. Edit\n```\nmake test\n```", "truncated": true],
                                           "decisions": [["id": "auto", "label": "批准并自动接受编辑", "style": "primary", "behavior": "allow"], ["id": "manual", "label": "批准并逐项确认", "style": "secondary", "behavior": "allow"],
                                                         ["id": "revise", "label": "继续修改计划", "style": "danger", "behavior": "deny", "acceptsReason": true]]]
        let changeApproval: [String: Any] = ["toolName": "apply_patch", "kind": "fileChange", "cwd": "/work", "paths": ["/work/a.txt"],
                                             "changes": [["path": "/work/a.txt", "kind": "update", "diff": "@@ -1 +1 @@\n-old\n+new", "truncated": true], ["path": "/work/b.txt", "kind": "add", "newText": "hello"],
                                                         ["path": "/work/c.txt", "kind": "move", "movePath": "/work/d.txt"], ["path": "/work/e.txt", "kind": "delete", "oldText": "bye"]],
                                             "decisions": approval["decisions"]!]
        let fields: [[String: Any]] = [
            ["id": "token", "question": "API token", "header": "Token", "kind": "string", "secret": true, "description": "Stored by the server"],
            ["id": "ok", "question": "Enable?", "kind": "boolean", "required": false],
            ["id": "n", "question": "How many?", "kind": "integer", "placeholder": "1-5", "default": 9, "constraints": ["minimum": 1, "maximum": 5]],
        ]
        let more: [(String, String, [String: Any])] = [
            ("plan", "approval", ["approval": planApproval]),
            ("fileChange", "approval", ["approval": changeApproval, "pendingCount": 3, "requestedAt": 1_700_000_000]),
            ("form", "questions", ["questions": fields, "adapter": "claudeHook", "form": ["title": "Sign in", "description": "The server needs a token", "source": "mcp", "serverName": "github", "url": "https://example.com/auth", "canCancel": true, "canSkip": true]]),
            ("emptyText", "text", ["adapter": "macosTerminal", "textPrompt": ""]),
            ("lastMessage", "text", ["adapter": "tmux", "textPrompt": "Which branch?", "promptSource": "lastMessage"]),
            ("readonlyApproval", "approval", ["canRespond": false, "approval": approval, "quickEnable": "approvals", "needsClaudeApprovals": true, "readonlyReason": "未启用弹窗审批"]),
            ("queue", "approval", ["approval": approval, "pendingCount": 2, "pendingRequests": [["requestId": "req-1", "kind": "approval", "title": "Bash"], ["requestId": "req-2", "kind": "questions", "title": "Pick"]]]),
            ("interruptible", "approval", ["approval": approval, "followUp": ["available": false, "canInterrupt": true]]),
        ]
        for (name, kind, extra) in more {
            try write("payload.json", card(kind: kind, state: "m-" + name, hash: name, extra: extra))
            let session = waiting("a", state: "m-" + name)
            await store.refresh(session: session)
            precondition(store.payloads["a"]?.kind == kind && store.feedback["a"] == nil, name + ": " + (store.feedback["a"]?.message ?? ""))
            let size = fitted(session)
            precondition(size.width >= 340 && size.height > 60, "\(name) card did not lay out: \(size)")
            heights[name] = size.height
        }
        precondition(heights["plan"]! > heights["none"]! && heights["fileChange"]! > heights["approval"]!, "\(heights)")
        precondition(heights["emptyText"]! > heights["none"]!, "A text card without a prompt still says what it waits for and offers its field: \(heights)")
        precondition(heights["interruptible"]! > heights["approval"]! && heights["queue"]! > heights["approval"]!, "\(heights)")
        precondition(heights["readonlyApproval"]! > heights["none"]!, "\(heights)")
        // The number field of the form starts from its default, which is out of range and says so.
        try write("payload.json", card(kind: "questions", state: "m-form", hash: "form", extra: ["questions": fields, "form": ["source": "mcp", "canCancel": true]]))
        await store.refresh(session: waiting("a", state: "m-form"))
        let loaded = store.payloads["a"]!
        precondition(store.draftBinding(for: loaded).customAnswers["n"] == "9" && !store.hasActiveDraft(for: "a"), "A default is not unsent user input")
        precondition(InteractionAnswers.problem(with: loaded.questions[2], in: store.draftBinding(for: loaded)) == "数值超出允许范围")
        // Follow-up cards of sessions that are not waiting, with and without a way to send.
        var running = LiveSession(id: "a", source: "fixture", phase: .working, updatedAt: Date(timeIntervalSince1970: 100))
        running.provider = "opencode"; running.machineId = "local"; running.machine = "mac"
        store.observeSessions([b, running])
        for (name, kind, extra) in [
            ("followup", "followup", ["adapter": "opencode", "textPrompt": "Done so far", "promptSource": "lastMessage", "followUp": ["available": true, "adapter": "opencode", "mode": "queue", "canInterrupt": true]] as [String: Any]),
            ("noFollowup", "none", ["canRespond": false, "followUp": ["available": false, "reason": "此会话的终端无法准确定位，请在原窗口输入"]] as [String: Any]),
        ] {
            try write("payload.json", card(kind: kind, state: "f-" + name, hash: name, extra: extra))
            await store.refresh(session: running)
            precondition(store.payloads["a"]?.kind == kind, name)
            let size = fitted(running)
            precondition(size.width >= 340 && size.height > 60, "\(name) card did not lay out: \(size)")
            heights[name] = size.height
        }
        precondition(heights["followup"]! > heights["noFollowup"]!, "\(heights)")
        // A session that starts asking while its follow-up card is still being read: the card that
        // lands is recorded as a waiting one, so the next session update does not prune it.
        for force in [true, false] {
            store.observeSessions([b, running])
            try write("payload.json", card(kind: "followup", state: "f-flip", hash: "flip", extra: ["followUp": ["available": true]]))
            await store.refresh(session: running)
            let fetched = calls("sessions.interaction.inspect").count
            try block("sessions.interaction.inspect")
            let flip = Task { await store.inspect(session: running, force: true) }
            try await waitFor("The blocked inspect did not start") { exists("started") }
            let asking = waiting("a", state: "w-flip")
            try write("payload.json", card(kind: "approval", state: "w-flip", hash: "wflip", extra: ["approval": approval]))
            await store.inspect(session: asking, force: force)
            try release()
            await flip.value
            precondition(calls("sessions.interaction.inspect").count == fetched + 2, "The new state is fetched after the running read, force: \(force)")
            store.observeSessions([b, asking])
            precondition(store.payloads["a"]?.stateKey == "w-flip" && store.payloads["a"]?.kind == "approval", "The fresh waiting card survives the session update, force: \(force)")
            store.observeSessions([b, asking])
            precondition(store.payloads["a"]?.stateKey == "w-flip")
        }
        precondition(fitted(waiting("unloaded")).height > 40, "A card without a payload shows its loading row")
        store.stop()

        // The live baseline is the first list core returns, not the empty placeholder before it.
        let model = AppModel(autostart: false)
        defer { model.interaction.stop() }
        var popped: [String] = []
        var petShown = true
        model.interaction.onAutoPopupRequested = { session in popped.append(session.id); return petShown }
        model.interaction.start(model)
        try await waitFor("The interaction status did not load") { model.interaction.status.autoPopup }
        try await Task.sleep(for: .milliseconds(100))
        model.live.sessions = [a]
        model.live.loaded = true
        try await Task.sleep(for: .milliseconds(150))
        precondition(popped.isEmpty, "A session already waiting at launch must not pop up: \(popped)")
        model.live.sessions = [a, b]
        try await waitFor("A newly waiting session did not pop up") { popped == ["b"] }
        // A request refused while the pet is hidden is offered again when the pet returns, although
        // the session list did not change in between.
        petShown = false
        model.live.petVisible = false
        model.live.sessions = [a, b, c]
        try await waitFor("A request behind a hidden pet was not offered") { popped == ["b", "c"] }
        try await Task.sleep(for: .milliseconds(150))
        precondition(popped == ["b", "c"], "Nothing offers the refused request while the pet stays hidden: \(popped)")
        petShown = true
        model.live.petVisible = true
        try await waitFor("The refused request was not offered when the pet returned") { popped == ["b", "c", "c"] }
        model.live.petVisible = false
        model.live.petVisible = true
        try await Task.sleep(for: .milliseconds(150))
        precondition(popped.count == 3, "A shown request is not offered twice: \(popped)")
        print("Session interaction verification passed")
    }
}
