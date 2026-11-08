import SwiftUI

extension DisplayPhase {
    var tint: Color {
        switch self {
        case .working, .thinking, .tool: return Color(red: 0.34, green: 0.40, blue: 0.94)
        case .needsInput: return Color(red: 0.80, green: 0.42, blue: 0.06)
        case .complete: return Color(red: 0.10, green: 0.57, blue: 0.43)
        case .failed: return Color(red: 0.84, green: 0.24, blue: 0.31)
        case .idle, .unknown: return .secondary
        }
    }
}

enum DisplayCardSize: String, CaseIterable {
    case small, medium, large
    var dimensions: CGSize {
        switch self {
        case .small: return CGSize(width: 170, height: 170)
        case .medium: return CGSize(width: 364, height: 170)
        case .large: return CGSize(width: 364, height: 360)
        }
    }
    var title: String { switch self { case .small: return "小号"; case .medium: return "中号"; case .large: return "大号" } }
}

/// Shared SwiftUI content, rendered both by WidgetKit and the live desktop panels.
/// No timers or application-only APIs are used here; the owner supplies the date.
struct DisplayCard: View {
    var snapshot: DisplaySnapshot
    var size: DisplayCardSize
    var date = Date()
    var live = false
    @Environment(\.colorScheme) private var scheme
    private var phase: DisplayPhase { snapshot.phase(at: date) }
    private var stale: Bool { snapshot.isStale(at: date) }
    private func t(_ source: String) -> String { snapshot.text(source) }

    var body: some View {
        VStack(alignment: .leading, spacing: size == .large ? 8 : 10) {
            HStack(spacing: 6) {
                Image(systemName: "eye.fill").foregroundStyle(phase.tint)
                Text("Aieyes").fontWeight(.semibold)
                Spacer(minLength: 2)
                HStack(spacing: 4) {
                    Circle().fill(stale ? Color.secondary : snapshot.isDemo ? Color.orange : phase.tint).frame(width: 5, height: 5)
                    Text(snapshot.status(at: date, live: live)).lineLimit(1)
                }.font(.system(size: 10, weight: .medium)).foregroundStyle(.secondary)
            }.font(.system(size: 12))
            if size == .small { usage; Spacer(minLength: 0); status }
            else if size == .medium {
                HStack(alignment: .center, spacing: 14) {
                    VStack(alignment: .leading, spacing: 9) { usage; status }.frame(maxWidth: .infinity, alignment: .leading)
                    Rectangle().fill(.primary.opacity(0.08)).frame(width: 1)
                    quotaPair
                }.frame(maxHeight: .infinity)
            } else {
                HStack(alignment: .center) {
                    usage
                    Spacer()
                    VStack(alignment: .trailing, spacing: 6) {
                        Text(stale || snapshot.sessionsUnavailable ? "—" : String(snapshot.activeCount)).font(.system(size: 30, weight: .semibold, design: .rounded)).monospacedDigit()
                        Text(t("活跃会话")).font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
                status
                Divider().opacity(0.45)
                HStack(alignment: .center, spacing: 20) {
                    quotaPair
                    VStack(alignment: .leading, spacing: 10) {
                        Text(snapshot.quotas.first?.provider ?? t("账户额度")).font(.system(size: 12, weight: .semibold)).lineLimit(1)
                        Text(t("剩余额度")).font(.system(size: 11)).foregroundStyle(.secondary)
                        Text(t("随账户查询更新")).font(.system(size: 10)).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
                Spacer(minLength: 0)
                HStack(spacing: 12) {
                    metric(t("缓存命中"), value: snapshot.tokens(at: date) == nil ? "—" : snapshot.cachePercent.map { String(format: "%.0f%%", $0) } ?? "—")
                    Spacer(minLength: 0)
                    metric(t("API 等价成本"), value: snapshot.tokens(at: date) == nil ? "—" : snapshot.costUSD.map { String(format: "$%.2f", $0) } ?? "—")
                }
                footer
            }
        }.padding(size == .large ? 16 : 14)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .accessibilityElement(children: .combine)
    }
    private var usage: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(t("今日 Token")).font(.system(size: 11, weight: .medium)).foregroundStyle(.secondary)
            Text(DisplaySnapshot.compact(snapshot.tokens(at: date)))
                .font(.system(size: size == .large ? 34 : 27, weight: .semibold, design: .rounded))
                .monospacedDigit().lineLimit(1).minimumScaleFactor(0.7)
                .foregroundStyle(snapshot.usageUnavailable ? .secondary : .primary)
                .accessibilityLabel(t("今日 Token") + " " + DisplaySnapshot.compact(snapshot.tokens(at: date)))
        }
    }
    private var status: some View {
        HStack(spacing: 5) {
            Image(systemName: phase.symbol).font(.system(size: 11, weight: .semibold))
            Text(t(phase.title)).font(.system(size: 11, weight: .semibold)).lineLimit(1).minimumScaleFactor(0.8)
            if size == .large {
                Spacer()
                if snapshot.waitingCount > 0 && !stale { Text(String(snapshot.waitingCount) + " " + t("待处理")).font(.system(size: 11)) }
            }
        }.foregroundStyle(phase.tint).padding(.horizontal, 8).padding(.vertical, 6)
            .background(phase.tint.opacity(scheme == .dark ? 0.17 : 0.09), in: RoundedRectangle(cornerRadius: 8))
    }
    private var quotaPair: some View {
        HStack(spacing: size == .large ? 14 : 10) {
            if snapshot.quotas.isEmpty {
                VStack(spacing: 7) {
                    Image(systemName: "circle.dashed").font(.system(size: 32, weight: .light))
                    Text(t("暂无额度")).font(.system(size: 10))
                }.foregroundStyle(.secondary).frame(width: 120)
            } else {
                ForEach(Array(snapshot.quotas.enumerated()), id: \.offset) { _, quota in
                    DisplayQuotaRing(quota: quota, snapshot: snapshot, date: date)
                }
            }
        }
    }
    private func metric(_ title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.system(size: 10)).foregroundStyle(.secondary)
            Text(value).font(.system(size: 16, weight: .semibold, design: .rounded)).monospacedDigit()
        }
    }
    private var footer: some View {
        HStack(spacing: 4) {
            Image(systemName: "clock").font(.system(size: 9))
            Text(Date(timeIntervalSince1970: snapshot.capturedAt), style: .time)
            Spacer()
            Text(t(snapshot.usageUnavailable ? "用量待更新" : "点击打开总览"))
        }.font(.system(size: 10)).foregroundStyle(.secondary)
    }
}

