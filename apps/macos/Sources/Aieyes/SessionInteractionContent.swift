import Foundation
import SwiftUI

/// Line classification of a file change for the approval card. Pure, so the native verifier covers it.
enum InteractionDiff {
    enum LineKind: Equatable { case added, removed, context, hunk, meta }
    struct Line: Equatable {
        var kind: LineKind
        var text: String
    }

    /// Lines to show for a change: the agent's unified diff when there is one, otherwise the replaced
    /// and the new text as removed and added lines around the lines they share.
    static func lines(for change: InteractionFileChange) -> [Line] {
        if !change.diff.isEmpty { return unified(change.diff) }
        return replaced(old: change.oldText, new: change.newText)
    }

    static func unified(_ diff: String) -> [Line] {
        split(diff).map { line in
            if line.hasPrefix("+++") || line.hasPrefix("---") || line.hasPrefix("diff ") || line.hasPrefix("index ") || line.hasPrefix("*** ") {
                return Line(kind: .meta, text: line)
            }
            if line.hasPrefix("@@") { return Line(kind: .hunk, text: line) }
            if line.hasPrefix("+") { return Line(kind: .added, text: line) }
            if line.hasPrefix("-") { return Line(kind: .removed, text: line) }
            return Line(kind: .context, text: line)
        }
    }

    /// Without a diff the two texts are shown in full, minus up to `context` shared lines kept at
    /// each end for orientation. Nothing that differs is ever left out.
    static func replaced(old: String, new: String, context: Int = 2) -> [Line] {
        let before = split(old), after = split(new)
        var head = 0
        while head < before.count, head < after.count, before[head] == after[head] { head += 1 }
        var tail = 0
        while tail < before.count - head, tail < after.count - head, before[before.count - 1 - tail] == after[after.count - 1 - tail] { tail += 1 }
        var lines: [Line] = []
        lines += before[max(0, head - context)..<head].map { Line(kind: .context, text: "  " + $0) }
        lines += before[head..<(before.count - tail)].map { Line(kind: .removed, text: "- " + $0) }
        lines += after[head..<(after.count - tail)].map { Line(kind: .added, text: "+ " + $0) }
        lines += before[(before.count - tail)..<min(before.count, before.count - tail + context)].map { Line(kind: .context, text: "  " + $0) }
        return lines
    }

    /// "+3 −1" for the change header.
    static func summary(_ lines: [Line]) -> String {
        let added = lines.filter { $0.kind == .added }.count, removed = lines.filter { $0.kind == .removed }.count
        return added + removed == 0 ? "" : "+\(added) −\(removed)"
    }

    private static func split(_ text: String) -> [String] {
        guard !text.isEmpty else { return [] }
        var lines = text.components(separatedBy: "\n").map { $0.hasSuffix("\r") ? String($0.dropLast()) : $0 }
        if lines.last == "" { lines.removeLast() }
        return lines
    }

    static func attributed(_ lines: [Line]) -> AttributedString {
        var out = AttributedString()
        for (index, line) in lines.enumerated() {
            var piece = AttributedString(line.text + (index == lines.count - 1 ? "" : "\n"))
            switch line.kind {
            case .added:
                piece.foregroundColor = Palette.ok
                piece.backgroundColor = Palette.ok.opacity(0.10)
            case .removed:
                piece.foregroundColor = Palette.danger
                piece.backgroundColor = Palette.danger.opacity(0.10)
            case .hunk: piece.foregroundColor = Palette.accent
            case .meta: piece.foregroundColor = .secondary
            case .context: break
            }
            out += piece
        }
        return out
    }
}

/// Light Markdown for a plan: headings, lists and inline styles are shown as the agent wrote them,
/// fenced code stays verbatim. Raw HTML and images are never interpreted.
enum InteractionMarkdown {
    enum Block: Equatable {
        case heading(level: Int, text: String)
        case text(String)
        case code(String)
        case blank
    }

    static func blocks(_ markdown: String) -> [Block] {
        var blocks: [Block] = []
        var fenced = false
        for raw in markdown.components(separatedBy: "\n") {
            let line = raw.hasSuffix("\r") ? String(raw.dropLast()) : raw
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~") {
                fenced.toggle()
                continue
            }
            if fenced {
                blocks.append(.code(line))
            } else if trimmed.isEmpty {
                if blocks.last != .blank, !blocks.isEmpty { blocks.append(.blank) }
            } else if let level = headingLevel(trimmed) {
                blocks.append(.heading(level: level, text: String(trimmed.dropFirst(level)).trimmingCharacters(in: .whitespaces)))
            } else {
                blocks.append(.text(line))
            }
        }
        if blocks.last == .blank { blocks.removeLast() }
        return blocks
    }

    private static func headingLevel(_ line: String) -> Int? {
        let level = line.prefix(while: { $0 == "#" }).count
        guard (1...6).contains(level), line.dropFirst(level).first == " " else { return nil }
        return level
    }

    static func attributed(_ markdown: String) -> AttributedString {
        let inline = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace, failurePolicy: .returnPartiallyParsedIfPossible)
        var out = AttributedString()
        let all = blocks(markdown)
        for (index, block) in all.enumerated() {
            var piece: AttributedString
            switch block {
            case .heading(let level, let text):
                piece = (try? AttributedString(markdown: text, options: inline)) ?? AttributedString(text)
                piece.font = .system(size: level <= 2 ? DesignTokens.fontEntry : DesignTokens.fontCaption, weight: .semibold)
            case .text(let text):
                piece = (try? AttributedString(markdown: text, options: inline)) ?? AttributedString(text)
                // A plan is read, not navigated: links stay text so a click cannot leave the card.
                for run in piece.runs where run.link != nil { piece[run.range].link = nil }
            case .code(let text):
                piece = AttributedString(text)
                piece.font = .system(size: DesignTokens.fontCaption, design: .monospaced)
            case .blank:
                piece = AttributedString("")
            }
            out += piece
            if index < all.count - 1 { out += AttributedString("\n") }
        }
        return out
    }
}
