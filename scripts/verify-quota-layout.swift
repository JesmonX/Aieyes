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
private struct GridFrames: PreferenceKey {
    static let defaultValue: [Int: CGRect] = [:]
    static func reduce(value: inout [Int: CGRect], nextValue: () -> [Int: CGRect]) { value.merge(nextValue(), uniquingKeysWith: { _, next in next }) }
}
private struct DetailColumnFrames: PreferenceKey {
    static let defaultValue: [String: CGRect] = [:]
    static func reduce(value: inout [String: CGRect], nextValue: () -> [String: CGRect]) { value.merge(nextValue(), uniquingKeysWith: { _, next in next }) }
}
@MainActor private final class DetailColumnState: ObservableObject {
    @Published var rows = 8
    @Published var frames: [String: CGRect] = [:]
}
private struct DetailColumnFixture: View {
    @ObservedObject var state: DetailColumnState
    var body: some View {
        RingDetailColumns {
            Color.blue.opacity(0.15).frame(minHeight: 20, maxHeight: .infinity)
                .background(GeometryReader { geometry in Color.clear.preference(key: DetailColumnFrames.self, value: ["primary": geometry.frame(in: .named("columns"))]) })
            VStack(spacing: 6) { ForEach(0..<state.rows, id: \.self) { Text("Row \($0)") } }
                .frame(maxWidth: .infinity, alignment: .top)
                .background(GeometryReader { geometry in Color.clear.preference(key: DetailColumnFrames.self, value: ["facts": geometry.frame(in: .named("columns"))]) })
        }.coordinateSpace(name: "columns").frame(width: 800)
            .onPreferenceChange(DetailColumnFrames.self) { state.frames = $0 }
    }
}
@MainActor private final class GridState: ObservableObject {
    @Published var columns = 2
    @Published var width: CGFloat = 800
    @Published var heights: [CGFloat] = [140, 350, 114, 180, 90, 120]
    @Published var full = false
    var frames: [Int: CGRect] = [:]
}
private struct GridFixture: View {
    @ObservedObject var state: GridState
    var body: some View {
        QuotaGridLayout(columns: state.columns) {
            ForEach(state.heights.indices, id: \.self) { index in
                RoundedRectangle(cornerRadius: 14).fill(index == 1 ? Color.purple.opacity(0.2) : Color.blue.opacity(0.2))
                    .overlay(Text(["Qoder CN", "Antigravity", "DeepSeek", "Card 4", "Card 5", "Card 6"][index]))
                    .frame(height: state.heights[index])
                    .background(GeometryReader { geometry in Color.clear.preference(key: GridFrames.self, value: [index: geometry.frame(in: .named("grid"))]) })
                    .layoutValue(key: QuotaFullWidth.self, value: state.full && index == 3)
            }
        }.coordinateSpace(name: "grid").frame(width: state.width)
            .onPreferenceChange(GridFrames.self) { state.frames = $0 }
    }
}
@MainActor private final class MasonryState: ObservableObject {
    /// (group, header, height); header rows carry the group's account count as span.
    @Published var items: [(String, Bool, CGFloat)] = []
    @Published var columns = 2
    var frames: [Int: CGRect] = [:]
}
private struct MasonryFixture: View {
    @ObservedObject var state: MasonryState
    var body: some View {
        QuotaMasonryLayout(columns: state.columns) {
            ForEach(state.items.indices, id: \.self) { index in
                let item = state.items[index]
                Rectangle().fill(item.1 ? Color.gray.opacity(0.2) : Color.blue.opacity(0.2)).frame(height: item.2)
                    .background(GeometryReader { geometry in Color.clear.preference(key: GridFrames.self, value: [index: geometry.frame(in: .named("masonry"))]) })
                    .layoutValue(key: QuotaMasonryItem.self, value: QuotaMasonryRole(group: item.0, span: state.items.filter { $0.0 == item.0 && !$0.1 }.count, header: item.1))
            }
        }.coordinateSpace(name: "masonry").frame(width: 1000)
            .onPreferenceChange(GridFrames.self) { state.frames = $0 }
    }
}
@main struct VerifyQuotaLayout {
    @MainActor static func main() async throws {
        _ = NSApplication.shared
        let cn = try JSONDecoder().decode(Aggregate.self, from: Data(#"{"key":"test","tokens":{"input":100,"output":0,"cacheRead":0,"cacheWrite":0},"total":100,"cost":1,"pricedTokens":100,"nativeCredits":{"qoder-cn":{"credits":10,"billable":0,"nonBillable":0,"unknownBillability":10,"requests":1}}}"#.utf8))
        precondition(!cn.hasDisplayCredits && cn.nativeCreditText.isEmpty && cn.total == 100)
        let identity = try JSONDecoder().decode(AccountIdentity.self, from: Data(#"{"key":"qoder-cn:hash","email":"","providerAccountId":"public-id"}"#.utf8))
        precondition(identity.displayName.contains("public-id") && !identity.displayName.contains("hash"))
        let groupedCodex = try JSONDecoder().decode(Quota.self, from: Data(#"{"provider":"codex","sourceId":"fixture","updatedAt":1,"origin":"live","accountId":"test","name":"test","windows":[{"name":"5h","groupName":"codex","windowMinutes":300,"usedPercent":0},{"name":"7d","groupName":"codex","windowMinutes":10080,"usedPercent":0}]}"#.utf8))
        precondition(!groupedCodex.groupsWindows)
        var qoder = groupedCodex
        qoder.provider = "qoder-cn"; qoder.identity = identity
        precondition(qoder.cardIdentity == nil)
        let creditMeter = QuotaMeter(id: "credits", label: "加购 Credits", used: 3, total: 100, remaining: 97, unit: "credits")
        precondition(qoder.meterSummary(creditMeter) == "97 / 100 Credits")
        qoder.windows = []
        qoder.meters = [QuotaMeter(id: "addOnQuota", label: "Addon", used: 2, total: 20, remaining: 18, unit: "credits"), QuotaMeter(id: "userQuota", label: "Plan", used: 25, total: 2000, remaining: 1975, unit: "credits")]
        precondition(qoderCreditSum(qoder) == "1975+18")
        guard case .duo(let addonArc, let planArc) = QuotaRingModel.ring([], quota: qoder).0 else { fatalError("Addon/Plan ring missing") }
        precondition(addonArc.remaining == 90 && addonArc.resetsAt == nil)
        precondition(planArc.remaining == 98.75 && planArc.resetsAt == nil)
        let withoutAddon = qoder
        qoder.meters?[0].total = 0
        precondition(QuotaRingModel.ring([], quota: qoder).0.top?.remaining == 0)
        qoder = withoutAddon
        qoder.meters?[0].remaining = 0
        precondition(qoderCreditSum(qoder) == "1975")
        qoder.meters?.removeLast()
        guard case .empty = QuotaRingModel.ring([], quota: qoder).0 else { fatalError("Addon must not substitute for missing Plan") }
        var kiro = withoutAddon
        kiro.provider = "kiro"
        kiro.meters = [QuotaMeter(id: "plan", label: "套餐 Credits", used: 25, total: 2000, remaining: 1975, unit: "credits", resetsAt: 1900000000),
                       QuotaMeter(id: "bonus", label: "赠送 Credits", used: 25, total: 100, remaining: 75, unit: "credits", resetsAt: 1900500000),
                       QuotaMeter(id: "overage", label: "超额 Credits", used: 12.5, unit: "credits", resetsAt: 1900000000)]
        let kiroItems = QuotaRingModel.items(kiro)
        precondition(kiroItems.count == 3 && Set(kiroItems.map(\.id)).count == 3)
        precondition(kiroItems[0].kind.top?.remaining == 98.75 && kiroItems[1].kind.top?.remaining == 75)
        precondition(kiroItems[0].meterReset?.resetsAt == 1900000000 && kiroItems[1].meterReset?.resetsAt == 1900500000)
        precondition(kiroItems[2].kind == .empty && kiroItems[2].meterReset?.resetsAt == 1900000000)
        precondition(QuotaRingModel.accessibility(kiroItems[0]).contains("1975"))
        kiro.meters?.reverse()
        precondition(QuotaRingModel.items(kiro).map(\.id) == kiroItems.reversed().map(\.id))
        kiro.meters = []
        precondition(QuotaRingModel.items(kiro).count == 1 && QuotaRingModel.items(kiro)[0].kind == .empty)
        precondition(Format.subscription("personal_professional") == "Pro")
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
        var deepseek = state.quotas.last!
        precondition(RingBalanceComposition.make(deepseek) == nil)
        deepseek.balances = [Balance(currency: "CNY", total: "100", granted: "25", toppedUp: "75")]
        precondition(RingBalanceComposition.make(deepseek)?.share == 0.25)
        precondition(RingBalanceComposition.make(deepseek)?.granted == "¥25")
        deepseek.balances?[0].toppedUp = "0"
        precondition(RingBalanceComposition.make(deepseek)?.share == 1)
        deepseek.balances?[0].granted = "nan"
        precondition(RingBalanceComposition.make(deepseek) == nil)
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
        let grid = GridState()
        let gridView = NSHostingView(rootView: GridFixture(state: grid))
        let gridWindow = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1200, height: 1600), styleMask: [.borderless], backing: .buffered, defer: false)
        gridWindow.isReleasedWhenClosed = false; gridWindow.contentView = gridView
        gridWindow.setFrameOrigin(NSPoint(x: -20000, y: -20000)); gridWindow.orderBack(nil)
        defer { gridWindow.close() }
        for columns in [2, 1, 3, 4, 2] {
            grid.columns = columns; grid.width = columns == 1 ? 360 : 1080
            for expanded in [false, true] {
                grid.heights[1] = expanded ? 470 : 350
                grid.full = expanded
                try await Task.sleep(for: .milliseconds(300)); gridView.layoutSubtreeIfNeeded()
                let frames = grid.frames
                precondition(frames.count == 6)
                for (i, left) in frames { for (j, right) in frames where i < j { precondition(!left.intersects(right), "Quota cards overlap: \(i), \(j)") } }
                if columns == 2 {
                    precondition(abs(frames[2]!.minY - frames[0]!.maxY - 10) < 1, "DeepSeek must follow Qoder, not the taller Antigravity card")
                    precondition(abs(frames[2]!.minX - frames[0]!.minX) < 1)
                }
                if expanded { precondition(abs(frames[3]!.width - grid.width) < 1, "Multi-account section fills the grid") }
            }
        }
        grid.full = false; grid.heights = [140, 350, 114]
        gridWindow.setContentSize(NSSize(width: 1080, height: 400))
        try await Task.sleep(for: .milliseconds(300)); gridView.layoutSubtreeIfNeeded()
        let output = URL(fileURLWithPath: ".local/quota-masonry-native.png")
        if let bitmap = gridView.bitmapImageRepForCachingDisplay(in: gridView.bounds) {
            gridView.cacheDisplay(in: gridView.bounds, to: bitmap)
            try bitmap.representation(using: .png, properties: [:])?.write(to: output)
        }
        let details = DetailColumnState()
        let detailView = NSHostingView(rootView: DetailColumnFixture(state: details))
        let detailWindow = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 500), styleMask: [.borderless], backing: .buffered, defer: false)
        detailWindow.isReleasedWhenClosed = false; detailWindow.contentView = detailView
        detailWindow.setFrameOrigin(NSPoint(x: -20000, y: -20000)); detailWindow.orderBack(nil)
        defer { detailWindow.close() }
        for (rows, taller) in [(8, true), (0, false)] {
            details.rows = rows
            try await Task.sleep(for: .milliseconds(300)); detailView.layoutSubtreeIfNeeded()
            guard let primary = details.frames["primary"], let facts = details.frames["facts"] else { fatalError("Detail column frames missing") }
            if taller {
                precondition(primary.height > 60, "Quota cards must stretch with a taller facts column: \(details.frames)")
                precondition(abs(primary.height - facts.height) < 1, "Quota cards must fill the facts height: \(details.frames)")
            } else {
                precondition(abs(primary.height - 20) < 1, "Quota cards must keep their natural height when facts are shorter: \(details.frames)")
            }
        }
        // Account groups: a group spans only its account columns, and later groups fill the shorter ones.
        func place(_ items: [(String, Bool, CGFloat)], columns: Int, width: CGFloat = 1000) -> [CGRect] {
            QuotaMasonryLayout(columns: columns).frames(width: width, count: items.count, role: { index in
                let item = items[index]; return QuotaMasonryRole(group: item.0, span: items.filter { $0.0 == item.0 && !$0.1 }.count, header: item.1)
            }, height: { index, _ in items[index].2 }).1
        }
        func disjoint(_ frames: [CGRect]) { for i in frames.indices { for j in frames.indices where i < j { precondition(!frames[i].insetBy(dx: 0.5, dy: 0.5).intersects(frames[j]), "Masonry items overlap: \(i), \(j) \(frames)") } } }
        // The reported overview: an expanded and a collapsed Codex card, then Claude Code and Antigravity.
        let overview: [(String, Bool, CGFloat)] = [("codex", true, 30), ("codex", false, 460), ("codex", false, 150), ("claude", true, 30), ("claude", false, 120), ("antigravity", true, 30), ("antigravity", false, 230)]
        let two = place(overview, columns: 2); disjoint(two)
        let cell = (1000 - 14) / 2.0
        precondition(abs(two[0].width - 1000) < 1 && two[0].minY == 0, "A two-account group heading spans both columns")
        precondition(abs(two[1].minX) < 1 && abs(two[2].minX - (cell + 14)) < 1 && two[1].minY == two[2].minY)
        precondition(abs(two[3].minX - two[2].minX) < 1 && abs(two[3].minY - (two[2].maxY + 10)) < 1, "Claude Code fills the space under the shorter Codex card: \(two)")
        precondition(abs(two[4].minY - (two[3].maxY + 8)) < 1 && abs(two[4].width - cell) < 1)
        precondition(abs(two[5].minX - two[2].minX) < 1 && two[5].minY > two[4].maxY, "The next group also takes the lower column")
        let three = place(overview, columns: 3); disjoint(three)
        let third = (1000 - 28) / 3.0
        precondition(abs(three[0].width - (2 * third + 14)) < 1, "A two-account group spans two of three columns")
        precondition(three[3].minY == 0 && abs(three[3].minX - 2 * (third + 14)) < 1, "The free third column starts the next group at the top")
        let collapsed = place([("codex", true, 30), ("claude", true, 30), ("claude", false, 120), ("antigravity", true, 30)], columns: 2); disjoint(collapsed)
        precondition(abs(collapsed[0].width - cell) < 1 && collapsed[1].minY == 0, "A collapsed group heading leaves the other column free")
        precondition(abs(collapsed[3].minX - collapsed[0].minX) < 1 && abs(collapsed[3].minY - 40) < 1, "Groups after a collapsed heading fill beneath it")
        let single = place(overview, columns: 1, width: 400); disjoint(single)
        precondition(zip(single, single.dropFirst()).allSatisfy { $0.maxY <= $1.minY + 0.5 } && single.allSatisfy { abs($0.width - 400) < 1 }, "One column keeps data order")
        let masonry = MasonryState(); masonry.items = overview
        let masonryView = NSHostingView(rootView: MasonryFixture(state: masonry))
        let masonryWindow = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1100, height: 1400), styleMask: [.borderless], backing: .buffered, defer: false)
        masonryWindow.isReleasedWhenClosed = false; masonryWindow.contentView = masonryView
        masonryWindow.setFrameOrigin(NSPoint(x: -20000, y: -20000)); masonryWindow.orderBack(nil)
        defer { masonryWindow.close() }
        for columns in [2, 3, 1] {
            masonry.columns = columns
            try await Task.sleep(for: .milliseconds(300)); masonryView.layoutSubtreeIfNeeded()
            let expected = place(overview, columns: columns), rendered = (0..<overview.count).map { masonry.frames[$0] ?? .null }
            for (want, got) in zip(expected, rendered) { precondition(abs(want.minX - got.minX) < 1 && abs(want.minY - got.minY) < 1 && abs(want.width - got.width) < 1, "SwiftUI placement matches the masonry model: \(columns) \(rendered)") }
            disjoint(rendered)
        }
        print("Native account-group masonry: groups span their account columns, later groups fill shorter columns, collapsed headings and one column keep order")
        print("Native masonry: 1–4 columns, expanding cards, shortest-column packing and full-width groups pass")
        print("Native detail columns: quota cards stretch to a taller facts column and keep their natural height when it is shorter")
        print("Native quota layout: collapsed heights match for multiple subscriptions, long text, errors, meters, stale and non-Codex cards and widths; group shrinks; initial panel layers fill their width")
    }
}
