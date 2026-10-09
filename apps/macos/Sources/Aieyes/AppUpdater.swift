import AppKit
import SwiftUI
import Sparkle
import OSLog

@MainActor final class UpdateProgress: ObservableObject {
    @Published var downloaded: UInt64 = 0
    @Published var expected: UInt64 = 0
}

/// One updater per application, independent of the sampling engine and window lifetime.
@MainActor final class AppUpdater: NSObject, ObservableObject, SPUUserDriver, SPUUpdaterDelegate, NSWindowDelegate {
    static let shared = AppUpdater()
    @Published private(set) var phase = "idle" {
        didSet { if phase != oldValue { logger.info("phase=\(self.phase, privacy: .public) elapsed_ms=\(Int(Date().timeIntervalSince(self.cycleStarted) * 1000))") } }
    }
    @Published private(set) var hasUpdate = false
    @Published private(set) var message = ""
    @Published private(set) var latestVersion = ""
    @Published private(set) var notes = ""
    let progress = UpdateProgress()
    var downloaded: UInt64 { progress.downloaded }
    var expected: UInt64 { progress.expected }
    @Published var automatic: Bool {
        didSet { defaults.set(automatic, forKey: "updates.automatic") }
    }
    private let defaults: UserDefaults
    override convenience init() { self.init(defaults: .standard) }
    init(defaults: UserDefaults) {
        self.defaults = defaults
        self.automatic = defaults.object(forKey: "updates.automatic") as? Bool ?? true
        super.init()
    }
    var currentVersion: String { Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "开发版本" }
    var busy: Bool { ["checking", "downloading", "extracting", "installing", "preparing"].contains(phase) }
    var prepareInstallation: ((Bool) async -> Bool)?
    var finishedInstallationAttempt: (() -> Void)?
    private let logger = Logger(subsystem: "app.aieyes.desktop", category: "updater")
    private let transport = UpdateTransport()
    private var checkTask: Task<Void, Never>?
    private var deadline: Task<Void, Never>?
    private var cycle = 0
    private var cycleStarted = Date()
    private var interruptedMessage: String?
    private var originalFeed: URL?
    private var updater: SPUUpdater?
    private var timer: Timer?
    private var window: NSWindow?
    private var choice: ((SPUUserUpdateChoice) -> Void)?
    private var cancellation: (() -> Void)?
    private var retryTermination: (() -> Void)?
    private var readyToInstall = false
    private var manual = false
    private var wantsPresentation = false

