#!/usr/bin/env python3
"""Isolation and fail-closed checks for the local release interaction profiler."""
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

SCRIPT=Path(__file__).with_name('profile-release-interactions.py')
SPEC=importlib.util.spec_from_file_location('interaction_profiler',SCRIPT)
profiler=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(profiler)


class IsolationTests(unittest.TestCase):
    def test_online_backup_includes_wal_and_does_not_change_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source=Path(directory)/'source.sqlite'; target=Path(directory)/'copy/data.sqlite'
            with sqlite3.connect(source) as db:
                db.execute('pragma journal_mode=wal')
                db.execute('create table example(value text)')
                db.execute("insert into example values('in the WAL')"); db.commit()
                before=source.read_bytes()
                profiler.database_copy(source,target)
                self.assertEqual(source.read_bytes(),before)
                with sqlite3.connect(target) as copy:
                    self.assertEqual(copy.execute('select * from example').fetchall(),[('in the WAL',)])
                    copy.execute("insert into example values('isolated mutation')");copy.commit()
                self.assertEqual(db.execute('select count(*) from example').fetchone()[0],1)

    def test_unknown_replay_filters_are_not_silently_substituted(self):
        self.assertEqual(profiler.dashboard_key({'days':7,'provider':'qoder'}),'7:qoder')
        for field in ['model','sourceId','accountId','dateRange']:
            with self.assertRaises(ValueError): profiler.dashboard_key({field:'unrecorded'})

    def test_bridge_denies_mutations_and_logs_no_payloads(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); logs=root/'rpc';logs.mkdir()
            fixture={'runtime.revisions':{'usage':1,'quotas':1},'hosts.sample':[],
                'hello':{'contractHash':'fixture','hostMonitoring':True},
                'settings.get':{'privateMarker':'do-not-log-this'},
                'dashboard.summary':{'sources':[],'quotas':[]},'dashboards':{'7:all':{'summary':{}}}}
            (root/'fixture.json').write_text(json.dumps(fixture))
            requests=[('hello',{}),('settings.get',{}),('settings.save',{}),
                      ('sources.scan',{}),('runtime.revisions',{}),('dashboard',{'days':7})]
            data=''.join(json.dumps(dict(id=i,method=m,params=p))+'\n' for i,(m,p) in enumerate(requests))
            run=subprocess.run([sys.executable,str(SCRIPT),'bridge'],input=data,text=True,
                capture_output=True,timeout=10,env=dict(os.environ,AIEYES_PROFILE_ROOT=str(root),
                    AIEYES_PROFILE_RPC=str(logs),AIEYES_PROFILE_CONTRACT='driver-contract'))
            self.assertEqual(run.returncode,0,run.stderr)
            replies=[json.loads(line) for line in run.stdout.splitlines()]
            self.assertEqual(replies[0]['result']['contractHash'],'driver-contract')
            self.assertIn('error',replies[2])
            self.assertEqual(replies[4]['result']['usage'],2)
            records=''.join(p.read_text() for p in logs.glob('*.jsonl'))
            self.assertNotIn('do-not-log-this',records)
            self.assertEqual(sum(bool(json.loads(line)['error']) for line in records.splitlines()),1)

    def test_core_read_allowlist_excludes_external_work(self):
        for method in ['sources.scan','quotas.refresh','network.test','hosts.sample',
                       'accounts.status.refreshBatch','settings.save','autoEstimates.tick']:
            self.assertNotIn(method,profiler.READS)

    def test_menu_full_history_is_rejected_against_status_display_projection(self):
        fixture={'dashboard.status':{'quotaEstimates':[{'id':'current'}],'creditEstimates':[]},
                 'dashboard.summary':{'quotaEstimates':[{'id':'current'}],'creditEstimates':[]}}
        profiler.validate_fixture(fixture)
        fixture['dashboard.summary']['quotaEstimates'].append({'id':'completed-history'})
        with self.assertRaisesRegex(ValueError,'display projection'):
            profiler.validate_fixture(fixture)


if __name__=='__main__': unittest.main()
