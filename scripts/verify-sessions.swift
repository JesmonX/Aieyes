import Foundation

@main struct VerifySessions {
    static func event(_ type: String) -> String { "{\"type\":\"event_msg\",\"payload\":{\"type\":\"\(type)\"}}\n" }
    static func item(_ type: String) -> String { "{\"type\":\"response_item\",\"payload\":{\"type\":\"\(type)\"}}\n" }
    static func main() async throws {
        let start = event("task_started")
        precondition(LiveSession.phase(in: Data(start.utf8)) == .working)
        precondition(LiveSession.phase(in: Data((start + item("reasoning")).utf8)) == .thinking)
        precondition(LiveSession.phase(in: Data((start + item("function_call")).utf8)) == .tool)
        precondition(LiveSession.phase(in: Data((start + item("custom_tool_call") + item("custom_tool_call_output")).utf8)) == .working)
        precondition(LiveSession.phase(in: Data((start + event("task_complete") + item("reasoning")).utf8)) == .complete)
        precondition(LiveSession.phase(in: Data((start + event("turn_aborted")).utf8)) == .interrupted)
        precondition(LiveSession.phase(in: Data((start + event("task_complete") + start).utf8)) == .working)
        precondition(LiveSession.phase(in: Data("broken\n{}\n".utf8)) == nil)
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("session.jsonl")
        try Data(start.utf8).write(to: file)
        let source = AgentSource(name: "Test", path: file.path)
        let monitor = SessionMonitor()
        let fresh = await monitor.read(sources: [source])
        precondition(fresh.sessions.count == 1 && fresh.sessions[0].phase == .working)
        let duplicate = await monitor.read(sources: [source, source])
        precondition(duplicate.sessions.count == 1)
        let stale = await monitor.read(sources: [source], now: Date().addingTimeInterval(190))
        precondition(stale.sessions.first?.phase == .unknown)
        try Data((start + event("task_complete")).utf8).write(to: file)
        let completed = await monitor.read(sources: [source])
        precondition(completed.sessions.first?.phase == .complete)
        let disabled = await monitor.read(sources: [AgentSource(name: "Disabled", path: file.path, enabled: false)])
        precondition(disabled.sessions.isEmpty)
        let remote = await monitor.read(sources: [AgentSource(name: "Remote", path: file.path, hostId: "remote")])
        precondition(remote.sessions.isEmpty)
        let missingSource = AgentSource(name: "Missing", path: root.appendingPathComponent("missing.jsonl").path)
        let missing = await monitor.read(sources: [missingSource])
        precondition(missing.unavailable && missing.sessions.isEmpty)
        print("Session lifecycle, stale state, completion, disabled and remote source checks passed")
    }
}
