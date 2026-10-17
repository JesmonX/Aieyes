import Foundation
import Combine

struct WakeTask: Codable, Identifiable, Equatable {
    var codexProfileId: String?
    var id = "", name = "订阅唤醒", sourceId = "", times = ["08:00"], model = "", effort = "low"
    var prompt = "Hi. Reply only OK. Do not use any tools.", binary = ""
    var parameters: [String: Any] { (try? JSONSerialization.jsonObject(with: JSONEncoder().encode(self))) as? [String: Any] ?? [:] }
}
struct WakeDeployment: Decodable { var task: WakeTask, deployedAt: Double, enabled: Bool, state: String, timezone: String, target: String, changed: Bool }
struct WakeRecord: Decodable, Identifiable { var task: WakeTask, deployment: WakeDeployment?; var id: String { task.id } }
struct WakeModel: Decodable, Identifiable { var id: String, efforts: [String] }
struct WakeCapabilities: Decodable { var models: [WakeModel], efforts: [String], timezone: String, message: String, recommendedModel: String? }
struct WakeRun: Decodable, Identifiable {
    var startedAt: Double, endedAt: Double?, status: String, model: String, effort: String
    var stage: String?, errorCode: String?, message: String?
    var id: String { "\(startedAt):\(model)" }
    var label: String { message ?? (["success":"已完成", "running":"运行中", "failed":"调用失败，请检查登录、模型及代理", "timeout":"调用超时", "interrupted":"执行中断", "skipped-overlap":"同账户任务运行中，已跳过"][status] ?? status) }
}
struct WakeStatus: Decodable { var installed: Bool, enabled: Bool, timezone: String, nextRunAt: Double?, history: [WakeRun] }

@MainActor final class WakeupsStore: ObservableObject {
    @Published var records: [WakeRecord] = []
    @Published var statuses: [String: WakeStatus] = [:]
    @Published var loading = false
    @Published var refreshing = Set<String>()
    @Published var operating = Set<String>()
    @Published var listError: String?
    @Published var statusErrors: [String: String] = [:]
    @Published var operationErrors: [String: [String: String]] = [:]
    @Published var notices: [String: String] = [:]
    private let listEngine = EngineClient(configurationOnly: true)
    private var taskEngines: [String: EngineClient] = [:]
    private var listTask: Task<Void, Never>?
    private var listRevision = 0
    private var refreshAfterLoad = false
    private var taskRevisions: [String: Int] = [:]
    private var statusTasks: [String: Task<Bool, Never>] = [:]
    private var followTasks: [String: Task<Void, Never>] = [:]
    private func reader(_ id: String) -> EngineClient {
        if let reader = taskEngines[id] { return reader }
        let reader = EngineClient(configurationOnly: true); taskEngines[id] = reader; return reader
    }
    private func pruneReaders() {
        let ids = Set(records.map(\.id))
        for id in Array(taskEngines.keys) where !ids.contains(id) && !operating.contains(id) {
            followTasks.removeValue(forKey: id)?.cancel(); statusTasks.removeValue(forKey: id)?.cancel()
            taskEngines.removeValue(forKey: id)?.stop(); taskRevisions.removeValue(forKey: id)
        }
    }
    func load(refreshStatuses: Bool = true) async {
        listRevision += 1
        refreshAfterLoad = refreshAfterLoad || refreshStatuses
        if let listTask { await listTask.value; return }
        loading = true
        let work = Task { @MainActor in
            defer { loading = false; listTask = nil }
            while !Task.isCancelled {
                let revision = listRevision
                do {
                    let next: [WakeRecord] = try await listEngine.call("wakeups.list")
                    guard revision == listRevision else { continue }
                    records = next; listError = nil; pruneReaders()
                    let ids = Set(records.map(\.id))
                    statuses = statuses.filter { ids.contains($0.key) }
                    statusErrors = statusErrors.filter { ids.contains($0.key) }
                    operationErrors = operationErrors.filter { ids.contains($0.key) }
                    notices = notices.filter { ids.contains($0.key) }
                    // Publish the local list before starting independent remote reads.
                    if refreshAfterLoad {
                        for record in records where record.deployment != nil && !operating.contains(record.id) {
                            Task { await readStatus(record.id) }
                        }
                    }
                    refreshAfterLoad = false
                    return
                } catch {
                    guard revision == listRevision else { continue }
                    listError = error.localizedDescription; refreshAfterLoad = false
                    return
                }
            }
        }
        listTask = work
        await work.value
    }
    @discardableResult func readStatus(_ id: String) async -> Bool {
        if let pending = statusTasks[id] { return await pending.value }
        guard records.contains(where: { $0.id == id }) else { return false }
        refreshing.insert(id)
        let work = Task { @MainActor in
            defer { refreshing.remove(id); statusTasks[id] = nil }
            let revision = taskRevisions[id, default: 0]
            do {
                let next: WakeStatus = try await reader(id).call("wakeups.status", params: ["id": id])
                guard records.contains(where: { $0.id == id }), revision == taskRevisions[id, default: 0], !Task.isCancelled else { return false }
                statuses[id] = next; statusErrors[id] = nil
                operationErrors[id]?["status"] = nil
                return true
            } catch {
                guard records.contains(where: { $0.id == id }), revision == taskRevisions[id, default: 0], !Task.isCancelled else { return false }
                statusErrors[id] = error.localizedDescription
                return false
            }
        }
        statusTasks[id] = work
        return await work.value
    }
    func save(_ task: WakeTask) async throws {
        let id = task.id.isEmpty ? "new" : task.id
        guard operating.insert(id).inserted else { throw ClientError.message("此任务正在操作，请稍候重试") }
        defer { operating.remove(id); pruneReaders() }
        let record: WakeRecord = try await reader(id).call("wakeups.save", params: ["task": task.parameters])
        listRevision += 1; taskRevisions[record.id, default: 0] += 1
        if let index = records.firstIndex(where: { $0.id == record.id }) { records[index] = record }
        else { records.append(record) }
        operationErrors[record.id]?["save"] = nil
    }
    func perform(_ action: String, _ id: String) {
        guard operating.insert(id).inserted else { return }
        notices[id] = nil
        Task { @MainActor in
            defer { operating.remove(id); pruneReaders() }
            if action == "status" { await readStatus(id); return }
            do {
                let _: Acknowledgement = try await reader(id).call("wakeups." + action, params: ["id": id])
                taskRevisions[id, default: 0] += 1
                operationErrors[id]?[action] = nil
                if action == "run" { notices[id] = "已启动一次唤醒，正在跟进运行结果。" }
                await load(refreshStatuses: false)
                if records.contains(where: { $0.id == id && $0.deployment != nil }) { Task { await readStatus(id) } }
                if action == "run" { followRun(id) }
            } catch {
                var failures = operationErrors[id] ?? [:]; failures[action] = error.localizedDescription; operationErrors[id] = failures
            }
        }
    }
    private func followRun(_ id: String) {
        followTasks[id]?.cancel()
        let started = Date().timeIntervalSince1970
        followTasks[id] = Task {
            defer { followTasks[id] = nil }
            for _ in 0..<6 {
                do { try await Task.sleep(for: .seconds(2.5)) } catch { return }
                guard records.contains(where: { $0.id == id }) else { return }
                await readStatus(id)
                if let last = statuses[id]?.history.first, last.startedAt >= started - 1, last.status != "running" { return }
            }
        }
    }
    func stop() {
        listTask?.cancel()
        for task in statusTasks.values { task.cancel() }
        for task in followTasks.values { task.cancel() }
        listEngine.stop()
        for reader in taskEngines.values { reader.stop() }
    }
}
