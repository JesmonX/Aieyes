import AppKit
import SwiftUI

/// The system search field: magnifying glass, clear button and Esc-to-clear come from AppKit.
struct SearchField: NSViewRepresentable {
    let prompt: String
    @Binding var text: String

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }
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
        if field.stringValue != text { field.stringValue = text }
        if field.placeholderString != prompt { field.placeholderString = prompt; field.setAccessibilityLabel(prompt) }
    }

    @MainActor final class Coordinator: NSObject, NSSearchFieldDelegate {
        var text: Binding<String>
        init(text: Binding<String>) { self.text = text }
        // Typing reaches both paths; the clear button only sends the action.
        @objc func changed(_ field: NSSearchField) { if text.wrappedValue != field.stringValue { text.wrappedValue = field.stringValue } }
        func controlTextDidChange(_ notification: Notification) { if let field = notification.object as? NSSearchField { changed(field) } }
    }
}
