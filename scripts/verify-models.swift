import Foundation

@main struct VerifyModels {
    static func main() throws {
        let automatic = try JSONDecoder().decode(AgentSource.self, from: Data(#"{"id":"s","name":"Shared","provider":"codex","path":"/fixture","accountId":"","enabled":true,"quotaCommand":"","quotaPreCommand":"","codexBinary":"codex","autoValuation":{"enabled":true,"consentVersion":1}}"#.utf8))
        precondition(automatic.autoValuation?.enabled == true && automatic.autoValuation?.consentVersion == 1)
        let roundtrip = try JSONDecoder().decode(AgentSource.self, from: JSONEncoder().encode(automatic))
        precondition(roundtrip.autoValuation == automatic.autoValuation)
        let legacySource = try JSONDecoder().decode(AgentSource.self, from: Data(#"{"id":"old","name":"Old","provider":"codex","path":"/fixture","accountId":"","enabled":true,"quotaCommand":"","quotaPreCommand":"","codexBinary":"codex"}"#.utf8))
        precondition(legacySource.autoValuation == nil)
        let legacy = try JSONDecoder().decode(AppearanceSettings.self, from: Data(#"{"theme":"dark","accent":"teal"}"#.utf8))
        precondition(legacy.chartPalette == "classic" && legacy.chartContrast == "high")
        let legacyRoundtrip = try JSONDecoder().decode(AppearanceSettings.self, from: JSONEncoder().encode(legacy))
        precondition(legacyRoundtrip == legacy)
        let deviceRows = try JSONDecoder().decode([AccountDeviceStatus].self, from: Data(#"[{"accountKey":"codex:a","sourceId":"one","machineId":"local","current":true,"credential":true},{"accountKey":"codex:a","sourceId":"two","machineId":"local","current":false,"credential":true},{"accountKey":"codex:a","sourceId":"remote","machineId":"server","current":false,"credential":true},{"accountKey":"codex:b","sourceId":"other","machineId":"offline","current":true,"credential":true}]"#.utf8))
        let counts = AccountDeviceCounts(accountKey: "codex:a", machines: ["local", "local", "server", "offline"], rows: deviceRows)
        precondition(counts.logged == 2 && counts.using == 1 && counts.total == 3 && counts.pending)
        precondition(counts.summary == "已登录设备2/3 · 使用中设备1/3 · 有设备待确认")
        let paused = AccountDeviceCounts(accountKey: "codex:a", machines: ["local"], rows: deviceRows)
        precondition(paused.summary == "已登录设备1/1 · 使用中设备1/1")
        precondition(AccountDeviceCounts(accountKey: "codex:a", machines: [], rows: deviceRows).summary == "已登录设备0/0 · 使用中设备0/0")
        let data = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))
        let dashboard = try JSONDecoder().decode(Dashboard.self, from: data)
        precondition(dashboard.trendDays.count == 7)
        precondition(dashboard.trendDays.reduce(0) { $0 + $1.total } == dashboard.dayModels.reduce(0) { $0 + $1.usage.total })
        precondition(dashboard.pricingGaps.reduce(0) { $0 + $1.unpricedTokens } == dashboard.summary.total - dashboard.summary.pricedTokens)
        precondition(dashboard.heatmap.count == 365)
        precondition(dashboard.summary.total == dashboard.models.reduce(0) { $0 + $1.total })
        let t = dashboard.summary.tokens
        precondition(dashboard.summary.total == t.input + t.output + t.cacheRead + t.cacheWrite)
        if CommandLine.arguments.count > 2 {
            let settings = try JSONDecoder().decode(Settings.self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2])))
            precondition(settings.version >= 6)
            for source in settings.sources where !source.accountId.isEmpty {
                precondition(settings.accounts.contains { $0.id == source.accountId && $0.provider == source.provider })
            }
        }
        print("Native model decoding and dashboard invariants passed")
    }
}
