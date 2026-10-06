import Foundation
@main struct VerifyCreditFormat {
    static func main() {
        let cases: [(String?,String)] = [
            ("1234.125","1234.13"),("1.005","1.01"),("10","10.00"),
            ("0","0.00"),("-0.004","0.00"),("-1.005","-1.01"),
            ("1e-3","0.00"),("9.999","10.00"),(nil,"—"),("","—"),
            ("NaN","—"),("12bad","—"),("Infinity","—")
        ]
        for (input,expected) in cases {
            precondition(Format.credits(input)==expected,"Credit formatting: \(input ?? "nil")")
        }
        let compactCases: [(Double, String)] = [(0,"0"),(999,"999"),(1000,"1.0K"),(999_949,"999.9K"),(999_950,"1.00M"),(1_000_000,"1.00M"),(999_995_000,"1.00B"),(2_400_000_000,"2.40B")]
        for (input, expected) in compactCases { precondition(Format.compact(input) == expected) }
        var estimate = QuotaEstimate(valuePer1000: 200, id: "e", accountKey: "codex:a", windowId: "credits", windowName: "Credits", sourceIds: [], sourceNames: [], status: "completed", reason: "", startedAt: 0, checkpointAt: 0, endedAt: nil, consumedPercent: 0, cost: 2, totalTokens: 100, pricedTokens: 100, weeklyValue: nil, calculationNote: "")
        let balance = CreditsBalance(hasCredits: true, unlimited: false, balance: "1234.125")
        precondition(Format.creditBalance(balance, estimate: estimate) == "余额 1234.13 credits ≈ $246.83 USD")
        precondition(Format.creditBalance(CreditsBalance(hasCredits: true, unlimited: false, balance: "0"), estimate: estimate) == "余额 0.00 credits ≈ $0.00 USD")
        precondition(Format.creditBalance(CreditsBalance(hasCredits: true, unlimited: true, balance: "100"), estimate: estimate) == "余额 无限")
        precondition(Format.creditBalance(nil, estimate: estimate) == "余额 —")
        estimate.status = "pending"
        precondition(!Format.creditBalance(balance, estimate: estimate).contains("≈"))
        estimate.status = "active"; estimate.valuePer1000 = nil
        precondition(!Format.creditBalance(balance, estimate: estimate).contains("≈"))
        print("Native credit balance valuation, decimal formatting and compact token boundaries passed")
    }
}
