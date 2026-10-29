import Foundation
@testable import Aieyes
typealias Host = Aieyes.Host

@main struct VerifySelection {
    static func main() throws {
        let wakeups = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: "scripts/fixtures/wakeup-times.json"))) as! [[String: Any]]
        let iso = ISO8601DateFormatter()
        for row in wakeups {
            let result = WakeupTime.next(row["time"] as! String, zone: row["zone"] as? String, now: iso.date(from: row["now"] as! String)!)
            let expected = (row["expected"] as? String).flatMap { iso.date(from: $0) }
            precondition(result.date == expected && result.status.rawValue == row["status"] as! String && result.approximate == row["approximate"] as! Bool, row["name"] as! String)
            if result.approximate { precondition(result.label.contains("按当前偏移估算")) }
        }
        let availability = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: "scripts/fixtures/source-availability.json"))) as! [String: Any]
        for row in availability["cases"] as! [[String: Any]] {
            var settings = Settings()
            settings.sources = (availability["sources"] as! [[String: Any]]).map { raw in
                var source = AgentSource(); source.id = raw["id"] as! String
                source.provider = raw["provider"] as! String; source.enabled = raw["enabled"] as! Bool
                source.hostId = raw["hostId"] as? String; return source
            }
            settings.agents = try JSONDecoder().decode([AgentConfiguration].self, from: JSONSerialization.data(withJSONObject: row["agents"]!))
            settings.hosts = (row["hosts"] as! [[String: Any]]).map { raw in
                var host = Host(); host.id = raw["id"] as! String; host.enabled = raw["enabled"] as! Bool; return host
            }
            precondition(settings.sources.filter { settings.sourceEnabled($0) }.map(\.id) == row["expectedEnabled"] as! [String], row["name"] as! String)
            precondition(settings.usageSources.map(\.id) == row["expectedUsage"] as! [String], row["name"] as! String)
        }
        let fixture = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: "scripts/fixtures/monitor-selection.json"))) as! [String: Any]
        for row in fixture["filesystems"] as! [[String: Any]] {
            precondition(MonitorSelection.recommendedFilesystem(row["id"] as! String, type: row["type"] as? String) == row["expected"] as! Bool)
        }
        for row in fixture["devices"] as! [[String: Any]] {
            var host = Host(); host.devices = row["tokens"] as! [String]
            let devices = try JSONDecoder().decode([DeviceMetric].self, from: JSONSerialization.data(withJSONObject: row["rows"]!))
            precondition(host.selectedDevices(row["group"] as! String, devices).map(\.id) == row["expected"] as! [String], row["name"] as! String)
        }
        let suite = "aieyes-filter-test-" + UUID().uuidString, defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        precondition(DashboardFilterPreference.read(defaults: defaults) == DashboardFilterPreference())
        let hidden = PanelAccountPreference.encode(["codex": []])
        defaults.set(hidden, forKey: NativePanelKey.accounts.rawValue)
        var newAccount = AgentAccount(id: "new", name: "New", provider: "codex", quotaEnabled: false)
        PanelAccountPreference.includeNew(newAccount, accounts: [newAccount], defaults: defaults)
        precondition(defaults.string(forKey: NativePanelKey.accounts.rawValue) == hidden, "An account without quota must not change a custom panel choice")
        newAccount.quotaEnabled = true
        PanelAccountPreference.includeNew(newAccount, accounts: [newAccount], defaults: defaults)
        let included = defaults.string(forKey: NativePanelKey.accounts.rawValue)!
        precondition(PanelAccountPreference.selections(included, accounts: [newAccount], order: [])["codex"] == ["codex:new"], "New accounts enter even an explicit empty panel choice")
        for provider in CoreContract.providers {
            defaults.removeObject(forKey: NativePanelKey.knownAccounts.rawValue)
            let visible = AgentAccount(id: "visible", name: "Visible", provider: provider, quotaEnabled: true)
            let hiddenOld = AgentAccount(id: "hidden", name: "Hidden", provider: provider, quotaEnabled: true)
            let added = AgentAccount(id: "added", name: "Added", provider: provider, quotaEnabled: true)
            defaults.set(PanelAccountPreference.encode([provider: [visible.key]]), forKey: NativePanelKey.accounts.rawValue)
            PanelAccountPreference.observeAccounts([visible, hiddenOld], defaults: defaults)
            PanelAccountPreference.observeAccounts([visible, hiddenOld, added], defaults: defaults)
            let included = defaults.string(forKey: NativePanelKey.accounts.rawValue)!
            precondition(PanelAccountPreference.selections(included, accounts: [visible, hiddenOld, added], order: [])[provider] == [visible.key, added.key], provider)
            defaults.set(PanelAccountPreference.encode([provider: [visible.key]]), forKey: NativePanelKey.accounts.rawValue)
            PanelAccountPreference.observeAccounts([visible, hiddenOld, added], defaults: defaults)
            let kept = defaults.string(forKey: NativePanelKey.accounts.rawValue)!
            precondition(PanelAccountPreference.selections(kept, accounts: [visible, hiddenOld, added], order: [])[provider] == [visible.key], "Repeated loads must preserve a user hiding the new account")
        }
        defaults.removeObject(forKey: NativePanelKey.accounts.rawValue)
        defaults.removeObject(forKey: NativePanelKey.knownAccounts.rawValue)
        let five = (0..<5).map { AgentAccount(id: "auto-\($0)", name: "Auto", provider: "codex", quotaEnabled: true) }
        PanelAccountPreference.observeAccounts(five, defaults: defaults)
        let sixth = AgentAccount(id: "sixth", name: "Sixth", provider: "codex", quotaEnabled: true)
        PanelAccountPreference.observeAccounts(five + [sixth], defaults: defaults)
        let six = defaults.string(forKey: NativePanelKey.accounts.rawValue)!
        precondition(PanelAccountPreference.selections(six, accounts: five + [sixth], order: [])["codex"]?.count == 6)
        let saved = DashboardFilterPreference(provider: "claude", source: "remote", account: "claude:a", model: "model", days: 30)
        saved.save(defaults: defaults)
        precondition(DashboardFilterPreference.read(defaults: defaults) == DashboardFilterPreference(provider: "claude", source: "remote", model: "model", days: 30), "Legacy account filters clear on save")
        DashboardFilterPreference(provider: "unknown", days: 0).save(defaults: defaults)
        precondition(DashboardFilterPreference.read(defaults: defaults) == DashboardFilterPreference())
        defaults.set(true, forKey: NativePanelKey.expansion(.server, id: "host", compact: false))
        defaults.set(false, forKey: NativePanelKey.expansion(.quota, id: "account", compact: true))
        let overrides = NativePanelKey.snapshotOverrides(defaults: defaults)
        precondition(NativePanelKey.allCases.allSatisfy { overrides[$0.rawValue] != nil })
        precondition(overrides[NativePanelKey.expansion(.server, id: "host", compact: false)] as? Bool == false)
        precondition(overrides[NativePanelKey.expansion(.quota, id: "account", compact: true)] as? Bool == true)
        let legacy: [String: Any] = ["accountAliases":["agy:a":"antigravity:legacy-a"], "accounts":[["id":"a","provider":"agy","quotaEnabled":true]], "quotaOrder":["agy:a"]]
        let normalized = ProviderBoundary.normalize(legacy) as! [String: Any]
        let account = (normalized["accounts"] as! [[String: Any]])[0]
        precondition(account["provider"] as? String == "antigravity" && account["id"] as? String == "legacy-a")
        precondition(normalized["quotaOrder"] as? [String] == ["antigravity:legacy-a"])
        let files: Set<String> = ["/", "/data", "/snap/x/1", "/run", "/boot/efi"]
        precondition(MonitorSelection.selected([], group: "filesystems", available: files) == ["/", "/data"])
        let explicit = MonitorSelection.write([], group: "filesystems", selected: files, all: true)
        precondition(explicit == ["filesystems:__all__"])
        precondition(MonitorSelection.selected(explicit, group: "filesystems", available: files) == files)
        let all: Set<String> = ["eth0", "eth1"]
        precondition(MonitorSelection.selected([], group: "network", available: all) == all)
        let cleared = MonitorSelection.write(["gpu:0"], group: "network", selected: [], all: false)
        precondition(cleared == ["gpu:0", "network:__none__"])
        precondition(MonitorSelection.selected(cleared, group: "network", available: all).isEmpty)
        let reset = MonitorSelection.write(cleared, group: "network", selected: all, all: true)
        precondition(reset == ["gpu:0"])
        precondition(MonitorSelection.selected(reset, group: "network", available: all.union(["eth2"])).count == 3)
        let selection: Set<String> = ["eth0", "ib0", "missing"]
        let inverted = MonitorSelection.apply(selection, targets: all, action: .invert)
        precondition(inverted == ["eth1", "ib0", "missing"])
        precondition(MonitorSelection.apply(inverted, targets: all, action: .invert) == selection)
        precondition(MonitorSelection.apply(selection, targets: all, action: .clear) == ["ib0", "missing"])
        precondition(MonitorSelection.deviceIDs(["cpu:cpu", "cpu:cpu0", "cpu:__none__"], group: "cpu") == ["cpu0"])
        precondition(MonitorSelection.parse(" gpu:0, , gpu:0, network:eth0 ") == ["gpu:0", "network:eth0"])
        let cores = Set((0..<512).map { "cpu\($0)" })
        let even = Set(stride(from: 0, to: 512, by: 2).map { "cpu\($0)" })
        precondition(MonitorSelection.apply(cores, targets: even, action: .invert).count == 256)
        print("Native monitor selection invariants passed")
    }
}
