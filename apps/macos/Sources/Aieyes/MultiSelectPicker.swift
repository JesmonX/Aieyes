import SwiftUI

struct SelectionOption: Identifiable {
    var id: String
    var label: String
    var unavailable = false
}

struct MultiSelectPicker: View {
    var title: String
    var options: [SelectionOption]
    var selected: Set<String>
    var all: Bool? = nil
    var onChange: (Set<String>, Bool) -> Void
    @State private var presented = false
    @State private var query = ""
    @FocusState private var searchFocused: Bool
    @FocusState private var triggerFocused: Bool

    private var term: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var filtered: [SelectionOption] {
        options.filter { term.isEmpty || ($0.id + " " + $0.label).localizedCaseInsensitiveContains(term) }
    }
    private var count: Int { options.filter { selected.contains($0.id) }.count }
    private var summary: String {
        if all ?? (!options.isEmpty && count == options.count) { return "全部" }
        return count == 0 ? "未选择" : "已选 \(count) / \(options.count)"
    }
    private func apply(_ action: MonitorSelection.Action) {
        let targets = Set(filtered.map(\.id))
        onChange(MonitorSelection.apply(selected, targets: targets, action: action), action == .all && term.isEmpty)
    }

    var body: some View {
        Button {
            query = ""
            presented.toggle()
        } label: {
            HStack {
                Text(title)
                Spacer()
                Text(summary).foregroundStyle(.secondary)
                Image(systemName: "chevron.down").font(.system(size: 11, weight: .semibold)).foregroundStyle(.secondary)
            }.font(AppFont.secondary).padding(.vertical, 5)
        }
        .focused($triggerFocused)
        .accessibilityLabel("\(title)，\(summary)")
        .popover(isPresented: $presented, arrowEdge: .bottom) {
            VStack(spacing: 10) {
                TextField("搜索", text: $query).textFieldStyle(.roundedBorder)
                    .accessibilityLabel("搜索\(title)").focused($searchFocused).onSubmit { }
                HStack(spacing: 12) {
                    Button(term.isEmpty ? "全选" : "全选结果") { apply(.all) }
                    Button(term.isEmpty ? "清空" : "清空结果") { apply(.clear) }
                    Button(term.isEmpty ? "反选" : "反选结果") { apply(.invert) }
                    Spacer()
                    Text("\(count) / \(options.count)").foregroundStyle(.secondary).monospacedDigit()
                }.font(AppFont.secondary).buttonStyle(.borderless).disabled(filtered.isEmpty)
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(filtered) { option in
                            Toggle(isOn: Binding(get: { selected.contains(option.id) }, set: { enabled in
                                onChange(MonitorSelection.apply(selected, targets: [option.id], action: enabled ? .all : .clear), false)
                            })) {
                                HStack {
                                    Text(option.label).fixedSize(horizontal: false, vertical: true)
                                    if option.unavailable { Spacer(); Text("暂不可用").font(AppFont.secondary).foregroundStyle(.secondary) }
                                }
                            }.toggleStyle(.checkbox).font(AppFont.secondary).padding(.vertical, 8)
                        }
                        if filtered.isEmpty { Text("没有匹配项").foregroundStyle(.secondary).padding() }
                    }.padding(.horizontal, 2)
                }.frame(maxHeight: 270)
            }
            .padding(14).frame(width: 380)
            .onAppear { searchFocused = true }
            .onExitCommand { presented = false }
        }
        .onChange(of: presented) { _, shown in if !shown { triggerFocused = true } }
    }
}
