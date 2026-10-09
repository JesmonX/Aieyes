import Foundation

@main struct VerifyServerFormat {
    static func main() throws {
        for (seconds, text): (Double, String) in [(0, "0 分"), (59, "0 分"), (60, "1 分"), (3600, "1 小时 0 分"), (90061, "1 天 1 小时 1 分")] {
            precondition(Format.uptime(seconds) == text)
        }
        for seconds: Double? in [nil, .nan, .infinity, -1] { precondition(Format.uptime(seconds) == "—") }
        precondition(Format.capacityPercent(0, 1024) == 0)
        precondition(Format.capacityPercent(1024, 2048) == 50)
        precondition(Format.capacityPercent(nil, 2048) == nil)
        precondition(Format.capacityPercent(0, 0) == nil)
        precondition(Format.usedCapacity(100, nil) == nil)
        precondition(Format.bytes(.nan) == "—")
        precondition(Format.bytes(0) == "0 B")
        var host = Host()
        precondition(host.shows("uptime"))
        host.details = ["gpuMemory"]
        let decoder = JSONDecoder()
        let restored = try decoder.decode(Host.self, from: JSONEncoder().encode(host))
        precondition(!restored.shows("uptime") && restored.shows("gpuMemory"))
        host.details = []
        let empty = try decoder.decode(Host.self, from: JSONEncoder().encode(host))
        precondition(!empty.shows("uptime"))
        var legacy = try JSONSerialization.jsonObject(with: JSONEncoder().encode(host)) as! [String: Any]
        legacy.removeValue(forKey: "details")
        let defaulted = try decoder.decode(Host.self, from: JSONSerialization.data(withJSONObject: legacy))
        precondition(defaulted.shows("uptime"))
        let gpus = [DeviceMetric(id: "0", name: nil, device: nil, type: nil, memoryUsedMiB: 0, memoryTotalMiB: 8192), DeviceMetric(id: "1", name: nil, device: nil, type: nil, memoryUsedMiB: 24576, memoryTotalMiB: 49152)]
        precondition(host.selectedDevices("gpu", gpus).count == 2)
        host.devices = ["gpu:1", "network:eth1", "cpu:__none__"]
        precondition(host.selectedDevices("gpu", gpus).map(\.id) == ["1"])
        precondition(host.selectedDevices("cpu", [DeviceMetric(id: "cpu", name: nil, device: nil, type: nil), DeviceMetric(id: "cpu0", name: nil, device: nil, type: nil)]).map(\.id) == ["cpu"])
        precondition(host.selectedDevices("network", [DeviceMetric(id: "eth0", name: nil, device: nil, type: nil), DeviceMetric(id: "eth1", name: nil, device: nil, type: nil)]).map(\.id) == ["eth1"])
        host.devices = ["gpu:__none__"]
        precondition(host.selectedDevices("gpu", gpus).isEmpty)
        host.metrics = []
        precondition(host.selectedDevices("network", [DeviceMetric(id: "eth0", name: nil, device: nil, type: nil)]).isEmpty)
        let missing = try decoder.decode(MetricSample.self, from: Data(#"{"timestamp":1,"load":[],"errors":{},"memory":{"total":1024}}"#.utf8))
        precondition(missing.memory?.used == nil && missing.uptime == nil)
        print("Native server uptime, capacities, missing readings, device filters and legacy configuration passed")
    }
}
