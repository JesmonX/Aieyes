import AppKit
import Carbon
import Darwin
import Foundation

struct ChatChannelStatus: Decodable, Equatable {
    struct Connection: Decodable, Equatable { var state: String; var error: String? }
    struct Failure: Decodable, Equatable { var message: String }
    var enabled: Bool, appId: String, hasSecret: Bool, revision: Int, paired: Bool
    var ownerOpenId: String?, allowTerminal: Bool, connection: Connection, pending: Int
    var deliveryError: Failure?, workerError: Failure?, undelivered: Int?
    var connectionTitle: String {
        switch connection.state {
        case "connected": return L("已连接")
        case "connecting": return L("正在连接")
        case "reconnecting": return L("正在重连")
        case "disabled": return L("已停用")
        default: return L("未连接")
        }
    }
}
struct ChatChannelPairing: Decodable { var code: String, message: String, expiresAt: Double }
struct ChatChannelCommand: Decodable, Identifiable, Equatable {
    var id: String, sessionId: String, body: String, status: String, reason: String
    var title: String {
        switch status {
        case "queued": return L("等待提交")
        case "blocked": return L("待处理")
        case "automation": return L("等待桌面操作")
        case "dispatching": return L("正在提交")
        case "submitted": return L("已提交，待确认")
        case "confirmed": return L("已确认")
        case "uncertain": return L("结果待核对")
        case "cancelled": return L("已取消")
        default: return L("未知状态")
        }
    }
    var canCancel: Bool { ["queued", "blocked", "automation"].contains(status) }
}
struct ChatChannelSession: Decodable, Identifiable, Equatable { var id: String, key: String, title: String, phase: String, control: String }
private struct ChatChannelCommands: Decodable { var commands: [ChatChannelCommand] }
private struct ChatChannelSessions: Decodable { var sessions: [ChatChannelSession] }
private struct ChatChannelEmpty: Decodable {}
private struct ChatChannelJob: Decodable { var id: String, lease: String, text: String, session: LiveSession }
private struct ChatChannelJobResponse: Decodable { var job: ChatChannelJob? }

/// Keeps the core's channel worker alive independently of visible windows. The only native
/// work here is an explicitly enabled, already-authorized terminal adapter.
@MainActor final class ChatChannelService: ObservableObject {
    @Published private(set) var status: ChatChannelStatus?
    @Published private(set) var commands: [ChatChannelCommand] = []
    @Published private(set) var sessions: [ChatChannelSession] = []
    @Published private(set) var pairing: ChatChannelPairing?
    @Published private(set) var error: String?
    @Published private(set) var busy = false
    var settingsVisible = false
    private weak var model: AppModel?
    private let client = EngineClient(configurationOnly: true)
    private var task: Task<Void, Never>?
    private var refreshing = false

