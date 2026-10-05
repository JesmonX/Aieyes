import Foundation

@main struct VerifyMacUI {
    @MainActor static func main() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("aieyes-ui-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        var settings = Settings()
        settings.accounts = [AgentAccount(id: "fixture", name: "Fixture", provider: "deepseek", quotaEnabled: true)]
        settings.sources = [AgentSource(id: "fixture-source", name: "Fixture", provider: "deepseek", accountId: "fixture", path: "", enabled: false)]
        let committedKey = root.appendingPathComponent("committed.key")
        try Data("old-fixture-key".utf8).write(to: committedKey)
        settings.sources[0].path = committedKey.path
        var dashboard = Dashboard()
        dashboard.modelOptions = ["alpha", "beta"]
        let encoder = JSONEncoder()
        try encoder.encode(settings).write(to: root.appendingPathComponent("settings.json"))
        try encoder.encode(dashboard).write(to: root.appendingPathComponent("dashboard.json"))
        let executable = root.appendingPathComponent("core.py")
        let script = #"""
#!/usr/bin/python3
import json,sys,os,hashlib
root=os.environ['AIEYES_UI_FIXTURE']
for line in sys.stdin:
    req=json.loads(line); method=req['method']
    with open(root+'/calls.jsonl','a') as log: log.write(json.dumps(req)+'\n')
    response=dict(jsonrpc='2.0',id=req['id'])
    if method=='settings.get': response['result']=json.load(open(root+'/settings.json'))
    elif method=='dashboard': response['result']=json.load(open(root+'/dashboard.json'))
    elif method=='settings.save' and req['params']['refreshSeconds'] < 5: response['error']=dict(message='刷新间隔无效')
    elif method=='quotas.refresh': response['error']=dict(message='模拟限额连接失败')
    elif method=='prices.save': response['error']=dict(message='模拟价格保存失败')
    elif method=='hosts.sample': response['error']=dict(message='模拟采样连接失败')
    elif method=='credentials.save':
        target=root+'/'+hashlib.sha256(req['params']['sourceId'].encode()).hexdigest()+'.key'
        with open(target,'x') as keyfile: keyfile.write(req['params']['apiKey'])
        response['result']=dict(path=target)
    elif method=='settings.save':
        with open(root+'/settings.json','w') as config: json.dump(req['params'],config)
        response['result']={}
    elif method in ['sources.scan','prices.list','hosts.sample']: response['result']=[]
    else: response['result']={}
    print(json.dumps(response),flush=True)
"""#
        try Data(script.utf8).write(to: executable)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        setenv("AIEYES_CORE_PATH", executable.path, 1)
        setenv("AIEYES_UI_FIXTURE", root.path, 1)
        setenv("AIEYES_DATA_DIR", root.path, 1)
        let model = AppModel(autostart: false)
        defer { model.engine.stop(); model.metricsEngine.stop() }
        await model.bootstrap()
        precondition(model.settings == settings && model.settingsDraft == settings && !model.settingsDirty)
        precondition(model.quotaError == "模拟限额连接失败")
        precondition(model.quotaNextAttempt.map { $0 > Date() } == true)
        func calls(_ method: String) throws -> Int {
            try String(contentsOf: root.appendingPathComponent("calls.jsonl"), encoding: .utf8).split(separator: "\n").filter { line in
                (try? JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any])?["method"] as? String == method
            }.count
        }
        let startupQueries = try calls("quotas.refresh")
        precondition(startupQueries == 1, "Bootstrap must attempt quotas without a manual click")
        model.tick()
        try await Task.sleep(nanoseconds: 80_000_000)
        let queriesAfterTick = try calls("quotas.refresh")
        precondition(queriesAfterTick == 1, "Failed quota reads must respect the retry deadline")
        model.settingsDraft.refreshSeconds = 1
        precondition(model.settingsDirty && model.settings.refreshSeconds == 300)
        let rejected = await model.saveSettingsDraft()
        precondition(!rejected && model.settings.refreshSeconds == 300 && model.settingsDraft.refreshSeconds == 1)
        precondition(model.settingsMessage == "刷新间隔无效" && !model.settingsSaving)
        model.discardSettingsDraft()
        precondition(!model.settingsDirty)
        model.settingsDraft.refreshSeconds = 60
        let accepted = await model.saveSettingsDraft()
        precondition(accepted && !model.settingsDirty && model.settings.refreshSeconds == 60)
        model.settings.accounts[0].quotaEnabled = false
        model.settingsDraft = model.settings
        model.settingsDraft.accounts[0].quotaEnabled = true
        let enabled = await model.saveSettingsDraft()
        precondition(enabled)
        try await Task.sleep(nanoseconds: 80_000_000)
        let queriesAfterEnabling = try calls("quotas.refresh")
        precondition(queriesAfterEnabling == 2, "Enabling quota queries must trigger an immediate first read")
        model.setWindowVisible(true, window: "detail")
        model.setServerVisible(true, window: "detail")
        model.setWindowVisible(true, window: "panel")
        model.setServerVisible(false, window: "panel")
        precondition(model.serverTabVisible, "Agent panel must not disable a visible server detail")
        model.setWindowVisible(false, window: "detail")
        precondition(!model.serverTabVisible)
        model.setServerVisible(true, window: "panel")
        precondition(model.serverTabVisible)
        model.setWindowVisible(false, window: "panel")
        precondition(!model.serverTabVisible && !model.panelVisible)
        model.selectedModel = "alpha"
        await model.reload()
        precondition(model.dashboard.modelOptions == ["alpha", "beta"])
        var old = try JSONSerialization.jsonObject(with: encoder.encode(dashboard)) as! [String: Any]
        old.removeValue(forKey: "modelOptions")
        let decoded = try JSONDecoder().decode(Dashboard.self, from: JSONSerialization.data(withJSONObject: old))
        precondition(decoded.modelOptions == nil, "Older cores must still decode")
        model.stageAPIKey("new-fixture-key", sourceID: "fixture-source")
        precondition(model.settingsDirty)
        let writesBeforeSave = try calls("credentials.save")
        precondition(writesBeforeSave == 0, "Editing a key must only stage it in memory")
        model.discardSettingsDraft()
        precondition(model.pendingAPIKeys.isEmpty && !model.settingsDirty)
        model.stageAPIKey("new-fixture-key", sourceID: "fixture-source")
        model.settingsDraft.refreshSeconds = 1
        let credentialRejected = await model.saveSettingsDraft()
        precondition(!credentialRejected && model.pendingAPIKeys["fixture-source"] == "new-fixture-key")
        precondition(model.settings.sources[0].path == committedKey.path && model.settingsDraft.sources[0].path == committedKey.path)
        let firstCredentialWrites = try calls("credentials.save")
        precondition(firstCredentialWrites == 1)
        let credentialRetry = await model.saveSettingsDraft()
        let writesAfterRetry = try calls("credentials.save")
        precondition(!credentialRetry && writesAfterRetry == 1, "Retry should reuse the prepared immutable credential file")
        model.settingsDraft.refreshSeconds = 60
        let credentialCommitted = await model.saveSettingsDraft()
        precondition(credentialCommitted && model.pendingAPIKeys.isEmpty && !model.settingsDirty)
        precondition(model.settings.sources[0].path != committedKey.path)
        let oldKey = try String(contentsOf: committedKey, encoding: .utf8)
        let newKey = try String(contentsOfFile: model.settings.sources[0].path, encoding: .utf8)
        precondition(oldKey == "old-fixture-key" && newKey == "new-fixture-key")
        let persisted = try JSONDecoder().decode(Settings.self, from: Data(contentsOf: root.appendingPathComponent("settings.json")))
        precondition(persisted.sources[0].path == model.settings.sources[0].path)
        model.settings.hosts = [Host(id: "fixture-host", name: "Fixture", target: "example.invalid")]
        let previousSample = MetricSample(timestamp: Date().timeIntervalSince1970 - 5, uptime: 10, load: [0], errors: [:])
        model.hosts = [HostResult(id: "fixture-host", name: "Fixture", sample: previousSample, error: nil)]
        await model.sampleHosts()
        precondition(model.hosts[0].error == "模拟采样连接失败" && model.hosts[0].sample?.timestamp == previousSample.timestamp)
        let samplesBeforeTick = try calls("hosts.sample")
        model.tick()
        try await Task.sleep(nanoseconds: 80_000_000)
        let samplesAfterTick = try calls("hosts.sample")
        precondition(samplesBeforeTick == 1 && samplesAfterTick == 1, "RPC failures must not retry on every 0.5s tick")
        let priceSaved = await model.savePrice(ModelPrice(id: "fixture/price", name: "Fixture"))
        precondition(!priceSaved && model.message == "模拟价格保存失败")
        print("macOS draft rollback/commit, automatic quota retry, per-window visibility, model-options compatibility, staged credential commit/rollback, sampling failure throttling, and price failure checks passed")
    }
}
