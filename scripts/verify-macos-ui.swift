import Foundation

@main struct VerifyMacUI {
    struct SemanticsFixture: Decodable {
        struct Panel: Decodable {
            var name: String, accounts: [AgentAccount], order: [String]
            var preference: PanelAccountPreference.Preference?, rawPreference: String?, expected: [String: [String]]
        }
        struct Windows: Decodable { var name: String, provider: String, windows: [QuotaWindow], expected: [String] }
        var panels: [Panel], windows: [Windows]
    }
    @MainActor static func main() async throws {
        let fixture = try JSONDecoder().decode(SemanticsFixture.self, from: Data(contentsOf: URL(fileURLWithPath: "scripts/fixtures/ui-semantics.json")))
        for row in fixture.panels {
            let preference = try row.rawPreference ?? String(data: JSONEncoder().encode(row.preference), encoding: .utf8)!
            precondition(PanelAccountPreference.selections(preference, accounts: row.accounts, order: row.order) == row.expected, row.name)
        }
        for row in fixture.windows {
            let raw: [String: Any] = ["sourceId":"fixture", "accountId":"fixture", "provider":row.provider, "name":"Fixture", "updatedAt":1, "origin":"fixture", "windows":try JSONSerialization.jsonObject(with: JSONEncoder().encode(row.windows))]
            let quota = try JSONDecoder().decode(Quota.self, from: JSONSerialization.data(withJSONObject: raw))
            precondition(EstimatePresentation.windows(quota).map(\.id) == row.expected, row.name)
        }
        let presentation = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: "scripts/fixtures/estimate-presentation.json"))) as! [String: Any]
        let recordBase = presentation["record"] as! [String: Any]
        let fieldNames = ["five": "fiveHourValue", "week": "weeklyValue", "ratio": "weeklyRatioValue"]
        for row in presentation["cases"] as! [[String: Any]] {
            let name = row["name"] as! String
            let group = row["group"] as? String
            let quota: Quota?
            if let raw = row["quota"] as? [String: Any] {
                let base: [String: Any] = ["sourceId": "s", "accountId": "fixture", "name": "Fixture", "updatedAt": 2, "origin": "fixture"]
                quota = try JSONDecoder().decode(Quota.self, from: JSONSerialization.data(withJSONObject: base.merging(raw) { _, next in next }))
            } else { quota = nil }
            let records = try ((row["records"] as? [[String: Any]]) ?? [[:]]).map { raw in
                try JSONDecoder().decode(QuotaEstimate.self, from: JSONSerialization.data(withJSONObject: recordBase.merging(raw) { _, next in next }))
            }
            precondition(EstimatePresentation.summary(records, quota: quota, group: group) == row["summary"] as! String, name)
            let fields = EstimatePresentation.entries(records, quota: quota, group: group).map { fieldNames[$0.id]! }
            precondition(fields == row["fields"] as! [String], name)
            let visible = EstimatePresentation.visibility(quota, records: records, group: group)
            if !visible.five { precondition(!visible.help.contains("5h") && !visible.help.contains("容量倍率"), name) }
            if !visible.direct { precondition(!visible.help.contains("同期") && !visible.help.contains("周额度消耗比例"), name) }
        }
        let panelAccounts = (0..<20).map { AgentAccount(id: "a\($0)", name: "账户 \($0)", provider: "codex", quotaEnabled: true) }
        let order = panelAccounts.reversed().map(\.key)
        let defaults = PanelAccountPreference.selections("{}", accounts: panelAccounts, order: order)
        precondition(defaults["codex"] == Array(order.prefix(5)))
        let manualAll = PanelAccountPreference.encode(["codex": panelAccounts.map(\.key)])
        precondition(PanelAccountPreference.selections(manualAll, accounts: panelAccounts, order: order)["codex"]?.count == 20, "Manual panel selection is unlimited")
        let empty = PanelAccountPreference.selections("{\"codex\":[]}", accounts: panelAccounts, order: order)
        precondition(empty["codex"]?.isEmpty == true)
        let malformed = PanelAccountPreference.selections("invalid", accounts: panelAccounts, order: order)
        precondition(malformed["codex"]?.count == 5)
        let chosen = PanelAccountPreference.selections(PanelAccountPreference.encode(["codex": ["codex:a19", "codex:a19", "codex:missing", "codex:a0"]]), accounts: panelAccounts, order: order)
        precondition(chosen["codex"] == ["codex:a19", "codex:a0"])
        let archivedAccounts = panelAccounts.map { var account = $0; account.archived = true; return account }
        precondition(PanelAccountPreference.selections(PanelAccountPreference.encode(chosen), accounts: archivedAccounts, order: order)["codex"]?.isEmpty == true)
        let renamed = [AgentAccount(id: "replacement", name: "替换账户", provider: "agy", quotaEnabled: true)]
        let stale = PanelAccountPreference.encode(["agy": ["agy:removed"]])
        precondition(PanelAccountPreference.selections(stale, accounts: renamed, order: [])["agy"] == [], "An unavailable manual choice must not fall back to another account")
        precondition(PanelAccountPreference.selections(PanelAccountPreference.encode(["agy": []]), accounts: renamed, order: [])["agy"] == [])
        precondition(PanelAccountPreference.selections(PanelAccountPreference.encode(["agy": []], automatic: ["agy"]), accounts: renamed, order: [])["agy"] == ["agy:replacement"])
        let preferenceSuite = "aieyes-antigravity-" + UUID().uuidString
        let preferences = UserDefaults(suiteName: preferenceSuite)!
        defer { preferences.removePersistentDomain(forName: preferenceSuite) }
        preferences.set(PanelAccountPreference.encode(["agy": ["agy:shared"], "antigravity": ["antigravity:shared"]]), forKey: "panel.accounts.v2")
        preferences.set(false, forKey: "quota.expanded.v2.panel.agy:shared")
        let aliases = ["agy:shared": "antigravity:legacy-shared"]
        PanelAccountPreference.migrateAntigravity(aliases, defaults: preferences)
        let migrated = PanelAccountPreference.decode(preferences.string(forKey: "panel.accounts.v2")!)
        precondition(migrated.providers["agy"] == nil)
        precondition(migrated.providers["antigravity"]?.keys == ["antigravity:shared", "antigravity:legacy-shared"])
        precondition(preferences.object(forKey: "quota.expanded.v2.panel.antigravity:legacy-shared") as? Bool == false)
        let savedPreference = preferences.string(forKey: "panel.accounts.v2")
        PanelAccountPreference.migrateAntigravity(aliases, defaults: preferences)
        precondition(preferences.string(forKey: "panel.accounts.v2") == savedPreference)
        let legacyFilters = DashboardFilterPreference(provider: "codex", source: "old-source", account: "codex:old", model: "old-model", days: 30)
        legacyFilters.save(defaults: preferences)
        for scope in ["panel", "overview", "analysis"] { precondition(DashboardFilterPreference.read(scope: scope, defaults: preferences) == legacyFilters) }
        let panelFilters = DashboardFilterPreference(provider: "claude", days: 1)
        panelFilters.save(scope: "panel", defaults: preferences)
        precondition(DashboardFilterPreference.read(scope: "panel", defaults: preferences) == panelFilters)
        precondition(DashboardFilterPreference.read(scope: "overview", defaults: preferences) == legacyFilters)
        precondition(DashboardFilterPreference.read(scope: "analysis", defaults: preferences) == legacyFilters)
        precondition(DashboardFilterPreference.read(defaults: preferences) == legacyFilters, "Migration preserves legacy preferences")
        precondition(Format.subscription("ultra") == "Ultra")
        let legacyQuota = try JSONDecoder().decode(Quota.self, from: Data(#"{"sourceId":"s","accountId":"a","provider":"antigravity","name":"Custom","updatedAt":1,"origin":"live","windows":[]}"#.utf8))
        precondition(legacyQuota.identity == nil && legacyQuota.metadataError == nil)
        let identified = try JSONDecoder().decode(AccountDeviceStatus.self, from: Data(#"{"accountKey":"antigravity:a","sourceId":"s","machineId":"local","authenticated":true,"identityConfirmed":true,"current":true,"credential":true,"identity":{"key":"google:fixture","email":"account@example.test","subscription":"Google AI Pro","stale":false}}"#.utf8))
        precondition(identified.identitySummary == "正在使用 account@example.test · Google AI Pro")
        var mismatch = identified; mismatch.current = false
        precondition(mismatch.identitySummary?.contains("身份不一致") == true)

        let root = FileManager.default.temporaryDirectory.appendingPathComponent("aieyes-ui-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        var settings = Settings()
        settings.accounts = [AgentAccount(id: "fixture", name: "Fixture", provider: "deepseek", quotaEnabled: true)]
        settings.sources = [AgentSource(id: "fixture-source", name: "Fixture", provider: "deepseek", accountId: "fixture", path: "", enabled: false)]
        settings.sources[0].path = root.appendingPathComponent("fixture.key").path
        var dashboard = Dashboard()
        dashboard.modelOptions = ["alpha", "beta"]
        dashboard.generatedAt = 1_790_000_000; dashboard.summary.total = 123_456
        let encoder = JSONEncoder()
        try encoder.encode(settings).write(to: root.appendingPathComponent("settings.json"))
        try encoder.encode(dashboard).write(to: root.appendingPathComponent("dashboard.json"))
        let executable = root.appendingPathComponent("core.py")
        let script = #"""
#!/usr/bin/python3
import json,sys,os
root=os.environ['AIEYES_UI_FIXTURE']
for line in sys.stdin:
    req=json.loads(line); method=req['method']
    with open(root+'/calls.jsonl','a') as log: log.write(json.dumps(req)+'\n')
    response=dict(jsonrpc='2.0',id=req['id'])
    if method=='settings.get': response['result']=json.load(open(root+'/settings.json'))
    elif method in ('dashboard','dashboard.summary'):
        if os.path.exists(root+'/fail-panel-query') and (method=='dashboard.summary' or req.get('params',{}).get('days') == 1):
            response['error']=dict(message='模拟今日面板查询失败')
            print(json.dumps(response),flush=True)
            continue
        if os.path.exists(root+'/hold-dashboard'):
            import time
            open(root+'/dashboard-started','w').close()
            deadline=time.monotonic()+10
            while not os.path.exists(root+'/release-dashboard') and time.monotonic()<deadline: time.sleep(0.01)
        response['result']=json.load(open(root+'/dashboard.json'))
        if req.get('params',{}).get('model'): response['result']['summary']['total']=77
    elif method in ('sources.scan','sources.rescan'):
        rows=json.load(open(root+'/scan-result.json')) if os.path.exists(root+'/scan-result.json') else []
        response['result']=[r for r in rows if not req.get('params',{}).get('sourceId') or r['id']==req['params']['sourceId']]
    elif method=='settings.patch' and os.path.exists(root+'/fail-configuration'): response['error']=dict(message='模拟配置保存失败')
    elif method=='settings.patch' and os.path.exists(root+'/fail-theme'): response['error']=dict(message='模拟主题保存失败')
    elif method=='settings.patch' and req['params']['settings']['refreshSeconds'] < 5: response['error']=dict(message='刷新间隔无效')
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
    elif method=='accounts.status.get':
        import time
        open(root+'/account-status-started','w').close()
        deadline=time.monotonic()+10
        while os.path.exists(root+'/hold-account-status') and not os.path.exists(root+'/release-account-status') and time.monotonic()<deadline: time.sleep(.01)
        response['result']=json.load(open(root+'/account-status.json')) if os.path.exists(root+'/account-status.json') else []
    elif method=='accounts.status.refresh': response['error']=dict(message='不应再次检查已确认的登录')
    elif method=='accounts.deployments.sync':
        if os.path.exists(root+'/hold-deployment-sync'):
            import time
            open(root+'/deployment-sync-started','w').close()
            deadline=time.monotonic()+10
            while not os.path.exists(root+'/release-deployment-sync') and time.monotonic()<deadline: time.sleep(.01)
        response['result']=dict(failedTasks=[])
    elif method=='prices.save': response['error']=dict(message='模拟价格保存失败')
    elif method=='prices.recalculate' and os.path.exists(root+'/fail-reprice'): response['error']=dict(message='模拟重算失败')
    elif method=='hosts.sample': response['error']=dict(message='模拟采样连接失败')
    elif method=='hosts.credentials.save':
        import time
        reference='ssh-fixture-'+str(time.time_ns())
        with open(root+'/'+reference,'x') as stored: stored.write(req['params']['password'])
        response['result']=dict(passwordRef=reference)
    elif method=='hosts.credentials.delete':
        os.remove(root+'/'+req['params']['passwordRef']); response['result']={}
    elif method=='network.test': response['result']=dict(testedAt=1790000000,mode='system',averageMs=73,status='ok',sites=[])
    elif method=='quotas.order.set': response['result']=req['params']['keys']
    elif method=='settings.patch':
        if os.path.exists(root+'/hold-save'):
            import time
            open(root+'/save-started','w').close()
            deadline=time.monotonic()+10
            while not os.path.exists(root+'/release-save') and time.monotonic()<deadline: time.sleep(.01)
        previous=json.load(open(root+'/settings.json'))
        req['params']['settings']['proxy']['url']=req['params']['settings']['proxy']['url'].strip()
        retained={h['id'] for h in req['params']['settings'].get('hosts',[])}
        removed={h['id']:h for h in previous.get('hosts',[]) if h['id'] not in retained}
        for source in req['params']['settings'].get('sources',[]):
            if source.get('hostId') in removed:
                source.update(machineBinding='unbound',enabled=False,lastMachineName=removed[source['hostId']]['name'])
        with open(root+'/settings.json','w') as config: json.dump(req['params']['settings'],config)
        response['result']=req['params']['settings']
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
        var settingsOpens = 0
        model.showSettings = { settingsOpens += 1 }
        model.settingsAgent = "claude"; model.settingsTab = "prices"
        model.openSettings()
        precondition(model.settingsTab == "integrations" && model.settingsAgent == "claude")
        model.openSettings(.init(category: "sources", provider: "codex", section: "machines"))
        precondition(model.settingsTab == "integrations" && model.settingsAgent == "codex" && model.settingsSection == "machines")
        model.openSettings(.init(category: "hosts", hostID: "local"))
        precondition(model.settingsTab == "servers" && model.requestedHostID == "local" && !model.requestHostEditor)
        model.openSettings(.init(category: "servers", section: "add-host"))
        precondition(model.requestHostEditor && model.requestedHostID == nil)
        model.openSettings(.init(category: "general", section: "network"))
        precondition(model.generalPage == "network")
        model.openPricing()
        precondition(model.settingsTab == "prices" && settingsOpens == 6)
        model.openSettings()
        let previousProvider = model.provider
        model.provider = "claude"
        var managedQuota: Quota?
        model.showEstimate = { quota, _, _ in managedQuota = quota }
        await model.openEstimate(accountKey: "codex:missing-quota")
        precondition(managedQuota?.id == "codex:missing-quota" && managedQuota?.windows.isEmpty == true, "Sampling management remains available without a current quota snapshot")
        precondition(model.provider == "claude" && model.dashboard.summary.total == dashboard.summary.total, "Sampling management must preserve the overview filter and usage snapshot")
        model.provider = previousProvider; model.showEstimate = nil
        let legacyWindow = try JSONDecoder().decode(QuotaWindow.self, from: Data(#"{"id":"","name":"Claude · 7d","usedPercent":20,"windowMinutes":10080,"groupName":""}"#.utf8))
        precondition(legacyWindow.id == "Claude · 7d" && legacyWindow.groupLabel == "Claude")
        model.settingsDraft.refreshSeconds = 321
        try await model.saveQuotaOrder(["deepseek:fixture"])
        precondition(model.settingsDraft.refreshSeconds == 321 && model.settings.refreshSeconds == 300 && model.settingsDirty)
        model.discardSettingsDraft()
        precondition(model.quotaError == "模拟限额连接失败")
        precondition(model.quotaNextAttempt == nil, "Autostart-disabled models do not schedule background polls")
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
        let startupRescans = try calls("sources.rescan")
        precondition(startupRescans == 0, "Startup and scheduled refresh must not trigger full history replay")
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
        precondition(model.message != "刷新间隔无效", "Successful retry clears the previous save error")
        try await Task.sleep(nanoseconds: 30_000_000)
        let dashboardsAfterSaving = try calls("dashboard")
        precondition(dashboardsAfterSaving == dashboardsBeforeSaving && model.quotaNextAttempt == quotaDeadlineBeforeSaving, "Refresh interval saves must not reload history or reset quota polling")
        // Continuous General editing queues a later value and keeps unrelated drafts.
        model.settingsDraft.refreshSeconds = 321
        model.mappingModel = "unfinished mapping"
        try Data().write(to: root.appendingPathComponent("hold-save"))
        let savingGeneral = Task { await model.saveGeneralConfiguration() }
        while !FileManager.default.fileExists(atPath: root.appendingPathComponent("save-started").path) { try await Task.sleep(for: .milliseconds(10)) }
        precondition(model.settingsSaving)
        model.settingsDraft.refreshSeconds = 432
        model.settingsDraft.serverRefreshSeconds = 19
        let queuedGeneral = Task { await model.saveGeneralConfiguration() }
        try Data().write(to: root.appendingPathComponent("release-save"))
        await savingGeneral.value; await queuedGeneral.value
        precondition(model.settings.refreshSeconds == 432 && model.settings.serverRefreshSeconds == 19)
        precondition(model.settingsDraft.refreshSeconds == 432 && model.mappingModel == "unfinished mapping" && !model.settingsSaving)
        precondition(model.message != "已保存", "General saves use inline feedback")
        try FileManager.default.removeItem(at: root.appendingPathComponent("hold-save"))
        model.mappingModel = ""
        model.settingsDraft.refreshSeconds = 60; model.settingsDraft.serverRefreshSeconds = settings.serverRefreshSeconds
        await model.saveGeneralConfiguration()
        let beforeReverting = model.settings.refreshSeconds
        for flag in ["release-save", "save-started"] { try? FileManager.default.removeItem(at: root.appendingPathComponent(flag)) }
        try Data().write(to: root.appendingPathComponent("hold-save"))
        model.settingsDraft.refreshSeconds = beforeReverting + 1
        let saveBeforeReverting = Task { await model.saveGeneralConfiguration() }
        while !FileManager.default.fileExists(atPath: root.appendingPathComponent("save-started").path) { try await Task.sleep(for: .milliseconds(10)) }
        model.settingsDraft.refreshSeconds = beforeReverting
        let queuedReversal = Task { await model.saveGeneralConfiguration() }
        try Data().write(to: root.appendingPathComponent("release-save"))
        await saveBeforeReverting.value; await queuedReversal.value
        precondition(model.settings.refreshSeconds == beforeReverting && model.settingsDraft.refreshSeconds == beforeReverting, "Editing A to B and back to A during the save must persist the final A")
        try FileManager.default.removeItem(at: root.appendingPathComponent("hold-save"))
        model.settingsDraft.proxy.url = "  http://127.0.0.1:7890  "
        model.mappingModel = "mapping kept through normalization"
        await model.saveGeneralConfiguration()
        precondition(model.settingsDraft.proxy.url == "http://127.0.0.1:7890" && model.mappingModel == "mapping kept through normalization", "Normalized saves must complete without requeuing the submitted text")
        model.settingsDraft.proxy = settings.proxy; model.mappingModel = ""
        await model.saveGeneralConfiguration()
        await model.deploymentSyncTask?.value
        let synchronizationsBefore = try calls("accounts.deployments.sync")
        try Data().write(to: root.appendingPathComponent("hold-deployment-sync"))
        model.synchronizeAccountDeployments()
        let syncDeadline = ProcessInfo.processInfo.systemUptime + 5
        while !FileManager.default.fileExists(atPath: root.appendingPathComponent("deployment-sync-started").path) && ProcessInfo.processInfo.systemUptime < syncDeadline { try await Task.sleep(for: .milliseconds(10)) }
        precondition(FileManager.default.fileExists(atPath: root.appendingPathComponent("deployment-sync-started").path))
        model.settingsDraft.serverRefreshSeconds = 18
        await model.saveGeneralConfiguration()
        precondition(model.settings.serverRefreshSeconds == 18 && !model.settingsSaving && model.deploymentSyncTask != nil, "Saving must complete while deployment synchronization is still waiting for the remote machine")
        try Data().write(to: root.appendingPathComponent("release-deployment-sync"))
        await model.deploymentSyncTask?.value
        let synchronizationsAfter = try calls("accounts.deployments.sync")
        precondition(synchronizationsAfter == synchronizationsBefore + 2, "A configuration change during synchronization must schedule one more pass")
        try FileManager.default.removeItem(at: root.appendingPathComponent("hold-deployment-sync"))
        model.settingsDraft.serverRefreshSeconds = settings.serverRefreshSeconds
        await model.saveGeneralConfiguration()
        let dashboardReadsBeforeReload = try calls("dashboard")
        await model.reload()
        let dashboardReadsAfterReload = try calls("dashboard")
        precondition(dashboardReadsAfterReload == dashboardReadsBeforeReload + 1, "The unfiltered one-day dashboard must be reused for menu data")
        let queriesBeforeEnabling = try calls("quotas.refresh")
        model.settings.accounts[0].quotaEnabled = false
        model.settingsDraft = model.settings
        model.settingsDraft.accounts[0].quotaEnabled = true
        let enabled = await model.saveSettingsDraft()
        precondition(enabled)
        try await Task.sleep(nanoseconds: 80_000_000)
        let queriesAfterEnabling = try calls("quotas.refresh")
        precondition(queriesAfterEnabling == queriesBeforeEnabling + 1, "Enabling quota queries must trigger an immediate first read")
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
        // Exercise the same transaction used by HostEditor, including credential rollback.
        let beforeHostEdit = model.settings
        var host = Host(id: "fixture-host", name: "Fixture", target: "example.invalid")
        host.authMode = "password"; host.username = "fixture-user"
        let failureMarker = root.appendingPathComponent("fail-configuration")
        try Data().write(to: failureMarker)
        for password in ["fixture-password", "replacement-fixture-password"] {
            do {
                try await model.saveHostConfiguration(host, password: password, base: beforeHostEdit)
                preconditionFailure("A rejected configuration must fail the editor save")
            } catch { precondition(error.localizedDescription.contains("模拟配置保存失败")) }
            precondition(model.settings == beforeHostEdit && model.settingsDraft == beforeHostEdit)
            let leftovers = try FileManager.default.contentsOfDirectory(atPath: root.path).filter { $0.hasPrefix("ssh-fixture-") }
            precondition(leftovers.isEmpty, "A failed save must not leave a credential behind")
        }
        let failedSaves = try calls("hosts.credentials.save"), failedDeletes = try calls("hosts.credentials.delete")
        precondition(failedSaves == 2 && failedDeletes == 2)
        try FileManager.default.removeItem(at: failureMarker)
        try await model.saveHostConfiguration(host, password: "replacement-fixture-password", base: beforeHostEdit)
        let storedHost = try JSONDecoder().decode(Settings.self, from: Data(contentsOf: root.appendingPathComponent("settings.json"))).hosts[0]
        precondition(storedHost.passwordRef != nil && storedHost.authMode == "password")
        let encodedHost = String(data: try encoder.encode(storedHost), encoding: .utf8)!
        precondition(!encodedHost.contains("replacement-fixture-password"))
        let storedPassword = try String(contentsOf: root.appendingPathComponent(storedHost.passwordRef!), encoding: .utf8)
        precondition(storedPassword == "replacement-fixture-password")
        host = storedHost; host.authMode = "ssh"
        try await model.saveHostConfiguration(host, password: "ignored password input", base: model.settings)
        let savesAfterSSH = try calls("hosts.credentials.save")
        precondition(savesAfterSSH == 3 && model.settings.hosts[0].passwordRef == storedHost.passwordRef, "Switching to key login ignores the hidden password field")
        await model.testNetwork()
        precondition(model.networkTest?.label == "系统 73 ms")
        model.settingsDraft.proxy.mode = "direct"
        let proxyCommitted = await model.saveSettingsDraft()
        precondition(proxyCommitted && model.networkTest == nil)
        let previousSample = MetricSample(timestamp: Date().timeIntervalSince1970 - 5, uptime: 10, load: [0], errors: [:])
        model.hosts = [HostResult(id: "fixture-host", name: "Fixture", sample: previousSample, error: nil)]
        await model.sampleHosts()
        let failedHost = model.hosts.first { $0.id == "fixture-host" }
        precondition(failedHost?.error == "模拟采样连接失败" && failedHost?.sample?.timestamp == previousSample.timestamp)
        let samplesBeforeTick = try calls("hosts.sample")
        model.tick()
        try await Task.sleep(nanoseconds: 80_000_000)
        let samplesAfterTick = try calls("hosts.sample")
        precondition(samplesBeforeTick >= 1 && samplesAfterTick == samplesBeforeTick, "RPC failures must not retry on every 0.5s tick: \(samplesBeforeTick) -> \(samplesAfterTick)")
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
        model.settings.menuMetric = "tokens"; model.settingsDraft.menuMetric = "tokens"; model.selectedModel = "alpha"; model.range = 30
        await model.reload()
        precondition(model.dashboard.summary.total == 77 && model.menuDashboard.summary.total == 123_456)
        precondition(model.menuText == Format.compact(123_456))
        precondition(model.dataTime == 0 && !model.statusText.contains(Format.time(dashboard.generatedAt)), "A query timestamp must not be presented as a record sync")
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
        try failures.write(to: root.appendingPathComponent("scan-result.json"))
        await model.rescan()
        precondition(model.rescanFailed && model.rescanMessage?.contains("first failed") == true && !model.rescanning)
        precondition(model.actionFailures.filter { $0.action == "rescan" }.count == 2)
        try Data("[]".utf8).write(to: root.appendingPathComponent("scan-result.json"))
        await model.retry(model.actionFailures.first { $0.action == "rescan" && $0.itemID == "first" }!)
        precondition(model.actionFailures.filter { $0.action == "rescan" }.map(\.itemID) == ["second"])
        await model.rescan()
        precondition(!model.rescanFailed && model.rescanMessage == "没有可重扫的已启用数据源。" && !model.rescanning)
        precondition(!model.actionFailures.contains { $0.action == "rescan" })
        let requests = try String(contentsOf: root.appendingPathComponent("calls.jsonl"), encoding: .utf8).split(separator: "\n").compactMap { try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any] }
        precondition(requests.contains { $0["method"] as? String == "sources.scan" && ($0["params"] as? [String: Any])?["sourceId"] as? String == "first" })
        precondition(requests.contains { $0["method"] as? String == "sources.rescan" && ($0["params"] as? [String: Any])?["sourceId"] as? String == "first" }, "Manual retry must also replay the failed source in full")
        let originalConfiguration = model.settings
        var linkedConfiguration = model.settings
        linkedConfiguration.hosts = [Host(id: "linked-host", name: "Linked", target: "example.invalid")]
        linkedConfiguration.sources[0].hostId = "linked-host"; linkedConfiguration.sources[0].enabled = true
        linkedConfiguration.accounts[0].quotaSourceId = linkedConfiguration.sources[0].id
        linkedConfiguration.agents = [AgentConfiguration(provider: "deepseek", enabled: true, machineIds: ["local", "linked-host"])]
        try await model.persistConfiguration(linkedConfiguration)
        let removal = model.removalRequest("host", id: "linked-host")!
        precondition(removal.affected == [model.settings.sources[0].name] && model.settings == linkedConfiguration && model.settingsDraft == linkedConfiguration, "Preparing or cancelling confirmation must not mutate configuration")
        try Data().write(to: failureMarker)
        await model.removeConfiguration(removal)
        precondition(model.settings == linkedConfiguration && model.settingsDraft == linkedConfiguration, "Failed removal must preserve the committed host and its connections")
        try FileManager.default.removeItem(at: failureMarker)
        await model.removeConfiguration(removal)
        precondition(model.settings.hosts.isEmpty && model.settings.sources[0].hostId == "linked-host" && model.settings.sources[0].unbound && !model.settings.sources[0].enabled)
        precondition(model.settings.agents?.first?.machineIds == ["local"])
        let persistedRemoval = try JSONDecoder().decode(Settings.self, from: Data(contentsOf: root.appendingPathComponent("settings.json")))
        precondition(persistedRemoval == model.settings && model.settingsDraft == model.settings, "Confirmed removal persists immediately")
        try await model.persistConfiguration(originalConfiguration)
        precondition(!model.settingsDirty)
        // A secondary dashboard failure must preserve all previous applied snapshots.
        let previousTotal = model.dashboard.summary.total, previousPanel = model.panelDashboard.summary.total, previousScope = model.appliedScope
        model.selectedModel = "all"; model.range = 90
        try Data().write(to: root.appendingPathComponent("fail-panel-query"))
        let failedPanel = await model.reload()
        precondition(!failedPanel && !model.dashboardPending && model.dashboardError == "模拟今日面板查询失败")
        precondition(model.dashboard.summary.total == previousTotal && model.panelDashboard.summary.total == previousPanel && model.appliedScope == previousScope)
        try FileManager.default.removeItem(at: root.appendingPathComponent("fail-panel-query"))
        await model.reload()
        precondition(model.dashboard.summary.total == 123_456 && model.panelDashboard.summary.total == 123_456 && model.dashboardError == nil)
        // Pending mapping input participates in save/reprice and survives failure.
        model.mappingModel = "half"
        let savesBeforeHalf = try calls("settings.patch")
        await model.saveAndReprice()
        let savesAfterHalf = try calls("settings.patch")
        precondition(savesBeforeHalf == savesAfterHalf && model.mappingModel == "half" && model.settingsDirty)
        model.mappingModel = "audit-alias"; model.mappingID = "provider/model"
        try Data().write(to: root.appendingPathComponent("fail-reprice"))
        await model.saveAndReprice()
        precondition(model.settings.modelMappings["audit-alias"] == "provider/model" && model.mappingModel == "audit-alias" && model.mappingID == "provider/model")
        precondition(model.settingsMessage?.contains("重算失败") == true)
        try FileManager.default.removeItem(at: root.appendingPathComponent("fail-reprice"))
        await model.saveAndReprice()
        precondition(model.mappingModel.isEmpty && model.mappingID.isEmpty && !model.settingsDirty)
        model.settingsDraft.refreshSeconds = 321
        await model.toggleTheme(currentlyDark: false)
        precondition(model.settings.appearance?.theme == "dark" && model.settingsDraft.appearance?.theme == "dark")
        precondition(model.settingsDraft.refreshSeconds == 321 && model.settings.refreshSeconds != 321)
        let savedTheme = model.settings.appearance
        try Data().write(to: root.appendingPathComponent("fail-theme"))
        await model.toggleTheme(currentlyDark: true)
        precondition(model.settings.appearance == savedTheme && model.settingsDraft.appearance == savedTheme && !model.settingsSaving)
        try FileManager.default.removeItem(at: root.appendingPathComponent("fail-theme"))
        await model.toggleTheme(currentlyDark: true)
        precondition(model.settings.appearance?.theme == "light" && model.settingsDraft.refreshSeconds == 321)
        for flag in ["release-save", "save-started"] { try? FileManager.default.removeItem(at: root.appendingPathComponent(flag)) }
        try Data().write(to: root.appendingPathComponent("hold-save"))
        let delayedTheme = Task { await model.toggleTheme(currentlyDark: false) }
        while !FileManager.default.fileExists(atPath: root.appendingPathComponent("save-started").path) { try await Task.sleep(for: .milliseconds(10)) }
        var newerAppearance = model.settingsDraft.appearance ?? AppearanceSettings(); newerAppearance.theme = "system"
        model.settingsDraft.appearance = newerAppearance
        try Data().write(to: root.appendingPathComponent("release-save"))
        await delayedTheme.value
        precondition(model.settings.appearance?.theme == "dark" && model.settingsDraft.appearance?.theme == "system" && model.settingsDraft.refreshSeconds == 321, "Quick theme save must preserve a newer theme edit and unrelated drafts")
        try FileManager.default.removeItem(at: root.appendingPathComponent("hold-save"))
        let agyModel = AppModel(autostart: false)
        defer { agyModel.stop() }
        var agySettings = Settings()
        agySettings.accounts = [AgentAccount(id: "a", name: "Custom", provider: "antigravity", quotaEnabled: true)]
        agySettings.sources = [AgentSource(id: "s", name: "Local", provider: "antigravity", accountId: "a", path: "fixture", enabled: true)]
        agyModel.settings = agySettings; agyModel.settingsDraft = agySettings
        agyModel.settingsDraft.refreshSeconds = 321
        var bound = agySettings; bound.accounts[0].identityKey = identified.identity?.key
        try encoder.encode(bound).write(to: root.appendingPathComponent("settings.json"))
        let staleAccountStatus = #"[{"accountKey":"antigravity:a","sourceId":"s","machineId":"local","current":null,"credential":null}]"#
        try Data(staleAccountStatus.utf8).write(to: root.appendingPathComponent("account-status.json"))
        try Data().write(to: root.appendingPathComponent("hold-account-status"))
        try? FileManager.default.removeItem(at: root.appendingPathComponent("account-status-started"))
        let oldRead = Task { await agyModel.loadAccountStatuses() }
        while !FileManager.default.fileExists(atPath: root.appendingPathComponent("account-status-started").path) { try await Task.sleep(for: .milliseconds(10)) }
        let acceptedAt = Date()
        try await agyModel.refreshAntigravityAccount(sourceID: "s", status: identified)
        precondition(Date().timeIntervalSince(acceptedAt) < 0.5, "Login acceptance must not wait for quota queries")
        precondition(agyModel.accountStatuses[0].identitySummary == identified.identitySummary)
        try Data().write(to: root.appendingPathComponent("release-account-status"))
        await oldRead.value
        precondition(agyModel.accountStatuses[0].current == true, "An older pending read must not replace the login")
        if let followUp = agyModel.antigravityRefreshTasks["s"] { await followUp.value }
        precondition(agyModel.antigravityRefreshTasks.isEmpty)
        precondition(agyModel.quotaError == "模拟限额连接失败")
        precondition(agyModel.accountStatuses[0].identity?.email == "account@example.test")
        precondition(agyModel.settings.accounts[0].identityKey == "google:fixture" && agyModel.settings.accounts[0].name == "Custom")
        precondition(agyModel.settingsDraft.refreshSeconds == 321, "Identity refresh must preserve unsaved input")
        let login = try JSONDecoder().decode(AntigravityLoginState.self, from: Data(#"{"id":"session","phase":"authenticated","message":"登录有效","authenticated":true,"identityConfirmed":true,"workspaceConfirmationRequired":false,"accountStatus":{"accountKey":"antigravity:a","sourceId":"s","machineId":"local","current":true,"credential":true}}"#.utf8))
        precondition(login.accountStatus?.current == true && login.workspaceConfirmationRequired == false)
        var historySettings = Settings()
        let historical = AgentAccount(id: "history", name: "旧账户历史", provider: "codex", quotaEnabled: false)
        historySettings.deletedAccounts = [historical]
        precondition(historySettings.filterAccounts.isEmpty)
        historySettings.historyShowDeletedAccounts = true
        precondition(historySettings.filterAccounts.map(\.key) == ["codex:history"])
        precondition(historySettings.accountFilterName(historical) == "旧账户历史（已删除）")
        model.settings = historySettings; model.settingsDraft = historySettings
        model.provider = "all"; model.selectedAccount = "codex:history"; model.selectedSource = "all"; model.selectedModel = "all"
        await model.reload()
        let lastHistoryCall = try String(contentsOf: root.appendingPathComponent("calls.jsonl"), encoding: .utf8).split(separator: "\n").compactMap {
            try JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any]
        }.last { $0["method"] as? String == "dashboard" && ($0["params"] as? [String: Any])?["accountId"] as? String == "history" }
        let historyParams = lastHistoryCall?["params"] as? [String: Any]
        precondition(historyParams?["accountId"] as? String == "history" && historyParams?["provider"] as? String == "codex")
        precondition(model.appliedScope.contains("旧账户历史（已删除）"))
        model.settingsDraft.historyShowDeletedAccounts = false
        let historyHidden = await model.saveSettingsDraft()
        precondition(historyHidden && model.selectedAccount == "all" && model.settings.filterAccounts.isEmpty)
        let workspace = AppModel(autostart: false)
        defer { workspace.stop() }
        workspace.settings = settings; workspace.settingsDraft = settings
        workspace.range = 30; workspace.dateRange = DashboardDateRange(kind: "custom", start: "2026-09-01", end: "2026-09-30")
        let originalAnalysis = workspace.analysisFilters
        workspace.updateFilters(DashboardFilterPreference(provider: "deepseek", account: "deepseek:fixture"), scope: "panel", resetDependents: false)
        workspace.updateFilters(DashboardFilterPreference(provider: "claude", days: 7), scope: "overview")
        precondition(workspace.analysisFilters == originalAnalysis && workspace.panelFilters.account == "deepseek:fixture", "Ordinary filters stay local to their surface")
        var destination: String?
        workspace.showDetailPage = { destination = $0 }
        workspace.openAnalysis(from: "panel")
        precondition(destination == "analysis" && workspace.analysisFilters.account == "deepseek:fixture" && workspace.analysisFilters.provider == "deepseek")
        precondition(workspace.analysisFilters.days == 30 && workspace.analysisFilters.dateRange == originalAnalysis.dateRange, "An explicit analysis jump carries identity and preserves analysis dates")
        precondition(workspace.overviewFilters.provider == "claude" && workspace.overviewFilters.days == 7)
        workspace.mergeConfiguration(Settings())
        precondition(workspace.panelFilters.account == "all" && workspace.analysisFilters.account == "all", "Removed accounts are reconciled in every scope")
        precondition(workspace.overviewFilters.days == 7 && workspace.analysisFilters.dateRange == originalAnalysis.dateRange)
        print("Workspace filter migration, independent scopes, analysis context and removal reconciliation passed")
        print("Deleted account history is hidden by default, filters use the historical account identity, and hiding resets a stale selection")
        print("Antigravity login accepts committed identity immediately, rejects older status reads, retains identity after quota failure, and preserves drafts")
        print("Theme quick toggle saves globally, preserves unrelated drafts and retains theme after failure")
        print("Atomic filtered/today snapshots and mapping save/reprice failure recovery passed")
        print("Unfiltered menu metric, data-time semantics, per-source retry, and immediate host removal checks passed")
        print("macOS draft rollback/commit, automatic quota retry, per-window visibility, model-options compatibility, host credential transaction rollback/retry, sampling failure throttling, and price failure checks passed")
    }
}
