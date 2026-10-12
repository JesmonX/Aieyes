import Foundation

enum WakeupTime {
    enum Status: String { case ready, pending, invalidTime, invalidZone, unavailable }
    struct Preview {
        var date: Date?
        var status: Status
        var approximate = false
        var label: String {
            if let date {
                return "本地 " + date.formatted(date: .abbreviated, time: .shortened) + (approximate ? "（按当前偏移估算）" : "")
            }
            switch status {
            case .pending: return "目标时区待读取"
            case .invalidTime: return "请输入 HH:mm"
            default: return "时区换算待确认，以部署状态为准"
            }
        }
    }
    private static let offsetPattern = try! NSRegularExpression(pattern: #"(?:^|\s)(?:UTC|GMT)?([+-])([0-9]{2}):?([0-9]{2})$"#)

    static func next(_ time: String, zone name: String?, now: Date = Date()) -> Preview {
        guard let name = name?.trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty else { return Preview(status: .pending) }
        guard time.utf8.count == 5, time.range(of: #"^([01][0-9]|2[0-3]):[0-5][0-9]$"#, options: .regularExpression) != nil else { return Preview(status: .invalidTime) }
        let parts = time.split(separator: ":").map { Int($0)! }
        let zone: TimeZone
        var approximate = false
        if name.contains("/") || name == "UTC" || name == "GMT" {
            guard let parsed = TimeZone(identifier: name) else { return Preview(status: .invalidZone) }
            zone = parsed
        } else {
            let value = name as NSString
            guard let match = offsetPattern.firstMatch(in: name, range: NSRange(location: 0, length: value.length)) else { return Preview(status: .invalidZone) }
            let hours = Int(value.substring(with: match.range(at: 2)))!, minutes = Int(value.substring(with: match.range(at: 3)))!
            let seconds = (hours * 60 + minutes) * 60
            guard minutes < 60, seconds <= 18 * 3600,
                  let parsed = TimeZone(secondsFromGMT: seconds * (value.substring(with: match.range(at: 1)) == "-" ? -1 : 1)) else { return Preview(status: .invalidZone) }
            zone = parsed; approximate = true
        }
        guard now.timeIntervalSince1970.isFinite else { return Preview(status: .unavailable) }
        var calendar = Calendar(identifier: .gregorian); calendar.timeZone = zone
        let start = floor(now.timeIntervalSince1970 / 60) * 60
        // A real minute timeline skips DST gaps and includes both occurrences of a repeated hour.
        for minute in 1...2880 {
            let candidate = Date(timeIntervalSince1970: start + Double(minute * 60))
            let local = calendar.dateComponents([.hour, .minute], from: candidate)
            if local.hour == parts[0], local.minute == parts[1] {
                return Preview(date: candidate, status: .ready, approximate: approximate)
            }
        }
        return Preview(status: .unavailable)
    }
}
