import Foundation
import AppKit
@main struct VerifyPets {
    @MainActor static func main() throws {
        let idleCadence = (0...6).map { PetState.idle.frame(elapsed: Double($0 * 100 + 1) / 1000, reduced: false) }
        precondition(idleCadence == [0,1,2,3,4,5,0])
        precondition(PetState.unknown.frameMs == PetState.idle.frameMs)
        for (row,count,ms) in [(0,6,100),(1,8,100),(2,8,100),(3,4,100),(4,5,100),(5,8,100),(6,6,100),(7,6,100),(8,6,100)] {
            let pet=PetState(behavior:"fixture",label:"Fixture",row:row,frames:count,frameMs:Double(ms),key:"fixture")
            precondition(pet.frame(elapsed:0,reduced:false)==row*8)
            precondition(pet.frame(elapsed:100,reduced:true)==row*8)
            precondition(pet.frame(elapsed:100,reduced:false)/8==row)
            for step in 0..<(count * 8) {
                precondition(pet.frame(elapsed: Double(step * 100 + 1) / 1000, reduced: false) == row * 8 + step % count)
            }
        }
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
            if let screen = NSScreen.main { precondition(screen.visibleFrame.contains(panel.frame)) }
            defaults.set(true, forKey: "aieyes.pet.enabled")
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
            precondition(panel.isVisible)
            precondition(panel.contentView?.accessibilityPerformPress() == true && opens == 1)
            let event = NSEvent.mouseEvent(with: .rightMouseDown, location: NSPoint(x: 30, y: 30), modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: panel.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 0)!
            let menu = panel.contentView!.menu(for: event)!
            menu.update()
            precondition(!menu.autoenablesItems && menu.items.count == 2 && menu.items.allSatisfy(\.isEnabled))
            menu.performActionForItem(at: 0); menu.performActionForItem(at: 0)
            precondition(opens == 3, "The context menu always requests opening, never toggling")
            menu.performActionForItem(at: 1)
            precondition(!defaults.bool(forKey: "aieyes.pet.enabled"))
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
            precondition(!panel.isVisible)
            for scale in [0.5, 1.0, 1.25, 1.5, 2.0] {
                defaults.set(scale, forKey: "aieyes.pet.scale")
                NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
                precondition(panel.frame.width == 144 * scale && panel.frame.height == 168 * scale)
                if let screen = NSScreen.main { precondition(screen.visibleFrame.contains(panel.frame)) }
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
        print("Pet uniform animation cadence, persistent state and reduced motion passed")
    }
}
