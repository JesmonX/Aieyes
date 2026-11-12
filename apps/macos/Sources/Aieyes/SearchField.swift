import AppKit
import SwiftUI

/// The system search field: magnifying glass, clear button and Esc-to-clear come from AppKit.
struct SearchField: NSViewRepresentable {
    let prompt: String
    @Binding var text: String
    var onSubmit: (() -> Void)? = nil

    func makeCoordinator() -> Coordinator { Coordinator(text: $text, onSubmit: onSubmit) }
    func makeNSView(context: Context) -> NSSearchField {
        let field = NSSearchField()
        field.sendsSearchStringImmediately = true
        field.target = context.coordinator
        field.action = #selector(Coordinator.changed(_:))
        field.delegate = context.coordinator
        field.font = .systemFont(ofSize: DesignTokens.fontSecondary)
        field.setContentHuggingPriority(.defaultLow, for: .horizontal)
        return field
    }
    func updateNSView(_ field: NSSearchField, context: Context) {
        context.coordinator.text = $text
        context.coordinator.onSubmit = onSubmit
        if field.stringValue != text { field.stringValue = text }
        if field.placeholderString != prompt { field.placeholderString = prompt; field.setAccessibilityLabel(prompt) }
    }

    @MainActor final class Coordinator: NSObject, NSSearchFieldDelegate {
        var text: Binding<String>
        var onSubmit: (() -> Void)?
        init(text: Binding<String>, onSubmit: (() -> Void)?) { self.text = text; self.onSubmit = onSubmit }
        func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
            guard commandSelector == #selector(NSResponder.insertNewline(_:)), let onSubmit else { return false }
            onSubmit(); return true
        }
        // Typing reaches both paths; the clear button only sends the action.
        @objc func changed(_ field: NSSearchField) { if text.wrappedValue != field.stringValue { text.wrappedValue = field.stringValue } }
        func controlTextDidChange(_ notification: Notification) { if let field = notification.object as? NSSearchField { changed(field) } }
    }
}