    func start() {
        guard updater == nil else { return }
        guard let key = Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") as? String, !key.isEmpty else {
            message = "此开发构建未配置更新签名公钥"; return
        }
        let service = SPUUpdater(hostBundle: .main, applicationBundle: .main, userDriver: self, delegate: self)
        service.automaticallyChecksForUpdates = false
        service.automaticallyDownloadsUpdates = false
        service.sendsSystemProfile = false
        do {
            try service.start(); updater = service; originalFeed = service.feedURL
            NotificationCenter.default.addObserver(self, selector: #selector(becameActive), name: NSApplication.didBecomeActiveNotification, object: nil)
            timer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in Task { @MainActor in self?.scheduledCheck() } }
            if automatic { check(manual: false) }
        } catch { phase = "error"; message = error.localizedDescription }
    }
    private func scheduledCheck() {
        let last = defaults.double(forKey: "updates.lastCheck")
        if automatic && Date().timeIntervalSince1970 - last >= 86400 { check(manual: false) }
    }
    func check(manual: Bool = true) {
        guard let updater else { phase = "error"; message = "此构建未配置应用内更新，请安装正式发布版本。"; return }
        if choice != nil || (busy && phase != "checking") { if manual { present() }; return }
        guard updater.canCheckForUpdates, !busy else { return }
        guard let feed = originalFeed else { phase = "error"; message = "此构建缺少更新清单地址"; return }
        cycle += 1; let request = cycle
        interruptedMessage = nil; cycleStarted = Date()
        self.manual = manual; phase = "checking"; message = "正在检查更新…"
        defaults.set(Date().timeIntervalSince1970, forKey: "updates.lastCheck")
        deadline?.cancel()
        deadline = Task { [weak self] in
            try? await Task.sleep(for: .seconds(25))
            guard !Task.isCancelled, let self, self.cycle == request, self.phase == "checking" else { return }
            self.interruptCheck(message: "检查更新超时，请检查应用代理后重试", failed: true)
        }
        checkTask = Task { [weak self] in
            guard let self else { return }
            do {
                try await transport.start(feed: feed)
                guard cycle == request, !Task.isCancelled else { return }
                // Scheduling is application-owned; no background focus stealing.
                updater.checkForUpdates()
            } catch {
                guard cycle == request, !Task.isCancelled else { return }
                deadline?.cancel(); phase = "error"; message = error.localizedDescription
            }
        }
    }
    func install() {
        guard let choice, !busy else { return }
        phase = "preparing"; message = "正在准备更新…"
        Task {
            guard await prepareInstallation?(readyToInstall) ?? true else { phase = "available"; message = "请先完成或保存当前操作，再点击更新。"; return }
            if !readyToInstall, let feed = originalFeed {
                do { try await transport.start(feed: feed) }
                catch { phase = "available"; message = error.localizedDescription; return }
            }
            self.choice = nil; phase = readyToInstall ? "installing" : "downloading"; message = readyToInstall ? "正在安装并重启…" : "正在下载更新…"; choice(.install)
        }
    }
    func later() {
        guard !busy else { return }
        defaults.set(latestVersion, forKey: "updates.deferredVersion")
        defaults.set(Date().addingTimeInterval(86400).timeIntervalSince1970, forKey: "updates.deferUntil")
        let reply = choice; choice = nil; wantsPresentation = false; reply?(readyToInstall ? .skip : .dismiss); window?.orderOut(nil)
    }
    func cancelDownload() { interruptCheck(message: "已取消更新", failed: false) }
    private func interruptCheck(message: String, failed: Bool) {
        interruptedMessage = message; cycle += 1
        deadline?.cancel(); checkTask?.cancel(); checkTask = nil
        let cancel = cancellation; cancellation = nil
        transport.stop(); cancel?()
        phase = failed ? "error" : "idle"; self.message = message
    }
    private func finishCheck() { deadline?.cancel(); deadline = nil; cancellation = nil }
    func feedURLString(for updater: SPUUpdater) -> String? {
        transport.feedURL?.absoluteString ?? (originalFeed == nil ? nil : "http://127.0.0.1:1/unavailable")
    }
    func updater(_ updater: SPUUpdater, shouldDownloadReleaseNotesForUpdate item: SUAppcastItem) -> Bool { false }
    func updater(_ updater: SPUUpdater, willDownloadUpdate item: SUAppcastItem, with request: NSMutableURLRequest) {
        // Fail closed: an invalid remote URL must never silently bypass the app proxy.
        request.url = request.url.flatMap { transport.archiveURL(for: $0) } ?? URL(string: "http://127.0.0.1:1/invalid-update")!
        request.cachePolicy = .reloadIgnoringLocalCacheData
        request.timeoutInterval = 620
    }
    func retryRestart() { retryTermination?() }
    @objc private func becameActive() { if wantsPresentation { present() } }
    private func present() {
        wantsPresentation = false
        if window == nil {
            let view = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 510, height: 400), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            view.title = "Aieyes 更新"; view.isReleasedWhenClosed = false; view.delegate = self
            view.contentMinSize = NSSize(width: 450, height: 300)
            view.contentView = NSHostingView(rootView: UpdateWindowView(updater: self)); view.center(); window = view
        }
        window?.makeKeyAndOrderFront(nil)
    }
    func windowShouldClose(_ sender: NSWindow) -> Bool { if !busy { later() }; sender.orderOut(nil); return false }

    func show(_ request: SPUUpdatePermissionRequest, reply: @escaping (SUUpdatePermissionResponse) -> Void) {
        reply(SUUpdatePermissionResponse(automaticUpdateChecks: false, sendSystemProfile: false))
    }
    func showUserInitiatedUpdateCheck(cancellation: @escaping () -> Void) { if interruptedMessage != nil { cancellation() } else { self.cancellation = cancellation } }
    func showUpdateFound(with appcastItem: SUAppcastItem, state: SPUUserUpdateState, reply: @escaping (SPUUserUpdateChoice) -> Void) {
        finishCheck()
        guard interruptedMessage == nil else { reply(.dismiss); return }
        guard !appcastItem.isInformationOnlyUpdate else {
            phase = "error"; message = "此版本暂未提供可安装的更新包"; reply(.dismiss); return
        }
        hasUpdate = true
        latestVersion = appcastItem.displayVersionString; notes = appcastItem.itemDescription ?? ""
        phase = "available"; message = "发现新版本 v\(latestVersion)"; choice = reply; cancellation = nil
        if !manual && defaults.string(forKey: "updates.deferredVersion") == latestVersion && defaults.double(forKey: "updates.deferUntil") > Date().timeIntervalSince1970 {
            choice = nil; reply(.dismiss); return
        }
        if NSApp.isActive { present() } else { wantsPresentation = true }
    }
    func showUpdateReleaseNotes(with downloadData: SPUDownloadData) { }
    func showUpdateReleaseNotesFailedToDownloadWithError(_ error: Error) { }
    func showUpdateNotFoundWithError(_ error: Error, acknowledgement: @escaping () -> Void) {
        finishCheck()
        guard interruptedMessage == nil else { acknowledgement(); return }
        hasUpdate = false
        let info = (error as NSError).userInfo
        let reason = (info[SPUNoUpdateFoundReasonKey] as? NSNumber)?.intValue
        if let item = info[SPULatestAppcastItemFoundKey] as? SUAppcastItem { latestVersion = item.displayVersionString }
        switch reason {
        case 1: phase = "current"; message = "当前已是最新版本 v\(currentVersion)"
        case 2: phase = "current"; message = "当前版本 v\(currentVersion) 高于最新稳定版 v\(latestVersion)"
        default: phase = "error"; message = "没有适用于当前系统的更新：" + error.localizedDescription
        }
        acknowledgement()
    }
    func showUpdaterError(_ error: Error, acknowledgement: @escaping () -> Void) {
        finishCheck()
        guard interruptedMessage == nil else { acknowledgement(); return }
        phase = "error"; message = transport.lastFailure ?? error.localizedDescription; acknowledgement()
    }
    func showDownloadInitiated(cancellation: @escaping () -> Void) {
        guard interruptedMessage == nil else { cancellation(); return }
        self.cancellation = cancellation; progress.downloaded = 0; progress.expected = 0; phase = "downloading"; message = "正在下载更新…"
    }
    func showDownloadDidReceiveExpectedContentLength(_ expectedContentLength: UInt64) { if interruptedMessage == nil { progress.expected = expectedContentLength } }
    func showDownloadDidReceiveData(ofLength length: UInt64) { if interruptedMessage == nil { progress.downloaded += length } }
    func showDownloadDidStartExtractingUpdate() { guard interruptedMessage == nil else { return }; cancellation = nil; phase = "extracting"; message = "正在校验并解包更新…" }
    func showExtractionReceivedProgress(_ progress: Double) { }
    func showReady(toInstallAndRelaunch reply: @escaping (SPUUserUpdateChoice) -> Void) {
        guard interruptedMessage == nil else { reply(.dismiss); return }
        readyToInstall = true
        Task {
            if await prepareInstallation?(true) ?? true { phase = "installing"; message = "正在安装并重启…"; reply(.install) }
            else { phase = "available"; message = "更新已就绪，请先完成或保存当前操作。"; choice = reply }
        }
    }
    func showInstallingUpdate(withApplicationTerminated applicationTerminated: Bool, retryTerminatingApplication: @escaping () -> Void) {
        phase = "installing"; message = applicationTerminated ? "正在安装更新…" : "正在等待应用退出；如取消过退出，可重试重启。"
        retryTermination = applicationTerminated ? nil : retryTerminatingApplication
    }
    func showUpdateInstalledAndRelaunched(_ relaunched: Bool, acknowledgement: @escaping () -> Void) { hasUpdate = false; phase = "installed"; message = "更新已安装"; acknowledgement() }
    func dismissUpdateInstallation() { finishedInstallationAttempt?(); readyToInstall = false; choice = nil; cancellation = nil; retryTermination = nil; wantsPresentation = false }
    func showUpdateInFocus() { present() }
    func updater(_ updater: SPUUpdater, didFinishUpdateCycleFor updateCheck: SPUUpdateCheck, error: Error?) {
        finishCheck()
        if interruptedMessage == nil, let error, (error as NSError).code != 1001 {
            phase = "error"; message = transport.lastFailure ?? error.localizedDescription
        }
        transport.stop()
    }
}

