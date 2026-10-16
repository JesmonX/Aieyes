import AppKit
import SwiftUI

@MainActor final class FormGeometry {
    var row: CGFloat = 0
    var field: CGFloat = 0
}
struct FormLayoutProbe: View {
    let measured: FormGeometry
    var body: some View {
        Form {
            Section("自适应设置") {
                HStack { Text("账户与服务器"); Spacer(); Text("右侧操作") }
                    .background { GeometryReader { geometry in Color.clear.onChange(of: geometry.size.width, initial: true) { _, width in measured.row = width } } }
                LabeledContent("刷新间隔") {
                    TextField("间隔", text: .constant("300")).labelsHidden().frame(width: DesignTokens.fieldShortWidth)
                        .background { GeometryReader { geometry in Color.clear.onChange(of: geometry.size.width, initial: true) { _, width in measured.field = width } } }
                }
            }
        }.formStyle(AdaptiveGroupedFormStyle())
    }
}
@MainActor final class FormLayoutDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        Task {
            let measured = FormGeometry(), window = NSWindow(contentRect: NSRect(x: -20000, y: -20000, width: 1000, height: 400), styleMask: .borderless, backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            let view = NSHostingView(rootView: FormLayoutProbe(measured: measured))
            window.contentView = view; window.orderBack(nil)
            let root = URL(fileURLWithPath: CommandLine.arguments[1])
            try! FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            var widths: [CGFloat] = []
            for width in [1000.0, 1152.0, 500.0] {
                let size = NSSize(width: width, height: 400)
                window.setContentSize(size); view.frame = NSRect(origin: .zero, size: size)
                try? await Task.sleep(for: .milliseconds(250))
                view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
                precondition(measured.row > width - 80 && measured.row < width, "Grouped form must use available width without overflowing")
                precondition(abs(measured.field - DesignTokens.fieldShortWidth) < 1, "Short fields keep their intended width")
                widths.append(measured.row)
                let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)!
                view.cacheDisplay(in: view.bounds, to: bitmap)
                try! bitmap.representation(using: .png, properties: [:])!.write(to: root.appendingPathComponent("form-\(Int(width)).png"))
            }
            precondition(abs(widths[1] - widths[0] - 152) < 1, "Space released by the sidebar must expand the native form")
            window.close()
            print("Native forms: released sidebar width expands rows; narrow layouts fit and short controls retain their width")
            NSApp.terminate(nil)
        }
    }
}
@main struct VerifyNativeFormLayout {
    static func main() {
        let app = NSApplication.shared, delegate = FormLayoutDelegate(); app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}
