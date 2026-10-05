"""No real CLI inference, SSH connection or system scheduler is used by these tests."""
import datetime
import importlib.util
import json
import os
import pathlib
import shlex
import sqlite3
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch

spec=importlib.util.spec_from_file_location('wake',pathlib.Path(__file__).with_name('remote_wakeup.py'))
wake=importlib.util.module_from_spec(spec);spec.loader.exec_module(wake)

class WakeTests(unittest.TestCase):
    def manifest(self, binary='fake'):
        return dict(version=1,id='wake-test',accountKey='codex:test',provider='codex',model='model',effort='low',binary=binary,args=['exec','-m','model','-c','model_reasoning_effort="low"',"hi; $(touch never)"],configPath='/fixture',preCommand='',shell='/bin/bash',proxy={'mode':'direct'},times=[datetime.datetime.now().strftime('%H:%M')],enabled=True)

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
            with patch.object(wake.pathlib.Path,'home',return_value=home),patch.object(wake,'probe',return_value={}),patch.object(wake,'resolve',side_effect=lambda m:m),patch.object(wake,'crontab'),patch.object(wake.sys,'platform','linux'):
                req={'action':'deploy','namespace':'a'*20,'manifest':self.manifest(),'script':'original runner'}
                result=wake.manage(req);root=pathlib.Path(result['root']);path=root/'tasks/wake-test.json'
                original=path.read_text()
                with patch.object(wake,'crontab',side_effect=ValueError('install failed')):
                    changed=dict(req,manifest=dict(self.manifest(),model='new'),script='updated runner')
                    with self.assertRaises(ValueError):wake.manage(changed)
                self.assertEqual(path.read_text(),original);self.assertEqual((root/'runner.py').read_text(),'original runner')
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
        with tempfile.TemporaryDirectory() as temporary:
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
