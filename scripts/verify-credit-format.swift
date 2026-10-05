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
        print("Native credit decimal formatting passed")
    }
}
