import AppKit
import SwiftUI
import Combine
@testable import Aieyes

struct SyntheticServers: View {
    @ObservedObject var model: AppModel
    var body: some View {
        ScrollView {
            LazyVStack(spacing: 12) {
                ForEach(model.settings.hosts) { host in
                    ServerCard(host: host, result: model.hosts.first { $0.id == host.id }, compact: false).equatable()
                }
            }.padding(24)
        }
    }
}
@MainActor final class ServerPerformanceDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        Task { do { try await run(); NSApp.terminate(nil) } catch { fputs("\(error)\n", stderr); exit(1) } }
    }
    func verifyDeadlines() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("aieyes-deadlines-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        var settings = Settings(); settings.hosts = [Aieyes.Host(id: "fixture")]
        settings.serverRefreshSeconds = 10; settings.serverForegroundRefreshSeconds = 2
        try JSONEncoder().encode(settings).write(to: directory.appendingPathComponent("settings.json"))
        try JSONEncoder().encode(Dashboard()).write(to: directory.appendingPathComponent("dashboard.json"))
        let script = directory.appendingPathComponent("core.py")
        let program = """
        #!/usr/bin/env python3
        import json,sys,pathlib,time
        root=pathlib.Path(__file__).parent
        for line in sys.stdin:
            request=json.loads(line);method=request['method']
            with (root/'calls').open('a') as log:log.write(method+'\\n')
            if method=='settings.get':result=json.loads((root/'settings.json').read_text())
            elif method.startswith('dashboard'):result=json.loads((root/'dashboard.json').read_text())
            elif method=='quotas.schedule':result={'nextDueAt':None}
            elif method=='hosts.sample':result={'rows':[]}
            elif method=='network.test':result={'testedAt':time.time(),'mode':'direct','status':'ok','sites':[]}
            elif method=='prices.recalculate':time.sleep(2.4);result={}
            else:result=[]
            print(json.dumps({'id':request['id'],'result':result}),flush=True)
        """
        try program.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions:0o700], ofItemAtPath:script.path)
        let previous = ProcessInfo.processInfo.environment["AIEYES_CORE_PATH"]
        setenv("AIEYES_CORE_PATH", script.path, 1)
        defer { if let previous { setenv("AIEYES_CORE_PATH",previous,1) } else { unsetenv("AIEYES_CORE_PATH") } }
        let model = AppModel()
        defer { model.stop() }
        func count(_ method: String) -> Int { ((try? String(contentsOf: directory.appendingPathComponent("calls"), encoding: .utf8)) ?? "").split(separator: "\n").filter { $0 == method }.count }
        for _ in 0..<200 { if count("hosts.sample") == 1 { break }; try await Task.sleep(for: .milliseconds(25)) }
        precondition(count("hosts.sample") == 1)
        try await Task.sleep(for: .milliseconds(1000))
        precondition(count("hosts.sample") == 1, "Background sampling must retain its ten-second interval")
        model.setWindowVisible(true, window: "detail"); model.setServerVisible(true, window: "detail")
        for _ in 0..<100 { if count("hosts.sample") == 2 { break }; try await Task.sleep(for: .milliseconds(25)) }
        precondition(count("hosts.sample") == 2, "Foreground sampling must use its two-second deadline")
        model.installingUpdate = true
        try await Task.sleep(for: .milliseconds(2400))
        precondition(count("hosts.sample") == 2)
        model.installingUpdate = false
        for _ in 0..<100 { if count("hosts.sample") == 3 { break }; try await Task.sleep(for: .milliseconds(25)) }
        precondition(count("hosts.sample") == 3, "Resume catches up once instead of replaying missed ticks")
        let repricing = Task { await model.reprice() }
        for _ in 0..<100 { if model.repricing { break }; try await Task.sleep(for: .milliseconds(10)) }
        precondition(model.repricing)
        try await Task.sleep(for: .milliseconds(2100))
        precondition(count("hosts.sample") == 3, "Repricing pauses background scheduling")
        let repriced = await repricing.value
        precondition(repriced)
        for _ in 0..<100 { if count("hosts.sample") == 4 { break }; try await Task.sleep(for: .milliseconds(25)) }
        precondition(count("hosts.sample") == 4, "Completing repricing must restore the one-shot timer")
        precondition(count("quotas.refresh") == 0, "No empty quota polls for an accountless model")
        model.stop(); try await Task.sleep(for: .milliseconds(100))
        precondition(count("hosts.sample") == 4)
        print("Native background/foreground deadlines, update/reprice pause/resume and empty quota suppression passed")
    }
    func run() async throws {
        try await verifyDeadlines()
        if CommandLine.arguments.contains("--deadlines-only") { return }
        let root = URL(fileURLWithPath: CommandLine.arguments.dropFirst().first ?? ".local/performance-implementation/native-servers")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let suite = "aieyes.server-performance." + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let model = AppModel(autostart: false)
        defer { model.stop() }
        let view = NSHostingView(rootView: SyntheticServers(model: model).defaultAppStorage(defaults))
        let window = NSWindow(contentRect: NSRect(x: 40, y: 80, width: 1000, height: 740), styleMask: [.titled], backing: .buffered, defer: false)
        window.title = "Aieyes 服务器性能验证（合成数据）"; window.isReleasedWhenClosed = false; window.contentView = view; window.orderFront(nil)
        defer { window.close() }
        var report: [[String: Any]] = []
        for (count, cores) in [(10, 64), (30, 256)] {
            model.settings.hosts = (0..<count).map { index in
                var host = Aieyes.Host(id: "fixture-\(index)"); host.name = "Synthetic \(index)"; host.target = "synthetic.invalid"; host.metrics = ["cpu", "memory"]; host.details = []
                return host
            }
            let rows = model.settings.hosts.map { host in
                HostResult(id: host.id, name: host.name, sample: MetricSample(timestamp: Date().timeIntervalSince1970, uptime: nil, load: [1,2,3], errors: [:], cpu: (0...cores).map { DeviceMetric(id: $0 == 0 ? "cpu" : "cpu\($0-1)", name: nil, device: nil, type: nil, utilization: 20) }, memory: MemoryMetric(total: 32e9, available: 16e9, cached: nil, buffers: nil, swapTotal: nil, swapFree: nil)), error: nil, sampleSession: "fixture-\(count)", sampleVersion: 1)
            }
            var publications = 0
            let observer = model.$hosts.dropFirst().sink { _ in publications += 1 }
            model.acceptHostSamples(rows); model.acceptHostSamples(rows); model.acceptHostSamples(rows)
            precondition(publications == 1, "Identical progress/final rows must publish only once")
            try await Task.sleep(for: .milliseconds(150)); view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
            defaults.set(true, forKey: "server.expanded.detail.fixture-0")
            defaults.set(true, forKey: "server.metric.expanded.detail.fixture-0:cpu")
            try await Task.sleep(for: .milliseconds(150))
            var durations: [Double] = []
            for index in 2...31 {
                var next = rows; next[0].sampleVersion = UInt64(index); next[0].sample?.cpu?[0].utilization = Double(index)
                let start = ContinuousClock.now
                model.acceptHostSamples([next[0]])
                try await Task.sleep(for: .milliseconds(16))
                view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
                let duration = start.duration(to: .now)
                durations.append(Double(duration.components.seconds)*1000 + Double(duration.components.attoseconds)/1e15)
            }
            precondition(model.hosts.count == count)
            observer.cancel()
            view.layoutSubtreeIfNeeded()
            let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)!
            view.cacheDisplay(in: view.bounds, to: bitmap)
            try bitmap.representation(using: .png, properties: [:])!.write(to: root.appendingPathComponent("servers-\(count)-\(cores).png"))
            report.append(["hosts":count,"cores":cores,"oneHostUpdates":30,"publications":publications,"updateToDisplayMs":durations])
            defaults.set(false, forKey: "server.expanded.detail.fixture-0")
            model.acceptHostSamples([])
        }
        try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted,.sortedKeys]).write(to: root.appendingPathComponent("report.json"))
        print("Native server snapshots deduplicate; 10×64 and 30×256 lazy views updated 30 times each. Report: \(root.path)")
    }
}
@main struct VerifyServerPerformance {
    static func main() { let delegate = ServerPerformanceDelegate(); NSApplication.shared.delegate = delegate; NSApplication.shared.run(); withExtendedLifetime(delegate) {} }
}
