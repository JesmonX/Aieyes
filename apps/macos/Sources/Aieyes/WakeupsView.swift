import SwiftUI

struct WakeTask: Codable, Identifiable, Equatable {
    var codexProfileId: String?
    var id = "", name = "订阅唤醒", sourceId = "", times = ["08:00"], model = "", effort = "low"
    var prompt = "Hi. Reply only OK. Do not use any tools.", binary = ""
    var parameters: [String: Any] { (try? JSONSerialization.jsonObject(with: JSONEncoder().encode(self))) as? [String: Any] ?? [:] }
}
struct WakeDeployment: Decodable { var task: WakeTask, deployedAt: Double, enabled: Bool, state: String, timezone: String, target: String, changed: Bool }
struct WakeRecord: Decodable, Identifiable { var task: WakeTask, deployment: WakeDeployment?; var id: String { task.id } }
struct WakeModel: Decodable, Identifiable { var id: String, efforts: [String], defaultEffort: String?, referenceCost: Double? }
struct WakeCapabilities: Decodable { var models: [WakeModel], efforts: [String], timezone: String, message: String, recommendedModel: String? }
struct WakeRun: Decodable, Identifiable {
    var startedAt: Double, endedAt: Double?, status: String, model: String, effort: String
    var id: String { "\(startedAt):\(model)" }
    var label: String { ["success":"已完成", "running":"运行中", "failed":"调用失败，请检查登录、模型和代理", "timeout":"调用超时", "interrupted":"执行中断", "skipped-overlap":"同账户任务运行中，已跳过"][status] ?? status }
}
struct WakeStatus: Decodable { var installed: Bool, enabled: Bool, timezone: String, nextRunAt: Double?, history: [WakeRun] }

struct WakeupsView: View {
    @ObservedObject var model: AppModel
    @State private var records: [WakeRecord] = []
    @State private var statuses: [String: WakeStatus] = [:]
    @State private var editing: WakeTask?
    @State private var removal: WakeRecord?
    @State private var busy = false
    @State private var error: String?
    @State private var success: String?
    @State private var statusErrors: [String: String] = [:]
    private let engine = EngineClient()
    var body: some View {
        Form {
            Section("定时唤醒") {
                Text("部署到本机或 SSH Linux 服务器，使用目标机器已登录的订阅 CLI。按目标机器时区执行，错过时刻不补发；退出 Aieyes 后继续运行。").foregroundStyle(.secondary)
                Text("任务草稿独立保存，修改后需要更新部署。本机任务在用户登录期间执行。").font(AppFont.secondary).foregroundStyle(.secondary)
                HStack { Button("新建任务") { editing = WakeTask() }.buttonStyle(.borderedProminent); Button("刷新列表") { Task { await load() } }; if busy { ProgressView().controlSize(.small) } }
                if let success { Text(success).foregroundStyle(Palette.ok) }
                if let error { Text(error).foregroundStyle(Palette.warn).textSelection(.enabled) }
                if records.isEmpty { Text("尚未配置定时任务").foregroundStyle(.secondary) }
            }
            ForEach(records) { record in Section { taskCard(record) } }
        }.formStyle(.grouped).font(AppFont.body).disabled(busy || model.installingUpdate).task { await load() }
        .updateOperation("定时唤醒任务", active: busy)
        .sheet(item: $removal) { record in
            DangerConfirmation(title: record.deployment == nil ? "删除任务草稿？" : "移除自动任务？", explanation: record.deployment == nil ? "删除「" + record.task.name + "」的任务草稿。" : "将从目标机器移除「" + record.task.name + "」的定时部署，后续自动唤醒会停止。运行历史保留。", affected: [], confirmLabel: "确认移除", cancel: { removal = nil }, confirm: { removal = nil; perform(record.deployment == nil ? "delete" : "remove", record.id) })
        }
        .sheet(item: $editing) { task in
            WakeEditor(model: model, task: task, deployed: records.first { $0.id == task.id }?.deployment != nil) { value in
                let _: WakeRecord = try await engine.call("wakeups.save", params: ["task": value.parameters])
                await load(); editing = nil
            }
        }
    }
    private func taskCard(_ r: WakeRecord) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack { Text(r.task.name).font(AppFont.section); Spacer(); Text(r.deployment.map { $0.state == "settings-pending" ? "部署待同步" : $0.state == "pending-removal" ? "待移除" : $0.state == "deployment-unconfirmed" ? "部署待确认" : $0.enabled ? "已部署" : "已停用" } ?? "未部署").foregroundStyle(.secondary) }
            Text("\(r.task.times.joined(separator: "、")) · \(r.task.model) · \(r.task.effort.isEmpty ? "模型默认 effort" : r.task.effort)").textSelection(.enabled)
            if let d = r.deployment { Text(d.target + " · " + (statuses[r.id]?.timezone ?? d.timezone) + (d.changed ? " · 有尚未部署的更改" : "")).font(AppFont.secondary).foregroundStyle(.secondary) }
            HStack {
                Button("编辑") { editing = r.task }
                Button(r.deployment == nil ? "部署" : r.deployment?.changed == true ? "更新部署" : "查看运行结果") { perform(r.deployment == nil || r.deployment?.changed == true ? "deploy" : "status", r.id) }.buttonStyle(.borderedProminent)
                Menu("更多操作") { if let d = r.deployment { Button(d.enabled ? "停用" : "启用") { perform(d.enabled ? "disable" : "enable", r.id) }; Button("立即运行") { perform("run", r.id) }; Button("状态与记录") { perform("status", r.id) } }; Button(r.deployment == nil ? "删除草稿" : "移除自动任务", role: .destructive) { removal = r } }
            }
            if let error = statusErrors[r.id] { Text("状态读取失败：" + error).font(AppFont.secondary).foregroundStyle(Palette.warn) }
            if let status = statuses[r.id] {
                Text((status.installed ? "系统任务存在" : "系统任务缺失") + " · " + (r.deployment?.enabled == false ? "已暂停" : status.nextRunAt == nil ? "下次执行待确认" : "下次 " + Format.date(status.nextRunAt))).font(AppFont.secondary)
                DisclosureGroup("最近运行记录（\(status.history.count)）") {
                    ForEach(status.history) { run in VStack(alignment: .leading) { Text(Format.date(run.startedAt) + " · " + run.label); Text(run.model + " / " + (run.effort.isEmpty ? "默认" : run.effort)).foregroundStyle(.secondary) }.font(AppFont.secondary).frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 4) }
                }
            }
        }.padding(.vertical, 8)
    }
    private func load() async {
        do { records = try await engine.call("wakeups.list"); for record in records where record.deployment != nil { await readStatus(record.id) } } catch { self.error = error.localizedDescription }
    }
    private func readStatus(_ id: String) async {
        do { statuses[id] = try await engine.call("wakeups.status", params: ["id": id]); statusErrors[id] = nil }
        catch { statuses[id] = nil; statusErrors[id] = error.localizedDescription }
    }
    private func followRun(_ id: String, started: Double) async {
        for _ in 0..<6 {
            try? await Task.sleep(for: .seconds(2.5)); await readStatus(id)
            if let last = statuses[id]?.history.first, last.startedAt >= started - 1, last.status != "running" { break }
        }
    }
    private func perform(_ action: String, _ id: String) {
        busy = true; error = nil; success = nil
        Task { @MainActor in
            defer { busy = false }
            do {
                if action == "status" { let result: WakeStatus = try await engine.call("wakeups.status", params: ["id":id]); statuses[id] = result }
                else { let _: Acknowledgement = try await engine.call("wakeups." + action, params: ["id":id]); statuses[id] = nil; if action == "run" { success = "已启动一次唤醒，正在跟进运行结果。"; let started = Date().timeIntervalSince1970; Task { await followRun(id, started: started) } } }
                await load()
            } catch { self.error = error.localizedDescription; await load() }
        }
    }
}

