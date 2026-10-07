import SwiftUI

enum Palette {
    static func adaptive(_ light: UInt32, _ dark: UInt32) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let hex = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
            return NSColor(srgbRed: Double((hex >> 16) & 255) / 255, green: Double((hex >> 8) & 255) / 255, blue: Double(hex & 255) / 255, alpha: 1)
        })
    }
    static var accent: Color {
        let pair: (UInt32, UInt32)
        switch UserDefaults.standard.string(forKey: "appearance.accent") {
        case "blue": pair = (0x2166be, 0x8bbcff)
        case "teal": pair = (0x16745f, 0x65ceae)
        case "purple": pair = (0x8054b7, 0xccabff)
        default: pair = (0x5566d9, 0xa3b3ff)
        }
        return adaptive(pair.0, pair.1)
    }
    static let balanceBlue = adaptive(0x1e5ebf, 0x8cbcff)
    static let ok = adaptive(0x176e58, 0x5cc4a4)
    static let warn = adaptive(0x895712, 0xe0b062)
    static let danger = adaptive(0xb83245, 0xff929c)
    static let cardBorder = adaptive(0x7b879d, 0x8794ad)
    static let radiusCard: CGFloat = 16
    static let radiusOverlay: CGFloat = 20
    static let radiusControl: CGFloat = 9
    static func quota(_ used: Double) -> Color {
        guard used.isFinite else { return .secondary }
        let remaining = min(100, max(0, 100 - used))
        return remaining < 10 ? danger : remaining <= 30 ? warn : accent
    }
    static let colors: [Color] = [accent, .teal, .purple, .orange, .pink, .cyan, .green, .indigo]
    private static var modelHues: [String: Double] = [:]
    private static let modelLock = NSLock()
    static func model(_ name: String) -> Color {
        modelLock.lock()
        let hash = name.utf8.reduce(UInt32(2166136261)) { ($0 ^ UInt32($1)) &* 16777619 }
        var hue = Double(hash % 3600) / 3600
        if let previous = modelHues[name] { hue = previous } else {
            for _ in 0..<24 {
                if !modelHues.values.contains(where: { min(abs($0 - hue), 1 - abs($0 - hue)) < 20.0 / 360 }) { break }
                hue = (hue + 137.508 / 360).truncatingRemainder(dividingBy: 1)
            }
            modelHues[name] = hue
        }
        modelLock.unlock()
        let identityHue = hue
        return Color(nsColor: NSColor(name: nil) { appearance in
            let dark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            return NSColor(hue: identityHue, saturation: dark ? 0.35 : 0.64, brightness: dark ? 0.95 : 0.5, alpha: 1)
        })
    }
}
