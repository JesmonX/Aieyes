import SwiftUI

/// Explanations belong beside the decision they describe, outside the normal reading flow.
struct HelpNote: View {
    let title: String
    let text: String
    @State private var showing = false
    var body: some View {
        Button { showing.toggle() } label: { Image(systemName: "questionmark.circle") }
            .buttonStyle(.plain).foregroundStyle(.secondary).help(title).accessibilityLabel(title)
            .popover(isPresented: $showing) {
                VStack(alignment: .leading, spacing: 12) {
                    Text(title).font(AppFont.section)
                    Text(text).font(AppFont.secondary).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                    HStack { Spacer(); Button(L("关闭")) { showing = false }.keyboardShortcut(.cancelAction) }
                }.padding(16).frame(width: 320).aieyesAccent()
            }
    }
}
