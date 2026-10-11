import Foundation

@main struct VerifySessions {
    static func event(_ type: String) -> String { "{\"type\":\"event_msg\",\"payload\":{\"type\":\"\(type)\"}}\n" }
    static func item(_ type: String) -> String { "{\"type\":\"response_item\",\"payload\":{\"type\":\"\(type)\"}}\n" }
    static func main() async throws {
        let start = event("task_started")
        let snapshot = try JSONDecoder().decode(SessionSnapshot.self, from: Data(#"{"sessions":[{"id":"s","source":"Codex","phase":"tool","updatedAt":1791640000}],"unavailable":false}"#.utf8))
        precondition(snapshot.sessions[0].phase == .tool)
        precondition(snapshot.sessions[0].updatedAt.timeIntervalSince1970 == 1791640000)
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("session.jsonl")
        try Data(start.utf8).write(to: file)
        // Exercise the production Rust monitor via a persistent, isolated RPC lane.
        setenv("AIEYES_DATA_DIR", root.appendingPathComponent("store").path, 1)
        let engine = EngineClient(configurationOnly: true)
        defer { engine.stop() }
        var settings = Settings()
        settings.sources = [AgentSource(id: "source", name: "Test", path: file.path)]
        func save(_ sources: [AgentSource]) async throws {
            settings.sources = sources
            let params = try JSONSerialization.jsonObject(with: JSONEncoder().encode(settings)) as! [String: Any]
            let _: Acknowledgement = try await engine.call("settings.save", params: params)
        }
        try await save(settings.sources)
        let fresh: SessionSnapshot = try await engine.call("sessions.list")
        precondition(fresh.sessions.count == 1 && fresh.sessions[0].phase == .working)
        try Data((start + event("task_complete")).utf8).write(to: file)
        let completed: SessionSnapshot = try await engine.call("sessions.list")
        precondition(completed.sessions.first?.phase == .complete)
        try await save([AgentSource(id: "source", name: "Disabled", path: file.path, enabled: false)])
        let disabled: SessionSnapshot = try await engine.call("sessions.list")
        precondition(disabled.sessions.isEmpty)
        try await save([AgentSource(id: "missing", name: "Missing", path: root.appendingPathComponent("missing.jsonl").path)])
        let missing: SessionSnapshot = try await engine.call("sessions.list")
        precondition(missing.unavailable && missing.sessions.isEmpty)
        print("Native session decoding and real-core lifecycle, completion, disabled and unavailable source RPC checks passed")
    }
}
