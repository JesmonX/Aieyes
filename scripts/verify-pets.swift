import Foundation
import AppKit
import Metal
import Darwin
@testable import Aieyes
@main struct VerifyPets {
    static func checkpoint(_ message: String) {
        FileHandle.standardOutput.write(Data((message + "\n").utf8))
    }
    static func unsupportedRenderer(virtualIntelCI: Bool, requireGraphics: Bool, signal: Int32?, diagnostic: String) -> Bool {
        virtualIntelCI && !requireGraphics && signal == SIGABRT
            && diagnostic.contains("+[MTLLoader sliceIDForDevice:")
            && diagnostic.contains("Target device architecture is nil")
    }
    @MainActor static func pump(_ duration: TimeInterval = 0.2) {
        let until = Date().addingTimeInterval(duration)
        while Date() < until {
            if let event = NSApp.nextEvent(matching: .any, until: Date().addingTimeInterval(0.01), inMode: .default, dequeue: true) { NSApp.sendEvent(event) }
            RunLoop.main.run(until: Date().addingTimeInterval(0.005))
        }
    }
    @MainActor static func waitUntil(_ message: String, condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(5)
        while !condition() && Date() < deadline { pump(0.05) }
        precondition(condition(), message)
    }
    @MainActor static func show(_ window: NSWindow) {
        window.orderFrontRegardless()
        waitUntil("Animation test window did not become visible", condition: {
            window.isVisible && window.occlusionState.contains(.visible)
        })
    }
    @MainActor static func main() throws {
        precondition(NativeContextMenu.entries(pet: true, visible: true).map { $0.1 } == ["open-panel", "settings", "pet-settings", "separator", "check-updates", "hide"])
        precondition(NativeContextMenu.entries(pet: false, visible: false, sampling: true).map { $0.0 } == ["打开详情", "设置…", "管理采样…", "", "检查更新", "显示桌宠", "", "退出 Aieyes"])

        if CommandLine.arguments.contains("--window-tests") {
            _ = NSApplication.shared
            NSApp.finishLaunching()
            try verifyWindow()
            return
        }
        let metalDevice = MTLCreateSystemDefaultDevice()
        let canRenderSymbolEffects = metalDevice != nil && !CommandLine.arguments.contains("--without-metal-device")
        if CommandLine.arguments.contains("--require-metal-device") {
            precondition(canRenderSymbolEffects, "The graphics acceptance run requires a Metal device")
        }
        checkpoint("Pet graphics environment: Metal device = \(metalDevice?.name ?? "none")")
        let driverAbort = "+[MTLLoader sliceIDForDevice:legacyDriverVersion:airntDriverVersion:]: Target device architecture is nil"
        precondition(unsupportedRenderer(virtualIntelCI: true, requireGraphics: false, signal: SIGABRT, diagnostic: driverAbort))
        precondition(!unsupportedRenderer(virtualIntelCI: false, requireGraphics: false, signal: SIGABRT, diagnostic: driverAbort))
        precondition(!unsupportedRenderer(virtualIntelCI: true, requireGraphics: true, signal: SIGABRT, diagnostic: driverAbort))
        precondition(!unsupportedRenderer(virtualIntelCI: true, requireGraphics: false, signal: SIGILL, diagnostic: driverAbort))
        precondition(!unsupportedRenderer(virtualIntelCI: true, requireGraphics: false, signal: SIGABRT, diagnostic: "Precondition failed"))
        precondition(!unsupportedRenderer(virtualIntelCI: true, requireGraphics: false, signal: nil, diagnostic: driverAbort))
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
        _ = NSApplication.shared
        NSApp.finishLaunching()
        let context = CGContext(data: nil, width: 8, height: 8, bitsPerComponent: 8, bytesPerRow: 32, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        let images = (0..<8).map { frame -> CGImage in context.setFillColor(CGColor(gray: Double(frame)/8, alpha: 1)); context.fill(CGRect(x:0,y:0,width:8,height:8)); return context.makeImage()! }
        for durations in CoreContract.petFrameDurations {
            let animation = PetLayerImage.keyframes(images: images, durations: durations)!
            precondition(animation.calculationMode == .discrete && animation.values?.count == durations.count)
            precondition(animation.duration == Double(durations.reduce(0,+))/1000 && animation.repeatCount == .infinity)
            var point = [Float](repeating:0,count:2)
            for index in [1,2] { animation.timingFunction!.getControlPoint(at:index,values:&point);precondition(abs(point[0]-point[1])<1e-6,"Global timing must remain linear so frame durations are exact") }
            var cumulative = 0
            for (index, duration) in durations.enumerated() { cumulative += duration; precondition(abs(animation.keyTimes![index+1].doubleValue - Double(cumulative)/Double(durations.reduce(0,+))) < 1e-12) }
        }
        checkpoint("Pet frame timing and Core Animation keyframe configuration passed")
        let maskContext = CGContext(data:nil,width:8,height:8,bitsPerComponent:8,bytesPerRow:32,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue)!
        maskContext.setFillColor(CGColor(gray:1,alpha:1)); maskContext.fill(CGRect(x:0,y:6,width:2,height:2))
        let hitSprite = PetLayerImage(frame:NSRect(x:0,y:0,width:80,height:80))
        hitSprite.configure(images:[maskContext.makeImage()!],state:.idle,identity:"alpha-corner",playing:false,reduced:true)
        precondition(hitSprite.opaque(at:NSPoint(x:10,y:70)), "The visible upper-left pixels must accept input")
        precondition(!hitSprite.opaque(at:NSPoint(x:10,y:10)) && !hitSprite.opaque(at:NSPoint(x:70,y:70)), "Transparent pixels must pass input through, without vertically mirroring the mask")
        precondition(!hitSprite.opaque(at:NSPoint(x:-1,y:70)))
        checkpoint("Pet alpha mask orientation and transparent pixel hit testing passed")
        let animationWindow = NSWindow(contentRect: NSRect(x:200,y:200,width:144,height:168), styleMask: [.titled], backing: .buffered, defer: false)
        animationWindow.isReleasedWhenClosed = false; animationWindow.level = .statusBar
        let sprite = PetLayerImage(); animationWindow.contentView = sprite
        show(animationWindow)
        var clockState = PetState.idle
        func configure() { sprite.configure(images: images, state: clockState, identity: clockState.animationIdentity, playing: true, reduced: false) }
        configure(); let starts = sprite.animationStarts
        precondition(starts > 0)
        for count in 0..<30 { clockState.activeCount = count; configure() }
        precondition(sprite.animationStarts == starts, "Unrelated state updates must not restart keyframes")
        clockState.key = "a-new-turn"; configure(); precondition(sprite.animationStarts == starts)
        for timings in [CoreContract.petFrameDurations[0], Array(repeating: 100, count: 6)] {
            clockState.frameDurationsMs = timings
            configure()
            // Pause the layer clock at each frame midpoint: Core Animation still evaluates the presented keyframes,
            // including the last frame, without depending on how promptly a loaded machine runs the sampler.
            guard let layer = sprite.layer, let frames = layer.animation(forKey: "pet.frames") else { preconditionFailure("Playing sprite has no frame animation") }
            let (speed, offset) = (layer.speed, layer.timeOffset)
            var boundary = 0.0
            for (index, duration) in timings.enumerated() {
                // A presentation copy reflects committed layer timing, so commit the paused clock before sampling it.
                layer.speed = 0; layer.timeOffset = frames.beginTime + (boundary + Double(duration) / 2) / 1000; CATransaction.flush()
                guard let contents = layer.presentation()?.contents else { preconditionFailure("Visible sprite has no presentation image") }
                let displayed = (contents as! CGImage).dataProvider?.data
                precondition(displayed == images[index].dataProvider?.data, "Presentation image must match the variable frame timing, including the last frame: frame \(index) of \(timings) showed \(images.firstIndex { $0.dataProvider?.data == displayed }.map(String.init) ?? "an unknown image")")
                boundary += Double(duration)
            }
            layer.speed = speed; layer.timeOffset = offset; CATransaction.flush()
        }
        checkpoint("Pet presentation images match normal and uniform frame timing")
        sprite.configure(images: images, state: clockState, identity: clockState.animationIdentity, playing: true, reduced: true)
        precondition(sprite.layer?.animation(forKey:"pet.frames") == nil, "Reduced motion retains the first frame")
        configure(); animationWindow.orderOut(nil)
        waitUntil("Hidden windows stop animation", condition: { sprite.layer?.animation(forKey: "pet.frames") == nil })
        animationWindow.close()
        checkpoint("Pet keyframe reuse, reduced motion and hidden-window pause passed")
        if CommandLine.arguments.contains("--without-metal-device") {
            checkpoint("SKIP Metal/SwiftUI graphics integration: forced no-GPU fixture; timing, geometry and layer lifecycle checks passed")
            return
        }
        let log = FileManager.default.temporaryDirectory.appendingPathComponent("aieyes-pet-window-" + UUID().uuidString + ".log")
        _ = FileManager.default.createFile(atPath: log.path, contents: nil)
        defer { try? FileManager.default.removeItem(at: log) }
        let output = try FileHandle(forWritingTo: log)
        let child = Process()
        child.executableURL = URL(fileURLWithPath: CommandLine.arguments[0])
        child.arguments = ["--window-tests"]
        child.standardOutput = output; child.standardError = output
        try child.run(); child.waitUntilExit(); try output.close()
        let diagnostic = try String(contentsOf: log, encoding: .utf8)
        FileHandle.standardOutput.write(Data(diagnostic.utf8))
        guard child.terminationStatus != 0 else { return }
        #if arch(x86_64)
        let virtualIntelCI = ProcessInfo.processInfo.environment["CI"] == "true"
            && (metalDevice == nil || metalDevice!.name.localizedCaseInsensitiveContains("paravirtual"))
        #else
        let virtualIntelCI = false
        #endif
        guard unsupportedRenderer(virtualIntelCI: virtualIntelCI,
            requireGraphics: CommandLine.arguments.contains("--require-metal-device"),
            signal: child.terminationReason == .uncaughtSignal ? child.terminationStatus : nil,
            diagnostic: diagnostic) else {
            throw NSError(domain: "Aieyes.PetWindowTests", code: Int(child.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: "Pet window integration failed; see child process diagnostics above"])
        }
        checkpoint("SKIP Metal/SwiftUI graphics integration: Intel CI virtual GPU cannot load the required Metal architecture; timing, geometry and Core Animation layer lifecycle checks passed")
    }
    @MainActor static func verifyWindow() throws {
        let canRenderSymbolEffects = MTLCreateSystemDefaultDevice() != nil
        // Keep both symbol effects and SwiftUI rendering in the graphics child:
        // driver shader compilation may complete after a view is closed.
        if canRenderSymbolEffects {
            let menuWindow = NSWindow(contentRect: NSRect(x:200,y:200,width:22,height:22), styleMask: [.borderless], backing: .buffered, defer: false)
            menuWindow.isReleasedWhenClosed = false; menuWindow.level = .statusBar
            let menuImage = MenuActivityImage(); menuWindow.contentView = menuImage
            show(menuWindow)
            menuImage.configure(phase:.working,playing:true);let pulses = menuImage.animationStarts
            precondition(pulses > 0)
            for _ in 0..<30 { menuImage.configure(phase:.working,playing:true) }
            precondition(menuImage.animationStarts == pulses, "Stable menu snapshots must not restart the system pulse")
            menuImage.configure(phase:.working,playing:false); menuWindow.close()
            checkpoint("Menu system pulse start, reuse and stop passed")
        } else {
            checkpoint("SKIP live menu symbol effect: no usable Metal device; SwiftUI window capability probe continues")
        }
        let defaults = UserDefaults.standard
        let savedEnabled = defaults.object(forKey: "aieyes.pet.enabled")
        let savedPosition = defaults.object(forKey: "aieyes.pet.position")
        let savedScale = defaults.object(forKey: "aieyes.pet.scale")
        let savedFullscreen = defaults.object(forKey: "aieyes.pet.hideInFullscreen")
        defer {
            defaults.set(savedEnabled, forKey: "aieyes.pet.enabled")
            defaults.set(savedPosition, forKey: "aieyes.pet.position")
            defaults.set(savedScale, forKey: "aieyes.pet.scale")
            defaults.set(savedFullscreen, forKey: "aieyes.pet.hideInFullscreen")
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
        checkpoint("Starting production SwiftUI pet window integration")
        var updateChecks = 0
        let controller = PetWindowController(model: model, checkUpdates: { updateChecks += 1 }) { _, toggles in precondition(!toggles); opens += 1 }
        withExtendedLifetime(controller) {
            guard let panel = NSApplication.shared.windows.first(where: { $0.title == "Aieyes 宠物" }) else { fatalError("missing pet panel") }
            precondition(!panel.isVisible && !panel.canBecomeKey && !panel.isOpaque)
            precondition(panel.frame.width == 144 && panel.frame.height >= 143 + 14, "The status label needs a readable line below the sprite")
            precondition(NSScreen.screens.isEmpty || NSScreen.screens.contains { $0.visibleFrame.contains(panel.frame) }, "The pet must be inside a display's work area")
            defaults.set(true, forKey: "aieyes.pet.enabled")
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
            precondition(panel.isVisible)
            panel.level = .normal
            NSWorkspace.shared.notificationCenter.post(name: NSWorkspace.didActivateApplicationNotification, object: nil)
            precondition(panel.level == .floating && panel.isVisible, "Application switches restore pet ordering synchronously")
            let popover = PanelPopover()
            popover.behavior = .applicationDefined
            popover.animates = false
            popover.contentViewController = PanelContainer(model: model)
            (popover.contentViewController as? PanelContainer)?.prepare(size: NSSize(width: 450, height: 540))
            popover.show(relativeTo: panel.contentView!.bounds, of: panel.contentView!, preferredEdge: .minY)
            pump(0.1)
            let overlay = popover.contentViewController!.view.window!
            precondition(overlay.level == .statusBar && !overlay.hidesOnDeactivate, "Pet-anchored panel remains above applications")
            precondition(overlay.collectionBehavior.contains(.canJoinAllSpaces))
            popover.performClose(nil)
            precondition(panel.contentView?.accessibilityPerformPress() == true && opens == 1)
            let event = NSEvent.mouseEvent(with: .rightMouseDown, location: NSPoint(x: 30, y: 30), modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: panel.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 0)!
            let menu = panel.contentView!.menu(for: event)!
            menu.update()
            precondition(!menu.autoenablesItems && menu.items.map(\.title) == ["打开面板", "设置…", "桌宠设置", "", "检查更新", "隐藏桌宠"] && menu.items.filter { !$0.isSeparatorItem }.allSatisfy(\.isEnabled))
            menu.performActionForItem(at: 0); menu.performActionForItem(at: 0)
            precondition(opens == 3, "The context menu always requests opening, never toggling")
            menu.performActionForItem(at: 1)
            precondition(model.settingsTab == "general", "The generic Settings action opens preferences")
            menu.performActionForItem(at: 2)
            precondition(model.settingsTab == "general" && model.generalPage == "desktop")
            menu.performActionForItem(at: 4)
            precondition(updateChecks == 1, "The native context menu invokes the update callback")
            menu.performActionForItem(at: 5)
            precondition(!defaults.bool(forKey: "aieyes.pet.enabled"))
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
            precondition(!panel.isVisible)
            for scale in [0.5, 1.0, 1.25, 1.5, 2.0] {
                defaults.set(scale, forKey: "aieyes.pet.scale")
                NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
                precondition(panel.frame.width == 144 * scale && panel.frame.height - 143 * scale >= 13, "The label keeps readable height even at 50% scale")
                precondition(NSScreen.screens.isEmpty || NSScreen.screens.contains { $0.visibleFrame.contains(panel.frame) }, "Scaled pets must remain inside their display's work area")
            }
            var pet = PetState.idle; pet.label = "等待1 · 工作2\n思考1"
            model.live.pet = pet
            precondition(panel.frame.height > 143 * 2 + 40, "Two status lines must expand the window")
            precondition(PetAppearance.scale(.nan) == 1 && PetAppearance.scale(0) == 0.5 && PetAppearance.scale(5) == 2)
            defaults.set(true, forKey: "aieyes.pet.enabled")
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
            RunLoop.main.run(until: Date().addingTimeInterval(0.15))
            if let screen = panel.screen ?? NSScreen.screens.first {
                defaults.set(true, forKey: "aieyes.pet.hideInFullscreen")
                NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
                let cover = NSWindow(contentRect:screen.frame,styleMask:[.borderless],backing:.buffered,defer:false)
                cover.isReleasedWhenClosed = false
                cover.level = .normal
                cover.orderFrontRegardless()
                waitUntil("A fullscreen surface on the pet's display must hide it", condition:{ !panel.isVisible })
                cover.orderOut(nil)
                waitUntil("Leaving fullscreen restores the pet", condition:{ panel.isVisible })
                cover.orderFrontRegardless()
                waitUntil("Reentering fullscreen hides the pet", condition:{ !panel.isVisible })
                defaults.set(false, forKey:"aieyes.pet.enabled")
                NotificationCenter.default.post(name:UserDefaults.didChangeNotification,object:defaults)
                cover.close()
                pump(1.2)
                precondition(!panel.isVisible, "Leaving fullscreen must not undo a manual disable")
                defaults.set(true, forKey:"aieyes.pet.enabled")
                NotificationCenter.default.post(name:UserDefaults.didChangeNotification,object:defaults)
                waitUntil("Reenabling the pet restores it",condition:{panel.isVisible})
                checkpoint("Pet fullscreen hide/restore and manual disable precedence passed")
            }
            checkpoint("Pet window visibility, actions and scaled geometry passed; capturing sprite previews")
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
                    precondition(panel.frame.width == 144 * scale && panel.frame.height - 143 * scale >= 13)
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
