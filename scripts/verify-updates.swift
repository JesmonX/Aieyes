import Foundation
import Sparkle
import Combine
@testable import Aieyes

@main struct VerifyUpdates {
    /// Bundled release notes: each language heading selects its own part, in any order and any
    /// number of times (the same cases as scripts/test-experience-ui.cjs).
    static func verifyReleaseNotes() {
        typealias Part = ReleaseNotes.Part
        func parse(_ lines: [String]) -> ReleaseNotes { ReleaseNotes(markdown: lines.joined(separator: "\n"))! }
        let chineseHeading = #"## 中文"#, note = #"说明"#, first = #"- 甲"#, second = #"- 乙"#, title = #"标题"#
        let link = "[x](https://example.com/c)"
        let forward = parse(["## English", "Intro", "- a", "- b", "After", chineseHeading, note, first, link])
        let reversed = parse([chineseHeading, note, first, "## English", "Intro", "- a", "- b", "After", link])
        precondition(forward.english == Part(headline: nil, intro: ["Intro"], items: ["a", "b"], outro: ["After"]))
        precondition(forward.chinese == Part(headline: nil, intro: [note], items: [String(first.dropFirst(2))], outro: []))
        precondition(forward.changelog == URL(string: "https://example.com/c"))
        precondition(reversed == forward, "Chinese before English parses like English before Chinese")
        let repeated = parse([#"# 中文"#, first, "### english", "## Title", "- a", chineseHeading, "## " + title, second, "## English", "- b"])
        precondition(repeated.english == Part(headline: "Title", intro: [], items: ["a", "b"], outro: []))
        precondition(repeated.chinese == Part(headline: title, intro: [], items: [String(first.dropFirst(2)), String(second.dropFirst(2))], outro: []))
        precondition(parse(["## Title", "Intro", "- a", chineseHeading, first]).english == Part(headline: "Title", intro: ["Intro"], items: ["a"], outro: []))
        let chineseLead = parse(["## " + title, first, "## English", "- a"])
        precondition(chineseLead.chinese == Part(headline: title, intro: [], items: [String(first.dropFirst(2))], outro: []) && chineseLead.english == Part(headline: nil, intro: [], items: ["a"], outro: []))
        precondition(parse(["# Release", "For all", chineseHeading, first, "## English", "- a"]).english == Part(headline: "Release", intro: ["For all"], items: ["a"], outro: []))
        precondition(ReleaseNotes(markdown: "# Only a title\nSome text") == nil)
        let fallback = parse(["# T", "- a", chineseHeading, note])
        precondition(fallback.part(chinese: true) == fallback.english)
        // Every notes file in the repository, found from this source file's location.
        let directory = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("docs/release-notes")
        let files = ((try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []).filter { $0.pathExtension == "md" }
        precondition(files.count >= 70, "release notes not found at " + directory.path)
        for file in files {
            let text = try! String(contentsOf: file, encoding: .utf8)
            guard let notes = ReleaseNotes(markdown: text) else { preconditionFailure(file.lastPathComponent + " has no items") }
            let headings = Set(text.components(separatedBy: .newlines).map { $0.trimmingCharacters(in: .whitespaces) }.filter { $0.hasPrefix("#") }
                .map { $0.drop { $0 == "#" }.trimmingCharacters(in: .whitespaces).lowercased() })
            if headings.contains(String(chineseHeading.dropFirst(3))) { precondition(!(notes.chinese.items.isEmpty && notes.chinese.intro.isEmpty), file.lastPathComponent) }
            if headings.contains("english") { precondition(!(notes.english.items.isEmpty && notes.english.intro.isEmpty), file.lastPathComponent) }
            for part in [notes.english, notes.chinese] { precondition(!["english", String(chineseHeading.dropFirst(3))].contains(part.headline?.lowercased() ?? ""), file.lastPathComponent) }
        }
        let current = try! String(contentsOf: directory.appendingPathComponent("v0.2.14.md"), encoding: .utf8)
        let shipped = ReleaseNotes(markdown: current)!
        precondition(shipped.english.items.count == 3 && shipped.chinese.items.count == 3, "v0.2.14 lists Chinese before English")
    }
    @MainActor static func main() async {
        verifyReleaseNotes()
        let suite = "aieyes-updates-test-" + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        var updater = AppUpdater(defaults: defaults)
        for selector in ["feedURLStringForUpdater:", "updater:willDownloadUpdate:withRequest:", "updater:shouldDownloadReleaseNotesForUpdate:"] {
            precondition(updater.responds(to: NSSelectorFromString(selector)), "Sparkle delegate hook missing: " + selector)
        }
        precondition(updater.automatic)
        updater.automatic = false
        precondition(!AppUpdater(defaults: defaults).automatic)
        var acknowledgements = 0
        updater.showUpdateNotFoundWithError(NSError(domain: SUSparkleErrorDomain, code: 1001, userInfo: [SPUNoUpdateFoundReasonKey: 1])) { acknowledgements += 1 }
        precondition(updater.phase == "current" && updater.message.contains("已是最新版本"))
        updater.showUpdateNotFoundWithError(NSError(domain: SUSparkleErrorDomain, code: 1001, userInfo: [SPUNoUpdateFoundReasonKey: 2])) { acknowledgements += 1 }
        precondition(updater.message.contains("高于最新稳定版"))
        updater.showUpdateNotFoundWithError(NSError(domain: SUSparkleErrorDomain, code: 1001, userInfo: [SPUNoUpdateFoundReasonKey: 3])) { acknowledgements += 1 }
        precondition(updater.phase == "error" && !updater.message.contains("当前已是最新"))
        var canceled = false
        updater.showDownloadInitiated { canceled = true }
        var stateChanges = 0
        let subscription = updater.objectWillChange.sink { stateChanges += 1 }
        updater.showDownloadDidReceiveExpectedContentLength(100)
        updater.showDownloadDidReceiveData(ofLength: 70)
        updater.showDownloadDidReceiveData(ofLength: 50)
        precondition(updater.busy && updater.downloaded == 120 && updater.expected == 100)
        precondition(stateChanges == 0, "Download progress must not invalidate the updater's parent views")
        withExtendedLifetime(subscription) {}
        updater.cancelDownload()
        precondition(canceled && !updater.busy)
        updater.showUpdaterError(NSError(domain: NSURLErrorDomain, code: -999)) { acknowledgements += 1 }
        precondition(updater.phase == "idle" && updater.message == "已取消更新", "Late network callbacks must not overwrite cancellation")
        var lateDownloadCanceled = false
        updater.showDownloadInitiated { lateDownloadCanceled = true }
        updater.showDownloadDidStartExtractingUpdate()
        precondition(lateDownloadCanceled && updater.phase == "idle")
        updater = AppUpdater(defaults: defaults)
        updater.showDownloadInitiated { }
        updater.showUpdaterError(NSError(domain: SUSparkleErrorDomain, code: 3001, userInfo: [NSLocalizedDescriptionKey: "签名校验失败"])) { acknowledgements += 1 }
        precondition(updater.phase == "error" && updater.message == "签名校验失败")
        var installed = false
        updater.prepareInstallation = { _ in false }
        updater.showReady(toInstallAndRelaunch: { installed = $0 == .install })
        try? await Task.sleep(nanoseconds: 10_000_000)
        precondition(!installed && updater.phase == "available")
        updater.prepareInstallation = { finalizing in precondition(finalizing); return true }
        updater.install()
        try? await Task.sleep(nanoseconds: 10_000_000)
        precondition(installed && updater.phase == "installing")
        var resumed = false
        updater.finishedInstallationAttempt = { resumed = true }
        updater.dismissUpdateInstallation()
        precondition(resumed && acknowledgements == 5)
        print("macOS update states, preferences, cancellation, signature failure and restart preparation passed")
    }
}
