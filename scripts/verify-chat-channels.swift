// Pure DTO and terminal-script checks. Never requests permission or sends terminal input.
import AppKit
import Foundation
@testable import Aieyes

@main struct VerifyChatChannels {
    @MainActor static func main() throws {
        let data = Data(#"{"enabled":true,"appId":"cli_fixture","hasSecret":true,"revision":2,"paired":false,"ownerOpenId":null,"allowTerminal":true,"connection":{"state":"connected"},"pending":1,"undelivered":2,"deliveryError":{"message":"fixture"},"workerError":null}"#.utf8)
        let status = try JSONDecoder().decode(ChatChannelStatus.self, from: data)
        precondition(status.enabled && status.hasSecret && status.allowTerminal && status.undelivered == 2)
        precondition(status.deliveryError?.message == "fixture" && status.workerError == nil)
        for (state, cancellable) in [("queued", true), ("blocked", true), ("automation", true), ("dispatching", false), ("submitted", false), ("confirmed", false), ("uncertain", false)] {
            let command = ChatChannelCommand(id: "fixture", sessionId: "session", body: "Literal user text", status: state, reason: "")
            precondition(command.canCancel == cancellable)
        }
        for tty in ["", "ttys001", "/dev/ttys001\n", "/dev/ttys001;command", "/dev/tty\"", "/dev/pts/../../shell"] {
            precondition(!SessionChannelAutomation.validTTY(tty))
        }
        precondition(SessionChannelAutomation.validTTY("/dev/ttys999"))
        precondition(SessionChannelAutomation.validTTY("/dev/pts/999"))
        precondition(!SessionChannelAutomation.foreground(pid: -1, tty: "/dev/ttys999"))
        precondition(SessionChannelAutomation.script(bundle: "unsupported") == nil)
        for bundle in SessionChannelAutomation.terminals {
            let script = SessionChannelAutomation.script(bundle: bundle)!
            precondition(script.contains("on deliver(targetTTY, messageText)"))
            precondition(script.contains("(count hits) is not 1"))
            precondition(!script.contains("System Events") && !script.contains("keystroke"))
            if NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) != nil {
                var error: NSDictionary?
                precondition(NSAppleScript(source: script)?.compileAndReturnError(&error) == true, "\(bundle): \(String(describing: error))")
                print("Compiled channel terminal script: \(bundle)")
            }
        }
        precondition(SettingsIndex.generalPageKeys.contains("channels"))
        print("Native chat channels: DTOs, command states, TTY guards, and installed terminal dictionaries passed; no input was sent")
    }
}
