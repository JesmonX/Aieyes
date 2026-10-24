import Foundation

@main struct VerifyLocalization {
    static func main() {
        let previous = UserDefaults.standard.string(forKey: "aieyes.language")
        defer {
            if let previous { UserDefaults.standard.set(previous, forKey: "aieyes.language") }
            else { UserDefaults.standard.removeObject(forKey: "aieyes.language") }
        }
        precondition(AieyesLocale.normalize(nil) == "zh-CN")
        precondition(AieyesLocale.normalize("fr") == "zh-CN")
        AieyesLocale.apply("zh-CN")
        precondition(L("剩余 {0}", "42%") == "剩余 42%")
        precondition(AieyesLocale.month(10) == "10月")
        AieyesLocale.apply("en")
        precondition(L("剩余 {0}", "42%") == "42% remaining")
        precondition(L("读取失败") == "Read failed")
        precondition(L("剩余 {0}", "用户 {1}") == "用户 {1} remaining")
        precondition(L("/Users/用户/设置.json") == "/Users/用户/设置.json")
        precondition(L("token") == "token")
        precondition(L("{0} 个账户", 1) == "1 account")
        precondition(L("{0} 个账户", 2) == "2 accounts")
        precondition(L("{0}", "设置") == "设置")
        precondition(AieyesLocale.month(10) == "Oct")
        let balance = CreditsBalance(hasCredits: true, unlimited: false, balance: "1234.125")
        precondition(Format.creditBalance(balance, estimate: nil) == "Balance 1234.13 credits")
        precondition(Format.creditBalance(balance, estimate: nil).dropFirst(L("余额 ").count) == "1234.13 credits")
        AieyesLocale.apply("zh-CN")
        precondition(L("用户 {1} remaining") == "剩余 用户 {1}")
        precondition(L("Read failed") == "读取失败")
        precondition(L("剩余 {0}", "42%") == "剩余 42%")
        precondition(Format.creditBalance(balance, estimate: nil) == "余额 1234.13 credits")
        for _ in 0..<5 {
            AieyesLocale.apply("en")
            precondition(L("剩余 42%") == "42% remaining")
            precondition(L("untranslated marker") == "untranslated marker")
            AieyesLocale.apply("zh-CN")
            precondition(L("42% remaining") == "剩余 42%")
        }
        UserDefaults.standard.set("en", forKey:"aieyes.language")
        NotificationCenter.default.post(name:UserDefaults.didChangeNotification,object:UserDefaults.standard)
        precondition(AieyesLocale.language == "en" && L("读取失败") == "Read failed", "External preference changes must invalidate the cached language")
        AieyesLocale.apply("zh-CN")
        let date = Date(timeIntervalSince1970:1_792_800_000)
        let reference = DateFormatter();reference.locale = Locale(identifier:"en_US_POSIX");reference.timeZone = .autoupdatingCurrent;reference.dateFormat = "MM/dd HH:mm"
        let expected = reference.string(from:date)
        DispatchQueue.concurrentPerform(iterations:200) { _ in
            precondition(L("读取失败") == "读取失败")
            precondition(Format.date(date.timeIntervalSince1970) == expected)
        }
        DispatchQueue.concurrentPerform(iterations:50) { AieyesLocale.apply($0.isMultiple(of:2) ? "en" : "zh-CN") }
        AieyesLocale.apply("zh-CN")
        precondition(AieyesLocale.language == "zh-CN" && L("读取失败") == "读取失败", "Concurrent preference notifications must not restore an older language")
        precondition(Host().details == Host.detailOptions.map { $0.0 }, "Untranslated default keys must preserve the saved configuration")
        print("Native localization: switch/restore, backend messages, literal arguments, user paths and balance formatting passed")
    }
}
