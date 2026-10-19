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
}
