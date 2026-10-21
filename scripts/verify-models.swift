import Foundation

@main struct VerifyModels {
    static func main() throws {
        let native = try JSONDecoder().decode(NativeConnection.self, from: Data(#"{"id":"native","provider":"qoder","machineId":"local","root":"/native","configPath":"/native/auth.json","selector":"","kind":"login","email":"","identityKey":"qoder:aaaaaaaaaaaa0123456789","baseUrl":"","models":[],"hasCredential":true,"status":"pending","note":"待验证","checkedAt":1}"#.utf8))
        precondition(native.statusLabel == "待验证" && native.displayName == "Qoder · aaaaaaaaaaaa")
        let qoderIdentity = try JSONDecoder().decode(AccountIdentity.self, from: Data(#"{"key":"qoder:aaaaaaaaaaaa0123456789","email":"","subscription":"personal_standard"}"#.utf8))
        precondition(qoderIdentity.displayName == "Qoder · aaaaaaaaaaaa")
        precondition(qoderIdentity.summary == "Qoder · aaaaaaaaaaaa · personal_standard")
        let qoderStatus = try JSONDecoder().decode(AccountDeviceStatus.self, from: Data(#"{"accountKey":"qoder:local","sourceId":"remote","machineId":"144","current":false,"credential":false,"authenticated":true,"identityConfirmed":true,"identity":{"key":"qoder:bbbbbbbbbbbb0123456789","email":"","subscription":"personal_standard"}}"#.utf8))
        precondition(qoderStatus.identitySummary?.hasPrefix("身份不一致 · 当前登录 Qoder · bbbbbbbbbbbb") == true)
        precondition(qoderStatus.machineIdentitySummary == "当前登录 Qoder · bbbbbbbbbbbb · personal_standard")
        var directory = AgentSource(provider: "qoder", path: "~/.qoder")
        precondition(directory.isDirectorySource)
        directory.path = "  "
        precondition(!directory.isDirectorySource)
        directory.path = "~/.qoder"
        directory.subscription = try JSONDecoder().decode(SubscriptionConfig.self, from: Data(#"{"mode":"cli","credentialRef":"","workspaceId":""}"#.utf8))
        precondition(!directory.isDirectorySource, "Quota connections must not appear as history directories")
        let quotaJSON = #"{"sourceId":"kimi","accountId":"wallet","provider":"kimi","name":"Kimi","updatedAt":1,"origin":"live","windows":[],"extraUsage":{"currency":"USD","total":"90071992.54740993","balance":"0.00000001","monthlyLimitEnabled":true,"monthlyLimit":"0.00","monthlyUsed":"0.00"}}"#
        let walletQuota = try JSONDecoder().decode(Quota.self, from: Data(quotaJSON.utf8))
        precondition(walletQuota.extraUsage?.amount(walletQuota.extraUsage?.balance) == "USD 0.00000001")
        precondition(walletQuota.extraUsage?.total == "90071992.54740993")
        precondition(walletQuota.extraUsage?.monthlyLimitLabel == "月度上限 USD 0.00")
        var oldQuota = try JSONSerialization.jsonObject(with: Data(quotaJSON.utf8)) as! [String: Any]
        oldQuota.removeValue(forKey: "extraUsage")
        let legacyQuota = try JSONDecoder().decode(Quota.self, from: JSONSerialization.data(withJSONObject: oldQuota))
        precondition(legacyQuota.extraUsage == nil)
        let missingWallet = try JSONDecoder().decode(ExtraUsageWallet.self, from: Data(#"{"currency":"USD","total":"12.34"}"#.utf8))
        precondition(missingWallet.amount(missingWallet.balance) == "未提供" && missingWallet.monthlyLimitLabel == "月度上限状态未提供")
        let creditUsage = try JSONDecoder().decode(Aggregate.self, from: Data(#"{"key":"qfmodel","tokens":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"total":0,"cost":0,"pricedTokens":0,"nativeCredits":{"qoder":{"credits":0.25,"originalCredits":0.5,"originalCreditsRequests":1,"billable":0,"billableRequests":0,"nonBillable":0.25,"nonBillableRequests":1,"unknownBillability":0,"unknownBillabilityRequests":0,"requests":1}}}"#.utf8))
        precondition(creditUsage.hasNativeCredits && !creditUsage.costAvailable)
        precondition(!creditUsage.displayCredits.hasNativeCredits)
        precondition(creditUsage.nativeCreditText.isEmpty && creditUsage.nativeCredits?["qoder"]?.detail.contains("标记不计费") == true)
        let codebuddyUsage = try JSONDecoder().decode(Aggregate.self, from: Data(#"{"key":"fixture","tokens":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"total":0,"cost":0,"pricedTokens":0,"nativeCredits":{"codebuddy":{"credits":0.25,"billable":0,"nonBillable":0,"unknownBillability":0.25,"requests":1}}}"#.utf8))
        precondition(codebuddyUsage.hasNativeCredits && !codebuddyUsage.costAvailable)
        precondition(codebuddyUsage.displayCredits.hasNativeCredits)
        precondition(codebuddyUsage.nativeCreditText.contains("1 条计量"))
        let legacyUsage = try JSONDecoder().decode(Aggregate.self, from: Data(#"{"key":"legacy","tokens":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"total":0,"cost":0,"pricedTokens":0}"#.utf8))
        precondition(!legacyUsage.hasNativeCredits && legacyUsage.costAvailable)
        let knownError = UserFacingError("请先连接订阅账户与运行机器")
        precondition(knownError.summary == "请先连接订阅账户与运行机器" && knownError.details == nil)
        let jsonCookie = UserFacingError(#"{"cookie":"session=private-json-cookie; other=private-json-secondary"}"#)
        precondition(jsonCookie.details?.contains("private-json-cookie") == false && jsonCookie.details?.contains("private-json-secondary") == false)
        let rawError = "HTTP 500\nAuthorization: Bearer private-token\nCookie: session=private-cookie; other=secret\n{\"api_key\":\"private-key\",\"refresh_token\":\"private-refresh\"}\nhttps://user:private-password@example.invalid/?access_token=private-query&retry=1\nsk-proj-privatekey1234"
        let safeError = UserFacingError(rawError, context: "额度查询失败")
        precondition(safeError.summary == "额度查询失败")
        for secret in ["private-token", "private-cookie", "secret", "private-key", "private-refresh", "private-password", "private-query", "privatekey1234"] { precondition(safeError.details?.contains(secret) == false) }
        precondition(safeError.details?.contains("HTTP 500") == true && safeError.details?.contains("retry=1") == true)
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
        precondition(counts.summary == "已登录设备 2/3 · 使用中设备 1/3 · 有设备待确认")
        let paused = AccountDeviceCounts(accountKey: "codex:a", machines: ["local"], rows: deviceRows)
        precondition(paused.summary == "已登录设备 1/1 · 使用中设备 1/1")
        precondition(AccountDeviceCounts(accountKey: "codex:a", machines: [], rows: deviceRows).summary == "已登录设备 0/0 · 使用中设备 0/0")
        let data = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))
        let dashboard = try JSONDecoder().decode(Dashboard.self, from: data)
        var labeledDashboard = dashboard
        labeledDashboard.modelDisplayNames = ["qfmodel": "Qwen3.8-Flash", "dfmodel": "DeepSeek-Flash"]
        precondition(labeledDashboard.modelName("qfmodel") == "Qwen3.8-Flash")
        precondition(labeledDashboard.modelName("dfmodel") == "DeepSeek-Flash")
        precondition(dashboard.modelName("legacy") == "legacy")
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
