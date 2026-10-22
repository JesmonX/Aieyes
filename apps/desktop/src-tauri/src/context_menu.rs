//! Native pet and tray adapters consume the same action definitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub label: &'static str,
    pub action: &'static str,
    pub enabled: bool,
}
pub(crate) fn entries(pet: bool, visible: bool, can_hide: bool, sampling: bool) -> Vec<Entry> {
    let mut rows = Vec::new();
    let mut add = |label, action, enabled| {
        rows.push(Entry {
            label: crate::localization::label(label),
            action,
            enabled,
        })
    };
    add(
        if pet { "打开面板" } else { "打开详情" },
        if pet { "open-panel" } else { "open" },
        true,
    );
    add("设置…", "settings", true);
    if pet {
        add("桌宠设置", "pet-settings", true);
    } else if sampling {
        add("管理采样…", "sampling", true);
    }
    add("", "separator", true);
    add("检查更新", "check-updates", true);
    add(
        if visible {
            "隐藏桌宠"
        } else {
            "显示桌宠"
        },
        "hide",
        !visible || can_hide,
    );
    if !pet {
        add("", "separator", true);
        add("退出 Aieyes", "quit", true);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menus_follow_native_mac_entries_and_visibility() {
        let pet = entries(true, true, true, false);
        assert_eq!(
            pet.iter().map(|e| e.action).collect::<Vec<_>>(),
            [
                "open-panel",
                "settings",
                "pet-settings",
                "separator",
                "check-updates",
                "hide"
            ]
        );
        assert!(!entries(true, true, false, false).last().unwrap().enabled);
        let tray = entries(false, false, false, true);
        assert_eq!(tray[2].action, "sampling");
        assert_eq!(
            tray.iter().find(|e| e.action == "hide").unwrap().label,
            "显示桌宠"
        );
        assert!(
            !entries(false, true, true, false)
                .iter()
                .any(|e| e.action == "sampling")
        );
    }
}
