import Foundation
import Combine
@testable import Aieyes

@main struct VerifySessions {
    static func event(_ type: String) -> String { "{\"type\":\"event_msg\",\"payload\":{\"type\":\"\(type)\"}}\n" }
    static func item(_ type: String) -> String { "{\"type\":\"response_item\",\"payload\":{\"type\":\"\(type)\"}}\n" }
    @MainActor static func main() async throws {
        var played: [String] = []
        var choices: [(String, Float)] = []
        let soundService = SessionSoundService(player: { kind, tone, volume in played.append(kind); choices.append((tone, volume)) })
        var row = LiveSession(id: "sound-test", source: "fixture", phase: .complete, updatedAt: Date(timeIntervalSince1970: 100))
        row.turnId = "first"
        @MainActor func sounds(_ at: Double) { soundService.consume(SessionSnapshot(sessions: [row], unavailable: false), preferences: nil, now: Date(timeIntervalSince1970: at)) }
        sounds(100); precondition(played.isEmpty)
        row.phase = .working; row.updatedAt = Date(timeIntervalSince1970: 101); sounds(101)
        row.phase = .needsInput; row.updatedAt = Date(timeIntervalSince1970: 102); sounds(102); sounds(103)
        precondition(played == ["needsInput"])
        row.phase = .complete; row.updatedAt = Date(timeIntervalSince1970: 104); sounds(104)
        precondition(played == ["needsInput", "complete"])
        row.turnId = "second"; row.updatedAt = Date(timeIntervalSince1970: 105); sounds(105)
        precondition(played.count == 3)
        row.phase = .needsInput; row.updatedAt = Date(timeIntervalSince1970: 106)
        soundService.consume(SessionSnapshot(sessions: [row], unavailable: false), preferences: SessionSoundSettings(needsInput:false, complete:false), now: Date(timeIntervalSince1970:106))
        sounds(107); precondition(played.count == 3)
        precondition(choices.allSatisfy { $0.0 == "default" && $0.1 == 1 })
        let customSound = SessionSoundSettings(needsInputTone: "bell", completeTone: "marimba", volume: 0.35)
        row.phase = .complete; row.turnId = "custom"; row.updatedAt = Date(timeIntervalSince1970: 108)
        soundService.consume(SessionSnapshot(sessions: [row], unavailable: false), preferences: customSound, now: Date(timeIntervalSince1970: 108))
        precondition(choices.last!.0 == "marimba" && abs(choices.last!.1 - 0.35) < 0.001)
        row.phase = .needsInput; row.updatedAt = Date(timeIntervalSince1970: 109)
        soundService.consume(SessionSnapshot(sessions: [row], unavailable: false), preferences: customSound, now: Date(timeIntervalSince1970: 109))
        precondition(choices.last!.0 == "bell")
        let countBeforeMute = played.count
        row.phase = .complete; row.updatedAt = Date(timeIntervalSince1970: 110)
        soundService.consume(SessionSnapshot(sessions: [row], unavailable: false), preferences: SessionSoundSettings(volume: 0), now: Date(timeIntervalSince1970: 110))
        soundService.consume(SessionSnapshot(sessions: [row], unavailable: false), preferences: customSound, now: Date(timeIntervalSince1970: 111))
        precondition(played.count == countBeforeMute, "Unmuting must not replay a muted event")
        let oldSound = try JSONDecoder().decode(SessionSoundSettings.self, from: Data(#"{"needsInput":true,"complete":false}"#.utf8))
        precondition(oldSound.playbackVolume == 1 && oldSound.file(for: "needsInput") == "needs-input")
        precondition(SessionSoundSettings(completeTone: "unknown", volume: 2).file(for: "complete") == "complete")
        precondition(SessionSoundSettings(volume: -1).playbackVolume == 0)
        precondition(SessionSoundSettings(volume: .nan).playbackVolume == 1)
        print("Native session sounds: legacy defaults, independent tones, per-app volume and mute/resume passed")

        let start = "{\"type\":\"session_meta\",\"payload\":{\"id\":\"fixture\",\"source\":\"cli\"}}\n" + event("task_started")
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
        precondition(missing.sources?.first?.sourceId == "missing" && missing.sources?.first?.error != nil)
        try Data(start.utf8).write(to: file)
        try await save([AgentSource(id: "source", name: "Test", path: file.path), settings.sources[0]])
        let partial: SessionSnapshot = try await engine.call("sessions.list")
        precondition(partial.unavailable && partial.sessions.first?.phase == .working)
        precondition(partial.sources?.filter { $0.state != "healthy" }.count == 1)
        let parentFile = root.appendingPathComponent("parent.jsonl")
        let childFile = root.appendingPathComponent("child.jsonl")
        let parentStart = #"{"type":"session_meta","payload":{"id":"parent","source":"cli"}}"# + "\n" + event("task_started")
        let childStart = #"{"type":"session_meta","payload":{"id":"child","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}}}"# + "\n" + event("task_started")
        let childWait = #"{"type":"response_item","payload":{"type":"function_call","name":"request_user_input","call_id":"question"}}"# + "\n"
        try Data(parentStart.utf8).write(to: parentFile)
        try Data((childStart + childWait).utf8).write(to: childFile)
        try await save([AgentSource(id: "parent-source", name: "Parent", path: parentFile.path), AgentSource(id: "child-source", name: "Child", path: childFile.path)])
        let childWaiting: SessionSnapshot = try await engine.call("sessions.list")
        precondition(childWaiting.sessions.count == 1 && childWaiting.sessions.first?.sourceId == "parent-source")
        precondition(childWaiting.sessions.first?.phase == .needsInput && childWaiting.pet?.behavior == "attention")
        try Data((childStart + childWait + event("error")).utf8).write(to: childFile)
        let childFailed: SessionSnapshot = try await engine.call("sessions.list")
        precondition(childFailed.sessions.count == 1 && childFailed.sessions.first?.phase == .working)
        precondition(childFailed.pet?.behavior == "running", "A failed child must not fail its running parent")
        print("Native session decoding and real-core lifecycle, completion, disabled and per-source failure RPC checks passed")
        print("Real-core child waiting, parent-only list/count and pet state aggregation passed")
        engine.stop()
        try await verifySurfaceProjections()
        try await verifyIndependentPolling(root.appendingPathComponent("polling"))
    }

    @MainActor static func verifySurfaceProjections() async throws {
        let model = AppModel(autostart:false); defer { model.stop() }
        model.detailPage = "analysis"; model.setWindowVisible(true,window:"detail")
        // Test projections without a delayed dashboard request changing the fixture.
        model.stop()
        func flushPublications() async {
            await withCheckedContinuation { continuation in
                DispatchQueue.main.async { continuation.resume(returning: ()) }
            }
        }
        let navigation = WindowNavigationStore(model:model)
        let surface = SurfaceObservation(model:model,compact:false,page:"analysis")
        var navigationChanges = 0, surfaceChanges = 0
        let nav = navigation.objectWillChange.sink { navigationChanges += 1 }
        let page = surface.objectWillChange.sink { surfaceChanges += 1 }
        model.message = "unrelated metric publication"
        await flushPublications()
        precondition(navigationChanges == 0 && surfaceChanges == 0,"Status messages must not invalidate page content")
        model.dashboard.summary.total = 1
        await flushPublications()
        precondition(navigationChanges == 0 && surfaceChanges == 1,"Visible dashboard changes must publish")
        model.setWindowVisible(false,window:"detail")
        await flushPublications()
        let hidden = surfaceChanges
        precondition(navigationChanges > 0 && hidden > 1,"Hiding must publish once so child timelines stop")
        for i in 0..<10 { model.message = "hidden metric \(i)" }
        model.dashboard.summary.total = 2
        await flushPublications()
        precondition(surfaceChanges == hidden,"Hidden surfaces defer unrelated publications")
        model.setWindowVisible(true,window:"detail")
        await flushPublications()
        precondition(surfaceChanges > hidden,"Showing publishes the latest retained state")
        nav.cancel();page.cancel()
        print("Navigation projections, hidden-page transition, deferred updates and show-time resumption passed")
    }
    @MainActor static func verifyIndependentPolling(_ root: URL) async throws {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        var settings = Settings()
        settings.sources = []
        settings.accounts = [AgentAccount(id: "fixture", name: "Fixture", provider: "deepseek", quotaEnabled: true)]
        try JSONEncoder().encode(settings).write(to: root.appendingPathComponent("settings.json"))
        try JSONEncoder().encode(Dashboard()).write(to: root.appendingPathComponent("dashboard.json"))
        try "working".write(to: root.appendingPathComponent("phase"), atomically: true, encoding: .utf8)
        let script = root.appendingPathComponent("core.py")
        try #"""
#!/usr/bin/env python3
import json,pathlib,sys,time
root=pathlib.Path(__file__).parent
states={
    'working':('running','进行中',1,8),
    'tool':('working','执行工具',7,6),
    'thinking':('thinking','思考中',8,6),
    'needsInput':('attention','等待输入',3,4),
    'complete':('success','已完成',4,5),
    'interrupted':('rest','已中断',2,8),
}
for line in sys.stdin:
    request=json.loads(line);method=request['method']
    with (root/'calls').open('a') as log:log.write(method+'\n')
    if (root/'blocked').exists() and (root/'blocked').read_text()==method:
        (root/'started').touch()
        deadline=time.monotonic()+30
        while not (root/'release').exists():
            if time.monotonic()>deadline:raise RuntimeError('test request was not released')
            time.sleep(.01)
        (root/'finished').touch()
    if method=='hello':result={}
    elif method=='settings.get':result=json.loads((root/'settings.json').read_text())
    elif method=='settings.patch':
        result=request['params']['settings']
        (root/'settings.json').write_text(json.dumps(result))
    elif method.startswith('dashboard'):result=json.loads((root/'dashboard.json').read_text())
    elif method=='sessions.list':
        phase=(root/'phase').read_text();behavior,label,row,frames=states[phase]
        result={'sessions':[{'id':'fixture','source':'Codex','phase':phase,'updatedAt':time.time()}],
            'unavailable':False,'summary':label,
            'pet':{'key':phase,'behavior':behavior,'label':label,'row':row,'frames':frames,'frameMs':100}}
    elif method=='pets.current':result={'pet':None,'error':None}
    elif method=='quotas.schedule':result={'nextDueAt':None}
    elif method=='autoEstimates.tick':result={'changed':False}
    elif method=='prices.recalculate':result={}
    else:result=[]
    print(json.dumps({'id':request['id'],'result':result}),flush=True)
"""#.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
        let previous = ProcessInfo.processInfo.environment["AIEYES_CORE_PATH"]
        setenv("AIEYES_CORE_PATH", script.path, 1)
        defer { if let previous { setenv("AIEYES_CORE_PATH", previous, 1) } else { unsetenv("AIEYES_CORE_PATH") } }
        func count(_ method: String) -> Int {
            ((try? String(contentsOf: root.appendingPathComponent("calls"), encoding: .utf8)) ?? "").split(separator: "\n").filter { $0 == method }.count
        }
        func waitFor(_ message: String, _ condition: () -> Bool) async throws {
            let deadline = ContinuousClock.now.advanced(by: .seconds(5))
            while !condition() {
                guard ContinuousClock.now < deadline else { throw ClientError.message(message) }
                try await Task.sleep(for: .milliseconds(20))
            }
        }
        func block(_ method: String) throws {
            for name in ["started", "finished", "release"] { try? FileManager.default.removeItem(at: root.appendingPathComponent(name)) }
            try method.write(to: root.appendingPathComponent("blocked"), atomically: true, encoding: .utf8)
        }
        func release() throws { try Data().write(to: root.appendingPathComponent("release")) }
        func exists(_ name: String) -> Bool { FileManager.default.fileExists(atPath: root.appendingPathComponent(name).path) }

        try block("dashboard")
        let model = AppModel()
        model.detailPage = "analysis"; model.setWindowVisible(true, window: "detail")
        defer { model.stop() }
        try await waitFor("Initial history query did not start") { exists("started") }
        try await waitFor("Live state waited for the initial history query") { model.petState.behavior == "running" }
        precondition(model.dashboardPending && !exists("finished"))
        try release()
        try await waitFor("Bootstrap did not finish") { count("quotas.refresh") > 0 && !model.busy && !model.quotaBusy && !model.dashboardPending }

        for (method, phase, label) in [("sources.scan", "tool", "执行工具"), ("quotas.refresh", "thinking", "思考中"), ("prices.recalculate", "needsInput", "等待输入"), ("settings.patch", "complete", "已完成")] {
            try block(method)
            let work = Task {
                switch method {
                case "sources.scan": await model.scan()
                case "quotas.refresh": await model.refreshQuotas()
                case "prices.recalculate": let saved = await model.reprice(); precondition(saved)
                default:
                    model.settingsDraft = model.settings
                    let saved = await model.saveSettingsDraft(refreshDashboard: false); precondition(saved)
                }
            }
            try await waitFor("Request did not start: " + method) { exists("started") }
            try phase.write(to: root.appendingPathComponent("phase"), atomically: true, encoding: .utf8)
            try await waitFor("Live state stalled behind " + method) { model.sessions.first?.phase.rawValue == label && model.petState.label == label }
            precondition(!exists("finished"), "The unrelated request must still be blocked when the live state changes")
            try release(); await work.value
        }

        // Updates must pause new polls and discard an already-running read; resume and stop stay bounded.
        let beforeUpdate = model.petState
        try block("sessions.list")
        try "interrupted".write(to: root.appendingPathComponent("phase"), atomically: true, encoding: .utf8)
        try await waitFor("Live read did not start") { exists("started") }
        model.installingUpdate = true
        try release()
        try await waitFor("Live read did not finish") { exists("finished") }
        let paused = count("sessions.list")
        try await Task.sleep(for: .milliseconds(1200))
        precondition(count("sessions.list") == paused && model.petState == beforeUpdate)
        model.installingUpdate = false
        try await waitFor("Live polling did not resume after update preparation") { model.petState.behavior == "rest" }
        model.stop()
        let stopped = count("sessions.list")
        try await Task.sleep(for: .milliseconds(1200))
        precondition(count("sessions.list") == stopped)
        print("Independent live polling passed during startup history, scans, quota queries, repricing and settings saves; update pause/resume and shutdown passed")
    }
}
