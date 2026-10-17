import SwiftUI

struct WakeupsView: View {
    @ObservedObject var model: AppModel
    @StateObject private var store = WakeupsStore()
    @State private var editing: WakeTask?
    @State private var removal: WakeRecord?
    var body: some View {
        Form {
            Text("按设定时间调用订阅模型。").font(AppFont.section.weight(.semibold)).foregroundStyle(Palette.accent).frame(maxWidth: .infinity, alignment: .leading).padding(12).background(Palette.accent.opacity(0.10), in: RoundedRectangle(cornerRadius: 8))
            Section("定时唤醒") {
                Text("使用本机或 SSH Linux 服务器的订阅登录，按目标机器时区运行。").foregroundStyle(.secondary)
                DisclosureGroup("执行规则") {
                    Text("保存草稿后需部署，修改后需更新部署。退出 Aieyes 后任务继续运行；本机 macOS / Windows 需保持用户登录。休眠、离线或错过时刻不补发。模型请求可能消耗额度。").font(AppFont.secondary).foregroundStyle(.secondary)
                }
                HStack {
                    Button("新建任务") { editing = WakeTask() }.buttonStyle(.aieyesPrimary)
                    Button("刷新列表") { Task { await store.load() } }.disabled(store.loading)
                    if store.loading { ProgressView().controlSize(.small) }
                }
                if let error = store.listError { ErrorNotice(message: error) }
                if store.records.isEmpty && !store.loading { Text("尚未配置定时任务").foregroundStyle(.secondary) }
            }
            ForEach(store.records) { record in Section { taskCard(record) } }
        }.formStyle(AdaptiveGroupedFormStyle()).background(OverlayScrollStyle()).font(AppFont.body).disabled(model.installingUpdate).task { await store.load() }
        .updateOperation("定时唤醒任务", active: !store.operating.isEmpty)
        .sheet(item: $removal) { record in
            DangerConfirmation(title: record.deployment == nil ? "删除任务草稿？" : "移除自动任务？", explanation: record.deployment == nil ? "删除「" + record.task.name + "」的任务草稿。" : "将从目标机器移除「" + record.task.name + "」的定时部署，后续自动唤醒会停止。运行历史保留。", affected: [], confirmLabel: "确认移除", cancel: { removal = nil }, confirm: { removal = nil; store.perform(record.deployment == nil ? "delete" : "remove", record.id) })
        }
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
                Text(r.deployment.map { $0.state == "settings-pending" ? "部署待同步" : $0.state == "pending-removal" ? "待移除" : $0.state == "deployment-unconfirmed" ? "部署待确认" : $0.enabled ? "已部署" : "已停用" } ?? "未部署").foregroundStyle(.secondary)
            }
            Text("\(r.task.times.joined(separator: "、")) · \(r.task.model) · \(r.task.effort.isEmpty ? "默认推理强度" : r.task.effort)").textSelection(.enabled)
            if let d = r.deployment { Text(d.target + " · " + (store.statuses[r.id]?.timezone ?? d.timezone) + (d.changed ? " · 草稿待部署" : "")).font(AppFont.secondary).foregroundStyle(.secondary) }
            HStack {
                Button("编辑") { editing = r.task }
                Button(r.deployment == nil ? "部署" : needsDeploy ? "重新部署" : r.deployment?.changed == true ? "更新部署" : "查看运行结果") { store.perform(needsDeploy || r.deployment?.changed == true ? "deploy" : "status", r.id) }.buttonStyle(.aieyesPrimary)
                Menu("更多操作") {
                    if let d = r.deployment, !needsDeploy {
                        Button(d.enabled ? "停用" : "启用") { store.perform(d.enabled ? "disable" : "enable", r.id) }
                        Button("立即运行") { store.perform("run", r.id) }.help("立即调用模型，可能消耗订阅额度")
                        Button("状态与记录") { store.perform("status", r.id) }
                    }
                    Button(r.deployment == nil ? "删除草稿" : "移除自动任务", role: .destructive) { removal = r }
                }
            }.disabled(store.operating.contains(r.id))
            if let notice = store.notices[r.id] { Text(notice).font(AppFont.secondary).foregroundStyle(Palette.ok) }
            ForEach((store.operationErrors[r.id] ?? [:]).keys.sorted(), id: \.self) { action in
                HStack { ErrorNotice(message: store.operationErrors[r.id]?[action] ?? "").textSelection(.enabled); Button("重试") { store.perform(action, r.id) }.disabled(store.operating.contains(r.id)) }.font(AppFont.secondary).foregroundStyle(Palette.warn)
            }
            if let error = store.statusErrors[r.id] {
                HStack { ErrorNotice(message: error, context: "状态读取失败").textSelection(.enabled); Button("重试状态") { Task { await store.readStatus(r.id) } }.disabled(store.refreshing.contains(r.id)) }.font(AppFont.secondary).foregroundStyle(Palette.warn)
            }
            if let status = store.statuses[r.id] {
                Text((status.installed ? "系统任务已安装" : "系统任务缺失") + " · " + (r.deployment?.enabled == false ? "已暂停" : status.nextRunAt == nil ? "下次执行待确认" : "下次 " + Format.date(status.nextRunAt))).font(AppFont.secondary)
                DisclosureGroup("最近运行记录（\(status.history.count)）") {
                    ForEach(status.history) { run in VStack(alignment: .leading) { Text(Format.date(run.startedAt) + " · " + run.label); Text(run.model + " / " + (run.effort.isEmpty ? "默认" : run.effort)).foregroundStyle(.secondary) }.font(AppFont.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 4) }
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
    @State private var selectedAccount = ""
    @State private var initialDraft = ""
    @State private var confirmDiscard = false
    @State private var capabilities: WakeCapabilities?
    @State private var busy = false
    @State private var error: String?
    private let engine = EngineClient()
    private var sources: [AgentSource] { model.settings.sources.filter { !$0.unbound && (!$0.accountId.isEmpty || $0.codexHomeId != nil) && CoreContract.wakeupProviders.contains($0.provider) } }
    private var efforts: [String] { capabilities?.models.first { $0.id == task.model }?.efforts ?? capabilities?.efforts ?? ["low","medium","high","xhigh","max","ultra"] }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(task.id.isEmpty ? "新建唤醒任务" : "编辑唤醒任务").font(AppFont.title)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("名称", text: $task.name)
                    Picker("账户", selection: Binding(get: { selectedAccount }, set: { selectedAccount = $0; task.sourceId = ""; task.codexProfileId = nil; task.model = ""; capabilities = nil })) {
                        Text("请选择账户").tag("")
                        ForEach(model.settings.accounts.filter { a in a.archived != true && targets.rows.contains { $0.accountKey == a.key && CoreContract.wakeupProviders.contains($0.provider) } }, id: \.key) { Text($0.name).tag($0.key) }
                    }.disabled(deployed)
                    Picker("运行位置", selection: Binding(get: { targets.rows.first { $0.accountKey == selectedAccount && $0.sourceId == task.sourceId && $0.profileId == task.codexProfileId }?.id ?? "" }, set: { id in
                        guard let target = targets.rows.first(where: { $0.id == id }) else { return }
                        task.sourceId = target.sourceId; task.codexProfileId = target.profileId; task.model = ""; capabilities = nil
                    })) {
                        Text("请选择位置").tag("")
                        ForEach(targets.rows.filter { $0.accountKey == selectedAccount }) { target in
                            Text(target.wakeupLocationLabel).tag(target.id).help(target.path)
                        }
                    }.disabled(deployed)
                    if !targets.checking.isEmpty { ProgressView("正在检查机器登录…").controlSize(.small) }
                    ForEach(targets.errors, id: \.self) { Text($0).font(AppFont.secondary).foregroundStyle(Palette.warn) }
                    Button("重新检查登录") { Task { await targets.load(model, includeUnavailable: true) } }.disabled(!targets.checking.isEmpty)
                    Text("离线时可先保存草稿。更换运行位置前请移除旧部署。").font(AppFont.secondary).foregroundStyle(.secondary)
                    ForEach(task.times.indices, id: \.self) { index in
                        HStack { Text("目标时刻"); TextField("HH:mm", text: $task.times[index]).frame(width: 80); Text(localTime(task.times[index])).font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Button("移除") { task.times.remove(at: index) }.accessibilityLabel("移除时刻 " + task.times[index]) }
                    }
                    Button("添加时刻") { task.times.append("08:00") }
                    Text("跟随目标机器时区，错过时刻不补发。").font(AppFont.secondary).foregroundStyle(.secondary)
                    TextField("CLI 路径（留空使用数据源配置）", text: $task.binary)
                    Button("读取可用模型") { probe() }.disabled(task.sourceId.isEmpty)
                    if let capabilities { Text(capabilities.timezone + " · " + capabilities.message).font(AppFont.secondary).foregroundStyle(.secondary)
                        if !capabilities.models.isEmpty {
                            Picker("可用模型", selection: $task.model) {
                                Text("自定义模型 ID").tag("")
                                ForEach(capabilities.models) { item in Text(item.id).tag(item.id) }
                            }
                        }
                    }
                    TextField("模型 ID", text: $task.model)
                    Picker("推理强度", selection: $task.effort) { Text("模型默认").tag(""); ForEach(efforts, id: \.self) { Text($0).tag($0) } }
                    Text("提示词").font(AppFont.section)
                    TextEditor(text: $task.prompt).frame(height: 80).overlay(RoundedRectangle(cornerRadius: 6).stroke(.secondary.opacity(0.2)))
                    Text("模型请求可能消耗订阅额度，实际消耗取决于模型和提示词。").font(AppFont.secondary).foregroundStyle(.secondary)
                }.textFieldStyle(.aieyes)
            }.frame(maxHeight: 490)
            if let error { ErrorNotice(message: error) }
            HStack { if busy { ProgressView().controlSize(.small) }; Spacer(); Button("取消") { if initialDraft != draftSnapshot(task) { confirmDiscard = true } else { dismiss() } }.keyboardShortcut(.cancelAction); Button("保存草稿") { submit() }.buttonStyle(.aieyesPrimary).disabled(task.sourceId.isEmpty || task.model.isEmpty) }
        }.padding(DesignTokens.dialogPadding).frame(width: DesignTokens.dialogWakeupWidth).font(AppFont.body).disabled(busy).onAppear { if task.sourceId.isEmpty { task.sourceId = sources.first?.id ?? "" }; initialDraft = draftSnapshot(task) }
        .interactiveDismissDisabled(busy || initialDraft != draftSnapshot(task))
        .discardDraftConfirmation($confirmDiscard) { dismiss() }
        .updateDraftGuard("定时唤醒", snapshot: draftSnapshot(task), dirty: !initialDraft.isEmpty && initialDraft != draftSnapshot(task), saving: busy, save: { await saveDraft() }, discard: { dismiss() })
        .task { await targets.load(model, includeUnavailable: true) }
        .onChange(of: targets.rows.map(\.id)) { _, _ in
            if selectedAccount.isEmpty { selectedAccount = targets.rows.first { $0.sourceId == task.sourceId && $0.profileId == task.codexProfileId }?.accountKey ?? "" }
        }
        .onChange(of: task.sourceId) { _, _ in capabilities = nil }
        .onChange(of: task.codexProfileId) { _, _ in capabilities = nil }
        .onChange(of: task.binary) { _, _ in capabilities = nil; task.model = "" }
        .onChange(of: task.model) { _, _ in if !efforts.contains(task.effort) { task.effort = efforts.contains("low") ? "low" : "" } }
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
        do { guard deployed || targets.rows.contains(where: { $0.accountKey == selectedAccount && $0.sourceId == task.sourceId && $0.profileId == task.codexProfileId && ($0.canDraft ?? $0.canWake) }) else { throw ClientError.message("此账户与所选目录的关联或身份尚未确认，请检查账户设置") }; try await save(task); return true } catch { self.error = error.localizedDescription; return false }
    }
}
