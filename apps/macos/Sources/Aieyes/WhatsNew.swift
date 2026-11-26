import SwiftUI

/// Release notes bundled with this build: `docs/release-notes/v<version>.md`, copied by the
/// build script. The file holds an English part and a Chinese part, in either order.
struct ReleaseNotes: Equatable {
    struct Part: Equatable {
        var headline: String?
        var intro: [String] = [], items: [String] = [], outro: [String] = []
    }
    var english = Part(), chinese = Part()
    /// The compare link that closes the file, when present.
    var changelog: URL?

    /// The part for the interface language; the other part stands in when it has no items.
    func part(chinese preferred: Bool) -> Part {
        let first = preferred ? chinese : english, second = preferred ? english : chinese
        return first.items.isEmpty ? second : first
    }

    /// A heading that reads `中文` or `English` selects that language's part, in any order and any
    /// number of times. Lines before the first such heading belong to the language the file gives
    /// no heading (English when it names neither or both).
    init?(markdown: String) {
        var lead = Part(), named: [Bool: Part] = [:]  // keyed by "is Chinese"
        var target: Bool?
        func edit(_ change: (inout Part) -> Void) {
            if let target { change(&named[target, default: Part()]) } else { change(&lead) }
        }
        for raw in markdown.components(separatedBy: .newlines) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.isEmpty { continue }
            if line.hasPrefix("#") {
                let text = line.drop { $0 == "#" }.trimmingCharacters(in: .whitespaces)
                // The marker headings are file structure, not interface copy.
                if text == #"中文"# || text.lowercased() == "english" {
                    target = text == #"中文"#
                    edit { _ in }
                } else if !text.isEmpty {
                    edit { if $0.headline == nil { $0.headline = text } }
                }
            } else if line.hasPrefix("- ") || line.hasPrefix("* ") {
                edit { $0.items.append(String(line.dropFirst(2))) }
            } else if line.hasPrefix("["), line.hasSuffix(")"), let open = line.range(of: "](", options: .backwards) {
                changelog = URL(string: String(line[open.upperBound...].dropLast()))
            } else {
                edit { if $0.items.isEmpty { $0.intro.append(line) } else { $0.outro.append(line) } }
            }
        }
        let unnamed = named[false] != nil && named[true] == nil
        if let rest = named[unnamed] {
            named[unnamed] = Part(headline: lead.headline ?? rest.headline, intro: lead.intro + rest.intro, items: lead.items + rest.items, outro: lead.outro + rest.outro)
        } else {
            named[unnamed] = lead
        }
        english = named[false] ?? Part()
        chinese = named[true] ?? Part()
        guard !english.items.isEmpty || !chinese.items.isEmpty else { return nil }
    }

    /// Nil for a build whose version has no notes file; the entry points then stay hidden.
    static let bundled: ReleaseNotes? = Bundle.main.url(forResource: "ReleaseNotes", withExtension: "md")
        .flatMap { try? String(contentsOf: $0, encoding: .utf8) }
        .flatMap(ReleaseNotes.init(markdown:))
    static var version: String { Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "" }
}

/// What changed in the running version.
struct WhatsNewView: View {
    let notes: ReleaseNotes
    let version: String
    var onClose: () -> Void
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    private func rich(_ text: String) -> AttributedString {
        (try? AttributedString(markdown: text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(text)
    }
    var body: some View {
        let part = notes.part(chinese: interfaceLanguage != "en")
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 4) {
                Text(L("本次更新")).font(AppFont.title)
                Text(verbatim: "Aieyes v" + version + (part.headline.map { " · " + $0 } ?? "")).font(AppFont.secondary).foregroundStyle(.secondary)
            }.padding(.horizontal, DesignTokens.dialogPadding).padding(.top, DesignTokens.dialogPadding).padding(.bottom, 14)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(Array(part.intro.enumerated()), id: \.offset) { _, line in
                        Text(rich(line)).font(AppFont.secondary).foregroundStyle(.secondary)
                    }
                    ForEach(Array(part.items.enumerated()), id: \.offset) { _, item in
                        HStack(alignment: .firstTextBaseline, spacing: 10) {
                            Circle().fill(Palette.accent).frame(width: 5, height: 5).alignmentGuide(.firstTextBaseline) { $0[.bottom] + 3 }
                            Text(rich(item)).fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    ForEach(Array(part.outro.enumerated()), id: \.offset) { _, line in
                        Text(rich(line)).font(AppFont.secondary).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, DesignTokens.dialogPadding).padding(.bottom, 12)
            }.background(OverlayScrollStyle())
            HStack {
                if let changelog = notes.changelog { Link(L("完整变更记录"), destination: changelog) }
                Spacer()
                Button(L("完成")) { onClose() }.buttonStyle(.aieyesPrimary).keyboardShortcut(.defaultAction)
            }.padding(DesignTokens.dialogPadding)
        }
        .font(AppFont.body).frame(width: 580, height: EditorLayout.height(600)).aieyesAccent()
    }
}

/// Shown once after an update, in the panel and on the overview: open the notes or dismiss.
struct WhatsNewBanner: View {
    @ObservedObject var service: OutboundEventService
    var open: () -> Void
    @AppStorage("aieyes.language") private var interfaceLanguage = "zh-CN"
    var body: some View {
        let _ = interfaceLanguage
        if service.showsWhatsNew, ReleaseNotes.bundled != nil, let version = service.launch?.version {
            HStack(spacing: 8) {
                Image(systemName: "sparkles").foregroundStyle(Palette.accent).accessibilityHidden(true)
                Text(L("已更新到 v{0}", version)).font(AppFont.secondary).lineLimit(1)
                Spacer(minLength: 8)
                Button(L("查看本次更新")) { open() }.buttonStyle(.plain).font(AppFont.secondary).foregroundStyle(Palette.accent)
                Button { withMotion(.standard) { service.showsWhatsNew = false } } label: { Image(systemName: "xmark") }
                    .buttonStyle(.plain).foregroundStyle(.secondary).accessibilityLabel(L("关闭提示"))
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .background(Palette.accent.opacity(DesignTokens.controlTintOpacity), in: RoundedRectangle(cornerRadius: DesignTokens.radiusOverlay))
        }
    }
}

/// Hosts the release-notes sheet, so the window itself does not observe the outbound service.
struct WhatsNewPresenter: View {
    @ObservedObject var service: OutboundEventService
    var body: some View {
        Color.clear.frame(width: 0, height: 0).accessibilityHidden(true)
            .sheet(isPresented: $service.presentingWhatsNew) {
                if let notes = ReleaseNotes.bundled {
                    WhatsNewView(notes: notes, version: ReleaseNotes.version) { service.presentingWhatsNew = false }
                }
            }
    }
}
