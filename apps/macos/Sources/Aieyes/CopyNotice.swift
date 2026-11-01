import SwiftUI
import AppKit

struct ErrorNotice: View {
    var message: String
    var context = L("操作未完成")
    var tint: Color = Palette.warn
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        let error = UserFacingError(message, context: context)
        VStack(alignment: .leading, spacing: 6) {
            Text(L(error.summary)).foregroundStyle(tint).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            if let details = error.details {
                DisclosureGroup(L("错误详情")) {
                    Text(L(details)).font(.system(size: DesignTokens.fontCaption, design: .monospaced))
                        .textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                }.foregroundStyle(.secondary)
            }
        }.font(AppFont.secondary)
    }
}

enum Pasteboard {
    /// Replaces the general pasteboard contents with plain text.
    @discardableResult static func copy(_ text: String) -> Bool {
        NSPasteboard.general.clearContents()
        return NSPasteboard.general.setString(text, forType: .string)
    }
}
