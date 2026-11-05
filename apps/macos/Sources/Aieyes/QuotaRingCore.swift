import SwiftUI

/// How a surface shows account quotas: the existing cards, dial tiles (rings with
/// reset times inside) or two-column tiles (percentages inside, times beside).
/// The panel and the main window each remember their own choice.
enum QuotaCardStyle: String, CaseIterable, Identifiable {
    case card, dial, tile
    var id: String { rawValue }
    var title: String {
        switch self {
        case .card: return L("卡片")
        case .dial: return L("表盘")
        case .tile: return L("双列")
        }
    }
    var help: String {
        switch self {
        case .card: return L("卡片样式")
        case .dial: return L("表盘样式")
        case .tile: return L("双列样式")
        }
    }
    var symbol: String {
        switch self {
        case .card: return "rectangle.grid.1x2"
        case .dial: return "circle.circle"
        case .tile: return "square.grid.2x2"
        }
    }
    static func storageKey(compact: Bool) -> String { NativePanelKey.scoped(.panelQuotaStyle, .detailQuotaStyle, compact: compact) }
}

struct QuotaRingArc: Equatable {
    var remaining: Double
    var resetsAt: Double?
    var windowMinutes: Int?
    /// No use reported yet: used 0 and no reset time.
    var unused: Bool
    var name: String
    var label: String {
        if windowMinutes == CoreContract.fiveHourMinutes { return "5h" }
        if windowMinutes == CoreContract.weekMinutes { return "7d" }
        return L(name.components(separatedBy: " · ").last ?? name)
    }
    var used: Double { 100 - remaining }
}

enum QuotaRingKind: Equatable {
    case duo(top: QuotaRingArc, bottom: QuotaRingArc)
    case single(QuotaRingArc)
    case coin(amount: String, low: Bool)
    case empty
    var top: QuotaRingArc? {
        switch self {
        case .duo(let top, _), .single(let top): return top
        default: return nil
        }
    }
    var bottom: QuotaRingArc? { if case .duo(_, let bottom) = self { return bottom }; return nil }
}

/// One ring: an account, or one model group of a grouped account (Antigravity).
struct QuotaRingItem: Identifiable, Equatable {
    var quota: Quota
    var group: String?
    var kind: QuotaRingKind
    /// Windows of this ring that are not drawn.
    var hidden: [QuotaWindow]
    var id: String { quota.id + (group.map { "#" + $0 } ?? "") }
    var title: String { group ?? quota.name }
}

