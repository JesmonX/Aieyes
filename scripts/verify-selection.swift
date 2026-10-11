import Foundation

@main struct VerifySelection {
    static func main() throws {
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
        let saved = DashboardFilterPreference(provider: "claude", source: "remote", account: "claude:a", model: "model", days: 30)
        saved.save(defaults: defaults)
        precondition(DashboardFilterPreference.read(defaults: defaults) == saved)
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
