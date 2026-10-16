import SwiftUI

struct ErrorNotice: View {
    var message: String
    var context = "操作未完成"
    var tint: Color = Palette.warn
    var body: some View {
        let error = UserFacingError(message, context: context)
        VStack(alignment: .leading, spacing: 6) {
            Text(error.summary).foregroundStyle(tint).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            if let details = error.details {
                DisclosureGroup("错误详情") {
                    Text(details).font(.system(size: DesignTokens.fontCaption, design: .monospaced))
                        .textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                }.foregroundStyle(.secondary)
            }
        }.font(AppFont.secondary)
    }
}
