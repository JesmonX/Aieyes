// Optimized diagnostic driver, compiled together with unmodified production views.
import AppKit
import SwiftUI
import Combine
import Darwin
import ObjectiveC
#if AIEYES_PERFORMANCE_MODULE
@testable import Aieyes
#endif

// A process-local causal control: suppress only scroll-position persistence.
// Page, account, expansion and other production preferences keep normal semantics.
private enum ScrollPersistenceControl {
    static var enabled=false
    static var suppressed=0
    static var writes=0
    static let lock=NSLock()
    static func install() {
        guard let original=class_getInstanceMethod(UserDefaults.self,NSSelectorFromString("setObject:forKey:")),
            let replacement=class_getInstanceMethod(UserDefaults.self,#selector(UserDefaults.audit_setObject(_:forKey:))) else { return }
        enabled=CommandLine.arguments.contains("--no-scroll-persistence")
        method_exchangeImplementations(original,replacement)
    }
}
extension UserDefaults {
    @objc dynamic fileprivate func audit_setObject(_ value:Any?,forKey key:String) {
        if key.hasPrefix("view.scroll.v2.") {
            ScrollPersistenceControl.lock.lock()
            if ScrollPersistenceControl.enabled { ScrollPersistenceControl.suppressed += 1 }
            else { ScrollPersistenceControl.writes += 1 }
            ScrollPersistenceControl.lock.unlock()
            if ScrollPersistenceControl.enabled { return }
        }
        audit_setObject(value,forKey:key)
    }
}

@MainActor final class InteractionAudit: NSObject, NSApplicationDelegate {
    private var workspace: NSWindow!
    private var hosting: NSHostingView<AnyView>!
    private var popover: NSPopover!
    private var statusItem: NSStatusItem?
    private var pet: PetWindowController?
    private var dialog: NSWindow?
    private var model: AppModel!
    private var subscriptions = Set<AnyCancellable>()
    private var publications = 0
    private var livePublications = 0
    private var appearance:AppAppearanceController?
    private var maxLag = 0.0
    private var heartbeat: Task<Void, Never>?
    private var rows: [[String: Any]] = []
    private var output: URL!
    private var repetitions = 20
    private let clock = ContinuousClock()
    private var fixture: [String: Any] = [:]
    private var only = ""
    private var scrollMetrics: [String: Any] = [:]
    private var finished=false

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        Task {
            do { try await run(); NSApp.terminate(nil) }
            catch {
                fputs("Interaction audit failed: \(error)\n", stderr)
                if let output {
                    try? JSONSerialization.data(withJSONObject:["error":String(describing:error),"at":Date().timeIntervalSince1970,
                        "publications":publications,"livePublications":livePublications],options:[.prettyPrinted,.sortedKeys])
                        .write(to:output.appendingPathComponent("failure.json"),options:.atomic)
                }
                cleanup(); exit(1)
            }
        }
    }
    private func cleanup() {
        heartbeat?.cancel(); model?.stop()
        popover?.close(); dialog?.close(); workspace?.close(); pet?.window.orderOut(nil)
        if let statusItem { NSStatusBar.system.removeStatusItem(statusItem) }
    }
    private func uptime() -> Double { ProcessInfo.processInfo.systemUptime }
    private func cpu() -> Double {
        var usage = rusage(); getrusage(RUSAGE_SELF, &usage)
        return Double(usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) + Double(usage.ru_utime.tv_usec + usage.ru_stime.tv_usec)/1e6
    }
    private func memory() -> UInt64 {
        var info = mach_task_basic_info(); var count = mach_msg_type_number_t(MemoryLayout.size(ofValue: info)/MemoryLayout<integer_t>.size)
        let result = withUnsafeMutablePointer(to: &info) { pointer in
            pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? info.resident_size : 0
    }
    private func pause(_ seconds: Double) async throws { try await Task.sleep(for: .seconds(seconds)) }
    private func matches(_ name:String)->Bool {
        only.isEmpty || only.split(separator: ",").contains { pattern in
            pattern.hasPrefix("=") ? name == String(pattern.dropFirst()) : name.contains(pattern)
        }
    }
    private func flush() {
        for window in [workspace, popover?.contentViewController?.view.window, dialog].compactMap({ $0 }) where window.isVisible {
            window.contentView?.layoutSubtreeIfNeeded(); window.contentView?.displayIfNeeded()
        }
    }
    private func save() throws {
        let report: [String: Any] = ["pid": ProcessInfo.processInfo.processIdentifier,"complete":finished,
            "optimized": true, "repetitions": repetitions, "samples": rows,
            "appearanceTheme":model.settings.appearance?.theme ?? "system",
            "activePageOnlyControl":CommandLine.arguments.contains("--only-active-page"),
            "backgroundPausedControl":CommandLine.arguments.contains("--static-input"),
            "scrollPersistenceDisabledControl":ScrollPersistenceControl.enabled,
            "suppressedScrollWrites":ScrollPersistenceControl.suppressed,
            "note": "Production handlers and NSHostingViews. First feedback is after a 1 ms event-loop yield and forced layout; completion includes the existing 120 ms query debounce for navigation. Scroll sends continuous pixel wheel events, and steps the native clip view when SwiftUI ignores an event. The per-sample fallback count distinguishes those paths. This measures rendered scrolling and concurrent handlers, not hardware input routing or photon-to-photon latency."]
        try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted,.sortedKeys])
            .write(to: output.appendingPathComponent("report.json"), options: .atomic)
    }
    private func phase(_ value: String) throws {
        try Data(value.utf8).write(to: output.appendingPathComponent("phase.txt"), options: .atomic)
    }
    private func quiesce(debounce: Bool = true) async throws {
        if debounce { try await pause(0.13) }
        let deadline = uptime() + 20
        while model.busy || model.quotaBusy || model.serverBusy || model.dashboardPending || model.settingsSaving {
            guard uptime() < deadline else {
                throw ClientError.message("UI idle deadline: busy=\(model.busy) quota=\(model.quotaBusy) server=\(model.serverBusy) dashboard=\(model.dashboardPending) settings=\(model.settingsSaving)")
            }
            try await pause(0.01)
        }
        try await pause(0.001); flush()
    }
    private func measure(_ name: String, debounce: Bool = true,
        setup: (() async throws -> Void)? = nil, action: @escaping () async throws -> Void) async throws {
        guard matches(name) else { return }
        for iteration in 0..<repetitions {
            try await setup?(); try await quiesce(); try phase(name + ":" + String(iteration))
            maxLag = 0
            scrollMetrics = [:]
            let start = uptime(), beforeCPU = cpu(), before = publications, beforeLive=livePublications, epoch=Date().timeIntervalSince1970
            let beforeWrites=ScrollPersistenceControl.writes
            let work = Task { try await action() }
            try await pause(0.001); flush()
            let feedback = (uptime()-start)*1000
            try await work.value; try await quiesce(debounce: debounce)
            if name.contains("scroll") {
                scrollMetrics["gestureCpuMs"]=(cpu()-beforeCPU)*1000
                scrollMetrics["gestureCompletionMs"]=(uptime()-start)*1000
                let settleCPU=cpu()
                // Include deferred persistence and the resulting render work.
                try await pause(0.35);flush();try await quiesce(debounce:false)
                scrollMetrics["settleCpuMs"]=(cpu()-settleCPU)*1000
                scrollMetrics["scrollPositionWrites"]=ScrollPersistenceControl.writes-beforeWrites
            }
            let elapsed = uptime()-start, cpuMs = (cpu()-beforeCPU)*1000
            var row: [String:Any] = ["scenario":name,"iteration":iteration,"first":iteration==0,
                "startedAt":epoch,"feedbackMs":feedback,"completionMs":elapsed*1000,"cpuMs":cpuMs,
                "cpuPercent":cpuMs/(elapsed*10), "maxEventLoopLagMs":maxLag,
                "publications":publications-before,"livePublications":livePublications-beforeLive,"rssBytes":memory(),
                "workspaceVisible":workspace.isVisible,
                "workspaceOccluded": !workspace.occlusionState.contains(.visible)]
            row.merge(scrollMetrics) { _,new in new }; rows.append(row)
            try save(); try phase("cooldown")
            if iteration==0,name.contains("scroll") {
                let root:NSView=name.hasPrefix("panel") ? popover.contentViewController!.view:
                    name.hasPrefix("details") ? dialog!.contentView!:hosting
                try capture(root,name:name.replacingOccurrences(of:".",with:"-"))
            }
            try await pause(0.12)
        }
        let samples = rows.filter { $0["scenario"] as? String == name }
        let values = samples.compactMap { $0["cpuMs"] as? Double }.sorted()
        print("\(name): median CPU ms \(values[values.count/2])", terminator:"\n")
        fflush(stdout)
    }
    private func idle(_ name: String, visible: Bool, duration: Double = 10) async throws {
        guard matches(name) || name=="idle.after-interactions" else { return }
        model.setWindowVisible(visible, window:"detail")
        if visible { workspace.orderFrontRegardless() } else { workspace.orderOut(nil) }
        try await quiesce(); try phase(name)
        let start=uptime(), beforeCPU=cpu(), before=publications; maxLag=0
        try await pause(only.isEmpty ? duration:min(5,duration))
        rows.append(["scenario":name,"iteration":0,"durationSeconds":uptime()-start,
            "cpuMs":(cpu()-beforeCPU)*1000,"cpuPercent":(cpu()-beforeCPU)/(uptime()-start)*100,
            "publications":publications-before,"maxEventLoopLagMs":maxLag,"rssBytes":memory(),
            "workspaceVisible":workspace.isVisible,"workspaceOccluded": !workspace.occlusionState.contains(.visible)])
        try save()
    }
    private func page(_ page: String, category: String? = nil, section: String? = nil) {
        if let category { model.settingsTab=category }
        if let section { model.generalPage=section }
        model.detailPage=page
        model.setWindowVisible(true,window:"detail"); workspace.orderFrontRegardless()
    }
    private func openEstimate() async throws {
        guard let quota = model.menuDashboard.quotas.first(where: { $0.provider=="codex" }) ?? model.menuDashboard.quotas.first else {
            throw ClientError.message("Snapshot has no quota to show")
        }
        model.showEstimate = { [weak self] value, credits, group in
            guard let self else { return }
            if self.dialog == nil {
                self.dialog = NSWindow(contentRect:NSRect(x:250,y:140,width:530,height:650),styleMask:[.titled,.closable,.resizable],backing:.buffered,defer:false)
                self.dialog?.isReleasedWhenClosed=false
            }
            self.dialog?.contentView=NSHostingView(rootView:QuotaEstimateView(model:self.model,quota:value,credits:credits,initialGroup:group))
            self.dialog?.title="Aieyes · isolated estimate audit"; self.dialog?.orderFrontRegardless()
        }
        await model.openEstimate(accountKey:quota.id)
    }
    private func scrollView(in root: NSView) -> NSScrollView? {
        var target=root.hitTest(NSPoint(x:root.bounds.width*0.72,y:root.bounds.height*0.45))
        while let view=target {
            if let scroll=view as? NSScrollView, (scroll.documentView?.bounds.height ?? 0)>scroll.contentView.bounds.height+5 { return scroll }
            target=view.superview
        }
        return scrollViews(in:root).filter { $0.bounds.width>root.bounds.width*0.5 && ($0.documentView?.bounds.height ?? 0)>$0.contentView.bounds.height+5 }
            .max { $0.bounds.width*$0.bounds.height < $1.bounds.width*$1.bounds.height }
    }
    private func scrollViews(in root:NSView)->[NSScrollView] {
        ((root as? NSScrollView).map{[$0]} ?? []) + root.subviews.flatMap { scrollViews(in:$0) }
    }
    private func scroll(in root: NSView, interacting: Bool = false,target:NSScrollView?=nil) async throws {
        guard let scroll=target ?? scrollView(in:root),let document=scroll.documentView else {
            throw ClientError.message("Expected an overflowing content scroll view")
        }
        scroll.contentView.scroll(to:.zero); scroll.reflectScrolledClipView(scroll.contentView)
        var durations:[Double]=[],positions:[Double]=[], inputDelays:[Double]=[]
        var hoverEvents=0, fallbackSteps=0, refresh:Task<Void,Never>?
        var refreshQueueMs=0.0,refreshCompletionMs=0.0,hoverMaxMs=0.0
        func tintUpdates(_ view:NSView)->Int {
            (view as? PanelTintView)?.colorUpdates ?? view.subviews.reduce(0) { $0+tintUpdates($1) }
        }
        let initialTint=tintUpdates(root)
        let contentIdentity=ObjectIdentifier(scroll)
        for step in 0..<60 {
            let start=uptime(), before=scroll.contentView.bounds.minY
            if let event=CGEvent(scrollWheelEvent2Source:nil,units:.pixel,wheelCount:1,wheel1:step<30 ? -48:48,wheel2:0,wheel3:0) {
                event.setIntegerValueField(.scrollWheelEventIsContinuous,value:1)
                event.setIntegerValueField(.scrollWheelEventScrollPhase,value:Int64((step==0 ? NSEvent.Phase.began : step==59 ? .ended:.changed).rawValue))
                if let native=NSEvent(cgEvent:event) { scroll.scrollWheel(with:native) }
            }
            if abs(scroll.contentView.bounds.minY-before)<0.1 {
                let destination=min(max(0,document.bounds.height-scroll.contentView.bounds.height),max(0,before+(step<30 ? 48:-48)))
                scroll.contentView.scroll(to:NSPoint(x:scroll.contentView.bounds.minX,y:destination))
                scroll.reflectScrolledClipView(scroll.contentView); fallbackSteps += 1
            }
            if interacting,step%10==0,let window=root.window {
                let point=root.convert(NSPoint(x:root.bounds.width*0.7,y:root.bounds.height*0.5),to:nil)
                if let event=NSEvent.mouseEvent(with:.mouseMoved,location:point,modifierFlags:[],timestamp:uptime(),windowNumber:window.windowNumber,context:nil,eventNumber:step,clickCount:0,pressure:0) {
                    let at=uptime();window.sendEvent(event);hoverMaxMs=max(hoverMaxMs,(uptime()-at)*1000);hoverEvents += 1
                }
                if step==20 {
                    let requested=uptime()
                    refresh=Task { refreshQueueMs=(self.uptime()-requested)*1000;await self.model.refreshQuotas();refreshCompletionMs=(self.uptime()-requested)*1000 }
                }
            }
            try await pause(1.0/60)
            let after=scroll.contentView.bounds.minY
            if abs(after-before)>0.1 { inputDelays.append((uptime()-start)*1000) }
            durations.append((uptime()-start)*1000);positions.append(after)
        }
        await refresh?.value
        guard (positions.max() ?? 0)-(positions.min() ?? 0)>10 else { throw ClientError.message("Scroll events did not move content") }
        let ordered=durations.sorted(), delays=inputDelays.sorted()
        scrollMetrics=["scrollFrames":durations.count,"scrollFrameP95Ms":ordered[Int(ceil(Double(ordered.count)*0.95))-1],
            "scrollFramesOver33ms":durations.filter{$0>33.33}.count,
            "scrollPositionRange":(positions.max() ?? 0)-(positions.min() ?? 0),
            "scrollViewRetained":scroll.isDescendant(of:root),
            "sameHitTestScrollTarget":scrollView(in:root).map{ObjectIdentifier($0)==contentIdentity} ?? false,
            "scrollInputResponseP95Ms":delays.isEmpty ? 0:delays[Int(ceil(Double(delays.count)*0.95))-1],
            "hoverEvents":hoverEvents,"contentHeight":document.bounds.height,"clipViewFallbackSteps":fallbackSteps,
            "panelTintUpdatesDuringScroll":tintUpdates(root)-initialTint,"hoverDispatchMaxMs":hoverMaxMs,
            "refreshQueueMs":refreshQueueMs,"refreshCompletionMs":refreshCompletionMs]
    }
    private func control(in root: Any, containing text: String) -> NSObject? {
        var visited=Set<ObjectIdentifier>()
        func getter(_ node:NSObject,_ name:String)->AnyObject? {
            let selector=NSSelectorFromString(name)
            return node.responds(to:selector) ? node.perform(selector)?.takeUnretainedValue():nil
        }
        func search(_ value:Any,depth:Int)->NSObject? {
            guard depth<40,visited.count<10000,let node=value as? NSObject else { return nil }
            let identity=ObjectIdentifier(node)
            guard visited.insert(identity).inserted else { return nil }
            let label=(getter(node,"accessibilityLabel") as? String) ?? (getter(node,"accessibilityTitle") as? String) ?? ""
            let role=(getter(node,"accessibilityRole") as? String) ?? ""
            if label.contains(text), ["AXButton","AXDisclosureTriangle","AXCheckBox"].contains(role) { return node }
            let children=(getter(node,"accessibilityChildren") as? [Any]) ?? (node as? NSView)?.subviews ?? []
            for child in children {
                if let found=search(child,depth:depth+1) { return found }
            }
            return nil
        }
        return search(root,depth:0)
    }
    private func press(_ control:NSObject?)->Bool {
        let selector=NSSelectorFromString("accessibilityPerformPress")
        guard let control,control.responds(to:selector),let implementation=control.method(for:selector) else { return false }
        typealias Action = @convention(c) (AnyObject,Selector)->Bool
        return unsafeBitCast(implementation,to:Action.self)(control,selector)
    }
    private func capture(_ root:NSView,name:String) throws {
        flush()
        if let bitmap=root.bitmapImageRepForCachingDisplay(in:root.bounds) {
            root.cacheDisplay(in:root.bounds,to:bitmap)
            try bitmap.representation(using:.png,properties:[:])?.write(to:output.appendingPathComponent(name+".png"))
        }
    }
    private func benchmarkFormatting() throws {
        let records=fixture["quotaEstimates.list"] as? [[String:Any]] ?? []
        let stamps=records.flatMap { record in ["startedAt","checkpointAt","endedAt"].compactMap { record[$0] as? Double } }
        let cached=DateFormatter(); cached.locale=Locale(identifier:"en_US_POSIX");cached.timeZone = .autoupdatingCurrent;cached.dateFormat="MM/dd HH:mm"
        var results:[[String:Any]]=[]
        for _ in 0..<20 {
            var start=cpu(); let original=stamps.map { Format.date($0) }; let fresh=(cpu()-start)*1000
            start=cpu();let reused=stamps.map { $0>0 ? cached.string(from:Date(timeIntervalSince1970:$0)):"—" };let shared=(cpu()-start)*1000
            guard original==reused else { throw ClientError.message("Formatter control changed rendered dates") }
            results.append(["freshCpuMs":fresh,"reusedCpuMs":shared])
        }
        try JSONSerialization.data(withJSONObject:["datesPerPass":stamps.count,"samples":results],options:[.prettyPrinted,.sortedKeys])
            .write(to:output.appendingPathComponent("formatting-report.json"),options:.atomic)
    }
    func run() async throws {
        let args=CommandLine.arguments
        guard args.count>=4 else { throw ClientError.message("Expected output, fixture, preferences") }
        output=URL(fileURLWithPath:args[1]); fixture=try JSONSerialization.jsonObject(with:Data(contentsOf:URL(fileURLWithPath:args[2]))) as! [String:Any]
        let prefs=try PropertyListSerialization.propertyList(from:Data(contentsOf:URL(fileURLWithPath:args[3])),format:nil) as! [String:Any]
        repetitions=Int(ProcessInfo.processInfo.environment["AIEYES_PROFILE_REPETITIONS"] ?? "20") ?? 20
        if let i=args.firstIndex(of:"--only"), i+1<args.count { only=args[i+1] }
        model=AppModel()
        model.objectWillChange.sink { [weak self] _ in self?.publications += 1 }.store(in:&subscriptions)
        model.live.objectWillChange.sink { [weak self] _ in self?.livePublications += 1 }.store(in:&subscriptions)
        heartbeat=Task { [weak self] in
            while !Task.isCancelled {
                let start=ProcessInfo.processInfo.systemUptime
                do { try await Task.sleep(for:.milliseconds(5)) } catch { return }
                self?.maxLag=max(self?.maxLag ?? 0,(ProcessInfo.processInfo.systemUptime-start-0.005)*1000)
            }
        }
        workspace=NSWindow(contentRect:NSRect(x:150,y:110,width:1120,height:780),styleMask:[.titled,.closable,.resizable],backing:.buffered,defer:false)
        if let saved=prefs["NSWindow Frame AieyesDetails"] as? String {
            let values=saved.split(separator:" ").compactMap { Double($0) }
            if values.count>=4 { workspace.setFrame(NSRect(x:150,y:110,width:values[2],height:values[3]),display:false) }
        }
        workspace.isReleasedWhenClosed=false; workspace.title="Aieyes · isolated interaction audit"
        let content:AnyView=args.contains("--only-active-page") ? AnyView(AuditActiveWindowView(model:model)):AnyView(MainWindowView(model:model))
        hosting=NSHostingView(rootView:AnyView(content.environment(\.previewAccessibleSurfaces,args.contains("--plain-surfaces")))); workspace.contentView=hosting
        statusItem=NSStatusBar.system.statusItem(withLength:100)
        if let button=statusItem?.button {
            let label=NSHostingView(rootView:MenuActivityLabel(model:model)); label.frame=NSRect(x:0,y:0,width:100,height:24); button.addSubview(label)
        }
        pet=PetWindowController(model:model) { _,_ in }
        popover=NSPopover(); popover.animates=false; popover.behavior = .applicationDefined
        popover.contentViewController=PanelContainer(model:model); popover.contentSize=NSSize(width:450,height:720)
        appearance=AppAppearanceController(popover:popover)
        model.$settings.map(\.appearance).removeDuplicates().sink { [weak self] value in
            self?.appearance?.apply(theme:value?.theme ?? "system",accent:value?.accent ?? "indigo",
                chartPalette:value?.chartPalette ?? "classic",chartContrast:value?.chartContrast ?? "high")
        }.store(in:&subscriptions)
        model.showSettings = { [weak self] in self?.page("settings") }
        model.showDetailPage = { [weak self] value in self?.page(value) }
        page("settings")
        let deadline=uptime()+45
        while !model.settingsLoaded || model.menuDashboard.generatedAt<=0 {
            guard uptime()<deadline else { throw ClientError.message("Bootstrap failed: \(model.message ?? model.dashboardError ?? "no settings")") }
            try await pause(0.05)
        }
        try await quiesce(); try await pause(1)
        if args.contains("--static-input") { model.profilePauseBackground() }
        try benchmarkFormatting()
        try await idle("idle.hidden",visible:false)
        try await idle("idle.settings",visible:true)
        for (name,destination,category,section) in [
            ("navigate.overview","agent",nil,nil),("navigate.analysis","analysis",nil,nil),
            ("navigate.servers","servers",nil,nil),("navigate.settings.accounts","settings","integrations",nil),
            ("navigate.settings.machines","settings","servers",nil),("navigate.settings.prices","settings","prices",nil),
            ("navigate.settings.wakeups","settings","wakeups",nil),("navigate.settings.appearance","settings","general","appearance"),
            ("navigate.settings.desktop","settings","general","desktop"),("navigate.settings.network","settings","general","network"),
            ("navigate.settings.refresh","settings","general","refresh"),("navigate.settings.notifications","settings","general","notifications"),
            ("navigate.settings.updates","settings","general","updates")
        ] as [(String,String,String?,String?)] {
            try await measure(name,setup:{ self.page(destination=="agent" ? "analysis":"agent") }) {
                self.page(destination,category:category,section:section)
            }
        }
        try await measure("panel.open",debounce:true,setup:{ self.popover.close(); self.model.setWindowVisible(false,window:"panel") }) {
            if let button=self.statusItem?.button {
                self.popover.show(relativeTo:button.bounds,of:button,preferredEdge:.minY)
                self.model.setWindowVisible(true,window:"panel")
            }
        }
        popover.close(); model.setWindowVisible(false,window:"panel")
        for days in [1,7,30,365] {
            try await measure("filter.analysis.\(days)d",setup:{
                self.page("analysis"); self.model.updateFilters(DashboardFilterPreference(provider:"qoder",days:days==7 ? 1:7),scope:"analysis")
            }) { self.model.updateFilters(DashboardFilterPreference(provider:"qoder",days:days),scope:"analysis") }
        }
        try await measure("filter.analysis.all-agents",setup:{
            self.page("analysis"); self.model.updateFilters(DashboardFilterPreference(provider:"qoder",days:7),scope:"analysis")
        }) { self.model.updateFilters(DashboardFilterPreference(provider:"all",days:7),scope:"analysis") }
        try await measure("refresh.history",debounce:false,setup:{ self.page("analysis") }) { await self.model.scan() }
        try await measure("refresh.quotas",debounce:false,setup:{ self.page("agent") }) { await self.model.refreshQuotas() }
        try await measure("refresh.prices",debounce:false,setup:{ self.page("settings",category:"prices") }) { await self.model.syncPrices() }
        try await measure("refresh.servers",debounce:false,setup:{ self.page("servers") }) { await self.model.sampleHosts() }
        try await measure("reload.unchanged-analysis",debounce:false,setup:{ self.dialog?.orderOut(nil);self.page("analysis") }) { _ = await self.model.reload() }
        try await measure("reload.unchanged-with-estimate",debounce:false,setup:{
            self.page("analysis");if self.dialog?.isVisible != true { try await self.openEstimate() }
        }) { _ = await self.model.reload() }
        try await measure("details.estimate.open",setup:{ self.dialog?.orderOut(nil) }) { try await self.openEstimate() }
        var disclosure:NSObject?
        try await measure("click.estimate.calculation",debounce:false,setup:{
            if self.dialog?.isVisible != true { try await self.openEstimate() }; try await self.quiesce()
            disclosure=self.control(in:self.dialog!.contentView!,containing:"计算依据")
        }) {
            if !self.press(disclosure) {
                self.scrollMetrics["unsupportedNativeAction"]="Disclosure AXPress unavailable; no click latency claim"
            }
        }
        try await measure("details.estimate.scroll",debounce:false,setup:{ if self.dialog?.isVisible != true { try await self.openEstimate() } }) { try await self.scroll(in:self.dialog!.contentView!) }
        dialog?.orderOut(nil)
        try await measure("analysis.scroll",debounce:false,setup:{ self.page("analysis") }) { try await self.scroll(in:self.hosting) }
        try await measure("prices.scroll",debounce:false,setup:{ self.page("settings",category:"prices") }) { try await self.scroll(in:self.hosting) }
        var catalog:NSScrollView?
        try await measure("prices.catalog-scroll",debounce:false,setup:{
            self.page("settings",category:"prices");try await self.quiesce()
            catalog=self.scrollViews(in:self.hosting).filter { $0.bounds.width>self.hosting.bounds.width*0.5 }
                .max { ($0.documentView?.bounds.height ?? 0)-$0.contentView.bounds.height < ($1.documentView?.bounds.height ?? 0)-$1.contentView.bounds.height }
            if let catalog { _=catalog.scrollToVisible(catalog.bounds) }
        }) { try await self.scroll(in:self.hosting,target:catalog) }
        if matches("servers.scroll") {
            // A saved single-host filter may fit without scrolling. Never let
            // the fallback select an overflowing *hidden* page in that case.
            // Restore the user's filter before testing the other surfaces.
            let defaults = UserDefaults.standard
            let selectionKey = NativePanelKey.detailServerSelection.rawValue, filterKey = NativePanelKey.detailServerFilter.rawValue
            let selection = defaults.object(forKey:selectionKey), filter = defaults.object(forKey:filterKey)
            defer { defaults.set(selection,forKey:selectionKey);defaults.set(filter,forKey:filterKey) }
            try await measure("servers.scroll",debounce:false,setup:{
                self.page("servers")
                defaults.set("all",forKey:selectionKey);defaults.set("all",forKey:filterKey)
                for host in self.model.settings.monitoredHosts where host.enabled {
                    defaults.set(true,forKey:"server.expanded.detail."+host.id)
                    defaults.set(true,forKey:"server.metric.expanded.detail."+host.id+":cpu")
                }
            }) { try await self.scroll(in:self.hosting) }
        }
        for interacting in [false,true] {
            try await measure(interacting ? "panel.scroll-with-feedback":"panel.scroll",debounce:false,setup:{
                if let button=self.statusItem?.button {
                    self.popover.show(relativeTo:button.bounds,of:button,preferredEdge:.minY)
                    self.model.setWindowVisible(true,window:"panel")
                }
            }) { try await self.scroll(in:self.popover.contentViewController!.view,interacting:interacting) }
        }
        if only.isEmpty || only.contains("panel.scroll") { try capture(popover.contentViewController!.view,name:"panel-after-scroll") }
        popover.close(); model.setWindowVisible(false,window:"panel")
        try await measure("window.reopen",setup:{ self.workspace.orderOut(nil); self.model.setWindowVisible(false,window:"detail") }) {
            self.workspace.orderFrontRegardless(); self.model.setWindowVisible(true,window:"detail")
        }
        try await measure("burst.refresh-and-navigate",setup:{ self.page("agent") }) {
            let refresh=Task { await self.model.refreshQuotas() }
            self.page("analysis"); self.page("servers"); self.page("settings",category:"prices"); self.page("analysis")
            await refresh.value
        }
        try await measure("burst.repeat-refresh",debounce:false,setup:{ self.page("agent") }) {
            await withTaskGroup(of:Void.self) { group in
                for _ in 0..<5 { group.addTask { await self.model.refreshQuotas() } }
            }
        }
        try await idle("idle.after-interactions",visible:true)
        finished=true;try phase("finished"); try save(); cleanup()
    }
}

@main struct ReleaseInteractionDriver {
    static func main() {
        // Each driver speaks its own UI contract; the replay only provides local reads.
        setenv("AIEYES_PROFILE_CONTRACT",CoreContract.contractHash,1)
        ScrollPersistenceControl.install()
        if CommandLine.arguments.contains("--no-scroll-persistence"), !ScrollPersistenceControl.enabled { fputs("Could not install causal control\n",stderr);exit(1) }
        let app=NSApplication.shared, delegate=InteractionAudit(); app.delegate=delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}
