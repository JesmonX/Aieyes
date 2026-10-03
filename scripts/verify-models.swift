import Foundation

@main struct VerifyModels {
    static func main() throws {
        let data = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))
        let dashboard = try JSONDecoder().decode(Dashboard.self, from: data)
        precondition(dashboard.days.count == 1)
        precondition(dashboard.trendDays.count == 7)
        precondition(dashboard.trendDays.reduce(0) { $0 + $1.total } == dashboard.dayModels.reduce(0) { $0 + $1.usage.total })
        precondition(dashboard.pricingGaps.reduce(0) { $0 + $1.unpricedTokens } == dashboard.summary.total - dashboard.summary.pricedTokens)
        precondition(dashboard.heatmap.count == 365)
        precondition(dashboard.summary.total == dashboard.models.reduce(0) { $0 + $1.total })
        precondition(dashboard.summary.total == dashboard.days.reduce(0) { $0 + $1.total })
        let t = dashboard.summary.tokens
        precondition(dashboard.summary.total == t.input + t.output + t.cacheRead + t.cacheWrite)
        if CommandLine.arguments.count > 2 {
            let settings = try JSONDecoder().decode(Settings.self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2])))
            precondition(settings.version == 2)
            for source in settings.sources where !source.accountId.isEmpty {
                precondition(settings.accounts.contains { $0.id == source.accountId && $0.provider == source.provider })
            }
        }
        print("Native model decoding and dashboard invariants passed")
    }
}
