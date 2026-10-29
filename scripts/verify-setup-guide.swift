import Foundation
@testable import Aieyes
typealias Host = Aieyes.Host

@main struct VerifySetupGuide {
    @MainActor static func main() throws {
        var settings = Settings()
        var first = AgentAccount(id: "first", name: "First", provider: "codex")
        first.quotaRefreshSeconds = 45
        first.deviceSettings = [.init(machineId: "host", preCommand: "")]
        let second = AgentAccount(id: "second", name: "Second", provider: "codex")
        let third = AgentAccount(id: "third", name: "Third", provider: "codex")
        var archived = AgentAccount(id: "archived", name: "Archived", provider: "codex"); archived.archived = true
        settings.accounts = [first, second, third, archived]
        settings.agents = [.init(provider: "codex", enabled: true, machineIds: ["local", "host"])]
        var host = Host(); host.id = "host"; host.name = "Server"; settings.hosts = [host]
        settings.proxy.mode = "direct"
        settings.sources = settings.accounts.map { a in var s = AgentSource(); s.id = "source-" + a.id; s.name = a.name; s.accountId = a.id; return s }
        var remote = settings.sources[0]; remote.id = "remote"; remote.hostId = "host"; settings.sources.append(remote)
        var dashboard = Dashboard()
        dashboard.quotas = settings.accounts.map { a in Quota(sourceId: "source-" + a.id, accountId: a.id, provider: "codex", name: a.name, updatedAt: 100, origin: "live", windows: [], bankReset: nil, bankUpdatedAt: nil, plan: nil, error: nil) }
        dashboard.quotas[1].error = "timeout"; dashboard.quotas[2].origin = "log"
        dashboard.quotaOrder = settings.accounts.map(\.key)
        var records: [WakeRecord]? = []
        func checks(_ scene: String = "mixed", failed: Bool = false) -> SetupGuideChecks {
            .init(settings: settings, dashboard: failed ? nil : dashboard, hosts: [], wakeups: records, errors: failed ? ["dashboard", "wakeups"] : [], scene: scene, panelPreference: "{}")
        }
        let quota = checks().rows("usage").filter { $0.destination.accountKey != nil }
        precondition(quota.count == 3 && quota[0].status == .querySucceeded && quota[0].updatedAt == 100)
        precondition(quota[1].level == "warn" && quota[2].level == "pending", "One success or log-only data cannot verify other accounts")
        precondition(!checks("ssh").rows("usage").contains { $0.status == .querySucceeded }, "A local result cannot verify SSH networking")
        precondition(checks("local").rows("network").allSatisfy { $0.level != "warn" }, "Direct connections do not require a proxy")
        let device = checks().rows("network").first { $0.destination.machineID == "host" }!
        precondition(device.status == .unconfigured && device.details.contains("服务器可直连时无需填写") && device.destination.accountKey == first.key && device.destination.section == "command")
        settings.accounts[0].deviceSettings?[0].preCommand = "PRIVATE COMMAND"
        let saved = checks().rows("network").first { $0.destination.machineID == "host" }!
        precondition(saved.status == .configured && saved.status != .querySucceeded && !(saved.text + saved.details).contains("PRIVATE"), "Saving is not proof of connectivity and commands are not rendered")
        settings.accounts[0].deviceSettings?[0].preCommandResetRequired = true
        precondition(checks().rows("network").contains { $0.level == "warn" && $0.status == .reconfirm })
        precondition(checks().rows("refresh").contains { $0.status == .custom && $0.text.contains("45") })
        precondition(checks().rows("refresh").contains { $0.status == .inherited && $0.text.contains("300") })
        precondition(checks().rows("panel").contains { $0.label == "Archived" && $0.status == .archived })
        precondition(!checks(failed: true).rows("usage").contains { $0.status == .querySucceeded })
        records = nil
        precondition(checks(failed: true).rows("wakeups").first!.status == .reconfirm)
        let task = WakeTask(id: "task", sourceId: "remote")
        records = [WakeRecord(task: task, deployment: nil)]
        precondition(checks().rows("wakeups").first!.status == .draft)
        for (state, expected) in [("settings-pending", SetupGuide.CheckState.pendingSync), ("pending-removal", .pendingRemoval), ("deployment-unconfirmed", .reconfirm), ("unknown", .reconfirm)] {
            records = [WakeRecord(task: task, deployment: .init(task: task, deployedAt: 1, enabled: true, state: state, timezone: "UTC", target: "Server", changed: false))]
            precondition(checks().rows("wakeups").first!.status == expected)
        }
        // Disabled stock providers do not become required setup tasks.
        settings.sources.append(AgentSource(id: "local-claude", provider: "claude"))
        settings.agents?.append(.init(provider: "claude", enabled: false, machineIds: []))
        precondition(!checks().rows("usage").contains { $0.destination.provider == "claude" })
        let model = AppModel(autostart: false); model.settings = settings
        model.openSettings(device.destination)
        precondition(model.requestedAccountKey == first.key && model.requestedAccountMachineID == "host" && model.settingsSection == "command")
        print("Native goal guide: account/scene evidence, direct networking, private commands, refresh overrides, archived panels, deployment states, unknown observations and account-device routing passed")
    }
}
