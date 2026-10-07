import AppKit

let root = URL(fileURLWithPath: CommandLine.arguments[1])
for theme in ["light", "dark"] {
    for opaque in [false, true] {
        let name = (opaque ? "opaque" : "translucent") + "-panel-" + theme + ".png"
        let bitmap = NSBitmapImageRep(data: try Data(contentsOf: root.appendingPathComponent(name)))!
        var translucentPixels = 0
        for y in stride(from: 0, to: bitmap.pixelsHigh, by: 10) {
            for x in stride(from: 0, to: bitmap.pixelsWide, by: 10) {
                let alpha = bitmap.colorAt(x: x, y: y)!.alphaComponent
                if alpha > 0 && alpha < 0.9 { translucentPixels += 1 }
                if opaque { precondition(alpha > 0.99, "Accessibility fallback must provide its own opaque background: \(name)") }
            }
        }
        if !opaque && !NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency {
            precondition(translucentPixels > 500, "Panel content must allow the system popover background through: \(name)")
        }
    }
}
print("Native panel content is translucent in both themes; accessibility fallback is opaque without an artificial screenshot background")
