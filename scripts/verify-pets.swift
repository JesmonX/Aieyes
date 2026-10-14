import Foundation
import AppKit
@main struct VerifyPets {
    @MainActor static func main() throws {
        let idleCadence = [1, 279, 281, 391, 501, 641, 781, 1101].map { PetState.idle.frame(elapsed: Double($0) / 1000, reduced: false) }
        precondition(idleCadence == [0,0,1,2,3,4,5,0])
        precondition(PetState.unknown.frameMs == PetState.idle.frameMs)
        for (row, values) in CoreContract.petFrameDurations.enumerated() {
            let pet = PetState(behavior: "fixture", label: "Fixture", row: row, frames: values.count, frameMs: 100, key: "fixture", frameDurationsMs: values)
            precondition(pet.frame(elapsed: 0, reduced: false) == row * 8)
            precondition(pet.frame(elapsed: 100, reduced: true) == row * 8)
            let cycle = values.reduce(0, +)
            for repetition in 0..<4 {
                var boundary = repetition * cycle
                for (index, duration) in values.enumerated() {
                    precondition(pet.frame(elapsed: Double(boundary + 1) / 1000, reduced: false) == row * 8 + index)
                    precondition(pet.frame(elapsed: Double(boundary + duration - 1) / 1000, reduced: false) == row * 8 + index)
                    boundary += duration
                }
            }
        }
        let legacyShort = PetState(behavior: "fixture", label: "Fixture", row: 0, frames: 2, frameMs: 90, key: "fixture")
        precondition(legacyShort.frame(elapsed: 0.091, reduced: false) == 1 && legacyShort.frame(elapsed: 0.181, reduced: false) == 0)
        var emptyDurations = PetState.idle
        emptyDurations.frameDurationsMs = []
        precondition(emptyDurations.frame(elapsed: 0.15, reduced: false) == 0, "Empty legacy durations share the row timing")
        emptyDurations.frameDurationsMs = [0]
        precondition(emptyDurations.frame(elapsed: 0.15, reduced: false) == 1, "Malformed explicit durations fall back to frameMs")
        let area = NSRect(x: 0, y: 24, width: 1440, height: 850)
        precondition(PetWindowGeometry.origin(frame: NSRect(x: 48, y: 200, width: 144, height: 168), areas: [area], snap: true).x == 0)
        precondition(PetWindowGeometry.origin(frame: NSRect(x: 49, y: 200, width: 144, height: 168), areas: [area], snap: true).x == 49)
        precondition(PetWindowGeometry.origin(frame: NSRect(x: 1250, y: 200, width: 144, height: 168), areas: [area], snap: true).x == 1296)
        let otherArea = NSRect(x: -1280, y: 0, width: 1280, height: 800)
        let onOtherScreen = PetWindowGeometry.origin(frame: NSRect(x: -1250, y: 40, width: 288, height: 336), areas: [area, otherArea], snap: true, preferredPoint: NSPoint(x: -1200, y: 80))
        precondition(onOtherScreen == NSPoint(x: -1280, y: 0), "Snapping uses each display's work area and logical points at every pet scale")
        let libraryViewport = CGRect(x: 200, y: 100, width: 360, height: 260)
        let settingsViewport = CGRect(x: 180, y: 80, width: 400, height: 400)
        precondition(PetPreviewVisibility.isVisible(CGRect(x: 204, y: 112, width: 352, height: 92), viewport: libraryViewport, enclosing: settingsViewport))
        precondition(!PetPreviewVisibility.isVisible(CGRect(x: 204, y: 364, width: 352, height: 92), viewport: libraryViewport, enclosing: settingsViewport), "Rows beyond the library viewport must stop")
        precondition(!PetPreviewVisibility.isVisible(CGRect(x: 204, y: 520, width: 352, height: 92), viewport: libraryViewport.offsetBy(dx: 0, dy: 420), enclosing: settingsViewport), "The entire library scrolled outside Settings must stop")
        let clock = PetAnimationClock()
        var clockState = PetState.idle
        clockState.key = "cadence"
        clock.configure(clockState, playing: true, reduced: false)
        let began = ProcessInfo.processInfo.systemUptime
        var visited = Set<Int>()
        while ProcessInfo.processInfo.systemUptime - began < 1.3 {
            clockState.activeCount = Int.random(in: 0...4)
            clock.configure(clockState, playing: true, reduced: false)
            RunLoop.main.run(until: Date().addingTimeInterval(0.025))
            visited.insert(clock.frame)
        }
        precondition(visited == Set(0..<6), "Frequent count updates must not restart the animation")
        clockState.key = "a-new-turn"
        clock.configure(clockState, playing: true, reduced: false)
        precondition(clock.frame == 0, "A new turn starts its own animation")
        clock.configure(clockState, playing: true, reduced: true)
        precondition(clock.frame == 0)
        clock.stop()
        _ = NSApplication.shared
        let defaults = UserDefaults.standard
        let savedEnabled = defaults.object(forKey: "aieyes.pet.enabled")
        let savedPosition = defaults.object(forKey: "aieyes.pet.position")
        let savedScale = defaults.object(forKey: "aieyes.pet.scale")
        defer {
            defaults.set(savedEnabled, forKey: "aieyes.pet.enabled")
            defaults.set(savedPosition, forKey: "aieyes.pet.position")
            defaults.set(savedScale, forKey: "aieyes.pet.scale")
        }
        defaults.set(false, forKey: "aieyes.pet.enabled")
        defaults.set(1.0, forKey: "aieyes.pet.scale")
        defaults.set("{100000, 100000}", forKey: "aieyes.pet.position")
        var opens = 0
        let model = AppModel(autostart: false)
        let fixture = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: fixture) }
        let process = Process(), output = Pipe()
        process.executableURL = URL(fileURLWithPath: FileManager.default.currentDirectoryPath + "/target/debug/aieyes-core")
        process.arguments = ["--data-dir", fixture.path, "--call", "pets.current"]
        process.standardOutput = output; process.standardError = FileHandle.nullDevice
        try process.run(); let data = output.fileHandleForReading.readDataToEndOfFile(); process.waitUntilExit()
        model.importedPet = try JSONDecoder().decode(PetSelection.self, from: data).pet
        precondition(model.importedPet != nil)
        let controller = PetWindowController(model: model) { _, toggles in precondition(!toggles); opens += 1 }
        withExtendedLifetime(controller) {
            guard let panel = NSApplication.shared.windows.first(where: { $0.title == "Aieyes 宠物" }) else { fatalError("missing pet panel") }
            precondition(!panel.isVisible && !panel.canBecomeKey && !panel.isOpaque)
            precondition(panel.frame.size == NSSize(width: 144, height: 168))
            precondition(NSScreen.screens.isEmpty || NSScreen.screens.contains { $0.visibleFrame.contains(panel.frame) }, "The pet must be inside a display's work area")
            defaults.set(true, forKey: "aieyes.pet.enabled")
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
            precondition(panel.isVisible)
            precondition(panel.contentView?.accessibilityPerformPress() == true && opens == 1)
            let event = NSEvent.mouseEvent(with: .rightMouseDown, location: NSPoint(x: 30, y: 30), modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: panel.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 0)!
            let menu = panel.contentView!.menu(for: event)!
            menu.update()
            precondition(!menu.autoenablesItems && menu.items.map(\.title) == ["打开面板", "设置", "桌宠设置", "隐藏宠物（可在设置中恢复）"] && menu.items.allSatisfy(\.isEnabled))
            menu.performActionForItem(at: 0); menu.performActionForItem(at: 0)
            precondition(opens == 3, "The context menu always requests opening, never toggling")
            menu.performActionForItem(at: 1)
            precondition(model.settingsTab == "integrations")
            menu.performActionForItem(at: 2)
            precondition(model.settingsTab == "general" && model.generalPage == "desktop")
            menu.performActionForItem(at: 3)
            precondition(!defaults.bool(forKey: "aieyes.pet.enabled"))
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
            precondition(!panel.isVisible)
            for scale in [0.5, 1.0, 1.25, 1.5, 2.0] {
                defaults.set(scale, forKey: "aieyes.pet.scale")
                NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
                precondition(panel.frame.width == 144 * scale && panel.frame.height == 168 * scale)
                precondition(NSScreen.screens.isEmpty || NSScreen.screens.contains { $0.visibleFrame.contains(panel.frame) }, "Scaled pets must remain inside their display's work area")
            }
            var pet = PetState.idle; pet.label = "等待1 · 工作2\n思考1"
            model.live.pet = pet
            precondition(panel.frame.height == 186 * 2)
            precondition(PetAppearance.scale(.nan) == 1 && PetAppearance.scale(0) == 0.5 && PetAppearance.scale(5) == 2)
            defaults.set(true, forKey: "aieyes.pet.enabled")
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
            RunLoop.main.run(until: Date().addingTimeInterval(0.15))
            let directory = URL(fileURLWithPath: FileManager.default.currentDirectoryPath + "/.local/pets/native")
            try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            for scale in [0.5, 1.0, 1.25, 1.5, 2.0] {
                defaults.set(scale, forKey: "aieyes.pet.scale")
                NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
                for row in 0..<9 {
                    model.petState = PetState(behavior: "fixture", label: "动作 \(row + 1)", row: row, frames: 1, frameMs: 100, key: "fixture-\(row)")
                    RunLoop.main.run(until: Date().addingTimeInterval(0.15))
                    let view = panel.contentView!
                    view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
                    precondition(panel.frame.size == NSSize(width: 144 * scale, height: 168 * scale))
                    precondition(view.bounds.size == panel.frame.size, "The click surface must follow the controller's scaled geometry")
                    let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)!
                    view.cacheDisplay(in: view.bounds, to: bitmap)
                    for dark in [false, true] {
                        let context = CGContext(data: nil, width: bitmap.pixelsWide, height: bitmap.pixelsHigh, bitsPerComponent: 8, bytesPerRow: bitmap.pixelsWide * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
                        let bounds = CGRect(x: 0, y: 0, width: bitmap.pixelsWide, height: bitmap.pixelsHigh)
                        context.setFillColor(CGColor(gray: dark ? 0.08 : 1, alpha: 1)); context.fill(bounds)
                        context.draw(bitmap.cgImage!, in: bounds)
                        let encoded = NSBitmapImageRep(cgImage: context.makeImage()!).representation(using: .png, properties: [:])!
                        try! encoded.write(to: directory.appendingPathComponent("\(scale)-row\(row)-\(dark ? "dark" : "light").png"))
                    }
                }
            }
        }
        print("Pet native window defaults, visibility, geometry, accessibility activation passed")
        print("Pet shared variable frame timing, persistent state, logical snapping and reduced motion passed")
    }
}