struct DisplayQuotaRing: View {
    let quota: DisplayQuota
    let snapshot: DisplaySnapshot
    var date = Date()
    private var expired: Bool { quota.resetsAt.map { $0 <= date.timeIntervalSince1970 } ?? false }
    private var unavailable: Bool { expired || quota.stale || snapshot.isStale(at: date) }
    private var color: Color { unavailable ? .secondary : quota.remaining <= 15 ? DisplayPhase.failed.tint : quota.remaining <= 30 ? DisplayPhase.needsInput.tint : DisplayPhase.working.tint }
    var body: some View {
        VStack(spacing: 5) {
            ZStack {
                Circle().stroke(color.opacity(0.13), lineWidth: 5)
                Circle().trim(from: 0, to: expired ? 0 : quota.fraction).stroke(color, style: StrokeStyle(lineWidth: 5, lineCap: .round)).rotationEffect(.degrees(-90))
                Text(expired ? "—" : String(format: "%.0f", quota.remaining))
                    .font(.system(size: 16, weight: .semibold, design: .rounded)).monospacedDigit()
            }.frame(width: 48, height: 48)
            Text(quota.title).font(.system(size: 10, weight: .medium)).lineLimit(1).frame(maxWidth: 64)
            Text(snapshot.text(unavailable ? "待更新" : "剩余 %")).font(.system(size: 9)).foregroundStyle(.secondary)
        }.accessibilityElement(children: .ignore)
            .accessibilityLabel(quota.provider + " " + quota.title + " " + (unavailable ? snapshot.text("待更新") : String(format: "%.0f%%", quota.remaining)))
    }
}

struct DisplayCardBackground: View {
    var phase: DisplayPhase
    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    var body: some View {
        ZStack {
            Color(nsColor: .windowBackgroundColor)
            if !reduceTransparency {
                LinearGradient(colors: [phase.tint.opacity(scheme == .dark ? 0.16 : 0.07), .clear], startPoint: .topTrailing, endPoint: .bottomLeading)
            }
        }
    }
}

struct DisplayRibbon: View {
    var snapshot: DisplaySnapshot
    var date = Date()
    var body: some View {
        let phase = snapshot.phase(at: date)
        HStack(spacing: 10) {
            Image(systemName: "eye.fill").font(.system(size: 18)).foregroundStyle(phase.tint)
            VStack(alignment: .leading, spacing: 2) {
                Text(snapshot.text(phase.title)).font(.system(size: 12, weight: .semibold))
                Text(snapshot.status(at: date, live: true)).font(.system(size: 9)).foregroundStyle(.secondary)
            }
            Spacer(minLength: 4)
            Text(DisplaySnapshot.compact(snapshot.tokens(at: date))).font(.system(size: 17, weight: .semibold, design: .rounded)).monospacedDigit()
            Text("Token").font(.system(size: 10)).foregroundStyle(.secondary)
            Image(systemName: "chevron.right").font(.system(size: 10, weight: .semibold)).foregroundStyle(.secondary)
        }.padding(.horizontal, 16).frame(maxWidth: .infinity, maxHeight: .infinity)
            .accessibilityElement(children: .combine)
    }
}
