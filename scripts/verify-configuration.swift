import Foundation

@main struct VerifyConfiguration {
    @MainActor static func main() async throws {
        let validation = AppModel(autostart: false)
        let intervals: [(String, WritableKeyPath<Settings, Int>, Int)] = [("quota", \.refreshSeconds, 5), ("history", \.historyInterval, 10), ("foreground", \.foregroundInterval, 2), ("background", \.serverRefreshSeconds, 2)]
        for (field, path, minimum) in intervals {
            validation.settingsDraft = Settings()
            validation.settingsDraft[keyPath: path] = minimum - 1
            await validation.saveGeneralConfiguration()
            precondition(validation.settingsValidationField == field, "Validation targets fields by identity, independently of copy")
            precondition(validation.settingsDraft[keyPath: path] == minimum - 1, "Invalid input remains available to edit")
        }
        validation.discardSettingsDraft()
        precondition(validation.settingsValidationField == nil)
        var baseDraft = Settings()
        baseDraft.accounts = [AgentAccount(id: "a", name: "Original", provider: "codex")]
        baseDraft.sources = [AgentSource(id: "s", name: "Directory", provider: "codex", accountId: "a", path: "/fixture")]
        var pending = baseDraft
        pending.modelMappings["pending"] = "vendor/model"
        pending.accounts[0].name = "My edited name"
        pending.proxy.mode = "direct"
        var login = baseDraft
        login.accounts[0].identityKey = "fixture-identity"
        login.accounts.append(AgentAccount(id: "b", name: "New login", provider: "codex"))
        login.sources[0].codexHomeId = "managed"
        let rebased = SettingsDraftMerge.rebase(base: baseDraft, draft: pending, saved: login)
        precondition(rebased.modelMappings["pending"] == "vendor/model" && rebased.proxy.mode == "direct")
        precondition(rebased.accounts.count == 2 && rebased.accounts[0].name == "My edited name")
        precondition(rebased.accounts[0].identityKey == "fixture-identity" && rebased.sources[0].codexHomeId == "managed")
        var deleted = pending; deleted.accounts.removeAll()
        let retainedNew = SettingsDraftMerge.rebase(base: baseDraft, draft: deleted, saved: login)
        precondition(retainedNew.accounts.map(\.id) == ["b"], "An explicit removal and a newly authenticated account must both survive")
        var submitted = baseDraft
        submitted.proxy = ProxySettings(mode: "manual", url: "  http://127.0.0.1:7890  ")
        var normalized = submitted
        normalized.proxy.url = "http://127.0.0.1:7890"
        var stillEditing = submitted
        stillEditing.modelMappings["unsaved"] = "vendor/pending"
        let acknowledged = SettingsDraftMerge.rebase(base: baseDraft, draft: stillEditing, saved: normalized, submitted: submitted)
        precondition(acknowledged.proxy == normalized.proxy && acknowledged.modelMappings["unsaved"] == "vendor/pending", "Save normalization must acknowledge submitted input without erasing an unrelated draft")
        stillEditing.proxy.url = "http://127.0.0.1:7891"
        let laterInput = SettingsDraftMerge.rebase(base: baseDraft, draft: stillEditing, saved: normalized, submitted: submitted)
        precondition(laterInput.proxy.url == stillEditing.proxy.url, "Input made while saving must stay queued")
        var submittedReversal = baseDraft
        submittedReversal.refreshSeconds = baseDraft.refreshSeconds + 1
        submittedReversal.historyRefreshSeconds = 90
        let reverted = SettingsDraftMerge.rebase(base: baseDraft, draft: baseDraft, saved: submittedReversal, submitted: submittedReversal, draftAtSubmission: submittedReversal)
        precondition(reverted == baseDraft, "Returning the entire draft to its pre-save value, including an absent optional field, must remain a new edit")
        let separateEditor = SettingsDraftMerge.rebase(base: baseDraft, draft: baseDraft, saved: submittedReversal, submitted: submittedReversal, draftAtSubmission: baseDraft)
        precondition(separateEditor == submittedReversal, "An unchanged main draft still accepts changes from a separate editor")
        var optionalBase = baseDraft; optionalBase.historyRefreshSeconds = 90
        var cleared = optionalBase; cleared.historyRefreshSeconds = nil
        var canonical = cleared; canonical.historyRefreshSeconds = 300
        let acknowledgedClear = SettingsDraftMerge.rebase(base: optionalBase, draft: cleared, saved: canonical, submitted: cleared, draftAtSubmission: cleared)
        precondition(acknowledgedClear.historyRefreshSeconds == 300, "Core normalization also acknowledges a submitted optional-field removal")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("aieyes-config-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        setenv("AIEYES_DATA_DIR", root.path, 1)
        setenv("AIEYES_CORE_PATH", FileManager.default.currentDirectoryPath + "/target/debug/aieyes-core", 1)
        let model = AppModel(autostart: false), other = EngineClient(configurationOnly: true)
        defer { model.stop(); other.stop() }
        func parameters(_ settings: Settings) throws -> [String: Any] {
            try JSONSerialization.jsonObject(with: JSONEncoder().encode(settings)) as! [String: Any]
        }
        var seed = Settings()
        seed.hosts = [Host(id: "shared", name: "Before", target: "fixture.invalid", enabled: false)]
        seed.accounts = [AgentAccount(id: "notify", name: "Notify", provider: "codex", quotaEnabled: false)]
        let _: Acknowledgement = try await other.call("settings.save", params: parameters(seed))
        let baseline: Settings = try await other.call("settings.get")
        model.settings = baseline; model.settingsDraft = baseline; model.settingsLoaded = true
        var editedHost = baseline.hosts[0]; editedHost.port = 2222
        var concurrent = baseline
        concurrent.hosts[0].name = "Other window"
        concurrent.refreshSeconds = 120
        let external: Settings = try await other.call("settings.patch", params: ["base":parameters(baseline), "settings":parameters(concurrent)])
        // Invalidation updates the model while the editor still holds its opening snapshot.
        model.settings = external; model.settingsDraft = external
        try await model.saveHostConfiguration(editedHost, password: "", base: baseline)
        precondition(model.settings.hosts[0].name == "Other window" && model.settings.hosts[0].port == 2222)
        precondition(model.settings.refreshSeconds == 120)
        let persisted: Settings = try await other.call("settings.get")
        precondition(persisted == model.settings && model.settingsDraft == persisted)

        let conflictBase = persisted
        var localHost = conflictBase.hosts[0]; localHost.port = 3333
        var remote = conflictBase; remote.hosts[0].port = 4444
        let changed: Settings = try await other.call("settings.patch", params: ["base":parameters(conflictBase), "settings":parameters(remote)])
        model.settings = changed; model.settingsDraft = changed
        do {
            try await model.saveHostConfiguration(localHost, password: "", base: conflictBase)
            preconditionFailure("A stale editor must not overwrite the same field changed elsewhere")
        } catch {
            precondition(error.localizedDescription.contains("此设置已在其他窗口修改"), error.localizedDescription)
        }
        let afterConflict: Settings = try await other.call("settings.get")
        precondition(afterConflict == changed && model.settings == changed && model.settingsDraft == changed)
        var automaticSeed = afterConflict
        automaticSeed.sources = [AgentSource(id: "auto-source", name: "Auto", provider: "claude", accountId: "", path: "/fixture", enabled: false)]
        let _: Acknowledgement = try await other.call("settings.save", params: parameters(automaticSeed))
        let automaticBaseline: Settings = try await other.call("settings.get")
        model.settings = automaticBaseline; model.settingsDraft = automaticBaseline
        await model.configureAutomatic(automaticBaseline.sources[0], enabled: true, confirmed: true)
        precondition(model.automaticError == nil && model.settings.sources[0].autoValuation?.enabled == true)
        let enabled: Settings = try await other.call("settings.get")
        precondition(enabled.sources[0].autoValuation?.consentVersion == 1 && model.settingsDraft == enabled)
        await model.configureAutomatic(enabled.sources[0], enabled: false, confirmed: false)
        precondition(model.automaticError == nil && model.settings.sources[0].autoValuation == nil)
        model.settingsDraft.quotaResetNotifications = QuotaResetNotificationSettings(unexpected: false, natural: true)
        await model.saveGeneralConfiguration()
        let notificationSettings: Settings = try await other.call("settings.get")
        precondition(notificationSettings.quotaResetNotifications?.unexpected == false && notificationSettings.quotaResetNotifications?.natural == true)
        precondition(model.settingsDraft == notificationSettings, "Notification switches must settle after autosaving")
        for preferences in [QuotaResetNotificationSettings(unexpected: true, natural: false), QuotaResetNotificationSettings()] {
            var next = model.settings
            next.accounts[0].quotaResetNotifications = preferences
            try await model.persistConfiguration(next)
            let saved: Settings = try await other.call("settings.get")
            precondition(saved.accounts[0].quotaResetNotifications == preferences, "Account notification overrides must round-trip, including return to inheritance")
            precondition(model.settingsDraft == saved)
        }
        print("Native notification defaults: real-core general autosave and draft synchronization passed")
        print("Native account notification overrides: enable, disable and restore inheritance passed")
        print("Native source-level automatic opt-in and disable: real-core persistence and draft synchronization passed")
        print("Native production configuration actions: real-core concurrent fields merged, stale same-field edit rejected, committed settings preserved")
    }
}
