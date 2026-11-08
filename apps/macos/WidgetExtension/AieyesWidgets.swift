import WidgetKit
import SwiftUI
import OSLog

struct AieyesWidgetEntry: TimelineEntry {
    var date: Date
    var snapshot: DisplaySnapshot
}

struct AieyesWidgetProvider: TimelineProvider {
    func placeholder(in context: Context) -> AieyesWidgetEntry { AieyesWidgetEntry(date: Date(), snapshot: .sample()) }
    func getSnapshot(in context: Context, completion: @escaping (AieyesWidgetEntry) -> Void) {
        let saved = DisplaySnapshotFile.read(from: DisplaySnapshotFile.url())
        let snapshot = context.isPreview ? DisplaySnapshot.sample(language: saved.language) : saved
        completion(AieyesWidgetEntry(date: Date(), snapshot: snapshot))
    }
    func getTimeline(in context: Context, completion: @escaping (Timeline<AieyesWidgetEntry>) -> Void) {
        let now = Date(), snapshot = DisplaySnapshotFile.read(from: DisplaySnapshotFile.url())
        Logger(subsystem: "app.aieyes.displaylab.widgets", category: "snapshot").info("Loaded snapshot: schema=\(snapshot.version) demo=\(snapshot.isDemo) running=\(snapshot.appRunning) phase=\(snapshot.phase.rawValue, privacy: .public)")
        var dates = [now]
        let stale = Date(timeIntervalSince1970: snapshot.capturedAt + DisplaySnapshot.staleAfter + 1)
        if stale > now { dates.append(stale) }
        // Even if the host crashes or is not running at midnight, never label
        // yesterday's tokens or an old active session as current information.
        if let midnight = Calendar.current.dateInterval(of: .day, for: now)?.end { dates.append(midnight) }
        for reset in snapshot.quotas.compactMap(\.resetsAt).map({ Date(timeIntervalSince1970: $0 + 1) }) where reset > now && reset < now.addingTimeInterval(86_400) { dates.append(reset) }
        let entries = Set(dates).sorted().map { AieyesWidgetEntry(date: $0, snapshot: snapshot) }
        completion(Timeline(entries: entries, policy: .after(now.addingTimeInterval(300))))
    }
}

struct AieyesWidgetView: View {
    var entry: AieyesWidgetEntry
    @Environment(\.widgetFamily) private var family
    private var size: DisplayCardSize { family == .systemSmall ? .small : family == .systemLarge ? .large : .medium }
    var body: some View {
        DisplayCard(snapshot: entry.snapshot, size: size, date: entry.date)
            .containerBackground(for: .widget) { DisplayCardBackground(phase: entry.snapshot.phase(at: entry.date)) }
            .widgetURL(URL(string: "aieyes-display-lab://overview"))
    }
}

@main struct AieyesWidgets: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: DisplaySnapshot.widgetKind, provider: AieyesWidgetProvider()) { AieyesWidgetView(entry: $0) }
            .configurationDisplayName("Aieyes Lab")
            .description("今日用量、账户额度与会话状态快照。")
            .supportedFamilies([.systemSmall, .systemMedium, .systemLarge])
            .contentMarginsDisabled()
    }
}
