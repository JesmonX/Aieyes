import SwiftUI

enum Palette {
    static func adaptive(_ light: UInt32, _ dark: UInt32) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let hex = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
            return NSColor(srgbRed: Double((hex >> 16) & 255) / 255, green: Double((hex >> 8) & 255) / 255, blue: Double(hex & 255) / 255, alpha: 1)
        })
    }
    static let accent = adaptive(0x5566d9, 0xa3b3ff)
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
    static func model(_ name: String) -> Color {
        let hash = name.utf8.reduce(UInt32(2166136261)) { ($0 ^ UInt32($1)) &* 16777619 }
        return Color(nsColor: NSColor(name: nil) { appearance in
            let dark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            return NSColor(hue: Double(hash % 3600) / 3600, saturation: dark ? 0.35 : 0.64, brightness: dark ? 0.95 : 0.5, alpha: 1)
        })
    }
}
