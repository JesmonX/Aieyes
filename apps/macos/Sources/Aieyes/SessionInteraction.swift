import AppKit
import Combine
import Foundation

struct InteractionHookEntry: Decodable, Equatable, Identifiable {
    var sourceId: String
    var provider: String
    var path: String
    var mode: String
    var id: String { sourceId + ":" + path }
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
}

struct InteractionDecision: Decodable, Equatable, Identifiable {
    var id: String
    var label: String
    var style: String
}

struct InteractionApproval: Decodable, Equatable {
    var toolName: String
    var reason: String
    var command: String
    var details: String
    var decisions: [InteractionDecision]
}

struct InteractionOption: Decodable, Equatable, Identifiable {
    var label: String
    var description: String?
    var preview: String?
    var id: String { label + "|" + (description ?? "") }
}

struct InteractionQuestion: Decodable, Equatable, Identifiable {
    var id: String
    var key: String?
    var question: String
    var kind: String?
    var multiSelect: Bool
    var allowCustom: Bool
    var options: [InteractionOption]
}

struct InteractionPayload: Decodable, Equatable {
    var sessionId: String
    var stateKey: String
    var promptHash: String
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
}

struct InteractionDraft: Equatable {
    var singleChoices: [String: String] = [:]
    var multiChoices: [String: Set<String>] = [:]
    var customAnswers: [String: String] = [:]
    var textReply: String = ""
    var declineReason: String = ""
    var hasUserInput: Bool {
        !singleChoices.isEmpty
            || multiChoices.values.contains(where: { !$0.isEmpty })
            || customAnswers.values.contains(where: { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
            || !textReply.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !declineReason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
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

    /// Called when a newly observed `needsInput` session should auto-popup its orbit agent card.
    var onAutoPopupRequested: ((LiveSession) -> Void)?

    private let client = EngineClient(configurationOnly: true)
    private var subscriptions = Set<AnyCancellable>()
    private var dismissedKeys = Set<String>()
    private var knownWaitingKeys = Set<String>()
    private var seeded = false
    private weak var model: AppModel?

    func start(_ model: AppModel) {
        self.model = model
        Task { await refreshStatus() }
        model.live.$sessions
            .receive(on: DispatchQueue.main)
            .sink { [weak self] sessions in
                self?.observeSessions(sessions)
            }
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

    func markDismissed(sessions: [LiveSession]) {
        for session in sessions where session.phase == .needsInput {
            dismissedKeys.insert(Self.dedupKey(for: session))
        }
    }

    func draftKey(for payload: InteractionPayload) -> String {
        "\(payload.sessionId)|\(payload.stateKey)|\(payload.promptHash)"
    }

    func hasActiveDraft(for sessionID: String) -> Bool {
        guard let payload = payloads[sessionID] else { return false }
        return drafts[draftKey(for: payload)]?.hasUserInput == true
    }

    func draftBinding(for payload: InteractionPayload) -> InteractionDraft {
        drafts[draftKey(for: payload)] ?? InteractionDraft()
    }

    func updateDraft(for payload: InteractionPayload, _ mutate: (inout InteractionDraft) -> Void) {
        let key = draftKey(for: payload)
        var current = drafts[key] ?? InteractionDraft()
        mutate(&current)
        drafts[key] = current
    }

    private func observeSessions(_ sessions: [LiveSession]) {
        let activeWaiting = sessions.filter { $0.phase == .needsInput }
        let currentKeys = Set(activeWaiting.map(Self.dedupKey))
        if !seeded {
            seeded = true
            knownWaitingKeys = currentKeys
            return
        }
        // Prune stale payloads for sessions that are no longer waiting.
        let waitingIDs = Set(activeWaiting.map(\.id))
        for id in Array(payloads.keys) where !waitingIDs.contains(id) {
            payloads.removeValue(forKey: id)
            feedback.removeValue(forKey: id)
        }
        let newlyWaiting = activeWaiting.filter { session in
            let key = Self.dedupKey(for: session)
            return !knownWaitingKeys.contains(key) && !dismissedKeys.contains(key)
        }
        knownWaitingKeys = currentKeys
        // Keep dismissed set bounded to active keys.
        dismissedKeys = dismissedKeys.intersection(currentKeys)

        guard status.enabled && status.autoPopup, let candidate = newlyWaiting.first else { return }
        dismissedKeys.insert(Self.dedupKey(for: candidate))
        onAutoPopupRequested?(candidate)
    }

    func inspect(session: LiveSession, force: Bool = false) async {
        guard session.phase == .needsInput else {
            payloads.removeValue(forKey: session.id)
            return
        }
        let expectedKey = session.inputStateKey
        if !force,
           let existing = payloads[session.id],
           expectedKey == nil || existing.stateKey == expectedKey,
           existing.kind != "none" {
            return
        }
        if loadingSessionIDs.contains(session.id) { return }
        loadingSessionIDs.insert(session.id)
        defer { loadingSessionIDs.remove(session.id) }
        do {
            let result: InteractionPayload = try await client.call(
                "sessions.interaction.inspect",
                params: ["sessionId": session.id]
            )
            payloads[session.id] = result
            if result.kind != "none" {
                feedback.removeValue(forKey: session.id)
            }
        } catch {
            feedback[session.id] = ("error", error.localizedDescription)
        }
    }

    func respond(
        session: LiveSession,
        payload: InteractionPayload,
        action: String,
        answers: [String: Any]? = nil,
        text: String? = nil,
        reason: String? = nil
    ) async -> Bool {
        guard !submittingSessionIDs.contains(session.id) else { return false }
        submittingSessionIDs.insert(session.id)
        defer { submittingSessionIDs.remove(session.id) }
        var params: [String: Any] = [
            "sessionId": payload.sessionId,
            "stateKey": payload.stateKey,
            "promptHash": payload.promptHash,
            "action": action,
        ]
        if let answers { params["answers"] = answers }
        if let text { params["text"] = text }
        if let reason, !reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            params["reason"] = reason
        }
        do {
            let result: InteractionRespondResult = try await client.call("sessions.interaction.respond", params: params)
            if result.status == "needsLocalTerminal", let job = result.terminalJob {
                let delivered = await SessionChannelAutomation.deliver(job.session, text: job.text, client: client)
                feedback[session.id] = (delivered.status, L(delivered.reason))
                if delivered.status == "submitted" || delivered.status == "confirmed" {
                    drafts.removeValue(forKey: draftKey(for: payload))
                    dismissedKeys.insert(Self.dedupKey(for: session))
                    return true
                }
                return false
            }
            feedback[session.id] = (result.status, result.message)
            if result.status == "confirmed" || result.status == "submitted" {
                drafts.removeValue(forKey: draftKey(for: payload))
                dismissedKeys.insert(Self.dedupKey(for: session))
                return true
            }
            return false
        } catch {
            feedback[session.id] = ("error", error.localizedDescription)
            return false
        }
    }

    func handoff(session: LiveSession, payload: InteractionPayload) async -> Bool {
        guard !submittingSessionIDs.contains(session.id) else { return false }
        submittingSessionIDs.insert(session.id)
        defer { submittingSessionIDs.remove(session.id) }
        do {
            let result: InteractionHandoffResult = try await client.call(
                "sessions.interaction.handoff",
                params: ["sessionId": payload.sessionId, "requestId": payload.requestId]
            )
            feedback[session.id] = (result.handedOff ? "confirmed" : "blocked", result.message)
            if result.handedOff {
                dismissedKeys.insert(Self.dedupKey(for: session))
                _ = await SessionFocus.focus(session, engine: SessionFocus.engine)
            }
            return result.handedOff
        } catch {
            feedback[session.id] = ("error", error.localizedDescription)
            return false
        }
    }
}
