import AppKit
import SwiftUI
import Combine
import ApplicationServices
@testable import Aieyes

@MainActor final class OptimizationChecks: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        AXUIElementSetAttributeValue(AXUIElementCreateApplication(getpid()), "AXManualAccessibility" as CFString, kCFBooleanTrue)
        Task { do { try await run(); print("CPU optimization state/IPC regressions passed"); NSApp.terminate(nil) }
            catch { fputs("CPU regression failed: \(error)\n", stderr); exit(1) } }
    }
    func wait(_ ms: Int = 50) async throws { try await Task.sleep(for: .milliseconds(ms)) }
    func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    func control(in root: NSObject, containing text: String) -> NSObject? {
        var visited = Set<ObjectIdentifier>()
        func find(_ node: NSObject, depth: Int) -> NSObject? {
            guard depth < 40, visited.insert(ObjectIdentifier(node)).inserted else { return nil }
            func value(_ name: String) -> AnyObject? {
                let selector = NSSelectorFromString(name)
                return node.responds(to: selector) ? node.perform(selector)?.takeUnretainedValue() : nil
            }
            let label = (value("accessibilityLabel") as? String) ?? (value("accessibilityTitle") as? String) ?? ""
            if label.contains(text), value("accessibilityRole") as? String == "AXButton" { return node }
            let children = ((value("accessibilityChildren") as? [NSObject]) ?? []) + ((node as? NSView)?.subviews ?? [])
            for child in children { if let found = find(child, depth: depth + 1) { return found } }
            return nil
        }
        return find(root, depth: 0)
    }
    func press(in root: NSObject, containing text: String) -> Bool {
        let selector = NSSelectorFromString("accessibilityPerformPress")
        guard let control = control(in: root, containing: text), control.responds(to: selector), let implementation = control.method(for: selector) else { return false }
        typealias Action = @convention(c) (AnyObject, Selector) -> Bool
        return unsafeBitCast(implementation, to: Action.self)(control, selector)
    }
    func run() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("aieyes-cpu-check-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let suite = "aieyes.scroll-check." + UUID().uuidString, defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = ScrollPositionStore(defaults: defaults, delay: 0.03, observeLifecycle: false)
        defaults.set([0.0, 42.0], forKey: "view.scroll.v2.old")
        precondition(store.position(for: "old")?.y == 42)
        for y in 1...60 { store.record(NSPoint(x: 0, y: y), for: "old") }
        precondition(store.writes == 0 && store.position(for: "old")?.y == 60)
        try await wait(70)
        precondition(store.writes == 1 && defaults.array(forKey: "view.scroll.v2.old") as? [Double] == [0,60])
        store.record(NSPoint(x: 0,y: 80), for: "old"); store.flush("old")
        store.record(NSPoint(x: 0,y: 90), for: "old")
        try await wait(70)
        precondition(store.writes == 3 && store.position(for: "old")?.y == 90, "An old delayed task must not overwrite the newest position")
        store.record(NSPoint(x: 0,y: 10), for: "panel"); store.flushAll()
        precondition(store.position(for: "old")?.y == 90 && store.position(for: "panel")?.y == 10)

        let window = NSWindow(contentRect: NSRect(x: 80,y: 80,width: 760,height: 600), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let scroll = NSScrollView(frame: NSRect(x: 0,y: 0,width: 500,height: 300))
        let document = NSView(frame: NSRect(x: 0,y: 0,width: 500,height: 2000));scroll.documentView = document
        window.contentView = scroll; window.orderFrontRegardless(); try await wait()
        let coordinator = ScrollMemory.Coordinator()
        coordinator.attach(scroll, key: "check.native", ready: true)
        scroll.contentView.scroll(to: NSPoint(x: 0,y: 240)); scroll.reflectScrolledClipView(scroll.contentView)
        try await wait(10);coordinator.setActive(false)
        precondition(ScrollPositionStore.shared.position(for: "check.native")?.y == 240)
        scroll.contentView.scroll(to: .zero);try await wait(10)
        precondition(ScrollPositionStore.shared.position(for: "check.native")?.y == 240, "Hidden layout must not overwrite position")
        coordinator.setActive(true);coordinator.attach(scroll, key: "check.native", ready: true)
        precondition(scroll.contentView.bounds.minY == 240)
        coordinator.detach()
        let recreated = ScrollMemory.Coordinator();scroll.contentView.scroll(to: .zero)
        recreated.attach(scroll, key: "check.native", ready: false)
        precondition(scroll.contentView.bounds.minY == 0)
        recreated.attach(scroll, key: "check.native", ready: true)
        precondition(scroll.contentView.bounds.minY == 240)
        recreated.detach();document.setFrameSize(NSSize(width: 500,height: 200))
        let shortened = ScrollMemory.Coordinator();shortened.attach(scroll, key: "check.native", ready: true)
        precondition(scroll.contentView.bounds.minY == 0, "Restore must clamp to shorter content")
        shortened.detach()
        ScrollPositionStore.shared.record(NSPoint(x: 0,y: 123), for: "close.flush")
        window.close()
        precondition(ScrollPositionStore.shared.pendingCount == 0, "Window close must flush pending positions")

        var content = Dashboard(); content.generatedAt = 1
        var newer = content; newer.generatedAt = 2
        precondition(content.hasSameContent(as: newer), "Query timestamps alone must not republish the payload")
        newer.summary.total = 1
        precondition(!content.hasSameContent(as: newer))
        newer = content; newer.modelDisplayNames = ["fixture":"renamed"]
        precondition(!content.hasSameContent(as: newer))
        newer = content; newer.sources = [SourceSummary(id:"fixture",name:"fixture",provider:"codex",enabled:true,status:nil,accountIds:nil)]
        content = newer; newer.sources[0].status = SourceStatus(updatedAt:2)
        precondition(!content.hasSameContent(as: newer), "Visible source timestamps remain observable")
        precondition(!Dashboard().hasSameContent(as: Dashboard(generatedAt:1)), "Readiness must still publish")

        let model = AppModel(autostart: false);defer { model.stop() }
        var quota = try JSONDecoder().decode(Quota.self, from: Data(#"{"sourceId":"fixture","accountId":"fixture","provider":"codex","name":"Fixture quota","updatedAt":1,"origin":"fixture","windows":[{"name":"5h","usedPercent":25,"windowMinutes":300}]}"#.utf8))
        var switches = 0
        func card() -> some View { QuotaCard(quota: quota, compact: true, collapsible: true, onSwitch: { switches += 1 }, actionOwner: ObjectIdentifier(model)).equatable() }
        let quotaWindow = NSWindow(contentRect:NSRect(x:100,y:100,width:450,height:650),styleMask:[.titled,.closable],backing:.buffered,defer:false)
        quotaWindow.isReleasedWhenClosed = false;defer { quotaWindow.close() }
        let quotaHost = NSHostingView(rootView: AnyView(card()));quotaHost.frame = NSRect(x:0,y:0,width:450,height:650)
        quotaWindow.contentView = quotaHost;quotaWindow.orderFrontRegardless()
        try await wait(100);quotaHost.layoutSubtreeIfNeeded();quotaHost.displayIfNeeded()
        precondition(press(in: quotaHost, containing: "展开额度"));try await wait(300)
        let expansionKey = NativePanelKey.expansion(.quota, id: quota.id, compact: true)
        precondition(UserDefaults.standard.bool(forKey: expansionKey))
        quota.updatedAt += 1;quota.windows[0].usedPercent = 35;quotaHost.rootView = AnyView(card())
        try await wait(100);quotaHost.layoutSubtreeIfNeeded()
        precondition(UserDefaults.standard.bool(forKey: expansionKey), "Data refresh must retain disclosure state")
        precondition(press(in: quotaHost, containing: "切换为此账户") && switches == 1, "Equatable cards must retain working actions")
        precondition(press(in: quotaHost, containing: "折叠额度"));try await wait(300)
        precondition(!UserDefaults.standard.bool(forKey: expansionKey))
        let projection = model.sessionStripStatus
        var sessionUpdates = 0
        let projectionObserver = projection.objectWillChange.sink { sessionUpdates += 1 };defer { projectionObserver.cancel() }
        for i in 0..<10 { model.live.petVisible = i % 2 == 0;model.petState = .idle }
        try await wait();precondition(sessionUpdates == 0)
        model.sessions = [LiveSession(id: "fixture",source: "fixture",phase: .working,updatedAt: Date())]
        try await wait();precondition(sessionUpdates == 1 && projection.snapshot.sessions.count == 1)
        UserDefaults.standard.set(true,forKey: "aieyes.pet.enabled");UserDefaults.standard.set(1.0,forKey: "aieyes.pet.scale")
        let pet = PetWindowController(model: model) { _,_ in };defer { pet.window.orderOut(nil) }
        try await wait(100)
        var visibilityUpdates = 0
        let visibleObserver = model.live.$petVisible.dropFirst().sink { _ in visibilityUpdates += 1 };defer { visibleObserver.cancel() }
        let petOrigin = pet.window.frame.origin
        for i in 0..<30 { UserDefaults.standard.set(i,forKey: "view.scroll.v2.unrelated") }
        NotificationCenter.default.post(name: UserDefaults.didChangeNotification,object: nil)
        try await wait(100)
        precondition(visibilityUpdates == 0 && pet.window.frame.origin == petOrigin)
        UserDefaults.standard.set(false,forKey: "aieyes.pet.enabled");NotificationCenter.default.post(name: UserDefaults.didChangeNotification,object: nil)
        try await wait();precondition(!pet.window.isVisible && !model.live.petVisible)
        UserDefaults.standard.set(1.25,forKey: "aieyes.pet.scale");UserDefaults.standard.set(true,forKey: "aieyes.pet.enabled")
        NotificationCenter.default.post(name: UserDefaults.didChangeNotification,object: nil)
        try await wait();precondition(pet.window.isVisible && pet.window.frame.width == 180)

        // Synthetic local RPCs: no credentials, histories or external operations.
        let seed = root.appendingPathComponent("fixture.json"), control = root.appendingPathComponent("control.json")
        var seedSettings = Settings()
        seedSettings.accounts = [try JSONDecoder().decode(AgentAccount.self, from: Data(#"{"id":"fixture","name":"Fixture quota","provider":"codex","quotaEnabled":true}"#.utf8))]
        quota.credits = CreditsBalance(hasCredits:true,unlimited:false,balance:"1000")
        var seedDashboard = Dashboard(generatedAt: 1);seedDashboard.quotas = [quota]
        let fixture: [String: Any] = ["dashboard": try JSONSerialization.jsonObject(with: JSONEncoder().encode(seedDashboard)), "settings": try JSONSerialization.jsonObject(with: JSONEncoder().encode(seedSettings))]
        try JSONSerialization.data(withJSONObject: fixture).write(to: seed)
        func configure(_ revision: Int, fail: Bool = false, extra: Int = 0, delay: Double = 0) throws {
            try JSONSerialization.data(withJSONObject: ["revision":revision,"fail":fail,"extra":extra,"delay":delay]).write(to: control,options:.atomic)
        }
        try configure(1)
        let script = root.appendingPathComponent("core.py")
        try #"""
        #!/usr/bin/env python3
        import json,sys,time,pathlib
        root=pathlib.Path(__file__).parent
        seed=json.loads((root/'fixture.json').read_text())
        for line in sys.stdin:
            r=json.loads(line);m=r['method'];p=r.get('params',{});c=json.loads((root/'control.json').read_text())
            with (root/'calls').open('a') as f:f.write(json.dumps({'method':m,'days':p.get('days')})+'\n')
            result={}
            if m=='runtime.revisions':
                time.sleep(c['delay']);result={'usage':c['revision'],'quotas':1,'settings':1,'prices':1,'sources':1,'estimates':1}
            elif m=='settings.get':result=seed['settings']
            elif m in ['dashboard','dashboard.summary']:
                if c['fail']:
                    print(json.dumps({'id':r['id'],'error':{'code':-1,'message':'fixture failure'}}),flush=True);continue
                result=json.loads(json.dumps(seed['dashboard']));result['generatedAt']=time.time();result['summary']['total']=p.get('days',1)+c['extra']
                if p.get('days')==7:time.sleep(.2)
            elif m=='dashboard.status':result={k:seed['dashboard'][k] for k in ['quotas','sources','quotaOrder','quotaEstimates','creditEstimates','automaticEstimateSummaries'] if k in seed['dashboard']}
            elif m=='hello':result={}
            else:result=[]
            print(json.dumps({'id':r['id'],'result':result}),flush=True)
        """#.write(to: script,atomically:true,encoding:.utf8)
        try FileManager.default.setAttributes([.posixPermissions:0o700],ofItemAtPath:script.path)
        setenv("AIEYES_CORE_PATH",script.path,1)
        let ipc = AppModel(autostart: false);defer { ipc.stop();unsetenv("AIEYES_CORE_PATH") }
        ipc.settings = seedSettings;ipc.settingsLoaded = true;ipc.settingsDraft = ipc.settings
        let initiallyLoaded = await ipc.reload();precondition(initiallyLoaded)
        ipc.setWindowVisible(true, window: "panel")
        quotaWindow.setContentSize(NSSize(width: 450, height: 720))
        let panelHost = NSHostingView(rootView: RootView(model: ipc))
        quotaWindow.contentView = panelHost;try await wait(200);panelHost.layoutSubtreeIfNeeded()
        ipc.quotaBusy = true;try await wait();panelHost.layoutSubtreeIfNeeded()
        precondition(self.control(in: panelHost, containing: "读取中…") != nil, "Local quota controls must still show busy feedback")
        ipc.quotaBusy = false;ipc.networkBusy = true;try await wait();panelHost.layoutSubtreeIfNeeded()
        precondition(self.control(in: panelHost, containing: "刷新额度") != nil && self.control(in: panelHost, containing: "测试中…") != nil, "Footer and quota feedback must update independently")
        let credit = try JSONDecoder().decode(QuotaEstimate.self, from: Data(#"{"id":"credit","accountKey":"codex:fixture","windowId":"credits","windowName":"credits","sourceIds":[],"sourceNames":[],"status":"completed","reason":"","startedAt":1,"checkpointAt":2,"cost":1,"totalTokens":1,"pricedTokens":1,"calculationNote":"","kind":"credits","consumedCredits":1,"valuePer1000":42}"#.utf8))
        ipc.dashboard.creditEstimates = [credit];try await wait(200);panelHost.layoutSubtreeIfNeeded()
        precondition(ipc.dashboard.creditEstimates?.first?.id == "credit")
        precondition(self.control(in: panelHost, containing: "42.00 USD") != nil, "An estimate-only change must reach an otherwise unchanged quota card")
        ipc.dashboard.creditEstimates = nil;try await wait()
        ipc.networkBusy = false;ipc.setWindowVisible(false, window: "panel")
        let indicatorHost = NSHostingView(rootView: RefreshIndicator(model: ipc, key: "scan"))
        quotaWindow.contentView = indicatorHost;try await wait();indicatorHost.layoutSubtreeIfNeeded()
        let indicatorSize = indicatorHost.fittingSize
        try configure(1, delay: 0.15)
        let loading = Task { await ipc.reload() };try await wait(60);indicatorHost.layoutSubtreeIfNeeded()
        precondition(ipc.dashboardPending)
        precondition(indicatorHost.fittingSize == indicatorSize, "Loading must retain the indicator geometry")
        _ = await loading.value;try await wait();indicatorHost.layoutSubtreeIfNeeded()
        precondition(indicatorHost.fittingSize == indicatorSize)
        try configure(1)
        var dataUpdates = 0
        let dataObservers = [ipc.$dashboard.dropFirst().sink { _ in dataUpdates += 1 },ipc.$menuDashboard.dropFirst().sink { _ in dataUpdates += 1 },ipc.$panelDashboard.dropFirst().sink { _ in dataUpdates += 1 },ipc.$overviewDashboard.dropFirst().sink { _ in dataUpdates += 1 }]
        defer { dataObservers.forEach { $0.cancel() } }
        let unchangedLoaded = await ipc.reload();precondition(unchangedLoaded);precondition(dataUpdates == 0, "Unchanged reload must not publish data snapshots")
        ipc.detailPage = "analysis";ipc.setWindowVisible(true,window:"detail")
        _ = await ipc.reload();try await wait(200)
        let surface = SurfaceObservation(model:ipc,compact:false,page:"analysis")
        var pageUpdates = 0, progressUpdates = 0
        let pageObserver = surface.objectWillChange.sink { pageUpdates += 1 }
        let progressObserver = ipc.$dashboardPending.dropFirst().sink { _ in progressUpdates += 1 }
        defer { pageObserver.cancel();progressObserver.cancel() }
        _ = await ipc.reload();try await wait()
        precondition(pageUpdates == 0 && progressUpdates >= 2, "Progress feedback must update without invalidating unchanged page content")
        ipc.quotaBusy = true;try await wait();precondition(pageUpdates == 0, "Busy feedback must remain local to controls")
        ipc.quotaBusy = false;try await wait()
        ipc.range = 7
        let old = Task { await ipc.reload() }
        for _ in 0..<100 {
            if ((try? String(contentsOf: root.appendingPathComponent("calls"),encoding:.utf8)) ?? "").contains("\"days\": 7") { break }
            try await wait(5)
        }
        ipc.range = 30
        let latest = Task { await ipc.reload() }
        _ = await old.value;_ = await latest.value
        precondition(ipc.dashboard.summary.total == 30 && !ipc.dashboardPending, "Old replies must not replace latest filters")
        try configure(2,fail:true)
        let failedReload = await ipc.reload();precondition(!failedReload && ipc.dashboardError != nil && !ipc.dashboardPending)
        try configure(3,extra:10)
        let recoveredReload = await ipc.reload();precondition(recoveredReload);precondition(ipc.dashboardError == nil && ipc.dashboard.summary.total == 40)
        ipc.detailPage = "settings";ipc.settingsTab = "general";ipc.generalPage = "network"
        ipc.settingsDraft.proxy.mode = "custom";ipc.settingsDraft.proxy.url = "http://127.0.0.1:9999"
        let host = NSHostingView(rootView: MainWindowView(model: ipc));window.contentView = host;window.orderFrontRegardless()
        try await wait(150);host.layoutSubtreeIfNeeded()
        let field = descendants(host).compactMap { $0 as? NSTextField }.first { $0.stringValue == "127.0.0.1" }
        guard let field else { throw ClientError.message("Expected native network editor") }
        precondition(window.makeFirstResponder(field));let focused = window.firstResponder
        _ = await ipc.reload();try await wait()
        precondition(window.firstResponder === focused && ipc.settingsDirty)
        ipc.detailPage = "analysis";try await wait();ipc.detailPage = "settings";try await wait()
        precondition(ipc.settingsDraft.proxy.url == "http://127.0.0.1:9999", "Page switches must retain unsaved input")
    }
}
@main struct VerifyOptimization {
    static func main() { let d=OptimizationChecks();NSApplication.shared.delegate=d;NSApplication.shared.run();withExtendedLifetime(d){} }
}