struct WakeEditor: View {
    @ObservedObject var model: AppModel
    @State var task: WakeTask
    var deployed: Bool
    var save: (WakeTask) async throws -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var initialDraft = ""
    @State private var confirmDiscard = false
    @State private var capabilities: WakeCapabilities?
    @State private var busy = false
    @State private var error: String?
    private let engine = EngineClient()
    private var sources: [AgentSource] { model.settings.sources.filter { source in source.enabled && (!source.accountId.isEmpty || source.codexHomeId != nil) && ["codex","claude","antigravity"].contains(source.provider) && (source.hostId == nil || model.settings.hosts.contains { h in h.id == source.hostId && h.enabled }) } }
    private var efforts: [String] { capabilities?.models.first { $0.id == task.model }?.efforts ?? capabilities?.efforts ?? ["low","medium","high","xhigh","max","ultra"] }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(task.id.isEmpty ? "新建唤醒任务" : "编辑唤醒任务").font(AppFont.title)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("名称", text: $task.name)
                    Picker("账户与运行位置", selection: $task.sourceId) { Text("请选择数据源").tag(""); ForEach(sources) { source in Text(source.name + " · " + (source.hostId.flatMap { id in model.settings.hosts.first { $0.id == id }?.name } ?? "本机")).tag(source.id) } }.disabled(deployed)
                    if let home = sources.first(where: { $0.id == task.sourceId })?.codexHomeId {
                        Picker("执行账号", selection: Binding(get: { task.codexProfileId ?? "" }, set: { task.codexProfileId = $0.isEmpty ? nil : $0; capabilities = nil })) {
                            Text("请选择已关联的额度账户").tag("")
                            ForEach(model.settings.accounts.filter { $0.archived != true && $0.profileRefs.contains { $0.hasPrefix(home + ":") } }, id: \.id) { Text($0.name).tag($0.profileRefs.first { $0.hasPrefix(home + ":") } ?? "") }
                        }.disabled(deployed)
                    }
                    Text("使用已保存的数据源；切换运行位置前需要移除旧部署。").font(AppFont.secondary).foregroundStyle(.secondary)
                    ForEach(task.times.indices, id: \.self) { index in
                        HStack { Text("目标时刻"); TextField("HH:mm", text: $task.times[index]).frame(width: 80); Text(localTime(task.times[index])).font(AppFont.secondary).foregroundStyle(.secondary); Spacer(); Button("移除") { task.times.remove(at: index) }.accessibilityLabel("移除时刻 " + task.times[index]) }
                    }
                    Button("添加时刻") { task.times.append("08:00") }
                    Text("跟随目标机器时区，错过时刻不补发。").font(AppFont.secondary).foregroundStyle(.secondary)
                    TextField("CLI 路径（留空使用数据源配置）", text: $task.binary)
                    Button("读取模型与执行能力") { probe() }.disabled(task.sourceId.isEmpty)
                    if let capabilities { Text(capabilities.timezone + " · " + capabilities.message).font(AppFont.secondary).foregroundStyle(.secondary)
                        if !capabilities.models.isEmpty {
                            Picker("可用模型", selection: $task.model) {
                                Text("自定义模型 ID").tag("")
                                ForEach(capabilities.models) { item in Text(item.id).tag(item.id) }
                            }
                        }
                    }
                    TextField("模型 ID", text: $task.model)
                    Picker("Effort", selection: $task.effort) { Text("模型默认（不传参数）").tag(""); ForEach(efforts, id: \.self) { Text($0).tag($0) } }
                    Text("提示词").font(AppFont.section)
                    TextEditor(text: $task.prompt).frame(height: 80).overlay(RoundedRectangle(cornerRadius: 6).stroke(.secondary.opacity(0.2)))
                }.textFieldStyle(.roundedBorder)
            }.frame(maxHeight: 490)
            if let error { Text(error).foregroundStyle(Palette.warn).textSelection(.enabled) }
            HStack { if busy { ProgressView().controlSize(.small) }; Spacer(); Button("取消") { if initialDraft != draftSnapshot(task) { confirmDiscard = true } else { dismiss() } }.keyboardShortcut(.cancelAction); Button("保存草稿") { submit() }.buttonStyle(.borderedProminent).disabled(task.sourceId.isEmpty || task.model.isEmpty) }
        }.padding(24).frame(width: 550).font(AppFont.body).disabled(busy).onAppear { if task.sourceId.isEmpty { task.sourceId = sources.first?.id ?? "" }; initialDraft = draftSnapshot(task) }
        .interactiveDismissDisabled(busy || initialDraft != draftSnapshot(task))
        .discardDraftConfirmation($confirmDiscard) { dismiss() }
        .updateDraftGuard("定时唤醒", snapshot: draftSnapshot(task), dirty: !initialDraft.isEmpty && initialDraft != draftSnapshot(task), saving: busy, save: { await saveDraft() }, discard: { dismiss() })
        .onChange(of: task.sourceId) { _, _ in capabilities = nil; task.codexProfileId = nil }
        .onChange(of: task.model) { _, _ in if !efforts.contains(task.effort) { task.effort = efforts.contains("low") ? "low" : "" } }
    }
    private func localTime(_ time: String) -> String {
        guard let name = capabilities?.timezone, let zone = TimeZone(identifier: name) else { return "目标时区待读取" }
        let parts = time.split(separator: ":").compactMap { Int($0) }
        guard parts.count == 2, (0..<24).contains(parts[0]), (0..<60).contains(parts[1]) else { return "请输入 HH:mm" }
        var calendar = Calendar(identifier: .gregorian); calendar.timeZone = zone
        guard let next = calendar.nextDate(after: Date(), matching: DateComponents(hour: parts[0], minute: parts[1]), matchingPolicy: .nextTime) else { return "时间待确认" }
        return name + " → 本地 " + next.formatted(date: .abbreviated, time: .shortened)
    }
    private func probe() {
        busy = true; error = nil
        Task { @MainActor in
            defer { busy = false }
            do { var input = task; input.model = ""; input.effort = ""; let result: WakeCapabilities = try await engine.call("wakeups.probe", params: ["task":input.parameters]); capabilities = result; if task.model.isEmpty { task.model = result.recommendedModel ?? "" }; if !efforts.contains(task.effort) { task.effort = efforts.contains("low") ? "low" : "" } }
            catch { self.error = error.localizedDescription }
        }
    }
    private func submit() {
        Task { await saveDraft() }
    }
    @discardableResult private func saveDraft() async -> Bool {
        busy = true; error = nil
        defer { busy = false }
        do { try await save(task); return true } catch { self.error = error.localizedDescription; return false }
    }
}
