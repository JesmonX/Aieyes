import SwiftUI

struct SSHConfigResult: Decodable {
    struct Entry: Decodable, Identifiable {
        var alias: String
        var hostName: String?, user: String?, port: Int?
        var id: String { alias }
        var preview: String { [user, hostName ?? "跟随 SSH 配置", port.map(String.init)].compactMap { $0 }.joined(separator: " · ") }
    }
    var path: String, hosts: [Entry], warnings: [String]
}
struct SSHConfigImportView: View {
    @ObservedObject var model: AppModel
    var preview: SSHConfigResult? = nil
    var imported: () -> Void = {}
    @Environment(\.dismiss) private var dismiss
    @State private var result: SSHConfigResult?
    @State private var selected: Set<String> = []
    @State private var search = ""
    @State private var error: String?
    @State private var busy = false
    private var existing: Set<String> { Set(model.settings.hosts.map { $0.target.lowercased() }) }
    private var rows: [SSHConfigResult.Entry] { (result?.hosts ?? []).filter { search.isEmpty || ($0.alias + " " + $0.preview).localizedCaseInsensitiveContains(search) } }
    var body: some View {
        VStack(spacing: 0) {
            HStack { Text("从 SSH 配置导入").font(AppFont.title); Spacer(); Button("取消") { dismiss() }.disabled(busy) }.padding(22)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let result {
                        Text(result.path).font(AppFont.secondary).foregroundStyle(.secondary).textSelection(.enabled)
                        Text("沿用 SSH 别名、密钥和跳板配置。导入后可测试连接。").foregroundStyle(.secondary)
                        TextField("搜索别名或地址", text: $search).textFieldStyle(.aieyes)
                        HStack {
                            Button("全选当前结果") { selected.formUnion(rows.filter { !existing.contains($0.alias.lowercased()) }.map(\.alias)) }
                            Button("清空选择") { selected.removeAll() }
                        }
                        ForEach(rows) { entry in
                            Toggle(isOn: Binding(get: { selected.contains(entry.alias) }, set: { if $0 { selected.insert(entry.alias) } else { selected.remove(entry.alias) } })) {
                                VStack(alignment: .leading, spacing: 4) { Text(entry.alias).bold(); Text(entry.preview + (existing.contains(entry.alias.lowercased()) ? " · 已添加" : "")).font(AppFont.secondary).foregroundStyle(.secondary) }.frame(maxWidth: .infinity, alignment: .leading)
                            }.toggleStyle(.checkbox).disabled(existing.contains(entry.alias.lowercased()))
                            Divider()
                        }
                        if result.hosts.isEmpty { Text("未找到 SSH 别名，可手动添加服务器。").foregroundStyle(.secondary) }
                        ForEach(result.warnings, id: \.self) { Text($0).font(AppFont.secondary).foregroundStyle(Palette.warn).textSelection(.enabled) }
                    } else if error == nil { ProgressView("正在读取 SSH 配置…") }
                    if let error { ErrorNotice(message: error); if result == nil { Button("重试读取") { Task { await load() } } } }
                }.padding(.horizontal, 22).padding(.bottom, 16).background(OverlayScrollStyle())
            }.disabled(busy)
            Divider()
            HStack { Text("已选 \(selected.count) 台").foregroundStyle(.secondary); Spacer(); Button(busy ? "导入中…" : "导入所选服务器") { Task { await save() } }.buttonStyle(.aieyesPrimary).disabled(selected.isEmpty || busy) }.padding(20)
        }.font(AppFont.body).frame(width: 650, height: EditorLayout.height(600)).interactiveDismissDisabled(busy).task { if let preview { result = preview } else { await load() } }
    }
    private func load() async {
        error = nil
        do { result = try await model.engine.call("hosts.sshConfig.read") }
        catch { self.error = error.localizedDescription }
    }
    private func save() async {
        busy = true; error = nil; defer { busy = false }
        let base = model.settings; var next = base
        for entry in result?.hosts ?? [] where selected.contains(entry.alias) && !existing.contains(entry.alias.lowercased()) {
            var host = Host(); host.name = entry.alias; host.target = entry.alias
            next.hosts.append(host)
        }
        do { try await model.persistConfiguration(next, base: base); dismiss(); imported() }
        catch { self.error = error.localizedDescription }
    }
}
