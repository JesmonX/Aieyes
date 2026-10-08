import Foundation
import Sparkle
import Combine

@main struct VerifyUpdates {
    @MainActor static func main() async {
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
        precondition(updater.phase == "current" && updater.message.contains("当前已是最新版本"))
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
