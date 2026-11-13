import AppKit
import Carbon

/// Resolve a native conversation, terminal surface, workspace window, then the owning app.
/// Automation is requested only after an explicit click; polling never asks for permissions.
@MainActor enum SessionFocus {
    static let engine = EngineClient(configurationOnly: true)
    struct Target: Decodable { var status: String; var ancestors: [Ancestor]?; var tty: String? }
    struct Ancestor: Decodable { var pid: Int32; var start: Double? }
    enum Outcome: Equatable {
        case session(String), pane(String), tab(String), window(String), application(String)
        case automationRequired(String), noWindow, exited, failed(String)
    }
    static func application(for ancestors: [Int32], lookup: (Int32) -> NSRunningApplication? = { NSRunningApplication(processIdentifier: $0) }) -> NSRunningApplication? {
        ancestors.lazy.compactMap(lookup).first { $0.activationPolicy == .regular && !$0.isTerminated }
    }
    /// Include minimized windows and other Spaces. One on-screen window does not prove uniqueness.
    static func windowCount(_ pid: Int32) -> Int {
        let rows = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
        return rows.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }.count
    }
    private static func activate(_ app: NSRunningApplication) async -> Bool {
        guard app.activate() else { return false }
        for _ in 0..<20 {
            if NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier { return true }
            try? await Task.sleep(for: .milliseconds(25))
        }
        return false
    }
    private static func openThread(_ url: URL, in appURL: URL) async -> Outcome {
        do {
            let app = try await NSWorkspace.shared.open([url], withApplicationAt: appURL, configuration: .init())
            return .session(app.localizedName ?? "Codex")
        } catch { return .failed("Codex") }
    }
    static func focus(_ session: LiveSession, engine: EngineClient, requestAutomation: Bool = true) async -> Outcome {
        guard session.machineId == "local" else { return .noWindow }
        if let url = session.threadURL, let appURL = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.openai.codex") {
            return await openThread(url, in: appURL)
        }
        guard let pid = session.pid, pid > 0 else { return .noWindow }
        guard let target: Target = try? await engine.call("sessions.focusTarget", params: ["pid": Int(pid), "processStart": session.processStart ?? 0]) else { return .failed("") }
        guard target.status == "found" else { return .exited }
        let observedStart = target.ancestors?.first(where: { $0.pid == pid })?.start ?? session.processStart ?? 0
        guard let app = application(for: (target.ancestors ?? []).map(\.pid)) else { return .noWindow }
        if let start = target.ancestors?.first(where: { $0.pid == app.processIdentifier })?.start,
           let launched = app.launchDate, abs(launched.timeIntervalSince1970 - start) >= 2 { return .exited }
        let name = app.localizedName ?? Format.provider(session.provider)
        if app.bundleIdentifier == "com.openai.codex", ["codex", "chatgpt-work"].contains(session.provider), let id = session.nativeSession,
           UUID(uuidString: id) != nil, let url = URL(string: "codex://threads/" + id), let appURL = app.bundleURL {
            return await openThread(url, in: appURL)
        }
        var needsAutomation = false
        if let bundle = app.bundleIdentifier, let tty = target.tty,
           let source = terminalScript(bundle: bundle, tty: tty) {
            let permission = await Task.detached(priority: .userInitiated) {
                let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
                return AEDeterminePermissionToAutomateTarget(target.aeDesc, typeWildCard, typeWildCard, requestAutomation)
            }.value
            needsAutomation = permission != noErr
            if permission == noErr {
                // A permission dialog may stay open for minutes. The old PID/TTY must not
                // focus a different process that acquired that terminal in the meantime.
                guard let current: Target = try? await engine.call("sessions.focusTarget", params: ["pid": Int(pid), "processStart": observedStart]),
                      current.status == "found", current.tty == tty, !app.isTerminated,
                      current.ancestors?.contains(where: { $0.pid == app.processIdentifier }) == true else { return .exited }
                let result = await Task.detached(priority: .userInitiated) {
                    var error: NSDictionary?
                    let value = NSAppleScript(source: source)?.executeAndReturnError(&error)
                    return error == nil ? value?.stringValue ?? "missing" : "missing"
                }.value
                if ["pane", "tab"].contains(result), await activate(app) {
                    return result == "pane" ? .pane(name) : .tab(name)
                }
            }
        }
        let matchedWindow = focusWorkspaceWindow(pid: app.processIdentifier, workspace: session.workspace)
        guard await activate(app) else { return .failed(name) }
        if needsAutomation { return .automationRequired(name) }
        let confirmedWindow = matchedWindow.map { window in
            var focused: CFTypeRef?
            return AXUIElementCopyAttributeValue(AXUIElementCreateApplication(app.processIdentifier), kAXFocusedWindowAttribute as CFString, &focused) == .success
                && focused.map { CFEqual($0, window) } == true
        } ?? false
        return confirmedWindow || windowCount(app.processIdentifier) == 1 ? .window(name) : .application(name)
    }

    /// Existing Accessibility access can locate IDE windows by their full document path.
    /// Never guess from a project basename, which may refer to another checkout.
    private static func focusWorkspaceWindow(pid: Int32, workspace: String?) -> AXUIElement? {
        guard AXIsProcessTrusted(), let workspace, !workspace.isEmpty else { return nil }
        let app = AXUIElementCreateApplication(pid)
        var raw: CFTypeRef?
        guard AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &raw) == .success,
              let windows = raw as? [AXUIElement] else { return nil }
        let matches = windows.filter { window in
            var document: CFTypeRef?
            guard AXUIElementCopyAttributeValue(window, kAXDocumentAttribute as CFString, &document) == .success,
                  let value = document as? String else { return false }
            return matchesWorkspace(document: value, workspace: workspace)
        }
        guard matches.count == 1, let window = matches.first else { return nil }
        AXUIElementSetAttributeValue(window, kAXMinimizedAttribute as CFString, kCFBooleanFalse)
        AXUIElementSetAttributeValue(window, kAXMainAttribute as CFString, kCFBooleanTrue)
        return AXUIElementPerformAction(window, kAXRaiseAction as CFString) == .success ? window : nil
    }
    nonisolated static func matchesWorkspace(document: String, workspace: String) -> Bool {
        let url = URL(string: document)
        let path = url?.isFileURL == true ? url!.path : document
        guard path.hasPrefix("/"), workspace.hasPrefix("/") else { return false }
        let root = URL(fileURLWithPath: workspace).standardizedFileURL.path
        let target = URL(fileURLWithPath: path).standardizedFileURL.path
        return root != "/" && (target == root || target.hasPrefix(root + "/"))
    }

    /// All scripts inspect identity/selection only. They never read or write terminal contents.
    nonisolated static func terminalScript(bundle: String, tty: String) -> String? {
        guard tty.hasPrefix("/dev/tty") || tty.hasPrefix("/dev/pts/"), tty.count <= 128,
              tty.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || [47, 45, 95].contains($0) }) else { return nil }
        let body: String
        switch bundle {
        case "com.mitchellh.ghostty":
            body = """
            set hits to {}
            repeat with candidate in terminals
                if tty of candidate is targetTTY then set end of hits to contents of candidate
            end repeat
            if (count hits) is not 1 then return "missing"
            set targetTerminal to item 1 of hits
            focus targetTerminal
            if id of focused terminal of selected tab of front window is id of targetTerminal then return "pane"
            """
        case "com.apple.Terminal":
            body = """
            set hitCount to 0
            repeat with w in windows
                repeat with t in tabs of w
                    if tty of t is targetTTY then
                        set hitCount to hitCount + 1
                        set hitWindow to contents of w
                        set hitTab to contents of t
                    end if
                end repeat
            end repeat
            if hitCount is not 1 then return "missing"
            set selected tab of hitWindow to hitTab
            set miniaturized of hitWindow to false
            set index of hitWindow to 1
            activate
            if tty of selected tab of front window is targetTTY then return "tab"
            """
        case "com.googlecode.iterm2":
            body = """
            set hitCount to 0
            repeat with w in windows
                repeat with t in tabs of w
                    repeat with s in sessions of t
                        if tty of s is targetTTY then
                            set hitCount to hitCount + 1
                            set hitWindow to contents of w
                            set hitTab to contents of t
                            set hitSession to contents of s
                        end if
                    end repeat
                end repeat
            end repeat
            if hitCount is not 1 then return "missing"
            select hitSession
            select hitTab
            select hitWindow
            activate
            if tty of current session of current window is targetTTY then return "pane"
            """
        default: return nil
        }
        return """
        with timeout of 3 seconds
            if application id "\(bundle)" is not running then return "missing"
            tell application id "\(bundle)"
                set targetTTY to "\(tty)"
                \(body)
            end tell
        end timeout
        return "missing"
        """
    }
    static func message(_ outcome: Outcome) -> String {
        switch outcome {
        case .session(let name): return L("已打开 {0} 中的会话", name)
        case .pane(let name): return L("已定位到 {0} 的分屏", name)
        case .tab(let name): return L("已定位到 {0} 的标签页", name)
        case .window(let name): return L("已切换到 {0} 的窗口", name)
        case .application(let name): return L("已激活 {0}，未定位到具体窗口", name)
        case .automationRequired(let name): return L("已激活 {0}；允许自动化权限后可精确定位终端", name)
        case .noWindow: return L("未找到所属窗口（可能运行在 tmux、screen 或后台服务中）")
        case .exited: return L("会话进程已退出")
        case .failed(let name): return name.isEmpty ? L("无法读取会话进程") : L("无法切换到 {0}", name)
        }
    }
}
