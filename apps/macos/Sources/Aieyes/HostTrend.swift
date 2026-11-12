import SwiftUI

/// One reading on a trend line: epoch seconds and a share from 0 to 100.
struct TrendPoint: Equatable { var t: Double, value: Double }

/// The readings one trend line draws: a fixed window that ends at the host's latest minute.
struct TrendSeries: Equatable {
    /// Minutes the window covers.
    static let window = 60
    /// Readings inside the window, oldest first.
    var points: [TrendPoint] = []
    /// Right edge of the window, epoch seconds.
    var end = 0.0
    /// Largest spacing between two readings that still counts as continuous sampling, seconds.
    var gap = 150.0
    /// Highest value inside the window; a minute's own maximum when the source keeps one.
    var peak: Double?

    var start: Double { end - Double(Self.window - 1) * 60 }
    /// Two neighbouring readings exist, so there is a line to draw. One reading is not a trend.
    var hasLine: Bool { zip(points, points.dropFirst()).contains { $1.t - $0.t <= gap } }
    /// The newest reading while it is current. An older one is history and carries no marker.
    var latest: TrendPoint? { points.last.flatMap { end - $0.t <= gap ? $0 : nil } }
    var summary: String {
        guard let first = points.first, let last = points.last, let peak else { return "" }
        let mean = points.reduce(0) { $0 + $1.value } / Double(points.count)
        return L("{0} 分钟走势 · 均值 {1}% · 峰值 {2}%", Int((last.t - first.t) / 60) + 1, String(format: "%.0f", mean), String(format: "%.0f", peak))
    }
}

/// Minute history of one host, as its card draws it.
struct HostTrend: Equatable {
    enum Metric { case cpu, memory, gpu }
    var minutes: [HostMinute] = []
    /// Largest spacing between two readings that still counts as continuous sampling, seconds.
    var gap = 150.0

    /// Sampling slower than once a minute leaves minutes without a reading; those still join.
    static func gap(samplingSeconds: Int) -> Double { max(150, Double(samplingSeconds) * 2.5) }

    /// A minute without the metric is skipped, so it is never drawn as zero.
    func series(_ metric: Metric) -> TrendSeries {
        guard let end = minutes.map(\.t).max() else { return TrendSeries(gap: gap) }
        var series = TrendSeries(end: end, gap: gap)
        for minute in minutes where minute.t >= series.start {
            let value: Double?, high: Double?
            switch metric {
            case .cpu: value = minute.cpu; high = minute.cpuMax ?? minute.cpu
            case .memory: value = minute.memory; high = minute.memory
            case .gpu: value = minute.gpu; high = minute.gpu
            }
            guard let value, value.isFinite else { continue }
            series.points.append(TrendPoint(t: minute.t, value: min(100, max(0, value))))
            if let high, high.isFinite { series.peak = max(series.peak ?? 0, min(100, max(0, high))) }
        }
        series.points.sort { $0.t < $1.t }
        return series
    }
}

extension AppModel {
    /// Two history minutes count as continuous while they are no further apart than the slower of
    /// the two sampling intervals allows.
    var hostTrendGap: Double { HostTrend.gap(samplingSeconds: max(settings.foregroundInterval, settings.serverRefreshSeconds)) }
    /// A host's minute history as its card draws it.
    func hostTrend(_ hostID: String) -> HostTrend { HostTrend(minutes: outbound.history[hostID] ?? [], gap: hostTrendGap) }
}

/// One layer of a trend line: the line, the wash under it, or the marker on the current reading.
/// The scale is always 0 to 100, the same as the bar under the tile's value, so a line near the top
/// means a nearly full resource on every card. Readings further apart than the series' gap are not
/// joined: a pause in sampling shows as a break, never as a drop.
struct TrendLineShape: Shape {
    enum Part { case line, area, marker }
    var series: TrendSeries
    var part = Part.line
    static let markerRadius: CGFloat = 2
    func path(in rect: CGRect) -> Path {
        var path = Path()
        // Room for the marker and the round line caps at the edges.
        let plot = rect.insetBy(dx: Self.markerRadius + 0.5, dy: Self.markerRadius + 0.5)
        let span = series.end - series.start
        guard span > 0, plot.width > 0, plot.height > 0 else { return path }
        func place(_ point: TrendPoint) -> CGPoint {
            CGPoint(x: plot.minX + CGFloat((point.t - series.start) / span) * plot.width,
                    y: plot.maxY - CGFloat(point.value / 100) * plot.height)
        }
        if part == .marker {
            guard let latest = series.latest else { return path }
            let spot = place(latest)
            path.addEllipse(in: CGRect(x: spot.x - Self.markerRadius, y: spot.y - Self.markerRadius, width: Self.markerRadius * 2, height: Self.markerRadius * 2))
            return path
        }
        var run: [CGPoint] = []
        func close() {
            defer { run.removeAll(keepingCapacity: true) }
            guard let first = run.first, let last = run.last else { return }
            if part == .area {
                guard run.count > 1 else { return }
                path.move(to: CGPoint(x: first.x, y: plot.maxY))
                for spot in run { path.addLine(to: spot) }
                path.addLine(to: CGPoint(x: last.x, y: plot.maxY)); path.closeSubpath()
            } else {
                // A zero-length first segment: a reading with no neighbour stays visible as a dot.
                path.move(to: first); path.addLine(to: first)
                for spot in run.dropFirst() { path.addLine(to: spot) }
            }
        }
        var previous: TrendPoint?
        for point in series.points {
            if let previous, point.t - previous.t > series.gap { close() }
            run.append(place(point)); previous = point
        }
        close()
        return path
    }
}

/// A stat tile's trend: a thin line in the tile's colour over a faint wash, with a dot on the
/// current reading. The tile's title names the series, so there is no legend and no axis.
struct TrendLine: View {
    var series: TrendSeries
    var tint: Color
    var height = TrendLine.compactHeight
    /// Beside a caption or inside a row of text.
    static let compactHeight: CGFloat = 16
    /// Beside a tile's headline value.
    static let tileHeight: CGFloat = 20
    var body: some View {
        ZStack {
            TrendLineShape(series: series, part: .area).fill(tint.opacity(0.14))
            TrendLineShape(series: series).stroke(tint.opacity(0.75), style: StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
            TrendLineShape(series: series, part: .marker).fill(tint)
        }.frame(height: height).contentShape(Rectangle()).help(series.summary)
            .accessibilityElement().accessibilityLabel(series.summary)
    }
}