/// Same rules as the core's `quota_ring.rs` and the Web UI's `quota-rings.js`.
enum QuotaRingModel {
    static func items(_ quota: Quota) -> [QuotaRingItem] {
        guard quota.groupsWindows, !quota.windows.isEmpty else {
            let (kind, hidden) = ring(quota.windows, quota: quota)
            return [QuotaRingItem(quota: quota, group: nil, kind: kind, hidden: hidden)]
        }
        let groups = quota.windows.reduce(into: [String]()) { result, window in if !result.contains(window.groupLabel) { result.append(window.groupLabel) } }
        return groups.map { group in
            let (kind, hidden) = ring(quota.windows.filter { $0.groupLabel == group }, quota: quota)
            return QuotaRingItem(quota: quota, group: group.isEmpty ? nil : group, kind: kind, hidden: hidden)
        }
    }
    private static func arc(_ window: QuotaWindow) -> QuotaRingArc {
        let used = window.usedPercent.isFinite ? min(100, max(0, window.usedPercent)) : 100
        let reset = window.resetsAt.flatMap { $0.isFinite && $0 > 0 ? $0 : nil }
        return QuotaRingArc(remaining: 100 - used, resetsAt: reset, windowMinutes: window.windowMinutes, unused: used == 0 && reset == nil, name: window.name)
    }
    /// A 5h + 7d pair wins; otherwise two windows of different known lengths, shorter on top;
    /// otherwise the first window alone as a full circle.
    static func pick(_ windows: [QuotaWindow]) -> (QuotaWindow?, QuotaWindow?) {
        if let five = windows.first(where: { $0.windowMinutes == CoreContract.fiveHourMinutes }),
           let week = windows.first(where: { $0.windowMinutes == CoreContract.weekMinutes }) { return (five, week) }
        let timed = windows.filter { ($0.windowMinutes ?? 0) > 0 }.enumerated().sorted { ($0.element.windowMinutes ?? 0, $0.offset) < ($1.element.windowMinutes ?? 0, $1.offset) }.map(\.element)
        if let top = timed.first, let bottom = timed.first(where: { $0.windowMinutes != top.windowMinutes }) { return (top, bottom) }
        return (windows.first, nil)
    }
    private static func currency(_ code: String) -> String {
        switch code.trimmingCharacters(in: .whitespaces).uppercased() {
        case "CNY", "RMB": return "¥"
        case "USD": return "$"
        case "": return ""
        case let other: return other + " "
        }
    }
    private static func meterAmount(_ unit: String, _ value: Double) -> String {
        let text = String(format: value.rounded() == value ? "%.0f" : "%.2f", value)
        return unit == "usd" ? "$" + text : unit == "cny" ? "¥" + text : text
    }
    static func ring(_ windows: [QuotaWindow], quota: Quota) -> (QuotaRingKind, [QuotaWindow]) {
        if isQoderCredits(quota) {
            guard let meter = quota.meters?.first(where: { $0.id == "userQuota" }), let fraction = meter.fraction else { return (.empty, []) }
            let addon = quota.meters?.first { $0.id == "addOnQuota" }
            // Keep addon above plan, including the empty track when no addon capacity exists.
            let top = QuotaRingArc(remaining: 100 - (addon?.fraction ?? 1) * 100, resetsAt: nil, windowMinutes: nil, unused: false, name: addon?.label ?? "Addon credits")
            let bottom = QuotaRingArc(remaining: 100 - fraction * 100, resetsAt: nil, windowMinutes: nil, unused: false, name: meter.label)
            return (.duo(top: top, bottom: bottom), [])
        }
        let (top, bottom) = pick(windows)
        let hidden = windows.filter { $0.id != top?.id && $0.id != bottom?.id }
        if let top, let bottom { return (.duo(top: arc(top), bottom: arc(bottom)), hidden) }
        if let top { return (.single(arc(top)), hidden) }
        if let meter = quota.visibleMeters.first(where: { $0.fraction != nil }), let fraction = meter.fraction {
            return (.single(QuotaRingArc(remaining: 100 - fraction * 100, resetsAt: meter.resetsAt.flatMap { $0 > 0 ? $0 : nil }, windowMinutes: nil, unused: false, name: meter.label)), [])
        }
        let low = quota.isAvailable == false
        if let balance = quota.balances?.first { return (.coin(amount: currency(balance.currency) + balance.total.trimmingCharacters(in: .whitespaces), low: low), []) }
        if let wallet = quota.extraUsage { return (.coin(amount: currency(wallet.currency) + (wallet.balance ?? wallet.total), low: low), []) }
        if let meter = quota.visibleMeters.first(where: { $0.remaining != nil }), let remaining = meter.remaining { return (.coin(amount: meterAmount(meter.unit, remaining), low: low), []) }
        if let balance = quota.credits?.balance { return (.coin(amount: Format.credits(balance), low: low), []) }
        return (.empty, [])
    }
    /// `47m`, `2h13m`, `14h`, `1d5h`, `3d`: unit letters only, so every language reads it.
    static func compact(minutes raw: Int) -> String {
        let minutes = max(1, raw)
        if minutes < 60 { return "\(minutes)m" }
        if minutes < 600 { return minutes % 60 == 0 ? "\(minutes / 60)h" : "\(minutes / 60)h\(minutes % 60)m" }
        if minutes < 1440 { return "\(Int((Double(minutes) / 60).rounded()))h" }
        if minutes < 4320 {
            let days = minutes / 1440, hours = Int((Double(minutes % 1440) / 60).rounded())
            return hours == 0 ? "\(days)d" : hours == 24 ? "\(days + 1)d" : "\(days)d\(hours)h"
        }
        return "\(Int((Double(minutes) / 1440).rounded()))d"
    }
    /// Unused 5h/7d windows show their length, a passed reset `…`, an unknown one `—`.
    static func resetText(_ arc: QuotaRingArc, now: Date = Date()) -> String {
        if arc.unused, arc.windowMinutes == CoreContract.fiveHourMinutes || arc.windowMinutes == CoreContract.weekMinutes { return arc.label }
        guard let reset = arc.resetsAt else { return "—" }
        let seconds = reset - now.timeIntervalSince1970
        guard seconds > 0, seconds / 60 < Double(Int.max) else { return "…" }
        return compact(minutes: Int(ceil(seconds / 60)))
    }
    static func remainingText(_ arc: QuotaRingArc) -> String { String(format: "%.0f%%", arc.remaining) }
    static func label(_ kind: QuotaRingKind, values: [String], now: Date = Date()) -> String {
        values.compactMap { value -> String? in
            switch value {
            case "topRemaining": if case .coin(let amount, _) = kind { return amount }; return kind.top.map(remainingText)
            case "topReset": return kind.top.map { resetText($0, now: now) }
            case "bottomRemaining": return kind.bottom.map(remainingText)
            case "bottomReset": return kind.bottom.map { resetText($0, now: now) }
            default: return nil
            }
        }.joined(separator: " ")
    }
    /// Full text for VoiceOver and tooltips.
    static func accessibility(_ item: QuotaRingItem, now: Date = Date()) -> String {
        if isQoderCredits(item.quota) {
            let rows = ["addOnQuota", "userQuota"].map { id in
                let meter = item.quota.meters?.first { $0.id == id }
                return (id == "userQuota" ? "Plan credits " : "Addon credits ") + qoderCreditNumber(meter?.remaining) + "/" + qoderCreditNumber(meter?.total)
            }
            return ([item.quota.name] + rows).joined(separator: " · ")
        }
        var parts = [item.group.map { item.quota.name + " · " + $0 } ?? item.quota.name]
        func window(_ arc: QuotaRingArc) -> String {
            arc.label + " " + L("剩余 ") + Format.percent(arc.remaining) + "，" + (arc.unused && arc.label.count == 2 ? L("窗口尚未开始") : Format.resetCountdown(arc.resetsAt, now: now))
        }
        switch item.kind {
        case .duo(let top, let bottom): parts += [window(top), window(bottom)]
        case .single(let top): parts.append(window(top))
        case .coin(let amount, let low):
            parts.append(L("可用余额") + " " + amount + (low ? "，" + L("余额不足") : ""))
            if let split = RingBalanceComposition.make(item.quota) { parts += [L("赠送 {0}", split.granted), L("充值 {0}", split.paid)] }
        case .empty: parts.append(item.quota.queryFailure != nil ? L("额度更新失败") : L("暂无额度数据"))
        }
        if !item.hidden.isEmpty { parts.append(L("另有 {0} 个窗口", item.hidden.count)) }
        return parts.joined(separator: "；")
    }
    /// The healthy arc color of the account's existing cards: the provider color or a neutral.
    static func healthy(_ provider: String) -> Color { Palette.knownCategory(provider) ?? Palette.neutralTint }
}

