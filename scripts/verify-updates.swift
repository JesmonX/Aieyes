import Foundation
import Sparkle

@main struct VerifyUpdates {
    @MainActor static func main() async {
        let suite = "aieyes-updates-test-" + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let updater = AppUpdater(defaults: defaults)
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
        updater.showDownloadDidReceiveExpectedContentLength(100)
        updater.showDownloadDidReceiveData(ofLength: 70)
        updater.showDownloadDidReceiveData(ofLength: 50)
        precondition(updater.busy && updater.downloaded == 120 && updater.expected == 100)
        updater.cancelDownload()
        precondition(canceled && !updater.busy)
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
        precondition(resumed && acknowledgements == 4)
        print("macOS update states, preferences, cancellation, signature failure and restart preparation passed")
    }
}
