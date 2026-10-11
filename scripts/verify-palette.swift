import SwiftUI

@main struct VerifyPalette {
    static func luminance(_ color: NSColor) -> Double {
        let rgb = color.usingColorSpace(.sRGB)!
        return zip([rgb.redComponent, rgb.greenComponent, rgb.blueComponent], [0.2126, 0.7152, 0.0722]).reduce(0) { sum, pair in
            let v = pair.0
            return sum + (v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)) * pair.1
        }
    }
    static func contrast(_ foreground: Color, _ background: NSColor) -> Double {
        let a = luminance(NSColor(foreground)), b = luminance(background)
        return (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }
    static func main() {
        for dark in [false, true] {
            NSAppearance(named: dark ? .darkAqua : .aqua)!.performAsCurrentDrawingAppearance {
                for preset in Palette.chartPresets {
                    for level in ["high", "low"] {
                        let backgroundHex = level == "high" ? (dark ? 0x2e3a52 : 0xe1e8f6) : (dark ? 0x29292e : 0xffffff)
                        let background = NSColor(srgbRed: Double((backgroundHex >> 16) & 255) / 255, green: Double((backgroundHex >> 8) & 255) / 255, blue: Double(backgroundHex & 255) / 255, alpha: 1)
                        for hex in preset.colors(dark: dark, contrast: level) {
                            let ratio = contrast(Palette.adaptive(hex, hex), background)
                            precondition(level == "high" ? ratio >= 3 : (2.1...2.3).contains(ratio), "Chart variant contrast: \(preset.id) \(level) dark=\(dark) ratio=\(ratio)")
                        }
                        let colors = (0..<preset.lightHigh.count).map { Palette.model("preset-model-\($0)", preset: preset.id, contrast: level) }
                        precondition(Set(colors.map { NSColor($0).usingColorSpace(.sRGB)!.description }).count == colors.count, "Preset slots collide before palette is full")
                    }
                }
                // Conservative opaque bounds for the captured neutral surfaces.
                let backgrounds = dark ? [0x222222, 0x2e3a52] : [0xffffff, 0xe1e8f6]
                for hex in backgrounds {
                    let background = NSColor(srgbRed: Double((hex >> 16) & 255) / 255, green: Double((hex >> 8) & 255) / 255, blue: Double(hex & 255) / 255, alpha: 1)
                    for (name, color) in [("ok", Palette.ok), ("warn", Palette.warn), ("danger", Palette.danger)] {
                        precondition(contrast(color, background) >= 4.5, "Status text contrast below 4.5:1: \(name) dark=\(dark) background=\(hex) ratio=\(contrast(color, background))")
                    }
                    for index in 0..<10000 {
                        precondition(contrast(Palette.model("contrast-model-\(index)", preset: "classic", contrast: "high"), background) >= 3, "Model graphic contrast below 3:1")
                    }
                }
            }
        }
        print("Native status text, 10,000 model colors and all 20 chart palette variants pass contrast and category bounds")
    }
}
