import Foundation

enum MonitorSelection {
    enum Action: Equatable { case all, clear, invert }

    static func parse(_ text: String) -> [String] {
        var seen = Set<String>()
        return text.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty && seen.insert($0).inserted }
    }

    static func deviceIDs(_ tokens: [String], group: String) -> Set<String> {
        Set(tokens.filter { $0.hasPrefix(group + ":") }.map { String($0.dropFirst(group.count + 1)) }
            .filter { $0 != "__none__" && !(group == "cpu" && $0 == "cpu") })
    }

    static func selected(_ tokens: [String], group: String, available: Set<String>) -> Set<String> {
        tokens.contains { $0.hasPrefix(group + ":") } ? deviceIDs(tokens, group: group) : available
    }

    static func write(_ tokens: [String], group: String, selected: Set<String>, all: Bool) -> [String] {
        let other = tokens.filter { !$0.hasPrefix(group + ":") }
        if all { return other }
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
