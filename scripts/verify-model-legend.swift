import AppKit
import SwiftUI
import ApplicationServices
@testable import Aieyes

@MainActor private final class LegendState: ObservableObject {
    @Published var cost = false
    @Published var total: Double = 300
}

private struct LegendFixture: View {
    @ObservedObject var state: LegendState
    var body: some View {
        UsageChart(days: [Aggregate(key: "2026-11-11", total: state.total, cost: 3)], rows: [
            DayModel(day: "2026-11-11", model: "first", usage: Aggregate(total: 100, cost: 1)),
            DayModel(day: "2026-11-11", model: "second", usage: Aggregate(total: state.total - 100, cost: 2))
        ], cost: state.cost, modelNames: ["first": "First model", "second": "Second model"])
            .equatable().padding(20)
    }
}

@MainActor private final class LegendChecks: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        AXUIElementSetAttributeValue(AXUIElementCreateApplication(getpid()), "AXManualAccessibility" as CFString, kCFBooleanTrue)
        Task {
            do {
                try await run()
                print("Native model legend: repeated toggles, independent models, metric switches and data refresh passed")
                NSApp.terminate(nil)
            } catch {
                fputs("Native model legend failed: \(error)\n", stderr)
                exit(1)
            }
        }
    }

    func value(_ node: NSObject, _ name: String) -> AnyObject? {
        let selector = NSSelectorFromString(name)
        return node.responds(to: selector) ? node.perform(selector)?.takeUnretainedValue() : nil
    }

    func nodes(in root: NSObject) -> [NSObject] {
        var visited = Set<ObjectIdentifier>()
        func visit(_ node: NSObject, depth: Int) -> [NSObject] {
            guard depth < 40, visited.insert(ObjectIdentifier(node)).inserted else { return [] }
            let children = ((value(node, "accessibilityChildren") as? [NSObject]) ?? []) + ((node as? NSView)?.subviews ?? [])
            return [node] + children.flatMap { visit($0, depth: depth + 1) }
        }
        return visit(root, depth: 0)
    }

    func button(in root: NSObject, label: String) -> NSObject? {
        nodes(in: root).first {
            value($0, "accessibilityRole") as? String == "AXButton" && value($0, "accessibilityLabel") as? String == label
        }
    }

    func press(_ node: NSObject) throws {
        let selector = NSSelectorFromString("accessibilityPerformPress")
        guard node.responds(to: selector), let implementation = node.method(for: selector) else {
            throw ClientError.message("Expected an accessible press action")
        }
        typealias Action = @convention(c) (AnyObject, Selector) -> Bool
        guard unsafeBitCast(implementation, to: Action.self)(node, selector) else {
            throw ClientError.message("The model legend rejected a press")
        }
    }

    func settle(_ host: NSView) async throws {
        try await Task.sleep(for: .milliseconds(150))
        host.layoutSubtreeIfNeeded()
    }

    func run() async throws {
        // Volatile preferences and a separate test bundle keep the user's app settings intact.
        UserDefaults.standard.setVolatileDomain([
            "aieyes.language": "en", "analysis.modelLegendExpanded": true
        ], forName: UserDefaults.argumentDomain)
        let state = LegendState(), host = NSHostingView(rootView: LegendFixture(state: state))
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 700, height: 460),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFront(nil)
        defer { window.close() }
        try await settle(host)
        func toggle(_ name: String, hidden: Bool) async throws {
            let before = (hidden ? "Hide model " : "Show model ") + name
            let after = (hidden ? "Show model " : "Hide model ") + name
            guard let control = button(in: host, label: before) else {
                let labels = nodes(in: host).compactMap { value($0, "accessibilityLabel") as? String }
                throw ClientError.message("Missing \(before); labels: \(labels)")
            }
            try press(control)
            try await settle(host)
            guard button(in: host, label: after) != nil else {
                throw ClientError.message("\(before) did not update to \(after) without collapsing the legend")
            }
        }
        for _ in 0..<3 {
            try await toggle("First model", hidden: true)
            try await toggle("First model", hidden: false)
        }
        try await toggle("First model", hidden: true)
        try await toggle("Second model", hidden: true)
        try await toggle("First model", hidden: false)
        try await toggle("Second model", hidden: false)
        state.cost = true
        try await settle(host)
        try await toggle("First model", hidden: true)
        state.total = 400
        try await settle(host)
        try await toggle("First model", hidden: false)
        try await toggle("First model", hidden: true)
        try await toggle("First model", hidden: false)
    }
}

@main private struct VerifyModelLegend {
    static func main() {
        let app = NSApplication.shared, delegate = LegendChecks()
        app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}
