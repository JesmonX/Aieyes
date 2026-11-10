// Manual desktop check (changes the frontmost app): `verify-session-focus <ancestor pids, nearest first>`.
// The pids come from `aieyes-core --call sessions.focusTarget '{"pid":<agent pid>}'`.
import AppKit
@testable import Aieyes

@main struct VerifySessionFocus {
    @MainActor static func main() async throws {
        let pids = CommandLine.arguments.dropFirst().compactMap { Int32($0) }
        precondition(!pids.isEmpty, "Pass ancestor pids")
        // Non-application processes (shell, login) are skipped; a helper without a Dock icon is never chosen.
        guard let app = SessionFocus.application(for: pids) else { print("No regular application among \(pids)"); return }
        let windows = SessionFocus.windowCount(app.processIdentifier)
        precondition(app.activate(), "Activation was refused")
        try await Task.sleep(for: .milliseconds(800))
        let front = NSWorkspace.shared.frontmostApplication
        precondition(front?.processIdentifier == app.processIdentifier, "Frontmost is \(front?.localizedName ?? "none"), not \(app.localizedName ?? "?")")
        print("Activated \(app.localizedName ?? "?") (pid \(app.processIdentifier)); on-screen windows: \(windows) → \(windows == 1 ? "exact window" : "application only")")
    }
}
