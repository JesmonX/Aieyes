import Foundation
@testable import Aieyes

/// `qoderCli.status` from the real core decodes natively, and the install prompt
/// hides once a Qoder account exists (archived accounts do not count).
@main struct VerifyQoderCLI {
    static func main() throws {
        let arguments = CommandLine.arguments
        let provider = arguments.count > 3 ? arguments[3] : "qoder"
        let binary = provider == "qoder-cn" ? "qoderclicn" : "qodercli"
        let status = try JSONDecoder().decode(QoderCLIStatus.self, from: Data(contentsOf: URL(fileURLWithPath: arguments[1])))
        precondition(status.cli ? status.command == binary + " login" : status.command.hasSuffix("~/.local/bin/" + binary + " login"))
        precondition(!status.account && !status.message.isEmpty && status.offerInstall == !status.apps.isEmpty)
        var settings = try JSONDecoder().decode(Settings.self, from: Data(contentsOf: URL(fileURLWithPath: arguments[2])))
        precondition(!QoderCLIStatus.signedIn(settings, provider: provider))
        settings.accounts.append(AgentAccount(id: "q", name: "Qoder", provider: provider))
        precondition(QoderCLIStatus.signedIn(settings, provider: provider))
        precondition(!QoderCLIStatus.signedIn(settings, provider: provider == "qoder-cn" ? "qoder" : "qoder-cn"))
        settings.accounts[settings.accounts.count - 1].archived = true
        precondition(!QoderCLIStatus.signedIn(settings, provider: provider))
        print("Qoder CLI prompt: core status decodes natively and the prompt hides after sign-in")
    }
}
