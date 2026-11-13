// Default: pure guards and compile native terminal scripts. No permission prompts or activation.
// Desktop check: --pid <agent pid> [--workspace <path>] [--expect pane|tab|window|application|automationRequired] [--restore]
// Desktop thread: --thread <native UUID> [--restore]. --inventory reports existing permission only.
import AppKit
import Carbon
@testable import Aieyes

@main struct VerifySessionFocus {
    @MainActor static func main() async throws {
        let args = Array(CommandLine.arguments.dropFirst())
        func option(_ name: String) -> String? {
            guard let index = args.firstIndex(of: name), index + 1 < args.count else { return nil }
            return args[index + 1]
        }
        let terminals = ["com.mitchellh.ghostty", "com.apple.Terminal", "com.googlecode.iterm2"]
        for bundle in terminals {
            for invalid in ["", "ttys001", "/dev/ttys001\"", "/dev/ttys001\n", "/dev/ttys001;activate", "/dev/tty" + String(repeating: "a", count: 129)] {
                precondition(SessionFocus.terminalScript(bundle: bundle, tty: invalid) == nil)
            }
            let source = SessionFocus.terminalScript(bundle: bundle, tty: "/dev/ttys999")!
            if NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) != nil {
                var error: NSDictionary?
                precondition(NSAppleScript(source: source)?.compileAndReturnError(&error) == true, "\(bundle): \(String(describing: error))")
                print("Compiled terminal script against installed dictionary: \(bundle)")
            } else { print("Dictionary unavailable (not installed): \(bundle)") }
        }
        precondition(SessionFocus.terminalScript(bundle: "unknown", tty: "/dev/ttys001") == nil)
        for (document, workspace, matches) in [
            ("file:///work/My%20Project/src/main.swift", "/work/My Project", true),
            ("/work/Aieyes", "/work/Aieyes/", true),
            ("/work/Aieyes/../other/file", "/work/Aieyes", false),
            ("/work/Aieyes-copy/file", "/work/Aieyes", false),
            ("/elsewhere/Aieyes/file", "/work/Aieyes", false),
            ("Aieyes", "/work/Aieyes", false),
            ("/work/Aieyes", "/", false)
        ] { precondition(SessionFocus.matchesWorkspace(document: document, workspace: workspace) == matches) }
        print("TTY guards and full workspace path matching passed")
        if args.contains("--inventory") {
            for bundle in terminals + ["com.openai.codex", "com.microsoft.VSCode"] {
                for app in NSRunningApplication.runningApplications(withBundleIdentifier: bundle) {
                    let descriptor = NSAppleEventDescriptor(bundleIdentifier: bundle)
                    let permission = terminals.contains(bundle) ? AEDeterminePermissionToAutomateTarget(descriptor.aeDesc, typeWildCard, typeWildCard, false) : 0
                    print("\(bundle): pid=\(app.processIdentifier), windows=\(SessionFocus.windowCount(app.processIdentifier)), automation=\(permission)")
                }
            }
            print("Existing accessibility permission: \(AXIsProcessTrusted())")
        }
        guard option("--pid") != nil || option("--thread") != nil else { return }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        setenv("AIEYES_DATA_DIR", root.path, 1)
        setenv("AIEYES_AUTO_SCAN", "off", 1)
        let engine = EngineClient(configurationOnly: true)
        defer { engine.stop() }
        var session = LiveSession(id: "focus-check", source: "fixture", phase: .working, updatedAt: Date())
        var expectedApp: NSRunningApplication?
        if let id = option("--thread") {
            session.provider = "chatgpt-work"; session.nativeSession = id
            precondition(session.threadURL != nil, "Pass a native thread UUID")
        } else {
            guard let value = option("--pid"), let pid = Int32(value), pid > 0 else { preconditionFailure("Pass a positive --pid") }
            let target: SessionFocus.Target = try await engine.call("sessions.focusTarget", params: ["pid": Int(pid)])
            precondition(target.status == "found", "Target process has exited")
            session.pid = pid; session.processStart = target.ancestors?.first?.start
            session.workspace = option("--workspace")
            expectedApp = SessionFocus.application(for: (target.ancestors ?? []).map(\.pid))
            print("Core target: pid=\(pid), tty=\(target.tty ?? "none"), GUI ancestor=\(expectedApp?.bundleIdentifier ?? "none")")
            if let tty = option("--tty") { precondition(target.tty == tty, "Unexpected controlling terminal") }
        }
        let previous = NSWorkspace.shared.frontmostApplication
        defer { if args.contains("--restore") { _ = previous?.activate() } }
        // Runs exactly the production focus pipeline; no test ever requests Automation permission.
        let outcome = await SessionFocus.focus(session, engine: engine, requestAutomation: false)
        let kind: String
        switch outcome {
        case .session: kind = "session"
        case .pane: kind = "pane"
        case .tab: kind = "tab"
        case .window: kind = "window"
        case .application: kind = "application"
        case .automationRequired: kind = "automationRequired"
        case .noWindow: kind = "noWindow"
        case .exited: kind = "exited"
        case .failed: kind = "failed"
        }
        if let expected = option("--expect") { precondition(kind == expected, "Expected \(expected), received \(kind)") }
        if ["pane", "tab", "window", "application", "automationRequired", "session"].contains(kind) {
            try await Task.sleep(for: .milliseconds(250))
            let front = NSWorkspace.shared.frontmostApplication
            if kind == "session" { precondition(front?.bundleIdentifier == "com.openai.codex") }
            else { precondition(front?.processIdentifier == expectedApp?.processIdentifier, "Target application did not reach the foreground") }
        }
        print("Production focus result: \(kind) — \(SessionFocus.message(outcome))")
    }
}
