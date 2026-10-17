import AppKit
import SwiftUI

@MainActor final class ControlValues: ObservableObject {
    @Published var text = ""
    @Published var disabled = false
    var focused = false
    var presses = 0
    var cancels = 0
}
struct ControlProbe: View {
    @ObservedObject var values: ControlValues
    @FocusState private var focused: Bool
    var body: some View {
        VStack(spacing: 18) {
            TextField("搜索", text: $values.text).focused($focused)
                .onChange(of: focused) { _, value in values.focused = value }
            HStack {
                Button("保存") { values.presses += 1 }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction).disabled(values.disabled)
                Button("取消") { values.cancels += 1 }.keyboardShortcut(.cancelAction)
            }
        }.padding(30).frame(width: 380, height: 150).aieyesAccent()
    }
}
@MainActor final class ControlDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        Task { await verify(); NSApp.terminate(nil) }
    }
    func verify() async {
        let root = URL(fileURLWithPath: CommandLine.arguments[1])
        try! FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        for dark in [false, true] {
            let values = ControlValues()
            let view = NSHostingView(rootView: ControlProbe(values: values).environment(\.colorScheme, dark ? .dark : .light))
            let window = NSWindow(contentRect: NSRect(x: -20000, y: -20000, width: 380, height: 150), styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false; window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            window.contentView = view; window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
            // App activation is asynchronous, particularly after the popover test
            // exits. An inactive editor correctly suppresses its focus outline.
            for _ in 0..<30 {
                if NSApp.isActive && window.isKeyWindow { break }
                window.makeKey()
                try! await Task.sleep(for: .milliseconds(100))
            }
            precondition(NSApp.isActive && window.isKeyWindow, "The focus probe needs an active key window")
            @MainActor func descendants(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap(descendants) }
            // Becoming key does not guarantee SwiftUI has installed the native editor yet.
            var nativeField: NSTextField?
            for _ in 0..<30 {
                view.layoutSubtreeIfNeeded()
                nativeField = descendants(view).compactMap { $0 as? NSTextField }.first
                if nativeField != nil { break }
                try! await Task.sleep(for: .milliseconds(100))
            }
            guard let field = nativeField else { preconditionFailure("SwiftUI did not install the native text field") }
            precondition(window.makeFirstResponder(field), "Styled TextField accepts keyboard focus")
            let editor = field.currentEditor() as! NSTextView
            editor.insertText("hello", replacementRange: NSRange(location: 0, length: 0))
            try! await Task.sleep(for: .milliseconds(150))
            precondition(values.text == "hello" && values.focused, "Native editing and the caller's FocusState binding survive styling")
            view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
            let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)!
            view.cacheDisplay(in: view.bounds, to: bitmap)
            try! bitmap.representation(using: .png, properties: [:])!.write(to: root.appendingPathComponent(dark ? "focused-dark.png" : "focused-light.png"))
            var focusPixels = 0
            window.effectiveAppearance.performAsCurrentDrawingAppearance {
                let accent = NSColor(Palette.accent).usingColorSpace(.sRGB)!
                for y in 0..<bitmap.pixelsHigh { for x in 0..<bitmap.pixelsWide {
                    let color = bitmap.colorAt(x: x, y: y)!
                    if color.alphaComponent > 0.95 && abs(color.redComponent - accent.redComponent) < 0.05 && abs(color.greenComponent - accent.greenComponent) < 0.05 && abs(color.blueComponent - accent.blueComponent) < 0.05 { focusPixels += 1 }
                } }
            }
            precondition(focusPixels > 300, "Focused fields need a visible accent outline, not just a caret: dark=\(dark), pixels=\(focusPixels)")
            window.makeFirstResponder(view)
            func key(_ text: String, _ code: UInt16) -> NSEvent {
                NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: text, charactersIgnoringModifiers: text, isARepeat: false, keyCode: code)!
            }
            _ = view.performKeyEquivalent(with: key("\r", 36))
            precondition(values.presses == 1, "Return invokes the default Button exactly once")
            _ = view.performKeyEquivalent(with: key("\u{1b}", 53))
            precondition(values.cancels == 1, "Escape retains its cancel action")
            values.disabled = true
            try! await Task.sleep(for: .milliseconds(100))
            _ = view.performKeyEquivalent(with: key("\r", 36))
            precondition(values.presses == 1, "Disabled Button ignores its shortcut")
            window.close()
        }
        print("Native styled controls: light/dark focus outlines, caller FocusState, native editing, Return, Escape and disabled shortcuts passed")
    }
}
@main struct VerifyNativeControls {
    static func main() {
        let app = NSApplication.shared, delegate = ControlDelegate(); app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}
