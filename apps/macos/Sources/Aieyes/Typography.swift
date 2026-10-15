import SwiftUI

enum AppFont {
    static let body = Font.system(size: DesignTokens.fontBody)
    static let caption = Font.system(size: DesignTokens.fontCaption)
    static let secondary = Font.system(size: DesignTokens.fontSecondary)
    static let section = Font.system(size: DesignTokens.fontSection, weight: DesignTokens.weightStrong)
    static let title = Font.system(size: DesignTokens.fontTitle, weight: DesignTokens.weightStrong)
}