    func start(_ model: AppModel) {
        guard task == nil else { return }
        self.model = model
        task = Task { [weak self] in
            var tick = 0
            while !Task.isCancelled {
                guard let self else { return }
                if self.model?.installingUpdate != true {
                    if tick % 15 == 0 {
                        do { let _: ChatChannelEmpty = try await self.client.call("channels.start") }
                        catch { self.error = error.localizedDescription }
                    }
                    if tick % 5 == 0 { await self.refresh() }
                    if self.status?.enabled == true, self.status?.allowTerminal == true {
                        await self.automation()
                    }
                }
                tick += 1
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
            }
        }
    }
    func refresh() async {
        guard !refreshing else { return }
        refreshing = true; defer { refreshing = false }
        do {
            let value: ChatChannelStatus = try await client.call("channels.status")
            if value != status { status = value }
            if value.paired { pairing = nil }
            if settingsVisible {
                let rows: ChatChannelCommands = try await client.call("channels.commands")
                let targets: ChatChannelSessions = try await client.call("channels.sessions")
                if rows.commands != commands { commands = rows.commands }
                if targets.sessions != sessions { sessions = targets.sessions }
            }
            error = nil
        } catch { self.error = error.localizedDescription }
    }
    func configure(revision: Int, enabled: Bool, appID: String, secret: String, allowTerminal: Bool) async throws {
        busy = true; defer { busy = false }
        let value: ChatChannelStatus = try await client.call("channels.configure", params: ["revision": revision, "enabled": enabled, "appId": appID, "appSecret": secret, "allowTerminal": allowTerminal])
        status = value; pairing = nil; error = nil
        await refresh()
    }
    func pair() async {
        guard !busy else { return }
        busy = true; defer { busy = false }
        do { pairing = try await client.call("channels.pair"); error = nil }
        catch { self.error = error.localizedDescription }
    }
    func unpair() async {
        guard !busy, let status else { return }
        busy = true; defer { busy = false }
        do {
            self.status = try await client.call("channels.unpair", params: ["revision": status.revision])
            pairing = nil; commands = []; error = nil
        } catch { self.error = error.localizedDescription }
    }
    func update(_ command: ChatChannelCommand, retry: Bool) async {
        guard !busy else { return }
        busy = true; defer { busy = false }
        do {
            let _: ChatChannelEmpty = try await client.call(retry ? "channels.retry" : "channels.cancel", params: ["id": command.id])
            error = nil; await refresh()
        } catch { self.error = error.localizedDescription }
    }
    private func automation() async {
        do {
            let response: ChatChannelJobResponse = try await client.call("channels.automation.take")
            guard let job = response.job else { return }
            let outcome = await SessionChannelAutomation.deliver(job.session, text: job.text, client: client)
            let _: ChatChannelEmpty = try await client.call("channels.automation.finish", params: ["id": job.id, "lease": job.lease, "status": outcome.status, "reason": outcome.reason])
        } catch { self.error = error.localizedDescription }
    }
    func stop() { task?.cancel(); task = nil; client.stop() }
}

