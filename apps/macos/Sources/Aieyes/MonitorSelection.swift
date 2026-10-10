import Foundation

enum MonitorSelection {
    static func recommendedFilesystem(_ id: String, type: String? = nil) -> Bool {
        let path = id.lowercased(), kind = (type ?? "").lowercased()
        return !["tmpfs", "devtmpfs", "squashfs", "overlay", "proc", "procfs", "sysfs", "devfs", "autofs", "cgroup", "cgroup2"].contains(kind)
            && !["/snap", "/var/lib/snapd/snap", "/run", "/efi", "/boot/efi", "/dev", "/proc", "/sys", "/system/volumes/preboot", "/system/volumes/vm", "/system/volumes/update", "/system/volumes/xarts", "/system/volumes/iscpreboot", "/system/volumes/hardware"].contains { path == $0 || path.hasPrefix($0 + "/") }
    }
    enum Action: Equatable { case all, clear, invert }

    static func parse(_ text: String) -> [String] {
        var seen = Set<String>()
        return text.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty && seen.insert($0).inserted }
    }

    static func deviceIDs(_ tokens: [String], group: String) -> Set<String> {
        Set(tokens.filter { $0.hasPrefix(group + ":") }.map { String($0.dropFirst(group.count + 1)) }
            .filter { $0 != "__none__" && $0 != "__all__" && !(group == "cpu" && $0 == "cpu") })
    }

    static func selected(_ tokens: [String], group: String, available: Set<String>, recommended: Set<String>? = nil) -> Set<String> {
        if tokens.contains(group + ":__all__") { return available }
        return tokens.contains { $0.hasPrefix(group + ":") } ? deviceIDs(tokens, group: group) : group == "filesystems" ? (recommended ?? available.filter { recommendedFilesystem($0) }) : available
    }

    static func write(_ tokens: [String], group: String, selected: Set<String>, all: Bool) -> [String] {
        let other = tokens.filter { !$0.hasPrefix(group + ":") }
        if all { return other + (group == "filesystems" ? ["filesystems:__all__"] : []) }
        let ids = selected.filter { $0 != "__none__" && !(group == "cpu" && $0 == "cpu") }.sorted()
        return other + (ids.isEmpty ? ["__none__"] : ids).map { group + ":" + $0 }
    }

    static func apply(_ selected: Set<String>, targets: Set<String>, action: Action) -> Set<String> {
        switch action {
        case .all: return selected.union(targets)
        case .clear: return selected.subtracting(targets)
        case .invert: return selected.symmetricDifference(targets)
        }
    }
}
