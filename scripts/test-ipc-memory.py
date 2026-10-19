#!/usr/bin/env python3
"""Exercise the exact EngineClient read loop, with real pipes and isolated echo IPC."""
import argparse
import importlib.util
import json
import os
import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
STUBS = '''
struct HostResult: Decodable {}
struct HostMonitorState: Decodable {}
struct Quota: Decodable {}
struct AccountDeviceStatus: Decodable {}
enum CoreContract {
    static let updateBehaviors: [String:String] = [:]
    static let methodLanes: [String:String] = [:]
    static let configurationMethods = Set<String>()
}
'''
HARNESS = '''
import Darwin
struct Echo: Decodable { let sequence: Int; let payload: String }
func memory() -> [String:UInt64] {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout.size(ofValue: info) / MemoryLayout<integer_t>.size)
    let result = withUnsafeMutablePointer(to: &info) { ptr in
        ptr.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
            task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
        }
    }
    precondition(result == KERN_SUCCESS)
    return ["rssBytes":info.resident_size, "physicalBytes":info.phys_footprint]
}
@main struct IPCMemoryTest {
    static func main() async throws {
        let client = EngineClient(configurationOnly: true)
        var samples: [[String:UInt64]] = []
        let count = Int(CommandLine.arguments[2])!
        for index in 1...count {
            let result: Echo = try await client.call("echo", params:["sequence":index])
            precondition(result.sequence == index && result.payload.count == 4096)
            if index % 1000 == 0 {
                var sample = memory(); sample["calls"] = UInt64(index); samples.append(sample)
            }
        }
        let live = memory()
        var disconnected = false
        do { let _: Echo = try await client.call("quit") } catch { disconnected = true }
        precondition(disconnected)
        let recovered: Echo = try await client.call("echo", params:["sequence":42])
        precondition(recovered.sequence == 42)
        client.stop()
        try await Task.sleep(nanoseconds: 200_000_000)
        let report: [String:Any] = ["samples":samples,"connectedEnd":live,"disconnectedEnd":memory(),"partialLinesAndReconnect":true]
        try JSONSerialization.data(withJSONObject:report, options:[.prettyPrinted,.sortedKeys]).write(to:URL(fileURLWithPath:CommandLine.arguments[1]))
    }
}
'''
ECHO = '''#!/usr/bin/python3
import json,os,sys,time
for line in sys.stdin.buffer:
    request=json.loads(line)
    if request['method']=='quit':os._exit(0)
    sequence=request['params']['sequence']
    response=(json.dumps(dict(jsonrpc='2.0',id=request['id'],result=dict(sequence=sequence,payload='x'*4096)))+'\\n').encode()
    if sequence%79==0:
        sys.stdout.buffer.write(response[:13]);sys.stdout.buffer.flush();time.sleep(.002);response=response[13:]
    sys.stdout.buffer.write(response);sys.stdout.buffer.flush()
'''


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--compare-head', action='store_true')
    parser.add_argument('--calls', type=int, default=12000)
    parser.add_argument('--output', type=pathlib.Path, default=ROOT/'.local/ipc-memory.json')
    args = parser.parse_args()
    assert args.calls >= 2000
    relative = 'apps/macos/Sources/Aieyes/EngineClient.swift'
    sources = {'after':(ROOT/relative).read_text()}
    if args.compare_head: sources = {'before':subprocess.check_output(['git','show','HEAD:'+relative],cwd=ROOT,text=True),**sources}
    models = (ROOT/'apps/macos/Sources/Aieyes/Models.swift').read_text()
    boundary = models[models.index('enum ProviderBoundary {'):models.index('enum NativePanelKey:')]
    reports = {}
    with tempfile.TemporaryDirectory(prefix='aieyes-ipc-') as directory:
        root = pathlib.Path(directory); echo = root/'echo.py'; echo.write_text(ECHO); echo.chmod(0o755)
        for label, source in sources.items():
            source = source[:source.index('struct Acknowledgement:')]
            swift = root/(label+'.swift'); swift.write_text(source+STUBS+boundary+HARNESS)
            executable = root/label; report = root/(label+'.json')
            subprocess.run(['swiftc','-parse-as-library','-module-cache-path',str(root/'module-cache'),str(swift),'-o',str(executable)],check=True)
            subprocess.run([str(executable),str(report),str(args.calls)],env={**os.environ,'AIEYES_CORE_PATH':str(echo)},check=True,timeout=120)
            reports[label] = json.loads(report.read_text())
            samples = reports[label]['samples']
            reports[label]['growthAfter1000Bytes'] = samples[-1]['rssBytes']-samples[0]['rssBytes']
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(reports,indent=2))
    print(json.dumps({key:dict(growthAfter1000MiB=round(value['growthAfter1000Bytes']/1024**2,2),samples=len(value['samples'])) for key,value in reports.items()}))
    assert reports['after']['growthAfter1000Bytes'] < 32*1024**2, 'Reader retains temporary objects over the connection lifetime'


if __name__ == '__main__': main()
