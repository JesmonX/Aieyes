// Compile against production objects, or the baseline sources without AieyesApp.swift.
import AppKit
import SwiftUI
import Darwin
#if AIEYES_PERFORMANCE_MODULE
@testable import Aieyes
#endif

@MainActor final class RuntimePerformanceDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem?
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        Task {
            do { try await run(); NSApp.terminate(nil) }
            catch { fputs("Runtime performance fixture failed: \(error)\n", stderr); exit(1) }
        }
    }
    private func memory() -> (UInt64, UInt64) {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout.size(ofValue: info) / MemoryLayout<integer_t>.size)
        let result = withUnsafeMutablePointer(to: &info) { ptr in
            ptr.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count) }
        }
        return result == KERN_SUCCESS ? (info.phys_footprint, info.resident_size) : (0, 0)
    }
    private func cpu() -> Double {
        var usage = rusage(); getrusage(RUSAGE_SELF, &usage)
        return Double(usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) + Double(usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) / 1e6
    }
    private func capture(_ view: NSView, _ path: URL) throws {
        view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
        guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: bitmap)
        try bitmap.representation(using: .png, properties: [:])?.write(to: path)
    }
    func run() async throws {
        let args = CommandLine.arguments
        guard args.count >= 4 else { throw ClientError.message("Expected output directory, fixture JSON, pet JSON, [duration=2400], [warmup=600]") }
        let output = URL(fileURLWithPath: args[1]); try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let duration = args.count > 4 ? Double(args[4]) ?? 2400 : 2400
        let warmup = args.count > 5 ? Double(args[5]) ?? 600 : 600
        let fixture = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: args[2]))) as! [String: Any]
        func decode<T: Decodable>(_ key: String) throws -> T { try JSONDecoder().decode(T.self, from: JSONSerialization.data(withJSONObject: fixture[key]!)) }
        var overrides = NativePanelKey.snapshotOverrides()
        overrides["aieyes.pet.enabled"] = ProcessInfo.processInfo.environment["AIEYES_PERF_PET"] != "0"; overrides["aieyes.pet.scale"] = 1.0
        overrides["aieyes.pet.position"] = "{100, 100}"
        UserDefaults.standard.setVolatileDomain(overrides, forName: UserDefaults.argumentDomain)
        let model = AppModel(autostart: false)
        defer { model.stop() }
        model.settings = try decode("settings"); model.settingsDraft = model.settings; model.settingsLoaded = true
        let dashboard: Dashboard = try decode("dashboard")
        model.dashboard = dashboard
        await model.reload()
        model.hosts = try decode("hosts")
        model.importedPet = try JSONDecoder().decode(PetSelection.self, from: Data(contentsOf: URL(fileURLWithPath: args[3]))).pet
        guard model.importedPet != nil else { throw ClientError.message("Synthetic pet missing") }
        model.sessions = [LiveSession(id: "synthetic", source: "Codex", phase: .working, updatedAt: Date())]
        model.petState = PetState(behavior: "running", label: "进行中", row: 1, frames: 8, frameMs: 100, key: "synthetic")
        if ProcessInfo.processInfo.environment["AIEYES_PERF_MENU"] != "0" { statusItem = NSStatusBar.system.statusItem(withLength: 80) }
        if let button = statusItem?.button {
            let label = NSHostingView(rootView: MenuActivityLabel(model: model)); label.frame = NSRect(x: 0, y: 0, width: 80, height: 24); button.addSubview(label)
        }
        let pet = PetWindowController(model: model) { _, _ in }
        let workspace = NSWindow(contentRect: NSRect(x: 180, y: 180, width: 1120, height: 780), styleMask: [.titled], backing: .buffered, defer: false)
        workspace.isReleasedWhenClosed = false; workspace.title = "Aieyes isolated runtime fixture"
        let view = NSHostingView(rootView: MainWindowView(model: model)); workspace.contentView = view
        model.detailPage = "analysis"; model.setWindowVisible(true, window: "detail"); workspace.orderFront(nil)
        try await Task.sleep(for: .seconds(2))
        try capture(view, output.appendingPathComponent("workspace.png"))
        model.setWindowVisible(false, window: "detail"); workspace.orderOut(nil)
        try await Task.sleep(for: .seconds(1))
        try capture(pet.window.contentView!, output.appendingPathComponent("pet-start.png"))
        #if AIEYES_PERFORMANCE_MODULE
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        let sprite = descendants(pet.window.contentView!).compactMap { $0 as? PetLayerImage }.first
        if ProcessInfo.processInfo.environment["AIEYES_PERF_PET"] != "0" {
            guard let sprite, sprite.animationStarts > 0, sprite.layer?.animation(forKey:"pet.frames") != nil else { throw ClientError.message("Visible pet must run its keyframes: visible=\(pet.window.isVisible) occlusion=\(pet.window.occlusionState.rawValue)") }
            var images = Set<UInt>()
            for _ in 0..<8 {
                if let value = sprite.layer?.presentation()?.contents { images.insert(CFHash(value as CFTypeRef)) }
                try await Task.sleep(for:.milliseconds(120))
            }
            guard images.count > 1 else { throw ClientError.message("Pet presentation contents must advance across frames") }
            print("Verified actual pet presentation frame changes: \(images.count)",terminator:"\n")
        }
        #endif
        let started = ProcessInfo.processInfo.systemUptime
        var lastTime = started, lastCPU = cpu(), samples: [[String: Any]] = []
        var tick = 0
        while ProcessInfo.processInfo.systemUptime - started < duration {
            try await Task.sleep(for: .seconds(1)); tick += 1
            // A real metric changes once per second. Retained hidden pages must stay quiet.
            if ProcessInfo.processInfo.environment["AIEYES_PERF_UPDATES"] != "0", !model.hosts.isEmpty { model.hosts[0].sample?.timestamp = Date().timeIntervalSince1970 }
            if tick % 10 == 0 {
                let now = ProcessInfo.processInfo.systemUptime, currentCPU = cpu(), bytes = memory()
                let sample: [String: Any] = ["elapsedSeconds":now-started, "cpuPercent":(currentCPU-lastCPU)/(now-lastTime)*100, "physicalBytes":bytes.0, "rssBytes":bytes.1, "afterWarmup":now-started >= warmup]
                samples.append(sample); lastTime = now; lastCPU = currentCPU
                let report: [String: Any] = ["pid":ProcessInfo.processInfo.processIdentifier, "durationSeconds":duration, "warmupSeconds":warmup, "complete":false, "samples":samples, "scenario":"Menu pulse + pet loop, retained hidden 365-day workspace, metric update each second. Synthetic data; CPU is ten-second process-time average."]
                try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]).write(to: output.appendingPathComponent("report.json"), options: .atomic)
            }
        }
        try capture(pet.window.contentView!, output.appendingPathComponent("pet-end.png"))
        let report: [String: Any] = ["pid":ProcessInfo.processInfo.processIdentifier, "durationSeconds":duration, "warmupSeconds":warmup, "complete":true, "samples":samples, "note":"Isolated synthetic debug build; does not replace installed app or establish Windows/Linux native performance."]
        try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]).write(to: output.appendingPathComponent("report.json"), options: .atomic)
        workspace.close(); pet.window.orderOut(nil)
        if let statusItem { NSStatusBar.system.removeStatusItem(statusItem) }
        print("Runtime performance verification completed: \(output.path)")
    }
}
@main struct RuntimePerformance {
    static func main() {
        let app = NSApplication.shared, delegate = RuntimePerformanceDelegate(); app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}
