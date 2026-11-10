import Foundation

extension Settings {
    func supports(_ provider: String, _ feature: String) -> Bool {
        if CoreContract.supports(provider, feature) { return true }
        return ["quota", "quotaReset", "valuation"].contains(feature) && sources.contains {
            $0.provider == provider && !$0.quotaCommand.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }
    func agentSupports(_ provider: String, _ feature: String) -> Bool {
        CoreContract.providers.contains { CoreContract.sameAgent($0, provider) && supports($0, feature) }
    }
    static let capabilityColumns = ["history", "quota", "api", "sessions", "wakeup"]
    enum CapabilityState: String { case ready, setup, unsupported, unneeded }
    /// Integration matrix cell, matching the Windows/Linux rules.
    func capabilityState(_ provider: String, _ capability: String, quotas: [Quota] = []) -> CapabilityState {
        let same = { (other: String) in CoreContract.sameAgent(other, provider) }
        let supported = capability == "wakeup" ? CoreContract.wakeupProviders.contains(where: same) : agentSupports(provider, capability)
        guard supported else {
            if capability == "wakeup" {
                let fiveHour = agentSupports(provider, "fiveHourQuota") || quotas.contains { same($0.provider) && $0.windows.contains { $0.windowMinutes == CoreContract.fiveHourMinutes } }
                return fiveHour ? .unsupported : .unneeded
            }
            return .unsupported
        }
        let agents = (self.agents ?? []).filter { same($0.provider) }
        if !agents.isEmpty && !agents.contains(where: \.enabled) { return .setup }
        let enabled = sources.filter { same($0.provider) && sourceEnabled($0) }
        let ready: Bool
        switch capability {
        case "history", "sessions": ready = enabled.contains { $0.isDirectorySource && supports($0.provider, capability) }
        case "quota": ready = accounts.contains { same($0.provider) && $0.archived != true && $0.quotaEnabled } || enabled.contains { !$0.quotaCommand.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        case "api": ready = (provider == "codex" && !(codexApis ?? []).isEmpty) || (nativeConnections ?? []).contains { same($0.provider) && $0.kind == "api" }
        default: ready = enabled.contains { CoreContract.wakeupProviders.contains($0.provider) && (!$0.accountId.isEmpty || $0.codexHomeId != nil) }
        }
        return ready ? .ready : .setup
    }
}
