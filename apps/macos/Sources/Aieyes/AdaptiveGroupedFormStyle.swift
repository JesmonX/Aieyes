import SwiftUI

/// Native grouped forms cap their ideal row width. Give rows the available
/// workspace width while keeping each control's own size and native behavior.
struct AdaptiveGroupedFormStyle: FormStyle {
    func makeBody(configuration: Configuration) -> some View {
        GeometryReader { geometry in
            Form {
                configuration.content
                    .frame(minWidth: max(0, geometry.size.width - 2 * DesignTokens.contentPadding), alignment: .leading)
                    .listRowSeparator(.hidden)
            }.formStyle(.grouped)
        }
    }
}
