import Foundation

@main struct VerifySelection {
    static func main() {
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