/// The first shown account as the menu bar ring.
struct MenuRingSnapshot: Equatable {
    var provider: String, kind: QuotaRingKind, label: String
    static func make(settings: Settings, quotas: [Quota], now: Date = Date()) -> MenuRingSnapshot? {
        let preference = settings.menuQuotaRing ?? MenuQuotaRing()
        guard preference.enabled, let quota = quotas.first(where: { !(settings.hideFreeQuotaCards == true && $0.freePlan == true) }),
              let item = QuotaRingModel.items(quota).first, item.kind != .empty else { return nil }
        return MenuRingSnapshot(provider: quota.provider, kind: item.kind, label: QuotaRingModel.label(item.kind, values: preference.values, now: now))
    }
}

/// Balance composition is a share of current funds, not a quota remaining percentage.
struct RingBalanceComposition {
    var granted: String
    var paid: String
    var share: Double
    static func make(_ quota: Quota) -> Self? {
        guard quota.provider == "deepseek", let balance = quota.balances?.first,
              let granted = Double(balance.granted), let paid = Double(balance.toppedUp),
              granted.isFinite, paid.isFinite, (granted + paid).isFinite, granted > 0, paid >= 0 else { return nil }
        let unit = ["CNY", "RMB"].contains(balance.currency.uppercased()) ? "¥" : balance.currency.uppercased() == "USD" ? "$" : balance.currency + " "
        return Self(granted: unit + qoderCreditNumber(granted), paid: unit + qoderCreditNumber(paid), share: granted / (granted + paid))
    }
}

