from types import SimpleNamespace
"""No real CLI inference, SSH connection or system scheduler is used by these tests."""
from contextlib import closing
import datetime
import base64
import importlib.util
import json
import os
import pathlib
import shlex
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import Mock, patch

spec=importlib.util.spec_from_file_location('wake',pathlib.Path(__file__).with_name('remote_wakeup.py'))
wake=importlib.util.module_from_spec(spec);spec.loader.exec_module(wake)

class WakeTests(unittest.TestCase):
    def test_next_run_uses_real_full_minutes(self):
        start = datetime.datetime.fromisoformat('2026-01-15T07:59:40+00:00').timestamp()
        with patch.object(wake.time, 'localtime', time.gmtime):
            self.assertEqual(wake.next_run(['08:00'], start), start + 20)
            self.assertEqual(wake.next_run(['08:00'], start + 20), start + 20 + 86400)
            self.assertIsNone(wake.next_run([], start))

    @unittest.skipUnless(hasattr(time, 'tzset'), 'Remote scheduler follows POSIX timezone rules')
    def test_next_run_matches_shared_dst_cases_and_recorded_slots(self):
        fixture = json.loads(pathlib.Path(__file__).with_name('fixtures').joinpath('wakeup-times.json').read_text())
        previous = os.environ.get('TZ')
        try:
            for row in fixture:
                if row['status'] != 'ready' or row['approximate']:
                    continue
                os.environ['TZ'] = row['zone']; time.tzset()
                start = datetime.datetime.fromisoformat(row['now'].replace('Z', '+00:00')).timestamp()
                expected = datetime.datetime.fromisoformat(row['expected'].replace('Z', '+00:00')).timestamp()
                self.assertEqual(wake.next_run([row['time']], start), expected, row['name'])
            os.environ['TZ'] = 'America/New_York'; time.tzset()
            start = datetime.datetime.fromisoformat('2026-11-01T05:45:00+00:00').timestamp()
            occurred = {'wake-test:2026-11-01:01:30'}
            expected = datetime.datetime.fromisoformat('2026-11-02T06:30:00+00:00').timestamp()
            self.assertEqual(wake.next_run(['01:30'], start, occurred, 'wake-test'), expected)
        finally:
            if previous is None: os.environ.pop('TZ', None)
            else: os.environ['TZ'] = previous
            time.tzset()

    def test_next_run_reads_only_this_tasks_occurrences(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary).resolve()
            self.assertEqual(wake.scheduled_occurrences(root, 'wake-test', 100), set())
            with closing(sqlite3.connect(root / 'runner.sqlite')) as db, db:
                db.execute('CREATE TABLE runs(task TEXT, occurrence TEXT, started INTEGER)')
                db.executemany('INSERT INTO runs VALUES(?,?,?)', [('wake-test', 'wake-test:2026-11-01:01:30', 100), ('other', 'other:2026-11-01:01:30', 100)])
            self.assertEqual(wake.scheduled_occurrences(root, 'wake-test', 100), {'wake-test:2026-11-01:01:30'})

    def manifest(self, binary='fake'):
        return dict(version=1,id='wake-test',accountKey='codex:test',provider='codex',model='model',effort='low',binary=binary,args=['exec','-m','model','-c','model_reasoning_effort="low"',"hi; $(touch never)"],configPath='/fixture',preCommand='',shell='/bin/bash',proxy={'mode':'direct'},times=[datetime.datetime.now().strftime('%H:%M')],enabled=True)

    @unittest.skipUnless(os.name == 'posix', 'Remote runner requires a POSIX host')
    def test_profile_probe_passes_initialization_to_auth_helper(self):
        spec = importlib.util.spec_from_file_location('auth_helper', pathlib.Path(__file__).with_name('remote_codex_auth.py'))
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary).resolve()
            claims = {'https://api.openai.com/auth': {'chatgpt_user_id': 'fixture', 'chatgpt_account_id': 'team'}}
            token = base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip('=')
            credential = {'auth_mode': 'chatgpt', 'tokens': {'access_token': 'h.' + token + '.s', 'refresh_token': 'fixture', 'account_id': 'team'}}
            helper.save_profile(root, 'fixture', 'Fixture', credential)
            binary = root / 'fake-codex'
            binary.write_text('''#!/usr/bin/env python3
import json, os, sys
if '--help' in sys.argv:
    print('--model --ignore-user-config'); sys.exit(0)
if '--version' in sys.argv:
    assert os.environ['AIEYES_WAKE_FIXTURE'] == 'ready'
    print('codex-cli 0.161.0'); sys.exit(0)
assert os.environ['AIEYES_WAKE_FIXTURE'] == 'ready'
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    if request['method'] == 'account/read': result = {'account': {'type': 'chatgpt'}}
    elif request['method'] == 'model/list': result = {'data': [{'model': 'fixture-model'}]}
    else: result = {}
    print(json.dumps({'id': request['id'], 'result': result}), flush=True)
''')
            binary.chmod(0o700)
            script = root / 'proxy setup'
            script.write_text('export AIEYES_WAKE_FIXTURE=ready\nprintf "private banner\\n"\nread ignored || true\n')
            m = dict(self.manifest(str(binary)), configPath=str(root), codexProfileId='home:fixture',
                     model='', effort='', preCommand='source ' + shlex.quote(str(script)), shell='/bin/bash')
            with patch.object(wake, 'auth_helper', return_value=helper):
                self.assertEqual(wake.probe(m)['models'][0]['id'], 'fixture-model')
            self.assertFalse((root / '.aieyes/codex/fixture/query/auth.json').exists())

    def test_profile_model_query_preserves_explicit_empty_precommand(self):
        helper = Mock()
        helper.call.return_value = {'models': {'data': []}}
        m = dict(self.manifest(), codexProfileId='home:fixture', preCommand='', shell='/bin/sh')
        with patch.object(wake, 'auth_helper', return_value=helper):
            self.assertEqual(wake.codex_rpc(m, 'model/list', {}), {'data': []})
        loc = helper.call.call_args.args[0]['location']
        self.assertEqual(loc['preCommand'], '')
        self.assertEqual(loc['shell'], '/bin/sh')

    @unittest.skipUnless(os.name == 'posix', 'Remote runner requires a POSIX host')
    def test_codex_rpc_uses_default_stdio_for_older_cli(self):
        with tempfile.TemporaryDirectory() as temporary:
            script = pathlib.Path(temporary) / 'fake-codex.py'
            script.write_text('''import json, sys
assert sys.argv[1:] == ['app-server'], 'unsupported transport argument'
for line in sys.stdin:
    request = json.loads(line)
    if 'id' in request:
        print(json.dumps({'id': request['id'], 'result': {'ok': True}}), flush=True)
''')
            # Keep the actual process/stdio exchange; prepend Python to avoid
            # depending on executable shebang support or a real Codex install.
            with patch.object(wake, 'shell_argv', side_effect=lambda m, args: [sys.executable, *args]):
                result = wake.codex_rpc(self.manifest(str(script)), 'account/read', {'refreshToken': False})
            self.assertEqual(result, {'ok': True})

    @unittest.skipUnless(os.name == "posix", "Remote runner integration requires a POSIX host")
    def test_sourced_environment_and_banner_are_isolated(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            script = root / 'proxy setup'
            script.write_text('export WAKE_TEST_VALUE=ready\nprintf "private banner\\n"\n')
            m = dict(self.manifest('/bin/sh'), shell='/bin/bash', preCommand='source ' + shlex.quote(str(script)))
            self.assertEqual(wake.call(m, ['-c', 'printf "%s" "$WAKE_TEST_VALUE"']), 'ready')
            # The same wrapper must leave stdin available for app-server RPC.
            argv = wake.shell_argv(m, ['/bin/sh', '-c', 'read value; printf "%s" "$value"'])
            result = subprocess.run(argv, input=b'rpc-message\n', stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            self.assertEqual(result.stdout, b'rpc-message')

    @unittest.skipUnless(os.name == "posix", "Remote runner integration requires a POSIX host")
    def test_precommand_failure_is_distinct_from_cli_failure(self):
        m = dict(self.manifest('/bin/sh'), preCommand='echo secret-token >&2\nfalse')
        with self.assertRaisesRegex(ValueError, '前置命令未完成（退出码 1）') as error:
            wake.call(m, ['--help'])
        self.assertNotIn('secret-token', str(error.exception))
        m['preCommand'] = 'exit 0'
        with self.assertRaisesRegex(ValueError, '前置命令未完成（退出码 0）'):
            wake.call(m, ['--help'])
        m['preCommand'] = 'export WAKE_TEST_VALUE=ready'
        with self.assertRaisesRegex(ValueError, 'CLI 调用失败（退出码 7）') as error:
            wake.call(m, ['-c', 'echo secret-token >&2; exit 7'])
        self.assertNotIn('secret-token', str(error.exception))

    @unittest.skipUnless(os.name == "posix", "Remote runner integration requires a POSIX host")
    def test_help_failure_reports_stage(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = pathlib.Path(temporary) / 'fake-codex'
            binary.write_text('#!/bin/sh\nexit 9\n')
            binary.chmod(0o700)
            with self.assertRaisesRegex(ValueError, '读取 CLI 帮助失败（退出码 9）'):
                wake.call(self.manifest(str(binary)), ['--help'])

    def test_diagnostics_without_host_shell(self):
        # Windows hosts can verify remote diagnostics without executing Unix programs.
        cases = [
            ('source ~/proxy', 1, b'secret-token', '前置命令未完成（退出码 1）'),
            ('exit 0', 0, b'', '前置命令未完成（退出码 0）'),
            ('source ~/proxy', 7, wake.CLI_READY + b'\nsecret-token', '读取 CLI 帮助失败（退出码 7）'),
            ('', 9, b'secret-token', '读取 CLI 帮助失败（退出码 9）'),
        ]
        for precommand, code, stderr, message in cases:
            with self.subTest(precommand=precommand, code=code):
                child = Mock(returncode=code)
                child.communicate.return_value = (b'', stderr)
                with patch.object(wake.subprocess, 'Popen', return_value=child):
                    with self.assertRaisesRegex(ValueError, message) as error:
                        wake.call(dict(self.manifest(), preCommand=precommand), ['--help'])
                self.assertNotIn('secret-token', str(error.exception))

    def test_scheduler_preserves_other_jobs_and_repeated_remove_is_safe(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=pathlib.Path(temporary)/'with spaces%';root.mkdir()
            content=['0 9 * * * echo unrelated\n'];commands=[]
            def fake_run(args,**kw):
                commands.append(args)
                if args==['crontab','-l']: return subprocess.CompletedProcess(args,0,content[0].encode(),b'')
                content[0]=kw['input'].decode();return subprocess.CompletedProcess(args,0)
            with patch.object(wake.subprocess,'run',side_effect=fake_run):
                wake.crontab(root);wake.crontab(root)
                self.assertEqual(content[0].count(wake.tag(root)),1)
                self.assertIn('echo unrelated',content[0]);self.assertIn('\\%',content[0])
                wake.crontab(root,True);wake.crontab(root,True)
                self.assertEqual(content[0],'0 9 * * * echo unrelated\n')

    def test_deploy_update_rollback_disable_remove(self):
        with tempfile.TemporaryDirectory() as temporary:
            home=pathlib.Path(temporary)
            with patch.object(wake.pathlib.Path,'home',return_value=home),patch.object(wake,'probe',return_value={}),patch.object(wake,'resolve',side_effect=lambda m:m),patch.object(wake,'crontab'),patch.object(wake.sys,'platform','linux'),patch.object(wake.subprocess,'run',return_value=SimpleNamespace(returncode=0,stdout=(wake.tag(home/'.local/share/aieyes-wakeups'/('a'*20))).encode())):
                req={'action':'deploy','namespace':'a'*20,'manifest':self.manifest(),'script':'original runner','authHelper':'fixture_version=1'}
                result=wake.manage(req);root=pathlib.Path(result['root']);path=root/'tasks/wake-test.json'
                original=path.read_text()
                with patch.object(wake,'crontab',side_effect=ValueError('install failed')):
                    changed=dict(req,manifest=dict(self.manifest(),model='new'),script='updated runner',authHelper='fixture_version=2')
                    with self.assertRaises(ValueError):wake.manage(changed)
                self.assertEqual(path.read_text(),original);self.assertEqual((root/'runner.py').read_text(),'original runner')
                self.assertEqual((root/'codex_auth.py').read_text(),'fixture_version=1')
                control={'root':str(root),'id':'wake-test'}
                wake.manage(dict(control,action='disable'));self.assertFalse(json.loads(path.read_text())['enabled'])
                wake.manage(dict(control,action='enable'));self.assertTrue(json.loads(path.read_text())['enabled'])
                wake.manage(dict(control,action='remove'));wake.manage(dict(control,action='remove'));self.assertFalse(path.exists())

    def test_tick_deduplicates_and_skips_missed_or_overlapping_tasks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=pathlib.Path(temporary);wake.private_dir(root/'tasks')
            m=self.manifest();wake.atomic(root/'tasks/wake-test.json',json.dumps(m))
            with patch.object(wake,'auth'),patch.object(wake,'call',return_value='{"type":"turn.completed"}') as cli:
                wake.tick(root);wake.tick(root);self.assertEqual(cli.call_count,1)
                self.assertEqual(wake.history(root,'wake-test')[0]['status'],'success')
                m['id']='wake-missed';m['times']=[(datetime.datetime.now()-datetime.timedelta(minutes=1)).strftime('%H:%M')]
                wake.atomic(root/'tasks/wake-missed.json',json.dumps(m));wake.tick(root);self.assertEqual(cli.call_count,1)
                m=self.manifest();m['id']='wake-overlap';wake.atomic(root/'tasks/wake-overlap.json',json.dumps(m))
                with sqlite3.connect(str(root/'runner.sqlite')) as db:db.execute('INSERT INTO leases VALUES(?,?)',('codex:test',int(wake.time.time())+100))
                db.close()
                wake.tick(root);self.assertEqual(cli.call_count,1);self.assertEqual(wake.history(root,'wake-overlap')[0]['status'],'skipped-overlap')

    def test_zero_exit_without_completion_and_timeout_are_failures(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=pathlib.Path(temporary);wake.atomic(root/'tasks/wake-test.json',json.dumps(self.manifest()))
            with patch.object(wake,'auth'),patch.object(wake,'call',return_value='{"type":"turn.failed","error":"secret"}'):
                wake.tick(root);self.assertEqual(wake.history(root,'wake-test')[0]['status'],'failed')
            with patch.object(wake,'auth'),patch.object(wake,'call',side_effect=ValueError('CLI 调用超时')):
                wake.tick(root,'wake-test');self.assertEqual(wake.history(root,'wake-test')[0]['status'],'timeout')
            self.assertNotIn('secret',json.dumps(wake.history(root,'wake-test')))

    def test_independent_accounts_start_together_and_same_account_is_skipped(self):
        scheduled = datetime.datetime.now()
        with tempfile.TemporaryDirectory() as temporary, patch.object(wake.datetime, 'datetime', wraps=datetime.datetime) as clock:
            clock.now.return_value = scheduled
            root = pathlib.Path(temporary)
            for identifier, account in [('wake-a', 'a'), ('wake-b', 'b'), ('wake-c', 'a')]:
                m = dict(self.manifest(), id=identifier, accountKey=account)
                wake.atomic(root / 'tasks' / (identifier + '.json'), json.dumps(m))
            barrier = threading.Barrier(2)
            def inference(*args):
                barrier.wait(timeout=3)
                return '{"type":"turn.completed"}'
            with patch.object(wake, 'auth'), patch.object(wake, 'call', side_effect=inference) as cli:
                wake.tick(root)
            self.assertEqual(cli.call_count, 2)
            self.assertEqual(wake.history(root, 'wake-a')[0]['status'], 'success')
            self.assertEqual(wake.history(root, 'wake-b')[0]['status'], 'success')
            self.assertEqual(wake.history(root, 'wake-c')[0]['status'], 'skipped-overlap')

    def test_foreign_roots_and_task_ids_are_rejected(self):
        for root,identifier in [('/tmp/foreign','wake-ok'),(str(pathlib.Path.home()/'.local/share/aieyes-wakeups'/('a'*20)),'../../anything')]:
            with self.assertRaises(ValueError):wake.manage({'action':'remove','root':root,'id':identifier})

if __name__=='__main__':unittest.main()