struct UpdateSettingsView: View {
    @ObservedObject var updater = AppUpdater.shared
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("当前版本 v\(updater.currentVersion)")
            Toggle("启动时及每天检查更新", isOn: $updater.automatic)
            HStack { Button(updater.phase == "checking" ? "检查中…" : "检查更新") { updater.check() }.disabled(updater.busy); if updater.phase == "checking" { Button("取消检查") { updater.cancelDownload() } }; if updater.phase == "available" || (updater.busy && updater.phase != "checking") { Button(updater.busy ? "查看进度" : "查看更新") { updater.check() } } }
            if !updater.message.isEmpty { Text(updater.message).font(AppFont.secondary).foregroundStyle(updater.phase == "error" ? Palette.warn : Color.secondary).textSelection(.enabled) }
        }.aieyesAccent().font(AppFont.body)
    }
}
struct UpdateWindowView: View {
    @ObservedObject var updater: AppUpdater
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Aieyes 更新").font(AppFont.title)
            Text("当前版本 v\(updater.currentVersion) → 新版本 v\(updater.latestVersion)").font(AppFont.section)
            ScrollView { Text(updater.notes.isEmpty ? "此版本包含改进与修复。" : updater.notes).frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled) }.frame(maxHeight: .infinity)
            Text(updater.message).font(AppFont.secondary).foregroundStyle(updater.phase == "error" ? Palette.warn : Color.secondary)
            if updater.phase == "downloading" { UpdateProgressView(progress: updater.progress) }
            HStack {
                if updater.phase == "downloading" { Button("取消下载") { updater.cancelDownload() } }
                Spacer()
                if updater.phase == "installing" { Button("重试重启") { updater.retryRestart() } }
                else if !updater.busy {
                    Button("稍后") { updater.later() }.keyboardShortcut(.cancelAction)
                    if updater.phase == "available" { Button("更新并重启") { updater.install() }.buttonStyle(.borderedProminent) }
                    else { Button("重新检查") { updater.check() } }
                }
            }
        }.aieyesAccent().font(AppFont.body).padding(24)
    }
}

struct UpdateProgressView: View {
    @ObservedObject var progress: UpdateProgress
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if progress.expected > 0 { ProgressView(value: Double(progress.downloaded), total: Double(max(progress.expected, progress.downloaded))) }
            else { ProgressView().controlSize(.small) }
            Text(ByteCountFormatter.string(fromByteCount: Int64(clamping: progress.downloaded), countStyle: .file) + (progress.expected > 0 ? " / " + ByteCountFormatter.string(fromByteCount: Int64(clamping: progress.expected), countStyle: .file) : "")).font(AppFont.secondary)
        }
    }
}

struct UpdateMenuButton: View {
    @ObservedObject private var updater = AppUpdater.shared
    var body: some View {
        Button { updater.check() } label: {
            Image(systemName: "arrow.down.circle").frame(width: 28, height: 28)
                .overlay(alignment: .topTrailing) { if updater.hasUpdate { Text("!").font(.system(size: 11, weight: .bold)).foregroundStyle(Palette.warn) } }
        }.help(updater.hasUpdate ? "发现新版本 · 查看更新" : "检查更新")
            .accessibilityLabel(updater.hasUpdate ? "发现新版本，查看更新" : "检查更新")
            .disabled(updater.phase == "checking")
    }
}
