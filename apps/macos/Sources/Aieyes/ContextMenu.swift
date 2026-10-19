import AppKit

enum NativeContextMenu {
    static func entries(pet: Bool, visible: Bool, sampling: Bool = false) -> [(String, String)] {
        var rows = [(pet ? "打开面板" : "打开详情", pet ? "open-panel" : "open"), ("设置…", "settings")]
        if pet { rows.append(("桌宠设置", "pet-settings")) }
        else if sampling { rows.append(("管理采样…", "sampling")) }
        rows += [("", "separator"), ("检查更新", "check-updates"), (visible ? "隐藏桌宠" : "显示桌宠", "hide")]
        if !pet { rows += [("", "separator"), ("退出 Aieyes", "quit")] }
        return rows
    }
}
