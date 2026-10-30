# Desktop UI repair, 2026-10-30

The repair covers the shared Windows/Linux UI and the native macOS client. No stored account, credential, quota, or collection configuration is migrated.

- Daily details use plain disclosure actions for dates. Numeric columns and headers share alignment, restrained weight, and a consistent header surface; expanded model rows retain the same columns. The annual table uses the same presentation, with readable native macOS rows and header alignment.
- Heatmap months and day cells use the same column pitch. Short ranges stay compact. Starting partial months no longer overlap the next month's label.
- Machine menus contain one “登录” and one “切换账户” action for Codex, and “登录” for Antigravity. Current/credentialed directories take priority as the initial location; the Codex flow retains all directory choices. Windows/Linux menu items use the same plain item treatment as other app menus.
- Panel account selection puts provider identity, selection count and reset action on separate rows. Long account names wrap, and the outer dialog scrolls instead of clipping or compressing account groups. The native picker uses checkboxes aligned to their labels.
- GPU utilization remains an ordinary category-colored measurement at 100%; memory/storage capacity warnings remain intact.
- Windows pets and panels preserve the topmost band during `WM_WINDOWPOSCHANGING`, before a demoted frame can be painted. Panels explicitly enter the topmost band when shown, without activating the foreground window. Hidden overlays remain hidden.
- macOS pets refresh ordering on application activation. Pet and menu-bar popovers set their overlay level after AppKit assigns the anchor level, and remain visible on deactivation when pinned. Outside dismissal and fullscreen pet suppression retain their existing behavior.

## Verification

`test-desktop-layout.cjs` exercises production CSS and markup in Chromium at 760/1000/1440px, light/dark, Windows/Linux, plus 450px account selection with long names. It checks shared table columns, square heat cells, exact month/cell alignment, short and boundary ranges, concise menus and keyboard control, account-choice reachability, and GPU versus capacity warning semantics. Screenshots are written under `.local/ui-desktop-layout/` and uploaded by desktop CI.

The native pet verifier opens a real pet-anchored popover, checks its window level and deactivation behavior, and checks immediate application-activation recovery. The Windows native verifier checks synchronous demotion prevention and focus preservation on the actual pet and panel. A Windows Rust window test additionally exercises panel topmost retention, focus preservation, hidden-window behavior and reopening without requiring an interactive input desktop.

The macOS `AIEYES_UI_LAYOUT_ONLY=1` snapshot mode includes heatmaps, short ranges, daily details, annual tables and the account picker at multiple widths and both themes, alongside the existing quota/GPU layout captures.

Browser rendering, native macOS windows and Windows native window tests are separate evidence. The Windows interactive verifier explicitly reports `not-run` if the runner has no usable input desktop; its result must not be described as a manual desktop acceptance test.
