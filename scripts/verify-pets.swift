import Foundation
import AppKit
@main struct VerifyPets {
    @MainActor static func main() throws {
        let idleCadence = (0...6).map { PetState.idle.frame(elapsed: Double($0 * 180 + 1) / 1000, reduced: false) }
        precondition(idleCadence == [0,1,2,3,4,5,0])
        precondition(PetState.unknown.frameMs == PetState.idle.frameMs)
        for (row,count,ms) in [(0,6,180),(1,8,180),(3,4,180),(4,5,180),(5,8,180),(6,6,180),(7,6,180),(8,6,180)] {
            let pet=PetState(behavior:"fixture",label:"Fixture",row:row,frames:count,frameMs:Double(ms),key:"fixture")
            precondition(pet.frame(elapsed:0,reduced:false)==row*8)
            precondition(pet.frame(elapsed:100,reduced:true)==row*8)
            precondition(pet.frame(elapsed:100,reduced:false)/8==row)
            for step in 0..<(count * 8) {
                precondition(pet.frame(elapsed: Double(step * 180 + 1) / 1000, reduced: false) == row * 8 + step % count)
            }
        }
        _ = NSApplication.shared
        let defaults = UserDefaults.standard
        let savedEnabled = defaults.object(forKey: "aieyes.pet.enabled")
        let savedPosition = defaults.object(forKey: "aieyes.pet.position")
        defer {
            defaults.set(savedEnabled, forKey: "aieyes.pet.enabled")
            defaults.set(savedPosition, forKey: "aieyes.pet.position")
        }
        defaults.set(false, forKey: "aieyes.pet.enabled")
        defaults.set("{100000, 100000}", forKey: "aieyes.pet.position")
        var opens = 0
        let controller = PetWindowController(model: AppModel(autostart: false)) { _ in opens += 1 }
        withExtendedLifetime(controller) {
            guard let panel = NSApplication.shared.windows.first(where: { $0.title == "Aieyes 宠物" }) else { fatalError("missing pet panel") }
            precondition(!panel.isVisible && !panel.canBecomeKey && !panel.isOpaque)
            precondition(panel.frame.size == NSSize(width: 144, height: 168))
            if let screen = NSScreen.main { precondition(screen.visibleFrame.contains(panel.frame)) }
            defaults.set(true, forKey: "aieyes.pet.enabled")
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
            precondition(panel.isVisible)
            precondition(panel.contentView?.accessibilityPerformPress() == true && opens == 1)
            defaults.set(false, forKey: "aieyes.pet.enabled")
            NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: defaults)
            precondition(!panel.isVisible)
        }
        print("Pet native window defaults, visibility, geometry, accessibility activation passed")
        print("Pet uniform animation cadence, persistent state and reduced motion passed")
    }
}
