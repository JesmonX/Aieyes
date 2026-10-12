import Foundation

@main struct VerifyConfiguration {
    @MainActor static func main() async throws {
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
        print("Native production configuration actions: real-core concurrent fields merged, stale same-field edit rejected, committed settings preserved")
    }
}
