import SwiftUI
import AppKit

struct DisplayLabView: View {
    let model: AppModel
    @ObservedObject private var store: DisplayStore
    @AppStorage(DisplayPreferences.dockEnabled) private var dockEnabled = false
    @AppStorage(DisplayPreferences.dockStyle) private var dockStyle = DynamicDockStyle.eyes.rawValue
    @AppStorage(DisplayPreferences.dockBadge) private var dockBadge = true
    @AppStorage(DisplayPreferences.animations) private var animations = true
    @AppStorage(DisplayPreferences.cardEnabled) private var cardEnabled = false
    @AppStorage(DisplayPreferences.cardSize) private var cardSize = DisplayCardSize.medium.rawValue
    @AppStorage(DisplayPreferences.ribbonEnabled) private var ribbonEnabled = false
    @AppStorage(DisplayPreferences.onTop) private var onTop = false
    @AppStorage(DisplayPreferences.allSpaces) private var allSpaces = true
    @AppStorage(DisplayPreferences.locked) private var locked = false
    @AppStorage("aieyes.language") private var language = "zh-CN"
    @State private var widgetSize = DisplayCardSize.medium

    init(model: AppModel) { self.model = model; _store = ObservedObject(wrappedValue: model.displays) }
    var body: some View {
        let _ = language
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                HStack(alignment: .top, spacing: 12) {
                    VStack(alignment: .leading, spacing: 7) {
                        Text(L("抬眼，就知道。")).font(.system(size: 28, weight: .semibold))
                        Text(L("为你的工作流，选一个舒服的位置。多种显示可以同时开启。")).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer(minLength: 0)
                    Text("DISPLAY LAB").font(.system(size: 9, weight: .bold, design: .monospaced)).tracking(1).foregroundStyle(Palette.accent)
                        .padding(8).background(Palette.accent.opacity(0.09), in: Capsule())
                }
                scenarioControls
                HStack(spacing: 8) {
                    Text(L("快速布置")).font(AppFont.secondary).foregroundStyle(.secondary)
                    Button(L("极简专注")) { dockEnabled = true; dockStyle = "eyes"; cardEnabled = false; ribbonEnabled = true; onTop = true }
                    Button(L("桌面仪表盘")) { dockEnabled = true; dockStyle = "rings"; cardEnabled = true; cardSize = "medium"; ribbonEnabled = false; onTop = false }
                    Button(L("桌宠陪伴")) { dockEnabled = true; dockStyle = "pet"; cardEnabled = false; ribbonEnabled = false }
                }.controlSize(.small)
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 330), spacing: 16, alignment: .top)], alignment: .leading, spacing: 16) {
                    surface(title: L("程序坞动态图标"), subtitle: L("关闭窗口后，状态仍留在 Dock。"), icon: "dock.rectangle", enabled: $dockEnabled) {
                        DockDesignPicker(store: store, live: model.live, selection: $dockStyle) { dockEnabled = true }
                        Toggle(L("等待与失败数量角标"), isOn: $dockBadge).toggleStyle(.checkbox).font(AppFont.secondary)
                        Toggle(L("启用动效"), isOn: $animations).toggleStyle(.checkbox).font(AppFont.secondary)
                        Text(L("动效遵循减少动态效果与低电量模式。")).font(AppFont.caption).foregroundStyle(.secondary)
                    }
                    surface(title: L("桌面实时卡片"), subtitle: L("用量、额度和会话状态，一张卡片看清。"), icon: "rectangle.on.rectangle", enabled: $cardEnabled) {
                        DisplayCard(snapshot: store.snapshot, size: .medium, live: true)
                            .frame(height: 170).background { DisplayCardBackground(phase: store.snapshot.phase).clipShape(RoundedRectangle(cornerRadius: 20)) }
                            .overlay(RoundedRectangle(cornerRadius: 20).strokeBorder(.primary.opacity(0.08)))
                        Picker(L("卡片尺寸"), selection: $cardSize) { ForEach(DisplayCardSize.allCases, id: \.self) { Text(L($0.title)).tag($0.rawValue) } }
                        Text(L("单击打开总览，拖动移动，右键显示选项")).font(AppFont.caption).foregroundStyle(.secondary)
                    }
                    surface(title: L("轻量状态条"), subtitle: L("适合放在屏幕边缘，留出更多工作空间。"), icon: "rectangle.compress.vertical", enabled: $ribbonEnabled) {
                        DisplayRibbon(snapshot: store.snapshot).frame(height: 54)
                            .background { DisplayCardBackground(phase: store.snapshot.phase).clipShape(RoundedRectangle(cornerRadius: 22)) }
                            .overlay(RoundedRectangle(cornerRadius: 22).strokeBorder(.primary.opacity(0.08)))
                        Text(L("状态条与卡片共享置顶、空间和锁定设置。")).font(AppFont.caption).foregroundStyle(.secondary)
                    }
                    VStack(alignment: .leading, spacing: 14) {
                        Label(L("桌面摆放"), systemImage: "move.3d").font(AppFont.section)
                        Toggle(L("悬浮置顶"), isOn: $onTop)
                        Toggle(L("在所有桌面空间显示"), isOn: $allSpaces)
                        Toggle(L("锁定位置"), isOn: $locked)
                        HStack {
                            Button(L("重置卡片位置")) { NotificationCenter.default.post(name: DisplayPreferences.resetPosition, object: nil) }
                            Spacer()
                            Button(L("收起新增显示")) { dockEnabled = false; cardEnabled = false; ribbonEnabled = false }
                        }.controlSize(.small)
                        Text(L("关闭置顶后，卡片留在桌面层。菜单栏和现有桌宠仍可照常使用。")).font(AppFont.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }.padding(18).frame(maxWidth: .infinity, alignment: .leading).modifier(NeutralCard())
                }
                widgetSection
                HStack(spacing: 8) {
                    Image(systemName: "menubar.rectangle")
                    Text(L("菜单栏与独立桌宠")).font(AppFont.secondary)
                    Spacer()
                    Button(L("打开桌宠与会话设置")) { model.generalPage = "desktop" }
                }.foregroundStyle(.secondary).padding(.top, 2)
            }.padding(18)
        }.accessibilityIdentifier("displayLab.page")
    }
    private var scenarioControls: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack {
                Label(L("测评场景"), systemImage: "slider.horizontal.3").font(AppFont.section)
                Spacer()
                Picker(L("测评场景"), selection: $store.scenario) {
                    ForEach(DisplayScenario.allCases) { Text($0.title).tag($0) }
                }.labelsHidden().frame(width: 150).accessibilityIdentifier("displayLab.scenario")
                if store.scenario != .live { Button(L("返回实时")) { store.scenario = .live }.controlSize(.small) }
            }
            Text(L(store.scenario == .live ? "显示今日全部数据，独立于主窗口筛选。" : "当前为演示数据，仅作用于新增显示；不会写入真实用量或会话。"))
                .font(AppFont.secondary).foregroundStyle(store.scenario == .live ? Color.secondary : Palette.warn).fixedSize(horizontal: false, vertical: true)
        }.padding(16).background(Palette.accent.opacity(0.055), in: RoundedRectangle(cornerRadius: 14))
    }
    private func surface<Content: View>(title: String, subtitle: String, icon: String, enabled: Binding<Bool>, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack { Label(title, systemImage: icon).font(AppFont.section); Spacer(); Toggle(title, isOn: enabled).labelsHidden().toggleStyle(.switch).controlSize(.small) }
            Text(subtitle).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            content()
        }.padding(18).frame(maxWidth: .infinity, alignment: .leading).modifier(NeutralCard())
    }
    private var widgetSection: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Label(L("macOS 系统小组件"), systemImage: "square.grid.2x2").font(AppFont.section)
                Spacer()
                Text(L("系统安排刷新")).font(AppFont.caption).foregroundStyle(.secondary)
            }
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: 24) { widgetPreview; widgetInstructions }
                VStack(alignment: .leading, spacing: 16) { widgetPreview; widgetInstructions }
            }
        }.padding(18).frame(maxWidth: .infinity, alignment: .leading).modifier(NeutralCard())
    }
    private var widgetPreview: some View {
        VStack(alignment: .leading, spacing: 10) {
            Picker(L("小组件尺寸"), selection: $widgetSize) {
                ForEach(DisplayCardSize.allCases, id: \.self) { Text(L($0.title)).tag($0) }
            }.pickerStyle(.segmented).frame(width: 240)
            DisplayCard(snapshot: store.snapshot, size: widgetSize)
                .frame(width: widgetSize.dimensions.width, height: widgetSize.dimensions.height)
                .background { DisplayCardBackground(phase: store.snapshot.phase).clipShape(RoundedRectangle(cornerRadius: 22)) }
                .overlay(RoundedRectangle(cornerRadius: 22).strokeBorder(.primary.opacity(0.08)))
        }.frame(width: 364, alignment: .leading)
    }
    private var widgetInstructions: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L("放到桌面，或放进通知中心。")).font(AppFont.body).fontWeight(.semibold)
            Text(L("桌面右键 → 编辑小组件 → 搜索 Aieyes Lab，选择尺寸后添加。")).font(AppFont.secondary).fixedSize(horizontal: false, vertical: true)
            Text(L("小组件显示最近一次快照。需要秒级状态时，可同时开启 Dock 或桌面实时卡片。")).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if store.widgetAvailable {
                Button(L("同步小组件")) { store.syncWidgets() }.accessibilityIdentifier("displayLab.syncWidgets")
                if !store.widgetMessage.isEmpty { Text(store.widgetMessage).font(AppFont.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
                if let count = store.installedWidgets { Text(L("已添加 {0} 个小组件", count)).font(AppFont.caption).foregroundStyle(.secondary) }
            } else {
                Text(L("此构建未包含系统小组件，请使用显示实验室开发版。")).font(AppFont.caption).foregroundStyle(Palette.warn).fixedSize(horizontal: false, vertical: true)
            }
        }.frame(minWidth: 200, maxWidth: .infinity, alignment: .leading)
    }
}

