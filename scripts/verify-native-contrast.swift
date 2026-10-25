// Verify rendered surfaces, rather than only the presence of an accessibility branch.
import AppKit

let root = URL(fileURLWithPath: CommandLine.arguments[1])
for scenario in ["empty", "single", "multi", "long", "failure", "capacity"] {
    for dark in [false, true] {
        let theme = dark ? "dark" : "light"
        let path = root.appendingPathComponent("\(scenario)/increased-contrast-\(theme).png")
        let bitmap = NSBitmapImageRep(data: try Data(contentsOf: path))!
        let palette = dark ? [135, 148, 173] : [123, 135, 157]
        let reference = NSColor(srgbRed: Double(palette[0]) / 255, green: Double(palette[1]) / 255, blue: Double(palette[2]) / 255, alpha: 1).usingColorSpace(bitmap.colorSpace)!
        let expected = [reference.redComponent, reference.greenComponent, reference.blueComponent].map { Int(($0 * 255).rounded()) }
        var borderPixels = 0
        for y in stride(from: 0, to: bitmap.pixelsHigh, by: 2) {
            for x in stride(from: 0, to: bitmap.pixelsWide, by: 2) {
                // Capture maps sRGB palette values into the display's profile.
                // Compare encoded channels against the same mapped reference.
                let color = bitmap.colorAt(x: x, y: y)!
                let rgb = [color.redComponent, color.greenComponent, color.blueComponent].map { Int(($0 * 255).rounded()) }
                if zip(rgb, expected).allSatisfy({ abs($0 - $1) <= 1 }) { borderPixels += 1 }
                precondition(color.alphaComponent > 0.99, "Accessible surface must be opaque: \(path.path)")
            }
        }
        precondition(borderPixels > 100, "Missing solid contrast border: \(path.path)")
    }
}
print("Native contrast screenshots contain opaque surfaces and solid palette borders in both themes")
