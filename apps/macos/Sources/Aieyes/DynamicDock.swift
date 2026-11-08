import AppKit
import SwiftUI
import Combine

enum DynamicDockStyle: String, CaseIterable, Identifiable {
    case eyes, rings, pet
    var id: String { rawValue }
    var title: String { switch self { case .eyes: return L("灵动双眼"); case .rings: return L("额度仪表"); case .pet: return L("桌宠图标") } }
}

final class DynamicDockView: NSView {
    var snapshot = DisplaySnapshot()
    var style = DynamicDockStyle.eyes
    var elapsed: TimeInterval = 0
    var petImage: CGImage?
    override var isOpaque: Bool { false }
    override func draw(_ dirtyRect: NSRect) { DockArtwork.draw(snapshot: snapshot, style: style, elapsed: elapsed, petImage: petImage, in: bounds) }
}

enum DockArtwork {
    static func draw(snapshot: DisplaySnapshot, style: DynamicDockStyle, elapsed: TimeInterval, petImage: CGImage? = nil, in bounds: NSRect) {
        NSGraphicsContext.saveGraphicsState()
        defer { NSGraphicsContext.restoreGraphicsState() }
        let transform = NSAffineTransform()
        transform.translateX(by: bounds.minX, yBy: bounds.minY)
        transform.scaleX(by: bounds.width / 128, yBy: bounds.height / 128); transform.concat()
        let phase = snapshot.phase
        let tint = NSColor(phase.tint)
        let tile = NSBezierPath(roundedRect: NSRect(x: 12, y: 12, width: 104, height: 104), xRadius: 25, yRadius: 25)
        NSGradient(starting: NSColor(calibratedRed: 0.19, green: 0.22, blue: 0.34, alpha: 1), ending: NSColor(calibratedRed: 0.07, green: 0.09, blue: 0.15, alpha: 1))?.draw(in: tile, angle: -70)
        NSColor.white.withAlphaComponent(0.14).setStroke(); tile.lineWidth = 1; tile.stroke()
        switch style {
        case .eyes: eyes(phase: phase, elapsed: elapsed, tint: tint)
        case .rings: rings(snapshot: snapshot, tint: tint, elapsed: elapsed)
        case .pet:
            if let petImage {
                NSGraphicsContext.current?.imageInterpolation = .high
                NSImage(cgImage: petImage, size: .zero).draw(in: NSRect(x: 11, y: 15, width: 106, height: 102), from: .zero, operation: .sourceOver, fraction: 1)
                let dot = NSBezierPath(ovalIn: NSRect(x: 93, y: 17, width: 12, height: 12))
                tint.setFill(); dot.fill()
            } else { eyes(phase: phase, elapsed: elapsed, tint: tint) }
        }
        if snapshot.isDemo {
            let badge = NSBezierPath(roundedRect: NSRect(x: 16, y: 98, width: 35, height: 13), xRadius: 4, yRadius: 4)
            NSColor(calibratedRed: 0.99, green: 0.72, blue: 0.33, alpha: 1).setFill(); badge.fill()
            text("DEMO", rect: NSRect(x: 16, y: 100, width: 35, height: 9), size: 7, color: .black)
        }
    }
    private static func eyes(phase: DisplayPhase, elapsed: TimeInterval, tint: NSColor) {
        let moving = phase.isAnimated
        let shift = moving ? sin(elapsed * 2.2) * 4 : 0
        let blink = moving && elapsed.truncatingRemainder(dividingBy: 5.8) > 5.55
        for x in [29.0, 70.0] {
            if phase == .complete {
                let line = NSBezierPath(); line.move(to: NSPoint(x: x, y: 61))
                line.curve(to: NSPoint(x: x + 27, y: 61), controlPoint1: NSPoint(x: x + 5, y: 84), controlPoint2: NSPoint(x: x + 22, y: 84))
                NSColor.white.setStroke(); line.lineWidth = 6; line.lineCapStyle = .round; line.stroke()
            } else {
                let height = blink ? 4.0 : phase == .idle ? 20.0 : 35.0
                let eye = NSBezierPath(roundedRect: NSRect(x: x, y: 69 - height / 2, width: 28, height: height), xRadius: 14, yRadius: 14)
                NSColor.white.withAlphaComponent(phase == .unknown ? 0.45 : 0.96).setFill(); eye.fill()
                if !blink {
                    if phase == .failed {
                        let cross = NSBezierPath()
                        cross.move(to: NSPoint(x: x + 9, y: 63)); cross.line(to: NSPoint(x: x + 19, y: 75))
                        cross.move(to: NSPoint(x: x + 9, y: 75)); cross.line(to: NSPoint(x: x + 19, y: 63))
                        tint.setStroke(); cross.lineWidth = 3; cross.lineCapStyle = .round; cross.stroke()
                    } else {
                        let y = phase == .thinking ? 69.0 : phase == .needsInput ? 67.0 : 62.0
                        let pupil = NSBezierPath(ovalIn: NSRect(x: x + 7 + shift, y: y, width: 13, height: 14))
                        NSColor(calibratedRed: 0.20, green: 0.24, blue: 0.43, alpha: 1).setFill(); pupil.fill()
                        NSColor.white.setFill(); NSBezierPath(ovalIn: NSRect(x: x + 10 + shift, y: y + 8, width: 4, height: 4)).fill()
                    }
                }
            }
        }
        let baseline = NSRect(x: 36, y: 28, width: 56, height: 10)
        let capsule = NSBezierPath(roundedRect: baseline, xRadius: 5, yRadius: 5)
        tint.withAlphaComponent(0.45).setFill(); capsule.fill()
        if moving {
            let width = phase == .needsInput ? 26.0 : 16.0
            let offset = (sin(elapsed * 2.4) + 1) / 2 * (56 - width)
            tint.blended(withFraction: 0.38, of: .white)?.setFill()
            NSBezierPath(roundedRect: NSRect(x: 36 + offset, y: 28, width: width, height: 10), xRadius: 5, yRadius: 5).fill()
        } else {
            let label = phase == .complete ? "✓" : phase == .failed ? "!" : phase == .unknown ? "?" : "· · ·"
            text(label, rect: NSRect(x: 36, y: 24, width: 56, height: 18), size: 14, color: .white)
        }
    }
    private static func rings(snapshot: DisplaySnapshot, tint: NSColor, elapsed: TimeInterval) {
        for index in 0..<2 {
            let radius = index == 0 ? 40.0 : 29.0
            let center = NSPoint(x: 64, y: 69)
            let circle = NSBezierPath(ovalIn: NSRect(x: 64 - radius, y: 69 - radius, width: radius * 2, height: radius * 2))
            NSColor.white.withAlphaComponent(0.12).setStroke(); circle.lineWidth = 6; circle.stroke()
            guard snapshot.quotas.indices.contains(index) else { continue }
            let quota = snapshot.quotas[index]
            let stale = quota.stale || (quota.resetsAt.map { $0 <= Date().timeIntervalSince1970 } ?? false)
            let color: NSColor = stale ? .gray : quota.remaining <= 15 ? .systemRed : index == 0 ? NSColor(calibratedRed: 0.55, green: 0.62, blue: 1, alpha: 1) : NSColor(calibratedRed: 0.35, green: 0.84, blue: 0.74, alpha: 1)
            let arc = NSBezierPath()
            arc.appendArc(withCenter: center, radius: radius, startAngle: 90, endAngle: 90 - 360 * quota.fraction, clockwise: true)
            color.setStroke(); arc.lineWidth = 6; arc.lineCapStyle = .round; arc.stroke()
        }
        let value = snapshot.quotas.first.flatMap { quota in
            quota.stale || (quota.resetsAt.map { $0 <= Date().timeIntervalSince1970 } ?? false) ? nil : String(format: "%.0f", quota.remaining)
        } ?? "—"
        text(value, rect: NSRect(x: 36, y: 59, width: 56, height: 26), size: 23, color: .white)
        text("%", rect: NSRect(x: 49, y: 46, width: 30, height: 12), size: 10, color: .lightGray)
        text(snapshot.quotas.isEmpty ? "AIEYES" : snapshot.quotas.map(\.title).joined(separator: " · "), rect: NSRect(x: 24, y: 16, width: 80, height: 12), size: 9, color: .white)
        if snapshot.phase.isAnimated {
            tint.withAlphaComponent(0.5 + sin(elapsed * 3) * 0.3).setFill()
            NSBezierPath(ovalIn: NSRect(x: 102, y: 14, width: 7, height: 7)).fill()
        }
    }
    private static func text(_ string: String, rect: NSRect, size: CGFloat, color: NSColor) {
        let paragraph = NSMutableParagraphStyle(); paragraph.alignment = .center
        (string as NSString).draw(in: rect, withAttributes: [.font: NSFont.systemFont(ofSize: size, weight: .semibold), .foregroundColor: color, .paragraphStyle: paragraph])
    }
}