private struct DockDesignPicker: View {
    @ObservedObject var store: DisplayStore
    @ObservedObject var live: LiveSessionState
    @Binding var selection: String
    var select: () -> Void
    @Environment(\.surfaceActive) private var surfaceActive
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @AppStorage(DisplayPreferences.animations) private var animations = true
    @State private var frames: [CGImage] = []
    @State private var epoch = Date()
    private var state: PetState {
        let phase = store.snapshot.phase
        let row: Int = switch phase { case .idle: 0; case .working: 1; case .needsInput: 3; case .complete: 4; case .failed: 5; case .unknown: 6; case .tool: 7; case .thinking: 8 }
        let durations = CoreContract.petFrameDurations[row]
        return PetState(behavior: phase.rawValue, label: phase.title, row: row, frames: durations.count, frameMs: 100, key: phase.rawValue, frameDurationsMs: durations)
    }
    var body: some View {
        let paused = !surfaceActive || reduceMotion || !animations || ProcessInfo.processInfo.isLowPowerModeEnabled
        TimelineView(.animation(minimumInterval: 0.125, paused: paused)) { timeline in
            let elapsed = paused ? 0 : timeline.date.timeIntervalSince(epoch)
            let index = state.frame(elapsed: elapsed, reduced: paused) - state.row * 8
            HStack(spacing: 5) {
                ForEach(DynamicDockStyle.allCases) { style in
                    Button { selection = style.rawValue; select() } label: {
                        VStack(spacing: 5) {
                            DockArtworkPreview(snapshot: store.snapshot, style: style, elapsed: elapsed, petImage: frames.indices.contains(index) ? frames[index] : nil).frame(width: 76, height: 76)
                            Text(style.title).font(.system(size: 11, weight: .medium)).lineLimit(1).minimumScaleFactor(0.8)
                        }.frame(maxWidth: .infinity).padding(.vertical, 8)
                            .background(selection == style.rawValue ? Palette.accent.opacity(0.10) : Color.clear, in: RoundedRectangle(cornerRadius: 12))
                            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(selection == style.rawValue ? Palette.accent.opacity(0.45) : .clear))
                    }.buttonStyle(.plain).accessibilityLabel(style.title).accessibilityAddTraits(selection == style.rawValue ? .isSelected : [])
                        .accessibilityIdentifier("displayLab.dock." + style.rawValue)
                }
            }
        }
        .task(id: (live.asset?.contentHash ?? "") + ":" + String(state.row)) {
            guard let asset = live.asset else { frames = []; return }
            let row = state.row
            let decoded: [CGImage] = await withCheckedContinuation { continuation in PetSprites.queue.async { continuation.resume(returning: PetSprites.frames(for: asset, row: row)) } }
            guard !Task.isCancelled else { return }; frames = decoded; epoch = Date()
        }
    }
}
