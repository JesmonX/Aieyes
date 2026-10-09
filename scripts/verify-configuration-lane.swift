import Foundation

@main struct VerifyConfigurationLane {
    static func main() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("aieyes-config-lane-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let script = directory.appendingPathComponent("core.py")
        let program = """
        #!/usr/bin/env python3
        import json,sys,time,os
        for line in sys.stdin:
            request=json.loads(line)
            if request['method']=='fixture.block':
                open(os.path.join(os.path.dirname(__file__),'blocked'),'w').close()
                time.sleep(20)
            print(json.dumps({'id':request['id'],'result':{}}),flush=True)
        """
        try program.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions:0o700], ofItemAtPath:script.path)
        setenv("AIEYES_CORE_PATH", script.path, 1)
        let engine = EngineClient()
        defer { engine.stop() }
        let _: Acknowledgement = try await engine.call("settings.get")
        let blocked = Task { let _: Acknowledgement? = try? await engine.call("fixture.block") }
        while !FileManager.default.fileExists(atPath:directory.appendingPathComponent("blocked").path) { try await Task.sleep(for:.milliseconds(10)) }
        for method in ["agents.set", "settings.patch", "accounts.connect", "accounts.status.get"] {
            let start = Date()
            let _: Acknowledgement = try await engine.call(method)
            precondition(Date().timeIntervalSince(start) < 0.5, "Configuration queued behind a 20-second request: " + method)
        }
        engine.stop(); await blocked.value
        print("Configuration requests returned under 500 ms while the main channel was blocked by a 20-second operation")
    }
}
