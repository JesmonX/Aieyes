import Foundation

@main struct VerifyResetFormat {
    struct Fixture: Decodable {
        struct Duration: Decodable { var seconds: Double, text: String }
        var now: Double, durations: [Duration]
    }
    static func main() throws {
        let fixture = try JSONDecoder().decode(Fixture.self, from: Data(contentsOf: URL(fileURLWithPath: "scripts/fixtures/reset-display.json")))
        let now = Date(timeIntervalSince1970: fixture.now)
        for row in fixture.durations {
            precondition(Format.resetCountdown(now.timeIntervalSince1970 + row.seconds, now: now) == row.text)
        }
        let old = now.timeIntervalSince1970 - 1
        for duration in [300, 10080] {
            let elapsedTimes: [TimeInterval] = [0, 60, 86400 * 30]
            for seconds in elapsedTimes {
                let clock = now.addingTimeInterval(seconds)
                let expired = QuotaWindow(name: "quota", usedPercent: 0, windowMinutes: duration, resetsAt: old)
                precondition(Format.quotaReset(expired, now: clock) == "确认重置中")
                precondition(Format.resetDisplayTime(old, now: clock) == nil)
                var missing = expired; missing.resetsAt = nil
                precondition(Format.quotaReset(missing, now: clock) == (duration == 300 ? "5h" : "7d"))
                missing.usedPercent = 20
                precondition(Format.quotaReset(missing, now: clock) == "重置时间未知")
            }
        }
        for stamp: Double? in [nil, 0, -1, .nan, .infinity] {
            precondition(Format.resetCountdown(stamp, now: now) == "重置时间未知")
            for used in [Double.nan, .infinity, -1, 0, 0.001, 20, 100] {
                let missing = QuotaWindow(name: "5h", usedPercent: used, windowMinutes: 300, resetsAt: stamp)
                precondition(Format.quotaReset(missing, now: now) == (used == 0 ? "5h" : "重置时间未知"))
            }
        }
        precondition(Format.quotaReset(QuotaWindow(name: "quota", usedPercent: 0, windowMinutes: nil, resetsAt: nil), now: now) == "重置时间未知")
        precondition(Format.resetCountdown(now.timeIntervalSince1970, now: now) == "确认重置中")
        var window = QuotaWindow(name: "5h", usedPercent: 35, windowMinutes: 300, resetsAt: old)
        precondition(Format.quotaReset(window, now: now) == "确认重置中")
        window.resetsAt = now.timeIntervalSince1970 + 120 * 60
        let nativeClock = DateFormatter.dateFormat(fromTemplate: "j", options: 0, locale: .current) ?? "HH"
        let expectedDate = DateFormatter()
        expectedDate.locale = AieyesLocale.locale
        expectedDate.timeZone = .current
        expectedDate.setLocalizedDateFormatFromTemplate("MMdd" + (nativeClock.contains("a") ? "hhmma" : "HHmm"))
        let resetDate = expectedDate.string(from: Date(timeIntervalSince1970: window.resetsAt!))
        precondition(Format.quotaReset(window, now: now) == "2 小时后重置 · " + resetDate, "Reset dates must follow the user's native clock and localized date format")
        precondition(Format.resetDisplayTime(window.resetsAt, now: now) == window.resetsAt)
        precondition(Format.subscription("plus") == "Plus" && Format.subscription(" pro ") == "Pro")
        precondition(Format.subscription("Special Plan") == "Special Plan")
        let full = QuotaWindow(name: "5h", usedPercent: 0, windowMinutes: 300, resetsAt: now.timeIntervalSince1970 + 7200)
        let weekly = QuotaWindow(name: "7d", usedPercent: 20, windowMinutes: 10080, resetsAt: now.timeIntervalSince1970 + 172800)
        let countdowns: [(Double, String)] = [(0, "2 小时"), (60, "1 小时 59 分"), (3600, "1 小时")]
        for (delta, countdown) in countdowns {
            let clock = now.addingTimeInterval(delta)
            let display = Format.quotaReset(full, now: clock)
            precondition(display.hasPrefix(countdown + "后重置 · "))
            precondition(display.components(separatedBy: " · ").last == Format.quotaReset(full, now: now).components(separatedBy: " · ").last)
            precondition(Format.resetDisplayTime(full.resetsAt, now: clock) == full.resetsAt)
            precondition(Format.resetDisplayTime(weekly.resetsAt, now: clock) == weekly.resetsAt)
        }
        precondition(Format.quotaReset(weekly, now: now.addingTimeInterval(3600)).hasPrefix("1 天 23 小时后重置 · "))
        for used in [Double.nan, .infinity, -1, 0, 0.001, 20, 100] {
            var active = full; active.usedPercent = used
            precondition(Format.quotaReset(active, now: now).hasPrefix("2 小时后重置 · "))
        }
        var bothFull = weekly; bothFull.usedPercent = 0
        precondition(Format.quotaReset(bothFull, now: now).hasPrefix("2 天后重置 · "))
        var noDuration = full; noDuration.windowMinutes = nil
        precondition(Format.quotaReset(noDuration, now: now).hasPrefix("2 小时后重置 · "))
        precondition(Format.quotaReset(full, now: now.addingTimeInterval(7200)) == "确认重置中")
        var refreshed = full; refreshed.resetsAt = now.timeIntervalSince1970 + 9000
        precondition(Format.quotaReset(refreshed, now: now.addingTimeInterval(7200)).hasPrefix("30 分后重置 · "))
        // Local Codex returned 0% used at 11:56:51 but an explicit 13:00:06 reset.
        let observedNow = ISO8601DateFormatter().date(from: "2026-10-09T11:56:51+08:00")!
        let observed = QuotaWindow(name: "5h", usedPercent: 0, windowMinutes: 300, resetsAt: 1_791_522_006)
        precondition(Format.resetDisplayTime(observed.resetsAt, now: observedNow) == 1_791_522_006)
        precondition(Format.quotaReset(observed, now: observedNow).hasPrefix("1 小时 4 分后重置 · "))
        precondition(Format.quotaReset(observed, now: observedNow.addingTimeInterval(60)).hasPrefix("1 小时 3 分后重置 · "))
        print("Native reset durations, source dates, zero usage, expired/unknown times and refresh transitions passed (native clock: \(nativeClock))")
    }
}
