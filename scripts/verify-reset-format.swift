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
        print("Native reset durations, dormant cycles, local dates, refresh transitions and subscription labels passed")
    }
}
