import Foundation

@main struct VerifyDashboardCache {
    static func counts(_ directory: URL) throws -> [String: Int] {
        var result: [String: Int] = [:]
        for path in try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil) where path.pathExtension == "jsonl" {
            for line in try String(contentsOf: path, encoding: .utf8).split(separator: "\n") {
                guard let value = try JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any], let method = value["method"] as? String else { continue }
                result[method, default: 0] += 1
                precondition(value["error"] as? Bool != true, "Unexpected IPC failure: \(method)")
            }
        }
        return result
    }
    static func delta(_ after: [String: Int], _ before: [String: Int]) -> [String: Int] {
        Dictionary(uniqueKeysWithValues: ["dashboard", "dashboard.summary", "dashboard.status"].map { ($0, after[$0, default: 0] - before[$0, default: 0]) })
    }
    @MainActor static func reload(_ model: AppModel) async {
        let loaded = await model.reload()
        precondition(loaded, model.dashboardError ?? "Native dashboard did not load")
    }
    @MainActor static func main() async throws {
        precondition(ProcessInfo.processInfo.environment["AIEYES_RUNTIME_CACHE_FIXTURE"] == "1", "Run through test-dashboard-cache.py with disposable data")
        let logs = URL(fileURLWithPath: ProcessInfo.processInfo.environment["AIEYES_AUDIT_LOG_DIR"]!)
        let model = AppModel(autostart: false)
        defer { model.stop() }
        model.settings = try await model.engine.call("settings.get")
        model.settingsDraft = model.settings
        model.settingsLoaded = true
        model.range = 30
        await reload(model)
        try await Task.sleep(for: .seconds(1))
        await reload(model)
        let total = model.dashboard.summary.total
        precondition(total > 0, "The native model must read the synthetic usage dataset")
        let initial = try counts(logs)
        for _ in 0..<5 { await reload(model) }
        let unchanged = try counts(logs)
        precondition(delta(unchanged, initial).values.allSatisfy { $0 == 0 }, "Unchanged native reloads must reuse all snapshots")
        let keys = model.settings.accounts.map(\.key)
        precondition(keys.count > 1)
        for index in 0..<5 {
            let order = index.isMultiple(of: 2) ? Array(keys.reversed()) : keys
            let saved: [String] = try await model.engine.call("quotas.order.set", params: ["keys": order])
            await reload(model)
            precondition(model.dashboard.quotaOrder == saved && model.menuDashboard.quotaOrder == saved)
            precondition(model.dashboard.summary.total == total, "Status merges must retain usage")
        }
        let quota = try counts(logs)
        let quotaDelta = delta(quota, unchanged)
        precondition(quotaDelta["dashboard"] == 0 && quotaDelta["dashboard.summary"] == 0 && quotaDelta["dashboard.status"] == 5, "Five quota revisions should merge five shared status responses")
        let _: Acknowledgement = try await model.engine.call("prices.save", params: ["prices": [["id": "fixture-0", "input": 0.000001, "output": 0.000003]]])
        await reload(model)
        let prices = delta(try counts(logs), quota)
        precondition(prices["dashboard", default: 0] > 0 && prices["dashboard.summary", default: 0] > 0, "Price changes must invalidate usage snapshots")
        let report: [String: Any] = ["unchanged": delta(unchanged, initial), "quotaOnly": quotaDelta, "prices": prices, "usageTokens": total, "passed": true]
        print(String(data: try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]), encoding: .utf8)!)
    }
}