@MainActor final class DynamicDockController {
    private let model: AppModel
    private let store: DisplayStore
    private let presence: DockPresence
    private let view = DynamicDockView(frame: NSRect(x: 0, y: 0, width: 128, height: 128))
    private var subscriptions = Set<AnyCancellable>()
    private var timer: Timer?
    private var epoch = Date()
    private var suspended = false
    private var petFrames: [CGImage] = []
    private var petState = PetState.idle
    private var petKey = ""
    private(set) var animating = false
    private(set) var drawCount = 0

    init(model: AppModel, presence: DockPresence) {
        self.model = model; store = model.displays; self.presence = presence
        store.$snapshot.sink { [weak self] value in self?.view.snapshot = value; self?.sync() }.store(in: &subscriptions)
        model.live.$asset.sink { [weak self] _ in DispatchQueue.main.async { self?.sync() } }.store(in: &subscriptions)
        NotificationCenter.default.publisher(for: UserDefaults.didChangeNotification).sink { [weak self] _ in self?.sync() }.store(in: &subscriptions)
        NotificationCenter.default.publisher(for: .NSProcessInfoPowerStateDidChange).sink { [weak self] _ in self?.sync() }.store(in: &subscriptions)
        NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification).sink { [weak self] _ in self?.sync() }.store(in: &subscriptions)
        for name in [NSWorkspace.screensDidWakeNotification, NSWorkspace.sessionDidBecomeActiveNotification] {
            NSWorkspace.shared.notificationCenter.publisher(for: name).sink { [weak self] _ in self?.suspended = false; self?.sync() }.store(in: &subscriptions)
        }
        for name in [NSWorkspace.screensDidSleepNotification, NSWorkspace.sessionDidResignActiveNotification] {
            NSWorkspace.shared.notificationCenter.publisher(for: name).sink { [weak self] _ in self?.suspended = true; self?.sync() }.store(in: &subscriptions)
        }
        sync()
    }
    static func shouldAnimate(enabled: Bool, motion: Bool, reduced: Bool, lowPower: Bool, suspended: Bool, style: DynamicDockStyle, phase: DisplayPhase) -> Bool {
        enabled && motion && !reduced && !lowPower && !suspended && (style == .pet || phase.isAnimated)
    }
    private func sync() {
        let defaults = UserDefaults.standard
        let enabled = defaults.bool(forKey: DisplayPreferences.dockEnabled)
        presence.setPersistent(enabled)
        guard enabled else { restore(); return }
        view.style = DynamicDockStyle(rawValue: defaults.string(forKey: DisplayPreferences.dockStyle) ?? "") ?? .eyes
        if NSApp.dockTile.contentView !== view { NSApp.dockTile.contentView = view; NSApp.dockTile.showsApplicationBadge = false }
        let count = view.snapshot.waitingCount + view.snapshot.failedCount
        NSApp.dockTile.badgeLabel = defaults.object(forKey: DisplayPreferences.dockBadge) as? Bool != false && count > 0 ? (count > 99 ? "99+" : String(count)) : nil
        if view.style == .pet { loadPet() }
        let wanted = Self.shouldAnimate(enabled: enabled, motion: defaults.object(forKey: DisplayPreferences.animations) as? Bool != false,
            reduced: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion, lowPower: ProcessInfo.processInfo.isLowPowerModeEnabled, suspended: suspended, style: view.style, phase: view.snapshot.phase)
        if wanted != animating {
            timer?.invalidate(); timer = nil; animating = wanted
            if wanted {
                let tick = Timer(timeInterval: 0.125, repeats: true) { [weak self] _ in MainActor.assumeIsolated { self?.draw() } }
                tick.tolerance = 0.025; RunLoop.main.add(tick, forMode: .common); timer = tick
            }
        }
        draw()
    }
    private func loadPet() {
        let phase = view.snapshot.phase
        let row: Int = switch phase { case .idle: 0; case .working: 1; case .needsInput: 3; case .complete: 4; case .failed: 5; case .unknown: 6; case .tool: 7; case .thinking: 8 }
        let durations = CoreContract.petFrameDurations[row]
        petState = PetState(behavior: phase.rawValue, label: phase.title, row: row, frames: durations.count, frameMs: 100, key: phase.rawValue, frameDurationsMs: durations)
        guard let asset = model.live.asset else { petFrames = []; petKey = ""; return }
        let key = asset.contentHash + ":" + String(row)
        guard key != petKey else { return }; petKey = key; petFrames = []; epoch = Date()
        PetSprites.queue.async { [weak self] in
            let frames = PetSprites.frames(for: asset, row: row)
            DispatchQueue.main.async { [weak self] in guard let self, self.petKey == key else { return }; self.petFrames = frames; self.draw() }
        }
    }
    private func draw() {
        view.elapsed = animating ? Date().timeIntervalSince(epoch) : 0
        let index = petState.frame(elapsed: view.elapsed, reduced: !animating) - petState.row * 8
        view.petImage = petFrames.indices.contains(index) ? petFrames[index] : nil
        view.needsDisplay = true; NSApp.dockTile.display(); drawCount += 1
    }
    private func restore() {
        timer?.invalidate(); timer = nil; animating = false
        guard NSApp.dockTile.contentView === view else { return }
        NSApp.dockTile.contentView = nil; NSApp.dockTile.badgeLabel = nil; NSApp.dockTile.display()
    }
    func stop() { subscriptions.removeAll(); restore(); presence.setPersistent(false) }
}

struct DockArtworkPreview: NSViewRepresentable {
    var snapshot: DisplaySnapshot
    var style: DynamicDockStyle
    var elapsed: TimeInterval = 0
    var petImage: CGImage?
    func makeNSView(context: Context) -> DynamicDockView { DynamicDockView() }
    func updateNSView(_ view: DynamicDockView, context: Context) {
        view.snapshot = snapshot; view.style = style; view.elapsed = elapsed; view.petImage = petImage; view.needsDisplay = true
        view.setAccessibilityElement(true); view.setAccessibilityRole(.image); view.setAccessibilityLabel(style.title + " · " + snapshot.text(snapshot.phase.title))
    }
}
