import SwiftUI

private struct NativeScanRow: Decodable, Identifiable {
    var provider: String, machineId: String, status: String
    var message: String?, configPath: String?
    var id: String { provider + machineId + status + (configPath ?? "") + (message ?? "") }
}
private struct NativeScanResult: Decodable {
    var settings: Settings, addedAccounts: Int, addedConnections: Int, results: [NativeScanRow]
}
@MainActor private final class NativeScanController: ObservableObject {
    static let shared = NativeScanController()
    @Published var busy = false
    @Published var stage = ""
    @Published var rows: [NativeScanRow] = []
    func scan(_ model: AppModel, provider: String?, machine: String?) async {
        guard !busy else { return }
        busy = true; stage = "正在读取原生配置…"; rows = []
        defer { busy = false }
        var parameters: [String: Any] = ["scope": machine == nil ? "agent" : "machine"]
        if let machine { parameters["machineId"] = machine }
        if let provider { parameters["provider"] = provider }
        do {
            let result: NativeScanResult = try await model.engine.call("accounts.scan", params: parameters, onProgress: { [weak self] stage in Task { @MainActor in if self?.busy == true { self?.stage = stage } } })
            model.acceptConfiguration(result.settings)
            rows = result.results
            let failures = rows.filter { $0.status == "failed" }.count
            stage = "新增 \(result.addedAccounts) 个账户、\(result.addedConnections) 个连接" + (failures > 0 ? "；\(failures) 项读取失败" : "")
            Task { await model.loadAccountStatuses() }
        } catch {
            if let settings: Settings = try? await model.engine.call("settings.get") { model.acceptConfiguration(settings) }
            stage = error.localizedDescription
        }
    }
}
@MainActor struct NativeScanButton: View {
    @SurfaceModel var model: AppModel
    var provider: String? = nil
    var machine: String? = nil
    @ObservedObject private var scanner = NativeScanController.shared
    @State private var showing = false
    private var available: Bool {
        if let machine { return machine == "local" ? model.settings.localDeviceEnabled != false : model.settings.hosts.contains { $0.id == machine && $0.enabled } }
        guard let provider else { return false }
        return (model.settings.agents ?? []).contains { a in CoreContract.sameAgent(a.provider, provider) && a.provider != "deepseek" && a.provider != "custom" && a.enabled && a.machineIds.contains { id in id == "local" ? model.settings.localDeviceEnabled != false : model.settings.hosts.contains { $0.id == id && $0.enabled } } }
    }
    var body: some View {
        Button(machine == nil ? "扫描已启用机器" : "一键扫描账户与凭据") { showing = true; Task { await scanner.scan(model, provider: provider, machine: machine) } }
            .disabled(scanner.busy || !available)
            .help(available ? "读取现有登录和原生 API 配置，凭据保留在原机器" : "先启用 Agent、选择机器并开启 Agent 采集")
            .sheet(isPresented: $showing) {
                VStack(alignment: .leading, spacing: 14) {
                    Text("扫描账户与凭据").font(AppFont.title)
                    if scanner.busy { ProgressView().controlSize(.small) }
                    Text(scanner.stage).textSelection(.enabled)
                    ScrollView { LazyVStack(alignment: .leading, spacing: 12) {
                        ForEach(Array(scanner.rows.enumerated()), id: \.offset) { _, row in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(Format.provider(row.provider) + " · " + model.machineName(row.machineId)).font(AppFont.section)
                                Text(row.message ?? row.status).font(AppFont.secondary).foregroundStyle(row.status == "failed" ? Palette.warn : .secondary)
                                if let path = row.configPath { Text(path).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled) }
                            }.frame(maxWidth: .infinity, alignment: .leading)
                        }
                    } }
                    HStack {
                        if !scanner.busy { Button("重新扫描") { Task { await scanner.scan(model, provider: provider, machine: machine) } } }
                        Spacer(); Button(scanner.busy ? "关闭，后台继续" : "完成") { showing = false }
                    }
                }.padding(22).frame(width: 640, height: 480)
            }
    }
}
@MainActor struct NativeConnectionsSection: View {
    @SurfaceModel var model: AppModel
    let provider: String, kind: String
    @ObservedObject private var scanner = NativeScanController.shared
    @State private var removing: String?
    @State private var error: String?
    @State private var expanded = false
    private var rows: [NativeConnection] { (model.settings.nativeConnections ?? []).filter { CoreContract.sameAgent($0.provider, provider) && $0.kind == kind } }
    var body: some View {
        if !rows.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text((kind == "api" ? "原生 API 连接" : "已发现的原生登录凭据") + "（\(rows.count)）").font(AppFont.section)
                    Spacer()
                    Button("一键取消引用") { remove(rows.map(\.id)) }.disabled(removing != nil || scanner.busy)
                }
                DisclosureGroup(kind == "api" ? "查看连接" : "查看凭据", isExpanded: $expanded) {
                ForEach(rows) { row in
                    HStack(alignment: .top) {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(row.displayName)
                            Text(model.machineName(row.machineId) + " · " + row.statusLabel + (row.accountKey == nil ? "" : " · 已关联账户")).font(AppFont.secondary).foregroundStyle(.secondary)
                            if !row.models.isEmpty { Text(row.models.joined(separator: "、")).font(AppFont.secondary) }
                            Text(row.configPath).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled)
                            Text(row.note).font(AppFont.secondary).foregroundStyle(.secondary)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                        Button("取消引用") { remove([row.id]) }.disabled(removing != nil || scanner.busy)
                    }
                }
                Text("凭据保留在原机器的 Agent 配置中；取消引用不会删除原文件。").font(AppFont.secondary).foregroundStyle(.secondary)
                }
                if let error { Text(error).foregroundStyle(Palette.warn) }
            }
        }
    }
    private func remove(_ ids: [String]) {
        guard removing == nil else { return }
        removing = ids.first
        Task {
            defer { removing = nil }
            do {
                let result: NativeRemovalResult = try await model.engine.call("accounts.native.remove", params: ["connectionIds":ids])
                model.acceptConfiguration(result.settings); error = nil
            } catch { self.error = error.localizedDescription }
        }
    }
}
private struct NativeRemovalResult: Decodable { var settings: Settings }
