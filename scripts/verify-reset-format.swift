import Foundation

@main struct VerifyResetFormat {
    static func main() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let cases: [(Double, String)] = [(1, "1 分后重置"), (59 * 60, "59 分后重置"), (59 * 60 + 1, "1 小时后重置"), (60 * 60, "1 小时后重置"), (24 * 3600, "1 天后重置"), (26 * 3600 + 15 * 60, "1 天 2 小时后重置"), (7 * 86400, "7 天后重置"), (23 * 3600 + 59 * 60, "23 小时 59 分后重置"), (86400 + 60, "1 天后重置"), (86400 + 3600 + 60, "1 天 1 小时后重置")]
        for (delta, expected) in cases {
            precondition(Format.resetCountdown(now.timeIntervalSince1970 + delta, now: now) == expected)
        }
        let old = now.timeIntervalSince1970 - 1
        for duration in [300, 10080] {
            let text = duration == 300 ? "5 小时后重置" : "7 天后重置"
            let elapsedTimes: [TimeInterval] = [0, 60, 86400 * 30]
            for seconds in elapsedTimes {
                let clock = now.addingTimeInterval(seconds)
                precondition(Format.resetCountdown(old, windowMinutes: duration, now: clock) == text)
                precondition(Format.resetDisplayTime(old, windowMinutes: duration, now: clock) == clock.timeIntervalSince1970 + Double(duration) * 60)
                precondition(Format.resetCountdown(nil, windowMinutes: duration, now: clock) == text)
            }
        }
        precondition(Format.resetCountdown(nil, now: now) == "重置时间未知")
        precondition(Format.resetCountdown(nil, windowMinutes: 0, now: now) == "重置时间未知")
        precondition(Format.resetCountdown(.nan, now: now) == "重置时间未知")
        var window = QuotaWindow(name: "5h", usedPercent: 35, windowMinutes: 300, resetsAt: old)
        let before = window.resetsAt
        let dormant = Format.quotaReset(window, now: now)
        precondition(dormant.hasPrefix("5 小时后重置 · "))
        precondition(dormant.range(of: #"^5 小时后重置 · \d{2}/\d{2} \d{2}:\d{2}$"#, options: .regularExpression) != nil)
        precondition(window.resetsAt == before, "Display dates must not alter source snapshots")
        window.resetsAt = now.timeIntervalSince1970 + 120 * 60
        precondition(Format.quotaReset(window, now: now).range(of: #"^2 小时后重置 · \d{2}/\d{2} \d{2}:\d{2}$"#, options: .regularExpression) != nil)
        precondition(Format.resetDisplayTime(window.resetsAt, windowMinutes: 300, now: now) == window.resetsAt)
        precondition(Format.subscription("plus") == "Plus" && Format.subscription(" pro ") == "Pro")
        precondition(Format.subscription("Special Plan") == "Special Plan")
        let full = QuotaWindow(name: "5h", usedPercent: 0, windowMinutes: 300, resetsAt: now.timeIntervalSince1970 + 7200)
        let weekly = QuotaWindow(name: "7d", usedPercent: 20, windowMinutes: 10080, resetsAt: now.timeIntervalSince1970 + 172800)
        let snapshot = try! JSONEncoder().encode([full, weekly])
        for delta: Double in [0, 60, 3600] {
            let clock = now.addingTimeInterval(delta)
            precondition(Format.quotaReset(full, now: clock).hasPrefix("5 小时后重置 · "))
            precondition(Format.resetDisplayTime(full.resetsAt, windowMinutes: 300, usedPercent: 0, now: clock) == clock.timeIntervalSince1970 + 18000)
            precondition(Format.resetDisplayTime(weekly.resetsAt, windowMinutes: 10080, usedPercent: 20, now: clock) == weekly.resetsAt)
        }
        precondition(Format.quotaReset(weekly, now: now.addingTimeInterval(3600)).hasPrefix("1 天 23 小时后重置 · "))
        precondition(try! JSONDecoder().decode([QuotaWindow].self, from: snapshot).map(\.resetsAt) == [full, weekly].map(\.resetsAt))
        for used in [Double.nan, .infinity, -1, 0.001, 20] {
            var active = full; active.usedPercent = used
            precondition(Format.quotaReset(active, now: now).hasPrefix("2 小时后重置 · "))
        }
        for used: Double in [0, 0.001, 10, 0] {
            var active = full; active.usedPercent = used
            precondition(Format.quotaReset(active, now: now.addingTimeInterval(3600)).hasPrefix(used == 0 ? "5 小时后重置 · " : "1 小时后重置 · "))
        }
        var bothFull = weekly; bothFull.usedPercent = 0
        precondition(Format.quotaReset(bothFull, now: now).hasPrefix("7 天后重置 · "))
        var noDuration = full; noDuration.windowMinutes = nil
        precondition(Format.quotaReset(noDuration, now: now).hasPrefix("2 小时后重置 · "))
        print("Native reset durations, dormant cycles, local dates, refresh transitions and subscription labels passed")
    }
}
