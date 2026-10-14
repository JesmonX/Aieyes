import Foundation

@main struct VerifyWakeups {
    @MainActor static func main() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("aieyes-native-wakeups-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let script = root.appendingPathComponent("core.py")
        let program = #"""
#!/usr/bin/env python3
import json, os, sys, time
from pathlib import Path
root = Path(__file__).parent
def held(name):
    if (root / ('hold-' + name)).exists():
        (root / (name + '-started')).touch()
        deadline = time.monotonic() + 8
        while not (root / ('release-' + name)).exists() and time.monotonic() < deadline:
            time.sleep(.01)
for line in sys.stdin:
    request = json.loads(line)
    method, params = request['method'], request.get('params', {})
    response = {'id': request['id']}
    rows = json.loads((root / 'records.json').read_text())
    if method == 'wakeups.list':
        held('list')
        response['result'] = rows
    elif method == 'wakeups.status':
        if params['id'] == 'slow': held('status')
        if (root / ('fail-status-' + params['id'])).exists():
            response['error'] = {'message': params['id'] + ' status failed'}
        else:
            response['result'] = {'installed': True, 'enabled': True, 'timezone': 'UTC', 'history': []}
    elif method == 'wakeups.save':
        task = params['task']
        for row in rows:
            if row['task']['id'] == task['id']: row['task'] = task
        (root / 'records.json').write_text(json.dumps(rows))
        response['result'] = next(row for row in rows if row['task']['id'] == task['id'])
    else:
        action = method.removeprefix('wakeups.')
        if (root / ('fail-' + action + '-' + params['id'])).exists():
            response['error'] = {'message': params['id'] + ' ' + action + ' failed'}
        else:
            for row in rows:
                if row['task']['id'] == params['id'] and action in ['enable', 'disable']:
                    row['deployment']['enabled'] = action == 'enable'
            (root / 'records.json').write_text(json.dumps(rows))
            response['result'] = {}
    print(json.dumps(response), flush=True)
"""#
        try program.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
        let rows: [[String: Any]] = ["slow", "fast"].map { id in
            let task: [String: Any] = ["id": id, "name": id, "sourceId": id, "times": ["08:00"], "model": "fixture", "effort": "low", "prompt": "OK", "binary": ""]
            return ["task": task, "deployment": ["task": task, "deployedAt": 1, "enabled": true, "state": "deployed", "timezone": "UTC", "target": id, "changed": false]]
        }
        try JSONSerialization.data(withJSONObject: rows).write(to: root.appendingPathComponent("records.json"))
        func mark(_ name: String) throws { try Data().write(to: root.appendingPathComponent(name)) }
        func exists(_ name: String) -> Bool { FileManager.default.fileExists(atPath: root.appendingPathComponent(name).path) }
        func remove(_ name: String) throws { try FileManager.default.removeItem(at: root.appendingPathComponent(name)) }
        func waitUntil(_ condition: () -> Bool) async throws {
            let deadline = ProcessInfo.processInfo.systemUptime + 5
            while !condition() && ProcessInfo.processInfo.systemUptime < deadline { try await Task.sleep(for: .milliseconds(10)) }
            precondition(condition(), "Fixture did not finish within five seconds")
        }
        setenv("AIEYES_CORE_PATH", script.path, 1)
        setenv("AIEYES_DATA_DIR", root.path, 1)
        let store = WakeupsStore()
        defer { store.stop() }
        try mark("hold-status")
        await store.load()
        precondition(store.records.count == 2 && !store.loading, "The task list appears before remote status reads complete")
        try await waitUntil { exists("status-started") && store.statuses["fast"] != nil }
        precondition(store.refreshing.contains("slow") && !store.refreshing.contains("fast"))
        var edited = store.records.first { $0.id == "fast" }!.task
        edited.name = "Edited while another machine is slow"
        try await store.save(edited)
        precondition(store.records.first { $0.id == "fast" }?.task.name == edited.name && store.refreshing.contains("slow"), "Saving one task must not wait for another machine's status")

        try mark("fail-enable-fast")
        store.perform("enable", "fast")
        try await waitUntil { !store.operating.contains("fast") }
        precondition(store.operationErrors["fast"]?["enable"] != nil && store.refreshing.contains("slow"))
        store.perform("disable", "fast")
        try await waitUntil { !store.operating.contains("fast") }
        precondition(store.operationErrors["fast"]?["enable"] != nil, "An unrelated successful action must retain the failed operation")
        try mark("release-status")
        try await waitUntil { !store.refreshing.contains("slow") }
        try mark("fail-deploy-slow")
        store.perform("deploy", "slow")
        try await waitUntil { !store.operating.contains("slow") }
        try remove("fail-enable-fast")
        store.perform("enable", "fast")
        try await waitUntil { !store.operating.contains("fast") }
        precondition(store.operationErrors["fast"]?["enable"] == nil && store.operationErrors["slow"]?["deploy"] != nil, "Retry success clears only that task and operation")
        try await waitUntil { !store.refreshing.contains("fast") }
        try mark("fail-status-fast")
        let statusFailed = await store.readStatus("fast")
        precondition(!statusFailed)
        precondition(store.statusErrors["fast"] != nil)
        try remove("fail-status-fast")
        let statusRecovered = await store.readStatus("fast")
        precondition(statusRecovered && store.statusErrors["fast"] == nil && store.operationErrors["slow"]?["deploy"] != nil)

        try mark("hold-list")
        let refresh = Task { await store.load(refreshStatuses: false) }
        try await waitUntil { exists("list-started") }
        edited.name = "Saved after the list snapshot"
        try await store.save(edited)
        try mark("release-list")
        await refresh.value
        precondition(store.records.first { $0.id == "fast" }?.task.name == edited.name, "An older list response must not replace a newly saved task")
        print("Native wakeups: independent status and edits, per-operation error recovery, and stale-list protection passed")
    }
}
