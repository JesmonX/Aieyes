import SwiftUI

struct WakeupsView: View {
    @ObservedObject var model: AppModel
    @StateObject private var store = WakeupsStore()
    @State private var editing: WakeTask?
    @State private var removal: WakeRecord?
    @State private var runConfirm: WakeRecord?
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        Form {
            Section {
                DisclosureGroup(L("执行规则")) {
                    Text(L("保存草稿后需部署，修改后需更新部署。退出 Aieyes 后任务继续运行；本机 macOS / Windows 需保持用户登录。休眠、离线或错过时刻不补发。模型请求可能消耗额度。")).font(AppFont.secondary).foregroundStyle(.secondary)
                }
                HStack {
                    Button(L("新建任务")) { editing = WakeTask() }.buttonStyle(.aieyesTinted)
                    Button(L("刷新列表")) { Task { await store.load() } }.disabled(store.loading)
                    if store.loading { ProgressView().controlSize(.small) }
                }
                if let error = store.listError { ErrorNotice(message: error) }
                if store.records.isEmpty && !store.loading && store.listError == nil { Text(L("尚未配置定时任务")).foregroundStyle(.secondary) }
            }
            ForEach(store.records) { record in Section { taskCard(record) } }
        }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle()).font(AppFont.body).disabled(model.installingUpdate).task { await store.load() }
        .updateOperation(L("定时唤醒任务"), active: !store.operating.isEmpty)
        .sheet(item: $removal) { record in
            DangerConfirmation(title: record.deployment == nil ? L("删除任务草稿？") : L("移除自动任务？"), explanation: record.deployment == nil ? L("删除「") + record.task.name + L("」的任务草稿。") : L("将从目标机器移除「") + record.task.name + L("」的定时部署，后续自动唤醒会停止。运行历史保留。"), affected: [], confirmLabel: L("确认移除"), cancel: { removal = nil }, confirm: { removal = nil; store.perform(record.deployment == nil ? "delete" : "remove", record.id) })
        }
        .confirmationDialog(L("立即运行任务？"), isPresented: Binding(get: { runConfirm != nil }, set: { if !$0 { runConfirm = nil } }), titleVisibility: .visible, presenting: runConfirm) { record in
            Button(L("确认运行")) { runConfirm = nil; store.perform("run", record.id) }
            Button(L("取消"), role: .cancel) { runConfirm = nil }
        } message: { record in Text(L("将立即调用「{0}」的模型，可能消耗订阅额度。", record.task.name)) }
        .sheet(item: $editing) { task in
            WakeEditor(model: model, task: task, deployed: store.records.first { $0.id == task.id }?.deployment != nil) { value in
                try await store.save(value); editing = nil
            }
        }
    }
    private func taskCard(_ r: WakeRecord) -> some View {
        let needsDeploy = r.deployment == nil || r.deployment?.state == "deployment-unconfirmed" || store.statuses[r.id]?.installed == false
        return VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(r.task.name).font(AppFont.section); Spacer()
                if store.operating.contains(r.id) || store.refreshing.contains(r.id) { ProgressView().controlSize(.small) }
                Text(r.deployment.map { $0.state == "settings-pending" ? L("部署待同步") : $0.state == "pending-removal" ? L("待移除") : $0.state == "deployment-unconfirmed" ? L("部署待确认") : $0.enabled ? L("已部署") : L("已停用") } ?? L("未部署")).foregroundStyle(.secondary)
            }
            Text("\(r.task.times.joined(separator: "、")) · \(r.task.model) · \(r.task.effort.isEmpty ? L("默认推理强度") : r.task.effort)").textSelection(.enabled)
            if let d = r.deployment { Text(d.target + " · " + (store.statuses[r.id]?.timezone ?? d.timezone) + (d.changed ? L(" · 草稿待部署") : "")).font(AppFont.secondary).foregroundStyle(.secondary) }
            HStack {
                Button(L("编辑")) { editing = r.task }
                Button(r.deployment == nil ? L("部署") : needsDeploy ? L("重新部署") : r.deployment?.changed == true ? L("更新部署") : L("查看运行结果")) { store.perform(needsDeploy || r.deployment?.changed == true ? "deploy" : "status", r.id) }.buttonStyle(.aieyesTinted)
                Menu(L("更多操作")) {
                    if let d = r.deployment, !needsDeploy {
                        Button(d.enabled ? L("停用") : L("启用")) { store.perform(d.enabled ? "disable" : "enable", r.id) }
                        Button(L("立即运行")) { runConfirm = r }.help(L("立即调用模型，可能消耗订阅额度"))
                        Button(L("状态与记录")) { store.perform("status", r.id) }
                    }
                    Button(r.deployment == nil ? L("删除草稿") : L("移除自动任务"), role: .destructive) { removal = r }
                }
            }.disabled(store.operating.contains(r.id))
            if let notice = store.notices[r.id] { Text(L(notice)).font(AppFont.secondary).foregroundStyle(Palette.ok) }
            ForEach((store.operationErrors[r.id] ?? [:]).keys.sorted(), id: \.self) { action in
                HStack { ErrorNotice(message: store.operationErrors[r.id]?[action] ?? "").textSelection(.enabled); Button(L("重试")) { store.perform(action, r.id) }.disabled(store.operating.contains(r.id)) }.font(AppFont.secondary).foregroundStyle(Palette.warn)
            }
            if let error = store.statusErrors[r.id] {
                HStack { ErrorNotice(message: error, context: L("状态读取失败")).textSelection(.enabled); Button(L("重试状态")) { Task { await store.readStatus(r.id) } }.disabled(store.refreshing.contains(r.id)) }.font(AppFont.secondary).foregroundStyle(Palette.warn)
            }
            if let status = store.statuses[r.id] {
                Text((status.installed ? L("系统任务已安装") : L("系统任务缺失")) + " · " + (r.deployment?.enabled == false ? L("已暂停") : status.nextRunAt == nil ? L("下次执行待确认") : L("下次 ") + Format.date(status.nextRunAt))).font(AppFont.secondary)
                DisclosureGroup(L("最近运行记录（{0}）" , status.history.count)) {
                    ForEach(status.history) { run in VStack(alignment: .leading) { Text(Format.date(run.startedAt) + " · " + L(run.label)); Text(run.model + " / " + (run.effort.isEmpty ? L("默认") : run.effort)).foregroundStyle(.secondary) }.font(AppFont.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 4) }
                }
            }
        }.padding(.vertical, 8)
    }
}