/// The ring itself: top half the short window, bottom half the long one. Colored arcs are
/// the remaining share; both start at the left and grow right, mirroring each other.
struct QuotaRingView: View {
    var kind: QuotaRingKind
    var healthy: Color
    var lineWidth: CGFloat
    var composition: RingBalanceComposition? = nil
    var gap: CGFloat = 2
    var track: Color = Color.primary.opacity(0.13)
    var body: some View {
        Canvas { context, size in
            let side = min(size.width, size.height), center = CGPoint(x: size.width / 2, y: size.height / 2)
            let radius = (side - lineWidth) / 2
            let style = StrokeStyle(lineWidth: lineWidth, lineCap: .round)
            func arc(_ start: Double, _ end: Double) -> Path {
                var path = Path()
                // y points down: increasing angles run visually clockwise.
                path.addArc(center: center, radius: radius, startAngle: .degrees(start), endAngle: .degrees(end), clockwise: end < start)
                return path
            }
            func color(_ arc: QuotaRingArc) -> Color { Palette.quotaBar(arc.used, healthy: healthy) }
            switch kind {
            case .duo(let top, let bottom):
                let half = Double((lineWidth / 2 + gap / 2) / radius) * 180 / .pi, span = 180 - 2 * half
                context.stroke(arc(180 + half, 360 - half), with: .color(track), style: style)
                context.stroke(arc(180 - half, half), with: .color(track), style: style)
                if top.remaining > 0 { context.stroke(arc(180 + half, 180 + half + max(0.4, span * top.remaining / 100)), with: .color(color(top)), style: style) }
                if bottom.remaining > 0 { context.stroke(arc(180 - half, 180 - half - max(0.4, span * bottom.remaining / 100)), with: .color(color(bottom)), style: style) }
            case .single(let top):
                let circle = Path(ellipseIn: CGRect(x: center.x - radius, y: center.y - radius, width: radius * 2, height: radius * 2))
                context.stroke(circle, with: .color(track), style: style)
                if top.remaining >= 99.95 { context.stroke(circle, with: .color(color(top)), style: style) }
                else if top.remaining > 0 { context.stroke(arc(270, 270 + max(0.4, 360 * top.remaining / 100)), with: .color(color(top)), style: style) }
            case .coin(_, let low):
                if let composition {
                    let circle = Path(ellipseIn: CGRect(x: center.x - radius, y: center.y - radius, width: radius * 2, height: radius * 2))
                    context.stroke(circle, with: .color(healthy), lineWidth: lineWidth)
                    if composition.share >= 1 { context.stroke(circle, with: .color(Palette.ok), lineWidth: lineWidth) }
                    else { context.stroke(arc(270, 270 + 360 * composition.share), with: .color(Palette.ok), lineWidth: lineWidth) }
                    break
                }
                let tint = low ? Palette.danger : healthy
                context.fill(Path(ellipseIn: CGRect(x: center.x - side / 2, y: center.y - side / 2, width: side, height: side)), with: .color(tint.opacity(low ? 0.2 : 0.16)))
                let inner = side / 2 - lineWidth * 0.8
                context.stroke(Path(ellipseIn: CGRect(x: center.x - inner, y: center.y - inner, width: inner * 2, height: inner * 2)), with: .color(tint.opacity(low ? 0.6 : 0.45)), lineWidth: max(1, lineWidth * 0.26))
            case .empty:
                let circle = Path(ellipseIn: CGRect(x: center.x - radius, y: center.y - radius, width: radius * 2, height: radius * 2))
                context.stroke(circle, with: .color(.secondary), style: StrokeStyle(lineWidth: lineWidth * 0.72, lineCap: .round, dash: [0.01, lineWidth * 1.45]))
            }
        }
        .accessibilityHidden(true)
    }
}

/// The menu bar icon inside the first account's ring.
struct MenuRingIcon<Icon: View>: View {
    var ring: MenuRingSnapshot?
    @ViewBuilder var icon: Icon
    var body: some View {
        if let ring {
            ZStack {
                QuotaRingView(kind: ring.kind, healthy: QuotaRingModel.healthy(ring.provider), lineWidth: 2.25, gap: 1.5, track: Color.primary.opacity(0.24))
                icon.frame(width: 13, height: 13)
            }.frame(width: 22, height: 22)
        } else {
            icon.frame(width: 22, height: 22)
        }
    }
}

func compactQuotaGroup(_ name: String) -> String {
    switch name.lowercased() {
    case "gemini", "gemini models": return "Geminis"
    case "claude-gpt", "claude and gpt models", "claudes & gpts": return "C&Gs"
    case "gemini-4-argon", "gemini 4 argon": return "Argon"
    default: return name
    }
}

func isQoderCredits(_ quota: Quota) -> Bool { ["qoder", "qoder-cn"].contains(quota.provider) }
func qoderCreditNumber(_ value: Double?) -> String {
    guard let value, value.isFinite else { return "—" }
    return String(format: value.rounded() == value ? "%.0f" : "%.2f", value)
}
func qoderCreditSum(_ quota: Quota) -> String {
    let plan = quota.meters?.first { $0.id == "userQuota" }?.remaining
    let addon = quota.meters?.first { $0.id == "addOnQuota" }?.remaining
    return qoderCreditNumber(plan) + (addon == 0 ? "" : "+" + qoderCreditNumber(addon))
}
