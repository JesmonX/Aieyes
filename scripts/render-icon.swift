// Code-native Aieyes artwork, using a 1024-point square with y upwards.
import AppKit
func color(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat, _ a: CGFloat = 1) -> CGColor {
    CGColor(red: r / 255, green: g / 255, blue: b / 255, alpha: a)
}
enum Artwork: String, CaseIterable { case application, brand, template }
for artwork in Artwork.allCases {
let template = artwork == .template
let application = artwork == .application
for size in [16, 32, 48, 64, 128, 256, 512, 1024] {
    let space = CGColorSpaceCreateDeviceRGB()
    let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
                        bytesPerRow: size * 4, space: space,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.scaleBy(x: CGFloat(size) / 1024, y: CGFloat(size) / 1024)
    if !application {
        ctx.translateBy(x: 512, y: 512); ctx.scaleBy(x: 1.4, y: 1.4); ctx.translateBy(x: -512, y: -520)
    }
    if application {
    let tile = CGPath(roundedRect: CGRect(x: 64, y: 64, width: 896, height: 896),
                      cornerWidth: 204, cornerHeight: 204, transform: nil)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 20, color: color(5, 12, 30, 0.28))
    ctx.addPath(tile); ctx.setFillColor(color(14, 25, 50)); ctx.fillPath()
    ctx.restoreGState()
    ctx.saveGState(); ctx.addPath(tile); ctx.clip()
    let bg = CGGradient(colorsSpace: space, colors: [color(13, 23, 44), color(27, 53, 84)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(bg, start: CGPoint(x: 750, y: 100), end: CGPoint(x: 280, y: 960), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
    let glow = CGGradient(colorsSpace: space, colors: [color(49, 158, 199, 0.22), color(49, 158, 199, 0)] as CFArray, locations: [0, 1])!
    ctx.drawRadialGradient(glow, startCenter: CGPoint(x: 400, y: 660), startRadius: 0,
                           endCenter: CGPoint(x: 400, y: 660), endRadius: 520, options: [])
    ctx.restoreGState()
    ctx.addPath(tile); ctx.setStrokeColor(color(156, 205, 234, 0.16)); ctx.setLineWidth(3); ctx.strokePath()
    }
    // Almond silhouette for recognition at small sizes.
    let eye = CGMutablePath()
    eye.move(to: CGPoint(x: 210, y: 520))
    eye.addCurve(to: CGPoint(x: 814, y: 520), control1: CGPoint(x: 367, y: 736), control2: CGPoint(x: 657, y: 736))
    eye.addCurve(to: CGPoint(x: 210, y: 520), control1: CGPoint(x: 657, y: 304), control2: CGPoint(x: 367, y: 304))
    eye.closeSubpath()
    ctx.saveGState()
    ctx.addPath(eye); ctx.setLineWidth(size <= 32 ? 43 : 34); ctx.setLineJoin(.round)
    ctx.replacePathWithStrokedPath(); ctx.clip()
    let outline = application ? [color(76, 188, 228), color(225, 251, 255)] : [color(43, 117, 187), color(57, 168, 214)]
    let ink = CGGradient(colorsSpace: space, colors: outline as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(ink, start: CGPoint(x: 660, y: 340), end: CGPoint(x: 360, y: 710), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
    ctx.restoreGState()
    // Quota gauge iris and a three-bar monitoring signal.
    ctx.setLineWidth(23); ctx.setStrokeColor(application ? color(91, 159, 187, 0.3) : color(55, 126, 167, 0.3))
    ctx.strokeEllipse(in: CGRect(x: 416, y: 424, width: 192, height: 192))
    ctx.setLineCap(.round); ctx.setStrokeColor(application ? color(104, 228, 248) : color(35, 156, 185)); ctx.setLineWidth(25)
    ctx.addArc(center: CGPoint(x: 512, y: 520), radius: 96, startAngle: .pi * 0.22, endAngle: .pi * 1.88, clockwise: false); ctx.strokePath()
    for (x, h) in [(469.0, 42.0), (512.0, 82.0), (555.0, 116.0)] {
        let bar = CGPath(roundedRect: CGRect(x: x - 12, y: 468, width: 24, height: h), cornerWidth: 12, cornerHeight: 12, transform: nil)
        ctx.addPath(bar); ctx.setFillColor(application ? color(218, 249, 255) : color(63, 135, 184)); ctx.fillPath()
    }
    if !template {
    ctx.setFillColor(color(9, 29, 46)); ctx.fillEllipse(in: CGRect(x: 733, y: 299, width: 94, height: 94))
    ctx.setFillColor(color(94, 231, 175)); ctx.fillEllipse(in: CGRect(x: 746, y: 312, width: 68, height: 68))
    }
    if size == 32 && artwork == .brand {
        try Data(bytes: ctx.data!, count: 32 * 32 * 4).write(to: URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("brand.rgba"))
    }
    let bitmap = NSBitmapImageRep(cgImage: ctx.makeImage()!)
    try bitmap.representation(using: .png, properties: [:])!.write(to:
        URL(fileURLWithPath: CommandLine.arguments[1]).appendingPathComponent("\(application ? "" : artwork.rawValue + "-")\(size).png"))
}

}
