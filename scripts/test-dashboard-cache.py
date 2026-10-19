#!/usr/bin/env python3
"""Exercise the production Swift AppModel cache against isolated real core IPC."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT/'target/debug/aieyes-core')
    args = parser.parse_args()
    binary = args.binary.resolve()
    (ROOT/'.local').mkdir(exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix='native-dashboard-cache-', dir=ROOT/'.local'))
    data = output/'data'
    data.mkdir()
    logs = output/'rpc'
    logs.mkdir()
    spec = importlib.util.spec_from_file_location('benchmark', ROOT/'scripts/benchmark-performance.py')
    benchmark = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(benchmark)
    settings = benchmark.seed(data, 10000, binary)
    for source in settings['sources']:
        path = output/'empty'/source['id']
        path.mkdir(parents=True)
        source['path'] = str(path)
    settings['proxy'] = dict(mode='custom', url='http://127.0.0.1:9')
    core = benchmark.Core(data, binary)
    try:
        core.call('settings.save', settings)
    finally:
        core.close()
    source = ROOT/'apps/macos/Sources/Aieyes'
    names = ['MonitorSelection', 'Models', 'ProviderPresentation', 'EngineClient', 'QuotaNotifications',
        'ConfigurationActions', 'DesignTokens', 'Palette', 'SessionActivity']
    executable = output/'verify-dashboard-cache'
    subprocess.run(['swiftc', '-module-cache-path', str(ROOT/'.build/swift-module-cache'),
        *[str(source/(name+'.swift')) for name in names], str(ROOT/'scripts/verify-dashboard-cache.swift'),
        '-o', str(executable)], check=True)
    proxy = output/'core-log.py'
    shutil.copyfile(ROOT/'scripts/runtime-ipc-proxy.py', proxy)
    proxy.chmod(0o700)
    env = dict(os.environ, AIEYES_DATA_DIR=str(data), AIEYES_CORE_PATH=str(proxy),
        AIEYES_AUDIT_REAL_CORE=str(binary), AIEYES_AUDIT_LOG_DIR=str(logs), AIEYES_RUNTIME_CACHE_FIXTURE='1')
    run = subprocess.run([str(executable)], env=env, capture_output=True, text=True, timeout=90)
    (output/'stdout.log').write_text(run.stdout)
    (output/'stderr.log').write_text(run.stderr)
    if run.returncode:
        raise RuntimeError(f'Native cache verification failed; artifacts: {output}\n{run.stderr}')
    report = json.loads(run.stdout.strip().splitlines()[-1])
    report.update(output=str(output), binary=str(binary), events=10000)
    (output/'report.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report), flush=True)


if __name__ == '__main__':
    main()