struct WakeEditor: View {
    @ObservedObject var model: AppModel
    @State var task: WakeTask
    var deployed: Bool
    var save: (WakeTask) async throws -> Void
    @Environment(\.dismiss) private var dismiss
    @StateObject private var targets = AccountTargetsModel()
    @State private var selectedProvider = ""
    @State private var selectedAccount = ""
    @State private var initialDraft = ""
    @State private var confirmDiscard = false
    @State private var capabilities: WakeCapabilities?
    @State private var suggestion: WakeSuggestion?
    @State private var suggestionLoading = false
    /// Only the untouched default of a new task is replaced by a suggestion; otherwise it is added.
    @State private var timesEdited = false
    @State private var associationSource: AgentSource?
    @State private var busy = false
    @State private var error: String?
    private let engine = EngineClient()
    private var sources: [AgentSource] { model.settings.sources.filter { !$0.unbound && (!$0.accountId.isEmpty || $0.codexHomeId != nil) && CoreContract.wakeupProviders.contains($0.provider) } }
    private var efforts: [String] { capabilities?.models.first { $0.id == task.model }?.efforts ?? capabilities?.efforts ?? ["low","medium","high","xhigh","max","ultra"] }
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage;
        VStack(alignment: .leading, spacing: 16) {
            Text(task.id.isEmpty ? L("新建唤醒任务") : L("编辑唤醒任务")).font(AppFont.title)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    LabeledContent(L("名称")) { TextField(L("名称"), text: $task.name).labelsHidden() }
                    Picker("Agent", selection: Binding(get: { selectedProvider }, set: {
                        selectedProvider = $0; selectedAccount = ""; task.sourceId = ""; task.codexProfileId = nil
                        task.binary = ""; task.model = ""; task.effort = ""; capabilities = nil
                    })) {
                        Text(L("请选择 Agent")).tag("")
                        ForEach(CoreContract.wakeupProviders, id: \.self) { Text(Format.provider($0)).tag($0) }
                    }.disabled(deployed).accessibilityIdentifier("wake-agent")
                    Picker(L("账户"), selection: Binding(get: { selectedAccount }, set: { selectedAccount = $0; task.sourceId = ""; task.codexProfileId = nil; task.model = ""; capabilities = nil })) {
                        Text(L("请选择账户")).tag("")
                        ForEach(model.settings.accounts.filter { a in a.archived != true && CoreContract.sameAgent(a.provider, selectedProvider) && targets.rows.contains { $0.accountKey == a.key && CoreContract.wakeupProviders.contains($0.provider) } }, id: \.key) { Text($0.name).tag($0.key) }
                    }.disabled(deployed || selectedProvider.isEmpty).accessibilityIdentifier("wake-account")
                    Picker(L("运行位置"), selection: Binding(get: { targets.rows.first { $0.accountKey == selectedAccount && $0.sourceId == task.sourceId && $0.profileId == task.codexProfileId }?.id ?? "" }, set: { id in
                        guard let target = targets.rows.first(where: { $0.id == id }) else { return }
                        task.sourceId = target.sourceId; task.codexProfileId = target.profileId; task.model = ""; capabilities = nil
                    })) {
                        Text(L("请选择位置")).tag("")
                        ForEach(targets.rows.filter { $0.accountKey == selectedAccount }) { target in
                            Text(target.wakeupLocationLabel).tag(target.id).help(target.path)
                        }
                    }.disabled(deployed)
                    if let target = targets.rows.first(where: { $0.accountKey == selectedAccount && $0.sourceId == task.sourceId && $0.profileId == task.codexProfileId }), let reason = target.draftReason ?? target.reason {
                        Text(L(reason)).font(AppFont.secondary).foregroundStyle(Palette.warn)
                    }
                    if let selected = model.settings.sources.first(where: { $0.id == task.sourceId && $0.provider == "codex" }) {
                        Button(L("完成账户关联 / 登录")) { associationSource = selected }
                    }
                    if !targets.checking.isEmpty { ProgressView(L("正在检查机器登录…")).controlSize(.small) }
                    ForEach(targets.errors, id: \.self) { Text($0).font(AppFont.secondary).foregroundStyle(Palette.warn) }
                    Button(L("重新检查登录")) { Task { await targets.load(model, includeUnavailable: true) } }.disabled(!targets.checking.isEmpty)
                    Text(L("离线时可先保存草稿。更换运行位置前请移除旧部署。")).font(AppFont.secondary).foregroundStyle(.secondary)
                    ForEach(task.times.indices, id: \.self) { index in
                        timeRow(index)
                    }
                    Button(L("添加时刻")) { timesEdited = true; task.times.append("08:00") }
                    suggestionRow
                    Text(L("跟随目标机器时区，错过时刻不补发。")).font(AppFont.secondary).foregroundStyle(.secondary)
                    DisclosureGroup(L("执行选项")) { TextField(L("CLI 路径（留空使用数据源配置）"), text: $task.binary) }
                    Button(L("读取可用模型")) { probe() }.disabled(task.sourceId.isEmpty)
                    if let capabilities { Text(capabilities.timezone + " · " + L(capabilities.message)).font(AppFont.secondary).foregroundStyle(.secondary)
                        if !capabilities.models.isEmpty {
                            Picker(L("可用模型"), selection: $task.model) {
                                Text(L("自定义模型 ID")).tag("")
                                ForEach(capabilities.models) { item in Text(item.id).tag(item.id) }
                            }
                        }
                    }
                    LabeledContent(L("模型 ID")) { TextField(L("模型 ID"), text: $task.model).labelsHidden() }
                    Picker(L("推理强度"), selection: $task.effort) { Text(L("模型默认")).tag(""); ForEach(efforts, id: \.self) { Text($0).tag($0) } }
                    Text(L("提示词")).font(AppFont.section)
                    TextEditor(text: $task.prompt).font(AppFont.body).scrollContentBackground(.hidden)
                        .padding(.horizontal, DesignTokens.controlPaddingX - 5).padding(.vertical, 4)
                        .frame(height: 80).modifier(AieyesControlSurface(field: true))
                    Text(L("模型请求可能消耗订阅额度，实际消耗取决于模型和提示词。")).font(AppFont.secondary).foregroundStyle(.secondary)
                }.textFieldStyle(.aieyes)
            }.frame(maxHeight: 490)
            if let error { ErrorNotice(message: error) }
            AccountFlowNotice(model: model)
            HStack { if busy { ProgressView().controlSize(.small) }; Spacer(); Button(L("取消")) { if initialDraft != draftSnapshot(task) { confirmDiscard = true } else { dismiss() } }.keyboardShortcut(.cancelAction); Button(L("保存草稿")) { submit() }.buttonStyle(.aieyesPrimary).disabled(task.sourceId.isEmpty || task.model.isEmpty) }
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogWakeupWidth).font(AppFont.body).disabled(busy).onAppear {
            if task.sourceId.isEmpty { task.sourceId = sources.first?.id ?? "" }
            selectedProvider = model.settings.sources.first { $0.id == task.sourceId }.map { CoreContract.agentProvider($0.provider) } ?? ""
            initialDraft = draftSnapshot(task)
        }
        .interactiveDismissDisabled(busy || initialDraft != draftSnapshot(task))
        .discardDraftConfirmation($confirmDiscard) { dismiss() }
        .updateDraftGuard(L("定时唤醒"), snapshot: draftSnapshot(task), dirty: !initialDraft.isEmpty && initialDraft != draftSnapshot(task), saving: busy, save: { await saveDraft() }, discard: { dismiss() })
        .task { await targets.load(model, includeUnavailable: true) }
        .sheet(item: $associationSource, onDismiss: { Task { await targets.load(model, includeUnavailable: true) } }) { source in
            AccountConnectionView(model: model, account: model.settings.accounts.first { $0.key == selectedAccount }, initialSourceID: source.id, intent: "login")
        }
        .onChange(of: targets.rows.map(\.id)) { _, _ in
            if selectedAccount.isEmpty, let target = targets.rows.first(where: { $0.sourceId == task.sourceId && $0.profileId == task.codexProfileId }) {
                selectedAccount = target.accountKey; selectedProvider = CoreContract.agentProvider(target.provider)
            }
        }
        .onChange(of: task.sourceId, initial: true) { _, _ in capabilities = nil; loadSuggestion() }
        .onChange(of: task.codexProfileId) { _, _ in capabilities = nil; loadSuggestion() }
        .onChange(of: task.binary) { _, _ in capabilities = nil; task.model = "" }
        .onChange(of: task.model) { _, _ in if !efforts.contains(task.effort) { task.effort = efforts.contains("low") ? "low" : "" } }
    }
    @ViewBuilder private var suggestionRow: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            if suggestionLoading { ProgressView().controlSize(.small) }
            Text(WakeSuggestion.text(suggestion, loading: suggestionLoading, shared: model.settings.sources.first { $0.id == task.sourceId }?.codexHomeId != nil))
                .font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            if let time = suggestion?.suggested { Button(L("采用")) { task.times = WakeSuggestion.apply(time, to: task.times, edited: timesEdited || !task.id.isEmpty); timesEdited = true } }
        }
    }
    private func loadSuggestion() {
        let sourceID = task.sourceId, profile = task.codexProfileId
        suggestion = nil; suggestionLoading = false
        guard !sourceID.isEmpty else { return }
        suggestionLoading = true
        Task { @MainActor in
            var parameters: [String: Any] = ["sourceId": sourceID]
            if let profile { parameters["codexProfileId"] = profile }
            let value: WakeSuggestion? = try? await engine.call("wakeups.suggest", params: parameters)
            guard sourceID == task.sourceId && profile == task.codexProfileId else { return }
            suggestion = value; suggestionLoading = false
        }
    }
    private func timeRow(_ index: Int) -> some View {
        HStack(spacing: 8) {
            Text(L("目标时刻"))
            Picker(L("目标时间 {0}：小时", index + 1), selection: timePart(index, component: 0)) {
                ForEach(0..<24, id: \.self) { Text(String(format: "%02d", $0)).tag($0) }
            }.pickerStyle(.menu).labelsHidden().frame(width: 76)
            Text(":").accessibilityHidden(true)
            Picker(L("目标时间 {0}：分钟", index + 1), selection: timePart(index, component: 1)) {
                ForEach(0..<60, id: \.self) { Text(String(format: "%02d", $0)).tag($0) }
            }.pickerStyle(.menu).labelsHidden().frame(width: 76)
            Text(localTime(task.times[index])).font(AppFont.secondary).foregroundStyle(.secondary)
            Spacer()
            Button(L("移除")) { timesEdited = true; task.times.remove(at: index) }
                .accessibilityLabel(L("移除时刻 {0}", task.times[index]))
        }
    }
    // Select wall-clock components directly: the viewer's time zone never changes the stored time.
    private func timePart(_ index: Int, component: Int) -> Binding<Int> {
        func parts() -> [Int] {
            guard task.times.indices.contains(index) else { return [8, 0] }
            let values = task.times[index].split(separator: ":").compactMap { Int($0) }
            guard values.count == 2, (0..<24).contains(values[0]), (0..<60).contains(values[1]) else { return [8, 0] }
            return values
        }
        return Binding(get: { parts()[component] }, set: { value in
            guard task.times.indices.contains(index) else { return }
            var next = parts(); next[component] = value
            timesEdited = true
            task.times[index] = String(format: "%02d:%02d", next[0], next[1])
        })
    }
    private func localTime(_ time: String) -> String {
        WakeupTime.next(time, zone: capabilities?.timezone).label
    }
    private func probe() {
        busy = true; error = nil
        Task { @MainActor in
            defer { busy = false }
            do { var input = task; input.model = ""; input.effort = ""; let result: WakeCapabilities = try await engine.call("wakeups.probe", params: ["task":input.parameters]); guard input.sourceId == task.sourceId && input.codexProfileId == task.codexProfileId && input.binary == task.binary else { return }; capabilities = result; if task.model.isEmpty { task.model = result.recommendedModel ?? "" }; if !efforts.contains(task.effort) { task.effort = efforts.contains("low") ? "low" : "" } }
            catch { self.error = error.localizedDescription }
        }
    }
    private func submit() {
        Task { await saveDraft() }
    }
    @discardableResult private func saveDraft() async -> Bool {
        busy = true; error = nil
        defer { busy = false }
        do { guard deployed || targets.rows.contains(where: { $0.accountKey == selectedAccount && $0.sourceId == task.sourceId && $0.profileId == task.codexProfileId && ($0.canDraft ?? $0.canWake) }) else { throw ClientError.message(L("此账户与所选目录的关联或身份尚未确认，请检查账户设置")) }; try await save(task); return true } catch { self.error = error.localizedDescription; return false }
    }
}
