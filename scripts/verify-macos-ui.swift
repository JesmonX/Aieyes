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
        dashboard.generatedAt = 1_790_000_000; dashboard.summary.total = 123_456
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
    elif method=='dashboard':
        if os.path.exists(root+'/hold-dashboard'):
            import time
            open(root+'/dashboard-started','w').close()
            deadline=time.monotonic()+10
            while not os.path.exists(root+'/release-dashboard') and time.monotonic()<deadline: time.sleep(0.01)
        response['result']=json.load(open(root+'/dashboard.json'))
        if req.get('params',{}).get('model'): response['result']['summary']['total']=77
    elif method=='sources.scan':
        rows=json.load(open(root+'/scan-result.json')) if os.path.exists(root+'/scan-result.json') else []
        response['result']=[r for r in rows if not req.get('params',{}).get('sourceId') or r['id']==req['params']['sourceId']]
    elif method=='settings.save' and req['params']['refreshSeconds'] < 5: response['error']=dict(message='刷新间隔无效')
    elif method.startswith('quotaEstimates.'):
        import time
        print(json.dumps(dict(jsonrpc='2.0',method='operations.progress',params=dict(stage='同步来源 · 测试来源'))),flush=True)
        time.sleep(0.15)
        estimate=json.load(open(root+'/estimate.json'))
        dashboard=json.load(open(root+'/dashboard.json'))
        if method=='quotaEstimates.restart':
            estimate['status']='pending'
            response['error']=dict(message='模拟采样同步失败')
        else: response['result']=estimate
        dashboard['quotaEstimates']=[estimate]
        with open(root+'/dashboard.json','w') as stored: json.dump(dashboard,stored)
    elif method=='quotas.refresh': response['error']=dict(message='模拟限额连接失败')
    elif method=='prices.save': response['error']=dict(message='模拟价格保存失败')
    elif method=='hosts.sample': response['error']=dict(message='模拟采样连接失败')
    elif method=='credentials.save':
        target=root+'/'+hashlib.sha256(req['params']['sourceId'].encode()).hexdigest()+'.key'
        with open(target,'x') as keyfile: keyfile.write(req['params']['apiKey'])
        response['result']=dict(path=target)
    elif method=='hosts.credentials.save':
        import time
        reference='ssh-fixture-'+str(time.time_ns())
        with open(root+'/'+reference,'x') as stored: stored.write(req['params']['password'])
        response['result']=dict(passwordRef=reference)
    elif method=='hosts.credentials.delete':
        os.remove(root+'/'+req['params']['passwordRef']); response['result']={}
    elif method=='network.test': response['result']=dict(testedAt=1790000000,mode='system',averageMs=73,status='ok',sites=[])
    elif method=='quotas.order.set': response['result']=req['params']['keys']
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
        defer { model.engine.stop(); model.metricsEngine.stop(); model.networkEngine.stop() }
        await model.bootstrap()
        precondition(model.settings == settings && model.settingsDraft == settings && !model.settingsDirty)
        let legacyWindow = try JSONDecoder().decode(QuotaWindow.self, from: Data(#"{"id":"","name":"Claude · 7d","usedPercent":20,"windowMinutes":10080,"groupName":""}"#.utf8))
        precondition(legacyWindow.id == "Claude · 7d" && legacyWindow.groupLabel == "Claude")
        model.settingsDraft.refreshSeconds = 321
        try await model.saveQuotaOrder(["deepseek:fixture"])
        precondition(model.settingsDraft.refreshSeconds == 321 && model.settings.refreshSeconds == 300 && model.settingsDirty)
        model.discardSettingsDraft()
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
        let dashboardsBeforeSaving = try calls("dashboard")
        let quotaDeadlineBeforeSaving = model.quotaNextAttempt
        let accepted = await model.saveSettingsDraft()
        precondition(accepted && !model.settingsDirty && model.settings.refreshSeconds == 60)
        try await Task.sleep(nanoseconds: 30_000_000)
        let dashboardsAfterSaving = try calls("dashboard")
        precondition(dashboardsAfterSaving == dashboardsBeforeSaving && model.quotaNextAttempt == quotaDeadlineBeforeSaving, "Refresh interval saves must not reload history or reset quota polling")
        let dashboardReadsBeforeReload = try calls("dashboard")
        await model.reload()
        let dashboardReadsAfterReload = try calls("dashboard")
        precondition(dashboardReadsAfterReload == dashboardReadsBeforeReload + 1, "The unfiltered one-day dashboard must be reused for menu data")
        model.settings.accounts[0].quotaEnabled = false
        model.settingsDraft = model.settings
        model.settingsDraft.accounts[0].quotaEnabled = true
        let enabled = await model.saveSettingsDraft()
        precondition(enabled)
        try await Task.sleep(nanoseconds: 80_000_000)
        let queriesAfterEnabling = try calls("quotas.refresh")
        precondition(queriesAfterEnabling == 2, "Enabling quota queries must trigger an immediate first read")
        try Data().write(to: root.appendingPathComponent("hold-dashboard"))
        model.settingsDraft.sources[0].name = "Renamed fixture"
        let saveStarted = Date()
        let backgroundSaved = await model.saveSettingsDraft()
        precondition(backgroundSaved && !model.settingsSaving && Date().timeIntervalSince(saveStarted) < 2, "Save feedback must not await a slow dashboard")
        for _ in 0..<100 {
            if FileManager.default.fileExists(atPath: root.appendingPathComponent("dashboard-started").path) { break }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        precondition(FileManager.default.fileExists(atPath: root.appendingPathComponent("dashboard-started").path), "Source configuration changes must still refresh the dashboard in the background")
        try Data().write(to: root.appendingPathComponent("release-dashboard"))
        await model.reload()
        try FileManager.default.removeItem(at: root.appendingPathComponent("hold-dashboard"))
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
        model.settingsDraft = model.settings
        model.settingsDraft.hosts[0].authMode = "password"
        model.settingsDraft.hosts[0].username = "fixture-user"
        model.stageHostPassword("fixture-password", hostID: "fixture-host")
        let stagedPasswordWrites = try calls("hosts.credentials.save")
        precondition(stagedPasswordWrites == 0)
        model.settingsDraft.refreshSeconds = 1
        let passwordRejected = await model.saveSettingsDraft()
        precondition(!passwordRejected && model.settings.hosts[0].passwordRef == nil && model.settingsDraft.hosts[0].passwordRef == nil)
        let passwordRetry = await model.saveSettingsDraft()
        let passwordWritesAfterRetry = try calls("hosts.credentials.save")
        precondition(!passwordRetry && passwordWritesAfterRetry == 1)
        model.stageHostPassword("replacement-fixture-password", hostID: "fixture-host")
        let replacementRejected = await model.saveSettingsDraft()
        let obsoletePasswordDeletes = try calls("hosts.credentials.delete")
        precondition(!replacementRejected && obsoletePasswordDeletes == 1)
        model.settingsDraft.refreshSeconds = 60
        let passwordCommitted = await model.saveSettingsDraft()
        precondition(passwordCommitted && model.pendingHostPasswords.isEmpty && model.settings.hosts[0].passwordRef != nil)
        let storedHost = try JSONDecoder().decode(Settings.self, from: Data(contentsOf: root.appendingPathComponent("settings.json"))).hosts[0]
        let encodedHost = String(data: try encoder.encode(storedHost), encoding: .utf8)!
        precondition(!encodedHost.contains("replacement-fixture-password"))
        await model.testNetwork()
        precondition(model.networkTest?.label == "系统 73 ms")
        model.settingsDraft.proxy.mode = "direct"
        let proxyCommitted = await model.saveSettingsDraft()
        precondition(proxyCommitted && model.networkTest == nil)
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
        let estimate = QuotaEstimate(id: "sample", accountKey: "codex:fixture", windowId: "seven_day", windowName: "7d", sourceIds: [], sourceNames: [], status: "active", reason: "", startedAt: 1, checkpointAt: 2, endedAt: nil, consumedPercent: 1, cost: 1, totalTokens: 100, pricedTokens: 100, weeklyValue: nil, calculationNote: "测试采样")
        try encoder.encode(estimate).write(to: root.appendingPathComponent("estimate.json"))
        model.setWindowVisible(true, window: "panel")
        let sampling = Task { await model.performEstimate("start", accountKey: estimate.accountKey, params: ["accountKey":estimate.accountKey]) }
        try await Task.sleep(nanoseconds: 30_000_000)
        precondition(model.estimateBusy.contains(estimate.accountKey))
        precondition(model.estimateStages[estimate.accountKey] == "同步来源 · 测试来源")
        let duplicate = await model.performEstimate("start", accountKey: estimate.accountKey, params: [:])
        precondition(!duplicate)
        model.setWindowVisible(false, window: "panel")
        let started = await sampling.value
        precondition(started && model.estimateBusy.isEmpty && !model.isPinned)
        precondition(model.runningEstimates.count == 1 && model.samplingSummary.contains("采样中"))
        model.selectedAccount = "deepseek:fixture"
        precondition(model.runningEstimates.count == 1, "Menu sampling badge must not depend on usage filters")
        let restarting = Task { await model.performEstimate("restart", accountKey: estimate.accountKey, params: ["id": estimate.id]) }
        try await Task.sleep(nanoseconds: 30_000_000)
        precondition(model.estimateStages[estimate.accountKey] == "同步来源 · 测试来源", "Actions addressed by estimate ID must route progress to the record's account")
        let failedRestart = await restarting.value
        precondition(!failedRestart && model.samplingNeedsAttention && model.estimateErrors[estimate.accountKey] == "模拟采样同步失败")
        precondition(!model.panelVisible && model.estimateBusy.isEmpty)
        // Menu data is unfiltered and always today, even for an empty filtered view.
        model.settings.menuMetric = "tokens"; model.selectedModel = "alpha"; model.range = 30
        await model.reload()
        precondition(model.dashboard.summary.total == 77 && model.menuDashboard.summary.total == 123_456)
        precondition(model.menuText == Format.compact(123_456))
        precondition(model.dataTime == dashboard.generatedAt && model.statusText.contains("数据 " + Format.time(dashboard.generatedAt)))
        let priorTime = model.dataTime
        let failures = Data(#"[{"id":"first","error":"first failed"},{"id":"second","error":"second failed"}]"#.utf8)
        try failures.write(to: root.appendingPathComponent("scan-result.json"))
        await model.scan()
        precondition(model.actionFailures.filter { $0.action == "scan" }.count == 2)
        precondition(model.refreshStates["scan"]?.busy == false && model.refreshStates["scan"]?.error != nil)
        precondition(model.dataTime == priorTime, "Failure must not replace the data timestamp with completion time")
        try Data("[]".utf8).write(to: root.appendingPathComponent("scan-result.json"))
        await model.retry(model.actionFailures.first { $0.itemID == "first" }!)
        precondition(model.actionFailures.filter { $0.action == "scan" }.map(\.itemID) == ["second"], "Targeted retry must preserve other failed sources")
        await model.scan()
        precondition(!model.actionFailures.contains { $0.action == "scan" } && model.refreshStates["scan"]?.succeededAt != nil)
        let requests = try String(contentsOf: root.appendingPathComponent("calls.jsonl"), encoding: .utf8).split(separator: "\n").compactMap { try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any] }
        precondition(requests.contains { $0["method"] as? String == "sources.scan" && ($0["params"] as? [String: Any])?["sourceId"] as? String == "first" })
        model.settingsDraft = model.settings
        model.settingsDraft.hosts = [Host(id: "linked-host", name: "Linked", target: "example.invalid")]
        model.settingsDraft.sources[0].hostId = "linked-host"; model.settingsDraft.sources[0].enabled = true
        model.settingsDraft.accounts[0].quotaSourceId = model.settingsDraft.sources[0].id
        let beforeConfirmation = model.settingsDraft
        let removal = model.removalRequest("host", id: "linked-host")!
        precondition(removal.affected == [model.settingsDraft.sources[0].name] && model.settingsDraft == beforeConfirmation, "Preparing or cancelling confirmation must leave the draft untouched")
        model.applyRemoval(removal)
        precondition(model.settingsDraft.hosts.isEmpty && model.settingsDraft.sources[0].hostId == nil && !model.settingsDraft.sources[0].enabled)
        precondition(model.settings.hosts != model.settingsDraft.hosts, "Removal must remain a draft until Save")
        let sourceRemoval = model.removalRequest("source", id: model.settingsDraft.sources[0].id)!
        precondition(sourceRemoval.affected == [model.settingsDraft.accounts[0].name])
        model.applyRemoval(sourceRemoval)
        precondition(model.settingsDraft.sources.isEmpty && model.settingsDraft.accounts[0].quotaSourceId == nil)
        model.applyRemoval(model.removalRequest("account", id: model.settingsDraft.accounts[0].key)!)
        precondition(model.settingsDraft.accounts[0].archived == true)
        model.discardSettingsDraft(); precondition(!model.settingsDirty)
        print("Unfiltered menu metric, data-time semantics, per-source retry, and destructive draft confirmation checks passed")
        print("macOS draft rollback/commit, automatic quota retry, per-window visibility, model-options compatibility, staged credential commit/rollback, sampling failure throttling, and price failure checks passed")
    }
}
