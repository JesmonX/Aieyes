import AppKit

enum NativeContextMenu {
    static func entries(pet: Bool, visible: Bool, sampling: Bool = false) -> [(String, String)] {
        var rows = [(pet ? L("打开面板") : L("打开详情"), pet ? "open-panel" : "open"), (L("设置…"), "settings")]
        if pet { rows.append((L("桌宠设置"), "pet-settings")) }
        else if sampling { rows.append((L("管理采样…"), "sampling")) }
        rows += [("", "separator"), (L("检查更新"), "check-updates"), (visible ? L("隐藏桌宠") : L("显示桌宠"), "hide")]
        if !pet { rows += [("", "separator"), (L("退出 Aieyes"), "quit")] }
        return rows
    }
}