@MainActor enum SessionChannelAutomation {
    struct Outcome { var status: String, reason: String }
    static let terminals = ["com.mitchellh.ghostty", "com.googlecode.iterm2", "com.apple.Terminal"]
    static var installed: [String] { terminals.filter { NSWorkspace.shared.urlForApplication(withBundleIdentifier: $0) != nil } }
    static func authorize() async {
        for bundle in installed {
            _ = await Task.detached {
                let descriptor = NSAppleEventDescriptor(bundleIdentifier: bundle)
                return AEDeterminePermissionToAutomateTarget(descriptor.aeDesc, typeWildCard, typeWildCard, true)
            }.value
        }
    }
    static func unlocked() -> Bool {
        guard let info = CGSessionCopyCurrentDictionary() as? [String: Any],
              info[kCGSessionOnConsoleKey as String] as? Bool == true else { return false }
        return info["CGSSessionScreenIsLocked"] as? Bool != true
    }
    nonisolated static func foreground(pid: Int32, tty: String) -> Bool {
        guard validTTY(tty), pid > 0 else { return false }
        let descriptor = Darwin.open(tty, O_RDONLY | O_NOCTTY | O_NONBLOCK)
        guard descriptor >= 0 else { return false }
        defer { Darwin.close(descriptor) }
        let group = getpgid(pid)
        return group > 0 && tcgetpgrp(descriptor) == group
    }
    static func deliver(_ session: LiveSession, text: String, client: EngineClient) async -> Outcome {
        guard session.machineId == "local", let pid = session.pid, let start = session.processStart, start > 0 else { return .init(status: "blocked", reason: "缺少原终端的进程身份") }
        guard unlocked() else { return .init(status: "blocked", reason: "桌面已锁定，指令已保留") }
        guard [.complete, .interrupted, .failed, .needsInput].contains(session.phase) else { return .init(status: "blocked", reason: "原终端仍在运行，请等待本轮结束后重试") }
        guard let target: SessionFocus.Target = try? await client.call("sessions.focusTarget", params: ["pid": Int(pid), "processStart": start]), target.status == "found", let tty = target.tty,
              let app = SessionFocus.application(for: (target.ancestors ?? []).map(\.pid)), let bundle = app.bundleIdentifier,
              terminals.contains(bundle), foreground(pid: pid, tty: tty) else { return .init(status: "blocked", reason: "无法准确定位原终端，指令已保留") }
        let permission = await Task.detached {
            let descriptor = NSAppleEventDescriptor(bundleIdentifier: bundle)
            return AEDeterminePermissionToAutomateTarget(descriptor.aeDesc, typeWildCard, typeWildCard, false)
        }.value
        guard permission == noErr else { return .init(status: "blocked", reason: "请在聊天渠道设置中授权终端自动化，然后重试") }
        guard unlocked(), !app.isTerminated, foreground(pid: pid, tty: tty),
              let current: SessionFocus.Target = try? await client.call("sessions.focusTarget", params: ["pid": Int(pid), "processStart": start]), current.status == "found", current.tty == tty,
              current.ancestors?.contains(where: { $0.pid == app.processIdentifier }) == true else { return .init(status: "blocked", reason: "原终端身份已改变，指令已保留") }
        guard let script = script(bundle: bundle) else { return .init(status: "blocked", reason: "此终端尚未适配") }
        let result = await Task.detached {
            invoke(script: script, tty: tty, text: text)
        }.value
        switch result {
        case "submitted": return .init(status: "submitted", reason: "已输入原终端，等待会话确认")
        case "missing": return .init(status: "blocked", reason: "无法唯一定位原终端，指令已保留")
        default: return .init(status: "uncertain", reason: "终端输入结果无法确认，请核对原窗口；系统不会重发")
        }
    }
    nonisolated static func validTTY(_ tty: String) -> Bool {
        (tty.hasPrefix("/dev/tty") || tty.hasPrefix("/dev/pts/")) && tty.count <= 128 && tty.utf8.allSatisfy { (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [47,45,95].contains($0) }
    }
    /// Text is passed as an Apple event argument, never interpolated into AppleScript source.
    nonisolated private static func invoke(script: String, tty: String, text: String) -> String {
        guard validTTY(tty), text.utf8.count <= 8192, !text.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) && $0 != "\n" && $0 != "\t" }) else { return "missing" }
        let event = NSAppleEventDescriptor(eventClass: AEEventClass(kASAppleScriptSuite), eventID: AEEventID(kASSubroutineEvent), targetDescriptor: nil, returnID: AEReturnID(kAutoGenerateReturnID), transactionID: AETransactionID(kAnyTransactionID))
        event.setParam(NSAppleEventDescriptor(string: "deliver"), forKeyword: AEKeyword(keyASSubroutineName))
        let args = NSAppleEventDescriptor.list()
        args.insert(NSAppleEventDescriptor(string: tty), at: 1)
        args.insert(NSAppleEventDescriptor(string: text), at: 2)
        event.setParam(args, forKeyword: AEKeyword(keyDirectObject))
        var error: NSDictionary?
        let result = NSAppleScript(source: script)?.executeAppleEvent(event, error: &error)
        return error == nil ? result?.stringValue ?? "uncertain" : "uncertain"
    }
    nonisolated static func script(bundle: String) -> String? {
        let body: String
        switch bundle {
        case "com.mitchellh.ghostty":
            body = """
            set hits to {}
            repeat with candidate in terminals
                if tty of candidate is targetTTY then set end of hits to contents of candidate
            end repeat
            if (count hits) is not 1 then return "missing"
            set hitTerminal to item 1 of hits
            input text messageText to hitTerminal
            send key "enter" to hitTerminal
            """
        case "com.googlecode.iterm2":
            body = """
            set hits to {}
            repeat with w in windows
                repeat with t in tabs of w
                    repeat with s in sessions of t
                        if tty of s is targetTTY then set end of hits to contents of s
                    end repeat
                end repeat
            end repeat
            if (count hits) is not 1 then return "missing"
            set hitSession to item 1 of hits
            tell hitSession to write text messageText
            """
        case "com.apple.Terminal":
            body = """
            set hits to {}
            repeat with w in windows
                repeat with t in tabs of w
                    if tty of t is targetTTY then set end of hits to contents of t
                end repeat
            end repeat
            if (count hits) is not 1 then return "missing"
            do script messageText in item 1 of hits
            """
        default: return nil
        }
        return """
        on deliver(targetTTY, messageText)
            with timeout of 3 seconds
                if application id "\(bundle)" is not running then return "missing"
                tell application id "\(bundle)"
                    \(body)
                end tell
            end timeout
            return "submitted"
        end deliver
        """
    }
}
