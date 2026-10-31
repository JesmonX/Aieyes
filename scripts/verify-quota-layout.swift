import AppKit
import SwiftUI
@testable import Aieyes

private struct CardFrames: PreferenceKey {
    static let defaultValue: [String: CGFloat] = [:]
    static func reduce(value: inout [String: CGFloat], nextValue: () -> [String: CGFloat]) { value.merge(nextValue(), uniquingKeysWith: max) }
}
@MainActor private final class FixtureState: ObservableObject {
    @Published var quotas: [Quota] = []
    @Published var width: CGFloat = 414
    var heights: [String: CGFloat] = [:]
}
private struct Fixture: View {
    @ObservedObject var state: FixtureState
    @State private var height: CGFloat = 0
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(state.quotas) { quota in
                QuotaCard(quota: quota, compact: true, collapsible: true)
                    .background(GeometryReader { p in Color.clear.preference(key: CardFrames.self, value: [quota.id:p.size.height]) })
            }
        }.frame(width: state.width).environment(\.collapsedQuotaHeight, height)
            .onPreferenceChange(CollapsedQuotaMeasurements.self) { height = $0.values.max() ?? 0 }
            .onPreferenceChange(CardFrames.self) { state.heights = $0 }
    }
}
@main struct VerifyQuotaLayout {
    @MainActor static func main() async throws {
        _ = NSApplication.shared
        let state = FixtureState()
        let raw = #"""
        [{"provider":"codex","accountId":"a","name":"Short","plan":"plus","windows":[{"id":"five","name":"5h","windowMinutes":300,"usedPercent":10}]},
         {"provider":"codex","accountId":"b","name":"A long subscription name that needs several lines when the card gets narrower, with an extra balance and quota type","plan":"pro","identity":{"key":"b","email":"long-subscription@example.test"},"credits":{"balance":"99999999.123456","hasCredits":true,"unlimited":false},"bankReset":{"availableCount":12},"windows":[{"id":"five","name":"5h","windowMinutes":300,"usedPercent":40},{"id":"week","name":"7d","windowMinutes":10080,"usedPercent":70}]},
         {"provider":"codex","accountId":"c","name":"Error account","error":"Synthetic quota update failure","windows":[]},
         {"provider":"workbuddy","accountId":"d","name":"Meters only","windows":[],"meters":[{"id":"month","label":"本月已用","used":120.5,"total":500,"unit":"credits"},{"id":"over","label":"Overage","used":0,"overage":3,"unit":"usd","estimated":true}],"notice":"Open WorkBuddy once to refresh"},
         {"provider":"claude","accountId":"e","name":"Stale grouped","staleSince":1,"snapshotAt":1,"windows":[{"id":"s5","groupName":"Sonnet","name":"Sonnet · 5h","windowMinutes":300,"usedPercent":30},{"id":"s7","groupName":"Sonnet","name":"Sonnet · 7d","windowMinutes":10080,"usedPercent":60}]},
         {"provider":"deepseek","accountId":"f","name":"Non-codex","windows":[],"balances":[{"currency":"CNY","total":"10.00","granted":"0.00","toppedUp":"10.00"}]}]
        """#
        var payload = try JSONSerialization.jsonObject(with: Data(raw.utf8)) as! [[String:Any]]
        for index in payload.indices { payload[index]["sourceId"] = "fixture";payload[index]["updatedAt"] = 1;payload[index]["origin"] = "live" }
        state.quotas = try JSONDecoder().decode([Quota].self, from: JSONSerialization.data(withJSONObject:payload))
        let saved = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
        defer { UserDefaults.standard.setVolatileDomain(saved, forName: UserDefaults.argumentDomain) }
        var overrides = NativePanelKey.snapshotOverrides()
        for quota in state.quotas { overrides[NativePanelKey.expansion(.quota, id: quota.id, compact: true)] = false }
        UserDefaults.standard.setVolatileDomain(overrides, forName: UserDefaults.argumentDomain)
        let view = NSHostingView(rootView: Fixture(state: state))
        let window = NSWindow(contentRect: NSRect(x:0,y:0,width:450,height:1600), styleMask:[.borderless], backing:.buffered, defer:false)
        window.isReleasedWhenClosed = false;window.contentView = view
        window.setFrameOrigin(NSPoint(x:-20000,y:-20000));window.orderBack(nil)
        defer { window.close() }
        func settle() async throws { try await Task.sleep(for:.milliseconds(300));view.layoutSubtreeIfNeeded();try await Task.sleep(for:.milliseconds(200)) }
        for width: CGFloat in [414, 320, 600] {
            state.width = width;try await settle()
            let heights = Array(state.heights.values)
            precondition(heights.count == 6 && heights.max()! - heights.min()! <= 1, "Collapsed native cards differ: \(state.heights)")
        }
        let largest = state.heights.values.max()!
        let extras = state.quotas[3...]
        precondition(extras[3].visibleMeters.count == 2 && extras[3].hasQuotaContent && extras[3].visibleNotice != nil)
        precondition(extras[4].groupsWindows && extras[4].isStale && !state.quotas[0].groupsWindows)
        state.quotas = [state.quotas[0], state.quotas[2]];try await settle()
        let shorter = Array(state.heights.values)
        precondition(shorter.count == 2 && shorter.max()! - shorter.min()! <= 1 && shorter.max()! < largest, "Removing the tallest card must shrink the group: \(largest) -> \(state.heights)")
        let model = AppModel(autostart:false)
        defer { model.stop() }
        let panel = PanelContainer(model:model)
        for width: CGFloat in [450,360] {
            panel.prepare(size:NSSize(width:width,height:540))
            precondition(abs(panel.view.frame.width-width)<1)
            precondition(panel.view.subviews.allSatisfy { abs($0.frame.width-width)<1 }, "Every first-layout panel layer fills its container: \(panel.view.subviews.map { $0.frame })")
        }
        print("Native quota layout: collapsed heights match for multiple subscriptions, long text, errors, meters, stale and non-Codex cards and widths; group shrinks; initial panel layers fill their width")
    }
}
